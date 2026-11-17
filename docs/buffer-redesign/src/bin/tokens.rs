//! Prototype 3: syntax colours. Today every token is moved on every edit
//! and each one a plugin sends is placed by copying its line; here,
//! tokens kept in blocks by offset into the block, and placed by the
//! rope's own UTF-16 and byte counts.

use buffer_prototypes::*;
use merlin_core::buffer::Change;
use merlin_core::layers::{Layer, Layers, Token};
use merlin_syntax::{Syntax, TextEdit};
use ropey::Rope;

/// A token a word or a run of punctuation, as a highlighter gives roughly.
fn tokenise(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.chars().enumerate() {
        let word = c.is_alphanumeric() || c == '_';
        match (start, word) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
        if !word && !c.is_whitespace() {
            out.push((i, i + 1));
        }
    }
    out
}

const NO_EDIT: TextEdit = TextEdit { start_byte: 0, old_end_byte: 0, new_end_byte: 0, start: (0, 0), old_end: (0, 0), new_end: (0, 0) };
const TOKENS_PER_BLOCK: usize = 512;

/// Tokens in blocks, each block a run of the text; inside one, tokens are
/// counted from its start, so an edit moves only its own block's tokens.
struct Blocks {
    blocks: Vec<TokBlock>,
}

struct TokBlock {
    /// Characters the block covers.
    len: usize,
    tokens: Vec<(u32, u32, Syntax)>,
}

impl Blocks {
    fn new(tokens: &[(usize, usize)], total: usize) -> Self {
        let mut blocks = Vec::new();
        let mut at = 0;
        for chunk in tokens.chunks(TOKENS_PER_BLOCK) {
            let end = chunk.last().unwrap().1;
            blocks.push(TokBlock {
                len: end - at,
                tokens: chunk
                    .iter()
                    .map(|&(a, b)| ((a - at) as u32, (b - at) as u32, Syntax::Keyword))
                    .collect(),
            });
            at = end;
        }
        blocks.push(TokBlock { len: total - at, tokens: Vec::new() });
        Self { blocks }
    }

    /// `removed` characters at `pos` replaced by `inserted`.
    fn edit(&mut self, pos: usize, removed: usize, inserted: usize) {
        let mut at = 0;
        let mut i = 0;
        while i + 1 < self.blocks.len() && at + self.blocks[i].len < pos {
            at += self.blocks[i].len;
            i += 1;
        }
        // Removals across blocks take from each in turn.
        let mut left = removed;
        let mut rel = pos - at;
        let mut j = i;
        loop {
            let b = &mut self.blocks[j];
            let here = left.min(b.len - rel.min(b.len));
            let add = if j == i { inserted } else { 0 };
            let map = |p: u32| -> u32 {
                let p = p as usize;
                (if p >= rel + here {
                    p - here + add
                } else {
                    p.min(rel)
                }) as u32
            };
            for t in &mut b.tokens {
                t.0 = map(t.0);
                t.1 = map(t.1).max(t.0);
            }
            b.tokens.retain(|t| t.0 < t.1);
            b.len = b.len - here + add;
            left -= here;
            if left == 0 || j + 1 == self.blocks.len() {
                break;
            }
            j += 1;
            rel = 0;
        }
    }

    fn spans(&self, range: std::ops::Range<usize>) -> Vec<(usize, usize, Syntax)> {
        let mut out = Vec::new();
        let mut at = 0;
        for b in &self.blocks {
            if at >= range.end {
                break;
            }
            if at + b.len > range.start {
                for &(s, e, k) in &b.tokens {
                    let (s, e) = (at + s as usize, at + e as usize);
                    if e > range.start && s < range.end {
                        out.push((s, e, k));
                    }
                }
            }
            at += b.len;
        }
        out
    }
}

