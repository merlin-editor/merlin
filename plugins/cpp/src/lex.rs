//! A lexer for C and C++ colours, as a VS Code grammar gives them:
//! comments, strings and their escapes, numbers, keywords, preprocessor
//! lines, names of types being declared, and calls. It works on the whole
//! file, so comments and raw strings can span lines.

/// What a run of text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyword,
    String,
    Escape,
    Comment,
    Number,
    Macro,
    Type,
    Function,
}

/// A run on one line: its line, and its start and end in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

const KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "co_await",
    "co_return",
    "co_yield",
    "compl",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "false",
    "final",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "import",
    "inline",
    "int",
    "long",
    "module",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "nullptr",
    "operator",
    "or",
    "or_eq",
    "override",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "restrict",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "true",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
    "xor_eq",
    "_Bool",
    "_Complex",
    "_Atomic",
    "_Generic",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "NULL",
];

/// Words after which a name is a type being declared.
const DECLARES: &[&str] = &[
    "class",
    "struct",
    "enum",
    "union",
    "namespace",
    "typename",
    "concept",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    line: usize,
    /// Where the current line starts in `chars`.
    line_start: usize,
    runs: Vec<Run>,
}

impl Lexer {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.i + k).copied()
    }

    /// Adds a run from `from` to the current position, split at line ends.
    fn run(&mut self, from: usize, kind: Kind) {
        let mut line = self.line;
        let mut line_start = self.line_start;
        // Walk back to the line `from` is on.
        let mut starts = vec![(line, line_start)];
        while from < line_start {
            let prev = self.chars[..line_start - 1]
                .iter()
                .rposition(|&c| c == '\n')
                .map_or(0, |p| p + 1);
            line -= 1;
            line_start = prev;
            starts.push((line, line_start));
        }
        starts.reverse();
        for (k, &(l, s)) in starts.iter().enumerate() {
            let a = from.max(s);
            let b = match starts.get(k + 1) {
                Some(&(_, next)) => next - 1,
                None => self.i,
            };
            if b > a {
                self.runs.push(Run {
                    line: l,
                    start: a - s,
                    end: b - s,
                    kind,
                });
            }
        }
    }

    fn bump(&mut self) {
        if self.chars.get(self.i) == Some(&'\n') {
            self.line += 1;
            self.line_start = self.i + 1;
        }
        self.i += 1;
    }

    /// The previous word on this line before `at`, skipping spaces.
    fn word_before(&self, at: usize) -> String {
        let mut j = at;
        while j > self.line_start && self.chars[j - 1] == ' ' {
            j -= 1;
        }
        let end = j;
        while j > self.line_start && is_ident(self.chars[j - 1]) {
            j -= 1;
        }
        self.chars[j..end].iter().collect()
    }

    /// A quoted string or character, with its escapes, from the quote.
    fn quoted(&mut self, quote: char, from: usize) {
        let mut start = from;
        self.bump();
        while let Some(c) = self.peek(0) {
            if c == '\n' {
                break;
            }
            if c == '\\' {
                self.run_between(start, self.i, Kind::String);
                let at = self.i;
                self.bump();
                if self.peek(0).is_some_and(|c| c != '\n') {
                    self.bump();
                }
                self.run_between(at, self.i, Kind::Escape);
                start = self.i;
                continue;
            }
            self.bump();
            if c == quote {
                break;
            }
        }
        self.run_between(start, self.i, Kind::String);
    }

    /// A run between two positions on the current line.
    fn run_between(&mut self, a: usize, b: usize, kind: Kind) {
        if b > a && a >= self.line_start {
            self.runs.push(Run {
                line: self.line,
                start: a - self.line_start,
                end: b - self.line_start,
                kind,
            });
        }
    }

    /// A raw string, `R"delim( … )delim"`, from the `R`.
    fn raw(&mut self, from: usize) {
        // Skip to the quote, then read the delimiter up to `(`.
        while self.peek(0) != Some('"') {
            self.bump();
        }
        self.bump();
        let mut delim = String::new();
        while let Some(c) = self.peek(0) {
            if c == '(' || c == '\n' || delim.len() > 16 {
                break;
            }
            delim.push(c);
            self.bump();
        }
        let close: Vec<char> = format!("){delim}\"").chars().collect();
        while self.i < self.chars.len() {
            if self.chars[self.i..].starts_with(&close) {
                for _ in 0..close.len() {
                    self.bump();
                }
                break;
            }
            self.bump();
        }
        self.run(from, Kind::String);
    }

    fn lex(&mut self) {
        while let Some(c) = self.peek(0) {
            let from = self.i;
            // Whether only spaces came before on this line, for `#`.
            let line_first = self.chars[self.line_start..self.i]
                .iter()
                .all(|c| c.is_whitespace());
            match c {
                '/' if self.peek(1) == Some('/') => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                    self.run(from, Kind::Comment);
                }
                '/' if self.peek(1) == Some('*') => {
                    self.bump();
                    self.bump();
                    while self.i < self.chars.len()
                        && !(self.peek(0) == Some('*') && self.peek(1) == Some('/'))
                    {
                        self.bump();
                    }
                    self.bump();
                    self.bump();
                    self.i = self.i.min(self.chars.len());
                    self.run(from, Kind::Comment);
                }
                '#' if line_first => self.directive(from),
                '"' | '\'' => self.quoted(c, from),
                c if c.is_ascii_digit()
                    || c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit()) =>
                {
                    while let Some(c) = self.peek(0) {
                        let exponent = matches!(c, '+' | '-')
                            && matches!(self.chars[self.i - 1], 'e' | 'E' | 'p' | 'P');
                        let separator =
                            c == '\'' && self.peek(1).is_some_and(|d| d.is_ascii_alphanumeric());
                        if c.is_ascii_alphanumeric() || c == '.' || exponent || separator {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    self.run(from, Kind::Number);
                }
                c if is_ident_start(c) => {
                    while self.peek(0).is_some_and(is_ident) {
                        self.bump();
                    }
                    let word: String = self.chars[from..self.i].iter().collect();
                    // String prefixes: u8"…", L'…', R"(…)" and so on.
                    let quote = self.peek(0);
                    if matches!(quote, Some('"' | '\''))
                        && matches!(word.as_str(), "u8" | "u" | "U" | "L")
                    {
                        self.quoted(quote.unwrap_or('"'), from);
                        continue;
                    }
                    if quote == Some('"')
                        && matches!(word.as_str(), "R" | "u8R" | "uR" | "UR" | "LR")
                    {
                        self.i = from;
                        self.raw(from);
                        continue;
                    }
                    if KEYWORDS.contains(&word.as_str()) {
                        self.run(from, Kind::Keyword);
                        continue;
                    }
                    let before = self.word_before(from);
                    let mut j = self.i;
                    while self.chars.get(j) == Some(&' ') {
                        j += 1;
                    }
                    if DECLARES.contains(&before.as_str()) {
                        self.run(from, Kind::Type);
                    } else if self.chars.get(j) == Some(&'(') {
                        self.run(from, Kind::Function);
                    }
                }
                _ => self.bump(),
            }
        }
    }

    /// A preprocessor line: the directive, and an include's file name.
    /// Lines ending in `\` carry on.
    fn directive(&mut self, from: usize) {
        self.bump();
        while self.peek(0) == Some(' ') {
            self.bump();
        }
        while self.peek(0).is_some_and(is_ident) {
            self.bump();
        }
        self.run(from, Kind::Macro);
        let name: String = self.chars[from + 1..self.i]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        while self.peek(0) == Some(' ') {
            self.bump();
        }
        if (name == "include" || name == "import")
            && let Some(open @ ('<' | '"')) = self.peek(0)
        {
            let close = if open == '<' { '>' } else { '"' };
            let at = self.i;
            self.bump();
            while self.peek(0).is_some_and(|c| c != close && c != '\n') {
                self.bump();
            }
            if self.peek(0) == Some(close) {
                self.bump();
            }
            self.run(at, Kind::String);
        }
        // The rest of the line is code, lexed as usual; a `\` at its end
        // carries the directive on, which is rare enough to ignore.
    }
}

