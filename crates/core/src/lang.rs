//! What language tooling tells the editor: diagnostics and completions.
//! Positions count UTF-16 code units in their line, as LSP does.

/// Most severe first, so the smallest wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Line and UTF-16 column.
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub severity: Severity,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    /// Shown dimmed after the label, such as a type.
    pub detail: String,
    /// What replaces the word being typed.
    pub insert: String,
}

/// A diagnostic placed in a document, by char offset. It moves with edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
}

/// A language a plugin adds, as a VS Code extension contributes one: the
/// files it's for, its name, and how its comments start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Language {
    pub id: String,
    /// As the status bar shows it, such as `C++`.
    pub name: String,
    /// File extensions, without the dot.
    pub extensions: Vec<String>,
    pub line_comment: Option<String>,
    pub block_comment: Option<(String, String)>,
}

/// Colours, symbols or folds from a plugin, before they're placed: lines
/// and UTF-16 columns, as LSP counts them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginToken {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub kind: merlin_syntax::Syntax,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSymbol {
    pub name: String,
    pub kind: String,
    pub detail: String,
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub depth: u32,
}
