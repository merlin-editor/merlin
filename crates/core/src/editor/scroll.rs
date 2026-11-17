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
        let total = self.row_index().total();
        if self.scroll_beyond_last_line {
            total.saturating_sub(1) as f64
        } else {
            total.saturating_sub(self.text_height()) as f64
        }
    }

    /// The rows before each line of the document drawn, and in all, with
    /// wrapped lines counted by their rows and folded ones not at all. The
    /// rows of wrapped lines are kept by the document, up to date with
    /// edits; folds and the rows views open between lines go on top.
    pub(crate) fn row_index(&self) -> RowIndex {
        let doc = self.doc();
        let lines = doc.last_line() + 1;
        let mut index = RowIndex {
            lines,
            wrap: None,
            hidden: Vec::new(),
            steps: Vec::new(),
        };
        // A row a line, as for most files: nothing to count.
        if !self.row_based() && self.merging().is_none() && self.agent_change_here().is_none() {
            return index;
        }
        if self.wrapping() {
            index.wrap = Some(doc.wrap_rows(self.text_columns()));
        }
        index.hidden = self.hidden_lines();
        let mut steps: Vec<(usize, i64)> = Vec::new();
        for r in &index.hidden {
            let rows = index.base(r.end) - index.base(r.start);
            steps.push((r.end, -(rows as i64)));
        }
        for line in self.gap_lines() {
            if line < lines && !index.hidden.iter().any(|r| r.contains(&line)) {
                let gap = self.peek_gap(line);
                if gap > 0 {
                    steps.push((line + 1, gap as i64));
                }
            }
        }
        steps.sort_unstable_by_key(|s| s.0);
        let mut sum = 0;
        for s in &mut steps {
            sum += s.1;
            s.1 = sum;
        }
        index.steps = steps;
        index
    }

    /// Where the view's top is, in rows from the top of the document.
    pub(crate) fn top_row(&self) -> f64 {
        let (row, frac) = self.top_sub();
        let before = self.row_index().before(self.view().scroll);
        before as f64 + row as f64 + frac as f64
    }

    /// Puts the top at row `at` of the document, part of a row too.
    pub(crate) fn set_top_row(&mut self, at: f64) {
        let index = self.row_index();
        let at = at.clamp(0.0, self.max_top_row());
        let whole = at.floor() as usize;
        let line = index.line_at(whole);
        let doc = self.doc_mut();
        doc.scroll = line;
        doc.scroll_sub = Sub {
            line,
            row: whole - index.before(line),
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

/// The rows before each line drawn, and in all.
#[derive(Clone)]
pub(crate) struct RowIndex {
    lines: usize,
    /// Each line's rows when wrapped; one each without.
    wrap: Option<std::rc::Rc<crate::rows::WrapRows>>,
    /// The lines folded away, which take no rows.
    hidden: Vec<std::ops::Range<usize>>,
    /// From each line on, the rows folds took away and views opened, in
    /// all: by line.
    steps: Vec<(usize, i64)>,
}

impl RowIndex {
    /// The rows before `line`, as wrapped, before folds and views.
    fn base(&self, line: usize) -> usize {
        match &self.wrap {
            Some(w) => w.before(line),
            None => line.min(self.lines),
        }
    }

    /// The rows before `line`; past the last line, before the last.
    pub(crate) fn before(&self, line: usize) -> usize {
        let line = line.min(self.lines.saturating_sub(1));
        // A folded line's rows are none: those before it are its fold's.
        let line = self
            .hidden
            .iter()
            .find(|r| r.contains(&line))
            .map_or(line, |r| r.start);
        let k = self.steps.partition_point(|s| s.0 <= line);
        let moved = k.checked_sub(1).map_or(0, |k| self.steps[k].1);
        (self.base(line) as i64 + moved).max(0) as usize
    }

    pub(crate) fn total(&self) -> usize {
        let moved = self.steps.last().map_or(0, |s| s.1);
        (self.base(self.lines) as i64 + moved).max(0) as usize
    }

    /// The line whose rows hold `row`: the last that starts at or before.
    pub(crate) fn line_at(&self, row: usize) -> usize {
        if self.wrap.is_none() && self.steps.is_empty() && self.hidden.is_empty() {
            return row.min(self.lines.saturating_sub(1));
        }
        // The last line whose rows before it are at most `row`.
        let (mut lo, mut hi) = (0, self.lines);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.before(mid) <= row {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }
}
