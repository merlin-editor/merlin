//! Hot exit: quitting keeps the unsaved files, which come back at the next
//! start as they were left, as they do after a crash. A file that changed
//! on disk meanwhile says so over its text.

use std::path::PathBuf;

use super::compare_files::Left;
use super::dialog::Ask;
use super::{Doc, Editor, Effect, Focus, Palette, Source, replace_text};

/// A file whose unsaved changes came back from a backup.
pub(crate) struct Restored {
    pub id: u64,
    /// The file's text on disk as it came back; none for an untitled file,
    /// or one gone from disk.
    pub disk: Option<String>,
    /// The file on disk isn't the one the changes were made to.
    pub moved: bool,
}

impl Editor {
    /// A file's unsaved changes, back from a backup: they replace its text
    /// as one step that can be undone. An untitled file, or one gone from
    /// disk, opens as a new tab.
    pub(crate) fn recovered(&mut self, path: Option<PathBuf>, text: &str, moved: bool) {
        let at = path.as_ref().and_then(|p| {
            self.docs
                .iter()
                .position(|d| d.buffer.path.as_ref() == Some(p))
        });
        let blank = {
            let doc = self.doc();
            doc.buffer.path.is_none() && !doc.buffer.is_dirty() && doc.buffer.text().is_empty()
        };
        let (i, disk) = match at {
            Some(i) => (i, Some(self.docs[i].buffer.text())),
            None => {
                let id = self.new_id();
                self.docs.push(Doc::new(id, "", path));
                (self.docs.len() - 1, None)
            }
        };
        let doc = &mut self.docs[i];
        let old = doc.buffer.text();
        if old != text {
            replace_text(doc, &old, text);
        }
        let id = doc.id;
        self.restored.retain(|r| r.id != id);
        self.restored.push(Restored {
            id,
            moved: moved && disk.is_some(),
            disk,
        });
        // The tabs left in front stay there; an empty window shows what
        // came back.
        if blank {
            self.current = i;
        }
        self.sync_groups();
    }

    /// Every backup is back: a notice says how many, and after a crash
    /// offers to drop them.
    pub(crate) fn recovered_all(&mut self, crashed: bool) {
        let files = match self.restored.len() {
            0 => return,
            1 => "1 file with unsaved changes is".to_string(),
            n => format!("{n} files with unsaved changes are"),
        };
        if crashed {
            self.notify_warning(format!("merlin didn't close properly. {files} back."));
            self.add_action("Discard Them", "files.discard_restored");
        } else {
            self.notify(format!("{files} back, as you left them."));
        }
        self.add_action("Show", "files.show_restored");
    }

    /// Quitting: with hot exit, unsaved files are kept for the next start
    /// rather than asked about.
    pub(crate) fn quit_asked(&mut self, fx: &mut Vec<Effect>) {
        let dirty = self.docs.iter().any(|d| d.buffer.is_dirty());
        if dirty && !self.hot_exit {
            self.ask(Ask::Quit);
        } else {
            self.keep_unsaved = dirty;
            self.quit(fx);
        }
    }

    /// The editor quit with unsaved files to keep for the next start.
    pub fn keeps_unsaved(&self) -> bool {
        self.keep_unsaved
    }

    /// The restored files that still have their changes, and where each
    /// is open.
    fn restored_open(&self) -> Vec<(usize, &Restored)> {
        self.restored
            .iter()
            .filter_map(|r| Some((self.index_of(r.id)?, r)))
            .filter(|(i, _)| self.docs[*i].buffer.is_dirty())
            .collect()
    }

    pub(crate) fn hot_exit_command(&mut self, id: &str) -> bool {
        match id {
            "files.show_restored" => {
                // Files first, then untitled ones.
                let mut open = self.restored_open();
                open.sort_by_key(|(_, r)| r.disk.is_none());
                let items = open
                    .into_iter()
                    .map(|(i, r)| (r.id.to_string(), self.docs[i].name()))
                    .collect();
                self.palette = Some(Palette::new(
                    Source::Restored,
                    Some("Unsaved Changes Back ›".into()),
                    items,
                ));
                self.focus = Focus::Palette;
            }
            "files.discard_restored" => {
                if !self.restored_open().is_empty() {
                    self.ask(Ask::DiscardRestored);
                }
            }
            _ => return false,
        }
        true
    }

