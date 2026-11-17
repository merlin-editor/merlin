//! Searching the text where it lies: the rope's lines are read in place,
//! never copied into characters, and nothing is kept per match. What's
//! kept is a count for each block of lines ([`Found`]), so the total, the
//! scrollbar's marks and Find next can skip the blocks with none, and an
//! edit counts again only the blocks it touched.

use std::cell::{Cell, RefCell};
use std::ops::{ControlFlow, Range};
use std::rc::Rc;

use memchr::memmem;
use ropey::Rope;

use crate::buffer::Change;

/// Lines in a block of counts.
const BLOCK: usize = 1024;
/// Lines in a band of a block: a block marks which of its 64 bands have
/// a match.
const BAND: usize = BLOCK / 64;
/// Up to this many lines the scrollbar marks each line with a match;
/// beyond it, each band with one, which is finer than a pixel.
const EXACT_MARKS: usize = 65_536;
/// About the most marks the scrollbar is given.
const MOST_MARKS: usize = 8_192;

/// How the find box, Vim's search or Ctrl+D match text, a line at a time.
#[derive(Clone)]
pub struct Matcher {
    pattern: Pattern,
    whole_word: bool,
    /// Only matches inside this range of characters count.
    pub scope: Option<(usize, usize)>,
    /// What tells one matcher from another, for the counts kept.
    key: String,
    /// Its matches never reach past a line's end, so many lines can be
    /// searched at once: a query typed as it is, not a regular expression.
    plain: bool,
    /// The query as typed and whether case is ignored, when it isn't a
    /// regular expression.
    typed: Option<(String, bool)>,
}

#[derive(Clone)]
enum Pattern {
    Exact(Box<memmem::Finder<'static>>),
    Regex(regex::Regex),
}

impl Matcher {
    /// A regular expression's matches.
    pub fn regex(re: regex::Regex) -> Self {
        Self {
            key: format!("r{}", re.as_str()),
            pattern: Pattern::Regex(re),
            whole_word: false,
            scope: None,
            plain: false,
            typed: None,
        }
    }

    /// Exactly `needle`, case and all.
    pub fn exact(needle: &str) -> Self {
        Self::plain(needle, false, false, None)
    }

    /// `query` as it's typed, with case ignored when `fold`, matching only
    /// whole words when `whole_word`.
    pub fn plain(query: &str, fold: bool, whole_word: bool, scope: Option<(usize, usize)>) -> Self {
        let pattern = if fold {
            // Escaped, a query always makes a regular expression.
            let re = regex::Regex::new(&format!("(?i){}", regex::escape(query)))
                .unwrap_or_else(|_| regex::Regex::new("$^").expect("matches nothing"));
            Pattern::Regex(re)
        } else {
            Pattern::Exact(Box::new(memmem::Finder::new(query.as_bytes()).into_owned()))
        };
        Self {
            key: format!("p{}{}{query}", fold as u8, whole_word as u8),
            pattern,
            whole_word,
            scope,
            plain: !query.chars().any(ends_line),
            typed: Some((query.to_string(), fold)),
        }
    }

    /// The same, counting only matches inside `scope`.
    pub fn with_scope(mut self, scope: Option<(usize, usize)>) -> Self {
        self.scope = scope;
        self
    }

    /// Whether every match of this one is inside a match of `old`, so only
    /// the lines `old` found need searching: the query grew as it's typed.
    fn narrows(&self, old: &Matcher) -> bool {
        let (Some((new, new_fold)), Some((was, was_fold))) = (&self.typed, &old.typed) else {
            return false;
        };
        if old.whole_word || old.scope != self.scope {
            return false;
        }
        match was_fold {
            // Folded as the regex crate does it, which plain lower case
            // matches only for ASCII.
            true => was.is_ascii() && new.to_lowercase().contains(&was.to_ascii_lowercase()),
            false => !new_fold && new.contains(was.as_str()),
        }
    }

    /// Whether case is ignored.
    pub fn folds(&self) -> bool {
        match &self.pattern {
            Pattern::Exact(_) => false,
            Pattern::Regex(re) => re.as_str().starts_with("(?i)"),
        }
    }

    fn key(&self) -> (String, Option<(usize, usize)>) {
        (self.key.clone(), self.scope)
    }

