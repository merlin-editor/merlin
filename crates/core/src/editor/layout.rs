//! Where each part of the window goes, as VS Code arranges it: the menu bar
//! on top, the side bar on the left, the editor with its tabs in the middle,
//! plugin panels on the right, and the status bar at the bottom.

use super::Editor;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn right(&self) -> u16 {
        self.x + self.w
    }

    pub fn bottom(&self) -> u16 {
        self.y + self.h
    }
}

/// Text columns a review needs to show side by side.
const SPLIT_WIDTH: u16 = 120;

/// Where an editor group's text goes, in a grid of its own as big as the
/// window made it: the window lays the groups out, with their tabs.
pub(crate) struct Layout {
    /// Gutter and text.
    pub text: Rect,
}

impl Editor {
    /// The group drawn or worked in.
    pub(crate) fn layout(&self) -> Layout {
        Layout {
            text: self.viewport(self.pane()),
        }
    }

    /// Group `g`'s text area in a window, at its own top left.
    pub(crate) fn viewport(&self, g: usize) -> Rect {
        let (w, h) = self
            .viewports
            .get(g)
            .copied()
            .unwrap_or((self.width, self.height));
        Rect { x: 0, y: 0, w, h }
    }

    /// The screen cell where the first visible column of the first visible
    /// line is drawn, just right of the gutter.
    pub fn text_origin(&self) -> (u16, u16) {
        let text = self.layout().text;
        (text.x + self.gutter() as u16, text.y)
    }

    pub(crate) fn text_height(&self) -> usize {
        self.layout().text.h as usize
    }

    /// Columns the text can use: the text area less the gutter, the
    /// minimap and the scrollbar.
    pub(crate) fn text_columns(&self) -> usize {
        // The window keeps its scrollbar and minimap out of the viewport.
        (self.layout().text.w as usize).saturating_sub(self.gutter())
    }

    /// Whether the gutter has a column for git signs or a review's + and -.
    pub(crate) fn has_signs(&self) -> bool {
        let doc = self.doc();
        doc.review.is_some() || doc.git_base.is_some()
    }

    /// The gutter's width: signs, room for the lightbulb, line numbers and
    /// a space.
    pub(crate) fn gutter(&self) -> usize {
        let doc = self.doc();
        let biggest = match &doc.review {
            Some(review) => review.lines.iter().map(|(_, n)| n + 1).max().unwrap_or(1),
            None => doc.buffer.rope().len_lines(),
        };
        biggest.to_string().len().max(3) + 1 + self.has_signs() as usize + self.bulb_room()
    }

    /// Cells left of the line numbers for the code actions' lightbulb: in
    /// code a plugin adds the language of, while a language server could
    /// offer them. The numbers are right-aligned, so it's their padding.
    pub(crate) fn bulb_room(&self) -> usize {
        let code = self.doc().review.is_none() && self.plugin_language().is_some();
        if code && self.has_lsp("code-actions") {
            2
        } else {
            0
        }
    }

    pub(crate) fn scroll_to_cursor(&mut self) {
        if self.row_based() {
            self.scroll_to_cursor_row();
            if self.wrapping() {
                return;
            }
        }
        let height = self.text_height().max(1);
        let cols = self.text_columns();
        let row_based = self.row_based();
        let doc = self.doc_mut();
        let (line, col) = doc.line_col(doc.cursor);
        if row_based {
            // Scrolled by rows already.
        } else if line < doc.scroll {
            doc.scroll = line;
        } else if line >= doc.scroll + height {
            doc.scroll = line + 1 - height;
        } else if line == doc.scroll {
            // The cursor's line whole, not part scrolled off.
            self.snap_top();
        }
        let doc = self.doc_mut();
        let x = doc.display_col(line, col);
        if x < doc.hscroll {
            doc.hscroll = x;
        } else if cols > 0 && x >= doc.hscroll + cols {
            doc.hscroll = x + 1 - cols;
        }
    }

