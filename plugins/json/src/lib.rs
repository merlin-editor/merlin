//! JSON, as a language plugin: the language, and colours from its
//! tree-sitter grammar.

use merlin_grammar::Grammar;

merlin_plugin::export!(Grammar => Grammar::new(
    tree_sitter_json::LANGUAGE.into(),
    tree_sitter_json::HIGHLIGHTS_QUERY,
    &["json"],
));
