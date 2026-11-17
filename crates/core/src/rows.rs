//! How many rows each line takes when it wraps, kept up to date with
//! edits: a byte a line, sums for each 64 lines, and the rows before each
//! block of 1 024. An edit wraps again only the lines it touched, and the
//! rows before a line, or the line at a row, take a few dozen additions.

use std::collections::BTreeMap;
use std::ops::{ControlFlow, Range};

use ropey::Rope;

use crate::buffer::Change;
use crate::editor::wrap::wrap;
use crate::lang::Inlay;

/// Lines in a sum.
const SUM: usize = 64;
/// Sums in a block.
const BLOCK_SUMS: usize = 16;
/// Lines in a block.
const BLOCK: usize = SUM * BLOCK_SUMS;
/// A line's byte when it takes this many rows or more: its rows are in
/// `tall`.
const TALL: u8 = u8::MAX;
/// A line's byte while it waits to be wrapped again.
const STALE: u8 = 0;

/// Each line's rows when wrapped at `width` columns.
#[derive(Clone, Debug)]
pub struct WrapRows {
    width: usize,
    /// The inlay hints' version the rows were wrapped with.
    inlays: u64,
    /// Each line's rows, [`TALL`] for those in `tall`, [`STALE`] for those
    /// to wrap again.
    lines: Vec<u8>,
    /// The rows of lines that take [`TALL`] or more.
    tall: BTreeMap<usize, usize>,
    /// The rows of each [`SUM`] lines.
    sums: Vec<u32>,
    /// The rows before each block of [`BLOCK`] lines, and in all.
    blocks: Vec<u64>,
    /// Lines with inlay hints, which take room.
    hinted: Vec<usize>,
    /// Whether some lines are [`STALE`].
    stale: bool,
}

/// What wrapping needs to know of a document.
pub struct Source<'a> {
    pub rope: &'a Rope,
    /// The inlay hints, by position, as `Doc::inlays` holds them.
    pub inlays: &'a [Inlay],
    /// The inlay hints' version.
    pub inlays_version: u64,
}

impl Source<'_> {
    /// The lines with inlay hints.
    fn hinted(&self) -> Vec<usize> {
        let end = self.rope.len_chars();
        let mut out: Vec<usize> = self
            .inlays
            .iter()
            .map(|i| self.rope.char_to_line(i.pos.min(end)))
            .collect();
        out.dedup();
        out
    }

    /// The hints on `line`, as the column each is before and its columns.
    fn line_inlays(&self, start: usize, len: usize) -> Vec<(usize, usize)> {
        let first = self.inlays.partition_point(|i| i.pos < start);
        self.inlays[first..]
            .iter()
            .take_while(|i| i.pos <= start + len)
            .map(|i| (i.pos - start, i.cols))
            .collect()
    }
}

/// The rows `text`, a line without its break, takes at `width` columns.
/// A line that can't be wider than the row takes one, without wrapping.
fn count(text: &str, inlays: &[(usize, usize)], width: usize, chars: &mut Vec<char>) -> usize {
    // No character is wider than its bytes, save a tab.
    if inlays.is_empty() && text.len() <= width && memchr::memchr(b'\t', text.as_bytes()).is_none()
    {
        return 1;
    }
    chars.clear();
    chars.extend(text.chars());
    wrap(chars, inlays, width).len()
}

impl WrapRows {
    /// Wraps every line of `src` at `width` columns.
    pub fn new(src: &Source, width: usize) -> Self {
        let lines = src.rope.len_lines();
        let mut rows = Self {
            width,
            inlays: src.inlays_version,
            lines: vec![STALE; lines],
            tall: BTreeMap::new(),
            sums: Vec::new(),
            blocks: Vec::new(),
            hinted: src.hinted(),
            stale: true,
        };
        rows.refresh(src);
        rows
    }

    /// The width the rows are for.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Follows `changes`: the lines they touched wait to be wrapped again
    /// by [`WrapRows::refresh`].
    pub fn update(&mut self, changes: &[Change]) {
        for change in changes {
            let (first, old_last, new_last) = change.lines;
            let old_last = old_last.min(self.lines.len().saturating_sub(1));
            let removed = old_last + 1 - first.min(old_last + 1);
            let added = new_last + 1 - first;
            self.lines
                .splice(first..first + removed, std::iter::repeat_n(STALE, added));
            let shift = |l: usize| {
                if l > old_last {
                    Some(l + added - removed)
                } else if l < first {
                    Some(l)
                } else {
                    None
                }
            };
            self.tall = std::mem::take(&mut self.tall)
                .into_iter()
                .filter_map(|(l, n)| shift(l).map(|l| (l, n)))
                .collect();
            self.hinted = self.hinted.iter().filter_map(|&l| shift(l)).collect();
        }
        self.stale |= !changes.is_empty();
    }

