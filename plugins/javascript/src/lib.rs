//! JavaScript and TypeScript language plugin: tree-sitter syntax highlighting,
//! folding, and `typescript-language-server` configured with `maxTsServerMemory: 512`
//! for bounded memory usage.

use merlin_grammar::Grammar;
use merlin_lsp::{JS_SERVER, WithLsp};

merlin_plugin::export!(WithLsp<Grammar> => WithLsp::new(
    Grammar::new(
        tree_sitter_javascript::LANGUAGE.into(),
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        &["js", "mjs", "cjs", "jsx"],
    ),
    "JavaScript",
    &[JS_SERVER],
));
