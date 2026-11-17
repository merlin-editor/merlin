//! A quick look through C and C++ sources for what they define, for the
//! palette's symbol search when clangd has no compile commands and so
//! knows only open files: namespaces, classes, structs, unions, enums,
//! functions and macros, by their names and where they are.
//!
//! It reads tokens, not C++: it doesn't know types or overloads, and a
//! container is what the name is written in, as `Circle` for
//! `Circle::area` or a class's body.

use merlin_lsp::Scanned;

/// A word or a mark, and where it starts: line, and byte in the line.
#[derive(Clone, Debug, PartialEq)]
struct Tok<'a> {
    text: &'a str,
    line: u32,
    at: usize,
}

/// Words that look like a call's name but aren't a function's.
const NOT_NAMES: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "return",
    "sizeof",
    "alignof",
    "decltype",
    "catch",
    "static_assert",
    "noexcept",
    "throw",
    "new",
    "delete",
    "typeid",
    "alignas",
    "defined",
    "__attribute__",
    "__declspec",
    "operator",
    "requires",
    "co_return",
    "co_await",
    "case",
];

/// Words after a function's `)` before its body or `;`.
const AFTER_PARAMS: &[&str] = &[
    "const", "noexcept", "override", "final", "volatile", "&", "&&", "mutable", "try",
];

/// Comments, strings and characters blanked, so they can't look like
/// code, the lines kept where they were; and `#define`d names.
fn code(text: &str) -> (String, Vec<Scanned>) {
    let mut out = String::with_capacity(text.len());
    let mut macros = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut line = 0u32;
    let mut line_start = 0usize;
    let mut at_line_start = true;
    while let Some((i, c)) = chars.next() {
        if c == '\n' {
            out.push('\n');
            line += 1;
            line_start = i + 1;
            at_line_start = true;
            continue;
        }
        // A preprocessor line: a #define's name kept, the rest blanked,
        // through its backslashed lines.
        if at_line_start && c == '#' {
            let end = text[i..]
                .match_indices('\n')
                .map(|(n, _)| i + n)
                .find(|&n| !text[..n].trim_end_matches('\r').ends_with('\\'))
                .unwrap_or(text.len());
            let directive = &text[i..end];
            let rest = directive[1..].trim_start();
            if let Some(after) = rest.strip_prefix("define") {
                let name: String = after
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() && after.starts_with(char::is_whitespace) {
                    let col = i + directive.find(&name).unwrap_or(0) - line_start;
                    macros.push(Scanned {
                        name,
                        kind: "macro",
                        container: String::new(),
                        line,
                        column: utf16(&text[line_start..], col),
                    });
                }
            }
            for d in directive.chars() {
                if d == '\n' {
                    out.push('\n');
                    line += 1;
                } else {
                    blank(&mut out, d);
                }
            }
            if let Some(last) = directive.rfind('\n') {
                line_start = i + last + 1;
            }
            while chars.peek().is_some_and(|&(j, _)| j < end) {
                chars.next();
            }
            at_line_start = false;
            continue;
        }
        if !c.is_whitespace() {
            at_line_start = false;
        }
        match c {
            '/' if chars.peek().map(|&(_, n)| n) == Some('/') => {
                out.push(' ');
                while let Some(&(_, n)) = chars.peek() {
                    if n == '\n' {
                        break;
                    }
                    chars.next();
                    blank(&mut out, n);
                }
            }
            '/' if chars.peek().map(|&(_, n)| n) == Some('*') => {
                out.push(' ');
                let mut last = ' ';
                for (j, n) in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        line += 1;
                        line_start = j + 1;
                    } else {
                        blank(&mut out, n);
                    }
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
            }
            '"' | '\'' => {
                out.push(' ');
                let mut escaped = false;
                for (j, n) in chars.by_ref() {
                    if n == '\n' {
                        // An unclosed one ends at the line.
                        out.push('\n');
                        line += 1;
                        line_start = j + 1;
                        at_line_start = true;
                        break;
                    }
                    blank(&mut out, n);
                    if escaped {
                        escaped = false;
                    } else if n == '\\' {
                        escaped = true;
                    } else if n == c {
                        break;
                    }
                }
            }
            c => out.push(c),
        }
    }
    (out, macros)
}

/// Spaces in place of `c`, as many as its bytes, so what follows stays
/// at the same byte.
fn blank(out: &mut String, c: char) {
    for _ in 0..c.len_utf8() {
        out.push(' ');
    }
}

/// The UTF-16 column of byte `at` in `line`.
fn utf16(line: &str, at: usize) -> u32 {
    let at = at.min(line.len());
    let mut end = at;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    line[..end].encode_utf16().count() as u32
}

