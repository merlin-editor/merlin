//! Git's version of an open file, kept without a second copy of it: a
//! rope cloned from the buffer's, with only the lines that differ put
//! back, so the two share every unchanged piece of text. Which lines are
//! the same in both is kept as runs. An edit cuts the runs it touches, and
//! only the gaps between runs are diffed again, so the gutter costs what
//! the edits touched and never gives up on a large file.

use std::ops::Range;
use std::time::{Duration, Instant};

use ropey::Rope;
use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};

use crate::buffer::Change;
use crate::diff::Sign;

/// The longest a diff of one gap may take before it settles for a rougher
/// answer, still a right one.
const DEADLINE: Duration = Duration::from_millis(250);

/// Lines the same in git's version and now: from `base` and `now`, `len`
/// of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Same {
    base: usize,
    now: usize,
    len: usize,
}

/// Lines that differ: git's, and those now in their place. Either may be
/// empty.
pub type Op = (Range<usize>, Range<usize>);

/// Git's version of a file, against the text now.
#[derive(Clone, Debug)]
pub struct GitBase {
    rope: Rope,
    same: Vec<Same>,
    /// The ops between the runs, in order.
    ops: Vec<Op>,
    /// Lines now that edits touched since the last [`GitBase::refresh`].
    dirty: Vec<Range<usize>>,
    /// Runs of lines with a sign, in order.
    marks: Vec<(Range<usize>, Sign)>,
    /// Changes the buffer had waiting when this was made, already in it.
    skip: usize,
}

/// The lines a diff counts: the empty line after a break that ends the
/// text isn't one, as `diff::signs` counts them.
fn counted(rope: &Rope) -> usize {
    let n = rope.len_lines();
    match rope.line(n - 1).len_bytes() == 0 {
        true => n - 1,
        false => n,
    }
}

/// A hash of a line, its break and all, eight bytes at a time. Where the
/// rope's chunks split the line doesn't change it.
fn hash<'a>(chunks: impl Iterator<Item = &'a str>) -> u64 {
    const K: u64 = 0x9e37_79b9_7f4a_7c15;
    let mix = |h: u64, w: u64| (h ^ w).wrapping_mul(K).rotate_left(31);
    let mut h: u64 = 0x2545_f491_4f6c_dd1d;
    let mut carry = [0u8; 8];
    let mut held = 0;
    let mut len = 0;
    for c in chunks {
        let mut b = c.as_bytes();
        len += b.len();
        if held > 0 {
            let take = (8 - held).min(b.len());
            carry[held..held + take].copy_from_slice(&b[..take]);
            held += take;
            b = &b[take..];
            if held < 8 {
                continue;
            }
            h = mix(h, u64::from_le_bytes(carry));
        }
        let (words, rest) = b.as_chunks::<8>();
        for w in words {
            h = mix(h, u64::from_le_bytes(*w));
        }
        carry[..rest.len()].copy_from_slice(rest);
        held = rest.len();
    }
    carry[held..].fill(0);
    mix(mix(h, u64::from_le_bytes(carry)), len as u64)
}

/// Where each line of `text` starts, as the rope breaks lines, and its end.
fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let mut at = 0;
    loop {
        let rest = &text[at..];
        let next = ropey::str_utils::line_to_byte_idx(rest, 1);
        if next == rest.len() {
            // The last line; after a break that ends the text, an empty
            // one, as the rope has.
            if ropey::str_utils::byte_to_line_idx(rest, rest.len()) > 0 {
                out.push(text.len());
            }
            break;
        }
        at += next;
        out.push(at);
    }
    out.push(text.len());
    out
}

