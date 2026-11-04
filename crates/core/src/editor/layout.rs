//! Where each part of the window goes, as VS Code arranges it: the menu bar
//! on top, the side bar on the left, the editor with its tabs in the middle,
//! plugin panels on the right, and the status bar at the bottom.

use super::{Editor, Side};
use crate::screen::str_width;

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

/// The activity bar's width: a marker, an icon and its overflow, a badge.
pub(crate) const ACTIVITY_W: u16 = 4;

/// Where each part goes. Parts are flat, set apart by shade as in VS Code.
pub(crate) struct Layout {
    /// The title bar: menus, the command center and layout buttons.
    pub menu: Rect,
    pub activity: Option<Rect>,
    /// The side bar's views.
    pub left: Option<Rect>,
    /// Plugin panels: the secondary side bar.
    pub right: Option<Rect>,
    /// The panel under the editor: Problems and Output.
    pub bottom: Option<Rect>,
    pub tabs: Rect,
    /// The breadcrumbs under the tabs: where the file and the cursor are.
    pub crumbs: Rect,
    /// Every editor group, with its tabs.
    pub editor: Rect,
    /// Gutter, text and scrollbar.
    pub text: Rect,
    pub status: Rect,
}

impl Layout {
    pub fn side(&self, side: Side) -> Option<Rect> {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
            Side::Bottom => self.bottom,
        }
    }
}

impl Editor {
    pub(crate) fn layout(&self) -> Layout {
        let (w, h) = (self.width, self.height);
        if self.windowed {
            return self.window_layout();
        }
        let menu_h = (h >= 6) as u16;
        let status_y = h.saturating_sub(1);
        let top = menu_h;
        let mid = status_y.saturating_sub(top);
        let mut x0 = 0;
        let mut x1 = w;
        let activity = (w >= 50 && mid > 0).then(|| {
            x0 = ACTIVITY_W;
            Rect {
                x: 0,
                y: top,
                w: ACTIVITY_W,
                h: mid,
            }
        });
        let left = (self.bars[Side::Left as usize].visible && w >= 60).then(|| {
            let lw = (w / 4).clamp(22, 36);
            let r = Rect {
                x: x0,
                y: top,
                w: lw,
                h: mid,
            };
            x0 += lw;
            r
        });
        let right = self.bars[Side::Right as usize].visible.then(|| {
            let rw = (w * 2 / 5).clamp(24, 48).min((x1 - x0).saturating_sub(16));
            x1 -= rw;
            Rect {
                x: x1,
                y: top,
                w: rw,
                h: mid,
            }
        });
        let bottom = (self.bars[Side::Bottom as usize].visible && mid >= 14).then(|| {
            let bh = (mid / 3).clamp(6, 14);
            Rect {
                x: x0,
                y: top + mid - bh,
                w: x1 - x0,
                h: bh,
            }
        });
        let editor_h = mid - bottom.map_or(0, |b| b.h);
        let editor = Rect {
            x: x0,
            y: top,
            w: x1 - x0,
            h: editor_h,
        };
        // The group being drawn or worked in, when the editor is split.
        let all = editor;
        let panes = self.pane_rects(editor);
        let editor = panes.get(self.pane()).copied().unwrap_or(editor);
        let tabs_h = 1.min(editor.h);
        let crumbs_h = u16::from(self.shows_crumbs() && editor.h > 4);
        Layout {
            menu: Rect {
                x: 0,
                y: 0,
                w,
                h: menu_h,
            },
            activity,
            left,
            right,
            bottom,
            tabs: Rect {
                h: tabs_h,
                ..editor
            },
            editor: all,
            crumbs: Rect {
                y: editor.y + tabs_h,
                h: crumbs_h,
                ..editor
            },
            text: Rect {
                y: editor.y + tabs_h + crumbs_h,
                h: editor.h - tabs_h - crumbs_h,
                ..editor
            },
            status: Rect {
                x: 0,
                y: status_y,
                w,
                h: (h > 0) as u16,
            },
        }
    }

