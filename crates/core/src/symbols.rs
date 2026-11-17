//! The symbols in a file, found from its text alone: functions, types and
//! the like by the keyword that declares them, and Markdown headings. They
//! nest by indentation. Breadcrumbs and the Outline view list them; a
//! language server could give better ones later.

use std::borrow::Cow;

use crate::doc::Doc;
use crate::editor::guides::indent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub line: usize,
    /// How many symbols it's inside.
    pub depth: usize,
    /// What declares it, such as `fn` or `class`, or `#` for a heading.
    pub kind: String,
    pub name: String,
    /// The last line of its block, when a plugin said where it ends.
    pub end_line: Option<usize>,
}

/// Words that declare a symbol, before its name.
const KEYWORDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "impl",
    "mod",
    "union",
    "macro_rules!",
    "def",
    "class",
    "function",
    "interface",
    "func",
    "type",
    "namespace",
];

/// Words that may come before the keyword.
const MODIFIERS: &[&str] = &[
    "pub",
    "pub(crate)",
    "pub(super)",
    "async",
    "unsafe",
    "const",
    "extern",
    "export",
    "default",
    "static",
    "abstract",
    "public",
    "private",
    "protected",
    "final",
    "declare",
];

/// The symbol a line declares, as its kind and name.
pub fn declared(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with('#') && !trimmed.starts_with("#[") {
        return None;
    }
    let mut words = trimmed.split_whitespace().peekable();
    while words
        .peek()
        .is_some_and(|w| MODIFIERS.contains(w) || w.starts_with("extern"))
    {
        words.next();
    }
    let next = *words.peek()?;
    let kind = *KEYWORDS
        .iter()
        .find(|&&k| next == k || (k == "impl" && next.starts_with("impl<")))?;
    words.next();
    if kind == "impl" {
        // `impl Trait for Type` or `impl<T> Type`, up to the brace.
        let rest: String = trimmed[trimmed.find("impl")? + 4..]
            .split(['{', ';'])
            .next()?
            .split(" where ")
            .next()?
            .trim()
            .to_string();
        // Leave out the generics it introduces, as VS Code does.
        let rest = match rest.strip_prefix('<') {
            Some(after) => {
                let mut depth = 1;
                let end = after.find(|c| {
                    match c {
                        '<' => depth += 1,
                        '>' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                });
                end.map_or(rest.clone(), |e| after[e + 1..].trim().to_string())
            }
            None => rest,
        };
        return (!rest.is_empty()).then(|| (kind.to_string(), format!("impl {rest}")));
    }
    let word = words.next()?;
    let name: String = word
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    (!name.is_empty()).then(|| (kind.to_string(), name))
}

/// A Markdown heading: its level and text.
fn heading(line: &str) -> Option<(usize, String)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = line[level..].strip_prefix(' ')?;
    (1..=6)
        .contains(&level)
        .then(|| (level, rest.trim().to_string()))
}

/// Every symbol in the document, top to bottom: a plugin's, if one sent
/// them, else what the text declares.
pub fn symbols(doc: &Doc) -> Vec<Symbol> {
    if let Some(list) = &doc.layers.symbols {
        let mut out: Vec<Symbol> = list
            .iter()
            .map(|s| Symbol {
                line: doc.line_col(s.start).0,
                depth: s.depth,
                kind: s.kind.clone(),
                name: s.name.clone(),
                end_line: Some(doc.line_col(s.end).0),
            })
            .collect();
        out.sort_by_key(|s| s.line);
        return out;
    }
    found(doc, 0..=doc.last_line())
}

/// Lines back from a line that [`path_at`] looks for what it's in; past
/// them, a line's outer symbols are left out.
const PATH_REACH: usize = 10_000;

/// The symbols around `line`, outermost first, as [`path_to`] finds them
/// in [`symbols`], but from the lines around it only, so the breadcrumbs
/// cost the same in any size of file.
///
/// The finder starts afresh at a line with no indent, and a block ends at
/// one, so nothing above the last such line before `line` holds it.
pub fn path_at(doc: &Doc, line: usize) -> Vec<Symbol> {
    if needs_all(doc) {
        return path_to(&symbols(doc), doc, line);
    }
    let line = line.min(doc.last_line());
    let floor = line.saturating_sub(PATH_REACH);
    let mut top = line;
    while top > floor && indent(doc, top) != Some(0) {
        top -= 1;
    }
    path_to(&found(doc, top..=line), doc, line)
}

/// Whether a line's path takes every symbol in the file: a plugin's, which
/// may nest any way, or Markdown's, where a fence anywhere above counts.
pub(crate) fn needs_all(doc: &Doc) -> bool {
    doc.layers.symbols.is_some() || is_markdown(doc)
}

fn is_markdown(doc: &Doc) -> bool {
    doc.buffer
        .path
        .as_ref()
        .and_then(|p| p.extension())
        .is_some_and(|e| e == "md" || e == "markdown")
}

/// Characters of a line the finder reads.
const LINE_READ: usize = 1024;

