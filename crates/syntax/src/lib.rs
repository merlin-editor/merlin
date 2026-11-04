//! Syntax highlighting with tree-sitter.
//!
//! A [`Highlighter`] keeps a file's syntax tree. Edits are fed to it as they
//! happen, so reparsing only redoes the changed parts, and highlighting only
//! queries the lines on screen.

use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use ropey::Rope;
use tree_sitter::{
    InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree,
};

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

/// Maps a highlight query's capture name, such as `function.method`.
fn classify(capture: &str) -> Syntax {
    let first = capture.split('.').next().unwrap_or_default();
    match (first, capture) {
        (_, "string.special.key") => Syntax::Property,
        (_, "variable.builtin") => Syntax::Keyword,
        ("keyword", _) => Syntax::Keyword,
        ("string", _) => Syntax::String,
        ("comment", _) => Syntax::Comment,
        ("function", _) => Syntax::Function,
        ("type" | "constructor", _) => Syntax::Type,
        ("constant" | "number" | "boolean", _) => Syntax::Constant,
        ("attribute", _) => Syntax::Attribute,
        ("property" | "label", _) => Syntax::Property,
        ("operator", _) => Syntax::Operator,
        ("punctuation", _) => Syntax::Punctuation,
        ("escape", _) => Syntax::Escape,
        _ => Syntax::Plain,
    }
}

struct Lang {
    name: &'static str,
    extensions: &'static [&'static str],
    language: Language,
    query: Query,
}

fn languages() -> &'static [Lang] {
    static LANGS: OnceLock<Vec<Lang>> = OnceLock::new();
    LANGS.get_or_init(|| {
        let defs: [(&str, &[&str], Language, &str); 5] = [
            (
                "Rust",
                &["rs"],
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY,
            ),
            (
                "Python",
                &["py", "pyi"],
                tree_sitter_python::LANGUAGE.into(),
                tree_sitter_python::HIGHLIGHTS_QUERY,
            ),
            (
                "JavaScript",
                &["js", "mjs", "cjs", "jsx"],
                tree_sitter_javascript::LANGUAGE.into(),
                tree_sitter_javascript::HIGHLIGHT_QUERY,
            ),
            (
                "JSON",
                &["json"],
                tree_sitter_json::LANGUAGE.into(),
                tree_sitter_json::HIGHLIGHTS_QUERY,
            ),
            (
                "TOML",
                &["toml"],
                tree_sitter_toml_ng::LANGUAGE.into(),
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            ),
        ];
        defs.into_iter()
            .filter_map(|(name, extensions, language, source)| {
                // A grammar whose query doesn't compile is left out rather
                // than taking the editor down.
                let query = Query::new(&language, source).ok()?;
                Some(Lang {
                    name,
                    extensions,
                    language,
                    query,
                })
            })
            .collect()
    })
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

pub struct Highlighter {
    lang: &'static Lang,
    parser: Parser,
    tree: Option<Tree>,
}

