//! What a piece of code is, for its colour: the editor draws what language
//! plugins say. And edits in the terms grammars count them, for plugins and
//! for keeping views in place.

/// What a piece of code is. The frontend picks the colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Syntax {
    #[default]
    Plain,
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Constant,
    Attribute,
    Property,
    Operator,
    Punctuation,
    Escape,
    /// Variables and parameters, which language servers can tell apart.
    Variable,
    /// Preprocessor directives and macros.
    Macro,
}

/// One change to the text, in the terms tree-sitter needs. Rows count
/// lines; columns count bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: (usize, usize),
    pub old_end: (usize, usize),
    pub new_end: (usize, usize),
}