/// A line's text without its line break, up to [`LINE_READ`] characters,
/// read in place when the rope holds
/// it in one piece.
fn line_text(doc: &Doc, line: usize) -> Cow<'_, str> {
    let slice = doc.buffer.rope().line(line);
    // A declaration or heading starts its line: a minified file's one
    // line is read no further than this.
    let slice = slice.slice(..slice.len_chars().min(LINE_READ));
    match slice.as_str() {
        Some(s) => Cow::Borrowed(s.trim_end_matches(['\n', '\r'])),
        None => Cow::Owned(slice.to_string().trim_end_matches(['\n', '\r']).to_string()),
    }
}

/// The symbols the text declares on `lines`, nested as if the finder had
/// started at their first.
fn found(doc: &Doc, lines: std::ops::RangeInclusive<usize>) -> Vec<Symbol> {
    let markdown = is_markdown(doc);
    let mut out = Vec::new();
    // The indent or heading level of each enclosing symbol.
    let mut stack: Vec<usize> = Vec::new();
    let mut in_fence = false;
    for line in lines {
        let text = line_text(doc, line);
        let text = text.as_ref();
        if markdown {
            if text.trim_start().starts_with("```") {
                in_fence = !in_fence;
            }
            if in_fence {
                continue;
            }
            if let Some((level, name)) = heading(text) {
                while stack.last().is_some_and(|&l| l >= level) {
                    stack.pop();
                }
                out.push(Symbol {
                    line,
                    depth: stack.len(),
                    kind: "#".into(),
                    name,
                    end_line: None,
                });
                stack.push(level);
            }
            continue;
        }
        let Some(at) = indent(doc, line) else {
            continue;
        };
        while stack.last().is_some_and(|&i| i >= at) {
            stack.pop();
        }
        if let Some((kind, name)) = declared(text) {
            out.push(Symbol {
                line,
                depth: stack.len(),
                kind,
                name,
                end_line: None,
            });
            stack.push(at);
        }
    }
    out
}

/// The symbols around a line, outermost first.
pub fn path_to(symbols: &[Symbol], doc: &Doc, line: usize) -> Vec<Symbol> {
    let mut path: Vec<Symbol> = Vec::new();
    for s in symbols.iter().take_while(|s| s.line <= line) {
        path.truncate(s.depth);
        // Only a symbol whose block reaches the line, or the line itself.
        let reaches = s.line == line
            || s.end_line.is_some_and(|end| end >= line)
            || s.end_line.is_none() && block_reaches(doc, s.line, line)
            || doc
                .buffer
                .path
                .as_ref()
                .and_then(|p| p.extension())
                .is_some_and(|e| e == "md");
        if reaches && path.len() == s.depth {
            path.push(s.clone());
        }
    }
    path
}

/// Whether the block that starts at `from` reaches `line`: as
/// `fold::block_end(doc, from) >= line`, but looking no further than the
/// first line with text at or past `line`.
fn block_reaches(doc: &Doc, from: usize, line: usize) -> bool {
    if doc.layers.folds.is_some() {
        return crate::editor::fold::block_end(doc, from).is_some_and(|end| end >= line);
    }
    let Some(base) = indent(doc, from) else {
        return false;
    };
    for l in from + 1..=doc.last_line() {
        match indent(doc, l) {
            None => continue,
            Some(i) if i > base => {
                if l >= line {
                    return true;
                }
            }
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path found from the lines around each line is the one the
    /// whole file's symbols give.
    #[test]
    fn the_path_from_nearby_lines_is_the_whole_files() {
        let python = "import os\n\nclass Outer:\n    x = 1\n\n    def first(self):\n        pass\n\n\n    class Inner:\n        def deep(self):\n            return 1\n\n    def after(self):\n        pass\n\ndef free():\n    pass\n";
        let rust = "mod a {\n    pub struct S;\n\n    impl S {\n        fn one(&self) {\n            let x = 1;\n        }\n\n        fn two(&self) {}\n    }\n}\n\nfn top() {\n\n    inner();\n}\n";
        let files = [
            ("a.py", python.to_string()),
            ("a.rs", rust.to_string()),
            ("symbols.rs", include_str!("symbols.rs").to_string()),
            ("crumbs.rs", include_str!("editor/crumbs.rs").to_string()),
        ];
        for (name, text) in files {
            let doc = Doc::new(1, &text, Some(name.into()));
            let all = symbols(&doc);
            for line in 0..=doc.last_line() {
                assert_eq!(
                    path_at(&doc, line),
                    path_to(&all, &doc, line),
                    "{name} line {}",
                    line + 1
                );
            }
        }
    }

    #[test]
    fn finds_declarations() {
        let d =
            |l: &str| declared(l).map(|(k, n)| if k == "impl" { n } else { format!("{k} {n}") });
        assert_eq!(
            d("pub(crate) fn split(&mut self) {").as_deref(),
            Some("fn split")
        );
        assert_eq!(
            d("impl<T> Iterator for Foo<T> {").as_deref(),
            Some("impl Iterator for Foo<T>")
        );
        assert_eq!(d("class Thing(Base):").as_deref(), Some("class Thing"));
        assert_eq!(
            d("export async function go() {").as_deref(),
            Some("function go")
        );
        assert_eq!(d("let fn_name = 3;"), None);
        assert_eq!(d("// fn commented()"), None);
    }
}
