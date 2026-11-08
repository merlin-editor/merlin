//! Where you are in a file, as VS Code shows it: breadcrumbs under the tabs
//! (the file's folders, the file, and the symbols around the cursor), and
//! sticky scroll, which pins the first lines of the blocks in view to the
//! top of the text.

use super::Editor;
use super::fold::block_end;
use super::guides::indent;
use super::layout::Rect;
use crate::symbols::{self, Symbol};

/// Most lines sticky scroll pins.
const MAX_STICKY: usize = 5;

impl Editor {
    pub(crate) fn shows_crumbs(&self) -> bool {
        self.breadcrumbs && self.doc().review.is_none() && self.doc().buffer.path.is_some()
    }

    /// The symbols around the cursor, outermost first.
    pub(crate) fn symbol_path(&self) -> Vec<Symbol> {
        let doc = self.doc();
        let line = doc.line_col(self.view().cursor).0;
        symbols::path_to(&symbols::symbols(doc), doc, line)
    }

    /// The breadcrumbs: each part's text, and the line it goes to, if any.
    pub(crate) fn crumbs(&self) -> Vec<(String, Option<usize>)> {
        let doc = self.doc();
        // A page, a document's included, has a toolbar of its own there.
        let Some(path) = doc.buffer.path.as_ref().filter(|_| doc.page.is_none()) else {
            return Vec::new();
        };
        let mut parts: Vec<(String, Option<usize>)> = path
            .iter()
            .map(|p| (p.to_string_lossy().to_string(), None))
            .collect();
        if let Some((name, _)) = parts.last_mut() {
            let (glyph, _) = crate::icons::file_icon(name, self.nerd);
            *name = format!("{glyph} {name}");
        }
        for s in self.symbol_path() {
            // A plugin's symbols show by name, as VS Code shows them.
            let name = if s.kind == "impl" || s.kind == "#" || s.end_line.is_some() {
                s.name
            } else {
                format!("{} {}", s.kind, s.name)
            };
            parts.push((name, Some(s.line)));
        }
        parts
    }

    /// Puts the cursor at the start of the text on `line`, for a jump back.
    pub(crate) fn go_to_block(&mut self, line: usize) {
        let doc = self.doc();
        let id = doc.id;
        let cursor = doc.cursor;
        let lead = doc
            .line_chars(line)
            .iter()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .count();
        let pos = doc.line_start(line) + lead;
        self.jumps.push((id, cursor));
        let doc = self.doc_mut();
        doc.cursor = pos;
        doc.anchor = None;
        doc.goal = None;
        self.focus = super::Focus::Editor;
    }

    /// The lines that start the blocks around `line`, outermost first.
    fn blocks_around(&self, line: usize) -> Vec<usize> {
        let doc = self.doc();
        let Some(mut target) = (line..=doc.last_line())
            .take(50)
            .find_map(|l| indent(doc, l))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for l in (0..line).rev().take(5000) {
            if target == 0 {
                break;
            }
            let Some(i) = indent(doc, l) else { continue };
            if i < target && block_end(doc, l).is_some_and(|end| end >= line) {
                out.push(l);
                target = i;
            }
        }
        out.reverse();
        out
    }

    /// The lines sticky scroll pins over the top of the text: the blocks
    /// the first line under them is in. Never over the cursor.
    pub(crate) fn sticky_lines(&self, area: Rect) -> Vec<usize> {
        if !self.sticky_scroll || self.doc().review.is_some() || area.h < 8 {
            return Vec::new();
        }
        let v = self.view();
        let row_of = |rows: usize| {
            if self.row_based() {
                self.line_at_row(rows).0
            } else {
                v.scroll + rows
            }
        };
        let most = MAX_STICKY.min(area.h as usize / 3);
        let mut lines: Vec<usize> = Vec::new();
        // Pinning lines covers more, which may be inside other blocks:
        // settle on lines that start the blocks of the first line still
        // seen below them.
        for _ in 0..4 {
            let first = row_of(lines.len());
            let mut next: Vec<usize> = self
                .blocks_around(first)
                .into_iter()
                .filter(|&l| l < first)
                .collect();
            next.truncate(most);
            if next == lines {
                break;
            }
            lines = next;
        }
        // Not over the cursor's row.
        let (cur_line, r, _) = self.visual(v.cursor);
        if let Some(row) = self.screen_row(cur_line, r) {
            lines.truncate(row);
        }
        lines
    }

    /// The line a click on a sticky row goes to.
    pub(crate) fn sticky_at(&self, area: Rect, y: u16) -> Option<usize> {
        let row = y.checked_sub(area.y)? as usize;
        self.sticky_lines(area).get(row).copied()
    }
}
