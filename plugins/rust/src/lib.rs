//! Rust language plugin: tree-sitter syntax highlighting, folding, and
//! `rust-analyzer` language server integration (including test & runnable lenses).

use merlin_grammar::Grammar;
use merlin_lsp::{RUST_SERVER, WithLsp};

merlin_plugin::export!(WithLsp<Grammar> => WithLsp::new(
    Grammar::new(
        tree_sitter_rust::LANGUAGE.into(),
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        &["rs"],
    ),
    "Rust",
    &[RUST_SERVER],
));