    /// Calls `f` with each match in `line`, a line's text without its line
    /// break, as a range of bytes: left to right, none empty, none
    /// overlapping.
    pub fn each(
        &self,
        line: &str,
        mut f: impl FnMut(Range<usize>) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        let word = |r: &Range<usize>| !self.whole_word || whole_word(line, r);
        match &self.pattern {
            Pattern::Exact(finder) => {
                let len = finder.needle().len();
                if len == 0 {
                    return ControlFlow::Continue(());
                }
                let hay = line.as_bytes();
                let mut at = 0;
                while let Some(i) = finder.find(&hay[at..]) {
                    let r = at + i..at + i + len;
                    at = r.end;
                    if word(&r) {
                        f(r)?;
                    }
                }
            }
            Pattern::Regex(re) => {
                for m in re.find_iter(line) {
                    if !m.is_empty() && word(&m.range()) {
                        f(m.range())?;
                    }
                }
            }
        }
        ControlFlow::Continue(())
    }

    /// The matches in a line's characters, as where each starts and how
    /// long it is. `start` is where the line starts in the document, for
    /// the scope.
    pub fn line(&self, chars: &[char], start: usize) -> Vec<(usize, usize)> {
        let text: String = chars.iter().collect();
        let mut found = Vec::new();
        let mut cols = Cols::new(&text);
        let _ = self.each(&text, |r| {
            let (m, len) = cols.of(r);
            if self.in_scope(start + m, len) {
                found.push((m, len));
            }
            ControlFlow::Continue(())
        });
        found
    }

    /// What the match at `at` in `chars` is replaced with: `with`, and for
    /// a regular expression with `$1` and the like filled in.
    pub fn replacement(&self, chars: &[char], at: usize, with: &str) -> String {
        let Pattern::Regex(re) = &self.pattern else {
            return with.to_string();
        };
        // A query with case ignored is a regular expression too, but its
        // replacement is as typed.
        if self.key.starts_with('p') {
            return with.to_string();
        }
        let text: String = chars.iter().collect();
        let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
        let mut out = String::new();
        if let Some(caps) = re.captures_at(&text, byte) {
            caps.expand(with, &mut out);
        }
        out
    }

    fn in_scope(&self, at: usize, len: usize) -> bool {
        self.scope.is_none_or(|(a, b)| at >= a && at + len <= b)
    }

    /// The lines the scope reaches, or all of them.
    fn lines(&self, rope: &Rope) -> Range<usize> {
        let count = rope.len_lines();
        match self.scope {
            Some((a, b)) => {
                let end = rope.len_chars();
                rope.char_to_line(a.min(end))..rope.char_to_line(b.min(end)) + 1
            }
            None => 0..count,
        }
    }

    /// Every match, left to right, as where each starts and how long it
    /// is, in characters.
    pub fn all(&self, rope: &Rope, found: Option<&Found>) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (lines, count) in spans(rope, self.lines(rope), found) {
            if count == Some(0) {
                continue;
            }
            let _ = self.matches_in(rope, lines, |_, at, len| {
                out.push((at, len));
                ControlFlow::Continue(())
            });
        }
        out
    }

