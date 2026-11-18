//! Line diffs, for reviewing proposed changes and for git markers in the
//! gutter.

use std::time::Duration;

use similar::{DiffOp, DiffTag, TextDiff};

/// How long a diff may take before it settles for a rougher answer, so a
/// huge file can't freeze the editor.
const TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Same,
    Added,
    Removed,
    /// Unchanged lines left out of a review.
    Gap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    /// The line, without its line break. For a gap, what was left out.
    pub text: String,
    /// The line's number in the new text, or the old one for a removed
    /// line, from 0. Unused for gaps.
    pub number: usize,
    /// The line's number in the old text, for unchanged and removed lines.
    pub old_number: usize,
}

fn ops<'a>(old: &'a str, new: &'a str) -> TextDiff<'a, 'a, 'a, str> {
    TextDiff::configure().timeout(TIMEOUT).diff_lines(old, new)
}

/// The changes from `old` to `new`, with `context` unchanged lines around
/// each one and a gap line where the rest are left out.
pub fn unified(old: &str, new: &str, context: usize) -> Vec<DiffLine> {
    let diff = ops(old, new);
    let total = diff.new_slices().len();
    let mut lines = Vec::new();
    let mut shown_to = 0;
    let gap = |hidden: usize| DiffLine {
        kind: LineKind::Gap,
        text: format!(
            "⋯ {hidden} unchanged line{}",
            if hidden == 1 { "" } else { "s" }
        ),
        number: 0,
        old_number: 0,
    };
    for group in diff.grouped_ops(context) {
        let first = group.first().map_or(0, |op| op.new_range().start);
        if first > shown_to {
            lines.push(gap(first - shown_to));
        }
        for op in &group {
            for change in diff.iter_changes(op) {
                let (kind, number) = match change.tag() {
                    similar::ChangeTag::Equal => (LineKind::Same, change.new_index()),
                    similar::ChangeTag::Insert => (LineKind::Added, change.new_index()),
                    similar::ChangeTag::Delete => (LineKind::Removed, change.old_index()),
                };
                let text = change.value().trim_end_matches(['\n', '\r']).to_string();
                lines.push(DiffLine {
                    kind,
                    text,
                    number: number.unwrap_or(0),
                    old_number: change.old_index().unwrap_or(0),
                });
            }
        }
        shown_to = group.last().map_or(shown_to, |op| op.new_range().end);
    }
    if !lines.is_empty() && total > shown_to {
        lines.push(gap(total - shown_to));
    }
    lines
}

/// How a line differs from the last commit, for the gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Added,
    Changed,
    /// Lines were deleted just above this one.
    Removed,
}

/// A sign for each line of `new`, by line number.
/// [`signs`] for the text in a rope, diffing only the lines between those
/// it starts and ends with in common: an edit to a large file costs a pass
/// over its bytes, not a diff of all its lines.
pub fn signs_rope(old: &str, new: &ropey::Rope) -> Vec<Option<Sign>> {
    let (o, n) = (old.as_bytes(), new.len_bytes());
    // The bytes both start with, back to a line's start.
    let mut prefix = 0;
    for chunk in new.chunks() {
        let theirs = &o[prefix.min(o.len())..];
        let same = common_prefix(chunk.as_bytes(), theirs);
        prefix += same;
        if same < chunk.len() {
            break;
        }
    }
    if prefix == o.len() && prefix == n {
        return vec![None; new.len_lines()];
    }
    prefix = o[..prefix]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);
    // The bytes both end with, after the prefix in each.
    let most = o.len().min(n) - prefix;
    let mut suffix = 0;
    let (mut chunks, ..) = new.chunks_at_byte(n);
    while suffix < most
        && let Some(chunk) = chunks.prev()
    {
        let theirs = &o[..o.len() - suffix];
        let same = common_suffix(chunk.as_bytes(), theirs).min(most - suffix);
        suffix += same;
        if same < chunk.len() {
            break;
        }
    }
    // On to a line's start, in both.
    let starts_line = |s: usize| {
        (s == n || new.byte(n - s - 1) == b'\n') && (s == o.len() || o[o.len() - s - 1] == b'\n')
    };
    if suffix > 0 && !starts_line(suffix) {
        let tail = &o[o.len() - suffix..];
        suffix = tail
            .iter()
            .position(|&b| b == b'\n')
            .map_or(0, |i| suffix - i - 1);
    }
    let lines = new.len_lines();
    let mut out = vec![None; lines];
    let at = new.byte_to_line(prefix);
    let old_middle = &old[prefix..o.len() - suffix];
    // One line for one, as typing on a minified file's: no diff needed.
    let old_one = match old_middle.bytes().position(|b| b == b'\n') {
        Some(i) => i + 1 == old_middle.len(),
        None => !old_middle.is_empty(),
    };
    let breaks = new.byte_to_line(n - suffix) - at;
    let new_one = match breaks {
        0 => n - suffix > prefix,
        1 => new.byte(n - suffix - 1) == b'\n',
        _ => false,
    };
    if old_one && new_one {
        out[at] = Some(Sign::Changed);
        return out;
    }
    let middle = new.byte_slice(prefix..n - suffix).to_string();
    for (i, sign) in signs(old_middle, &middle).into_iter().enumerate() {
        if let (Some(sign), Some(slot)) = (sign, out.get_mut(at + i)) {
            *slot = Some(sign);
        }
    }
    out
}