/// The runs the same in `old` and `new`, each a list of lines' hashes,
/// and the ops between, counted from `base` and `now`.
///
/// The ends both share are runs as they are. In the middle, a line with
/// no match on the other side can't be part of a run, so it's left out
/// before diffing, as GNU diff does: a paste of thousands of new lines
/// costs nothing to diff.
fn diff(old: &[u64], new: &[u64], base: usize, now: usize) -> (Vec<Same>, Vec<Op>) {
    let most = old.len().min(new.len());
    let p = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let s = old[p..]
        .iter()
        .rev()
        .zip(new[p..].iter().rev())
        .take(most - p)
        .take_while(|(a, b)| a == b)
        .count();
    let (om, nm) = (p..old.len() - s, p..new.len() - s);
    let in_new: std::collections::HashSet<u64> = new[nm.clone()].iter().copied().collect();
    let in_old: std::collections::HashSet<u64> = old[om.clone()].iter().copied().collect();
    let oi: Vec<usize> = om.filter(|&i| in_new.contains(&old[i])).collect();
    let ni: Vec<usize> = nm.filter(|&j| in_old.contains(&new[j])).collect();
    drop((in_new, in_old));
    let oh: Vec<u64> = oi.iter().map(|&i| old[i]).collect();
    let nh: Vec<u64> = ni.iter().map(|&j| new[j]).collect();
    let mut runs: Vec<Same> = Vec::new();
    let mut put = |b: usize, n: usize, len: usize| {
        if len == 0 {
            return;
        }
        match runs.last_mut() {
            Some(r) if r.base + r.len == b && r.now + r.len == n => r.len += len,
            _ => runs.push(Same {
                base: b,
                now: n,
                len,
            }),
        }
    };
    put(0, 0, p);
    if !oh.is_empty() && !nh.is_empty() {
        let deadline = Instant::now() + DEADLINE;
        for op in capture_diff_slices_deadline(Algorithm::Myers, &oh, &nh, Some(deadline)) {
            if let DiffOp::Equal {
                old_index,
                new_index,
                len,
            } = op
            {
                for k in 0..len {
                    put(oi[old_index + k], ni[new_index + k], 1);
                }
            }
        }
    }
    put(old.len() - s, new.len() - s, s);
    // The ops are the gaps between the runs.
    let mut ops = Vec::new();
    let (mut b, mut n) = (0, 0);
    let ends = runs.iter().copied().chain(std::iter::once(Same {
        base: old.len(),
        now: new.len(),
        len: 0,
    }));
    for r in ends {
        if r.base > b || r.now > n {
            ops.push((base + b..base + r.base, now + n..now + r.now));
        }
        (b, n) = (r.base + r.len, r.now + r.len);
    }
    for r in &mut runs {
        r.base += base;
        r.now += now;
    }
    (runs, ops)
}

/// How many bytes `text` and the rope start with alike, and end with
/// alike after those, compared a chunk at a time.
fn common_ends(text: &str, rope: &Rope) -> (usize, usize) {
    let t = text.as_bytes();
    let mut prefix = 0;
    for chunk in rope.chunks() {
        let theirs = &t[prefix.min(t.len())..];
        let same = crate::diff::common_prefix(chunk.as_bytes(), theirs);
        prefix += same;
        if same < chunk.len() {
            break;
        }
    }
    let most = t.len().min(rope.len_bytes()) - prefix;
    let mut suffix = 0;
    let (mut chunks, ..) = rope.chunks_at_byte(rope.len_bytes());
    while suffix < most
        && let Some(chunk) = chunks.prev()
    {
        let theirs = &t[..t.len() - suffix];
        let same = crate::diff::common_suffix(chunk.as_bytes(), theirs).min(most - suffix);
        suffix += same;
        if same < chunk.len() {
            break;
        }
    }
    (prefix, suffix)
}

