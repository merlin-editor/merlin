//! Python language plugin: tree-sitter syntax highlighting, folding,
//! `debugpy` target registration, and `pyright-langserver` configured with
//! `diagnosticMode = "openFilesOnly"` for low CPU and memory usage.

use merlin_grammar::Grammar;
use merlin_lsp::{Spec, WithLsp};
use merlin_plugin::prelude::*;

/// pyright configured to analyze open files only, avoiding whole-workspace
/// and virtualenv background indexing spikes.
const PYTHON_SERVER: Spec = Spec {
    program: "pyright-langserver",
    args: &["--stdio"],
    files: &[("py", "python"), ("pyi", "python")],
    settings: Some(r#"{"python":{"analysis":{"diagnosticMode":"openFilesOnly"}}}"#),
    options: &[],
    install: Some("npm install -g pyright"),
};

struct Python(Grammar);

impl Default for Python {
    fn default() -> Self {
        Self(Grammar::new(
            tree_sitter_python::LANGUAGE.into(),
            tree_sitter_python::HIGHLIGHTS_QUERY,
            &["py", "pyi"],
        ))
    }
}

impl Plugin for Python {
    fn buffer(&mut self, event: BufferEvent) {
        // The file in front can be debugged as it is.
        if matches!(event, BufferEvent::Switched)
            && let Ok(Some(path)) = host::active_path()
            && path.ends_with(".py")
        {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            host::set_debug_targets(&[DebugTarget {
                name: format!("This file: {name}"),
                debugger: String::new(),
                language: "python".into(),
                program: path.clone(),
                args: Vec::new(),
                build: String::new(),
                detail: path,
                launch: String::new(),
            }]);
        }
        self.0.buffer(event);
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        self.0.colour_text(text)
    }
}

merlin_plugin::export!(WithLsp<Python> => WithLsp::new(
    Python::default(),
    "Python",
    &[PYTHON_SERVER],
));