    /// A tab's file icon and its colour.
    pub(crate) fn tab_icon(&self, i: usize) -> (char, crate::style::Hue) {
        let doc = &self.docs[i];
        let file = doc
            .review
            .as_ref()
            .map(|r| &r.path)
            .or(doc.buffer.path.as_ref());
        let shown = file
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        crate::icons::file_icon(&shown, self.nerd)
    }

    /// The rows of a review shown side by side, if the text is wide enough:
    /// the buffer line on the left (old) and on the right (new). Removed
    /// lines pair up with the added lines that replace them.
    pub(crate) fn split_rows(&self) -> Option<Vec<(Option<usize>, Option<usize>)>> {
        use crate::diff::LineKind;
        let review = self.doc().review.as_ref()?;
        if self.layout().text.w < SPLIT_WIDTH {
            return None;
        }
        let lines = &review.lines;
        let mut rows = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            if !matches!(lines[i].0, LineKind::Removed | LineKind::Added) {
                rows.push((Some(i), Some(i)));
                i += 1;
                continue;
            }
            let mut removed = Vec::new();
            while i < lines.len() && lines[i].0 == LineKind::Removed {
                removed.push(i);
                i += 1;
            }
            let mut added = Vec::new();
            while i < lines.len() && lines[i].0 == LineKind::Added {
                added.push(i);
                i += 1;
            }
            for k in 0..removed.len().max(added.len()) {
                rows.push((removed.get(k).copied(), added.get(k).copied()));
            }
        }
        Some(rows)
    }

    /// The text position drawn at a screen cell. Rows above or below the
    /// text give the line just outside the view, so dragging there scrolls.
    pub(crate) fn pos_at_screen(&self, x: u16, y: u16) -> usize {
        let text = self.layout().text;
        let doc = self.doc();
        if let Some(rows) = self.split_rows() {
            let first = super::text_view::split_first(&rows, doc.scroll);
            let row = first + y.saturating_sub(text.y) as usize;
            let (left, right) = rows.get(row).copied().unwrap_or((None, None));
            let on_left = x < text.x + text.w.saturating_sub(2) / 2;
            let line = if on_left {
                left.or(right)
            } else {
                right.or(left)
            };
            return line.map_or(doc.cursor, |l| doc.line_start(l));
        }
        if self.row_based() {
            let x = (x.saturating_sub(text.x) as usize).saturating_sub(self.gutter())
                + if self.wrapping() { 0 } else { doc.hscroll };
            if y < text.y {
                return doc.line_start(doc.scroll.saturating_sub(1));
            }
            let (line, r) = self.line_at_row((y - text.y).min(text.h) as usize);
            if line > doc.last_line() {
                return doc.buffer.len();
            }
            return self.pos_in_row(line, r, x);
        }
        let row = (y as isize - text.y as isize).min(text.h as isize);
        let line = (doc.scroll as isize + row).max(0) as usize;
        let col = (x.saturating_sub(text.x) as usize).saturating_sub(self.gutter()) + doc.hscroll;
        doc.pos_at(line, col)
    }

    /// Where a position in the text is on screen, if it's in view.
    pub(crate) fn cell_of(&self, pos: usize) -> Option<(u16, u16)> {
        let text = self.layout().text;
        let doc = self.doc();
        let v = self.view();
        if self.row_based() {
            let (line, r, x) = self.visual(pos);
            let row = self.screen_row(line, r)?;
            let x = if self.wrapping() {
                x
            } else {
                x.checked_sub(v.hscroll)?
            };
            return (row < text.h as usize && x <= self.text_columns()).then(|| {
                (
                    text.x + (self.gutter() + x.min(self.text_columns().saturating_sub(1))) as u16,
                    text.y + row as u16,
                )
            });
        }
        let (line, col) = doc.line_col(pos);
        let dcol = doc.display_col(line, col);
        let x = (self.gutter() + dcol).checked_sub(v.hscroll)?;
        let row = line.checked_sub(v.scroll)?;
        (row < text.h as usize && x < self.text_columns() + self.gutter() && dcol >= v.hscroll)
            .then(|| (text.x + x as u16, text.y + row as u16))
    }
}
