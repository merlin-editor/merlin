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