    /// Whether [`WrapRows::refresh`] has anything to do.
    pub fn needs_refresh(&self, src: &Source) -> bool {
        self.stale || src.inlays_version != self.inlays
    }

    /// Wraps again the lines that wait for it, and those whose inlay hints
    /// may have changed, then sums them.
    pub fn refresh(&mut self, src: &Source) {
        if src.inlays_version != self.inlays {
            let hinted = src.hinted();
            for &l in self.hinted.iter().chain(&hinted) {
                if let Some(b) = self.lines.get_mut(l) {
                    *b = STALE;
                    self.stale = true;
                }
            }
            self.hinted = hinted;
            self.inlays = src.inlays_version;
        }
        if !self.stale {
            return;
        }
        debug_assert_eq!(self.lines.len(), src.rope.len_lines());
        let mut chars = Vec::new();
        let mut first = None;
        let mut at = 0;
        while let Some(i) = memchr::memchr(STALE, &self.lines[at..]) {
            let start = at + i;
            let end = self.lines[start..]
                .iter()
                .position(|&b| b != STALE)
                .map_or(self.lines.len(), |n| start + n);
            first.get_or_insert(start);
            self.wrap_lines(src, start..end, &mut chars);
            at = end;
        }
        self.stale = false;
        self.sum_from(first.unwrap_or(0));
    }

    fn wrap_lines(&mut self, src: &Source, range: Range<usize>, chars: &mut Vec<char>) {
        let width = self.width;
        let hinted = &self.hinted;
        let lines = &mut self.lines;
        let tall = &mut self.tall;
        let _ = crate::search::each_line(src.rope, range, |line, byte, text| {
            let inlays = if hinted.binary_search(&line).is_ok() {
                let start = src.rope.byte_to_char(byte);
                src.line_inlays(start, text.chars().count())
            } else {
                Vec::new()
            };
            let n = count(text, &inlays, width, chars);
            if n >= TALL as usize {
                lines[line] = TALL;
                tall.insert(line, n);
            } else {
                lines[line] = n as u8;
                tall.remove(&line);
            }
            ControlFlow::Continue(())
        });
    }

    /// Makes the sums again from the one holding `line` on.
    fn sum_from(&mut self, line: usize) {
        let sums = self.lines.len().div_ceil(SUM);
        let from = (line / BLOCK) * BLOCK_SUMS;
        self.sums.resize(sums, 0);
        for s in from..sums {
            let range = s * SUM..((s + 1) * SUM).min(self.lines.len());
            self.sums[s] = self.sum_lines(range) as u32;
        }
        let blocks = sums.div_ceil(BLOCK_SUMS);
        self.blocks.resize(blocks + 1, 0);
        for b in from / BLOCK_SUMS..blocks {
            let end = ((b + 1) * BLOCK_SUMS).min(sums);
            let rows: u64 = self.sums[b * BLOCK_SUMS..end]
                .iter()
                .map(|&n| n as u64)
                .sum();
            self.blocks[b + 1] = self.blocks[b] + rows;
        }
    }

    /// The rows of `lines`, one by one.
    fn sum_lines(&self, lines: Range<usize>) -> usize {
        let mut rows: usize = self.lines[lines.clone()].iter().map(|&b| b as usize).sum();
        if self.lines[lines.clone()].contains(&TALL) {
            for (_, &n) in self.tall.range(lines) {
                rows += n - TALL as usize;
            }
        }
        rows
    }

    /// The rows `line` takes.
    pub fn rows(&self, line: usize) -> usize {
        match self.lines.get(line) {
            Some(&TALL) => self.tall[&line],
            Some(&n) => n as usize,
            None => 1,
        }
    }

    /// The rows before `line`; past the end, all of them.
    pub fn before(&self, line: usize) -> usize {
        let line = line.min(self.lines.len());
        let block = line / BLOCK;
        let mut rows = self.blocks[block] as usize;
        let first = block * BLOCK_SUMS;
        let sum = line / SUM;
        rows += self.sums[first..sum]
            .iter()
            .map(|&n| n as usize)
            .sum::<usize>();
        rows + self.sum_lines(sum * SUM..line)
    }

    /// All the rows.
    pub fn total(&self) -> usize {
        *self.blocks.last().unwrap_or(&0) as usize
    }

