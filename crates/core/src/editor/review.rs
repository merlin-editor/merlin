//! A plugin's proposed change, shown as a diff to review before it's kept,
//! with keys to step through its changes.

use super::*;

impl Editor {
    pub(super) fn show_review(
        &mut self,
        plugin: String,
        id: String,
        path: PathBuf,
        old: &str,
        new: &str,
    ) {
        let refresh = plugin.is_empty() && self.diff_refresh.remove(&path);
        let back_to = match &self.doc().review {
            Some(review) => review.back_to,
            None => self.doc().id,
        };
        let doc_id = self.new_id();
        // Changes since the last commit show the whole file, to edit.
        let context = if plugin.is_empty() || plugin == compare_files::COMPARE {
            compare::WHOLE_FILE
        } else {
            3
        };
        let doc = Doc::review_in(
            doc_id,
            (plugin.clone(), id.clone(), path, back_to),
            old,
            new,
            context,
        );
        let same = |d: &Doc| {
            d.review
                .as_ref()
                .is_some_and(|r| r.plugin == plugin && r.id == id)
        };
        match self.docs.iter().position(same) {
            // The same changes again, as the file or the index changed:
            // the tab keeps its place, and focus stays where it is.
            Some(i) if refresh => {
                let (cursor, scroll) = (self.docs[i].cursor, self.docs[i].scroll);
                let id = self.docs[i].id;
                self.docs[i] = doc;
                self.docs[i].id = id;
                let len = self.docs[i].buffer.rope().len_chars();
                let line = self.docs[i].line_col(cursor.min(len)).0;
                self.docs[i].cursor = self.docs[i].line_start(line.min(self.docs[i].last_line()));
                self.docs[i].scroll = scroll.min(self.docs[i].last_line());
                self.colour_review(i);
                return;
            }
            Some(i) => {
                self.docs[i] = doc;
                self.current = i;
            }
            None => {
                self.current += 1;
                self.docs.insert(self.current, doc);
            }
        }
        self.colour_review(self.current);
        self.focus = Focus::Editor;
        self.popup = None;
        let doc = self.doc_mut();
        let line = doc.line_col(doc.cursor).0;
        doc.scroll = line.saturating_sub(3);
    }

    pub(super) fn close_review(&mut self, plugin: &str, id: &str) {
        let Some(i) = self.docs.iter().position(|d| {
            d.review
                .as_ref()
                .is_some_and(|r| r.plugin == plugin && r.id == id)
        }) else {
            return;
        };
        let back_to = self.docs[i].review.as_ref().map(|r| r.back_to);
        let was_current = i == self.current;
        self.docs.remove(i);
        if self.docs.is_empty() {
            let id = self.new_id();
            self.docs.push(Doc::new(id, "", None));
        }
        if i < self.current || self.current >= self.docs.len() {
            self.current = self.current.saturating_sub(1);
        }
        if was_current {
            // Another review waiting comes next; else where the user was.
            if let Some(next) = self.docs.iter().position(|d| d.review.is_some()) {
                self.current = next;
            } else if let Some(j) = self.docs.iter().position(|d| Some(d.id) == back_to) {
                self.current = j;
            }
        }
    }

    /// Keys in a review. Returns whether the key was used; moving around
    /// and copying work as in any file.
    pub(super) fn review_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        use KeyCode::*;
        let Some(review) = &self.doc().review else {
            return false;
        };
        if review.plugin == rename::RENAME {
            return self.rename_review_key(key, fx);
        }
        let (plugin, id, back_to) = (review.plugin.clone(), review.id.clone(), review.back_to);
        let answer = |accepted: bool| Effect::PluginReview {
            plugin: plugin.clone(),
            id: id.clone(),
            accepted,
        };
        if key.ctrl || key.alt {
            return false;
        }
        // A commit's changes are only looked at: Esc goes back.
        if plugin == blame::COMMIT {
            if key.code == Esc {
                if let Some(i) = self.docs.iter().position(|d| d.id == back_to) {
                    self.current = i;
                }
                return true;
            }
            return !matches!(
                key.code,
                Left | Right | Up | Down | Home | End | PageUp | PageDown
            );
        }
        // Changes since the last commit edit their file, which keys that
        // type reach first; Esc goes back.
        if plugin.is_empty() {
            match key.code {
                Esc => {
                    if let Some(i) = self.docs.iter().position(|d| d.id == back_to) {
                        self.current = i;
                    }
                }
                _ => return false,
            }
            return true;
        }
        match key.code {
            Char('y') | Enter => {
                fx.push(answer(true));
                self.close_review(&plugin, &id);
                self.notify("Accepted".into());
            }
            Char('n') => {
                fx.push(answer(false));
                self.close_review(&plugin, &id);
                self.notify("Rejected".into());
            }
            Char(']') => self.next_change(true),
            Char('[') => self.next_change(false),
            Esc => {
                if let Some(i) = self.docs.iter().position(|d| d.id == back_to) {
                    self.current = i;
                }
            }
            Left | Right | Up | Down | Home | End | PageUp | PageDown => return false,
            _ => self.notify(format!("Review: {}", self.review_hint())),
        }
        true
    }

    /// The keys a review takes.
    pub(crate) fn review_hint(&self) -> &'static str {
        match &self.doc().review {
            Some(r) if r.plugin.is_empty() => {
                "Alt+S stage · Alt+R revert · Alt+F5 next change · Esc back"
            }
            Some(r) if r.plugin == rename::RENAME => {
                "Reviewing a rename: Enter applies, Esc discards"
            }
            Some(r) if r.plugin == blame::COMMIT => "A commit's changes · Esc back",
            _ => "y accept · n reject · ] next change · Esc later",
        }
    }

    /// Moves to the next or previous run of changed lines in a review.
    pub(super) fn next_change(&mut self, forward: bool) {
        let doc = self.doc();
        let Some(review) = &doc.review else { return };
        let changed = |i: usize| {
            review
                .lines
                .get(i)
                .is_some_and(|(k, _)| matches!(k, LineKind::Added | LineKind::Removed))
        };
        let line = doc.line_col(doc.cursor).0;
        let starts: Vec<usize> = (0..review.lines.len())
            .filter(|&i| changed(i) && (i == 0 || !changed(i - 1)))
            .collect();
        let target = if forward {
            starts.into_iter().find(|&i| i > line)
        } else {
            starts.into_iter().rev().find(|&i| i < line)
        };
        match target {
            Some(i) => {
                let pos = self.doc().line_start(i);
                self.set_cursor(pos);
            }
            None => self.notify("No more changes".into()),
        }
    }
}
