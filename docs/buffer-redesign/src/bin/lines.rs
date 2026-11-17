//! Prototype 1: facts about each line (display width, rows when wrapped)
//! kept in blocks with totals, updated only where edits touch, against
//! working them out again for the whole file.

use buffer_prototypes::*;
use ropey::Rope;

const TAB: usize = 4;
const BLOCK: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    width: u32,
    rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Block {
    lines: Vec<Line>,
    rows: u64,
    widest: u32,
}

impl Block {
    fn new(lines: Vec<Line>) -> Self {
        let rows = lines.iter().map(|l| l.rows as u64).sum();
        let widest = lines.iter().map(|l| l.width).max().unwrap_or(0);
        Self { lines, rows, widest }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct LineIndex {
    blocks: Vec<Block>,
    cols: usize,
}

/// Width and wrapped rows of one line's text, ASCII a byte at a time.
fn measure<'a>(chunks: impl Iterator<Item = &'a str>, cols: usize) -> Line {
    let (mut d, mut rows, mut row_start, mut last_space) = (0usize, 1u32, 0usize, None::<usize>);
    for chunk in chunks {
        for c in chunk.chars() {
            if c == '\n' || c == '\r' {
                continue;
            }
            let w = if c == '\t' {
                TAB - d % TAB
            } else if c.is_ascii() {
                1
            } else {
                unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)
            };
            if cols > 0 && d + w - row_start > cols {
                rows += 1;
                // Break after the last space on the row, if there is one.
                row_start = match last_space {
                    Some(s) if s > row_start => s,
                    _ => d,
                };
                last_space = None;
            }
            d += w;
            if c == ' ' {
                last_space = Some(d);
            }
        }
    }
    Line { width: d as u32, rows }
}

/// Measures lines as their bytes come, a chunk at a time.
struct Measurer {
    cols: usize,
    d: usize,
    rows: u32,
    row_start: usize,
    last_space: Option<usize>,
}

impl Measurer {
    fn new(cols: usize) -> Self {
        Self { cols, d: 0, rows: 1, row_start: 0, last_space: None }
    }

    #[inline]
    fn push(&mut self, w: usize, space: bool) {
        if self.cols > 0 && self.d + w - self.row_start > self.cols {
            self.rows += 1;
            self.row_start = match self.last_space {
                Some(s) if s > self.row_start => s,
                _ => self.d,
            };
            self.last_space = None;
        }
        self.d += w;
        if space {
            self.last_space = Some(self.d);
        }
    }

    fn end(&mut self) -> Line {
        let line = Line { width: self.d as u32, rows: self.rows };
        *self = Self::new(self.cols);
        line
    }
}

/// Every line's facts in one pass over the text.
fn measure_all(rope: &Rope, cols: usize) -> Vec<Line> {
    let mut out = Vec::with_capacity(rope.len_lines());
    let mut m = Measurer::new(cols);
    for chunk in rope.chunks() {
        let bytes = chunk.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'\n' => {
                    out.push(m.end());
                    i += 1;
                }
                b'\r' => i += 1,
                b'\t' => {
                    let w = TAB - m.d % TAB;
                    m.push(w, false);
                    i += 1;
                }
                0..0x80 => {
                    m.push(1, b == b' ');
                    i += 1;
                }
                _ => {
                    let c = chunk[i..].chars().next().unwrap();
                    m.push(unicode_width::UnicodeWidthChar::width(c).unwrap_or(1), false);
                    i += c.len_utf8();
                }
            }
        }
    }
    out.push(m.end());
    out
}