impl Highlighter {
    /// A highlighter for the file's language, if merlin knows it.
    pub fn for_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?;
        let lang = languages().iter().find(|l| l.extensions.contains(&ext))?;
        let mut parser = Parser::new();
        parser.set_language(&lang.language).ok()?;
        Some(Self {
            lang,
            parser,
            tree: None,
        })
    }

    pub fn language(&self) -> &'static str {
        self.lang.name
    }

    /// Records an edit, so the next [`Highlighter::parse`] reuses the rest
    /// of the tree.
    pub fn edit(&mut self, edit: &TextEdit) {
        let point = |(row, column)| Point { row, column };
        if let Some(tree) = &mut self.tree {
            tree.edit(&InputEdit {
                start_byte: edit.start_byte,
                old_end_byte: edit.old_end_byte,
                new_end_byte: edit.new_end_byte,
                start_position: point(edit.start),
                old_end_position: point(edit.old_end),
                new_end_position: point(edit.new_end),
            });
        }
    }

    pub fn parse(&mut self, rope: &Rope) {
        let len = rope.len_bytes();
        let mut read = |byte: usize, _: Point| -> &[u8] {
            if byte >= len {
                return &[];
            }
            let (chunk, start, _, _) = rope.chunk_at_byte(byte);
            &chunk.as_bytes()[byte - start..]
        };
        self.tree = self
            .parser
            .parse_with_options(&mut read, self.tree.as_ref(), None);
    }

    /// Highlighted byte ranges on `lines`, outer ones first, so a later
    /// range wins where they overlap.
    pub fn highlights(&self, rope: &Rope, lines: Range<usize>) -> Vec<(Range<usize>, Syntax)> {
        let Some(tree) = &self.tree else {
            return Vec::new();
        };
        let last = rope.len_lines();
        let bytes =
            rope.line_to_byte(lines.start.min(last))..rope.line_to_byte(lines.end.min(last));
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(bytes);
        let query = &self.lang.query;
        let names = query.capture_names();
        let text = |node: Node| {
            rope.byte_slice(node.byte_range())
                .chunks()
                .map(str::as_bytes)
        };
        let mut captures = cursor.captures(query, tree.root_node(), text);
        let mut found: Vec<(Range<usize>, Syntax)> = Vec::new();
        while let Some((m, index)) = captures.next() {
            let capture = m.captures()[*index];
            let range = capture.node.byte_range();
            // The first pattern to match a node wins; later ones are fallbacks.
            if found.last().is_some_and(|(r, _)| *r == range) {
                continue;
            }
            found.push((range, classify(names[capture.index as usize])));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(path: &str, text: &str) -> Vec<(String, Syntax)> {
        let rope = Rope::from_str(text);
        let mut h = Highlighter::for_path(Path::new(path)).unwrap();
        h.parse(&rope);
        h.highlights(&rope, 0..rope.len_lines())
            .into_iter()
            .filter(|(_, s)| *s != Syntax::Punctuation)
            .map(|(r, s)| (text[r].to_string(), s))
            .collect()
    }

    #[test]
    fn highlights_rust() {
        let found = spans("a.rs", "fn main() { let s = \"hi\"; } // done");
        assert!(found.contains(&("fn".into(), Syntax::Keyword)), "{found:?}");
        assert!(
            found.contains(&("main".into(), Syntax::Function)),
            "{found:?}"
        );
        assert!(
            found.contains(&("\"hi\"".into(), Syntax::String)),
            "{found:?}"
        );
        assert!(
            found.contains(&("// done".into(), Syntax::Comment)),
            "{found:?}"
        );
    }

    #[test]
    fn every_language_compiles_its_query() {
        assert_eq!(languages().len(), 5);
        for (path, text) in [
            ("a.py", "def f(): pass"),
            ("a.js", "let x = 1"),
            ("a.json", "{\"a\": 1}"),
            ("a.toml", "a = 1"),
        ] {
            assert!(!spans(path, text).is_empty(), "{path}");
        }
    }

    #[test]
    fn reparses_after_an_edit() {
        let mut rope = Rope::from_str("fn a() {}\n");
        let mut h = Highlighter::for_path(Path::new("a.rs")).unwrap();
        h.parse(&rope);
        // Insert "// " at the start, turning the line into a comment.
        rope.insert(0, "// ");
        h.edit(&TextEdit {
            start_byte: 0,
            old_end_byte: 0,
            new_end_byte: 3,
            start: (0, 0),
            old_end: (0, 0),
            new_end: (0, 3),
        });
        h.parse(&rope);
        let found = h.highlights(&rope, 0..1);
        assert_eq!(found, [(0..12, Syntax::Comment)]);
    }

    #[test]
    fn unknown_files_have_no_highlighter() {
        assert!(Highlighter::for_path(Path::new("notes.txt")).is_none());
    }
}