    /// What changed in a restored file, for Show's list.
    pub(crate) fn restored_detail(&self, id: &str) -> String {
        let found = id
            .parse()
            .ok()
            .and_then(|id| self.restored.iter().find(|r| r.id == id));
        let Some(r) = found else {
            return String::new();
        };
        let Some(disk) = &r.disk else {
            return "untitled".into();
        };
        let Some(i) = self.index_of(r.id) else {
            return String::new();
        };
        let (mut added, mut removed) = (0, 0);
        for (old, new) in crate::diff::runs(disk, &self.docs[i].buffer.text()) {
            removed += old.len();
            added += new.len();
        }
        let mut detail = format!("+{added} −{removed}");
        if r.moved {
            detail.push_str(" · changed on disk");
        }
        detail
    }

    /// A file picked in Show: it shows, beside what's on disk.
    pub(crate) fn restored_picked(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let Some(i) = id.parse().ok().and_then(|id| self.index_of(id)) else {
            return;
        };
        self.current = i;
        self.focus = Focus::Editor;
        self.sync_groups();
        let path = self.docs[i].buffer.path.clone();
        if let Some(path) = path {
            self.start_compare(Left::Saved, path, fx);
        }
    }

    pub(crate) fn discard_restored_ask(&self) -> (String, String) {
        let title = match self.restored_open().len() {
            1 => "Discard the unsaved changes to 1 file?".to_string(),
            n => format!("Discard the unsaved changes to {n} files?"),
        };
        (
            title,
            "Each file goes back to what's on disk, and untitled files close.".into(),
        )
    }

    /// Discard Them: each restored file back as it is on disk.
    pub(crate) fn discard_restored(&mut self) {
        let back = self.docs[self.current].id;
        for r in std::mem::take(&mut self.restored) {
            let Some(i) = self.index_of(r.id) else {
                continue;
            };
            match r.disk {
                Some(disk) => {
                    let doc = &mut self.docs[i];
                    let old = doc.buffer.text();
                    replace_text(doc, &old, &disk);
                    doc.buffer.mark_saved(doc.buffer.revision());
                }
                None => {
                    self.current = i;
                    self.close(true);
                }
            }
        }
        self.current = self.index_of(back).unwrap_or(self.current);
        self.sync_groups();
        self.notify("Discarded the unsaved changes".into());
    }

    /// The bar over the shown file, if it changed on disk while merlin was
    /// closed: its id and name.
    pub(crate) fn away_bar(&self) -> Option<(u64, String)> {
        let doc = &self.docs[self.pane_doc()];
        let moved = self.restored.iter().any(|r| r.id == doc.id && r.moved);
        (moved && doc.buffer.is_dirty()).then(|| (doc.id, doc.name()))
    }

    /// The bar's buttons: `away:compare:`, `away:disk:` and `away:keep:`,
    /// then the file's id.
    pub(crate) fn away_press(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let Some((what, doc)) = id
            .strip_prefix("away:")
            .and_then(|rest| rest.split_once(':'))
        else {
            return;
        };
        let Some(doc) = doc.parse().ok() else { return };
        let (Some(i), Some(at)) = (
            self.index_of(doc),
            self.restored.iter().position(|r| r.id == doc),
        ) else {
            return;
        };
        match what {
            "keep" => self.restored[at].moved = false,
            "disk" => {
                let r = self.restored.remove(at);
                if let Some(disk) = r.disk {
                    let doc = &mut self.docs[i];
                    let old = doc.buffer.text();
                    replace_text(doc, &old, &disk);
                    doc.buffer.mark_saved(doc.buffer.revision());
                }
            }
            "compare" => {
                if let Some(path) = self.docs[i].buffer.path.clone() {
                    self.current = i;
                    self.focus = Focus::Editor;
                    self.start_compare(Left::Saved, path, fx);
                }
            }
            _ => {}
        }
    }
}
