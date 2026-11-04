//! Selections beyond the arrows: growing and shrinking them by what the
//! code is made of, as JetBrains' Ctrl+W and VS Code's Shift+Alt+Right do;
//! skipping an occurrence; taking back a cursor; box selections; and
//! dragging by words or lines after a double or triple click.

use super::Editor;
use crate::doc::Caret;

/// Brackets and quotes that bound what expanding selects.
const BRACKETS: &[(char, char)] = &[('(', ')'), ('[', ']'), ('{', '}')];
const QUOTES: &[char] = &['"', '\'', '`'];

impl Editor {
    /// Shift+Alt+Right: the smallest thing around the selection that's
    /// bigger than it: the word, a string's or brackets' inside, the
    /// brackets too, the line's text, the whole lines, then the file.
    pub(crate) fn expand_selection(&mut self) {
        self.clear_carets();
        let doc = self.doc();
        let (s, e) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let bigger = self
            .around(s, e)
            .into_iter()
            .filter(|&(a, b)| a <= s && b >= e && b - a > e - s)
            .min_by_key(|&(a, b)| b - a);
        let Some((a, b)) = bigger else { return };
        // Back to this one on shrinking, unless it's where shrinking began.
        if self.expanded.last() != Some(&(s, e)) {
            self.expanded.push((s, e));
        }
        self.expanded.push((a, b));
        let doc = self.doc_mut();
        doc.anchor = Some(a);
        doc.cursor = b;
        doc.goal = None;
    }

    /// Shift+Alt+Left: back to what the selection was before it grew.
    pub(crate) fn shrink_selection(&mut self) {
        let doc = self.doc();
        let now = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        if self.expanded.last() != Some(&now) {
            self.expanded.clear();
            return;
        }
        self.expanded.pop();
        let Some(&(a, b)) = self.expanded.last() else {
            return;
        };
        if self.expanded.len() == 1 {
            self.expanded.clear();
        }
        let doc = self.doc_mut();
        doc.anchor = (a < b).then_some(a);
        doc.cursor = b;
    }

    /// Ranges around `s..e` that expanding may pick from.
    fn around(&self, s: usize, e: usize) -> Vec<(usize, usize)> {
        let doc = self.doc();
        let rope = doc.buffer.rope();
        let mut out = Vec::new();
        let (word_start, word_end) = (doc.word_start_at(s), doc.word_end_at(e));
        if word_start < word_end {
            out.push((word_start, word_end));
        }
        // Brackets around, however far out.
        for &(open, close) in BRACKETS {
            if let Some(o) = unmatched_before(rope, s, open, close)
                && let Some(c) = unmatched_after(rope, e, open, close)
            {
                out.push((o + 1, c));
                out.push((o, c + 1));
            }
        }
        // Quotes around, on the selection's lines.
        let (first, _) = doc.line_col(s);
        let (last, _) = doc.line_col(e);
        let (from, to) = (doc.line_start(first), self.line_end(last));
        for &q in QUOTES {
            let before = (from..s).rev().find(|&p| rope.char(p) == q);
            let after = (e..to).find(|&p| rope.char(p) == q);
            if let (Some(o), Some(c)) = (before, after) {
                out.push((o + 1, c));
                out.push((o, c + 1));
            }
        }
        // The lines' text without its indent, then the whole lines.
        let indent = doc
            .line_chars(first)
            .iter()
            .take_while(|c| c.is_whitespace())
            .count();
        out.push((from + indent, to));
        let next = if last < doc.last_line() {
            doc.line_start(last + 1)
        } else {
            doc.buffer.len()
        };
        out.push((from, next));
        out.push((0, doc.buffer.len()));
        out
    }

    /// Ctrl+K Ctrl+D: the last selection added moves on to the next
    /// occurrence instead.
    pub(crate) fn skip_occurrence(&mut self) {
        let doc = self.doc();
        let Some((start, end)) = doc.selection() else {
            return self.add_next_occurrence();
        };
        let needle: Vec<char> = doc.slice(start, end).chars().collect();
        let taken: Vec<usize> = self.carets().iter().map(|c| c.start()).collect();
        let hits = self.exact_matches(&needle);
        let next = hits
            .iter()
            .copied()
            .filter(|h| !taken.contains(h))
            .find(|&h| h > start)
            .or_else(|| hits.iter().copied().find(|h| !taken.contains(h)));
        let Some(hit) = next else {
            return self.notify("No more occurrences".into());
        };
        let doc = self.doc_mut();
        doc.anchor = Some(hit);
        doc.cursor = hit + needle.len();
    }