    /// Calls `f` with each match in `lines` that's in the scope: its line,
    /// where it starts and how long it is, in characters.
    fn matches_in(
        &self,
        rope: &Rope,
        lines: Range<usize>,
        mut f: impl FnMut(usize, usize, usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        if self.plain {
            return self.plain_matches_in(rope, lines, f);
        }
        each_line(rope, lines, |line, byte, text| {
            let mut line_char = None;
            let mut cols = Cols::new(text);
            self.each(text, |r| {
                let start = *line_char.get_or_insert_with(|| rope.byte_to_char(byte));
                let (m, len) = cols.of(r);
                if self.in_scope(start + m, len) {
                    f(line, start + m, len)?;
                }
                ControlFlow::Continue(())
            })
        })
    }

    /// As [`Matcher::matches_in`], searching many lines at once: the gaps
    /// between matches give their lines and characters.
    fn plain_matches_in(
        &self,
        rope: &Rope,
        lines: Range<usize>,
        mut f: impl FnMut(usize, usize, usize) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        use ropey::str_utils::{byte_to_char_idx, byte_to_line_idx};
        let mut joined = String::new();
        each_piece(rope, lines, &mut joined, |first, byte, text| {
            let (mut last, mut line, mut ch) = (0, first, None);
            self.each(text, |r| {
                let gap = &text[last..r.start];
                last = r.start;
                line += byte_to_line_idx(gap, gap.len());
                let at = match ch {
                    Some(c) => c + byte_to_char_idx(gap, gap.len()),
                    None => rope.byte_to_char(byte + r.start),
                };
                ch = Some(at);
                let len = byte_to_char_idx(&text[r.clone()], r.len());
                if self.in_scope(at, len) {
                    f(line, at, len)?;
                }
                ControlFlow::Continue(())
            })
        })
    }

    /// The first match at or after `from` that `take` accepts, wrapping
    /// around to the start; it ends where it began.
    pub fn next(
        &self,
        rope: &Rope,
        from: usize,
        found: Option<&Found>,
        mut take: impl FnMut(usize) -> bool,
    ) -> Option<(usize, usize)> {
        let from = from.min(rope.len_chars());
        let line = rope.char_to_line(from);
        let lines = self.lines(rope);
        let mut look = |range: Range<usize>, want: &dyn Fn(usize) -> bool| {
            let mut hit = None;
            for (span, count) in spans(rope, range, found) {
                if count == Some(0) {
                    continue;
                }
                let _ = self.matches_in(rope, span, |_, at, len| {
                    if want(at) && take(at) {
                        hit = Some((at, len));
                        return ControlFlow::Break(());
                    }
                    ControlFlow::Continue(())
                });
                if hit.is_some() {
                    break;
                }
            }
            hit
        };
        look(line.max(lines.start)..lines.end, &|at| at >= from)
            .or_else(|| look(lines.start..(line + 1).min(lines.end), &|at| at < from))
    }

    /// The last match before `before`, wrapping around to the end.
    pub fn previous(
        &self,
        rope: &Rope,
        before: usize,
        found: Option<&Found>,
    ) -> Option<(usize, usize)> {
        let before = before.min(rope.len_chars());
        let line = rope.char_to_line(before);
        let lines = self.lines(rope);
        let look = |range: Range<usize>, want: &dyn Fn(usize) -> bool| {
            for (span, count) in spans(rope, range, found).into_iter().rev() {
                if count == Some(0) {
                    continue;
                }
                let mut last = None;
                let _ = self.matches_in(rope, span, |_, at, len| {
                    if want(at) {
                        last = Some((at, len));
                    }
                    ControlFlow::Continue(())
                });
                if last.is_some() {
                    return last;
                }
            }
            None
        };
        look(lines.start..(line + 1).min(lines.end), &|at| at < before)
            .or_else(|| look(line.max(lines.start)..lines.end, &|at| at >= before))
    }
}

/// Where a line's matches start, as characters, for byte ranges given left
/// to right.
struct Cols<'a> {
    text: &'a str,
    byte: usize,
    col: usize,
}

impl<'a> Cols<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            byte: 0,
            col: 0,
        }
    }

    /// The column `r` starts at and its length, in characters.
    fn of(&mut self, r: Range<usize>) -> (usize, usize) {
        self.col += self.text[self.byte..r.start].chars().count();
        self.byte = r.start;
        (self.col, self.text[r].chars().count())
    }
}

/// Whether `r` in `line` is a whole word: no word character just before
/// or after.
fn whole_word(line: &str, r: &Range<usize>) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !line[..r.start].chars().next_back().is_some_and(word)
        && !line[r.end..].chars().next().is_some_and(word)
}

/// Calls `f` with each of `lines`: its number, the byte it starts at, and
/// its text without the line break, as `Buffer::line_len` counts it. A line
/// that lies in one of the rope's chunks is read where it is.
pub fn each_line(
    rope: &Rope,
    lines: Range<usize>,
    mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let end = lines.end.min(rope.len_lines());
    if lines.start >= end {
        return ControlFlow::Continue(());
    }
    let mut byte = rope.line_to_byte(lines.start);
    let mut joined = String::new();
    for (n, line) in (lines.start..end).zip(rope.lines_at(lines.start)) {
        let len = line.len_bytes();
        let text = match line.as_str() {
            Some(text) => text,
            None => {
                joined.clear();
                joined.extend(line.chunks());
                &joined
            }
        };
        let text = text
            .strip_suffix('\n')
            .map_or(text, |t| t.strip_suffix('\r').unwrap_or(t));
        f(n, byte, text)?;
        byte += len;
    }
    ControlFlow::Continue(())
}

