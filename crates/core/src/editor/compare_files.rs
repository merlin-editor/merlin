//! Compare Files, as VS Code has it: any two files side by side in the
//! changes tab's view, with its bar. The right side is the real file and
//! can be edited; Swap puts the other one there. A file can also be
//! compared with the clipboard, or with what's saved of it.

use std::path::{Path, PathBuf};

use super::{Editor, Effect, Focus};

/// The review kind of a comparison: its id is what's on the left.
pub(crate) const COMPARE: &str = "merlin.compare";

/// The left side of a comparison: another file, the clipboard, or the file
/// as saved. Only a file can swap to the right.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Left {
    File(PathBuf),
    Clipboard,
    Saved,
}

impl Left {
    /// The review's id for it.
    fn id(&self) -> String {
        match self {
            Left::File(p) => p.display().to_string(),
            Left::Clipboard => "clipboard:".into(),
            Left::Saved => "saved:".into(),
        }
    }
}

/// A comparison waiting for a file's text from disk, or for the second
/// file to be picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Comparing {
    /// Compare Files…: the first file picked, once it is.
    Picking(Option<PathBuf>),
    Reading {
        left: Left,
        right: PathBuf,
        left_text: Option<String>,
        right_text: Option<String>,
    },
    /// Compare with Clipboard, until the clipboard's text comes.
    Clipboard(PathBuf),
}

fn name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    )
}

impl Editor {
    /// Whether the document shown is a comparison of two files.
    pub(crate) fn in_compare(&self) -> bool {
        self.doc()
            .review
            .as_ref()
            .is_some_and(|r| r.plugin == COMPARE)
    }

    /// The comparison's left side, from its review's id.
    pub(crate) fn compare_left(&self) -> Option<Left> {
        let review = self.doc().review.as_ref().filter(|r| r.plugin == COMPARE)?;
        Some(match review.id.as_str() {
            "clipboard:" => Left::Clipboard,
            "saved:" => Left::Saved,
            path => Left::File(PathBuf::from(path)),
        })
    }

    /// The title each side of a comparison shows: its name and folder.
    pub(crate) fn compare_titles(&self) -> Option<((String, String), (String, String))> {
        let review = self.doc().review.as_ref().filter(|r| r.plugin == COMPARE)?;
        let folder = |p: &Path| {
            p.parent()
                .map(|d| d.display().to_string())
                .filter(|d| !d.is_empty())
                .map_or(String::new(), |d| format!("{d}/"))
        };
        let right = name(&review.path);
        let left = match self.compare_left()? {
            Left::File(p) => (name(&p), folder(&p)),
            Left::Clipboard => ("Clipboard".into(), "read-only".into()),
            Left::Saved => (format!("{right} (saved)"), "on disk".into()),
        };
        let editable = match self.compare_left()? {
            Left::File(_) => "you can edit both",
            _ => "you can edit here",
        };
        let detail = match folder(&review.path) {
            f if f.is_empty() => editable.to_string(),
            f => format!("{f} · {editable}"),
        };
        Some((left, (right, detail)))
    }

    /// The palette's and the Explorer's compare commands.
    pub(crate) fn compare_command(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        match id {
            "file.compare" => {
                self.comparing = Some(Comparing::Picking(None));
                fx.push(Effect::ListFiles);
                self.notify("Compare Files: pick the first file".into());
            }
            "file.compare_clipboard" => {
                let Some(path) = self.compare_path() else {
                    return true;
                };
                self.comparing = Some(Comparing::Clipboard(path));
                fx.push(Effect::ReadClipboard);
            }
            "file.compare_saved" => {
                let Some(path) = self.compare_path() else {
                    return true;
                };
                self.start_compare(Left::Saved, path, fx);
            }
            "compare.swap" => self.swap_compare(fx),
            _ => return false,
        }
        true
    }

    /// The file in the tab shown, to compare: a changes tab's or a
    /// comparison's is its file.
    fn compare_path(&mut self) -> Option<PathBuf> {
        let doc = self.doc();
        let path = doc
            .review
            .as_ref()
            .map(|r| r.path.clone())
            .or_else(|| doc.buffer.path.clone());
        if path.is_none() {
            self.notify("Save the file first, to compare it".into());
        }
        path
    }

    /// A file picked in the palette while Compare Files asks for one: the
    /// first, then the second. Whether it was for that.
    pub(crate) fn compare_picked(&mut self, path: PathBuf, fx: &mut Vec<Effect>) -> bool {
        match self.comparing.take() {
            Some(Comparing::Picking(None)) => {
                let first = name(&path);
                self.comparing = Some(Comparing::Picking(Some(path)));
                fx.push(Effect::ListFiles);
                self.notify(format!("Compare {first} with…"));
                true
            }
            Some(Comparing::Picking(Some(first))) => {
                self.start_compare(Left::File(first), path, fx);
                true
            }
            other => {
                self.comparing = other;
                false
            }
        }
    }

