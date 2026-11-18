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
//! coloured again: the changes are applied to the text kept, the tree is
//! reparsed from the old one, and tree-sitter says which parts differ.
//!
//! A file larger than [`MAX_BYTES`] is coloured around the view: a window
//! of its lines is read, parsed and kept, edits in it are followed, and it
//! moves when the view nears its edge. A tree costs 20 to 80 times its
//! text, so the whole file's would be far too much.

use std::ops::Range;

use merlin_plugin::prelude::*;
use tree_sitter::{
    InputEdit, Language, Parser, Point, Query, QueryCursor, StreamingIterator, Tree,
};

/// Files up to this are parsed whole. Colouring a whole file takes 4 to 7
/// million instructions per KB, so 512 KB stays well within the six
/// billion the editor lets one call use, and within about half a second.
const MAX_BYTES: usize = 512 * 1024;

/// Lines read above and below the view in a larger file.
const REACH: u32 = 1_000;
/// A window moves once the view comes this near its edge.
const MARGIN: u32 = 250;
/// How far into a window to look for a line that starts at its left edge,
/// where a parse can start without being inside something.
const SETTLE: usize = 300;

pub struct Grammar {
    parser: Parser,
    query: Option<Query>,
    extensions: &'static [&'static str],
    last: Option<Coloured>,
}

