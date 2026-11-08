//! Opening files: in tabs or the preview tab, at a line, the editor's own
//! pages, going back, and files that change on disk.

use super::*;

impl Editor {
    /// The open document of `path`, if any.
    pub(super) fn doc_at(&mut self, path: &std::path::Path) -> Option<&mut Doc> {
        self.docs
            .iter_mut()
            .find(|d| d.review.is_none() && d.buffer.path.as_deref() == Some(path))
    }

    /// Switches to `path` if it's open, or asks the host to read it.
    pub(super) fn open(&mut self, path: PathBuf, fx: &mut Vec<Effect>) {
        // A file a plugin draws, such as a PDF, opens as its pages.
        if let Some(editor) = self.doc_editor_for(&path) {
            return self.open_surface(path, editor, fx);
        }
        match self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            Some(i) => {
                self.current = i;
                self.focus = Focus::Editor;
            }
            None => fx.push(Effect::Open(path)),
        }
    }

    /// Opens a file as the preview tab, replacing the last preview, unless
    /// it's open already.
    pub(crate) fn open_preview(&mut self, path: PathBuf, fx: &mut Vec<Effect>) {
        let open = self
            .docs
            .iter()
            .any(|d| d.buffer.path.as_ref() == Some(&path));
        if !open {
            self.opening_preview = Some(path.clone());
        }
        self.open(path, fx);
    }

    /// Keeps the current tab open, if it's the preview.
    pub(crate) fn keep_open(&mut self) {
        if self.preview == Some(self.doc().id) {
            self.preview = None;
        }
    }

    pub(super) fn opened(&mut self, path: PathBuf, text: &str) {
        // A changes tab's file opens as the tab after it, and the changes
        // tab stays shown.
        if self.opening_beside.as_ref() == Some(&path) {
            self.opening_beside = None;
            if !self
                .docs
                .iter()
                .any(|d| d.buffer.path.as_ref() == Some(&path))
            {
                let doc = Doc::new(self.new_id(), text, Some(path.clone()));
                self.docs.insert(self.current + 1, doc);
                self.place_marks(&path);
            }
            return;
        }
        // The palette's search keeps the keys while it previews a file.
        if !self.search_open() {
            self.focus = Focus::Editor;
        }
        let as_preview = self.opening_preview.take().is_some_and(|p| p == path);
        if let Some(i) = self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            self.current = i;
            return;
        }
        let doc = Doc::new(self.new_id(), text, Some(path.clone()));
        let id = doc.id;
        // The new preview takes the old one's place, if this group has it.
        let old = self
            .preview
            .filter(|old| as_preview && self.groups[self.group].tabs.contains(old))
            .and_then(|old| self.index_of(old));
        if as_preview {
            self.preview = Some(id);
        }
        if let Some(i) = old {
            let old_id = self.docs[i].id;
            self.docs[i] = doc;
            self.current = i;
            let group = &mut self.groups[self.group];
            for tab in &mut group.tabs {
                if *tab == old_id {
                    *tab = id;
                }
            }
            group.saved.remove(&old_id);
            if group.active == old_id {
                group.active = id;
            }
            self.shown = (self.group, id);
        } else if self.doc().is_blank() {
            self.docs[self.current] = doc;
        } else {
            self.current += 1;
            self.docs.insert(self.current, doc);
        }
        self.place_marks(&path);
        if let Some((goto, line, column)) = self.pending_goto.take() {
            if goto == path {
                self.go_to(line, column);
            } else {
                self.pending_goto = Some((goto, line, column));
            }
        }
    }

    /// Places every plugin's diagnostics for `path` in its document, if
    /// it's open.
    pub(super) fn place_marks(&mut self, path: &PathBuf) {
        let Some(doc) = self
            .docs
            .iter_mut()
            .find(|d| d.buffer.path.as_ref() == Some(path))
        else {
            return;
        };
        let mut marks: Vec<Mark> = self
            .diagnostics
            .iter()
            .filter(|((_, p), _)| p == path)
            .flat_map(|((source, _), list)| list.iter().map(move |d| (source, d)))
            .map(|(source, d)| {
                let start = doc.pos_at_utf16(d.start.0, d.start.1);
                // A task's problem points at a place: it covers the word.
                let mut end = d.end;
                if source.starts_with("task:")
                    && d.end == (d.start.0, d.start.1 + 1)
                    && (d.start.0 as usize) < doc.buffer.rope().len_lines()
                {
                    let line: String = doc.buffer.rope().line(d.start.0 as usize).chars().collect();
                    end.1 = matcher::word_end(&line, d.start.1);
                }
                Mark {
                    start,
                    end: doc.pos_at_utf16(end.0, end.1).max(start),
                    severity: d.severity,
                    message: d.message.clone(),
                    source: d.source.clone(),
                    code: d.code.clone(),
                    link: d.link.clone(),
                }
            })
            .collect();
        marks.sort_by_key(|m| (m.start, m.severity));
        doc.marks = marks;
    }

    pub(super) fn open_at(&mut self, path: PathBuf, line: u32, column: u32, fx: &mut Vec<Effect>) {
        self.jumps.push((self.doc().id, self.cursor()));
        self.focus = Focus::Editor;
        match self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(&path))
        {
            Some(i) => {
                self.current = i;
                self.go_to(line, column);
            }
            None => {
                self.pending_goto = Some((path.clone(), line, column));
                fx.push(Effect::Open(path));
            }
        }
    }

    /// Moves to an LSP-style position in the current file, with it in the
    /// middle of the view.
    pub(super) fn go_to(&mut self, line: u32, column: u32) {
        let pos = self.doc().pos_at_utf16(line, column);
        self.set_cursor(pos);
        let half = self.text_height() / 2;
        let doc = self.doc_mut();
        doc.scroll = (line as usize).saturating_sub(half);
    }

    pub(super) fn go_back(&mut self) {
        while let Some((id, pos)) = self.jumps.pop() {
            if let Some(i) = self.docs.iter().position(|d| d.id == id) {
                self.current = i;
                self.set_cursor(pos);
                self.focus = Focus::Editor;
                return;
            }
        }
        self.notify("Nowhere to go back to".into());
    }

    /// Opens a page, such as a plugin's, in a tab of its own, or goes to
    /// the tab it's in.
    pub(crate) fn open_page(&mut self, name: &str, title: &str) {
        match self
            .docs
            .iter()
            .position(|d| d.page.as_ref().is_some_and(|(n, _)| n == name))
        {
            Some(i) => self.current = i,
            None => {
                let doc = Doc::page(self.new_id(), name, title);
                self.current = (self.current + 1).min(self.docs.len());
                self.docs.insert(self.current, doc);
            }
        }
        self.focus = Focus::Editor;
        self.popup = None;
        self.sync_groups();
    }

    /// Opens the Settings tab, at a plugin's page if one is named.
    pub(crate) fn open_settings(&mut self, plugin: Option<String>, fx: &mut Vec<Effect>) {
        self.open_page(SETTINGS_PAGE, "Settings");
        fx.push(Effect::OpenSettingsWindow {
            plugin,
            plugins: false,
        });
    }

    pub(super) fn file_changed(&mut self, path: &PathBuf, text: &str) {
        let Some(i) = self
            .docs
            .iter()
            .position(|d| d.buffer.path.as_ref() == Some(path))
        else {
            return;
        };
        let doc = &mut self.docs[i];
        let old = doc.buffer.text();
        if old == text {
            if doc.buffer.is_dirty() {
                doc.buffer.mark_saved(doc.buffer.revision());
            }
            return;
        }
        if doc.buffer.is_dirty() {
            let id = doc.id;
            // Changed again while asked: the newest text is the choice.
            if let Some(dialog::Dialog {
                ask: dialog::Ask::Conflict(asked, disk),
                ..
            }) = &mut self.dialog
                && *asked == id
            {
                *disk = text.to_string();
                return;
            }
            // Another question is open: it isn't pushed aside.
            if self.dialog.is_some() {
                let name = doc.name();
                return self.error(format!(
                    "{name} changed on disk, and has unsaved changes here. Saving will overwrite it."
                ));
            }
            return self.ask(dialog::Ask::Conflict(id, text.to_string()));
        }
        replace_text(doc, &old, text);
        doc.buffer.mark_saved(doc.buffer.revision());
        let name = doc.name();
        // A discard from Source Control says so itself, with Undo, and a
        // notice with something to do, such as a pull's conflicts, stays.
        let discarded = self
            .git
            .discarded
            .as_ref()
            .is_some_and(|(p, _)| doc.buffer.path.as_deref() == Some(std::path::Path::new(p)));
        let asking = self.message.as_ref().is_some_and(|m| !m.actions.is_empty());
        if !discarded && !asking {
            self.notify(format!("Reloaded {name}, changed on disk"));
        }
    }
}
