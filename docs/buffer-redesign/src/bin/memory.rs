//! Prototype 7: exactly how much memory each way of holding things takes,
//! counted by the allocator. Per file: the rope, facts about each line,
//! and colours, each in a few layouts.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use buffer_prototypes::*;

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        NOW.fetch_add(l.size(), Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        NOW.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        NOW.fetch_add(new, Ordering::Relaxed);
        NOW.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.realloc(p, l, new) }
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Bytes `f`'s result holds, kept alive while counted.
fn held<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = NOW.load(Ordering::Relaxed);
    let out = f();
    let after = NOW.load(Ordering::Relaxed);
    (out, after.saturating_sub(before))
}

fn row(what: &str, bytes: usize, file: usize, per: Option<(usize, &str)>) {
    let mb = bytes as f64 / (1 << 20) as f64;
    let x = bytes as f64 / file as f64;
    let per = per.map_or(String::new(), |(n, unit)| {
        format!("{:>7.2} B/{unit}", bytes as f64 / n.max(1) as f64)
    });
    println!("  {what:<46} {mb:>9.1} MB {x:>7.3}x {per}");
}

/// A token's kind as the editor keeps it today.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct TodayToken {
    start: usize,
    end: usize,
    kind: merlin_syntax::Syntax,
}

/// A line's run of colours: where each starts, from the last, and its
/// kind; it ends where the next starts. Plain text between is a run too.
#[derive(Clone, Copy)]
#[allow(dead_code)]
#[repr(C, packed)]
struct Run {
    gap: u16,
    kind: u8,
}

fn run(name: &str, text: String) {
    let file = text.len();
    println!("{name}: {} MB", file >> 20);
    let (rope, b) = held(|| ropey::Rope::from_str(&text));
    row("ropey rope", b, file, None);
    let (crop, b) = held(|| crop::Rope::from(text.as_str()));
    row("crop rope", b, file, None);
    drop(crop);
    let lines = rope.len_lines();
    let (v, b) = held(|| vec![(0u32, 0u32); lines]);
    row("line facts: width + rows, u32 each", b, file, Some((lines, "line")));
    drop(v);
    let (v, b) = held(|| vec![(0u16, 0u8); lines]);
    row("line facts: u16 width + u8 rows", b, file, Some((lines, "line")));
    drop(v);
    // Only each block's totals; a block's lines measured again when one
    // is needed (see `lines --summaries`).
    let (v, b) = held(|| vec![(0u64, 0u32, 0u32); lines.div_ceil(1024)]);
    row("line facts: block totals only", b, file, Some((lines, "line")));
    drop(v);

    // Colours: a token a word or a run of punctuation.
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        let word = c.is_alphanumeric() || c == '_';
        match (start, word) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
        if !word && !c.is_whitespace() {
            spans.push((i, i + 1));
        }
    }
    let n = spans.len();
    let (v, b) = held(|| {
        spans
            .iter()
            .map(|&(s, e)| TodayToken { start: s, end: e, kind: merlin_syntax::Syntax::Keyword })
            .collect::<Vec<_>>()
    });
    row(&format!("tokens today: (usize, usize, kind), {n}"), b, file, Some((n, "token")));
    drop(v);
    let (v, b) = held(|| {
        spans
            .iter()
            .map(|&(s, e)| (s as u32, (e - s) as u32, 0u8))
            .collect::<Vec<_>>()
    });
    row("tokens in blocks: (u32, u32, kind)", b, file, Some((n, "token")));
    drop(v);
    // Runs: a token and the plain gap before it, 3 bytes each.
    let (v, b) = held(|| {
        let mut out = Vec::with_capacity(n * 2);
        let mut last = 0;
        for &(s, e) in &spans {
            if s > last {
                out.push(Run { gap: (s - last) as u16, kind: 0 });
            }
            out.push(Run { gap: (e - s) as u16, kind: 1 });
            last = e;
        }
        out.shrink_to_fit();
        out
    });
    row("tokens as runs: u16 length + u8 kind", b, file, Some((n, "token")));
    drop(v);
    // Undo after deleting a large part: today the removed text is kept as
    // a String; a clone of the rope from before shares its nodes instead.
    let mut now = rope.clone();
    let half = now.len_chars() / 2;
    let (removed, b) = held(|| now.slice(..half).to_string());
    row("undo of deleting half: removed text as String", b, file, None);
    drop(removed);
    let before = now.clone();
    let (_, b) = held(|| now.remove(..half));
    row("undo of deleting half: rope from before kept", b, file, None);
    drop(before);
    // A small edit in the middle, with the rope from before kept: what the
    // two share.
    let mut now = rope.clone();
    let before = now.clone();
    let (_, b) = held(|| now.insert(now.len_chars() / 2, "x"));
    row("a keystroke with the rope from before kept", b, file, None);
    drop((before, now));
    drop(rope);
}

fn main() {
    run("rust 20MB", rust(20));
    run("prose 85MB", prose(85));
    run("log 342MB", log(342));
}