    /// The clipboard's text, for Compare with Clipboard. Whether it was
    /// for that.
    pub(crate) fn compare_clipboard(&mut self, text: &str, fx: &mut Vec<Effect>) -> bool {
        let Some(Comparing::Clipboard(path)) = self.comparing.clone() else {
            return false;
        };
        self.comparing = Some(Comparing::Reading {
            left: Left::Clipboard,
            right: path.clone(),
            left_text: Some(text.to_string()),
            right_text: None,
        });
        self.read_compare(fx);
        true
    }

    /// Compares `left` with the file at `right`, reading what isn't open.
    pub(crate) fn start_compare(&mut self, left: Left, right: PathBuf, fx: &mut Vec<Effect>) {
        self.comparing = Some(Comparing::Reading {
            left,
            right,
            left_text: None,
            right_text: None,
        });
        self.read_compare(fx);
    }

    /// An open file's text, unsaved edits and all.
    fn open_text(&self, path: &Path) -> Option<String> {
        self.docs
            .iter()
            .find(|d| d.review.is_none() && d.buffer.path.as_deref() == Some(path))
            .map(|d| d.buffer.text())
    }

    /// Fills in what's open, asks for the rest, and shows the comparison
    /// once both texts are in.
    fn read_compare(&mut self, fx: &mut Vec<Effect>) {
        let Some(Comparing::Reading {
            left,
            right,
            left_text,
            right_text,
        }) = self.comparing.clone()
        else {
            return;
        };
        let right_text = right_text.or_else(|| self.open_text(&right));
        let left_text = left_text.or_else(|| match &left {
            Left::File(p) => self.open_text(p),
            _ => None,
        });
        let mut wanted = Vec::new();
        if left_text.is_none() {
            wanted.push(match &left {
                Left::File(p) => p.clone(),
                _ => right.clone(),
            });
        }
        if right_text.is_none() && !wanted.contains(&right) {
            wanted.push(right.clone());
        }
        match (left_text, right_text) {
            (Some(old), Some(new)) => {
                self.comparing = None;
                let replace = self.compare_replace.take();
                self.show_review(COMPARE.into(), left.id(), right, &old, &new);
                // Swapped: the new tab takes the old one's place.
                if let Some(i) = replace.and_then(|id| self.index_of(id)) {
                    let shown = self.docs[self.current].id;
                    self.current = i;
                    self.close(true);
                    self.sync_groups();
                    if let Some(j) = self.index_of(shown) {
                        self.current = j;
                    }
                }
                self.focus = Focus::Editor;
            }
            (left_text, right_text) => {
                self.comparing = Some(Comparing::Reading {
                    left,
                    right,
                    left_text,
                    right_text,
                });
                fx.push(Effect::ReadFiles(wanted));
            }
        }
    }

    /// Files read from disk, for a comparison waiting on them.
    pub(crate) fn compare_files_read(
        &mut self,
        files: &[(PathBuf, Option<String>)],
        fx: &mut Vec<Effect>,
    ) {
        let Some(Comparing::Reading {
            left,
            right,
            mut left_text,
            mut right_text,
        }) = self.comparing.clone()
        else {
            return;
        };
        let left_path = match &left {
            Left::File(p) => Some(p.clone()),
            Left::Saved => Some(right.clone()),
            Left::Clipboard => None,
        };
        for (path, text) in files {
            let text = text.clone().unwrap_or_default();
            if left_text.is_none() && left_path.as_ref() == Some(path) {
                left_text = Some(text.clone());
            }
            if right_text.is_none() && *path == right && left != Left::Saved {
                right_text = Some(text);
            }
        }
        self.comparing = Some(Comparing::Reading {
            left,
            right,
            left_text,
            right_text,
        });
        self.read_compare(fx);
    }

    /// ⇄: the other file goes on the right, to edit.
    fn swap_compare(&mut self, fx: &mut Vec<Effect>) {
        let Some(review) = self.doc().review.clone().filter(|r| r.plugin == COMPARE) else {
            return;
        };
        let Some(Left::File(left)) = self.compare_left() else {
            return self.notify("Only two files can swap".into());
        };
        self.compare_replace = Some(self.doc().id);
        self.start_compare(Left::File(review.path.clone()), left, fx);
    }

    /// The Explorer's Select for Compare, and Compare with it.
    pub(crate) fn select_for_compare(&mut self, path: PathBuf) {
        let shown = name(&path);
        self.compare_selected = Some(path);
        self.notify(format!(
            "{shown} selected: right-click another file to compare"
        ));
    }

    pub(crate) fn compare_with_selected(&mut self, path: PathBuf, fx: &mut Vec<Effect>) {
        if let Some(left) = self.compare_selected.clone() {
            self.start_compare(Left::File(left), path, fx);
        }
    }
}