    /// Remembers the cursors before more are added, for Ctrl+U.
    pub(crate) fn save_cursors(&mut self) {
        let doc = self.doc();
        let saved = (
            doc.id,
            doc.buffer.version(),
            Caret {
                cursor: doc.cursor,
                anchor: doc.anchor,
            },
            doc.extra.clone(),
        );
        self.cursor_undo.push(saved);
        // Enough to take back a long run of Ctrl+D.
        if self.cursor_undo.len() > 100 {
            self.cursor_undo.remove(0);
        }
    }

    /// Ctrl+U: the cursors as they were before the last one was added.
    pub(crate) fn undo_cursor(&mut self) {
        let doc = self.doc();
        let (id, version) = (doc.id, doc.buffer.version());
        // Saved cursors from before an edit point at the wrong text.
        self.cursor_undo
            .retain(|(i, v, ..)| *i == id && *v == version);
        let Some((_, _, primary, extra)) = self.cursor_undo.pop() else {
            return;
        };
        let doc = self.doc_mut();
        doc.cursor = primary.cursor;
        doc.anchor = primary.anchor;
        doc.extra = extra;
        doc.extra_version = version;
    }

    /// A box selection from where Shift+Alt+drag started to `(line,
    /// column)`: a cursor on each line, each selecting those columns.
    pub(crate) fn select_box(&mut self, to: (usize, usize)) {
        let Some((line0, col0)) = self.box_from else {
            return;
        };
        let (line1, col1) = to;
        let doc = self.doc();
        let lines: Vec<usize> = if line0 <= line1 {
            (line0..=line1).collect()
        } else {
            (line1..=line0).rev().collect()
        };
        let mut carets: Vec<Caret> = lines
            .iter()
            .map(|&l| {
                let (a, b) = (doc.pos_at(l, col0), doc.pos_at(l, col1));
                Caret {
                    cursor: b,
                    anchor: (a != b).then_some(a),
                }
            })
            .collect();
        // The line under the mouse has the primary cursor.
        let primary = carets.pop().expect("a box has a line");
        let doc = self.doc_mut();
        doc.cursor = primary.cursor;
        doc.anchor = primary.anchor;
        doc.extra = carets;
        doc.extra_version = doc.buffer.version();
        doc.goal = None;
    }

    /// The word or line at `pos`, for a double or triple click.
    pub(crate) fn unit_at(&self, pos: usize, clicks: u8) -> (usize, usize) {
        let doc = self.doc();
        if clicks == 2 {
            let (start, end) = (doc.word_start_at(pos), doc.word_end_at(pos));
            if start == end {
                // Not on a word: the character clicked, as VS Code does.
                let end = (pos + 1).min(doc.buffer.len());
                let on_newline = doc.slice(pos, end) == "\n";
                (pos, if on_newline { pos } else { end })
            } else {
                (start, end)
            }
        } else {
            let line = doc.line_col(pos).0;
            let end = if line < doc.last_line() {
                doc.line_start(line + 1)
            } else {
                doc.buffer.len()
            };
            (doc.line_start(line), end)
        }
    }

    /// Dragging after a double or triple click: the selection grows by
    /// words or lines from the one clicked.
    pub(crate) fn drag_units(&mut self, pos: usize) {
        let Some((start, end, clicks)) = self.drag_unit else {
            return;
        };
        let (a, b) = self.unit_at(pos, clicks);
        let doc = self.doc_mut();
        if a < start {
            doc.anchor = Some(end);
            doc.cursor = a;
        } else {
            doc.anchor = Some(start);
            doc.cursor = b.max(end);
        }
    }
}

/// The nearest `open` before `pos` that isn't closed before it.
fn unmatched_before(rope: &ropey::Rope, pos: usize, open: char, close: char) -> Option<usize> {
    const LIMIT: usize = 100_000;
    let mut depth = 0usize;
    let mut chars = rope.chars_at(pos);
    let mut p = pos;
    while let Some(c) = chars.prev() {
        p -= 1;
        if pos - p > LIMIT {
            return None;
        }
        if c == close {
            depth += 1;
        } else if c == open {
            if depth == 0 {
                return Some(p);
            }
            depth -= 1;
        }
    }
    None
}

/// The nearest `close` from `pos` on that wasn't opened after it.
fn unmatched_after(rope: &ropey::Rope, pos: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in rope.chars_at(pos).enumerate().take(100_000) {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(pos + i);
            }
            depth -= 1;
        }
    }
    None
}
