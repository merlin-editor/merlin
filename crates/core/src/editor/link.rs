//! Ctrl over a name: the name becomes a link to its definition, a rest
//! shows the definition in a card, and Ctrl+click goes there, as in VS
//! Code.

use std::path::PathBuf;

use super::{Editor, Effect, Focus, Popup};
use crate::lang::Location;

/// How long the pointer stops on a new name, with Ctrl down, before the
/// language server is asked about it.
pub(crate) const LINK_WAIT_MS: u64 = 60;

/// Lines of a definition the card shows, and of its doc comment above.
const CARD_LINES: usize = 6;
const DOC_LINES: usize = 3;

/// The name under the pointer while Ctrl is held.
pub(crate) struct Link {
    doc: u64,
    /// The name's first and last character.
    word: (usize, usize),
    tick: u64,
    asked: bool,
    /// Where it's defined, once the server says.
    places: Option<Vec<Location>>,
    /// A click came before the answer: where to go when it comes, to the
    /// side or not.
    jump: Option<bool>,
}

/// A definition in the card that a rest with Ctrl brings up.
pub(crate) struct DefinitionCard {
    pub path: PathBuf,
    /// The definition's line, from 0.
    pub line: usize,
    /// Its text, once read: the first line shown, and the lines.
    pub lines: Option<(usize, Vec<String>)>,
}

impl Editor {
    /// The name at screen cell (`x`, `y`), if there is one.
    fn name_at(&self, x: u16, y: u16) -> Option<(usize, usize)> {
        let text = self.layout().text;
        if !text.contains(x, y) || (x as usize) < text.x as usize + self.gutter() {
            return None;
        }
        let pos = self.pos_at_screen(x, y);
        let doc = self.doc();
        let (start, end) = (doc.word_start_at(pos), doc.word_end_at(pos));
        (start < end && (start..end).contains(&pos)).then_some((start, end))
    }

    /// The pointer moved over the text, with Ctrl down or not.
    pub(crate) fn link_moved(&mut self, x: u16, y: u16, ctrl: bool) {
        let word = ctrl
            .then(|| self.name_at(x, y))
            .flatten()
            .filter(|_| self.focus == Focus::Editor && self.has_lsp("definition"));
        let Some(word) = word else {
            self.link = None;
            if matches!(self.popup, Some(Popup::Definition { .. })) {
                self.popup = None;
                self.hover_anchor = None;
            }
            return;
        };
        let doc = self.doc().id;
        if self
            .link
            .as_ref()
            .is_some_and(|l| l.doc == doc && l.word == word)
        {
            return;
        }
        self.link_tick += 1;
        self.link = Some(Link {
            doc,
            word,
            tick: self.link_tick,
            asked: false,
            places: None,
            jump: None,
        });
    }

    /// How long to wait before asking about the name under Ctrl, and the
    /// tick to send back as [`super::Event::LinkDue`].
    pub fn link_wait(&self) -> Option<(u64, u64)> {
        let link = self.link.as_ref().filter(|l| !l.asked)?;
        Some((link.tick, LINK_WAIT_MS))
    }

    /// The pointer stayed on the name: asks where it's defined.
    pub(crate) fn link_due(&mut self, tick: u64, fx: &mut Vec<Effect>) {
        if let Some(link) = self.link.as_mut().filter(|l| l.tick == tick && !l.asked) {
            link.asked = true;
            self.ask_link(fx);
        }
    }