/// The words and marks of blanked code; `::` and `->` as one.
fn tokens(code: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    for (line, text) in code.split('\n').enumerate() {
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
                continue;
            }
            let start = i;
            if c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80 {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] >= 0x80)
                {
                    i += 1;
                }
            } else if text[i..].starts_with("::")
                || text[i..].starts_with("->")
                || text[i..].starts_with("&&")
            {
                i += 2;
            } else {
                i += 1;
            }
            out.push(Tok {
                text: &text[start..i],
                line: line as u32,
                at: start,
            });
        }
    }
    out
}

fn is_name(t: &str) -> bool {
    t.starts_with(|c: char| c.is_alphabetic() || c == '_')
}

/// What a `{` opened.
#[derive(Clone, Debug, PartialEq)]
enum Scope {
    /// A namespace or a class, struct, union or enum, by name: names
    /// written in it are its.
    Named(String),
    /// A function's body, an initializer, or anything else: nothing in it
    /// is looked at.
    Body,
}

/// The definitions in a C or C++ file's text.
pub fn symbols(text: &str) -> Vec<Scanned> {
    let (blank, mut found) = code(text);
    let lines: Vec<&str> = text.split('\n').collect();
    let toks = tokens(&blank);
    let mut scopes: Vec<Scope> = Vec::new();
    // A `{` that a definition just found opens, and with what.
    let mut opening: Option<Scope> = None;
    let container = |scopes: &[Scope], extra: &str| -> String {
        let mut parts: Vec<&str> = scopes
            .iter()
            .filter_map(|s| match s {
                Scope::Named(n) if !n.is_empty() => Some(n.as_str()),
                _ => None,
            })
            .collect();
        if !extra.is_empty() {
            parts.push(extra);
        }
        parts.join("::")
    };
    let at = |t: &Tok| utf16(lines.get(t.line as usize).copied().unwrap_or(""), t.at);
    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        match t.text {
            "{" => {
                scopes.push(opening.take().unwrap_or(Scope::Body));
                i += 1;
                continue;
            }
            "}" => {
                scopes.pop();
                i += 1;
                continue;
            }
            ";" => {
                opening = None;
                i += 1;
                continue;
            }
            _ => {}
        }
        // Inside a body, only braces count.
        if scopes.contains(&Scope::Body) {
            i += 1;
            continue;
        }
        let in_class =
            matches!(scopes.last(), Some(Scope::Named(_))) && class_depth(&toks, i).is_some();
        match t.text {
            "namespace" => {
                // `namespace a::b {`, or one with no name.
                let mut j = i + 1;
                let mut name = String::new();
                let mut first: Option<&Tok> = None;
                while let Some(n) = toks.get(j) {
                    if is_name(n.text) {
                        first.get_or_insert(n);
                        name.push_str(n.text);
                    } else if n.text == "::" {
                        name.push_str("::");
                    } else {
                        break;
                    }
                    j += 1;
                }
                if toks.get(j).is_some_and(|n| n.text == "{") {
                    if let Some(f) = first {
                        found.push(Scanned {
                            name: name.clone(),
                            kind: "namespace",
                            container: container(&scopes, ""),
                            line: f.line,
                            column: at(f),
                        });
                    }
                    opening = Some(Scope::Named(name));
                }
                i = j;
                continue;
            }
            "class" | "struct" | "union" | "enum" => {
                let kind = match t.text {
                    "class" => "class",
                    "struct" => "struct",
                    "union" => "union",
                    _ => "enum",
                };
                let mut j = i + 1;
                // `enum class`, attributes and alignas before the name.
                while let Some(n) = toks.get(j) {
                    match n.text {
                        "class" | "struct" | "final" => j += 1,
                        "[" => j = skip_pair(&toks, j, "[", "]"),
                        "alignas" | "__attribute__" | "__declspec" => {
                            j = skip_pair(&toks, j + 1, "(", ")")
                        }
                        _ => break,
                    }
                }
                let name = toks.get(j).filter(|n| is_name(n.text));
                // What follows decides: a body is a definition.
                let mut k = j + usize::from(name.is_some());
                while let Some(n) = toks.get(k) {
                    if matches!(n.text, "{" | ";" | "(" | ")" | "=" | ",") {
                        break;
                    }
                    k += 1;
                }
                if toks.get(k).is_some_and(|n| n.text == "{") {
                    if let Some(n) = name {
                        found.push(Scanned {
                            name: n.text.to_string(),
                            kind,
                            container: container(&scopes, ""),
                            line: n.line,
                            column: at(n),
                        });
                    }
                    // An enum's values aren't looked at; a class's members are.
                    opening = Some(if kind == "enum" {
                        Scope::Body
                    } else {
                        Scope::Named(name.map(|n| n.text.to_string()).unwrap_or_default())
                    });
                    i = k;
                } else {
                    i = j + 1;
                }
                continue;
            }
            _ => {}
        }
        // A function: a name, maybe `A::B::`-qualified, then `(`.
        if is_name(t.text)
            && !NOT_NAMES.contains(&t.text)
            && toks.get(i + 1).is_some_and(|n| n.text == "(")
            && i.checked_sub(1).is_none_or(|p| {
                !matches!(
                    toks[p].text,
                    "." | "->" | "=" | "new" | "return" | "~" | "operator" | ","
                )
            })
        {
            let mut qualifiers = Vec::new();
            let mut p = i;
            while p >= 2 && toks[p - 1].text == "::" && is_name(toks[p - 2].text) {
                qualifiers.insert(0, toks[p - 2].text);
                p -= 2;
            }
            let close = skip_pair(&toks, i + 1, "(", ")");
            let mut k = close;
            while let Some(n) = toks.get(k) {
                if AFTER_PARAMS.contains(&n.text) {
                    k += 1;
                } else if n.text == "->" {
                    // A trailing return type, up to the body.
                    k += 1;
                    while toks
                        .get(k)
                        .is_some_and(|n| !matches!(n.text, "{" | ";" | "="))
                    {
                        k += 1;
                    }
                } else {
                    break;
                }
            }
            let after = toks.get(k).map(|n| n.text);
            let defines = matches!(after, Some("{" | ":"));
            let declares = in_class && matches!(after, Some(";" | "="));
            if defines || declares {
                let own = qualifiers.join("::");
                found.push(Scanned {
                    name: t.text.to_string(),
                    kind: if in_class || !own.is_empty() {
                        "method"
                    } else {
                        "function"
                    },
                    container: container(&scopes, &own),
                    line: t.line,
                    column: at(t),
                });
            }
            if after == Some(":") {
                // A constructor's initializers, up to its body.
                let mut m = k + 1;
                while let Some(n) = toks.get(m) {
                    match n.text {
                        "(" => m = skip_pair(&toks, m, "(", ")"),
                        "{" if toks.get(m - 1).is_some_and(|b| is_name(b.text)) => {
                            m = skip_pair(&toks, m, "{", "}")
                        }
                        "{" => break,
                        _ => m += 1,
                    }
                }
                i = m;
            } else {
                i = k;
            }
            continue;
        }
        i += 1;
    }
    found.sort_by_key(|s| (s.line, s.column));
    found
}

