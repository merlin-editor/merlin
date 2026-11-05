//! A file's changes since the last commit, side by side: staging or
//! reverting a change, or single lines, from the changes tab, as
//! JetBrains does from its diff. The tab compares the last commit with the
//! working tree; what's staged comes from the index's diffs Source Control
//! already has.

use std::ops::Range;

use super::{Editor, Effect, GitAction};
use crate::diff::LineKind;
use crate::doc::Review;
use crate::git::{FileDiff, patch_lines};
use crate::panel::Check;

/// Unchanged lines a changes tab shows around each change: all of them.
pub(crate) const WHOLE_FILE: usize = 1_000_000;

/// The changes tab's bar, at the right of its tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffBarView {
    /// The change the cursor is on or after, from 1, and how many.
    pub change: usize,
    pub changes: usize,
    pub inline: bool,
}

/// The box and buttons for one row of a changes tab side by side, in the
/// gap between the halves and before the new side's numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffMark {
    /// The screen row, from the top of the text.
    pub row: i32,
    /// On a change's first row: its index, whether it's staged, and how
    /// many rows it has.
    pub change: Option<(usize, Check, usize)>,
    /// A changed row's own box: its first buffer line, and whether its
    /// lines (the removed one and the one that replaced it) are staged.
    pub line: Option<(usize, bool)>,
    /// The row is part of a change.
    pub changed: bool,
}

/// What the gap of a changes tab side by side shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffMarks {
    /// The gap's first column and width, in the group's cells.
    pub gap: (u16, u16),
    /// Only on the new side.
    pub rows: Vec<DiffMark>,
    /// This side's rows with no line, to hatch.
    pub fillers: Vec<i32>,
}

