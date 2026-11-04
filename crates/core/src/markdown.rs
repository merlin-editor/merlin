//! Markdown, as plugins write it, laid out for a panel: headings, lists,
//! quotes, bold, italics, inline code, links and fenced code blocks, which
//! get syntax colours. Anything else shows as written.

use std::path::Path;

use merlin_syntax::{Highlighter, Syntax};
use ropey::Rope;

use crate::panel::Row;
use crate::screen::{Style, char_width, str_width};

type Cell = (char, Style);

/// Lays out `text` in rows at most `width` cells wide.
pub(crate) fn layout(text: &str, width: usize) -> Vec<Row> {
    let width = width.max(4);
    let mut rows: Vec<Row> = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut lines = text.lines().peekable();
    let flush = |paragraph: &mut Vec<&str>, rows: &mut Vec<Row>| {
        if !paragraph.is_empty() {
            let joined = paragraph.join(" ");
            push_cells(
                rows,
                &inline(&joined, Style::Normal),
                "",
                "",
                Style::Normal,
                width,
            );
            paragraph.clear();
        }
    };
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(lang) = trimmed.strip_prefix("```") {
            flush(&mut paragraph, &mut rows);
            let mut code = Vec::new();
            for line in lines.by_ref() {
                if line.trim_start().starts_with("```") {
                    break;
                }
                code.push(line);
            }
            push_code(&mut rows, &code, lang.trim(), width);
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut rows);
            if rows.last().is_some_and(|r| !r.is_blank()) && lines.peek().is_some() {
                rows.push(Row::blank());
            }
            continue;
        }
        if let Some(block) = block_line(line) {
            flush(&mut paragraph, &mut rows);
            match block {
                Block::Heading(text) => push_cells(
                    &mut rows,
                    &inline(text, Style::Heading),
                    "",
                    "",
                    Style::Heading,
                    width,
                ),
                Block::Rule => rows.push(Row::text(&"─".repeat(width), Style::Muted)),
                Block::Item {
                    indent,
                    marker,
                    text,
                } => {
                    let pad = " ".repeat(indent);
                    let first = format!("{pad}{marker} ");
                    let rest = " ".repeat(str_width(&first));
                    push_cells(
                        &mut rows,
                        &inline(text, Style::Normal),
                        &first,
                        &rest,
                        Style::Muted,
                        width,
                    );
                }
                Block::Quote(text) => push_cells(
                    &mut rows,
                    &inline(text, Style::Muted),
                    "│ ",
                    "│ ",
                    Style::Muted,
                    width,
                ),
                Block::Raw(text) => push_cells(
                    &mut rows,
                    &inline(text, Style::Normal),
                    "",
                    "",
                    Style::Normal,
                    width,
                ),
            }
            continue;
        }
        paragraph.push(trimmed);
    }
    flush(&mut paragraph, &mut rows);
    while rows.last().is_some_and(Row::is_blank) {
        rows.pop();
    }
    rows
}

