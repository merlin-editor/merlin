//! Multiple cursors, as VS Code has them: Ctrl+D adds the next occurrence of
//! the selection, Ctrl+Shift+L selects them all, Shift+Alt+Up and Down add
//! a cursor on the line above or below, and Alt+click adds one anywhere.
//!
//! The document's own cursor is the primary one; the others are in
//! `Doc::extra`. Keys run once per cursor, from the last in the text to the
//! first, so each edit leaves the offsets before it alone, and the whole
//! keystroke is one undo step.

use std::cmp::Reverse;

use super::Editor;
use crate::doc::Caret;
use crate::history::Kind;

impl Editor {
    /// Every cursor, the primary one first. Another group being drawn has
    /// only its own.
    pub(crate) fn carets(&self) -> Vec<Caret> {
        let v = self.view();
        let mut all = vec![Caret {
            cursor: v.cursor,
            anchor: v.anchor,
        }];
        all.extend(v.extra);
        all
    }

    pub(crate) fn has_carets(&self) -> bool {
        !self.doc().extra.is_empty()
    }

    pub(crate) fn clear_carets(&mut self) {
        self.doc_mut().extra.clear();
    }

    /// Runs `op` at every cursor, as one undo step. `op` gets each cursor's
    /// place in the text, counted from the top.
    pub(crate) fn each_caret(&mut self, kind: Kind, mut op: impl FnMut(&mut Self, usize)) {
        if !self.has_carets() {
            return op(self, 0);
        }
        let doc = self.doc_mut();
        let goal = doc.goal;
        let primary = Caret {
            cursor: doc.cursor,
            anchor: doc.anchor,
        };
        let mut all: Vec<(Caret, bool)> = std::iter::once((primary, true))
            .chain(doc.extra.drain(..).map(|c| (c, false)))
            .collect();
        all.sort_by_key(|(c, _)| Reverse(c.start()));
        let before = std::iter::once(primary)
            .chain(all.iter().filter(|(_, p)| !p).map(|(c, _)| *c))
            .collect();
        doc.buffer.begin_at(before, kind);
        let n = all.len();
        let mut done: Vec<(Caret, bool)> = Vec::with_capacity(n);
        let mut primary_goal = goal;
        for (k, (caret, is_primary)) in all.into_iter().enumerate() {
            let doc = self.doc_mut();
            doc.cursor = caret.cursor;
            doc.anchor = caret.anchor;
            doc.goal = if is_primary { goal } else { None };
            let before = doc.buffer.len() as isize;
            op(self, n - 1 - k);
            let doc = self.doc();
            let delta = doc.buffer.len() as isize - before;
            // The cursors already done are later in the text, so they move
            // by what this one added or removed.
            let shift = |p: usize| (p as isize + delta).max(0) as usize;
            for (c, _) in &mut done {
                c.cursor = shift(c.cursor);
                c.anchor = c.anchor.map(shift);
            }
            if is_primary {
                primary_goal = doc.goal;
            }
            done.push((
                Caret {
                    cursor: doc.cursor,
                    anchor: doc.anchor,
                },
                is_primary,
            ));
        }
        let (main, _) = done.iter().find(|(_, p)| *p).copied().unwrap_or(done[0]);
        let mut extra: Vec<Caret> = done
            .into_iter()
            .filter(|(_, p)| !p)
            .map(|(c, _)| c)
            .collect();
        // Cursors that ran into each other become one.
        extra.sort_by_key(|c| c.cursor);
        extra.dedup_by_key(|c| c.cursor);
        extra.retain(|c| c.cursor != main.cursor);
        let doc = self.doc_mut();
        doc.cursor = main.cursor;
        doc.anchor = main.anchor;
        doc.goal = primary_goal;
        let after = std::iter::once(main).chain(extra.iter().copied()).collect();
        doc.buffer.commit_at(after);
        doc.extra = extra;
        doc.extra_version = doc.buffer.version();
    }

    /// Puts the cursors and selections back where an undo or redo says
    /// they were, the primary one first.
    pub(crate) fn put_cursors(&mut self, cursors: &[Caret]) {
        let Some((primary, rest)) = cursors.split_first() else {
            return;
        };
        self.set_cursor(primary.cursor);
        let doc = self.doc_mut();
        let len = doc.buffer.len();
        doc.anchor = primary.anchor.map(|a| a.min(len));
        doc.extra = rest
            .iter()
            .map(|c| Caret {
                cursor: c.cursor.min(len),
                anchor: c.anchor.map(|a| a.min(len)),
            })
            .filter(|c| c.cursor != doc.cursor)
            .collect();
        doc.extra_version = doc.buffer.version();
    }

