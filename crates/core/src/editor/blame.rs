//! Who last changed each line, from git blame: the cursor's line says so,
//! faint, after its end, as GitLens shows it.

use super::{Editor, Effect};

impl Editor {
    /// Asks for the current file's blame when its text has changed since
    /// it was last asked for. The host waits for a pause before it runs.
    pub(crate) fn ask_blame(&mut self, fx: &mut Vec<Effect>) {
        let doc = self.doc();
        if doc.review.is_some() || doc.page.is_some() || doc.git_base.is_none() {
            return;
        }
        let Some(path) = doc.buffer.path.clone() else {
            return;
        };
        let version = doc.buffer.version();
        let key = (path.clone(), version);
        let known = self.blames.get(&path).is_some_and(|(v, _)| *v == version);
        if known || self.blame_asked.as_ref() == Some(&key) {
            return;
        }
        let text = doc.buffer.text();
        let ignore = self.blame_ignore.get(&path).cloned().unwrap_or_default();
        self.blame_asked = Some(key);
        fx.push(Effect::Blame {
            path,
            version,
            text,
            ignore,
        });
    }

    /// The blame shown after `line` of the current file, as of its text
    /// now: who, when and the commit's summary. None for lines not yet
    /// committed.
    pub(crate) fn line_blame(&self, line: usize) -> Option<String> {
        let doc = self.doc();
        let (version, blame) = self.blames.get(doc.buffer.path.as_ref()?)?;
        if *version != doc.buffer.version() {
            return None;
        }
        let commit = blame.at(line).filter(|c| c.committed)?;
        let when = crate::git::ago(commit.time, self.now);
        Some(format!("{}, {when} · {}", commit.author, commit.summary))
    }
}

/// The review a commit's changes to a file show in.
pub(crate) const COMMIT: &str = "merlin.commit";

/// Columns the blame column takes, before the line numbers.
pub(crate) const BLAME_COLS: usize = 26;

/// A line's entry in the blame column.
#[derive(Clone, Debug, PartialEq)]
pub struct BlameCell {
    /// Who, how long ago and why, on the first line of a run from one
    /// commit; empty on the rest.
    pub text: String,
    /// How old the commit is among the file's: 1 the newest, 0 the oldest.
    pub age: f32,
    pub committed: bool,
    /// The first line of its run, which has a rule above it.
    pub first: bool,
    /// Its commit is the one the card is open for.
    pub hovered: bool,
}

/// A commit's card, over the blame column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameCardView {
    /// The screen row it opens at.
    pub row: i32,
    pub author: String,
    /// "2 weeks ago · 22 Oct 2026".
    pub when: String,
    pub hash: String,
    pub summary: String,
    /// The rest of the message, once it's come.
    pub body: String,
    /// Lines added and removed, and in how many files, once they've come.
    pub stats: Option<(usize, usize, usize)>,
}

/// What a commit's card shows past its summary, from git.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitInfo {
    pub body: String,
    pub added: usize,
    pub removed: usize,
    pub files: usize,
}

