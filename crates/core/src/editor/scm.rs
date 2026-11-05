//! Source Control's hunks and file menu: staging, unstaging or discarding
//! one hunk at a time, unfolding a file's hunks under it, and Undo for a
//! discard.

use std::path::PathBuf;

use super::context::ContextMenu;
use super::dialog::Ask;
use super::views::CHANGES;
use super::{Editor, Effect, Focus, GitAction, Side, SyncKind, SyncOutcome};
use crate::key::{Key, KeyCode};
use crate::panel::Target;

/// The right-click menu on a changed file.
pub(crate) const SCM_ITEMS: &[(&str, &str)] = &[
    ("scm.open_file", "Open File"),
    ("scm.open_changes", "Open Changes"),
    ("-", ""),
    ("scm.stage", "Stage"),
    ("scm.unstage", "Unstage"),
    ("scm.discard", "Discard Changes"),
    ("-", ""),
    ("scm.copy_path", "Copy Path"),
    ("scm.reveal", "Reveal in Explorer"),
];

/// Keys Source Control takes itself, for the menu to show.
pub(crate) const SCM_KEYS: &[(&str, &str)] = &[("scm.discard", "Ctrl+Del")];

impl Editor {
    /// A click on a hunk's row or one of its buttons. Returns whether `id`
    /// was one.
    pub(crate) fn hunk_click(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        if let Some(rest) = id.strip_prefix("hk:") {
            let Some((list, rest)) = rest.split_once(':') else {
                return true;
            };
            if let Some((path, change)) = self.change(list == "s", rest) {
                let line = change.line.saturating_sub(1) as u32;
                self.open_at(PathBuf::from(path), line, 0, fx);
            }
            return true;
        }
        let Some((kind, rest)) = id.split_once(':') else {
            return false;
        };
        let staged = kind == "hu";
        if !matches!(kind, "hs" | "hu" | "hd") {
            return false;
        }
        let Some((path, index)) = rest.rsplit_once(':') else {
            return true;
        };
        let Ok(index) = index.parse::<usize>() else {
            return true;
        };
        let diffs = if staged {
            &self.git.staged_diffs
        } else {
            &self.git.changed_diffs
        };
        let Some(file) = diffs.get(path) else {
            return true;
        };
        let Some(change) = file.changes().into_iter().nth(index) else {
            return true;
        };
        let action = match kind {
            "hs" => GitAction::Apply {
                patch: file.change_patch(&change, false),
                cached: true,
                reverse: false,
            },
            "hu" => GitAction::Apply {
                patch: file.change_patch(&change, true),
                cached: true,
                reverse: true,
            },
            _ => GitAction::Discard {
                path: path.to_string(),
                patch: Some(file.change_patch(&change, true)),
            },
        };
        fx.push(Effect::Git(action));
        true
    }

    /// The file and change a `path:index` id is for.
    fn change(&self, staged: bool, id: &str) -> Option<(String, crate::git::Change)> {
        let (path, index) = id.rsplit_once(':')?;
        let diffs = if staged {
            &self.git.staged_diffs
        } else {
            &self.git.changed_diffs
        };
        let change = diffs
            .get(path)?
            .changes()
            .into_iter()
            .nth(index.parse().ok()?)?;
        Some((path.to_string(), change))
    }

    /// The Source Control file a row's id is for, and whether it's staged.
    fn scm_file(id: &str) -> Option<(bool, &str)> {
        if let Some(path) = id.strip_prefix("cs:") {
            return Some((true, path));
        }
        id.strip_prefix("c:").map(|p| (false, p))
    }

    /// Folds or unfolds a changed file's hunks, from its arrow.
    pub(crate) fn fold_clicked(&mut self, side: Side, index: usize) {
        if let Some(Target::Button(id)) = self.panel_targets(side).into_iter().nth(index) {
            self.fold_file(&id, None);
        }
    }

