//! Formatting with the language server: the file (Ctrl+Shift+I), the
//! selection (Ctrl+K Ctrl+F), and on save. A save waits for the server's
//! edits, at most [`FORMAT_WAIT_MS`], and always saves; the window keeps
//! the time and says when it's up with [`Event::FormatTimedOut`].
//!
//! [`Event::FormatTimedOut`]: super::Event::FormatTimedOut

use std::path::PathBuf;

use super::{Editor, Effect};
use crate::lang::FileEdit;

/// How long a save waits for the language server's formatting.
pub const FORMAT_WAIT_MS: u64 = 1500;

/// What saving formats first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormatOnSave {
    #[default]
    Off,
    File,
    /// Only the lines git says changed.
    ChangedLines,
}

/// A save waiting for its file's formatting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Waiting {
    pub(crate) doc: u64,
    pub(crate) path: PathBuf,
    /// The text the server formats, so edits for older text are dropped.
    pub(crate) revision: u64,
}

impl Editor {
    /// Whether the language plugin can format the active file.
    fn can_format(&self) -> bool {
        let doc = self.doc();
        doc.review.is_none()
            && doc.page.is_none()
            && self.plugin_language().is_some()
            && self.has_lsp("format")
    }

    /// Ctrl+Shift+I: formats the file.
    pub(crate) fn format_file(&mut self, fx: &mut Vec<Effect>) {
        self.ask_format("format", fx);
    }

    /// Ctrl+K Ctrl+F: formats what's selected.
    pub(crate) fn format_selection(&mut self, fx: &mut Vec<Effect>) {
        if self.doc().selection().is_none() {
            return self.notify("Select what to format first".into());
        }
        self.ask_format("format-selection", fx);
    }

    fn ask_format(&mut self, command: &str, fx: &mut Vec<Effect>) {
        if !self.has_lsp("format") {
            return;
        }
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: command.into(),
        });
    }

    /// Ctrl+S: saves, formatting first as the settings ask. The save waits
    /// for the server's edits.
    pub(crate) fn save_formatted(&mut self, fx: &mut Vec<Effect>) {
        let command = match self.format_on_save {
            FormatOnSave::Off => None,
            FormatOnSave::File => Some("format-on-save"),
            FormatOnSave::ChangedLines => Some("format-changed"),
        };
        let (Some(command), true) = (command, self.can_format()) else {
            return self.save_current(fx);
        };
        let doc = self.doc();
        let Some(path) = doc.buffer.path.clone() else {
            return self.save_current(fx);
        };
        self.waiting_format = Some(Waiting {
            doc: doc.id,
            path,
            revision: doc.buffer.revision(),
        });
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: command.into(),
        });
    }

    /// The active file's lines changed since git's copy, as first and last
    /// line, for formatting only those; none when it isn't in git.
    pub fn changed_lines(&self) -> Option<Vec<(u32, u32)>> {
        use crate::diff::Sign;
        let doc = self.doc();
        doc.git_base.as_ref()?;
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for (line, sign) in doc.git_signs.iter().enumerate() {
            if !matches!(sign, Some(Sign::Added | Sign::Changed)) {
                continue;
            }
            let line = line as u32;
            match ranges.last_mut() {
                Some((_, end)) if *end + 1 == line => *end = line,
                _ => ranges.push((line, line)),
            }
        }
        Some(ranges)
    }

    /// The document a save is waiting to format, for the window to time.
    pub fn formatting(&self) -> Option<u64> {
        self.waiting_format.as_ref().map(|w| w.doc)
    }

    /// Edits came for the file a save waits on: they're its formatting, so
    /// it's saved once they're made. Returns the edits left for others.
    pub(crate) fn format_arrived(
        &mut self,
        edits: Vec<FileEdit>,
        fx: &mut Vec<Effect>,
    ) -> Vec<FileEdit> {
        let Some(waiting) = self.waiting_format.clone() else {
            return edits;
        };
        let (mine, others): (Vec<FileEdit>, Vec<FileEdit>) =
            edits.into_iter().partition(|f| f.path == waiting.path);
        if mine.is_empty() {
            return others;
        }
        self.waiting_format = None;
        let Some(i) = self.docs.iter().position(|d| d.id == waiting.doc) else {
            return others;
        };
        // Typing since it asked would make the edits land wrong.
        if self.docs[i].buffer.revision() == waiting.revision {
            for file in &mine {
                self.edit_file(file);
            }
        }
        // A notice since the save began says why it didn't format; the
        // usual "Saved" mustn't hide it.
        if self.message.is_some() {
            self.saving.quiet.insert(waiting.doc);
        }
        self.finish_save(waiting.doc, fx);
        others
    }

    /// The window's time for formatting is up: the file saves as it is.
    pub(crate) fn format_timed_out(&mut self, fx: &mut Vec<Effect>) {
        let Some(waiting) = self.waiting_format.take() else {
            return;
        };
        // Its notice says it saved, instead of the usual one.
        self.saving.quiet.insert(waiting.doc);
        self.finish_save(waiting.doc, fx);
        self.notify(format!(
            "Saved without formatting: the language server didn't answer in {:.1} s",
            FORMAT_WAIT_MS as f64 / 1000.0
        ));
        self.add_action("Settings", "settings.open");
    }

    /// Saves the document with id `doc`, tidied, if it's still open.
    fn finish_save(&mut self, doc: u64, fx: &mut Vec<Effect>) {
        if self.doc().id == doc {
            return self.save_current(fx);
        }
        if let Some(d) = self.docs.iter().find(|d| d.id == doc)
            && let Some(path) = &d.buffer.path
        {
            fx.push(Effect::Save {
                doc: d.id,
                path: path.clone(),
                text: d.buffer.text(),
                revision: d.buffer.revision(),
            });
        }
    }
}
