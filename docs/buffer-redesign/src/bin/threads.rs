//! Prototype 10: what more threads buy and cost. A whole-file search on 1
//! to all cores: how long it takes, and how much CPU it uses in all.

use buffer_prototypes::*;
use regex::bytes::Regex;
use ropey::Rope;

/// This process's CPU time so far, user and system, in seconds.
fn cpu() -> f64 {
    let s = std::fs::read_to_string("/proc/self/stat").unwrap();
    let f: Vec<&str> = s[s.rfind(')').unwrap() + 2..].split_whitespace().collect();
    let ticks: f64 = f[11].parse::<f64>().unwrap() + f[12].parse::<f64>().unwrap();
    ticks / 100.0
}

fn search(rope: &Rope, re: &Regex, threads: usize) -> usize {
    let lines = rope.len_lines();
    let cuts: Vec<usize> = (0..=threads)
        .map(|t| if t == threads { rope.len_bytes() } else { rope.line_to_byte(lines * t / threads) })
        .collect();
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let (rope, re) = (rope.clone(), re.clone());
                let (a, b) = (cuts[t], cuts[t + 1]);
                s.spawn(move || {
                    let mut n = 0;
                    let mut carry = Vec::new();
                    for chunk in rope.byte_slice(a..b).chunks() {
                        let bytes = chunk.as_bytes();
                        let Some(nl) = memchr::memrchr(b'\n', bytes) else {
                            carry.extend_from_slice(bytes);
                            continue;
                        };
                        carry.extend_from_slice(&bytes[..=nl]);
                        n += re.find_iter(&carry).count();
                        carry.clear();
                        carry.extend_from_slice(&bytes[nl + 1..]);
                    }
                    n + re.find_iter(&carry).count()
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).sum()
    })
}

fn main() {
    let rope = Rope::from_str(&log(342));
    let re = Regex::new("(?i)status=200").unwrap();
    println!("log 342MB, finding every match, any case");
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let mut seen = None;
    for threads in [1, 2, 4, 8, cores] {
        let (c0, t0) = (cpu(), std::time::Instant::now());
        let n = search(&rope, &re, threads);
        let wall = t0.elapsed().as_secs_f64() * 1e3;
        let used = (cpu() - c0) * 1e3;
        assert!(seen.is_none_or(|s| s == n));
        seen = Some(n);
        println!("  {threads:>2} threads: {wall:>7.0} ms, {used:>7.0} ms of CPU in all");
    }
}