/// How many bytes `a` and `b` start with in common: compared a block at a
/// time, which the compiler makes a memcmp.
/// How many bytes `chunks`, taken in order, start with alike with `b`.
pub(crate) fn common_prefix_chunks<'a>(chunks: impl Iterator<Item = &'a str>, b: &[u8]) -> usize {
    let mut n = 0;
    for chunk in chunks {
        let same = common_prefix(chunk.as_bytes(), &b[n.min(b.len())..]);
        n += same;
        if same < chunk.len() {
            break;
        }
    }
    n
}

/// How many bytes, at most `most`, the rope and `b` end with alike.
pub(crate) fn common_suffix_rope(rope: &ropey::Rope, b: &[u8], most: usize) -> usize {
    let mut n = 0;
    let (mut chunks, ..) = rope.chunks_at_byte(rope.len_bytes());
    while n < most
        && let Some(chunk) = chunks.prev()
    {
        let same = common_suffix(chunk.as_bytes(), &b[..b.len() - n]).min(most - n);
        n += same;
        if same < chunk.len() {
            break;
        }
    }
    n
}

pub(crate) fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let mut i = 0;
    while i + 64 <= n && a[i..i + 64] == b[i..i + 64] {
        i += 64;
    }
    i + a[i..n]
        .iter()
        .zip(&b[i..n])
        .take_while(|(x, y)| x == y)
        .count()
}

/// How many bytes `a` and `b` end with in common.
pub(crate) fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let (a, b) = (&a[a.len() - n..], &b[b.len() - n..]);
    let mut i = 0;
    while i + 64 <= n && a[n - i - 64..n - i] == b[n - i - 64..n - i] {
        i += 64;
    }
    i + a[..n - i]
        .iter()
        .rev()
        .zip(b[..n - i].iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

pub fn signs(old: &str, new: &str) -> Vec<Option<Sign>> {
    let diff = ops(old, new);
    let count = new.split('\n').count();
    let mut signs = vec![None; count];
    let mut mark = |range: std::ops::Range<usize>, sign| {
        for line in range {
            if let Some(slot) = signs.get_mut(line) {
                *slot = Some(sign);
            }
        }
    };
    for op in diff.ops() {
        let new_range = op.new_range();
        match (op.tag(), op) {
            (DiffTag::Equal, _) => {}
            (DiffTag::Insert, _) => mark(new_range, Sign::Added),
            // A longer replacement adds the lines beyond what it replaced.
            (DiffTag::Replace, _) => {
                let changed = new_range.start + op.old_range().len().min(new_range.len());
                mark(new_range.start..changed, Sign::Changed);
                mark(changed..new_range.end, Sign::Added);
            }
            (DiffTag::Delete, DiffOp::Delete { new_index, .. }) => {
                let at = (*new_index).min(count - 1);
                mark(at..at + 1, Sign::Removed)
            }
            (DiffTag::Delete, _) => {}
        }
    }
    signs
}

/// How alike two texts are, by their lines: 1 the same, 0 nothing alike.
pub fn similarity(old: &str, new: &str) -> f32 {
    ops(old, new).ratio()
}

/// Each run of changed lines from `old` to `new`: the lines it had in
/// `old` and has in `new`, by line number. Ops next to each other join, as
/// the gutter marks them as one.
pub fn runs(old: &str, new: &str) -> Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let diff = ops(old, new);
    let mut out: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    for op in diff.ops() {
        if op.tag() == DiffTag::Equal {
            continue;
        }
        let (o, n) = (op.old_range(), op.new_range());
        match out.last_mut() {
            Some((lo, ln)) if lo.end == o.start && ln.end == n.start => {
                lo.end = o.end;
                ln.end = n.end;
            }
            _ => out.push((o, n)),
        }
    }
    out
}