/// Runs of changed lines in a review, as buffer line ranges.
pub(crate) fn runs(review: &Review) -> Vec<Range<usize>> {
    let changed = |i: usize| {
        review
            .lines
            .get(i)
            .is_some_and(|(k, _)| matches!(k, LineKind::Added | LineKind::Removed))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < review.lines.len() {
        if !changed(i) {
            i += 1;
            continue;
        }
        let start = i;
        while changed(i) {
            i += 1;
        }
        out.push(start..i);
    }
    out
}

impl Editor {
    /// Whether the document shown is a file's changes since the last
    /// commit.
    pub(crate) fn in_changes_tab(&self) -> bool {
        self.doc()
            .review
            .as_ref()
            .is_some_and(|r| r.plugin.is_empty())
    }

    /// Works out which changed lines are staged in every changes tab, from
    /// the index's diffs.
    pub(crate) fn mark_staged(&mut self) {
        for doc in &mut self.docs {
            let Some(review) = doc.review.as_mut().filter(|r| r.plugin.is_empty()) else {
                continue;
            };
            let path = review.path.to_string_lossy().to_string();
            let staged = self.git.staged_diffs.get(&path);
            let changed = self.git.changed_diffs.get(&path);
            review.staged = review
                .lines
                .iter()
                .enumerate()
                .map(|(i, (kind, number))| match kind {
                    // Nothing's staged of a file the index has no change to.
                    _ if staged.is_none() && !matches!(kind, LineKind::Same | LineKind::Gap) => {
                        Some(false)
                    }
                    LineKind::Added => {
                        Some(changed.is_none_or(|c| c.find('+', number + 1).is_none()))
                    }
                    LineKind::Removed => {
                        let old = review.old_numbers[i];
                        Some(staged.is_some_and(|s| s.find('-', old + 1).is_some()))
                    }
                    _ => None,
                })
                .collect();
        }
    }

    /// Whether every line in `lines` is staged, none is, or some are.
    fn check_of(review: &Review, lines: Range<usize>) -> Check {
        let states: Vec<bool> = lines
            .filter_map(|l| review.staged.get(l).copied().flatten())
            .collect();
        if states.iter().all(|s| *s) {
            Check::On
        } else if states.iter().any(|s| *s) {
            Check::Some
        } else {
            Check::Off
        }
    }

    /// The change the cursor is on, else the next one, as an index into
    /// [`runs`].
    fn change_at_cursor(&self) -> Option<usize> {
        let review = self.doc().review.as_ref()?;
        let line = self.doc().line_col(self.doc().cursor).0;
        let runs = runs(review);
        runs.iter()
            .position(|r| r.end > line)
            .or_else(|| runs.len().checked_sub(1))
    }

    /// The changes tab's bar, when one is shown.
    pub(crate) fn diff_bar(&self) -> Option<DiffBarView> {
        if !self.in_changes_tab() {
            return None;
        }
        let review = self.doc().review.as_ref()?;
        let changes = runs(review).len();
        Some(DiffBarView {
            change: self.change_at_cursor().map_or(0, |i| i + 1),
            changes,
            inline: self.split_rows().is_none(),
        })
    }

    /// How many of the changes tab's changes are staged, for the status
    /// bar.
    pub(crate) fn staged_count(&self) -> Option<(usize, usize)> {
        if !self.in_changes_tab() {
            return None;
        }
        let review = self.doc().review.as_ref()?;
        let runs = runs(review);
        let staged = runs
            .iter()
            .filter(|r| Self::check_of(review, (*r).clone()) == Check::On)
            .count();
        Some((staged, runs.len()))
    }

    /// The marks for the rows a changes tab shows side by side, from
    /// `first`, `height` rows of `rows`: the old side's, then the new's.
    pub(crate) fn diff_marks(
        &self,
        rows: &[(Option<usize>, Option<usize>)],
        first: usize,
        height: usize,
        gap: (u16, u16),
    ) -> Option<(DiffMarks, DiffMarks)> {
        if !self.in_changes_tab() {
            return None;
        }
        let review = self.doc().review.as_ref()?;
        let runs = runs(review);
        let run_of = |line: usize| runs.iter().position(|r| r.contains(&line));
        let mut marks = DiffMarks {
            gap,
            ..Default::default()
        };
        let mut old_side = marks.clone();
        // Where each change starts and ends among the rows.
        let mut spans: Vec<(usize, usize)> = vec![(usize::MAX, 0); runs.len()];
        for (row, (l, r)) in rows.iter().enumerate() {
            for line in [*l, *r].into_iter().flatten() {
                if let Some(c) = run_of(line) {
                    spans[c].0 = spans[c].0.min(row);
                    spans[c].1 = spans[c].1.max(row + 1);
                }
            }
        }
        for (row, (l, r)) in rows.iter().enumerate().skip(first).take(height) {
            let screen = (row - first) as i32;
            let change = [*l, *r]
                .into_iter()
                .flatten()
                .find_map(run_of)
                .filter(|&c| spans[c].0 == row || row == first && spans[c].0 < first)
                .map(|c| {
                    let check = Self::check_of(review, runs[c].clone());
                    (c, check, spans[c].1 - row)
                });
            let row_lines: Vec<usize> = [*l, *r]
                .into_iter()
                .flatten()
                .filter(|&x| matches!(review.lines[x].0, LineKind::Added | LineKind::Removed))
                .collect();
            let line = row_lines.first().map(|&first| {
                let all = row_lines
                    .iter()
                    .all(|&x| review.staged.get(x).copied().flatten() == Some(true));
                (first, all)
            });
            let changed = [*l, *r].into_iter().flatten().any(|x| run_of(x).is_some());
            marks.rows.push(DiffMark {
                row: screen,
                change,
                line,
                changed,
            });
            if l.is_none() {
                old_side.fillers.push(screen);
            }
            if r.is_none() {
                marks.fillers.push(screen);
            }
        }
        Some((old_side, marks))
    }

    /// Stages or unstages buffer lines of the changes tab: staging the
    /// ones not staged when `stage`, else unstaging the staged ones.
    fn set_staged(&mut self, lines: &[usize], stage: bool, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.as_ref() else {
            return;
        };
        let path = review.path.to_string_lossy().to_string();
        let staged = self.git.staged_diffs.get(&path);
        let changed = self.git.changed_diffs.get(&path);
        // A file git doesn't track yet is staged whole.
        let untracked = self
            .git
            .changed
            .iter()
            .any(|(p, s)| *p == path && *s == '?');
        if untracked {
            if stage {
                fx.push(Effect::Git(GitAction::Stage(path)));
            }
            return;
        }
        let mut chosen: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut take = |at: Option<(usize, usize)>| {
            if let Some((h, i)) = at {
                match chosen.iter_mut().find(|(hh, _)| *hh == h) {
                    Some((_, list)) => list.push(i),
                    None => chosen.push((h, vec![i])),
                }
            }
        };
        for &line in lines {
            let (kind, number) = review.lines[line];
            let old = review.old_numbers[line];
            if review.staged.get(line).copied().flatten() == Some(stage) {
                continue;
            }
            match (kind, stage) {
                // In the index's diff, from the index to the working tree.
                (LineKind::Added, true) => take(changed.and_then(|c| c.find('+', number + 1))),
                (LineKind::Removed, true) => {
                    let in_index = staged.map_or(old + 1, |s| s.map_line(old + 1, true));
                    take(changed.and_then(|c| c.find('-', in_index)));
                }
                // In the staged diff, from the last commit to the index.
                (LineKind::Added, false) => {
                    let in_index = changed.map_or(number + 1, |c| c.map_line(number + 1, false));
                    take(staged.and_then(|s| s.find('+', in_index)));
                }
                (LineKind::Removed, false) => take(staged.and_then(|s| s.find('-', old + 1))),
                _ => {}
            }
        }
        if chosen.is_empty() {
            return;
        }
        let file: Option<&FileDiff> = if stage { changed } else { staged };
        let Some(file) = file else { return };
        fx.push(Effect::Git(GitAction::Apply {
            patch: patch_lines(file, &chosen, !stage),
            cached: true,
            reverse: !stage,
        }));
    }

    /// The box of change `index`: stages it, or unstages it when it's all
    /// staged.
    pub(crate) fn toggle_change(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.as_ref() else {
            return;
        };
        let Some(run) = runs(review).into_iter().nth(index) else {
            return;
        };
        let stage = Self::check_of(review, run.clone()) != Check::On;
        let lines: Vec<usize> = run.collect();
        self.set_staged(&lines, stage, fx);
    }

    /// A changed row's own box, by its first buffer line: side by side, a
    /// removed line and the one that replaced it go together.
    pub(crate) fn toggle_line(&mut self, line: usize, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.as_ref() else {
            return;
        };
        let lines: Vec<usize> = match self.split_rows() {
            Some(rows) => rows
                .into_iter()
                .find(|(l, r)| *l == Some(line) || *r == Some(line))
                .map(|(l, r)| [l, r].into_iter().flatten().collect())
                .unwrap_or_else(|| vec![line]),
            None => vec![line],
        };
        let lines: Vec<usize> = lines
            .into_iter()
            .filter(|&x| review.staged.get(x).copied().flatten().is_some())
            .collect();
        let all = lines
            .iter()
            .all(|&x| review.staged.get(x).copied().flatten() == Some(true));
        self.set_staged(&lines, !all, fx);
    }

    /// « on change `index`: the working tree goes back to the last commit
    /// there, unstaging it first. The notice has Undo.
    pub(crate) fn revert_change(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.as_ref() else {
            return;
        };
        let Some(run) = runs(review).into_iter().nth(index) else {
            return;
        };
        let lines: Vec<usize> = run.clone().collect();
        let path = review.path.to_string_lossy().to_string();
        let patch = revert_patch(self.doc(), &[run]);
        self.set_staged(&lines, false, fx);
        fx.push(Effect::Git(GitAction::Discard {
            path,
            patch: Some(patch),
        }));
    }

    /// Stage File and Revert File, from the changes tab's bar.
    pub(crate) fn stage_file(&mut self, fx: &mut Vec<Effect>) {
        if let Some(review) = self.doc().review.as_ref().filter(|r| r.plugin.is_empty()) {
            let path = review.path.to_string_lossy().to_string();
            fx.push(Effect::Git(GitAction::Stage(path)));
        }
    }

    pub(crate) fn revert_file(&mut self, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.as_ref().filter(|r| r.plugin.is_empty()) else {
            return;
        };
        let all = runs(review);
        if all.is_empty() {
            return;
        }
        let patch = revert_patch(self.doc(), &all);
        let path = review.path.to_string_lossy().to_string();
        fx.push(Effect::Git(GitAction::Unstage(path.clone())));
        fx.push(Effect::Git(GitAction::Discard {
            path,
            patch: Some(patch),
        }));
    }

    /// The changes tab's commands: next and previous change, side by side
    /// or inline, and staging or reverting the change at the cursor.
    pub(crate) fn diff_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        if !self.in_changes_tab() {
            return;
        }
        match id {
            "diff.next" => self.next_change(true),
            "diff.prev" => self.next_change(false),
            "diff.inline" => {
                let inline = self.split_rows().is_none();
                self.diff_inline = Some(!inline);
            }
            "diff.stage" => {
                if let Some(i) = self.change_at_cursor() {
                    self.toggle_change(i, fx);
                }
            }
            "diff.revert" => {
                if let Some(i) = self.change_at_cursor() {
                    self.revert_change(i, fx);
                }
            }
            "diff.stage_file" => self.stage_file(fx),
            "diff.revert_file" => self.revert_file(fx),
            _ => {}
        }
    }
}

