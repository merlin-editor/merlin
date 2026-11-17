//! Prototype 4: whole-file work on a snapshot in another thread while
//! the window's thread keeps editing, and what a plugin pays to keep up
//! with the text.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use buffer_prototypes::*;
use regex::bytes::Regex;
use ropey::Rope;

/// Searches `rope` a chunk at a time, around `first` (the screen's bytes)
/// first, sending matches in batches, and stopping once `latest` says the
/// text it has is no longer the newest.
fn search(
    rope: Rope,
    version: u64,
    latest: Arc<AtomicU64>,
    re: Regex,
    first: std::ops::Range<usize>,
    out: mpsc::Sender<Result<Vec<usize>, &'static str>>,
) {
    let send_range = |a: usize, b: usize| -> bool {
        let slice = rope.byte_slice(a..b);
        let mut batch = Vec::new();
        let mut carry = Vec::new();
        let mut base = a;
        for chunk in slice.chunks() {
            if latest.load(Ordering::Relaxed) != version {
                return false;
            }
            let bytes = chunk.as_bytes();
            let Some(nl) = memchr::memrchr(b'\n', bytes) else {
                carry.extend_from_slice(bytes);
                continue;
            };
            carry.extend_from_slice(&bytes[..=nl]);
            for m in re.find_iter(&carry) {
                batch.push(base + m.start());
            }
            base += carry.len();
            carry.clear();
            carry.extend_from_slice(&bytes[nl + 1..]);
            if batch.len() >= 4096 {
                let _ = out.send(Ok(std::mem::take(&mut batch)));
            }
        }
        for m in re.find_iter(&carry) {
            batch.push(base + m.start());
        }
        let _ = out.send(Ok(batch));
        true
    };
    let len = rope.len_bytes();
    let done = send_range(first.start, first.end)
        && send_range(first.end, len)
        && send_range(0, first.start);
    let _ = out.send(if done { Err("done") } else { Err("stopped") });
}

fn main() {
    let text = log(342);
    let mut rope = Rope::from_str(&text);
    drop(text);
    println!("log 342MB");
    each("take a snapshot (clone the rope)", 10000, |_| {
        std::hint::black_box(rope.clone());
    });

    let re = Regex::new("(?i)status=200").unwrap();
    let latest = Arc::new(AtomicU64::new(1));
    let (tx, rx) = mpsc::channel();
    let screen = rope.line_to_byte(2_000_000)..rope.line_to_byte(2_000_060);
    let start = Instant::now();
    {
        let (snap, latest, re, screen) = (rope.clone(), latest.clone(), re.clone(), screen.clone());
        std::thread::spawn(move || search(snap, 1, latest, re, screen, tx));
    }
    // Meanwhile the window's thread types.
    let mut worst = 0f64;
    let mut typed = 0;
    let mut first_batch = None;
    let mut total = 0;
    loop {
        let t = Instant::now();
        let at = rope.line_to_char(1_000_000 + typed);
        rope.insert(at, "x");
        worst = worst.max(t.elapsed().as_secs_f64() * 1e6);
        typed += 1;
        match rx.try_recv() {
            Ok(Ok(batch)) => {
                if first_batch.is_none() {
                    first_batch = Some((start.elapsed().as_secs_f64() * 1e3, batch.len()));
                }
                total += batch.len();
            }
            Ok(Err(why)) => {
                println!(
                    "  search {why}: {total} matches in {:.1} ms",
                    start.elapsed().as_secs_f64() * 1e3
                );
                break;
            }
            Err(_) => {}
        }
    }
    let (ms, n) = first_batch.unwrap();
    println!("  the screen's {n} matches came after {ms:.3} ms");
    println!("  typed {typed} characters meanwhile; slowest edit {worst:.1} µs");

    // A newer version stops the search.
    let (tx, rx) = mpsc::channel();
    let latest = Arc::new(AtomicU64::new(1));
    {
        let (snap, l, re) = (rope.clone(), latest.clone(), re.clone());
        std::thread::spawn(move || search(snap, 1, l, re, 0..0, tx));
    }
    std::thread::sleep(std::time::Duration::from_millis(30));
    let t = Instant::now();
    latest.store(2, Ordering::Relaxed);
    loop {
        if let Ok(Err(why)) = rx.recv() {
            println!(
                "  {why} {:.3} ms after the text changed",
                t.elapsed().as_secs_f64() * 1e3
            );
            break;
        }
    }

    // What a highlighting plugin pays a keystroke to keep up, today and
    // from the changes since its version.
    for (name, mb) in [("rust 20MB", 20), ("log 342MB", 342)] {
        let text = if mb == 20 { rust(20) } else { log(342) };
        let rope = Rope::from_str(&text);
        let mut mine = text.clone();
        println!("{name}");
        let now = time("today: the whole text as a String", || rope.to_string());
        time("today: find what changed by comparing", || {
            let a = mine.as_bytes();
            let b = now.as_bytes();
            let p = a.iter().zip(b).take_while(|(x, y)| x == y).count();
            let s = a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count();
            (p, s)
        });
        time("changes: apply one to its own String", || mine.insert(mine.len() / 2, 'x'));
        let mut own = rope.clone();
        time("changes: apply one to its own rope", || own.insert(own.len_chars() / 2, "x"));
    }
}