    fn ask_link(&mut self, fx: &mut Vec<Effect>) {
        self.link_asking = true;
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: "definition-link".into(),
        });
    }

    /// Where plugins see the cursor while the link is asked about.
    pub(crate) fn link_cursor(&self) -> Option<usize> {
        self.link
            .as_ref()
            .filter(|_| self.link_asking)
            .map(|l| l.word.0)
    }

    /// The server's answer.
    pub(crate) fn definitions_came(&mut self, places: Vec<Location>, fx: &mut Vec<Effect>) {
        self.link_asking = false;
        let Some(link) = self.link.as_mut().filter(|l| l.places.is_none()) else {
            return;
        };
        link.places = Some(places);
        if let Some(side) = link.jump.take() {
            self.link_go(side, fx);
        }
    }

    /// The name drawn as a link: its first and last character, in the
    /// current file.
    pub(crate) fn link_span(&self) -> Option<(usize, usize)> {
        self.link
            .as_ref()
            .filter(|l| l.doc == self.doc().id && l.places.as_ref().is_some_and(|p| !p.is_empty()))
            .map(|l| l.word)
    }

    /// Ctrl+click at (`x`, `y`): goes to the definition, at once if the
    /// server has said where, else when it does.
    pub(crate) fn link_click(&mut self, x: u16, y: u16, side: bool, fx: &mut Vec<Effect>) {
        let Some(word) = self.name_at(x, y) else {
            return;
        };
        if !self.has_lsp("definition") {
            return;
        }
        self.popup = None;
        self.hover_anchor = None;
        let doc = self.doc().id;
        let known = self
            .link
            .as_ref()
            .is_some_and(|l| l.doc == doc && l.word == word && l.places.is_some());
        if known {
            return self.link_go(side, fx);
        }
        self.link_tick += 1;
        self.link = Some(Link {
            doc,
            word,
            tick: self.link_tick,
            asked: true,
            places: None,
            jump: Some(side),
        });
        self.ask_link(fx);
    }

    /// Goes to the link's definition: the one place, or the peek view for
    /// several. On the definition itself, it finds the references instead.
    fn link_go(&mut self, side: bool, fx: &mut Vec<Effect>) {
        let Some(link) = &self.link else {
            return;
        };
        let (word, places) = (link.word, link.places.clone().unwrap_or_default());
        self.link = None;
        let doc = self.doc();
        let here = doc.buffer.path.clone();
        let own = places.len() == 1
            && here.as_ref() == Some(&places[0].path)
            && (word.0..=word.1).contains(&doc.pos_at_utf16(places[0].start.0, places[0].start.1));
        let asked = (doc.id, doc.line_col(word.0).0, doc.slice(word.0, word.1));
        match places.as_slice() {
            [] => {}
            _ if own => {
                self.set_cursor(word.0);
                self.find_references(fx)
            }
            [place] => {
                let place = place.clone();
                if side {
                    self.run("view.split", fx);
                }
                self.open_at(place.path, place.start.0, place.start.1, fx);
            }
            _ => {
                self.peek_asked = Some(asked);
                self.show_references(places, fx);
            }
        }
    }

    /// A rest with Ctrl on the link: its definition in a card.
    pub(crate) fn link_rest(&mut self, fx: &mut Vec<Effect>) {
        let Some(link) = &self.link else {
            return;
        };
        let Some(place) = link.places.as_ref().and_then(|p| p.first()) else {
            return;
        };
        let word = link.word;
        let (path, line) = (place.path.clone(), place.start.0 as usize);
        let lines = match self.doc_at(&path) {
            Some(doc) => Some(card_lines(&doc.buffer.text(), line)),
            None => {
                fx.push(Effect::ReadFiles(vec![path.clone()]));
                None
            }
        };
        self.hover_anchor = Some(word.0);
        self.popup = Some(Popup::Definition {
            span: word,
            card: DefinitionCard { path, line, lines },
        });
    }

    /// Files read for the card.
    pub(crate) fn link_files_read(&mut self, files: &[(PathBuf, Option<String>)]) {
        let Some(Popup::Definition { card, .. }) = &mut self.popup else {
            return;
        };
        if card.lines.is_none()
            && let Some((_, text)) = files.iter().find(|(p, _)| *p == card.path)
        {
            card.lines = Some(card_lines(text.as_deref().unwrap_or_default(), card.line));
        }
    }
}

/// The lines a card shows of `text` for a definition on `line`: its doc
/// comment and attributes above, then the definition's first lines.
fn card_lines(text: &str, line: usize) -> (usize, Vec<String>) {
    let all: Vec<&str> = text.lines().collect();
    let line = line.min(all.len().saturating_sub(1));
    let mut first = line;
    while first > 0 && line - first < DOC_LINES {
        let above = all[first - 1].trim_start();
        let doc = [
            "///", "//!", "#[", "#!", "/**", "* ", "*/", "@", "\"\"\"", "--", "#",
        ]
        .iter()
        .any(|s| above.starts_with(s));
        if !doc || above.is_empty() {
            break;
        }
        first -= 1;
    }
    let end = (line + CARD_LINES).min(all.len());
    let lines = all[first..end].iter().map(|l| l.to_string()).collect();
    (first, lines)
}

#[cfg(test)]
mod tests {
    use super::card_lines;

    #[test]
    fn a_card_takes_the_doc_comment_and_the_first_lines() {
        let text = "use x;\n\n/// Adds.\n/// Two.\npub fn add() {\n    1\n}\n";
        let (first, lines) = card_lines(text, 4);
        assert_eq!(first, 2);
        assert_eq!(lines[0], "/// Adds.");
        assert_eq!(lines[2], "pub fn add() {");
        assert_eq!(lines.len(), 5);
    }
}
