//! What language tooling tells the editor: diagnostics and completions.
//! Positions count UTF-16 code units in their line, as LSP does.

/// Most severe first, so the smallest wins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    #[default]
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    /// Line and UTF-16 column.
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub severity: Severity,
    pub message: String,
    /// What reported it, such as `rustc` or `clippy`, its code, such as
    /// `E0308`, and a web page that explains the code. Any may be empty.
    pub source: String,
    pub code: String,
    pub link: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    /// Shown dimmed after the label, such as a type.
    pub detail: String,
    /// What replaces the word being typed.
    pub insert: String,
}

/// What a code action does, for the group it's listed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionKind {
    QuickFix,
    Refactor,
    Source,
    Other,
    /// The editor's own rows, formatting the file or the selection.
    Format,
}

/// Something to do to the code at the cursor, from a plugin: chosen by
/// its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeAction {
    pub id: String,
    pub title: String,
    pub kind: ActionKind,
    pub preferred: bool,
}

/// A range by line and UTF-16 column, and what replaces it.
pub type TextEdit = ((u32, u32), (u32, u32), String);

/// Changes to one file, each in the text before any of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEdit {
    pub path: std::path::PathBuf,
    pub edits: Vec<TextEdit>,
}

/// Where a name is used: a file, and the UTF-16 range in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: std::path::PathBuf,
    pub start: (u32, u32),
    pub end: (u32, u32),
}

/// A symbol anywhere in the project, as a language plugin found it for the
/// palette: where it starts, as UTF-16 line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSymbol {
    pub name: String,
    pub kind: String,
    /// What it's in, such as `impl Editor`; empty for nothing.
    pub container: String,
    pub path: std::path::PathBuf,
    pub line: u32,
    pub column: u32,
}

/// What renaming the name at the cursor would change: the name's place in
/// the active file, as UTF-16 line and column, and how far it reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameInfo {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub places: u32,
    pub files: u32,
}

/// A function's signature as a call is typed: its label, the parameter
/// the cursor is in as a range of the label's characters, a line of its
/// documentation, and which of how many overloads it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    pub active: Option<(usize, usize)>,
    pub doc: String,
    pub index: u32,
    pub count: u32,
}

/// What an inlay hint names, for how it's drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlayKind {
    Type,
    Parameter,
    Other,
    /// What Vim's `:s` puts in, shown after what it replaces.
    Replacement,
}

/// A hint a language server writes between the characters, such as a
/// variable's type or a parameter's name, at a line and UTF-16 column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    pub at: (u32, u32),
    pub label: String,
    pub kind: InlayKind,
}

/// An inlay hint placed in a document, before the character at `pos`. It
/// takes `cols` columns, so the text after it moves along, and moves
/// with edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inlay {
    pub pos: usize,
    pub label: String,
    pub kind: InlayKind,
    pub cols: usize,
}

/// A diagnostic placed in a document, by char offset. It moves with edits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub severity: Severity,
    pub message: String,
    pub source: String,
    pub code: String,
    pub link: String,
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
    /// What a line ends with for Enter to indent the next a level more,
    /// as brackets do: Python's `:`.
    pub indent_after: Vec<String>,
    /// What the language calls an exception, such as Rust's `panic`, for
    /// the card when a run stops on one.
    pub exception: Option<String>,
}

/// How a plugin reads problems out of a task's output, as a VS Code
/// extension's problem matcher does: lines one after another, each a
/// pattern and which of its groups say what.
#[derive(Clone, Debug)]
pub struct ProblemMatcher {
    pub lines: Vec<MatcherLine>,
    /// What may come between the lines, such as a traceback's indented
    /// ones; nothing may without it.
    pub between: Option<regex::Regex>,
    /// A page about a problem's code, with `{code}` where it goes.
    pub link: Option<String>,
}

/// A line of a [`ProblemMatcher`]: its pattern, and the group with each
/// part of the problem, 0 for the whole line.
#[derive(Clone, Debug)]
pub struct MatcherLine {
    pub regex: regex::Regex,
    pub file: Option<usize>,
    /// From 1, as tools print them.
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// `error`, `warning`, `note` or `info`; an error without it.
    pub severity: Option<usize>,
    pub code: Option<usize>,
    pub message: Option<usize>,
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

/// How something in the language card is doing, worst last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusState {
    #[default]
    Ready,
    Busy,
    Warning,
    Error,
}

/// A line a plugin puts in the language card: a language server and how
/// it's doing, say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusLine {
    pub name: String,
    pub state: StatusState,
    pub detail: String,
    /// How far along it is, from 0 to 100, while busy.
    pub progress: Option<u8>,
    /// A command line shown under it, with Copy.
    pub hint: Option<String>,
    /// A button's label, and the command it runs in the plugin.
    pub action: Option<(String, String)>,
    /// The extensions of the files it's for; empty for every file of the
    /// plugin's languages.
    pub extensions: Vec<String>,
}
