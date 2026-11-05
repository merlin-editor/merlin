//! Questions that need an answer before anything else happens, asked in a
//! dialog as VS Code asks them: about unsaved changes when a file closes
//! or the editor quits, or when it changes on disk too, and before a file
//! goes to the trash. Enter takes
//! the button picked, Esc cancels, and the arrows and Tab move between
//! buttons.

use std::path::PathBuf;

use super::{Editor, Effect, FileOp};
use crate::key::{Key, KeyCode};

/// What a dialog asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Closing the document with this id, which has unsaved changes.
    Close(u64),
    /// Quitting with unsaved files.
    Quit,
    /// Moving a file or folder to the trash.
    Trash(PathBuf),
    /// The document with this id, which has unsaved changes, changed on
    /// disk to this text.
    Conflict(u64, String),
    /// Completing the merge of the document with this id, which has
    /// conflicts left.
    MergeLeft(u64),
}

pub(crate) struct Dialog {
    pub ask: Ask,
    /// The button picked, which Enter takes.
    pub selected: usize,
}

/// A dialog as a window draws it, in the middle of the window over the
/// rest, dimmed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogView {
    pub title: String,
    pub detail: String,
    pub buttons: Vec<String>,
    pub selected: usize,
    /// The button shown as the one to press, which Enter starts on.
    pub primary: usize,
}

impl Ask {
    /// The button a dialog starts on. One that comes up by itself,
    /// mid-typing, starts on the button that changes nothing.
    fn primary(&self) -> usize {
        match self {
            Ask::Conflict(..) => 2,
            Ask::MergeLeft(_) => 1,
            _ => 0,
        }
    }
}

impl Editor {
    pub(crate) fn ask(&mut self, ask: Ask) {
        self.popup = None;
        self.close_context();
        self.menu = None;
        let selected = ask.primary();
        self.dialog = Some(Dialog { ask, selected });
    }

    pub(crate) fn dialog_view(&self) -> Option<DialogView> {
        let dialog = self.dialog.as_ref()?;
        let (title, detail, buttons): (String, String, &[&str]) = match &dialog.ask {
            Ask::Close(id) => {
                let name = self
                    .index_of(*id)
                    .map_or(String::new(), |i| self.docs[i].name());
                (
                    format!("Do you want to save the changes you made to {name}?"),
                    "Your changes will be lost if you don't save them.".into(),
                    &["Save", "Don't Save", "Cancel"],
                )
            }
            Ask::Quit => {
                let dirty = self.docs.iter().filter(|d| d.buffer.is_dirty()).count();
                let title = match dirty {
                    1 => "Do you want to save the changes to 1 file?".to_string(),
                    n => format!("Do you want to save the changes to {n} files?"),
                };
                (
                    title,
                    "Your changes will be lost if you don't save them.".into(),
                    &["Save All", "Don't Save", "Cancel"],
                )
            }
            Ask::Conflict(id, _) => {
                let name = self
                    .index_of(*id)
                    .map_or(String::new(), |i| self.docs[i].name());
                (
                    format!("{name} changed on disk."),
                    "It has unsaved changes here. Reload it from disk and lose them, or overwrite the file with them?".into(),
                    &["Reload from Disk", "Overwrite", "Keep Editing"],
                )
            }
            Ask::MergeLeft(id) => {
                let i = self.index_of(*id);
                let name = i.map_or(String::new(), |i| self.docs[i].name());
                let left = self
                    .merges
                    .iter()
                    .find(|m| m.doc == *id)
                    .map_or(0, |m| m.left());
                let (title, detail) = if left == 1 {
                    (
                        format!("{name} has 1 conflict left"),
                        "Complete it anyway, and that conflict keeps its base text, without markers.",
                    )
                } else {
                    (
                        format!("{name} has {left} conflicts left"),
                        "Complete it anyway, and those conflicts keep their base text, without markers.",
                    )
                };
                (title, detail.into(), &["Complete Anyway", "Keep Resolving"])
            }
            Ask::Trash(path) => {
                let name = path
                    .file_name()
                    .map_or(path.display().to_string(), |n| n.to_string_lossy().into());
                (
                    format!("Are you sure you want to delete '{name}'?"),
                    "You can restore it from the Trash.".into(),
                    &["Move to Trash", "Cancel"],
                )
            }
        };
        Some(DialogView {
            title,
            detail,
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
            selected: dialog.selected,
            primary: dialog.ask.primary(),
        })
    }