    /// Unfolds a file's hunks when `open` is true, folds them when it's
    /// false, and turns them over when it's `None`. Returns whether `id`
    /// was a file with hunks.
    fn fold_file(&mut self, id: &str, open: Option<bool>) -> bool {
        let Some((staged, path)) = Self::scm_file(id) else {
            return false;
        };
        let diffs = if staged {
            &self.git.staged_diffs
        } else {
            &self.git.changed_diffs
        };
        if diffs.get(path).is_none_or(|d| d.hunks.is_empty()) {
            return false;
        }
        let key = format!("{}:{path}", if staged { "s" } else { "c" });
        let now = self.git.open.contains(&key);
        if open.unwrap_or(!now) {
            self.git.open.insert(key);
        } else {
            self.git.open.remove(&key);
        }
        self.git.version += 1;
        self.build_views();
        true
    }

    /// Keys on Source Control's rows: Right unfolds a file's hunks and Left
    /// folds them, as in a tree, and Ctrl+Delete discards.
    pub(crate) fn scm_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if self.active_panel_name(Side::Left).as_deref() != Some(CHANGES) {
            return false;
        }
        let index = self.selected_index(Side::Left);
        let Some(Target::Button(id)) = self.panel_targets(Side::Left).into_iter().nth(index) else {
            return false;
        };
        match key.code {
            KeyCode::Right if !key.ctrl => self.fold_file(&id, Some(true)),
            KeyCode::Left if !key.ctrl => {
                if self.fold_file(&id, Some(false)) {
                    return true;
                }
                // On a hunk, Left goes to its file.
                let Some(rest) = id.strip_prefix("hk:") else {
                    return false;
                };
                let Some((list, rest)) = rest.split_once(':') else {
                    return false;
                };
                let Some((path, _)) = rest.rsplit_once(':') else {
                    return false;
                };
                let file = format!("c{}:{path}", if list == "s" { "s" } else { "" });
                self.bar_mut(Side::Left).target = Some(Target::Button(file));
                true
            }
            KeyCode::Delete if key.ctrl => {
                self.git.menu_row = Some(id);
                self.scm_command("scm.discard", fx);
                true
            }
            _ => false,
        }
    }

    /// The right-click menu on a Source Control row.
    pub(crate) fn open_scm_menu(&mut self, row: Option<String>) {
        let Some((staged, _)) = row.as_deref().and_then(Self::scm_file) else {
            return;
        };
        self.popup = None;
        self.git.menu_row = row;
        let items = SCM_ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|&id| match id {
                "scm.stage" | "scm.discard" => !staged,
                "scm.unstage" => staged,
                _ => true,
            })
            .collect();
        self.context = Some(ContextMenu {
            items,
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    /// Starts a push, pull, sync or publish, unless one is running.
    pub(crate) fn start_sync(&mut self, kind: SyncKind, fx: &mut Vec<Effect>) {
        if self.git.syncing.is_some_and(|k| k != SyncKind::Fetch) {
            return;
        }
        self.git.syncing = Some(kind);
        self.git.version += 1;
        fx.push(Effect::Git(GitAction::Sync(kind)));
    }

    /// A push, pull, sync or fetch finished: says how, with what to do
    /// next, and keeps git's words in the Output panel.
    pub(crate) fn synced(&mut self, kind: SyncKind, outcome: SyncOutcome, output: String) {
        if self.git.syncing == Some(kind) {
            self.git.syncing = None;
        }
        self.git.version += 1;
        let verb = match kind {
            SyncKind::Push => "push",
            SyncKind::Pull => "pull",
            SyncKind::Sync => "sync",
            SyncKind::Publish => "publish",
            SyncKind::Fetch => "fetch",
        };
        if !output.trim().is_empty() {
            for line in output.lines().filter(|l| !l.trim().is_empty()) {
                self.log
                    .push((outcome != SyncOutcome::Done, format!("git {verb}: {line}")));
            }
        }
        // A fetch is quiet: it only brings the counts up to date.
        if kind == SyncKind::Fetch {
            return;
        }
        let upstream = self
            .git
            .sync
            .upstream
            .clone()
            .or_else(|| self.git.sync.remote.clone())
            .unwrap_or_else(|| "the remote".into());
        let remote = upstream.split('/').next().unwrap_or(&upstream).to_string();
        match outcome {
            SyncOutcome::Done => self.notify(match kind {
                SyncKind::Push => format!("Pushed to {upstream}"),
                SyncKind::Pull => format!("Pulled from {upstream}"),
                SyncKind::Publish => {
                    let branch = self.git.branch.clone().unwrap_or_default();
                    format!("Published {branch} to {remote}")
                }
                _ => format!("Synced with {upstream}"),
            }),
            SyncOutcome::Rejected => {
                let behind = self.git.sync.behind;
                let what = if behind > 0 {
                    let s = if behind == 1 { "" } else { "s" };
                    format!("{behind} commit{s}")
                } else {
                    "commits".into()
                };
                self.error(format!(
                    "The push was rejected: {remote} has {what} this branch doesn't."
                ));
                // The last button is the main one.
                self.add_action("Show Output", "view.output");
                self.add_action("Pull and Push", "scm.sync_all");
            }
            SyncOutcome::NeedsAuth { remote } => {
                self.notify(format!(
                    "{remote} wants a password. Set up an SSH key or a git credential helper, then try again."
                ));
                self.add_action("Show Output", "view.output");
            }
            SyncOutcome::Conflicts(files) => {
                let n = files.len();
                let s = if n == 1 { "" } else { "s" };
                self.git.conflicts = files;
                self.notify(format!("The pull stopped on conflicts in {n} file{s}."));
                self.add_action("Abort Pull", "scm.abort_pull");
                self.add_action("Resolve", "scm.resolve");
            }
            SyncOutcome::Cancelled => self.notify(format!("Stopped the {verb}")),
            SyncOutcome::Failed(line) => {
                self.error(format!("git {verb}: {line}"));
                self.add_action("Show Output", "view.output");
            }
        }
    }

    /// Runs a command from Source Control's menu, on the row it was opened
    /// for, else the one selected.
    pub(crate) fn scm_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        match id {
            "scm.sync_all" => return self.start_sync(SyncKind::Sync, fx),
            "scm.push" => return self.start_sync(SyncKind::Push, fx),
            "scm.pull" => return self.start_sync(SyncKind::Pull, fx),
            "scm.abort_pull" => return fx.push(Effect::Git(GitAction::AbortPull)),
            // The first file with conflicts, until the merge editor is
            // built.
            "scm.resolve" => {
                if let Some(path) = self.git.conflicts.first().cloned() {
                    self.open(PathBuf::from(path), fx);
                }
                return;
            }
            _ => {}
        }
        if id == "scm.undo_discard" {
            if let Some((path, text)) = self.git.discarded.take() {
                fx.push(Effect::Git(GitAction::Undiscard { path, text }));
            }
            return;
        }
        let row = self.git.menu_row.take().or_else(|| {
            let index = self.selected_index(Side::Left);
            match self.panel_targets(Side::Left).into_iter().nth(index) {
                Some(Target::Button(id)) => Some(id),
                _ => None,
            }
        });
        let Some((_, path)) = row.as_deref().and_then(Self::scm_file) else {
            return;
        };
        let path = path.to_string();
        match id {
            "scm.open_file" => self.open(PathBuf::from(&path), fx),
            "scm.open_changes" => fx.push(Effect::OpenDiff(PathBuf::from(&path))),
            "scm.stage" => fx.push(Effect::Git(GitAction::Stage(path))),
            "scm.unstage" => fx.push(Effect::Git(GitAction::Unstage(path))),
            "scm.discard" => {
                let untracked = self
                    .git
                    .changed
                    .iter()
                    .any(|(p, s)| *p == path && *s == '?');
                if untracked {
                    // Discarding a file git doesn't know deletes it, so it
                    // asks first, and it goes to the trash.
                    self.ask(Ask::Trash(PathBuf::from(path)));
                } else {
                    fx.push(Effect::Git(GitAction::Discard { path, patch: None }));
                }
            }
            "scm.copy_path" => {
                let text = self.root.join(&path).display().to_string();
                self.clipboard = text.clone();
                fx.push(Effect::Copy(text));
            }
            "scm.reveal" => {
                self.open(PathBuf::from(&path), fx);
                self.reveal_in_explorer(fx);
            }
            _ => {}
        }
    }

    /// A discard went through: says so, with Undo.
    pub(crate) fn discarded(&mut self, path: String, before: String, all: bool) {
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        self.git.discarded = Some((path, before));
        self.notify(if all {
            format!("Discarded the changes to {name}")
        } else {
            format!("Discarded a change to {name}")
        });
        self.add_action("Undo", "scm.undo_discard");
    }
}