/// [`measure_all`] split across threads at line starts: each takes a
/// share of the lines from its own cheap clone of the rope.
fn measure_parallel(rope: &Rope, cols: usize) -> Vec<Line> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let lines = rope.len_lines();
    let per = lines.div_ceil(threads);
    let parts: Vec<Vec<Line>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let rope = rope.clone();
                s.spawn(move || {
                    let a = (t * per).min(lines);
                    let b = ((t + 1) * per).min(lines);
                    if a == b {
                        return Vec::new();
                    }
                    let from = rope.line_to_char(a);
                    let to = if b == lines { rope.len_chars() } else { rope.line_to_char(b) };
                    let part = rope.slice(from..to);
                    let mut part = measure_all(&Rope::from(part), cols);
                    // Every part but the last ends with a line break, so
                    // seems to have an empty line after it: the next
                    // part's first.
                    if b != lines {
                        part.pop();
                    }
                    part
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    parts.concat()
}

impl LineIndex {
    fn build_parallel(rope: &Rope, cols: usize) -> Self {
        let lines = measure_parallel(rope, cols);
        Self {
            blocks: lines.chunks(BLOCK).map(|c| Block::new(c.to_vec())).collect(),
            cols,
        }
    }

    fn build_fast(rope: &Rope, cols: usize) -> Self {
        let lines = measure_all(rope, cols);
        Self {
            blocks: lines.chunks(BLOCK).map(|c| Block::new(c.to_vec())).collect(),
            cols,
        }
    }

    fn build(rope: &Rope, cols: usize) -> Self {
        let lines: Vec<Line> = (0..rope.len_lines())
            .map(|l| measure(rope.line(l).chunks(), cols))
            .collect();
        Self {
            blocks: lines.chunks(BLOCK).map(|c| Block::new(c.to_vec())).collect(),
            cols,
        }
    }

    /// Lines `first..first + old` became `first..first + new` in `rope`.
    fn edit(&mut self, rope: &Rope, first: usize, old: usize, new: usize) {
        // The blocks the old lines are in.
        let mut at = 0;
        let mut b0 = 0;
        while b0 + 1 < self.blocks.len() && at + self.blocks[b0].lines.len() <= first {
            at += self.blocks[b0].lines.len();
            b0 += 1;
        }
        let mut b1 = b0;
        let mut end = at + self.blocks[b0].lines.len();
        while b1 + 1 < self.blocks.len() && end < first + old {
            b1 += 1;
            end += self.blocks[b1].lines.len();
        }
        let mut lines: Vec<Line> = self.blocks[b0..=b1]
            .iter()
            .flat_map(|b| b.lines.iter().copied())
            .collect();
        let fresh = (first..first + new).map(|l| measure(rope.line(l).chunks(), self.cols));
        lines.splice(first - at..first - at + old, fresh);
        let rebuilt: Vec<Block> = if lines.len() <= 2 * BLOCK {
            vec![Block::new(lines)]
        } else {
            lines.chunks(BLOCK).map(|c| Block::new(c.to_vec())).collect()
        };
        self.blocks.splice(b0..=b1, rebuilt);
        // An emptied block goes, unless it's the only one.
        if self.blocks.len() > 1 {
            self.blocks.retain(|b| !b.lines.is_empty());
        }
    }

    fn total_rows(&self) -> u64 {
        self.blocks.iter().map(|b| b.rows).sum()
    }

    fn widest(&self) -> u32 {
        self.blocks.iter().map(|b| b.widest).max().unwrap_or(0)
    }

    fn rows_before(&self, line: usize) -> u64 {
        let (mut rows, mut at) = (0, 0);
        for b in &self.blocks {
            if at + b.lines.len() > line {
                return rows + b.lines[..line - at].iter().map(|l| l.rows as u64).sum::<u64>();
            }
            rows += b.rows;
            at += b.lines.len();
        }
        rows
    }

    fn line_at_row(&self, row: u64) -> usize {
        let (mut rows, mut at) = (0, 0);
        for b in &self.blocks {
            if rows + b.rows > row {
                for l in &b.lines {
                    if rows + l.rows as u64 > row {
                        return at;
                    }
                    rows += l.rows as u64;
                    at += 1;
                }
            }
            rows += b.rows;
            at += b.lines.len();
        }
        at.saturating_sub(1)
    }

    fn flat(&self) -> Vec<Line> {
        self.blocks.iter().flat_map(|b| b.lines.iter().copied()).collect()
    }
}

/// Only each block's totals: a block's lines are measured again from the
/// text when one of them is needed, so it costs a few bytes a block.
struct Totals {
    /// Lines, rows and widest line of each block.
    blocks: Vec<(usize, u64, u32)>,
    cols: usize,
}

impl Totals {
    fn sum(rope: &Rope, first: usize, n: usize, cols: usize) -> (usize, u64, u32) {
        let mut rows = 0;
        let mut widest = 0;
        for l in first..first + n {
            let m = measure(rope.line(l).chunks(), cols);
            rows += m.rows as u64;
            widest = widest.max(m.width);
        }
        (n, rows, widest)
    }

    fn from(index: &LineIndex) -> Self {
        Self {
            blocks: index.blocks.iter().map(|b| (b.lines.len(), b.rows, b.widest)).collect(),
            cols: index.cols,
        }
    }

    /// Lines `first..first + old` became `first..first + new`: the blocks
    /// they were in measured again, split if they grew too large.
    fn edit(&mut self, rope: &Rope, first: usize, old: usize, new: usize) {
        let (mut at, mut b0) = (0, 0);
        while b0 + 1 < self.blocks.len() && at + self.blocks[b0].0 <= first {
            at += self.blocks[b0].0;
            b0 += 1;
        }
        let (mut b1, mut end) = (b0, at + self.blocks[b0].0);
        while b1 + 1 < self.blocks.len() && end < first + old {
            b1 += 1;
            end += self.blocks[b1].0;
        }
        let lines = end - at - old + new;
        let mut fresh = Vec::new();
        let mut l = at;
        while l < at + lines {
            let n = if at + lines - l <= 2 * BLOCK { at + lines - l } else { BLOCK };
            fresh.push(Self::sum(rope, l, n, self.cols));
            l += n;
        }
        self.blocks.splice(b0..=b1, fresh);
    }

    fn rows_before(&self, rope: &Rope, line: usize) -> u64 {
        let (mut rows, mut at) = (0, 0);
        for &(n, r, _) in &self.blocks {
            if at + n > line {
                return rows + Self::sum(rope, at, line - at, self.cols).1;
            }
            rows += r;
            at += n;
        }
        rows
    }

    fn widest(&self) -> u32 {
        self.blocks.iter().map(|b| b.2).max().unwrap_or(0)
    }
}

/// One random edit to `rope`: the lines it replaced and how many there are now.
fn random_edit(rope: &mut Rope, rng: &mut Rng) -> (usize, usize, usize) {
    let len = rope.len_chars();
    let at = rng.below(len);
    let first = rope.char_to_line(at);
    match rng.below(4) {
        0 => {
            rope.insert(at, "x");
            (first, 1, 1)
        }
        1 => {
            rope.insert(at, "two\nnew lines\n");
            (first, 1, 3)
        }
        2 => {
            let end = (at + 1 + rng.below(200)).min(len);
            let last = rope.char_to_line(end);
            rope.remove(at..end);
            (first, last - first + 1, 1)
        }
        _ => {
            rope.insert(at, "a much longer line of words that will wrap at the width it is drawn at, and again ");
            (first, 1, 1)
        }
    }
}

fn run(name: &str, text: String, cols: usize) {
    println!("{name}: {} MB, cols {cols}", text.len() >> 20);
    let mut rope = Rope::from_str(&text);
    drop(text);
    let fast = time("build index in one pass over the bytes", || LineIndex::build_fast(&rope, cols));
    let par = time("build index on every core", || LineIndex::build_parallel(&rope, cols));
    let mut index = time("build index a line at a time", || LineIndex::build(&rope, cols));
    assert_eq!(fast.flat(), index.flat(), "one pass measures the same");
    assert_eq!(par.flat(), index.flat(), "on every core measures the same");
    drop(par);
    drop(fast);
    println!("  lines {}, rows {}, widest {}", rope.len_lines(), index.total_rows(), index.widest());
    let mut totals = Totals::from(&index);
    {
        // The totals-only layout through the same edits, on its own copy.
        let mut rope = rope.clone();
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let n = 2000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let (first, old, new) = random_edit(&mut rope, &mut rng);
            totals.edit(&rope, first, old, new);
        }
        let per = t.elapsed().as_secs_f64() * 1e6 / n as f64;
        println!("  {:<48} {per:>10.3} µs each", "totals only: edit + update");
        let lines = rope.len_lines();
        each("totals only: rows before a line", 2000, |i| {
            std::hint::black_box(totals.rows_before(&rope, (i * 7919) % lines));
        });
        let fresh = LineIndex::build_fast(&rope, cols);
        assert_eq!(totals.widest(), fresh.widest());
        assert_eq!(totals.rows_before(&rope, lines), fresh.rows_before(lines));
        assert_eq!(totals.rows_before(&rope, lines / 3), fresh.rows_before(lines / 3));
        println!(
            "  totals only: {} blocks, {:.1} KB, same as rebuilt: yes",
            totals.blocks.len(),
            totals.blocks.len() as f64 * 24.0 / 1024.0
        );
    }
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let n = 2000;
    let t = std::time::Instant::now();
    for _ in 0..n {
        let (first, old, new) = random_edit(&mut rope, &mut rng);
        index.edit(&rope, first, old, new);
    }
    let per = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    println!("  {:<48} {per:>10.3} µs each", "edit + update index");
    let lines = rope.len_lines();
    let total = index.total_rows();
    each("rows before a line", 10000, |i| {
        std::hint::black_box(index.rows_before((i * 7919) % lines));
    });
    each("line at a row", 10000, |i| {
        std::hint::black_box(index.line_at_row((i as u64 * 7919) % total));
    });
    each("total rows + widest", 10000, |_| {
        std::hint::black_box((index.total_rows(), index.widest()));
    });
    let fresh = time("rebuild for comparison", || LineIndex::build(&rope, cols));
    assert_eq!(index.flat(), fresh.flat(), "kept up to date the same as rebuilt");
    println!("  same as rebuilt after {n} random edits: yes");
}

fn main() {
    run("log 342MB, no wrap", log(342), 0);
    run("log 342MB, wrapped", log(342), 40);
    run("prose 85MB, wrapped", prose(85), 30);
    run("rust 20MB, wrapped", rust(20), 20);
}