enum Block<'a> {
    Heading(&'a str),
    Rule,
    Item {
        indent: usize,
        marker: String,
        text: &'a str,
    },
    Quote(&'a str),
    /// A table row, kept on its own line.
    Raw(&'a str),
}

fn block_line(line: &str) -> Option<Block<'_>> {
    let trimmed = line.trim_start();
    let indent = (line.len() - trimmed.len()) / 2 * 2;
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        return Some(Block::Heading(trimmed[hashes..].trim()));
    }
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|c| compact.chars().all(|x| x.to_string() == *c))
    {
        return Some(Block::Rule);
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(text) = trimmed.strip_prefix(bullet) {
            return Some(Block::Item {
                indent,
                marker: "•".into(),
                text,
            });
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits < 4 {
        let rest = &trimmed[digits..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(Block::Item {
                indent,
                marker: format!("{}.", &trimmed[..digits]),
                text,
            });
        }
    }
    if let Some(text) = trimmed.strip_prefix('>') {
        return Some(Block::Quote(text.trim_start()));
    }
    if trimmed.starts_with('|') {
        return Some(Block::Raw(trimmed));
    }
    None
}

/// Inline markup: `**bold**`, `*italic*`, `` `code` `` and `[text](url)`.
fn inline(text: &str, base: Style) -> Vec<Cell> {
    let chars: Vec<char> = text.chars().collect();
    let mut cells = Vec::new();
    let mut bold = false;
    let mut italic = false;
    let mut i = 0;
    let style = |bold: bool, italic: bool| {
        if bold {
            Style::Heading
        } else if italic {
            Style::Emphasis
        } else {
            base
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '`'
            && let Some(len) = chars[i + 1..].iter().position(|x| *x == '`')
        {
            cells.extend(chars[i + 1..i + 1 + len].iter().map(|c| (*c, Style::Code)));
            i += len + 2;
            continue;
        }
        if (c == '*' || c == '_') && next == Some(c) {
            bold = !bold;
            i += 2;
            continue;
        }
        // A lone `*` or `_` inside a word, as in snake_case, is text.
        let in_word =
            i > 0 && chars[i - 1].is_alphanumeric() && next.is_some_and(char::is_alphanumeric);
        if (c == '*' || c == '_')
            && !in_word
            && (italic || next.is_some_and(|n| !n.is_whitespace()))
        {
            italic = !italic;
            i += 1;
            continue;
        }
        if c == '['
            && let Some(close) = chars[i..].iter().position(|x| *x == ']')
            && chars.get(i + close + 1) == Some(&'(')
            && let Some(end) = chars[i + close..].iter().position(|x| *x == ')')
        {
            cells.extend(chars[i + 1..i + close].iter().map(|c| (*c, Style::Link)));
            i += close + end + 1;
            continue;
        }
        cells.push((c, style(bold, italic)));
        i += 1;
    }
    cells
}

/// Word-wraps styled text. The first row starts with `first`, later ones
/// with `rest`, both drawn in `prefix_style`.
fn push_cells(
    rows: &mut Vec<Row>,
    cells: &[Cell],
    first: &str,
    rest: &str,
    prefix_style: Style,
    width: usize,
) {
    let room = width.saturating_sub(str_width(first)).max(1);
    let mut lines: Vec<Vec<Cell>> = vec![Vec::new()];
    let mut used = 0;
    for word in cells.split(|(c, _)| *c == ' ').filter(|w| !w.is_empty()) {
        let w: usize = word.iter().map(|(c, _)| char_width(*c)).sum();
        if used > 0 && used + 1 + w > room {
            lines.push(Vec::new());
            used = 0;
        }
        let line = lines.last_mut().unwrap();
        if used > 0 {
            line.push((' ', line.last().map_or(Style::Normal, |(_, s)| *s)));
            used += 1;
        }
        for &(c, style) in word {
            let cw = char_width(c);
            if used + cw > room && used > 0 {
                lines.push(Vec::new());
                used = 0;
            }
            lines.last_mut().unwrap().push((c, style));
            used += cw;
        }
    }
    for (i, line) in lines.into_iter().enumerate() {
        let prefix = if i == 0 { first } else { rest };
        let mut segments = Vec::new();
        if !prefix.is_empty() {
            segments.push((prefix.to_string(), prefix_style, Syntax::Plain));
        }
        segments.extend(group(line.into_iter().map(|(c, s)| (c, s, Syntax::Plain))));
        rows.push(Row::from_segments(segments));
    }
}

/// Runs of characters with the same look, as segments.
fn group(cells: impl Iterator<Item = (char, Style, Syntax)>) -> Vec<(String, Style, Syntax)> {
    let mut segments: Vec<(String, Style, Syntax)> = Vec::new();
    for (c, style, syntax) in cells {
        match segments.last_mut() {
            Some((text, s, x)) if *s == style && *x == syntax => text.push(c),
            _ => segments.push((c.to_string(), style, syntax)),
        }
    }
    segments
}

/// File extensions for the languages merlin highlights, by the names
/// fences use.
fn extension(lang: &str) -> &str {
    match lang.to_ascii_lowercase().as_str() {
        "rust" | "rs" => "rs",
        "python" | "py" => "py",
        "javascript" | "js" | "jsx" => "js",
        "json" => "json",
        "toml" => "toml",
        _ => "",
    }
}

fn push_code(rows: &mut Vec<Row>, code: &[&str], lang: &str, width: usize) {
    let text = code.join("\n");
    let rope = Rope::from_str(&text);
    let mut syntax = vec![Syntax::Plain; rope.len_chars()];
    let ext = extension(lang);
    if let Some(mut h) = (!ext.is_empty())
        .then(|| Highlighter::for_path(Path::new(&format!("code.{ext}"))))
        .flatten()
    {
        h.parse(&rope);
        for (range, kind) in h.highlights(&rope, 0..rope.len_lines()) {
            let (a, b) = (rope.byte_to_char(range.start), rope.byte_to_char(range.end));
            syntax[a..b].fill(kind);
        }
    }
    // One cell of padding on the left; long lines are cut into pieces.
    let room = width.saturating_sub(1).max(1);
    let mut pos = 0;
    for line in code {
        let chars: Vec<(char, Syntax)> = line.chars().zip(syntax[pos..].iter().copied()).collect();
        pos += chars.len() + 1;
        let mut pieces: Vec<Vec<(char, Syntax)>> = vec![Vec::new()];
        let mut used = 0;
        for (c, s) in chars {
            // Tabs would throw off the width; they're rare in what plugins write.
            let c = if c == '\t' { ' ' } else { c };
            let w = char_width(c);
            if used + w > room && used > 0 {
                pieces.push(Vec::new());
                used = 0;
            }
            pieces.last_mut().unwrap().push((c, s));
            used += w;
        }
        for piece in pieces {
            let mut segments = vec![(" ".to_string(), Style::CodeBlock, Syntax::Plain)];
            segments.extend(group(
                piece.into_iter().map(|(c, s)| (c, Style::CodeBlock, s)),
            ));
            let mut row = Row::from_segments(segments);
            row.fill = Some(Style::CodeBlock);
            rows.push(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| r.segments.iter().map(|(t, _, _)| t.as_str()).collect())
            .collect()
    }

    fn style_of(rows: &[Row], needle: &str) -> Style {
        rows.iter()
            .flat_map(|r| &r.segments)
            .find(|(t, _, _)| t.contains(needle))
            .map(|(_, s, _)| *s)
            .unwrap()
    }

    #[test]
    fn paragraphs_join_and_wrap() {
        let rows = layout("one two\nthree four five", 10);
        assert_eq!(text(&rows), ["one two", "three four", "five"]);
    }

    #[test]
    fn inline_styles() {
        let rows = layout(
            "a **bold** and *it* and `code_x` and [link](http://x) snake_case",
            80,
        );
        assert_eq!(
            text(&rows),
            ["a bold and it and code_x and link snake_case"]
        );
        assert_eq!(style_of(&rows, "bold"), Style::Heading);
        assert_eq!(style_of(&rows, "it"), Style::Emphasis);
        assert_eq!(style_of(&rows, "code_x"), Style::Code);
        assert_eq!(style_of(&rows, "link"), Style::Link);
    }

    #[test]
    fn lists_hang_their_indent() {
        let rows = layout("- first item wraps\n  - nested\n2. two", 12);
        assert_eq!(
            text(&rows),
            ["• first item", "  wraps", "  • nested", "2. two"]
        );
    }

    #[test]
    fn code_blocks_keep_lines_and_get_colours() {
        let rows = layout(
            "Look:\n```rust\nfn main() {\n    let x = 1;\n}\n```\nDone",
            40,
        );
        assert_eq!(
            text(&rows),
            ["Look:", " fn main() {", "     let x = 1;", " }", "Done"]
        );
        assert_eq!(rows[1].fill, Some(Style::CodeBlock));
        let keyword = rows[1].segments.iter().find(|(t, _, _)| t == "fn").unwrap();
        assert_eq!(keyword.2, Syntax::Keyword);
    }

    #[test]
    fn an_unclosed_fence_is_code_to_the_end() {
        let rows = layout("```\nstreaming", 20);
        assert_eq!(text(&rows), [" streaming"]);
    }

    #[test]
    fn headings_and_blank_lines() {
        let rows = layout("# Title\n\n\nBody", 20);
        assert_eq!(text(&rows), ["Title", "", "Body"]);
        assert_eq!(style_of(&rows, "Title"), Style::Heading);
    }
}