fn run(name: &str, text: String) {
    let rope = Rope::from_str(&text);
    let found = tokenise(&text);
    println!("{name}: {} MB, {} tokens", text.len() >> 20, found.len());

    // Placing what a plugin sends: (line, UTF-16 column) pairs, as now.
    let utf16: Vec<(u32, u32)> = found
        .iter()
        .map(|&(a, _)| {
            let line = rope.char_to_line(a);
            let col = rope.char_to_utf16_cu(a) - rope.char_to_utf16_cu(rope.line_to_char(line));
            (line as u32, col as u32)
        })
        .collect();
    let doc = merlin_core::doc::Doc::new(1, &text, None);
    let sample = utf16.len().min(if rope.len_lines() < 10 { 40 } else { 20_000 });
    let t = std::time::Instant::now();
    let today: Vec<usize> = utf16[..sample].iter().map(|&(l, c)| doc.pos_at_utf16(l, c)).collect();
    let per = t.elapsed().as_secs_f64() / sample as f64;
    println!(
        "  {:<48} {:>10.2} ms (from {sample}: {:.3} µs each)",
        "today: place all, copying each one's line",
        per * utf16.len() as f64 * 1e3,
        per * 1e6
    );
    let by_rope: Vec<usize> = time("place all with the rope's UTF-16 count", || {
        utf16
            .iter()
            .map(|&(l, c)| {
                let start = rope.char_to_utf16_cu(rope.line_to_char(l as usize));
                rope.utf16_cu_to_char(start + c as usize)
            })
            .collect()
    });
    assert_eq!(today, by_rope[..sample]);
    let bytes: Vec<usize> = found.iter().map(|&(a, _)| rope.char_to_byte(a)).collect();
    time("place all from byte offsets", || {
        bytes.iter().map(|&b| rope.byte_to_char(b)).sum::<usize>()
    });
    println!("  {:<48} {:>10.2} ms", "place all if kept as byte offsets", 0.0);

    // Keeping them as the text changes.
    let tokens: Vec<Token> = found
        .iter()
        .map(|&(start, end)| Token { start, end, kind: Syntax::Keyword })
        .collect();
    let mut layers = Layers::default();
    time("today: set the tokens (sorted)", || layers.set(Layer::Base, None, tokens));
    let mut blocks = time("blocks: build", || Blocks::new(&found, rope.len_chars()));
    let mut rng = Rng(7);
    let len = rope.len_chars();
    let edits: Vec<(usize, usize, usize)> = (0..200)
        .map(|_| {
            let at = rng.below(len - 100);
            match rng.below(3) {
                0 => (at, 0, 1),
                1 => (at, 1, 0),
                _ => (at, rng.below(50), rng.below(50)),
            }
        })
        .collect();
    each("today: move every token for an edit", edits.len(), |i| {
        let (at, removed, inserted) = edits[i];
        layers.map(&Change { at, removed, inserted, edit: NO_EDIT });
        layers.tidy();
    });
    each("blocks: move a block's tokens for an edit", edits.len(), |i| {
        let (at, removed, inserted) = edits[i];
        blocks.edit(at, removed, inserted);
    });
    let probe = len / 2..len / 2 + 6000;
    let a = layers.spans(&probe);
    let b = blocks.spans(probe.clone());
    let norm = |v: Vec<(usize, usize, Syntax)>| -> Vec<(usize, usize)> {
        let mut v: Vec<_> = v.into_iter().filter(|t| t.0 >= probe.start && t.1 <= probe.end).map(|t| (t.0, t.1)).collect();
        v.sort();
        v
    };
    assert_eq!(norm(a), norm(b), "the same tokens after the same edits");
    println!("  same tokens on a screen after {} edits: yes", edits.len());
    each("today: tokens for a screen", 1000, |i| {
        std::hint::black_box(layers.spans(&(i * 997 % len..i * 997 % len + 6000)));
    });
    each("blocks: tokens for a screen", 1000, |i| {
        std::hint::black_box(blocks.spans(i * 997 % len..i * 997 % len + 6000));
    });
}

fn main() {
    if std::env::args().nth(1).is_none() {
        run("rust 20MB", rust(20));
        run("prose 85MB", prose(85));
    }
    let json = format!("[{}]", "{\"a\":1,\"b\":\"text\"},".repeat((5 << 20) / 16));
    run("json one line 5MB", json);
}
