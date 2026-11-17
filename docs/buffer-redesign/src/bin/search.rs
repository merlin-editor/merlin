//! Prototype 2: finding every match in a large file. Today's way copies
//! each line into a Vec<char>; the others search the rope's bytes.

use buffer_prototypes::*;
use regex::bytes::Regex;
use ropey::Rope;

/// Today's way, as `all_matches` and `line_matches` do it.
fn current(rope: &Rope, query: &str, fold: bool) -> usize {
    let q: Vec<char> = query.chars().collect();
    let same = |a: char, b: char| {
        if fold {
            a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    };
    let mut count = 0;
    for l in 0..rope.len_lines() {
        let chars: Vec<char> = rope.line(l).chars().collect();
        let mut i = 0;
        while i + q.len() <= chars.len() {
            if chars[i..i + q.len()].iter().zip(&q).all(|(a, b)| same(*a, *b)) {
                count += 1;
                i += q.len();
            } else {
                i += 1;
            }
        }
    }
    count
}

/// A literal, case and all, with memchr's searcher over each chunk; the
/// end of one chunk is carried into the next so matches across them count.
fn memmem(rope: &Rope, query: &str, out: &mut Vec<usize>) {
    let finder = memchr::memmem::Finder::new(query.as_bytes());
    let keep = query.len() - 1;
    let mut carry: Vec<u8> = Vec::new();
    let mut base = 0usize; // byte offset of carry[0]
    for chunk in rope.chunks() {
        carry.extend_from_slice(chunk.as_bytes());
        let mut last_end = 0;
        for at in finder.find_iter(&carry) {
            if at >= last_end {
                out.push(base + at);
                last_end = at + query.len();
            }
        }
        let cut = carry.len().saturating_sub(keep).max(last_end.min(carry.len()));
        base += cut;
        carry.drain(..cut);
    }
}

/// A regular expression over whole lines: each chunk up to its last line
/// break is searched, and the rest carried on to the next.
fn regex_lines(rope: &Rope, re: &Regex, from: usize, to: usize, out: &mut Vec<usize>) {
    let mut carry: Vec<u8> = Vec::new();
    let mut base = from;
    let slice = rope.byte_slice(from..to);
    for chunk in slice.chunks() {
        let bytes = chunk.as_bytes();
        let Some(nl) = memchr::memrchr(b'\n', bytes) else {
            carry.extend_from_slice(bytes);
            continue;
        };
        let (whole, rest) = bytes.split_at(nl + 1);
        let text: &[u8] = if carry.is_empty() {
            whole
        } else {
            carry.extend_from_slice(whole);
            &carry
        };
        for m in re.find_iter(text) {
            out.push(base + m.start());
        }
        base += text.len();
        carry.clear();
        carry.extend_from_slice(rest);
    }
    for m in re.find_iter(&carry) {
        out.push(base + m.start());
    }
}

/// [`regex_lines`] on every core, each taking a share of the bytes cut at
/// line breaks.
fn regex_parallel(rope: &Rope, re: &Regex) -> Vec<usize> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let len = rope.len_bytes();
    let lines = rope.len_lines();
    let cuts: Vec<usize> = (0..=threads)
        .map(|t| {
            if t == threads {
                len
            } else {
                rope.line_to_byte(lines * t / threads)
            }
        })
        .collect();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let rope = rope.clone();
                let re = re.clone();
                let (a, b) = (cuts[t], cuts[t + 1]);
                s.spawn(move || {
                    let mut out = Vec::new();
                    regex_lines(&rope, &re, a, b, &mut out);
                    out
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

fn run(name: &str, text: String, query: &str) {
    println!("{name}: {} MB, finding {query:?}", text.len() >> 20);
    let rope = Rope::from_str(&text);
    drop(text);
    let today = time("today: a Vec<char> a line, case folded", || current(&rope, query, true));
    let mut hits = Vec::new();
    time("memchr literal over chunks, exact case", || memmem(&rope, query, &mut hits));
    let exact = hits.len();
    let folded = Regex::new(&format!("(?i){}", regex::escape(query))).unwrap();
    let mut rhits = Vec::new();
    time("regex over whole lines, case folded", || {
        regex_lines(&rope, &folded, 0, rope.len_bytes(), &mut rhits)
    });
    let par = time("regex over whole lines on every core", || regex_parallel(&rope, &folded));
    let exact_re = Regex::new(&regex::escape(query)).unwrap();
    let mut ehits = Vec::new();
    regex_lines(&rope, &exact_re, 0, rope.len_bytes(), &mut ehits);
    println!(
        "  matches: today {today}, regex {}, on every core {}, memchr {exact} (regex exact {})",
        rhits.len(),
        par.len(),
        ehits.len()
    );
    assert_eq!(today, rhits.len());
    assert_eq!(rhits, par);
    assert_eq!(hits, ehits);
    // Positions in the editor are characters: what turning matches into
    // them costs, for the ones a screen shows and for all of them.
    time("first 1000 to char positions", || {
        hits.iter().take(1000).map(|&b| rope.byte_to_char(b)).sum::<usize>()
    });
    time("all to char positions", || hits.iter().map(|&b| rope.byte_to_char(b)).sum::<usize>());
    let first_screen = time("matches in one screen (50 lines) only", || {
        let (a, b) = (rope.line_to_byte(1000), rope.line_to_byte(1050));
        let mut out = Vec::new();
        regex_lines(&rope, &folded, a, b, &mut out);
        out.len()
    });
    println!("  in that screen: {first_screen}");
}

fn main() {
    run("log 342MB", log(342), "status=200");
    run("log 342MB, rare", log(342), "nothing like this");
    run("prose 85MB", prose(85), "quick");
    run("rust 20MB", rust(20), "println");
}
