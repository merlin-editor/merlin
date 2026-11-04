//! Python, as a language plugin: the language, and colours from its
//! tree-sitter grammar.

use merlin_grammar::Grammar;
use merlin_plugin::prelude::*;

struct Language(Grammar);

impl Default for Language {
    fn default() -> Self {
        Self(Grammar::new(
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            &["py", "pyi"],
        ))
    }
}

impl Plugin for Language {
    fn buffer(&mut self, event: BufferEvent) {
        self.0.buffer(event);
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        self.0.colour_text(text)
    }
}

merlin_plugin::export!(Language);
