//! Prototype 9: the git gutter without keeping git's copy of the file:
//! a hash a line for git's version and for the text now, diffed as
//! numbers, against today's diff of the two texts.

use buffer_prototypes::*;
use merlin_core::diff::{self, Sign};
use ropey::Rope;
use similar::{Algorithm, DiffOp, DiffTag, capture_diff_slices};

/// FNV-1a over a line's bytes, without its line break.
fn hash<'a>(chunks: impl Iterator<Item = &'a str>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for c in chunks {
        for &b in c.as_bytes() {
            if b == b'\n' {
                continue;
            }
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

fn hashes(rope: &Rope) -> Vec<u64> {
    (0..rope.len_lines()).map(|l| hash(rope.line(l).chunks())).collect()
}

/// The gutter from the two lists of hashes, as `diff::signs` marks it.
fn signs(old: &[u64], new: &[u64]) -> Vec<Option<Sign>> {
    let p = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let most = old.len().min(new.len()) - p;
    let s = old.iter().rev().zip(new.iter().rev()).take(most).take_while(|(a, b)| a == b).count();
    let (om, nm) = (&old[p..old.len() - s], &new[p..new.len() - s]);
    let mut out = vec![None; new.len()];
    let count = nm.len() + 1;
    let mut mark = |r: std::ops::Range<usize>, sign| {
        for l in r {
            if let Some(slot) = out.get_mut(p + l) {
                *slot = Some(sign);
            }
        }
    };
    for op in capture_diff_slices(Algorithm::Myers, om, nm) {
        let nr = op.new_range();
        match (op.tag(), op) {
            (DiffTag::Insert, _) => mark(nr, Sign::Added),
            (DiffTag::Replace, _) => {
                let changed = nr.start + op.old_range().len().min(nr.len());
                mark(nr.start..changed, Sign::Changed);
                mark(changed..nr.end, Sign::Added);
            }
            (DiffTag::Delete, DiffOp::Delete { new_index, .. }) => {
                let at = new_index.min(count - 1);
                mark(at..at + 1, Sign::Removed)
            }
            _ => {}
        }
    }
    out
}

fn main() {
    for (name, text) in [("rust 20MB", rust(20)), ("log 342MB", log(342))] {
        println!("{name}");
        let base = text.clone();
        let mut rope = Rope::from_str(&text);
        drop(text);
        let base_hashes = time("hash git's version, a line at a time", || hashes(&rope));
        let mut now = base_hashes.clone();
        println!(
            "  {:<48} {:>10.1} MB against {:.1} MB for the text",
            "git's version kept as hashes",
            base_hashes.len() as f64 * 8.0 / (1 << 20) as f64,
            base.len() as f64 / (1 << 20) as f64
        );
        let mut rng = Rng(11);
        let mut worst_text = 0f64;
        let mut worst_hash = 0f64;
        let mut differ = 0;
        let mut marked = (0, 0);
        let edits = 30;
        for _ in 0..edits {
            // An edit somewhere, and the hashes of the lines it touched.
            let at = rng.below(rope.len_chars() - 10);
            let line = rope.char_to_line(at);
            let lines_before = rope.len_lines();
            if rng.below(2) == 0 {
                rope.insert(at, "changed");
            } else {
                rope.insert(at, "\nnew line\n");
            }
            let added = rope.len_lines() - lines_before;
            let fresh: Vec<u64> = (line..=line + added).map(|l| hash(rope.line(l).chunks())).collect();
            now.splice(line..=line, fresh);
            let t = std::time::Instant::now();
            let by_text = diff::signs_rope(&base, &rope);
            worst_text = worst_text.max(t.elapsed().as_secs_f64() * 1e3);
            let t = std::time::Instant::now();
            let by_hash = signs(&base_hashes, &now);
            worst_hash = worst_hash.max(t.elapsed().as_secs_f64() * 1e3);
            differ += by_text.iter().zip(&by_hash).filter(|(a, b)| a != b).count();
            marked = (
                by_text.iter().filter(|s| s.is_some()).count(),
                by_hash.iter().filter(|s| s.is_some()).count(),
            );
        }
        println!(
            "  lines marked after the last edit: {} from the texts, {} from hashes",
            marked.0, marked.1
        );
        println!("  {:<48} {:>10.2} ms", "slowest gutter after an edit, from the texts", worst_text);
        println!("  {:<48} {:>10.2} ms", "slowest gutter after an edit, from hashes", worst_hash);
        println!("  lines marked differently over {edits} edits: {differ}");
    }
}
