//! The name under the cursor's other uses, from its language server's
//! `documentHighlight`: where it's read and where it's written, lit up
//! while the cursor rests on one of them, and marked in the scrollbar.
//! Without a server, the editor marks the same word instead
//! ([`super::guides`]).

use std::path::Path;

use super::{Editor, Effect, Focus};

/// A use as a language server sends it: UTF-16 start and end, and
/// whether it writes the name.
pub type HighlightRange = ((u32, u32), (u32, u32), bool);

/// One file's uses, as of its text then.
pub(crate) struct Highlights {
    doc: u64,
    version: u64,
    /// Character ranges, and whether each writes the name.
    pub(crate) ranges: Vec<(usize, usize, bool)>,
}

impl Editor {
    /// The word the cursor is on: its start and end.
    fn name_at_cursor(&self) -> Option<(usize, usize)> {
        let doc = self.doc();
        let v = self.view();
        if self.focus != Focus::Editor || v.selection().is_some() || doc.review.is_some() {
            return None;
        }
        let (line, col) = doc.line_col(v.cursor);
        let chars = doc.line_chars(line);
        let at = |i: usize| chars.get(i).copied().is_some_and(super::guides::is_word);
        let mut a = col;
        if !at(a) && a > 0 && at(a - 1) {
            a -= 1;
        }
        if !at(a) {
            return None;
        }
        while a > 0 && at(a - 1) {
            a -= 1;
        }
        let mut b = a;
        while at(b) {
            b += 1;
        }
        let start = doc.line_start(line);
        Some((start + a, start + b))
    }

    /// Asks the language server for the uses of the name the cursor came
    /// to: once per name and text, and not while the cursor stays on one of
    /// the uses it has.
    pub(crate) fn ask_highlights(&mut self, fx: &mut Vec<Effect>) {
        let Some((start, _)) = self.name_at_cursor() else {
            self.highlights = None;
            self.highlight_asked = None;
            return;
        };
        if self.current_highlights().is_some() {
            return;
        }
        let doc = self.doc();
        let key = (doc.id, doc.buffer.version(), start);
        if self.highlight_asked == Some(key) {
            return;
        }
        self.highlights = None;
        self.highlight_asked = Some(key);
        // Any plugin that finds references speaks to a language server.
        if let Some(plugin) = self.lsp_plugin("references") {
            fx.push(Effect::PluginCommand {
                plugin,
                command: "highlight".into(),
            });
        }
    }

    /// A language server's answer, for the file shown.
    pub(crate) fn highlights_came(&mut self, path: &Path, ranges: Vec<HighlightRange>) {
        let doc = self.doc();
        if !doc
            .buffer
            .path
            .as_deref()
            .is_some_and(|p| self.same_file(p, path))
            || ranges.is_empty()
        {
            self.highlights = None;
            return;
        }
        let ranges = ranges
            .into_iter()
            .map(|((sl, sc), (el, ec), write)| {
                (doc.pos_at_utf16(sl, sc), doc.pos_at_utf16(el, ec), write)
            })
            .filter(|(a, b, _)| a < b)
            .collect();
        self.highlights = Some(Highlights {
            doc: doc.id,
            version: doc.buffer.version(),
            ranges,
        });
    }

    /// The uses lit up now: the shown file's, as of its text, while the
    /// cursor is on one of them.
    pub(crate) fn current_highlights(&self) -> Option<&[(usize, usize, bool)]> {
        let h = self.highlights.as_ref()?;
        let doc = self.doc();
        let cursor = self.view().cursor;
        (h.doc == doc.id
            && h.version == doc.buffer.version()
            && h.ranges.iter().any(|&(a, b, _)| (a..=b).contains(&cursor)))
        .then_some(h.ranges.as_slice())
    }
}
