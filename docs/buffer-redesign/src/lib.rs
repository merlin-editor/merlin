//! Shared helpers for the prototypes.
use std::time::Instant;

pub fn time<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let out = f();
    println!("  {what:<48} {:>10.2} ms", t.elapsed().as_secs_f64() * 1e3);
    out
}

/// Per-call time of `f` over `n` calls, in microseconds.
pub fn each(what: &str, n: usize, mut f: impl FnMut(usize)) {
    let t = Instant::now();
    for i in 0..n {
        f(i);
    }
    let us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    println!("  {what:<48} {us:>10.3} µs each");
}

pub fn log(mb: usize) -> String {
    let line = "2026-11-17T10:00:00Z INFO request served path=/api/x status=200 took=3ms\n";
    line.repeat((mb << 20) / line.len())
}

pub fn prose(mb: usize) -> String {
    let line = "the quick brown fox jumps over the lazy dog 0123456789\n";
    line.repeat((mb << 20) / line.len())
}

pub fn rust(mb: usize) -> String {
    let block = "fn main() {\n    let x = vec![1, 2, 3];\n    println!(\"{x:?}\");\n}\n";
    block.repeat((mb << 20) / block.len())
}

/// A small deterministic random number source.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

pub fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: f64 = s.split_whitespace().nth(1).and_then(|p| p.parse().ok()).unwrap_or(0.0);
    pages * 4096.0 / (1 << 20) as f64
}