/// Where each line of `old` is in `new`. A changed or deleted line maps to
/// where its replacement starts.
pub fn line_map(old: &str, new: &str) -> Vec<usize> {
    let diff = ops(old, new);
    let mut map = vec![0; old.split('\n').count()];
    for op in diff.ops() {
        let (from, to) = (op.old_range(), op.new_range());
        for (i, line) in from.clone().enumerate() {
            if let Some(slot) = map.get_mut(line) {
                *slot = match op.tag() {
                    DiffTag::Equal => to.start + i,
                    _ => to.start + i.min(to.len().saturating_sub(1)),
                };
            }
        }
    }
    // The empty line after a final line break isn't in the diff.
    if let (Some(last), Some(before)) = (map.len().checked_sub(1), map.len().checked_sub(2)) {
        map[last] = map[last]
            .max(map[before] + 1)
            .min(new.split('\n').count() - 1);
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_from_a_rope_match_a_whole_diff() {
        let base = "one\ntwo\nthree\nfour\nfive\n";
        for new in [
            base,
            "one\ntwo\nTHREE\nfour\nfive\n",
            "one\ntwo\nthree\nnew\nfour\nfive\n",
            "one\ntwo\nfour\nfive\n",
            "ONE\ntwo\nthree\nfour\nfive\n",
            "one\ntwo\nthree\nfour\nfive",
            "one\ntwo\nthree\nfour\nfive\nsix",
            "one\ntwo\nthree\nfour\nfive\nsix\n",
            "zero\none\ntwo\nthree\nfour\nfive\n",
            "",
            "two\nthree\n",
            "one\ntwo\nthree\nfour\nfivex\n",
            "one\r\ntwo\nthree\nfour\nfive\n",
            "one\ntwo\nthree\nthree\nfour\nfive\n",
        ] {
            let rope = ropey::Rope::from_str(new);
            assert_eq!(signs_rope(base, &rope), signs(base, new), "{new:?}");
        }
        // One chunk of a rope at most is a few kilobytes: changes past it.
        // Small enough that the whole diff never meets its time limit, even
        // in a debug build with every test running.
        let big: String = (0..600).map(|i| format!("line {i}\n")).collect();
        let changed = big
            .replace("line 300\n", "changed\n")
            .replace("line 599\n", "");
        let rope = ropey::Rope::from_str(&changed);
        assert_eq!(signs_rope(&big, &rope), signs(&big, &changed));
    }

    fn kinds(lines: &[DiffLine]) -> String {
        lines
            .iter()
            .map(|l| match l.kind {
                LineKind::Same => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
                LineKind::Gap => '~',
            })
            .collect()
    }

    #[test]
    fn unified_keeps_context_and_marks_gaps() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n";
        let lines = unified(old, new, 1);
        assert_eq!(kinds(&lines), "~ -+ ~");
        assert_eq!(lines[0].text, "⋯ 3 unchanged lines");
        assert_eq!((lines[2].text.as_str(), lines[2].number), ("5", 4));
        assert_eq!((lines[3].text.as_str(), lines[3].number), ("five", 4));
    }

    #[test]
    fn identical_texts_have_no_lines() {
        assert!(unified("a\nb\n", "a\nb\n", 3).is_empty());
    }

    #[test]
    fn signs_mark_added_changed_and_removed_lines() {
        let old = "a\nb\nc\nd\n";
        let new = "a\nB\nc\nnew\n";
        let signs = signs(old, new);
        assert_eq!(signs[1], Some(Sign::Changed));
        assert_eq!(signs[0], None);
        assert_eq!(signs[3], Some(Sign::Changed));
        let signs = super::signs("a\nb\nc\n", "a\nc\n");
        assert_eq!(signs[1], Some(Sign::Removed));
        let signs = super::signs("a\n", "a\nb\n");
        assert_eq!(signs[1], Some(Sign::Added));
        let signs = super::signs("a\nb\n", "a\nB\nc\nd\n");
        assert_eq!(
            signs[1..4],
            [Some(Sign::Changed), Some(Sign::Added), Some(Sign::Added)]
        );
    }

    #[test]
    fn line_map_follows_unchanged_lines() {
        let map = line_map("a\nb\nc\nd\n", "new\na\nB\nc\n");
        assert_eq!(map, [1, 2, 3, 4, 4]);
    }
}
