//! Colour a language in a merlin plugin with a tree-sitter grammar, as a
//! VS Code extension does with a TextMate grammar.
//!
//! ```ignore
//! use merlin_grammar::Grammar;
//! use merlin_plugin::prelude::*;
//!
//! struct Rust(Grammar);
//!
//! impl Default for Rust {
//!     fn default() -> Self {
//!         Self(Grammar::new(
//!             tree_sitter_rust::LANGUAGE.into(),
//!             tree_sitter_rust::HIGHLIGHTS_QUERY,
//!             &["rs"],
//!         ))
//!     }
//! }
//!
//! impl Plugin for Rust {
//!     fn buffer(&mut self, event: BufferEvent) {
//!         self.0.buffer(event);
//!     }
//! }
//! ```
//!
//! The grammar's C code builds against the headers in `wasm-include`: see
//! a language plugin's `.cargo/config.toml`.
//!
//! On each change only the lines that changed, or whose meaning did, are
//! coloured again: the tree is reparsed from the old one, and tree-sitter
//! says which parts differ.

use std::ops::Range;

use merlin_plugin::prelude::*;
use tree_sitter::{
    InputEdit, Language, Parser, Point, Query, QueryCursor, StreamingIterator, Tree,
};

/// Files bigger than this aren't coloured, so one call stays well within
/// what the editor lets a plugin compute.
const MAX_BYTES: usize = 2_000_000;

pub struct Grammar {
    parser: Parser,
    query: Option<Query>,
    extensions: &'static [&'static str],
    /// The file last coloured, its text and its tree.
    last: Option<(String, String, Tree)>,
}

/// The colour of a highlight query's capture, such as `function.method`.
pub fn classify(capture: &str) -> Option<TokenKind> {
    let first = capture.split('.').next().unwrap_or_default();
    Some(match (first, capture) {
        (_, "string.special.key") => TokenKind::Property,
        (_, "variable.builtin") => TokenKind::Keyword,
        (_, "variable.parameter") => TokenKind::Variable,
        ("keyword", _) => TokenKind::Keyword,
        ("string", _) => TokenKind::String,
        ("comment", _) => TokenKind::Comment,
        ("function", _) => TokenKind::Function,
        ("type" | "constructor" | "module" | "namespace", _) => TokenKind::Type,
        ("constant" | "number" | "boolean" | "float", _) => TokenKind::Constant,
        ("attribute", _) => TokenKind::Attribute,
        ("property" | "label" | "field", _) => TokenKind::Property,
        ("operator", _) => TokenKind::Operator,
        ("punctuation", _) => TokenKind::Punctuation,
        ("escape", _) => TokenKind::Escape,
        ("embedded" | "macro", _) => TokenKind::Macro,
        _ => return None,
    })
}

/// Where a byte offset is, as a tree-sitter point: row and byte column.
fn point(text: &str, byte: usize) -> Point {
    let before = &text[..byte];
    let row = before.matches('\n').count();
    let column = byte - before.rfind('\n').map_or(0, |i| i + 1);
    Point { row, column }
}

/// The edit that turns `old` into `new`: what they share at the start and
/// the end stays.
fn edit_between(old: &str, new: &str) -> InputEdit {
    let mut start = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }
    let most = old.len().min(new.len()) - start;
    let mut end = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(most)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - end) || !new.is_char_boundary(new.len() - end) {
        end -= 1;
    }
    let (old_end, new_end) = (old.len() - end, new.len() - end);
    InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point(old, start),
        old_end_position: point(old, old_end),
        new_end_position: point(new, new_end),
    }
}