/// Whether the innermost scope at token `i` is a class's: some named
/// scope is, which the caller already checked is innermost.
fn class_depth(toks: &[Tok], i: usize) -> Option<()> {
    // A namespace's names aren't members: look back for what opened the
    // innermost `{`.
    let mut depth = 0i32;
    let mut j = i;
    while j > 0 {
        j -= 1;
        match toks[j].text {
            "}" => depth += 1,
            "{" if depth > 0 => depth -= 1,
            "{" => {
                // Back from the brace to `namespace` or a class word.
                let mut k = j;
                while k > 0 {
                    k -= 1;
                    match toks[k].text {
                        "namespace" => return None,
                        "class" | "struct" | "union" => return Some(()),
                        ";" | "{" | "}" => return None,
                        _ => {}
                    }
                }
                return None;
            }
            _ => {}
        }
    }
    None
}

/// The index after the mark that closes the one at `from`.
fn skip_pair(toks: &[Tok], from: usize, open: &str, close: &str) -> usize {
    let mut depth = 0;
    let mut i = from;
    while let Some(t) = toks.get(i) {
        if t.text == open {
            depth += 1;
        } else if t.text == close {
            depth -= 1;
            if depth <= 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, &'static str, String, u32)> {
        symbols(text)
            .into_iter()
            .map(|s| (s.name, s.kind, s.container, s.line))
            .collect()
    }

    #[test]
    fn definitions_and_members() {
        let text = r#"#include "shape.h"
#define AREA_EPS 1e-9
// int fake(void) {}
namespace geo {
struct Point { double x, y; };
class Shape {
public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
    Shape(int n) : n_(n), v_{1} {}
private:
    int n_;
};
enum class Kind { Circle, Square };
double Circle::area() const {
    if (r > 0) { return helper(r); }
    const char *s = "void nope() {";
    return 1;
}
}
static int helper(int r) { return r * 2; }
int prototype(int);
auto trailing(int x) -> int { return x; }
"#;
        let found = names(text);
        let want = [
            ("AREA_EPS", "macro", "", 1),
            ("geo", "namespace", "", 3),
            ("Point", "struct", "geo", 4),
            ("Shape", "class", "geo", 5),
            ("area", "method", "geo::Shape", 8),
            ("Shape", "method", "geo::Shape", 9),
            ("Kind", "enum", "geo", 13),
            ("area", "method", "geo::Circle", 14),
            ("helper", "function", "", 20),
            ("trailing", "function", "", 22),
        ];
        let want: Vec<_> = want
            .iter()
            .map(|(n, k, c, l)| (n.to_string(), *k, c.to_string(), *l))
            .collect();
        assert_eq!(found, want);
    }

    #[test]
    fn columns_are_utf16() {
        let s = symbols("/* é */ int f(void) {}\n");
        assert_eq!((s[0].line, s[0].column), (0, 12));
    }
}