/// The coloured runs in a file, in order.
pub fn lex(text: &str) -> Vec<Run> {
    let mut lexer = Lexer {
        chars: text.chars().collect(),
        i: 0,
        line: 0,
        line_start: 0,
        runs: Vec::new(),
    };
    lexer.lex();
    lexer.runs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text and kind of each run.
    fn runs(text: &str) -> Vec<(String, Kind)> {
        let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
        lex(text)
            .into_iter()
            .map(|r| (lines[r.line][r.start..r.end].iter().collect(), r.kind))
            .collect()
    }

    #[test]
    fn keywords_calls_numbers_and_strings() {
        let found = runs("int main() { return printf(\"hi\\n\", 0x1F, 1'000); }");
        assert_eq!(
            found,
            [
                ("int".into(), Kind::Keyword),
                ("main".into(), Kind::Function),
                ("return".into(), Kind::Keyword),
                ("printf".into(), Kind::Function),
                ("\"hi".into(), Kind::String),
                ("\\n".into(), Kind::Escape),
                ("\"".into(), Kind::String),
                ("0x1F".into(), Kind::Number),
                ("1'000".into(), Kind::Number),
            ]
        );
    }

    #[test]
    fn comments_span_lines() {
        let found = runs("a /* one\ntwo */ b // three");
        assert_eq!(
            found,
            [
                ("/* one".into(), Kind::Comment),
                ("two */".into(), Kind::Comment),
                ("// three".into(), Kind::Comment),
            ]
        );
    }

    #[test]
    fn preprocessor_lines_and_includes() {
        let found = runs("#include <vector>\n  #define N 4\n");
        assert_eq!(
            found,
            [
                ("#include".into(), Kind::Macro),
                ("<vector>".into(), Kind::String),
                ("#define".into(), Kind::Macro),
                ("4".into(), Kind::Number),
            ]
        );
    }

    #[test]
    fn declared_types() {
        let found = runs("class Shape : public Base {};\nnamespace geo {}");
        assert!(found.contains(&("Shape".into(), Kind::Type)), "{found:?}");
        assert!(found.contains(&("geo".into(), Kind::Type)), "{found:?}");
    }

    #[test]
    fn raw_strings_and_prefixes() {
        let found = runs("auto s = R\"x(a\n)\" still)x\"; auto t = u8\"é\";");
        assert_eq!(found[1], ("R\"x(a".into(), Kind::String));
        assert_eq!(found[2], (")\" still)x\"".into(), Kind::String));
        assert!(
            found.contains(&("u8\"é\"".into(), Kind::String)),
            "{found:?}"
        );
    }

    #[test]
    fn character_literals() {
        let found = runs("char c = '\\'';");
        assert_eq!(
            found,
            [
                ("char".into(), Kind::Keyword),
                ("'".into(), Kind::String),
                ("\\'".into(), Kind::Escape),
                ("'".into(), Kind::String),
            ]
        );
    }
}
