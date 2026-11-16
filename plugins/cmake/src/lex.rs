//! A lexer for CMake's colours: comments and bracket comments, strings
//! and bracket arguments, variable references, generator expressions,
//! commands and their control-flow words, and the all-caps words that
//! name a command's options.

/// What a run of text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    Function,
    String,
    Escape,
    Comment,
    Variable,
    Macro,
    Constant,
}

/// A run on one line: its line, and its start and end in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

/// Commands that shape the flow, coloured as keywords.
const CONTROL: &[&str] = &[
    "if",
    "elseif",
    "else",
    "endif",
    "foreach",
    "endforeach",
    "while",
    "endwhile",
    "function",
    "endfunction",
    "macro",
    "endmacro",
    "block",
    "endblock",
    "return",
    "break",
    "continue",
];

/// Words that are values, whatever their case.
const CONSTANTS: &[&str] = &[
    "ON", "OFF", "TRUE", "FALSE", "YES", "NO", "Y", "N", "IGNORE", "NOTFOUND",
];

struct Lexer {
    chars: Vec<char>,
    i: usize,
    /// Spans over the whole text, split into lines at the end.
    spans: Vec<(usize, usize, Kind)>,
    /// Inside a command's parentheses, and how deep.
    depth: usize,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

impl Lexer {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).copied()
    }

    fn span(&mut self, from: usize, kind: Kind) {
        if self.i > from {
            self.spans.push((from, self.i, kind));
        }
    }

    /// `[[`, `[=[`, `[==[` and so on at the current position: how many `=`.
    fn bracket_open(&self) -> Option<usize> {
        if self.peek(0) != Some('[') {
            return None;
        }
        let mut n = 0;
        while self.peek(1 + n) == Some('=') {
            n += 1;
        }
        (self.peek(1 + n) == Some('[')).then_some(n)
    }

    /// Past a bracket's contents to its matching close, or the end.
    fn bracket(&mut self, equals: usize) {
        self.i += equals + 2;
        let close: Vec<char> = std::iter::once(']')
            .chain(std::iter::repeat_n('=', equals))
            .chain(std::iter::once(']'))
            .collect();
        while self.i < self.chars.len() {
            if self.chars[self.i..].starts_with(&close) {
                self.i += close.len();
                return;
            }
            self.i += 1;
        }
    }

    /// `${…}`, `$ENV{…}`, `$CACHE{…}` and `$<…>` from the `$`, nested, as a
    /// variable or a generator expression. Whether there was one.
    fn reference(&mut self) -> bool {
        let from = self.i;
        let mut j = self.i + 1;
        while self
            .chars
            .get(j)
            .copied()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            j += 1;
        }
        let (open, close, kind) = match self.chars.get(j) {
            Some('{') => ('{', '}', Kind::Variable),
            Some('<') if j == self.i + 1 => ('<', '>', Kind::Macro),
            _ => return false,
        };
        self.i = j + 1;
        let mut depth = 1;
        while let Some(c) = self.peek(0) {
            if c == '\n' && open == '<' {
                break;
            }
            self.i += 1;
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
        }
        self.span(from, kind);
        true
    }

    /// A quoted argument, with its escapes and references.
    fn quoted(&mut self) {
        let mut start = self.i;
        self.i += 1;
        while let Some(c) = self.peek(0) {
            match c {
                '\\' => {
                    self.span(start, Kind::String);
                    let at = self.i;
                    self.i += 1;
                    if self.peek(0).is_some() {
                        self.i += 1;
                    }
                    self.span(at, Kind::Escape);
                    start = self.i;
                }
                '$' => {
                    let here = self.i;
                    self.span(start, Kind::String);
                    if !self.reference() {
                        self.i = here + 1;
                        start = here;
                        continue;
                    }
                    start = self.i;
                }
                '"' => {
                    self.i += 1;
                    break;
                }
                _ => self.i += 1,
            }
        }
        self.span(start, Kind::String);
    }

    fn lex(&mut self) {
        while let Some(c) = self.peek(0) {
            let from = self.i;
            match c {
                '#' => {
                    self.i += 1;
                    match self.bracket_open() {
                        Some(n) => self.bracket(n),
                        None => {
                            while self.peek(0).is_some_and(|c| c != '\n') {
                                self.i += 1;
                            }
                        }
                    }
                    self.span(from, Kind::Comment);
                }
                '"' => self.quoted(),
                '[' if self.bracket_open().is_some() => {
                    let n = self.bracket_open().unwrap_or(0);
                    self.bracket(n);
                    self.span(from, Kind::String);
                }
                '$' => {
                    if !self.reference() {
                        self.i += 1;
                    }
                }
                '(' => {
                    self.depth += 1;
                    self.i += 1;
                }
                ')' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.i += 1;
                }
                c if is_ident_start(c) => {
                    while self.peek(0).is_some_and(is_ident) {
                        self.i += 1;
                    }
                    let word: String = self.chars[from..self.i].iter().collect();
                    if self.depth == 0 {
                        // A command: the name before its `(`.
                        let mut j = self.i;
                        while self.chars.get(j).is_some_and(|c| *c == ' ' || *c == '\t') {
                            j += 1;
                        }
                        if self.chars.get(j) == Some(&'(') {
                            let lower = word.to_ascii_lowercase();
                            let kind = if CONTROL.contains(&lower.as_str()) {
                                Kind::Keyword
                            } else {
                                Kind::Function
                            };
                            self.span(from, kind);
                        }
                    } else if CONSTANTS.contains(&word.to_ascii_uppercase().as_str())
                        || (word.len() > 1
                            && word.starts_with(|c: char| c.is_ascii_uppercase())
                            && word
                                .chars()
                                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                    {
                        // A value such as ON, or an option such as PUBLIC.
                        self.span(from, Kind::Constant);
                    }
                }
                c if c.is_ascii_digit() && self.depth > 0 => {
                    while self
                        .peek(0)
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '.')
                    {
                        self.i += 1;
                    }
                    self.span(from, Kind::Constant);
                }
                _ => self.i += 1,
            }
        }
    }
}