impl GitBase {
    /// Git's version `base` against the text `now`, which the buffer has
    /// with `pending` changes not yet synced.
    pub fn new(base: &str, now: &Rope, pending: usize) -> Self {
        let now_lines = counted(now);
        // The lines in the bytes both start and end with, less one for a
        // line break split between them.
        let (bytes_before, bytes_after) = common_ends(base, now);
        let first = now
            .byte_to_line(bytes_before)
            .saturating_sub(1)
            .min(now_lines);
        let tail = now.byte_to_line(now.len_bytes() - bytes_after) + 1;
        let last = now_lines
            .saturating_sub(tail)
            .saturating_sub(1)
            .min(now_lines - first);
        // Git's lines between those, found in its text where they lie: its
        // bytes before and after are the same as now's.
        let from = now.line_to_byte(first);
        let to = base.len() - (now.len_bytes() - now.line_to_byte(now_lines - last));
        let mut starts: Vec<usize> = line_starts(&base[from..to])
            .into_iter()
            .map(|b| b + from)
            .collect();
        let mut middle = starts.len() - 1;
        if starts[middle - 1] == to {
            middle -= 1;
            starts.truncate(middle + 1);
        }
        let base_lines = first + middle + last;
        let line = |i: usize| &base[starts[i - first]..starts[i - first + 1]];
        // Then the rest a line at a time.
        let mut prefix = first;
        while prefix < first + middle
            && prefix < now_lines - last
            && now.line(prefix) == line(prefix)
        {
            prefix += 1;
        }
        let mut suffix = last;
        while base_lines - suffix > prefix
            && now_lines - suffix > prefix
            && now.line(now_lines - 1 - suffix) == line(base_lines - 1 - suffix)
        {
            suffix += 1;
        }
        let mut same = Vec::new();
        if prefix > 0 {
            same.push(Same {
                base: 0,
                now: 0,
                len: prefix,
            });
        }
        let old: Vec<u64> = (prefix..base_lines - suffix)
            .map(|i| hash(std::iter::once(line(i))))
            .collect();
        let new: Vec<u64> = (prefix..now_lines - suffix)
            .map(|i| hash(now.line(i).chunks()))
            .collect();
        let (middle, ops) = diff(&old, &new, prefix, prefix);
        drop((old, new));
        same.extend(middle);
        if suffix > 0 {
            same.push(Same {
                base: base_lines - suffix,
                now: now_lines - suffix,
                len: suffix,
            });
        }
        // Git's version: the text now, with git's lines put back where
        // they differ, last first so the earlier places hold.
        let mut rope = now.clone();
        for (o, n) in ops.iter().rev() {
            let (a, b) = (rope.line_to_char(n.start), rope.line_to_char(n.end));
            rope.remove(a..b);
            rope.insert(a, &base[starts[o.start - first]..starts[o.end - first]]);
        }
        let mut git = Self {
            rope,
            same,
            ops,
            dirty: Vec::new(),
            marks: Vec::new(),
            skip: pending,
        };
        git.mark(now.len_lines());
        git
    }

    /// Git's version.
    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// Follows `changes`, the buffer's since the last sync: the runs they
    /// touch are cut, to be diffed again by [`GitBase::refresh`].
    pub fn update(&mut self, changes: &[Change]) {
        let changes = &changes[self.skip.min(changes.len())..];
        self.skip = 0;
        for change in changes {
            let (first, old_last, new_last) = change.lines;
            let shift = |l: usize| l + new_last - old_last;
            let mut same = Vec::with_capacity(self.same.len() + 1);
            for s in &self.same {
                let end = s.now + s.len;
                if s.now < first {
                    let len = end.min(first) - s.now;
                    same.push(Same { len, ..*s });
                }
                if end > old_last + 1 {
                    let from = s.now.max(old_last + 1);
                    same.push(Same {
                        base: s.base + from - s.now,
                        now: shift(from),
                        len: end - from,
                    });
                }
            }
            self.same = same;
            // Ops touching the edit go; it's diffed again with them.
            self.ops
                .retain(|(_, n)| n.end < first || n.start > old_last + 1);
            for (_, n) in &mut self.ops {
                if n.start > old_last {
                    *n = shift(n.start)..shift(n.end);
                }
            }
            for d in &mut self.dirty {
                if d.start > old_last {
                    *d = shift(d.start)..shift(d.end);
                } else if d.end > first {
                    *d = d.start.min(first)..shift(d.end.max(old_last + 1));
                }
            }
            self.dirty.push(first..new_last + 1);
        }
    }