    /// In a window that draws its own frame, the screen is all editor
    /// groups, without tabs or breadcrumbs: the window draws those.
    fn window_layout(&self) -> Layout {
        let (w, h) = (self.width, self.height);
        let none = Rect::default();
        let all = Rect { x: 0, y: 0, w, h };
        let panes = self.pane_rects(all);
        let editor = panes.get(self.pane()).copied().unwrap_or(all);
        Layout {
            menu: none,
            activity: None,
            left: None,
            right: None,
            bottom: None,
            tabs: Rect { h: 0, ..editor },
            crumbs: Rect { h: 0, ..editor },
            editor: all,
            text: editor,
            status: none,
        }
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
        (self.layout().text.w as usize)
            .saturating_sub(self.gutter() + 1 + self.minimap_width() as usize)
    }

    /// Whether the gutter has a column for git signs or a review's + and -.
    pub(crate) fn has_signs(&self) -> bool {
        let doc = self.doc();
        doc.review.is_some() || doc.git_base.is_some()
    }

    /// The gutter's width: signs, line numbers and a space.
    pub(crate) fn gutter(&self) -> usize {
        let doc = self.doc();
        let biggest = match &doc.review {
            Some(review) => review.lines.iter().map(|(_, n)| n + 1).max().unwrap_or(1),
            None => doc.buffer.rope().len_lines(),
        };
        biggest.to_string().len().max(3) + 1 + self.has_signs() as usize
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
        }
        let x = doc.display_col(line, col);
        if x < doc.hscroll {
            doc.hscroll = x;
        } else if cols > 0 && x >= doc.hscroll + cols {
            doc.hscroll = x + 1 - cols;
        }
    }

    /// A tab's label: icon and name, then a dot for unsaved changes or ×
    /// to close it. Other tabs show × only under the mouse.
    pub(crate) fn tab_label(&self, i: usize) -> String {
        use crate::icons::{Icon, icon};
        let doc = &self.docs[i];
        let name = doc.name();
        let mark = if doc.buffer.is_dirty() {
            icon(Icon::Dirty, self.nerd)
        } else if i == self.pane_doc() || self.hovered_tab() == Some(i) {
            icon(Icon::Close, self.nerd)
        } else {
            ' '
        };
        let (glyph, _) = self.tab_icon(i);
        format!(" {glyph} {name} {mark} ")
    }

    /// A tab's file icon and its colour.
    pub(crate) fn tab_icon(&self, i: usize) -> (char, crate::screen::Hue) {
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

    /// The tab under the mouse, if any.
    pub(crate) fn hovered_tab(&self) -> Option<usize> {
        let (x, y) = self.hover?;
        let tabs = self.layout().tabs;
        if !tabs.contains(x, y) {
            return None;
        }
        self.tab_spans()
            .into_iter()
            .find(|(s, e, _)| (*s..*e).contains(&x))
            .map(|(_, _, i)| i)
    }

    /// A tab's width, which doesn't change with its mark.
    fn tab_width(&self, i: usize) -> usize {
        str_width(&self.docs[i].name()) + 6
    }

    /// Each tab's start and end column on screen and document index,
    /// scrolled so the current tab shows.
    pub(crate) fn tab_spans(&self) -> Vec<(u16, u16, usize)> {
        let area = self.layout().tabs;
        let mut spans = Vec::new();
        let mut x = 0;
        for i in self.pane_tabs() {
            let w = self.tab_width(i);
            spans.push((x, x + w, i));
            x += w;
        }
        let active = self.pane_doc();
        let shift = spans
            .iter()
            .find(|s| s.2 == active)
            .map_or(0, |s| s.1)
            .saturating_sub(area.w as usize);
        spans
            .into_iter()
            .filter(|(s, _, _)| *s >= shift)
            .map(|(s, e, i)| {
                let x = area.x as usize;
                ((x + s - shift) as u16, (x + e - shift) as u16, i)
            })
            .filter(|(s, _, _)| *s < area.right())
            .collect()
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
            let first = super::render::split_first(&rows, doc.scroll);
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

    /// Where the text cursor is on screen, if it's in view.
    pub(crate) fn cursor_cell(&self) -> Option<(u16, u16)> {
        self.cell_of(self.view().cursor)
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
