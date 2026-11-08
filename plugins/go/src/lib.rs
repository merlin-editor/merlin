//! Go language plugin: tree-sitter syntax highlighting and `gopls`.

use merlin_grammar::Grammar;
use merlin_lsp::{Spec, WithLsp};

const GO_SERVER: Spec = Spec {
    program: "gopls",
    args: &[],
    files: &[("go", "go")],
    settings: None,
    options: &[],
    install: Some("go install golang.org/x/tools/gopls@latest"),
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