    /// Keys while a dialog is open: nothing else gets them.
    pub(crate) fn dialog_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        let count = self.dialog_view().map_or(0, |v| v.buttons.len());
        let Some(dialog) = &mut self.dialog else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.dialog = None,
            KeyCode::Enter | KeyCode::Char(' ') => {
                let picked = dialog.selected;
                self.answer(picked, fx);
            }
            KeyCode::Right | KeyCode::Tab | KeyCode::Down => {
                dialog.selected = (dialog.selected + 1) % count.max(1);
            }
            KeyCode::Left | KeyCode::BackTab | KeyCode::Up => {
                dialog.selected = (dialog.selected + count.max(1) - 1) % count.max(1);
            }
            _ => {}
        }
    }

    /// The dialog's `button`th button, pressed.
    pub(crate) fn answer(&mut self, button: usize, fx: &mut Vec<Effect>) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match (dialog.ask, button) {
            (Ask::Close(id), 0) => {
                if let Some(i) = self.index_of(id) {
                    self.current = i;
                    self.closing.push(id);
                    self.run("file.save", fx);
                }
            }
            (Ask::Close(id), 1) => {
                if let Some(i) = self.index_of(id) {
                    self.current = i;
                    self.close(true);
                    self.sync_groups();
                }
            }
            (Ask::Quit, 0) => {
                self.quitting = true;
                self.save_all_then_quit(fx);
            }
            (Ask::Quit, 1) => self.quit(fx),
            (Ask::Trash(path), 0) => fx.push(Effect::File(FileOp::Trash(path))),
            (Ask::MergeLeft(id), 0) => {
                if let Some(i) = self.index_of(id) {
                    self.current = i;
                    self.finish_merge_file(fx);
                }
            }
            // As an edit, so Ctrl+Z brings the changes back.
            (Ask::Conflict(id, disk), 0) => {
                if let Some(i) = self.index_of(id) {
                    let doc = &mut self.docs[i];
                    let old = doc.buffer.text();
                    super::replace_text(doc, &old, &disk);
                    doc.buffer.mark_saved(doc.buffer.revision());
                }
            }
            (Ask::Conflict(id, _), 1) => {
                if let Some(i) = self.index_of(id) {
                    let back = self.current;
                    self.current = i;
                    self.run("file.save", fx);
                    self.current = back.min(self.docs.len() - 1);
                }
            }
            _ => {}
        }
    }

    /// Saves every file with changes; the editor quits once the last is
    /// on disk. An untitled one is asked a name for first.
    pub(crate) fn save_all_then_quit(&mut self, fx: &mut Vec<Effect>) {
        let dirty: Vec<usize> = (0..self.docs.len())
            .filter(|&i| self.docs[i].buffer.is_dirty())
            .collect();
        if dirty.is_empty() {
            return self.quit(fx);
        }
        let back = self.current;
        for i in dirty {
            self.current = i;
            self.run("file.save", fx);
        }
        self.current = back.min(self.docs.len() - 1);
    }

    /// A file was saved: one waiting to close does, and quitting goes on.
    pub(crate) fn after_save(&mut self, doc: u64, fx: &mut Vec<Effect>) {
        // A file completed in the merge editor is staged once it's on disk.
        if let Some(at) = self.merge_staging.iter().position(|(id, _)| *id == doc) {
            let (_, path) = self.merge_staging.remove(at);
            fx.push(Effect::Git(super::GitAction::Stage(path)));
        }
        if let Some(at) = self.closing.iter().position(|&id| id == doc) {
            self.closing.remove(at);
            if let Some(i) = self.index_of(doc)
                && !self.docs[i].buffer.is_dirty()
            {
                let back = self.docs[self.current].id;
                self.current = i;
                self.close(true);
                self.sync_groups();
                if let Some(j) = self.index_of(back) {
                    self.current = j;
                }
            }
        }
        if self.quitting && self.docs.iter().all(|d| !d.buffer.is_dirty()) {
            self.quit(fx);
        }
    }

    /// Quits now, opening the folder asked for first, if one was.
    pub(crate) fn quit(&mut self, fx: &mut Vec<Effect>) {
        if let Some(folder) = self.open_folder.take() {
            fx.push(Effect::OpenFolder(folder));
        }
        fx.push(Effect::Quit);
    }
}