/// Whether the rope ends a line at `c`.
fn ends_line(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// About how many bytes of whole lines [`each_piece`] gives at once.
const PIECE: usize = 64 << 10;

/// Calls `f` with `lines` a run of whole lines at a time, their breaks
/// and all: the first line's number, the byte it starts at, and the text.
fn each_piece(
    rope: &Rope,
    lines: Range<usize>,
    joined: &mut String,
    mut f: impl FnMut(usize, usize, &str) -> ControlFlow<()>,
) -> ControlFlow<()> {
    let end = lines.end.min(rope.len_lines());
    let mut line = lines.start;
    while line < end {
        let byte = rope.line_to_byte(line);
        let reach = (byte + PIECE).min(rope.len_bytes());
        let next = (rope.byte_to_line(reach) + 1).min(end);
        let slice = rope.byte_slice(byte..rope.line_to_byte(next));
        let text = match slice.as_str() {
            Some(text) => text,
            None => {
                joined.clear();
                joined.extend(slice.chunks());
                joined.as_str()
            }
        };
        f(line, byte, text)?;
        line = next;
    }
    ControlFlow::Continue(())
}

/// `lines` in the blocks `found` keeps, with each one's count, or in plain
/// blocks with no count when there's none.
fn spans(
    rope: &Rope,
    lines: Range<usize>,
    found: Option<&Found>,
) -> Vec<(Range<usize>, Option<usize>)> {
    let end = lines.end.min(rope.len_lines());
    let mut out = Vec::new();
    match found.filter(|f| f.scope.is_none()) {
        Some(found) => {
            let mut start = 0;
            for b in &found.blocks {
                let block = start..start + b.lines as usize;
                start = block.end;
                let piece = block.start.max(lines.start)..block.end.min(end);
                if !piece.is_empty() {
                    out.push((piece, Some(b.count as usize)));
                }
            }
        }
        None => {
            let mut at = lines.start;
            while at < end {
                let to = (at + BLOCK).min(end);
                out.push((at..to, None));
                at = to;
            }
        }
    }
    out
}

/// A block's lines and how many matches they have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Block {
    lines: u32,
    count: u32,
    /// Which of the block's bands of [`BAND`] lines have a match.
    bands: u64,
    /// Counted from text that changed since: to count again.
    stale: bool,
}

/// What a search keeps about a document: a count for each block of lines,
/// kept up to date with edits.
#[derive(Clone)]
pub struct Found {
    key: (String, Option<(usize, usize)>),
    matcher: Matcher,
    scope: Option<(usize, usize)>,
    /// The text's version the counts are for.
    version: u64,
    blocks: Vec<Block>,
    total: usize,
    /// The scrollbar's marks, made when first asked for.
    marks: RefCell<Option<Rc<Vec<usize>>>>,
    /// The last match asked about and which it is, from 1.
    current: Cell<Option<(usize, usize, bool)>>,
}

impl Found {
    /// Counts `matcher`'s matches in `rope`, at text version `version`.
    pub fn new(matcher: &Matcher, rope: &Rope, version: u64) -> Self {
        let mut found = Self {
            key: matcher.key(),
            matcher: matcher.clone(),
            scope: matcher.scope,
            version,
            blocks: vec![Block {
                lines: rope.len_lines() as u32,
                count: 0,
                bands: 0,
                stale: true,
            }],
            total: 0,
            marks: RefCell::new(None),
            current: Cell::new(None),
        };
        found.recount(rope);
        found
    }

    /// The counts for `matcher`, a query grown from this one's, from the
    /// bands of lines this one found matches in; `None` if it isn't one.
    pub fn narrowed(&self, matcher: &Matcher) -> Option<Self> {
        (matcher.narrows(&self.matcher) && self.scope.is_none()).then_some(())?;
        Some(Self {
            key: matcher.key(),
            matcher: matcher.clone(),
            scope: matcher.scope,
            version: self.version,
            blocks: self.blocks.clone(),
            total: self.total,
            marks: RefCell::new(None),
            current: Cell::new(None),
        })
    }

