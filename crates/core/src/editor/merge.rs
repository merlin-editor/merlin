//! A merge, rebase or cherry-pick that stopped on conflicts, as Source
//! Control shows it: a banner saying what's coming in, Abort, and Commit
//! Merge (or Continue Rebase) once no conflicts are left, with git's
//! message ready.

use super::views::CHANGES;
use super::{Editor, Effect, GitAction, MergeKind, MergeState};
use crate::panel::{Button, Node, Tone};

/// "1 conflict", "3 conflicts".
fn count(n: usize, what: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("{n} {what}{s}")
}

impl MergeState {
    /// Conflicts left in all its files.
    pub(crate) fn left(&self) -> usize {
        self.conflicts.iter().map(|(_, n)| n).sum()
    }

    /// What's left, as "3 conflicts in 2 files".
    pub(crate) fn summary(&self) -> String {
        match (self.left(), self.conflicts.len()) {
            (_, 0) => "No conflicts left".into(),
            (0, files) => format!("{} resolved, to stage", count(files, "file")),
            (left, files) => format!("{} in {}", count(left, "conflict"), count(files, "file")),
        }
    }

    /// The verb for it, as the status bar starts.
    pub(crate) fn doing(&self) -> &'static str {
        match self.kind {
            MergeKind::Merge => "Merging",
            MergeKind::Rebase => "Rebasing",
            MergeKind::CherryPick => "Cherry-picking",
        }
    }

    fn finish_label(&self) -> &'static str {
        match self.kind {
            MergeKind::Merge => "✓ Commit Merge",
            MergeKind::Rebase => "✓ Continue Rebase",
            MergeKind::CherryPick => "✓ Continue Cherry-Pick",
        }
    }
}

impl Editor {
    /// What git is in the middle of, from each status. A merge starting
    /// puts git's message in the commit box, and ending takes it out.
    pub(crate) fn merge_state(&mut self, merge: Option<MergeState>) {
        let key = (CHANGES.to_string(), "message".to_string());
        match (&self.git.merge, &merge) {
            (None, Some(m)) if m.kind == MergeKind::Merge && !m.message.is_empty() => {
                if self.inputs.get(&key).is_none_or(|t| t.trim().is_empty()) {
                    self.inputs.insert(key, m.message.clone());
                }
            }
            // Aborted: git's message goes with it, unless it was edited.
            (Some(m), None) if self.inputs.get(&key) == Some(&m.message) => {
                self.inputs.remove(&key);
                self.input_cursors.remove(&key);
            }
            _ => {}
        }
        if self.git.merge != merge {
            self.git.merge = merge;
            self.git.version += 1;
        }
    }

    /// Source Control's top while merging: the banner, the message, and
    /// the button that finishes, waiting until no conflicts are left.
    pub(crate) fn merge_nodes(&self, merge: &MergeState, branch: &str) -> Vec<Node> {
        let title = match merge.kind {
            MergeKind::Merge => vec![
                ("Merging ".to_string(), false),
                (merge.from.clone(), true),
                (format!(" into {branch}"), false),
            ],
            MergeKind::Rebase => vec![("Rebasing ".to_string(), false), (merge.from.clone(), true)],
            MergeKind::CherryPick => {
                vec![
                    ("Cherry-picking ".to_string(), false),
                    (merge.from.clone(), true),
                ]
            }
        };
        let abort = match merge.kind {
            MergeKind::Merge => "Abort Merge",
            MergeKind::Rebase => "Abort Rebase",
            MergeKind::CherryPick => "Abort Cherry-Pick",
        };
        let mut nodes = vec![Node::Banner {
            title,
            detail: merge.summary(),
            tone: if merge.conflicts.is_empty() {
                Tone::Success
            } else {
                Tone::Error
            },
            buttons: vec![Button {
                id: "g:abort".into(),
                label: abort.into(),
            }],
        }];
        // A rebase or cherry-pick keeps each commit's own message.
        if merge.kind == MergeKind::Merge {
            nodes.push(Node::Input {
                id: "message".into(),
                placeholder: "Message (Ctrl+Enter to commit)".into(),
                secret: false,
                multiline: true,
            });
        }
        nodes.push(Node::Wide {
            button: Button {
                id: "g:continue".into(),
                label: merge.finish_label().into(),
            },
            ready: merge.conflicts.is_empty(),
        });
        nodes
    }

    /// Commit Merge, or Continue Rebase: says what's left instead while
    /// files have conflicts.
    pub(crate) fn finish_merge(&mut self, fx: &mut Vec<Effect>) {
        let Some(merge) = &self.git.merge else { return };
        if !merge.conflicts.is_empty() {
            let text = match merge.left() {
                0 => format!(
                    "Stage the resolved {} first",
                    if merge.conflicts.len() == 1 {
                        "file"
                    } else {
                        "files"
                    }
                ),
                n => format!("Resolve the {} first", count(n, "conflict")),
            };
            return self.error(text);
        }
        if merge.kind != MergeKind::Merge {
            return fx.push(Effect::Git(GitAction::Continue));
        }
        let key = (CHANGES.to_string(), "message".to_string());
        let typed = self.inputs.get(&key).cloned().unwrap_or_default();
        let message = if typed.trim().is_empty() {
            merge.message.clone()
        } else {
            typed.trim().to_string()
        };
        if message.is_empty() {
            return self.error("Type a commit message first".into());
        }
        self.inputs.remove(&key);
        self.input_cursors.remove(&key);
        fx.push(Effect::Git(GitAction::Commit(message)));
    }
}