    /// Adds a cursor at `pos`, which becomes the primary one.
    pub(crate) fn add_caret_at(&mut self, pos: usize) {
        let doc = self.doc_mut();
        if doc.cursor == pos || doc.extra.iter().any(|c| c.cursor == pos) {
            return;
        }
        self.save_cursors();
        let doc = self.doc_mut();
        let old = Caret {
            cursor: doc.cursor,
            anchor: doc.anchor,
        };
        doc.extra.push(old);
        doc.cursor = pos;
        doc.anchor = None;
        doc.goal = None;
        doc.extra_version = doc.buffer.version();
    }

    /// Adds a cursor on the line above the top one, or below the bottom one,
    /// at the primary cursor's column.
    pub(crate) fn add_caret_vertical(&mut self, down: bool) {
        let carets = self.carets();
        let doc = self.doc();
        let lines: Vec<usize> = carets.iter().map(|c| doc.line_col(c.cursor).0).collect();
        let (line, col) = doc.line_col(doc.cursor);
        let goal = doc.goal.unwrap_or_else(|| doc.display_col(line, col));
        let target = if down {
            let last = *lines.iter().max().unwrap_or(&line);
            if last >= doc.last_line() {
                return;
            }
            last + 1
        } else {
            let first = *lines.iter().min().unwrap_or(&line);
            let Some(above) = first.checked_sub(1) else {
                return;
            };
            above
        };
        let pos = doc.pos_at(target, goal);
        self.add_caret_at(pos);
        self.doc_mut().goal = Some(goal);
    }

    /// What Ctrl+D looks for: the selection, or the word at the cursor.
    fn occurrence(&self) -> Option<(usize, usize)> {
        let doc = self.doc();
        if let Some(sel) = doc.selection() {
            return Some(sel);
        }
        let (start, end) = (doc.word_start_at(doc.cursor), doc.word_end_at(doc.cursor));
        (start < end).then_some((start, end))
    }

    /// Selects the word at the cursor, or adds a cursor at the next place
    /// the selected text appears, as VS Code's Ctrl+D does.
    pub(crate) fn add_next_occurrence(&mut self) {
        let Some((start, end)) = self.occurrence() else {
            return;
        };
        if self.doc().selection().is_none() {
            let doc = self.doc_mut();
            doc.anchor = Some(start);
            doc.cursor = end;
            return;
        }
        let needle: Vec<char> = self.doc().slice(start, end).chars().collect();
        let taken: Vec<usize> = self.carets().iter().map(|c| c.start()).collect();
        let hits = self.exact_matches(&needle);
        let after = hits
            .iter()
            .copied()
            .filter(|h| !taken.contains(h))
            .find(|&h| h > start)
            .or_else(|| hits.iter().copied().find(|h| !taken.contains(h)));
        let Some(hit) = after else {
            return self.notify("No more occurrences".into());
        };
        self.add_caret_at(hit + needle.len());
        self.doc_mut().anchor = Some(hit);
    }

    /// Selects every place the selection, or the word at the cursor,
    /// appears.
    pub(crate) fn select_all_occurrences(&mut self) {
        let Some((start, end)) = self.occurrence() else {
            return;
        };
        let needle: Vec<char> = self.doc().slice(start, end).chars().collect();
        let hits = self.exact_matches(&needle);
        let len = needle.len();
        self.save_cursors();
        let doc = self.doc_mut();
        doc.extra = hits
            .iter()
            .filter(|&&h| h != start)
            .map(|&h| Caret {
                cursor: h + len,
                anchor: Some(h),
            })
            .collect();
        doc.anchor = Some(start);
        doc.cursor = end;
        doc.extra_version = doc.buffer.version();
        let n = hits.len();
        self.notify(format!("{n} selections"));
    }

    /// Where `needle` appears, case and all, left to right.
    pub(crate) fn exact_matches(&self, needle: &[char]) -> Vec<usize> {
        if needle.is_empty() || needle.contains(&'\n') {
            return Vec::new();
        }
        self.all_matches(needle, false)
    }

    /// The text at every cursor: the selections, or each cursor's line.
    /// Returns the pieces top to bottom, and whether they're lines.
    pub(crate) fn caret_texts(&self) -> (Vec<String>, bool) {
        let doc = self.doc();
        let mut carets = self.carets();
        carets.sort_by_key(|c| c.start());
        let selections: Vec<(usize, usize)> = carets.iter().filter_map(|c| c.selection()).collect();
        if !selections.is_empty() {
            let texts = selections.iter().map(|&(a, b)| doc.slice(a, b)).collect();
            return (texts, false);
        }
        let mut lines: Vec<usize> = carets.iter().map(|c| doc.line_col(c.cursor).0).collect();
        lines.dedup();
        let texts = lines
            .into_iter()
            .map(|line| {
                let end = if line < doc.last_line() {
                    doc.line_start(line + 1)
                } else {
                    doc.buffer.len()
                };
                let mut text = doc.slice(doc.line_start(line), end);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text
            })
            .collect();
        (texts, true)
    }
}
