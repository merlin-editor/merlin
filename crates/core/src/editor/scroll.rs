//! Scrolling as a window does it: by rows and parts of a row, so a
//! touchpad moves the text as far as the fingers do. Wrapped lines count
//! by the rows they take and folded ones not at all, so the top of the
//! view can be any row of any line.
//!
//! The top is the first line shown (`scroll`, as everything else reads
//! it), which of its rows is at the top, and how far up that row has gone.

use super::Editor;
use crate::doc::Sub;

impl Editor {
    /// Which row of the first line shown is at the top, and how much of it
    /// is scrolled off, in the group drawn.
    pub(crate) fn top_sub(&self) -> (usize, f32) {
        let v = self.view();
        if v.sub.line != v.scroll {
            return (0, 0.0);
        }
        let rows = self.line_rows(v.scroll).len().max(1);
        (v.sub.row.min(rows - 1), v.sub.frac)
    }

    /// Scrolls the focused group's view by `delta` rows, down for more than
    /// none, no further than the top or the last line.
    pub(crate) fn scroll_by(&mut self, delta: f64) {
        let (mut row, frac) = self.top_sub();
        let mut line = self.doc().scroll;
        let last = self.doc().last_line();
        let pos = frac as f64 + delta;
        let mut whole = pos.floor() as i64;
        let mut frac = (pos - whole as f64) as f32;
        let rows = |e: &Self, l: usize| e.line_rows(l).len();
        while whole > 0 {
            if row + 1 < rows(self, line) {
                row += 1;
            } else if let Some(next) = (line + 1..=last).find(|&l| rows(self, l) > 0) {
                (line, row) = (next, 0);
            } else {
                frac = 0.0;
                break;
            }
            whole -= 1;
        }
        while whole < 0 {
            if row > 0 {
                row -= 1;
            } else if let Some(above) = (0..line).rev().find(|&l| rows(self, l) > 0) {
                (line, row) = (above, rows(self, above) - 1);
            } else {
                frac = 0.0;
                break;
            }
            whole += 1;
        }
        // The last row can be the top, as VS Code lets it, but no further.
        let at_end =
            (line + 1..=last).all(|l| rows(self, l) == 0) && row + 1 >= rows(self, line).max(1);
        if at_end {
            frac = 0.0;
        }
        let doc = self.doc_mut();
        doc.scroll = line;
        doc.scroll_sub = Sub { line, row, frac };
        let max = self.max_top_row();
        if self.top_row() > max {
            self.set_top_row(max);
        }
    }

    /// The furthest down the top goes: to the last row, as VS Code lets
    /// it, or without `scroll_beyond_last_line`, a view's height above the
    /// end.
    fn max_top_row(&self) -> f64 {
        let total = *self.row_index().last().unwrap_or(&0);
        if self.scroll_beyond_last_line {
            total.saturating_sub(1) as f64
        } else {
            total.saturating_sub(self.text_height()) as f64
        }
    }

    /// The rows before each line of the document drawn, and in all, with
    /// wrapped lines counted by their rows and folded ones not at all:
    /// cached until the text, the width or the folds change.
    pub(crate) fn row_index(&self) -> std::rc::Rc<Vec<usize>> {
        let doc = self.doc();
        let key = (
            doc.id,
            doc.buffer.version(),
            self.wrapping().then(|| self.text_columns()),
            self.view().folds,
            self.peek.as_ref().map(|p| (p.line, self.peek_gap(p.line))),
            self.change_peek
                .as_ref()
                .map(|p| (p.index, p.version, self.change_peeking())),
            self.merging()
                .map(|m| (m.version, m.left(), self.merge_inline())),
        );
        let mut cache = self.row_cache.borrow_mut();
        if let Some((k, index)) = cache.as_ref()
            && *k == key
        {
            return index.clone();
        }
        let lines = doc.last_line() + 1;
        let mut before = Vec::with_capacity(lines + 1);
        let mut rows = 0;
        for line in 0..lines {
            before.push(rows);
            rows += self.line_rows(line).len();
        }
        before.push(rows);
        let index = std::rc::Rc::new(before);
        *cache = Some((key, index.clone()));
        index
    }

    /// Where the view's top is, in rows from the top of the document.
    pub(crate) fn top_row(&self) -> f64 {
        let (row, frac) = self.top_sub();
        let before = self.row_index()[self.view().scroll.min(self.doc().last_line())];
        before as f64 + row as f64 + frac as f64
    }

    /// Puts the top at row `at` of the document, part of a row too.
    pub(crate) fn set_top_row(&mut self, at: f64) {
        let index = self.row_index();
        let at = at.clamp(0.0, self.max_top_row());
        let whole = at.floor() as usize;
        // The line whose rows hold it: the last that starts at or before.
        let line = index[..index.len() - 1]
            .partition_point(|&r| r <= whole)
            .saturating_sub(1);
        let doc = self.doc_mut();
        doc.scroll = line;
        doc.scroll_sub = Sub {
            line,
            row: whole - index[line],
            frac: (at - whole as f64) as f32,
        };
    }

    /// Lines the top row so it's whole, as revealing the cursor does.
    pub(crate) fn snap_top(&mut self) {
        let doc = self.doc_mut();
        if doc.scroll_sub.line == doc.scroll {
            doc.scroll_sub.frac = 0.0;
        }
    }

    /// Puts row `row` of `line` at the top.
    pub(crate) fn set_top(&mut self, line: usize, row: usize) {
        let doc = self.doc_mut();
        doc.scroll = line;
        doc.scroll_sub = Sub {
            line,
            row,
            frac: 0.0,
        };
    }
}
