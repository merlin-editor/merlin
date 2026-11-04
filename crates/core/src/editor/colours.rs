//! Colours for text that isn't an open file: a diff in a review tab, or a
//! code block in Markdown. The editor has no grammars of its own, so it
//! asks the plugin that added the language, through the host
//! ([`Editor::colour_requests`]), and keeps the answers.

use std::cell::RefCell;
use std::collections::HashMap;

use merlin_syntax::Syntax;

use super::Editor;
use crate::lang::PluginToken;
use crate::layers::Token;

/// Coloured runs by character offset in the text.
pub(crate) type Spans = Vec<(usize, usize, Syntax)>;

/// Answers kept, at most, before starting again.
const KEPT: usize = 256;

#[derive(Default)]
pub(crate) struct TextColours {
    /// By language and text. An empty answer is kept too, so it isn't
    /// asked again.
    done: RefCell<HashMap<(String, String), Spans>>,
    wanted: RefCell<Vec<(String, String)>>,
}

impl TextColours {
    /// The colours of `text` as `language`, or `None` while they're asked
    /// for.
    pub fn get(&self, language: &str, text: &str) -> Option<Spans> {
        if language.is_empty() {
            return Some(Vec::new());
        }
        let key = (language.to_ascii_lowercase(), text.to_string());
        if let Some(spans) = self.done.borrow().get(&key) {
            return Some(spans.clone());
        }
        let mut wanted = self.wanted.borrow_mut();
        if !wanted.contains(&key) {
            wanted.push(key);
        }
        None
    }
}

/// Places tokens by line and UTF-16 column within `text`.
fn place(text: &str, tokens: &[PluginToken]) -> Spans {
    let lines: Vec<&str> = text.split('\n').collect();
    let starts: Vec<usize> = lines
        .iter()
        .scan(0, |at, line| {
            let start = *at;
            *at += line.chars().count() + 1;
            Some(start)
        })
        .collect();
    let at = |(line, column): (u32, u32)| {
        let Some(line_text) = lines.get(line as usize) else {
            return text.chars().count();
        };
        let mut units = 0;
        let chars = line_text
            .chars()
            .take_while(|c| {
                units += c.len_utf16() as u32;
                units <= column
            })
            .count();
        starts[line as usize] + chars
    };
    tokens
        .iter()
        .map(|t| (at(t.start), at(t.end), t.kind))
        .filter(|(a, b, _)| a < b)
        .collect()
}

impl Editor {
    /// Text to colour, by language (an id or an extension) and the text:
    /// the host asks the plugin that added the language and sends back
    /// [`Event::TextColours`](super::Event::TextColours).
    pub fn colour_requests(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.colours.wanted.borrow_mut())
    }

    /// Keeps the colours a plugin gave for some text, and colours the
    /// reviews showing it.
    pub(crate) fn text_coloured(
        &mut self,
        language: String,
        text: String,
        tokens: Vec<PluginToken>,
    ) {
        let spans = place(&text, &tokens);
        for doc in &mut self.docs {
            let ext = doc
                .review
                .as_ref()
                .and_then(|r| r.path.extension())
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            if ext.as_deref() == Some(language.as_str()) && doc.buffer.text() == text {
                let tokens = spans
                    .iter()
                    .map(|&(start, end, kind)| Token { start, end, kind })
                    .collect();
                doc.layers.base = Some(tokens);
            }
        }
        let mut done = self.colours.done.borrow_mut();
        if done.len() >= KEPT {
            done.clear();
        }
        done.insert((language.to_ascii_lowercase(), text), spans);
    }

    /// Asks for a review's colours, as its file's language.
    pub(crate) fn colour_review(&mut self, i: usize) {
        let doc = &self.docs[i];
        let Some(ext) = doc
            .review
            .as_ref()
            .and_then(|r| r.path.extension())
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
        else {
            return;
        };
        if let Some(spans) = self.colours.get(&ext, &doc.buffer.text()) {
            let tokens = spans
                .into_iter()
                .map(|(start, end, kind)| Token { start, end, kind })
                .collect();
            self.docs[i].layers.base = Some(tokens);
        }
    }
}