    /// As [`Found::narrowed`], searching `rope` again in the bands that
    /// had a match.
    pub fn narrow(&self, matcher: &Matcher, rope: &Rope) -> Option<Self> {
        let mut found = self.narrowed(matcher)?;
        let mut start = 0;
        for b in &mut found.blocks {
            let block = start;
            start += b.lines as usize;
            if b.count == 0 {
                continue;
            }
            let mut bands = b.bands;
            (b.count, b.bands) = (0, 0);
            // Each run of bands with matches, searched at once.
            while bands != 0 {
                let first = bands.trailing_zeros() as usize;
                let run = (bands >> first).trailing_ones() as usize;
                bands &= !(u64::MAX >> (64 - run) << first);
                let from = block + first * BAND;
                let to = (block + (first + run) * BAND).min(start);
                let _ = found.matcher.matches_in(rope, from..to, |line, _, _| {
                    b.count += 1;
                    b.bands |= 1 << ((line - block) / BAND).min(63);
                    ControlFlow::Continue(())
                });
            }
        }
        found.total = found.blocks.iter().map(|b| b.count as usize).sum();
        Some(found)
    }

    /// Whether these are counts for text version `version`.
    pub fn is_at(&self, version: u64) -> bool {
        self.version == version
    }

    /// Whether these are the counts for `matcher` at text version `version`.
    pub fn is_for(&self, matcher: &Matcher, version: u64) -> bool {
        self.version == version && self.key == matcher.key()
    }

    /// How many matches there are.
    pub fn total(&self) -> usize {
        self.total
    }

    /// Follows `changes`, made to the text now in `rope`, now at version
    /// `version`: only the blocks they touched are counted again. Counts
    /// for a scope are dropped instead, as the scope moves; `false` then.
    pub fn update(&mut self, changes: &[Change], rope: &Rope, version: u64) -> bool {
        if self.scope.is_some() {
            return false;
        }
        for change in changes {
            let (first, old_last, new_last) = change.lines;
            let mut start = 0;
            let mut i = 0;
            while i + 1 < self.blocks.len() && start + self.blocks[i].lines as usize <= first {
                start += self.blocks[i].lines as usize;
                i += 1;
            }
            let mut j = i;
            let mut end = start + self.blocks[i].lines as usize;
            while j + 1 < self.blocks.len() && end <= old_last {
                j += 1;
                end += self.blocks[j].lines as usize;
            }
            let lines: usize = self.blocks[i..=j].iter().map(|b| b.lines as usize).sum();
            let merged = Block {
                lines: (lines + new_last - old_last) as u32,
                count: 0,
                bands: 0,
                stale: true,
            };
            self.blocks.splice(i..=j, [merged]);
        }
        self.version = version;
        self.recount(rope);
        true
    }

    /// Counts the stale blocks again, split into blocks of [`BLOCK`] lines.
    fn recount(&mut self, rope: &Rope) {
        let mut out = Vec::with_capacity(self.blocks.len());
        let mut start = 0;
        for b in std::mem::take(&mut self.blocks) {
            let lines = b.lines as usize;
            if !b.stale {
                out.push(b);
                start += lines;
                continue;
            }
            let mut at = start;
            start += lines;
            while at < start {
                let to = (at + BLOCK).min(start);
                let mut block = Block {
                    lines: (to - at) as u32,
                    count: 0,
                    bands: 0,
                    stale: false,
                };
                let _ = self.matcher.matches_in(rope, at..to, |line, _, _| {
                    block.count += 1;
                    let band = (line - at) / BAND;
                    block.bands |= 1 << band.min(63);
                    ControlFlow::Continue(())
                });
                out.push(block);
                at = to;
            }
        }
        self.total = out.iter().map(|b| b.count as usize).sum();
        self.blocks = out;
        self.marks.replace(None);
        self.current.set(None);
    }

    /// Which match starts at `at`, from 1, if one does.
    pub fn index_of(&self, rope: &Rope, at: usize) -> Option<usize> {
        let (count, on) = self.through(rope, at);
        on.then_some(count)
    }

    /// How many matches start at or before `at`, and whether one starts
    /// there. The blocks before it are counted from what's kept; only its
    /// own block is searched.
    pub fn through(&self, rope: &Rope, at: usize) -> (usize, bool) {
        if let Some((pos, count, on)) = self.current.get()
            && pos == at
        {
            return (count, on);
        }
        let line = rope.char_to_line(at.min(rope.len_chars()));
        let mut count = 0;
        let mut on = false;
        for (span, n) in spans(rope, 0..line + 1, Some(self)) {
            let n = n.unwrap_or(0);
            if span.end <= line || n == 0 {
                count += n;
                continue;
            }
            let _ = self.matcher.matches_in(rope, span, |_, pos, _| {
                if pos > at {
                    return ControlFlow::Break(());
                }
                count += 1;
                on = pos == at;
                ControlFlow::Continue(())
            });
        }
        self.current.set(Some((at, count, on)));
        (count, on)
    }

