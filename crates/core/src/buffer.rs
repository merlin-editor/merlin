use std::path::PathBuf;

use merlin_syntax::TextEdit;
use ropey::Rope;

use crate::edit::Edit;
use crate::history::{History, Kind, Revision};

/// One change to the text, for whatever tracks positions in it: char
/// offsets for diagnostics, and bytes and rows for the syntax tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: usize,
    pub removed: usize,
    pub inserted: usize,
    pub edit: TextEdit,
}

impl Change {
    /// Where `pos` ends up. Positions inside removed text move to its start.
    pub fn map(&self, pos: usize) -> usize {
        if pos >= self.at + self.removed {
            pos - self.removed + self.inserted
        } else {
            pos.min(self.at)
        }
    }
}

pub struct Buffer {
    rope: Rope,
    pub path: Option<PathBuf>,
    history: History,
    saved: Revision,
    version: u64,
    /// Changes since [`Buffer::take_changes`] was last called.
    changes: Vec<Change>,
}

impl Buffer {
    pub fn new(text: &str, path: Option<PathBuf>) -> Self {
        Self {
            rope: Rope::from_str(text),
            path,
            history: History::default(),
            saved: 0,
            version: 0,
            changes: Vec::new(),
        }
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    pub fn len(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The undo revision. Typing can extend a revision, so use
    /// [`Buffer::version`] to notice that the text changed.
    pub fn revision(&self) -> Revision {
        self.history.revision()
    }

    /// Goes up on every change to the text, including undo and redo.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn is_dirty(&self) -> bool {
        self.revision() != self.saved
    }

    /// Records that `revision` is what's on disk. Saving is asynchronous, so
    /// this is the revision the save started from, not the current one.
    pub fn mark_saved(&mut self, revision: Revision) {
        self.saved = revision;
    }

    pub fn begin(&mut self, cursor: usize, kind: Kind) {
        self.history.begin(cursor, kind);
    }

    pub fn apply(&mut self, edit: Edit) {
        self.change(&edit);
        self.history.record(edit);
        self.version += 1;
    }

    /// Applies an edit and records it as a [`Change`].
    fn change(&mut self, edit: &Edit) {
        let rope = &self.rope;
        let start_byte = rope.char_to_byte(edit.at);
        let row = rope.char_to_line(edit.at);
        let col = start_byte - rope.line_to_byte(row);
        let end = |text: &str| match text.rfind('\n') {
            Some(i) => (row + text.matches('\n').count(), text.len() - i - 1),
            None => (row, col + text.len()),
        };
        self.changes.push(Change {
            at: edit.at,
            removed: edit.removed.chars().count(),
            inserted: edit.inserted.chars().count(),
            edit: TextEdit {
                start_byte,
                old_end_byte: start_byte + edit.removed.len(),
                new_end_byte: start_byte + edit.inserted.len(),
                start: (row, col),
                old_end: end(&edit.removed),
                new_end: end(&edit.inserted),
            },
        });
        edit.apply(&mut self.rope);
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    pub fn commit(&mut self, cursor: usize) {
        self.history.commit(cursor, self.saved);
    }

    /// Reverts the newest change and returns where the cursor was before it.
    pub fn undo(&mut self) -> Option<usize> {
        let change = self.history.undo()?;
        for edit in change.edits.iter().rev() {
            self.change(&edit.inverse());
        }
        self.version += 1;
        Some(change.cursor_before)
    }

    pub fn redo(&mut self) -> Option<usize> {
        let change = self.history.redo()?;
        for edit in &change.edits {
            self.change(edit);
        }
        self.version += 1;
        Some(change.cursor_after)
    }

    /// Length of a line in chars, without its line break.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }
}