    /// Diffs again the gaps between runs that edits touched, and marks the
    /// lines.
    pub fn refresh(&mut self, now: &Rope) {
        if self.dirty.is_empty() {
            return;
        }
        let dirty = std::mem::take(&mut self.dirty);
        let touched = |r: &Range<usize>| dirty.iter().any(|d| d.start <= r.end && r.start <= d.end);
        let (base_lines, now_lines) = (counted(&self.rope), counted(now));
        let mut same = Vec::with_capacity(self.same.len());
        let mut ops = Vec::with_capacity(self.ops.len());
        let mut kept = self.ops.iter().peekable();
        let (mut b, mut n) = (0, 0);
        let ends = self.same.iter().copied().chain(std::iter::once(Same {
            base: base_lines,
            now: now_lines,
            len: 0,
        }));
        for s in ends {
            let (gb, gn) = (b..s.base, n..s.now);
            // The ops in this gap, from before.
            let mut old_ops = Vec::new();
            while let Some((oo, on)) = kept.peek() {
                if oo.end > gb.end || on.end > gn.end {
                    break;
                }
                old_ops.push(kept.next().unwrap().clone());
            }
            if !gb.is_empty() || !gn.is_empty() {
                if touched(&gn) {
                    let (s2, o2) = self.diff_gap(gb, gn, now);
                    same.extend(s2);
                    ops.extend(o2);
                } else {
                    ops.extend(old_ops);
                }
            }
            if s.len > 0 {
                same.push(s);
            }
            (b, n) = (s.base + s.len, s.now + s.len);
        }
        self.same = same;
        self.ops = ops;
        self.mark(now.len_lines());
    }

    fn diff_gap(&self, base: Range<usize>, now: Range<usize>, rope: &Rope) -> (Vec<Same>, Vec<Op>) {
        if base.is_empty() || now.is_empty() {
            return (Vec::new(), vec![(base, now)]);
        }
        // A line each, as typing in one gives: compared, which stops at the
        // first difference, not hashed through.
        if base.len() == 1 && now.len() == 1 {
            return match self.rope.line(base.start) == rope.line(now.start) {
                true => (
                    vec![Same {
                        base: base.start,
                        now: now.start,
                        len: 1,
                    }],
                    Vec::new(),
                ),
                false => (Vec::new(), vec![(base, now)]),
            };
        }
        let old: Vec<u64> = base
            .clone()
            .map(|l| hash(self.rope.line(l).chunks()))
            .collect();
        let new: Vec<u64> = now.clone().map(|l| hash(rope.line(l).chunks())).collect();
        diff(&old, &new, base.start, now.start)
    }

    /// Marks each op's lines as `diff::signs` does: added, changed as far
    /// as it replaced lines, and a removal on the line after it.
    fn mark(&mut self, lines: usize) {
        let mut marks: Vec<(Range<usize>, Sign)> = Vec::new();
        let mut put = |r: Range<usize>, sign: Sign| {
            if r.is_empty() {
                return;
            }
            // A later mark wins over an earlier on the same line.
            while let Some((last, _)) = marks.last_mut()
                && last.end > r.start
            {
                last.end = r.start.max(last.start);
                if last.start >= last.end {
                    marks.pop();
                } else {
                    break;
                }
            }
            match marks.last_mut() {
                Some((last, s)) if *s == sign && last.end == r.start => last.end = r.end,
                _ => marks.push((r, sign)),
            }
        };
        for (o, n) in &self.ops {
            if n.is_empty() {
                let at = n.start.min(lines.saturating_sub(1));
                put(at..at + 1, Sign::Removed);
            } else if o.is_empty() {
                put(n.clone(), Sign::Added);
            } else {
                let changed = n.start + o.len().min(n.len());
                put(n.start..changed, Sign::Changed);
                put(changed..n.end, Sign::Added);
            }
        }
        self.marks = marks;
    }

    /// Runs of lines with a sign, in order.
    pub fn marks(&self) -> &[(Range<usize>, Sign)] {
        &self.marks
    }