    /// The lines the scrollbar marks: each line with a match in a file of
    /// up to [`EXACT_MARKS`] lines, else each band with one, at most about
    /// [`MOST_MARKS`].
    pub fn marks(&self, rope: &Rope) -> Rc<Vec<usize>> {
        if let Some(marks) = self.marks.borrow().as_ref() {
            return marks.clone();
        }
        let lines = rope.len_lines();
        let mut out = Vec::new();
        let mut start = 0;
        let gap = (lines / MOST_MARKS).max(1);
        for b in &self.blocks {
            let block = start;
            start += b.lines as usize;
            if b.count == 0 {
                continue;
            }
            for band in (0..64).filter(|band| b.bands & (1 << band) != 0) {
                let from = block + band * BAND;
                let to = (from + BAND).min(start);
                if lines <= EXACT_MARKS {
                    let _ = self.matcher.matches_in(rope, from..to, |line, _, _| {
                        if out.last() != Some(&line) {
                            out.push(line);
                        }
                        ControlFlow::Continue(())
                    });
                } else if out.last().is_none_or(|&last| from >= last + gap) {
                    out.push(from);
                }
            }
        }
        let out = Rc::new(out);
        self.marks.replace(Some(out.clone()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn all_plain(text: &str, query: &str, fold: bool) -> Vec<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        let q: Vec<char> = query.chars().collect();
        let mut out = Vec::new();
        let mut start = 0;
        for line in text.split('\n') {
            let line_chars: Vec<char> = line.trim_end_matches('\r').chars().collect();
            let mut i = 0;
            while i + q.len() <= line_chars.len() {
                let same = line_chars[i..i + q.len()]
                    .iter()
                    .zip(&q)
                    .all(|(a, b)| a == b || fold && a.to_lowercase().eq(b.to_lowercase()));
                if same {
                    out.push((start + i, q.len()));
                    i += q.len();
                } else {
                    i += 1;
                }
            }
            start += line.chars().count() + 1;
        }
        assert!(start >= chars.len());
        out
    }

    fn sample() -> String {
        let mut s = String::new();
        for i in 0..5000 {
            match i % 7 {
                0 => s.push_str("fn main() { let ünïcode = 1; }\n"),
                1 => s.push_str("    Main loop over MAIN things\r\n"),
                2 => s.push('\n'),
                3 => s.push_str("mainmainmain\n"),
                _ => s.push_str(&format!("line {i} nothing here\n")),
            }
        }
        s.push_str("last main");
        s
    }

    #[test]
    fn matches_are_those_of_a_walk_over_each_lines_characters() {
        let text = sample();
        let rope = Rope::from_str(&text);
        for (query, fold) in [
            ("main", false),
            ("main", true),
            ("ü", false),
            ("ïc", true),
            ("mainmain", false),
        ] {
            let m = Matcher::plain(query, fold, false, None);
            let want = all_plain(&text, query, fold);
            assert_eq!(m.all(&rope, None), want, "{query}");
            let found = Found::new(&m, &rope, 0);
            assert_eq!(found.total(), want.len());
            assert_eq!(m.all(&rope, Some(&found)), want);
        }
    }

    #[test]
    fn next_and_previous_wrap_around() {
        let text = sample();
        let rope = Rope::from_str(&text);
        let m = Matcher::plain("main", true, false, None);
        let all = m.all(&rope, None);
        let found = Found::new(&m, &rope, 0);
        for from in [0, 1, 37, 5000, 60_000, rope.len_chars()] {
            let want = all.iter().find(|h| h.0 >= from).or(all.first()).copied();
            assert_eq!(m.next(&rope, from, None, |_| true), want);
            assert_eq!(m.next(&rope, from, Some(&found), |_| true), want);
            let want = all
                .iter()
                .rev()
                .find(|h| h.0 < from)
                .or(all.last())
                .copied();
            assert_eq!(m.previous(&rope, from, None), want);
            assert_eq!(m.previous(&rope, from, Some(&found)), want);
        }
        for (i, h) in all.iter().enumerate().step_by(97) {
            assert_eq!(found.index_of(&rope, h.0), Some(i + 1));
            assert_eq!(found.index_of(&rope, h.0 + 1), None);
        }
    }

    #[test]
    fn counts_follow_edits() {
        let mut buffer = Buffer::new(&sample(), None);
        let m = Matcher::plain("main", false, false, None);
        let mut found = Found::new(&m, buffer.rope(), buffer.version());
        let mut seed = 7u64;
        for _ in 0..300 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = buffer.len();
            let at = (seed >> 33) as usize % len;
            let removed = ((seed >> 20) as usize % 40).min(len - at);
            let inserted = ["", "main", "\n", "x\nmain\n\nma", "in"][(seed >> 8) as usize % 5];
            buffer.begin(at, crate::history::Kind::Other);
            buffer.apply(crate::edit::Edit {
                at,
                removed: buffer.rope().slice(at..at + removed).to_string(),
                inserted: inserted.into(),
            });
            buffer.commit(at);
            let changes = buffer.take_changes();
            assert!(found.update(&changes, buffer.rope(), buffer.version()));
            let fresh = Found::new(&m, buffer.rope(), buffer.version());
            assert_eq!(found.total(), fresh.total());
            assert_eq!(found.total(), m.all(buffer.rope(), None).len());
            assert_eq!(*found.marks(buffer.rope()), *fresh.marks(buffer.rope()));
            assert!(
                found
                    .blocks
                    .iter()
                    .all(|b| b.lines as usize <= BLOCK && !b.stale)
            );
        }
    }

    #[test]
    fn many_lines_at_once_give_the_lines_the_rope_counts() {
        let mut text = String::new();
        for i in 0..20_000 {
            let end = ["\n", "\r\n", "\r", "\u{2028}", "\u{85}"][i % 5];
            text.push_str(&format!("{} ab{end}", "é".repeat(i % 9)));
        }
        let rope = Rope::from_str(&text);
        let plain = Matcher::plain("ab", false, true, None);
        let by_line = Matcher::regex(regex::Regex::new(r"\bab\b").unwrap());
        assert!(plain.plain && !by_line.plain);
        let mut hits = Vec::new();
        let _ = plain.matches_in(&rope, 0..rope.len_lines(), |line, at, len| {
            assert_eq!(line, rope.char_to_line(at));
            assert_eq!(rope.slice(at..at + len), "ab");
            hits.push((at, len));
            ControlFlow::Continue(())
        });
        assert_eq!(hits.len(), 20_000);
        assert_eq!(hits, by_line.all(&rope, None));
    }

    #[test]
    fn a_growing_query_counts_from_the_last_ones_bands() {
        let rope = Rope::from_str(&sample());
        let steps = [
            ("m", true),
            ("ma", true),
            ("mai", true),
            ("Main", false),
            ("Main l", false),
        ];
        let mut found = Found::new(&Matcher::plain("m", true, false, None), &rope, 0);
        for (query, fold) in steps.into_iter().skip(1) {
            let m = Matcher::plain(query, fold, false, None);
            found = found.narrow(&m, &rope).expect(query);
            let fresh = Found::new(&m, &rope, 0);
            assert_eq!(found.total(), fresh.total(), "{query}");
            assert_eq!(found.blocks, fresh.blocks, "{query}");
        }
        let exact = Matcher::plain("Main", false, false, None);
        let found = Found::new(&exact, &rope, 0);
        assert!(
            found
                .narrow(&Matcher::plain("main x", true, false, None), &rope)
                .is_none()
        );
        assert!(
            found
                .narrow(&Matcher::plain("Mai", false, false, None), &rope)
                .is_none()
        );
        let word = Found::new(&Matcher::plain("Main", false, true, None), &rope, 0);
        assert!(
            word.narrow(&Matcher::plain("Mainx", false, false, None), &rope)
                .is_none()
        );
    }

    #[test]
    fn whole_words_and_scopes() {
        let rope = Rope::from_str("a cat, cats and a cat_\ncat\n");
        let m = Matcher::plain("cat", false, true, None);
        assert_eq!(m.all(&rope, None), vec![(2, 3), (23, 3)]);
        let m = Matcher::plain("cat", false, false, Some((3, 30)));
        assert_eq!(m.all(&rope, None), vec![(7, 3), (18, 3), (23, 3)]);
    }
}