/// What was last coloured.
struct Coloured {
    path: String,
    version: Option<u64>,
    /// The text parsed: the file, or the window's lines.
    text: String,
    tree: Tree,
    /// For a window, its first line in the file and how many it has.
    window: Option<(u32, u32)>,
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

/// The byte of `text` at a line, from `base`, and UTF-16 column; `None`
/// past its end.
fn byte_at(text: &str, base: u32, at: Position) -> Option<usize> {
    let row = at.line.checked_sub(base)? as usize;
    let start = match row {
        0 => 0,
        _ => text.match_indices('\n').nth(row - 1)?.0 + 1,
    };
    let line = &text[start..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    let mut units = 0;
    for (i, c) in line.char_indices() {
        if units >= at.column as usize {
            return Some(start + i);
        }
        units += c.len_utf16();
    }
    Some(start + line.len())
}

/// Applies `change`, made to the file, to `text`, its lines from `base`,
/// and gives tree-sitter's edit for it; `None` if it isn't all in `text`.
fn apply(text: &mut String, base: u32, change: &TextChange) -> Option<InputEdit> {
    let start = byte_at(text, base, change.start)?;
    let end = byte_at(text, base, change.end)?.max(start);
    let (start_position, old_end_position) = (point(text, start), point(text, end));
    text.replace_range(start..end, &change.text);
    let new_end = start + change.text.len();
    Some(InputEdit {
        start_byte: start,
        old_end_byte: end,
        new_end_byte: new_end,
        start_position,
        old_end_position,
        new_end_position: point(text, new_end),
    })
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

    /// Colours the open file when it switches or changes, or, for a large
    /// one, when the view moves away from what was coloured.
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
        let Ok(size) = host::active_size() else {
            return;
        };
        let version = host::active_version().ok();
        if size.bytes as usize <= MAX_BYTES {
            if !matches!(event, BufferEvent::Viewed) {
                self.colour_whole(path, version);
            }
        } else {
            self.colour_window(path, version, size.lines);
        }
    }

    /// The last colouring, if it was of `path` and in the same way.
    fn last_of(&mut self, path: &str, window: bool) -> Option<Coloured> {
        self.last
            .take()
            .filter(|c| c.path == path && c.window.is_some() == window)
    }

    /// Follows the changes made since `last` was coloured: the edits for
    /// tree-sitter, and the bytes they cover now. A change above a window
    /// moves it. `None` when the changes aren't kept any more, or don't all
    /// fall in the text kept.
    fn follow(last: &mut Coloured) -> Option<(Vec<InputEdit>, Range<usize>)> {
        let since = last.version?;
        let changes = host::active_changes(since).ok().flatten()?;
        let mut base = last.window.map_or(0, |(first, _)| first);
        let mut edits = Vec::new();
        let mut covered: Option<Range<usize>> = None;
        for change in &changes {
            if last.window.is_some() && change.end.line < base {
                let gone = change.end.line - change.start.line;
                let added = change.text.matches('\n').count() as u32;
                base = base + added - gone;
                continue;
            }
            let edit = apply(&mut last.text, base, change)?;
            let shift = |b: usize| match b >= edit.old_end_byte {
                true => b + edit.new_end_byte - edit.old_end_byte,
                false => b.min(edit.new_end_byte),
            };
            covered = Some(match covered {
                Some(c) => shift(c.start).min(edit.start_byte)..shift(c.end).max(edit.new_end_byte),
                None => edit.start_byte..edit.new_end_byte,
            });
            edits.push(edit);
        }
        if let Some((first, lines)) = &mut last.window {
            *first = base;
            *lines = last.text.matches('\n').count() as u32;
        }
        Some((edits, covered.unwrap_or(0..0)))
    }

    /// Parses again after `edits`, from the old tree, and sends the
    /// colours of the lines that changed, or whose meaning did.
    fn reparse(&mut self, mut last: Coloured, edits: Vec<InputEdit>, covered: Range<usize>) {
        if edits.is_empty() {
            self.last = Some(last);
            return;
        }
        for edit in &edits {
            last.tree.edit(edit);
        }
        let Some(tree) = self.parser.parse(&last.text, Some(&last.tree)) else {
            return;
        };
        let text = &last.text;
        let mut range = covered;
        for r in last.tree.changed_ranges(&tree) {
            range.start = range.start.min(r.start_byte);
            range.end = range.end.max(r.end_byte);
        }
        // Whole lines.
        let start = text[..range.start.min(text.len())]
            .rfind('\n')
            .map_or(0, |i| i + 1);
        let end = text[range.end.min(text.len())..]
            .find('\n')
            .map_or(text.len(), |i| range.end + i + 1);
        let base = last.window.map_or(0, |(first, _)| first);
        let tokens = self.tokens(text, &tree, Some(start..end), base);
        let first = base + text[..start].matches('\n').count() as u32;
        let lines = LineRange {
            start: first,
            end: first + text[start..end].matches('\n').count() as u32 + 1,
        };
        host::set_tokens(&last.path, TokenLayer::Base, Some(lines), &tokens);
        last.tree = tree;
        self.last = Some(last);
    }

    /// A file small enough to parse whole: the changes since it was last
    /// coloured are applied to the text kept, or it's read again.
    fn colour_whole(&mut self, path: String, version: Option<u64>) {
        if let Some(mut last) = self.last_of(&path, false) {
            if version.is_some() && last.version == version {
                self.last = Some(last);
                return;
            }
            if let Some((edits, covered)) = Self::follow(&mut last) {
                last.version = version;
                return self.reparse(last, edits, covered);
            }
        }
        let Ok(text) = host::active_text() else {
            return;
        };
        let Some(tree) = self.parser.parse(&text, None) else {
            return;
        };
        let tokens = self.tokens(&text, &tree, None, 0);
        host::set_tokens(&path, TokenLayer::Base, None, &tokens);
        self.last = Some(Coloured {
            path,
            version,
            text,
            tree,
            window: None,
        });
    }

    /// A large file, coloured around the view: edits in the window are
    /// followed while the view stays well inside it; otherwise the lines
    /// around the view now are read and parsed.
    fn colour_window(&mut self, path: String, version: Option<u64>, count: u32) {
        let Ok(view) = host::active_view() else {
            return;
        };
        if let Some(mut last) = self.last_of(&path, true) {
            let followed = match version.is_some() && last.version == version {
                true => Some((Vec::new(), 0..0)),
                false => Self::follow(&mut last),
            };
            let (first, lines) = last.window.expect("a window");
            // Well inside, or the window reaches the file's edge there.
            let above = first == 0 || view.start >= first + MARGIN;
            let below = first + lines >= count || view.end + MARGIN <= first + lines;
            if let Some((edits, covered)) = followed
                && above
                && below
            {
                last.version = version;
                return self.reparse(last, edits, covered);
            }
        }
        let start = view.start.saturating_sub(REACH);
        let end = (view.end + REACH).min(count);
        let Ok(mut text) = host::active_lines(LineRange { start, end }) else {
            return;
        };
        // From a line at the left edge, so the parse doesn't start inside
        // a comment or a block: the first in the window's first lines.
        let mut first = start;
        if start > 0 {
            let mut at = 0;
            for (i, line) in text.split_inclusive('\n').take(SETTLE).enumerate() {
                if line.starts_with(|c: char| !c.is_whitespace()) {
                    first += i as u32;
                    break;
                }
                at += line.len();
            }
            if first > start {
                text.drain(..at);
            }
        }
        let Some(tree) = self.parser.parse(&text, None) else {
            return;
        };
        let tokens = self.tokens(&text, &tree, None, first);
        let lines = text.matches('\n').count() as u32;
        let span = LineRange {
            start: first,
            end: end.max(first + lines),
        };
        host::set_tokens(&path, TokenLayer::Base, Some(span), &tokens);
        self.last = Some(Coloured {
            path,
            version,
            text,
            tree,
            window: Some((first, lines)),
        });
    }

    /// Colours a piece of text, such as a diff or a code block, apart from
    /// the open file.
    pub fn colour_text(&mut self, text: &str) -> Vec<Token> {
        if text.len() > MAX_BYTES {
            return Vec::new();
        }
        match self.parser.parse(text, None) {
            Some(tree) => self.tokens(text, &tree, None, 0),
            None => Vec::new(),
        }
    }

    /// The query's tokens over `bytes`, or the whole text, with lines
    /// counted from `base`.
    fn tokens(
        &self,
        text: &str,
        tree: &Tree,
        bytes: Option<Range<usize>>,
        base: u32,
    ) -> Vec<Token> {
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
        let mut min_byte = text.len();
        let mut max_byte = 0;
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let range = capture.node.byte_range();
            // The first pattern to match a node wins; later ones are
            // fallbacks.
            if found.last().is_some_and(|(r, _)| *r == range) {
                continue;
            }
            if let Some(kind) = classify(names[capture.index as usize]) {
                min_byte = min_byte.min(range.start);
                max_byte = max_byte.max(range.end);
                found.push((range, kind));
            }
        }
        if found.is_empty() {
            return Vec::new();
        }
        let mut lines = Lines::new_in(text, min_byte..max_byte);
        lines.base_line += base as usize;
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

impl merlin_plugin::Plugin for Grammar {
    fn buffer(&mut self, event: BufferEvent) {
        Grammar::buffer(self, event);
    }

    fn colour_text(&mut self, _language: &str, text: &str) -> Vec<Token> {
        Grammar::colour_text(self, text)
    }
}

/// Turns byte offsets into lines and UTF-16 columns, as positions count
/// them, indexing only the slice of `text` that holds the tokens.
struct Lines<'a> {
    text: &'a str,
    base_line: usize,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new_in(text: &'a str, span: Range<usize>) -> Self {
        let first_start = text[..span.start].rfind('\n').map_or(0, |i| i + 1);
        let base_line = text[..first_start].matches('\n').count();
        let end = span.end.min(text.len());
        let starts = std::iter::once(first_start)
            .chain(
                text[first_start..end]
                    .match_indices('\n')
                    .map(|(i, _)| first_start + i + 1),
            )
            .collect();
        Self {
            text,
            base_line,
            starts,
        }
    }

    fn position(&mut self, byte: usize) -> Position {
        let rel = self.starts.partition_point(|&s| s <= byte) - 1;
        let start = self.starts[rel];
        let slice = &self.text[start..byte];
        let column = if slice.is_ascii() {
            slice.len() as u32
        } else {
            slice.chars().map(|c| c.len_utf16() as u32).sum()
        };
        Position {
            line: (self.base_line + rel) as u32,
            column,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_apply_to_the_text_from_its_first_line() {
        let change = |start: (u32, u32), end: (u32, u32), text: &str| TextChange {
            start: Position {
                line: start.0,
                column: start.1,
            },
            end: Position {
                line: end.0,
                column: end.1,
            },
            text: text.into(),
        };
        let mut text = "let a = 1;\nlet é = 2;\n".to_string();
        let e = apply(&mut text, 0, &change((0, 5), (0, 5), "b")).unwrap();
        assert_eq!(text, "let ab = 1;\nlet é = 2;\n");
        assert_eq!((e.start_byte, e.old_end_byte, e.new_end_byte), (5, 5, 6));
        // Lines counted from 10: line 11 is the text's second.
        let e = apply(&mut text, 10, &change((11, 5), (11, 6), "\nx")).unwrap();
        assert_eq!(text, "let ab = 1;\nlet é\nx= 2;\n");
        assert_eq!(e.new_end_position, Point { row: 2, column: 1 });
        assert!(apply(&mut text, 10, &change((9, 0), (9, 1), "")).is_none());
    }

    #[test]
    fn lines_index_only_the_requested_span() {
        let text = "zero\none\ntwo é\nthree\n";
        let span_start = text.find("two").unwrap();
        let span_end = span_start + "two é".len();
        let mut lines = Lines::new_in(text, span_start..span_end);
        assert_eq!(lines.starts.len(), 1);
        let p0 = lines.position(span_start);
        let p1 = lines.position(span_end);
        assert_eq!((p0.line, p0.column), (2, 0));
        assert_eq!((p1.line, p1.column), (2, 5));
    }
}