impl Grammar {
    /// A grammar for files with these extensions, coloured by a highlight
    /// query, as tree-sitter grammars ship them.
    pub fn new(language: Language, highlights: &str, extensions: &'static [&'static str]) -> Self {
        let mut parser = Parser::new();
        let ok = parser.set_language(&language).is_ok();
        let query = Query::new(&language, highlights).ok().filter(|_| ok);
        if query.is_none() {
            host::notify("This grammar's highlight query doesn't load");
        }
        Self {
            parser,
            query,
            extensions,
            last: None,
        }
    }

    /// Whether `path` is one of this grammar's files.
    pub fn handles(&self, path: &str) -> bool {
        path.rsplit_once('.')
            .is_some_and(|(_, ext)| self.extensions.contains(&ext.to_ascii_lowercase().as_str()))
    }

    /// Colours the open file when it switches or changes.
    pub fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Saved(_)) {
            return;
        }
        let Some(path) = host::active_path().ok().flatten() else {
            return;
        };
        if !self.handles(&path) {
            return;
        }
        let Ok(text) = host::active_text() else {
            return;
        };
        if text.len() > MAX_BYTES {
            self.last = None;
            return;
        }
        self.colour(path, text);
    }

    fn colour(&mut self, path: String, text: String) {
        // The same file again: reparse from the old tree, and colour only
        // what changed.
        let mut changed: Option<Range<usize>> = None;
        let mut old_tree = None;
        if let Some((last_path, last_text, mut tree)) = self.last.take()
            && last_path == path
        {
            if last_text == text {
                self.last = Some((last_path, last_text, tree));
                return;
            }
            let edit = edit_between(&last_text, &text);
            tree.edit(&edit);
            changed = Some(edit.start_byte..edit.new_end_byte);
            old_tree = Some(tree);
        }
        let Some(tree) = self.parser.parse(&text, old_tree.as_ref()) else {
            return;
        };
        let bytes = match (changed, &old_tree) {
            (Some(edit), Some(old)) => {
                let mut range = edit;
                for r in old.changed_ranges(&tree) {
                    range.start = range.start.min(r.start_byte);
                    range.end = range.end.max(r.end_byte);
                }
                // Whole lines.
                let start = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
                let end = text[range.end.min(text.len())..]
                    .find('\n')
                    .map_or(text.len(), |i| range.end + i + 1);
                Some(start..end)
            }
            _ => None,
        };
        let tokens = self.tokens(&text, &tree, bytes.clone());
        let lines = bytes.map(|b| LineRange {
            start: text[..b.start].matches('\n').count() as u32,
            end: (text[..b.end].matches('\n').count() + 1) as u32,
        });
        host::set_tokens(&path, TokenLayer::Base, lines, &tokens);
        self.last = Some((path, text, tree));
    }

    /// Colours a piece of text, such as a diff or a code block, apart from
    /// the open file.
    pub fn colour_text(&mut self, text: &str) -> Vec<Token> {
        if text.len() > MAX_BYTES {
            return Vec::new();
        }
        match self.parser.parse(text, None) {
            Some(tree) => self.tokens(text, &tree, None),
            None => Vec::new(),
        }
    }

    /// The query's tokens over `bytes`, or the whole file.
    fn tokens(&self, text: &str, tree: &Tree, bytes: Option<Range<usize>>) -> Vec<Token> {
        let Some(query) = &self.query else {
            return Vec::new();
        };
        let mut cursor = QueryCursor::new();
        if let Some(b) = bytes {
            cursor.set_byte_range(b);
        }
        let names = query.capture_names();
        let mut captures = cursor.captures(query, tree.root_node(), text.as_bytes());
        let mut found: Vec<(Range<usize>, TokenKind)> = Vec::new();
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let range = capture.node.byte_range();
            // The first pattern to match a node wins; later ones are
            // fallbacks.
            if found.last().is_some_and(|(r, _)| *r == range) {
                continue;
            }
            if let Some(kind) = classify(names[capture.index as usize]) {
                found.push((range, kind));
            }
        }
        let mut lines = Lines::new(text);
        found
            .into_iter()
            .map(|(r, kind)| Token {
                start: lines.position(r.start),
                end: lines.position(r.end),
                kind,
            })
            .collect()
    }
}

/// Turns byte offsets into lines and UTF-16 columns, as positions count
/// them.
struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        Self { text, starts }
    }

    fn position(&mut self, byte: usize) -> Position {
        let line = self.starts.partition_point(|&s| s <= byte) - 1;
        let start = self.starts[line];
        let column = self.text[start..byte]
            .chars()
            .map(|c| c.len_utf16() as u32)
            .sum();
        Position {
            line: line as u32,
            column,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_cover_what_changed() {
        let e = edit_between("let a = 1;\n", "let ab = 1;\n");
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (5, 5, 6));
        let e = edit_between("aaa", "aaaa");
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (3, 3, 4));
        let e = edit_between("x\ny\n", "x\n");
        assert_eq!(e.old_end_position, Point { row: 2, column: 0 });
        let e = edit_between("é", "è");
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (0, 2, 2));
    }
}
