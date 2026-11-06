use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use merlin_syntax::TextEdit;
use ropey::Rope;

use crate::doc::Caret;
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

/// One change as language servers count it: lines and UTF-16 columns in
/// the text as it was just before it, and what replaced that range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub text: String,
}

/// Most changes kept for plugins that sync as they go, and most text in
/// them; past either, the oldest go, and a plugin that's behind reads the
/// whole text instead.
const LOG_CHANGES: usize = 512;
const LOG_BYTES: usize = 1 << 20;

/// The text and its recent changes, shared rather than copied, so taking
/// one for a plugin call costs the same for any size of file.
#[derive(Clone, Debug, Default)]
pub struct TextSnapshot {
    rope: Rope,
    version: u64,
    log: Arc<VecDeque<(u64, Arc<TextChange>)>>,
    /// The oldest version the log goes back to.
    log_from: u64,
    /// From a buffer, rather than made up of some text: only then do
    /// versions and changes mean anything.
    versioned: bool,
}

impl TextSnapshot {
    /// The buffer's version, if it's from one.
    pub fn version(&self) -> Option<u64> {
        self.versioned.then_some(self.version)
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    /// The changes since `version`, oldest first; `None` if the log no
    /// longer goes back that far.
    pub fn changes_since(&self, version: u64) -> Option<Vec<TextChange>> {
        if !self.versioned || version > self.version || version < self.log_from {
            return None;
        }
        Some(
            self.log
                .iter()
                .filter(|(v, _)| *v > version)
                .map(|(_, c)| TextChange::clone(c))
                .collect(),
        )
    }
}

impl std::fmt::Display for TextSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for chunk in self.rope.chunks() {
            f.write_str(chunk)?;
        }
        Ok(())
    }
}

impl From<&str> for TextSnapshot {
    fn from(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            ..Self::default()
        }
    }
}

impl From<String> for TextSnapshot {
    fn from(text: String) -> Self {
        Self::from(text.as_str())
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
    /// Recent changes by the version they made, for plugins.
    log: Arc<VecDeque<(u64, Arc<TextChange>)>>,
    log_from: u64,
    log_bytes: usize,
}

impl Buffer {
    pub fn new(text: &str, path: Option<PathBuf>) -> Self {
        // Each buffer counts its versions from a start of its own, so a
        // version a plugin kept from a file closed since can't be taken
        // for one of this buffer's.
        static BUFFERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let start = BUFFERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) << 32;
        Self {
            rope: Rope::from_str(text),
            path,
            history: History::default(),
            saved: 0,
            version: start,
            changes: Vec::new(),
            log: Arc::default(),
            log_from: start,
            log_bytes: 0,
        }
    }

    /// The text and its recent changes, for a plugin call.
    pub fn snapshot(&self) -> TextSnapshot {
        TextSnapshot {
            rope: self.rope.clone(),
            version: self.version,
            log: self.log.clone(),
            log_from: self.log_from,
            versioned: true,
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
        self.history.begin(vec![Caret::at(cursor)], kind);
    }

    /// Starts an undo step at several cursors, the primary one first.
    pub fn begin_at(&mut self, cursors: Vec<Caret>, kind: Kind) {
        self.history.begin(cursors, kind);
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
        // As LSP counts: lines, and UTF-16 units into them.
        let utf16 = |at: usize| {
            let line = rope.char_to_line(at);
            let column = rope.char_to_utf16_cu(at) - rope.char_to_utf16_cu(rope.line_to_char(line));
            (line as u32, column as u32)
        };
        let logged = TextChange {
            start: utf16(edit.at),
            end: utf16(edit.at + edit.removed.chars().count()),
            text: edit.inserted.clone(),
        };
        self.log_bytes += logged.text.len();
        // Undo and redo make one version of several edits.
        let log = Arc::make_mut(&mut self.log);
        log.push_back((self.version + 1, Arc::new(logged)));
        while log.len() > LOG_CHANGES || (self.log_bytes > LOG_BYTES && log.len() > 1) {
            if let Some((version, gone)) = log.pop_front() {
                self.log_bytes -= gone.text.len();
                self.log_from = version;
            }
        }
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

    /// How many undo steps there are, for [`Buffer::join_undo`].
    pub fn undo_steps(&self) -> usize {
        self.history.steps()
    }

    /// Makes the undo steps from `from` on one.
    pub fn join_undo(&mut self, from: usize) {
        self.history.join_from(from, self.saved);
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    pub fn commit(&mut self, cursor: usize) {
        self.history.commit(vec![Caret::at(cursor)], self.saved);
    }

    /// Ends an undo step at several cursors, the primary one first.
    pub fn commit_at(&mut self, cursors: Vec<Caret>) {
        self.history.commit(cursors, self.saved);
    }

    /// Reverts the newest change and returns the cursors and selections
    /// from before it, the primary one first.
    pub fn undo(&mut self) -> Option<Vec<Caret>> {
        let change = self.history.undo()?;
        for edit in change.edits.iter().rev() {
            self.change(&edit.inverse());
        }
        self.version += 1;
        Some(change.cursors_before)
    }

    pub fn redo(&mut self) -> Option<Vec<Caret>> {
        let change = self.history.redo()?;
        for edit in &change.edits {
            self.change(edit);
        }
        self.version += 1;
        Some(change.cursors_after)
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
