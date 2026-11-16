//! Go language plugin: tree-sitter syntax highlighting and `gopls`.

use merlin_grammar::Grammar;
use merlin_lsp::{Spec, WithLsp};

const GO_SERVER: Spec = Spec {
    program: "gopls",
    args: &[],
    files: &[("go", "go")],
    settings: None,
    options: &[],
    snippets: false,
    // The manifest's [programs] says how to install it, per system.
    install: None,
    flags: &[],
    restarts: &[],
    shared: &[],
    adjust: None,
};

merlin_plugin::export!(WithLsp<Grammar> => WithLsp::new(
    Grammar::new(
        tree_sitter_go::LANGUAGE.into(),
        tree_sitter_go::HIGHLIGHTS_QUERY,
        &["go"],
    ),
    "Go",
    &[GO_SERVER],
));