/// The coloured runs in a file, in order, split at line ends.
pub fn lex(text: &str) -> Vec<Run> {
    let mut lexer = Lexer {
        chars: text.chars().collect(),
        i: 0,
        spans: Vec::new(),
        depth: 0,
    };
    lexer.lex();
    // Where each line starts, in characters.
    let mut starts = vec![0];
    for (i, c) in lexer.chars.iter().enumerate() {
        if *c == '\n' {
            starts.push(i + 1);
        }
    }
    let mut runs = Vec::new();
    for (a, b, kind) in lexer.spans {
        let mut line = starts.partition_point(|&s| s <= a) - 1;
        let mut from = a;
        while from < b {
            let next = starts.get(line + 1).copied().unwrap_or(usize::MAX);
            // A line's run stops before its `\n`.
            let to = b.min(next.saturating_sub(1));
            if to > from {
                runs.push(Run {
                    line,
                    start: from - starts[line],
                    end: to - starts[line],
                    kind,
                });
            }
            if next >= b {
                break;
            }
            line += 1;
            from = next;
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(text: &str) -> Vec<(String, Kind)> {
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        lex(text)
            .into_iter()
            .map(|r| (lines[r.line][r.start..r.end].iter().collect(), r.kind))
            .collect()
    }

    #[test]
    fn commands_options_and_variables() {
        let found = runs(
            "add_library(shapes STATIC ${SOURCES})\n\
             target_link_libraries(shapes PUBLIC fmt::fmt)",
        );
        assert_eq!(
            found,
            [
                ("add_library".into(), Kind::Function),
                ("STATIC".into(), Kind::Constant),
                ("${SOURCES}".into(), Kind::Variable),
                ("target_link_libraries".into(), Kind::Function),
                ("PUBLIC".into(), Kind::Constant),
            ]
        );
    }

    #[test]
    fn control_flow_strings_and_generator_expressions() {
        let found = runs("if (WIN32)\n  set(X \"a\\n${Y}\" $<CONFIG:Debug>)\nendif()");
        assert_eq!(
            found,
            [
                ("if".into(), Kind::Keyword),
                ("WIN32".into(), Kind::Constant),
                ("set".into(), Kind::Function),
                ("\"a".into(), Kind::String),
                ("\\n".into(), Kind::Escape),
                ("${Y}".into(), Kind::Variable),
                ("\"".into(), Kind::String),
                ("$<CONFIG:Debug>".into(), Kind::Macro),
                ("endif".into(), Kind::Keyword),
            ]
        );
    }

    #[test]
    fn comments_and_brackets_span_lines() {
        let found = runs("#[[ one\ntwo ]] message([=[ a\nb ]=]) # done\n$ENV{HOME}");
        assert_eq!(
            found,
            [
                ("#[[ one".into(), Kind::Comment),
                ("two ]]".into(), Kind::Comment),
                ("message".into(), Kind::Function),
                ("[=[ a".into(), Kind::String),
                ("b ]=]".into(), Kind::String),
                ("# done".into(), Kind::Comment),
                ("$ENV{HOME}".into(), Kind::Variable),
            ]
        );
    }

    #[test]
    fn nested_references_and_unclosed_text_end_safely() {
        let found = runs("set(A ${B_${C}} 3.20)\nmessage(\"open");
        assert_eq!(found[1], ("${B_${C}}".into(), Kind::Variable));
        assert_eq!(found[2], ("3.20".into(), Kind::Constant));
        assert_eq!(found.last(), Some(&("\"open".into(), Kind::String)));
    }
}
