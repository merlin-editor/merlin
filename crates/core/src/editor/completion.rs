//! The popup under the cursor: completions as a word is typed, the
//! signature of the call it's in, and the keys that work while it shows.

use super::*;

impl Editor {
    /// The text of the popup, if one is showing, for tests.
    pub fn popup_text(&self) -> Option<String> {
        match self.popup.as_ref()? {
            Popup::Hover {
                problems, lines, ..
            } => Some(
                problems
                    .iter()
                    .map(|p| p.message.as_str())
                    .chain(lines.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Popup::Actions { actions, .. } => Some(
                actions
                    .iter()
                    .map(|a| a.title.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Popup::Definition { card, .. } => card.lines.as_ref().map(|(_, l)| l.join("\n")),
            Popup::Completion { .. } => Some(
                self.completions()
                    .iter()
                    .map(|c| c.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
        }
    }

    pub(super) fn word_start(&self, pos: usize) -> usize {
        let rope = self.doc().buffer.rope();
        let mut start = pos;
        while start > 0 && {
            let c = rope.char(start - 1);
            c.is_alphanumeric() || c == '_'
        } {
            start -= 1;
        }
        start
    }

    /// Completions matching the word typed so far, best first.
    pub(super) fn completions(&self) -> Vec<&Completion> {
        let Some(Popup::Completion { items, start, .. }) = &self.popup else {
            return Vec::new();
        };
        let doc = self.doc();
        if doc.cursor < *start {
            return Vec::new();
        }
        let word = doc.slice(*start, doc.cursor).to_lowercase();
        let mut ranked: Vec<(i64, usize)> = items
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((fuzzy::score(&word, &c.label)?, i)))
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), i));
        ranked.into_iter().map(|(_, i)| &items[i]).collect()
    }

    /// Closes the completion list once nothing matches or the cursor left
    /// the word.
    pub(super) fn check_completion(&mut self) {
        let Some(Popup::Completion { start, .. }) = &self.popup else {
            return;
        };
        let doc = self.doc();
        let same_line =
            doc.cursor >= *start && doc.line_col(*start).0 == doc.line_col(doc.cursor).0;
        let count = self.completions().len();
        if !same_line || count == 0 || self.focus != Focus::Editor {
            self.popup = None;
        } else if let Some(Popup::Completion { selected, .. }) = &mut self.popup {
            *selected = (*selected).min(count - 1);
        }
    }

    pub(super) fn accept_completion(&mut self) {
        let chosen = match &self.popup {
            Some(Popup::Completion {
                selected, start, ..
            }) => self
                .completions()
                .get(*selected)
                .map(|c| (c.insert.clone(), *start)),
            _ => None,
        };
        self.popup = None;
        if let Some((insert, start)) = chosen {
            self.replace(start, self.cursor(), &insert, Kind::Other);
        }
    }

    /// Signature help goes when the cursor leaves the call's line or goes
    /// back before it, or keys go elsewhere.
    pub(super) fn check_signature(&mut self) {
        let Some((doc, at, _)) = &self.signature else {
            return;
        };
        let cursor = self.cursor();
        let d = self.doc();
        let gone = self.focus != Focus::Editor
            || d.id != *doc
            || cursor < *at
            || d.line_col(cursor).0 != d.line_col((*at).min(d.buffer.len())).0;
        if gone {
            self.signature = None;
        }
    }

    /// Handles a key while a popup shows. Returns whether it used the key.
    pub(super) fn popup_key(&mut self, key: Key) -> bool {
        let plain = !key.ctrl && !key.alt;
        if key.code == KeyCode::Esc && self.popup.is_none() && self.signature.take().is_some() {
            return true;
        }
        match &mut self.popup {
            None => false,
            Some(Popup::Hover { .. } | Popup::Definition { .. }) => {
                self.popup = None;
                key.code == KeyCode::Esc
            }
            // Its own keys are in `actions_key`, which can choose one.
            Some(Popup::Actions { .. }) => false,
            Some(Popup::Completion { selected, .. }) => match key.code {
                KeyCode::Up if plain => {
                    *selected = selected.saturating_sub(1);
                    true
                }
                KeyCode::Down if plain => {
                    *selected += 1;
                    true
                }
                KeyCode::Enter | KeyCode::Tab if plain => {
                    self.accept_completion();
                    true
                }
                KeyCode::Esc => {
                    self.popup = None;
                    true
                }
                // Typing narrows the list; check_completion closes it if
                // nothing's left.
                KeyCode::Char(_) | KeyCode::Backspace if plain => false,
                _ => {
                    self.popup = None;
                    false
                }
            },
        }
    }
}