/// A patch from the last commit to the working tree for some runs of a
/// review, with the unchanged lines around them, for `git apply -R`.
fn revert_patch(doc: &crate::doc::Doc, runs: &[Range<usize>]) -> String {
    const CONTEXT: usize = 3;
    let Some(review) = doc.review.as_ref() else {
        return String::new();
    };
    let p = review.path.to_string_lossy();
    let mut out = format!("diff --git a/{p} b/{p}\n--- a/{p}\n+++ b/{p}\n");
    let same = |i: usize| {
        review
            .lines
            .get(i)
            .is_some_and(|(k, _)| *k == LineKind::Same)
    };
    for run in runs {
        let mut start = run.start;
        while start > 0 && run.start - start < CONTEXT && same(start - 1) {
            start -= 1;
        }
        let mut end = run.end;
        while end - run.end < CONTEXT && same(end) {
            end += 1;
        }
        let old_start = (start..end)
            .find(|&i| review.lines[i].0 != LineKind::Added)
            .map_or(0, |i| review.old_numbers[i] + 1);
        let new_start = (start..end)
            .find(|&i| review.lines[i].0 != LineKind::Removed)
            .map_or(0, |i| review.lines[i].1 + 1);
        let n = |skip: LineKind| (start..end).filter(|&i| review.lines[i].0 != skip).count();
        out.push_str(&format!(
            "@@ -{old_start},{} +{new_start},{} @@\n",
            n(LineKind::Added),
            n(LineKind::Removed)
        ));
        for i in start..end {
            let sign = match review.lines[i].0 {
                LineKind::Added => '+',
                LineKind::Removed => '-',
                _ => ' ',
            };
            out.push(sign);
            out.extend(
                doc.line_chars(i)
                    .into_iter()
                    .take_while(|c| *c != '\n' && *c != '\r'),
            );
            out.push('\n');
        }
    }
    out
}