    /// The line whose rows hold `row`, and its rows before it; past the
    /// end, the last line.
    pub fn line_at(&self, row: usize) -> (usize, usize) {
        let lines = self.lines.len();
        if lines == 0 {
            return (0, 0);
        }
        let row = row.min(self.total().saturating_sub(1)) as u64;
        let block = self.blocks.partition_point(|&r| r <= row).saturating_sub(1);
        let mut seen = self.blocks[block] as usize;
        let mut line = block * BLOCK;
        let row = row as usize;
        for &n in &self.sums[block * BLOCK_SUMS..] {
            let n = n as usize;
            if seen + n > row {
                break;
            }
            seen += n;
            line += SUM;
        }
        while line + 1 < lines {
            let n = self.rows(line);
            if seen + n > row {
                break;
            }
            seen += n;
            line += 1;
        }
        (line, seen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    fn source(rope: &Rope) -> Source<'_> {
        Source {
            rope,
            inlays: &[],
            inlays_version: 0,
        }
    }

    fn text() -> String {
        let mut s = String::new();
        for i in 0..3000 {
            match i % 6 {
                0 => s.push_str(&"word ".repeat(i % 50)),
                1 => s.push_str("\tshort"),
                2 => s.push_str(&"x".repeat(i % 400)),
                3 => s.push_str("ünï cödé 漢字 ".repeat(i % 7).as_str()),
                _ => s.push_str("plain"),
            }
            s.push('\n');
        }
        s.push_str(&"long ".repeat(400));
        s
    }

    /// Each line wrapped as the editor does, one at a time.
    fn by_hand(rope: &Rope, width: usize) -> Vec<usize> {
        (0..rope.len_lines())
            .map(|l| {
                let line = rope.line(l);
                let chars: Vec<char> = line.chars().filter(|&c| c != '\n' && c != '\r').collect();
                wrap(&chars, &[], width).len()
            })
            .collect()
    }

    fn check(rows: &WrapRows, want: &[usize]) {
        let mut before = 0;
        for (line, &n) in want.iter().enumerate() {
            assert_eq!(rows.rows(line), n, "line {line}");
            assert_eq!(rows.before(line), before, "before line {line}");
            for r in before..before + n {
                assert_eq!(rows.line_at(r), (line, before), "row {r}");
            }
            before += n;
        }
        assert_eq!(rows.total(), before);
    }

    #[test]
    fn rows_are_each_lines_wrap() {
        let rope = Rope::from_str(&text());
        for width in [1, 7, 40, 120] {
            let rows = WrapRows::new(&source(&rope), width);
            check(&rows, &by_hand(&rope, width));
        }
    }

    #[test]
    fn rows_follow_edits() {
        let mut buffer = Buffer::new(&text(), None);
        let mut rows = WrapRows::new(&source(buffer.rope()), 30);
        let mut seed = 3u64;
        for _ in 0..200 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = buffer.len();
            let at = (seed >> 33) as usize % len;
            let removed = ((seed >> 20) as usize % 300).min(len - at);
            let inserted = [
                "",
                "a much longer piece of text here ",
                "\n",
                "x\ny\n\n",
                "\r",
            ][(seed >> 8) as usize % 5];
            buffer.begin(at, crate::history::Kind::Other);
            buffer.apply(crate::edit::Edit {
                at,
                removed: buffer.rope().slice(at..at + removed).to_string(),
                inserted: inserted.into(),
            });
            buffer.commit(at);
            rows.update(&buffer.take_changes());
            rows.refresh(&source(buffer.rope()));
            assert_eq!(rows.lines.len(), buffer.rope().len_lines());
            let want = by_hand(buffer.rope(), 30);
            assert_eq!(rows.total(), want.iter().sum::<usize>());
        }
        check(&rows, &by_hand(buffer.rope(), 30));
    }

    #[test]
    fn inlay_hints_take_room() {
        let rope = Rope::from_str("let mix = 1\nlet other = 2\n");
        let hints = [Inlay {
            pos: 7,
            label: ": f32".into(),
            kind: crate::lang::InlayKind::Type,
            cols: 6,
        }];
        let src = Source {
            rope: &rope,
            inlays: &hints,
            inlays_version: 1,
        };
        let mut rows = WrapRows::new(&src, 14);
        assert_eq!(rows.rows(0), 2);
        let src = Source {
            rope: &rope,
            inlays: &[],
            inlays_version: 2,
        };
        rows.refresh(&src);
        assert_eq!(rows.rows(0), 1);
    }
}