/// The day `secs` after 1970 is, as "22 Oct 2026".
fn date(secs: i64) -> String {
    // Howard Hinnant's days-to-civil.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

impl Editor {
    /// Git: Blame: the column beside the numbers, on or off for the file.
    pub(crate) fn toggle_blame_column(&mut self) {
        let id = self.doc().id;
        if self.blame_column == Some(id) {
            self.blame_column = None;
            self.blame_card = None;
            return;
        }
        if self.doc().git_base.is_none() {
            return self.notify("This file isn't in git".into());
        }
        self.blame_column = Some(id);
    }

    /// Whether the current file shows its blame column.
    pub(crate) fn blame_shown(&self) -> bool {
        self.blame_column == Some(self.doc().id) && self.doc().review.is_none()
    }

    /// Columns the blame column takes in the current file's gutter.
    pub(crate) fn blame_cols(&self) -> usize {
        if self.blame_shown() { BLAME_COLS } else { 0 }
    }

    /// The current file's blame, if it's for its text as it is now.
    fn current_blame(&self) -> Option<&crate::git::Blame> {
        let doc = self.doc();
        let (version, blame) = self.blames.get(doc.buffer.path.as_ref()?)?;
        (*version == doc.buffer.version()).then_some(blame)
    }

    /// The blame column's entry for `line`.
    pub(crate) fn blame_cell(&self, line: usize) -> Option<BlameCell> {
        if !self.blame_shown() {
            return None;
        }
        let blame = self.current_blame()?;
        let index = *blame.lines.get(line)?;
        let commit = &blame.commits[index];
        let first = line == 0 || blame.lines.get(line - 1) != Some(&index);
        let times: Vec<i64> = blame
            .commits
            .iter()
            .filter(|c| c.committed)
            .map(|c| c.time)
            .collect();
        let (old, new) = (
            times.iter().copied().min().unwrap_or(0),
            times.iter().copied().max().unwrap_or(0),
        );
        let age = if new > old {
            (commit.time - old) as f32 / (new - old) as f32
        } else {
            1.0
        };
        let text = match (first, commit.committed) {
            (false, _) => String::new(),
            (true, false) => "Not committed yet".into(),
            (true, true) => {
                let ago = crate::git::ago(commit.time, self.now);
                let ago = ago.trim_end_matches(" ago");
                format!("{} · {ago} · {}", commit.author, commit.summary)
            }
        };
        let hovered = self.blame_card.is_some_and(|(_, c)| c == index);
        Some(BlameCell {
            text,
            age,
            committed: commit.committed,
            first,
            hovered,
        })
    }

    /// A rest over the blame column at `line`: its commit's card, asking
    /// git for the rest of its message.
    pub(crate) fn hover_blame(&mut self, line: usize, fx: &mut Vec<Effect>) {
        let Some(blame) = self.current_blame() else {
            return;
        };
        let Some(&index) = blame.lines.get(line) else {
            return;
        };
        let commit = &blame.commits[index];
        if !commit.committed {
            self.blame_card = None;
            return;
        }
        let hash = commit.hash.clone();
        self.blame_card = Some((line, index));
        if !self.commit_info.contains_key(&hash) {
            fx.push(Effect::CommitInfo(hash));
        }
    }

    /// The card, opening on the row after `row`, the screen row of the
    /// line it's for.
    pub(crate) fn blame_card_view(&self, row: i32) -> Option<BlameCardView> {
        let (_, index) = self.blame_card?;
        let commit = self.current_blame()?.commits.get(index)?;
        let info = self.commit_info.get(&commit.hash);
        Some(BlameCardView {
            row: row + 1,
            author: commit.author.clone(),
            when: format!(
                "{} · {}",
                crate::git::ago(commit.time, self.now),
                date(commit.time)
            ),
            hash: commit.hash.chars().take(7).collect(),
            summary: commit.summary.clone(),
            body: info.map(|i| i.body.clone()).unwrap_or_default(),
            stats: info.map(|i| (i.added, i.removed, i.files)),
        })
    }

    /// The card's buttons: the commit's changes to this file, blame from
    /// the commit before it for these lines, and its hash.
    pub(crate) fn blame_card_button(&mut self, button: &str, fx: &mut Vec<Effect>) {
        let Some((_, index)) = self.blame_card else {
            return;
        };
        let Some(hash) = self
            .current_blame()
            .and_then(|b| b.commits.get(index))
            .map(|c| c.hash.clone())
        else {
            return;
        };
        let Some(path) = self.doc().buffer.path.clone() else {
            return;
        };
        self.blame_card = None;
        match button {
            "open" => fx.push(Effect::OpenCommit { hash, path }),
            // Leaves the commit out, so its lines show the one before.
            "before" => {
                self.blame_ignore
                    .entry(path.clone())
                    .or_default()
                    .push(hash);
                self.blames.remove(&path);
                self.blame_asked = None;
            }
            _ => {
                self.clipboard = hash.clone();
                fx.push(Effect::Copy(hash));
                self.notify("Copied the commit's hash".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn days_become_dates() {
        assert_eq!(super::date(0), "1 Jan 1970");
        assert_eq!(super::date(1_792_627_200), "22 Oct 2026");
    }
}
