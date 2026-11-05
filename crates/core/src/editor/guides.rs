//! What VS Code draws over code to help read it: indent guides, with the
//! current block's brighter; bracket pairs coloured by depth; and other uses
//! of the word under the cursor.

use std::ops::Range;

use merlin_syntax::Syntax;

use super::Editor;
use crate::doc::{Doc, advance};

/// Columns per indent level.
const INDENT: usize = 4;
/// Lines looked at around a blank line or the cursor's block.
const REACH: usize = 200;
/// Lines scanned back for bracket depth; further up starts again at 0.
const BRACKET_REACH: usize = 5000;

/// What a cached bracket depth is for: document, version and line.
pub(crate) type DepthKey = (u64, u64, usize);

/// What the text area overlays on the lines it shows.
pub(crate) struct Guides {
    /// Indent guide columns per visible line, from the first visible one.
    pub levels: Vec<usize>,
    /// The guide of the block with the cursor: its column and lines.
    pub active: Option<(usize, Range<usize>)>,
    /// Bracket depth where the first visible line starts, if brackets are
    /// coloured.
    pub depth: Option<usize>,
    /// The word under the cursor, if it should be marked elsewhere.
    pub word: Option<Vec<char>>,
}

impl Guides {
    pub fn is_guide(&self, line: usize, first: usize, col: usize) -> bool {
        col.is_multiple_of(INDENT) && self.levels.get(line - first).is_some_and(|&l| col < l)
    }

    pub fn is_active(&self, line: usize, col: usize) -> bool {
        self.active
            .as_ref()
            .is_some_and(|(c, lines)| *c == col && lines.contains(&line))
    }
}

/// A line's indent in display columns, or `None` if it's blank.
pub(crate) fn indent(doc: &Doc, line: usize) -> Option<usize> {
    let chars = doc.line_chars(line);
    let mut d = 0;
    for &c in &chars {
        match c {
            ' ' | '\t' => d += advance(c, d),
            '\n' | '\r' => return None,
            _ => return Some(d),
        }
    }
    None
}

/// The guide level of a line: its indent, or for a blank line the lesser
/// of the lines around it, so guides run through gaps in a block.
fn level(doc: &Doc, line: usize) -> usize {
    if let Some(i) = indent(doc, line) {
        return i;
    }
    let last = doc.last_line();
    let up = (line.saturating_sub(REACH)..line)
        .rev()
        .find_map(|l| indent(doc, l));
    let down = (line + 1..=(line + REACH).min(last)).find_map(|l| indent(doc, l));
    up.unwrap_or(0).min(down.unwrap_or(0))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    pub(crate) fn guides(&self, visible: Range<usize>) -> Guides {
        let doc = self.doc();
        let review = doc.review.is_some();
        let levels = if review {
            Vec::new()
        } else {
            visible.clone().map(|l| level(doc, l)).collect()
        };
        Guides {
            levels,
            active: if review { None } else { self.active_guide() },
            depth: if review || !self.bracket_colours {
                None
            } else {
                self.bracket_depth(visible.start)
            },
            word: self.word_at_cursor(),
        }
    }

    /// The cursor's block: the guide just left of its line, or of the lines
    /// it opens if the next line is indented further.
    fn active_guide(&self) -> Option<(usize, Range<usize>)> {
        let doc = self.doc();
        let v = self.view();
        let cur = doc.line_col(v.cursor).0;
        let here = level(doc, cur);
        let last = doc.last_line();
        let next = (cur + 1..=(cur + REACH).min(last)).find_map(|l| indent(doc, l));
        let col = if next.is_some_and(|n| n > here) {
            here
        } else {
            here.checked_sub(INDENT)?
        };
        let inside = |l: usize| level(doc, l) > col;
        let mut start = cur;
        while start > 0 && cur - start < REACH && inside(start - 1) {
            start -= 1;
        }
        let mut end = cur;
        while end < last && end - cur < REACH && inside(end + 1) {
            end += 1;
        }
        Some((col, start..end + 1))
    }

    /// Bracket depth at the start of `line`, counting brackets that aren't
    /// in strings or comments. `None` for files without syntax colours.
    pub(crate) fn bracket_depth(&self, line: usize) -> Option<usize> {
        let doc = self.doc();
        if !doc.has_colours() {
            return None;
        }
        let key = (doc.id, doc.buffer.version(), line);
        if let Some((k, depth)) = self.bracket_cache.get()
            && k == key
        {
            return Some(depth);
        }
        let from = line.saturating_sub(BRACKET_REACH);
        let rope = doc.buffer.rope();
        let mut skip: Vec<Range<usize>> = doc
            .spans(from..line)
            .into_iter()
            .filter(|(_, _, s)| matches!(s, Syntax::String | Syntax::Comment))
            .map(|(a, b, _)| a..b)
            .collect();
        skip.sort_by_key(|r| r.start);
        let (start, end) = (doc.line_start(from), doc.line_start(line));
        let mut depth = 0usize;
        let mut k = 0;
        for (i, c) in rope.chars_at(start).take(end - start).enumerate() {
            let pos = start + i;
            while k < skip.len() && skip[k].end <= pos {
                k += 1;
            }
            if skip.get(k).is_some_and(|r| r.contains(&pos)) {
                continue;
            }
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        self.bracket_cache.set(Some((key, depth)));
        Some(depth)
    }

    /// The word the cursor is in or just after, when nothing's selected.
    fn word_at_cursor(&self) -> Option<Vec<char>> {
        let doc = self.doc();
        let v = self.view();
        if self.focus != super::Focus::Editor || v.selection().is_some() || doc.review.is_some() {
            return None;
        }
        let (line, col) = doc.line_col(v.cursor);
        let chars = doc.line_chars(line);
        let at = |i: usize| chars.get(i).copied().is_some_and(is_word);
        let mut a = col;
        if !at(a) && a > 0 && at(a - 1) {
            a -= 1;
        }
        if !at(a) {
            return None;
        }
        while a > 0 && at(a - 1) {
            a -= 1;
        }
        let mut b = a;
        while at(b) {
            b += 1;
        }
        Some(chars[a..b].to_vec())
    }
}

/// Where `word` appears in `chars` as a whole word.
pub(crate) fn word_matches(chars: &[char], word: &[char]) -> Vec<usize> {
    let n = word.len();
    (0..chars.len().saturating_sub(n - 1))
        .filter(|&i| {
            i + n <= chars.len()
                && chars[i..i + n] == *word
                && (i == 0 || !is_word(chars[i - 1]))
                && chars.get(i + n).is_none_or(|&c| !is_word(c))
        })
        .collect()
}