    /// The sign on `line`, if it has one.
    pub fn sign(&self, line: usize) -> Option<Sign> {
        let i = self.marks.partition_point(|(r, _)| r.end <= line);
        self.marks
            .get(i)
            .filter(|(r, _)| r.contains(&line))
            .map(|&(_, s)| s)
    }

    /// Each run of changed lines: git's, and those now, as `diff::runs`
    /// gives them, with ops next to each other joined.
    pub fn runs(&self) -> Vec<Op> {
        let mut out: Vec<Op> = Vec::new();
        for (o, n) in &self.ops {
            match out.last_mut() {
                Some((lo, ln)) if lo.end == o.start && ln.end == n.start => {
                    lo.end = o.end;
                    ln.end = n.end;
                }
                _ => out.push((o.clone(), n.clone())),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::diff;

    /// Git's version rebuilt from the text now and the ops.
    fn rebuilt(git: &GitBase, now: &Rope) -> String {
        let mut out = String::new();
        let mut line = 0;
        let lines = now.len_lines();
        for (o, n) in &git.ops {
            out.extend(
                now.slice(now.line_to_char(line)..now.line_to_char(n.start))
                    .chunks(),
            );
            out.extend(
                git.rope
                    .slice(git.rope.line_to_char(o.start)..git.rope.line_to_char(o.end))
                    .chunks(),
            );
            line = n.end;
        }
        out.extend(now.slice(now.line_to_char(line.min(lines))..).chunks());
        out
    }

    /// The runs and ops tile both texts, in order.
    fn tiles(git: &GitBase, now: &Rope) -> Result<(), String> {
        let mut parts: Vec<(Range<usize>, Range<usize>)> = git
            .same
            .iter()
            .map(|s| (s.base..s.base + s.len, s.now..s.now + s.len))
            .chain(git.ops.iter().cloned())
            .collect();
        parts.sort_by_key(|(o, n)| (o.start, n.start, o.end, n.end));
        let (mut b, mut n) = (0, 0);
        for (o, nn) in &parts {
            if o.start != b || nn.start != n {
                return Err(format!("gap or overlap at {:?}: {parts:?}", (b, n)));
            }
            (b, n) = (o.end, nn.end);
        }
        if (b, n) != (counted(&git.rope), counted(now)) {
            return Err(format!("ends at {:?}", (b, n)));
        }
        Ok(())
    }

    fn signs(git: &GitBase, lines: usize) -> Vec<Option<Sign>> {
        (0..lines).map(|l| git.sign(l)).collect()
    }

    #[test]
    fn a_line_s_hash_is_the_same_however_it_s_split() {
        let text = "a line long enough to fill some words, ünïcode too\n";
        let whole = hash(std::iter::once(text));
        for at in 0..text.len() {
            if let (Some(a), Some(b)) = (text.get(..at), text.get(at..)) {
                assert_eq!(hash([a, b].into_iter()), whole, "{at}");
            }
        }
        assert_ne!(hash(std::iter::once("ab")), hash(std::iter::once("ab\0")));
    }

    #[test]
    fn signs_are_a_whole_diffs() {
        let base = "one\ntwo\nthree\nfour\nfive\n";
        for new in [
            base,
            "one\ntwo\nTHREE\nfour\nfive\n",
            "one\ntwo\nthree\nnew\nfour\nfive\n",
            "one\ntwo\nfour\nfive\n",
            "ONE\ntwo\nthree\nfour\nfive\n",
            "one\ntwo\nthree\nfour\nfive",
            "one\ntwo\nthree\nfour\nfive\nsix",
            "zero\none\ntwo\nthree\nfour\nfive\n",
            "",
            "two\nthree\n",
            "one\r\ntwo\nthree\nfour\nfive\n",
            "one\ntwo\nthree\nthree\nfour\nfive\n",
        ] {
            let rope = Rope::from_str(new);
            let git = GitBase::new(base, &rope, 0);
            assert_eq!(git.rope().to_string(), base, "{new:?}");
            assert_eq!(
                signs(&git, rope.len_lines()),
                diff::signs_rope(base, &rope),
                "{new:?}"
            );
        }
    }

    #[test]
    fn edits_keep_git_s_version_and_a_right_diff() {
        let base: String = (0..3000)
            .map(|i| match i % 7 {
                0 => "\n".to_string(),
                3 => "same line\n".to_string(),
                _ => format!("line {i}\n"),
            })
            .collect();
        let mut buffer = Buffer::new(&base.replace("line 1500\n", "changed\n"), None);
        let mut git = GitBase::new(&base, buffer.rope(), 0);
        let mut seed = 5u64;
        for step in 0..150 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let len = buffer.len();
            let at = (seed >> 33) as usize % (len + 1);
            let removed = ((seed >> 20) as usize % 60).min(len - at);
            let inserted =
                ["", "x", "\n", "line 7\nsame line\n", "new\n\n"][(seed >> 8) as usize % 5];
            buffer.begin(at, crate::history::Kind::Other);
            buffer.apply(crate::edit::Edit {
                at,
                removed: buffer.rope().slice(at..at + removed).to_string(),
                inserted: inserted.into(),
            });
            buffer.commit(at);
            let before = (git.same.clone(), git.ops.clone());
            let changes = buffer.take_changes();
            git.update(&changes);
            git.refresh(buffer.rope());
            if let Err(e) = tiles(&git, buffer.rope()) {
                let near = |r: &Range<usize>| {
                    r.start + 3 > changes[0].lines.0 && r.start < changes[0].lines.2 + 3
                };
                panic!(
                    "step {step}: {:?} broke it: {e}\nbefore: same {:?}\nops {:?}",
                    changes[0].lines,
                    before
                        .0
                        .iter()
                        .filter(|s| near(&(s.now..s.now + s.len))
                            || s.now + s.len + 3 > changes[0].lines.0
                                && s.now < changes[0].lines.2 + 3)
                        .collect::<Vec<_>>(),
                    before.1.iter().filter(|(_, n)| near(n)).collect::<Vec<_>>()
                );
            }
            assert_eq!(git.rope().to_string(), base, "step {step}");
            let back = rebuilt(&git, buffer.rope());
            if back != base {
                let at = back
                    .bytes()
                    .zip(base.bytes())
                    .take_while(|(a, b)| a == b)
                    .count();
                let line = base[..at].matches('\n').count();
                panic!(
                    "step {step}: differs at base line {line}: {:?} vs {:?}\nops {:?}\nsame {:?}",
                    &back[at.saturating_sub(30)..(at + 30).min(back.len())],
                    &base[at.saturating_sub(30)..(at + 30).min(base.len())],
                    git.ops
                        .iter()
                        .filter(|(o, _)| o.start + 50 > line && o.start < line + 50)
                        .collect::<Vec<_>>(),
                    git.same
                        .iter()
                        .filter(|s| s.base + s.len + 50 > line && s.base < line + 50)
                        .collect::<Vec<_>>(),
                );
            }
            // As few lines marked as a fresh diff, give or take its choices.
            let fresh = GitBase::new(&base, buffer.rope(), 0);
            let count = |g: &GitBase| g.marks().iter().map(|(r, _)| r.len()).sum::<usize>();
            assert!(count(&git) <= count(&fresh) + 8, "step {step}");
        }
        // Undone to git's version, nothing's marked.
        let now = Rope::from_str(&base);
        let len = buffer.len();
        buffer.begin(0, crate::history::Kind::Other);
        buffer.apply(crate::edit::Edit {
            at: 0,
            removed: buffer.rope().slice(..len).to_string(),
            inserted: base.clone(),
        });
        buffer.commit(0);
        git.update(&buffer.take_changes());
        git.refresh(buffer.rope());
        assert!(git.marks().is_empty(), "{:?}", git.marks());
        assert_eq!(buffer.rope(), &now);
    }
}
