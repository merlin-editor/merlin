//! Problems from a task's output, as VS Code's problem matchers find them:
//! the matchers language plugins bring, such as rustc's or Python's
//! tracebacks, then the `path:line:col: message` most compilers and
//! linters print. Each task's go in with the language servers' problems,
//! under the task, and its next run replaces them.

use std::path::PathBuf;

use regex::Captures;

use super::Editor;
use crate::lang::{Diagnostic, MatcherLine, ProblemMatcher, Severity};

/// What problems from a task are kept under, by its name.
fn key(task: &str) -> String {
    format!("task:{task}")
}

/// A problem found in output: where, how bad, what, and its code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// From 0.
    pub line: u32,
    pub column: u32,
    pub severity: Severity,
    pub message: String,
    pub code: String,
    /// A page about its code, if the matcher knows one.
    pub link: String,
}

/// A path as a tool prints it, if it looks like one: no spaces, a file
/// name with a dot, and not a web address.
fn path_of(s: &str) -> Option<PathBuf> {
    let s = s.trim().trim_start_matches("./");
    let name = s.rsplit('/').next()?;
    let ok = !s.is_empty()
        && !s.contains(char::is_whitespace)
        && !s.contains("://")
        && name.contains('.')
        && !name.starts_with('.')
        && !name.ends_with('.');
    ok.then(|| PathBuf::from(s))
}

fn severity_of(word: &str) -> Option<Severity> {
    match word.trim().to_ascii_lowercase().as_str() {
        "error" | "fatal error" | "fatal" => Some(Severity::Error),
        "warning" | "warn" => Some(Severity::Warning),
        "note" | "info" => Some(Severity::Info),
        _ => None,
    }
}

/// `path:line:col`, or `path:line`, at the start of `s`, and the rest.
fn located(s: &str) -> Option<(PathBuf, u32, u32, &str)> {
    let mut parts = s.splitn(4, ':');
    let path = path_of(parts.next()?)?;
    let line: u32 = parts.next()?.trim().parse().ok()?;
    let rest = parts.next().unwrap_or("");
    let (column, rest) = match rest.trim().parse::<u32>() {
        Ok(c) => (c, parts.next().unwrap_or("")),
        Err(_) => {
            // `path:line: message`: the rest is the message.
            let tail = &s[s.find(&format!(":{line}:"))? + line.to_string().len() + 2..];
            (1, tail)
        }
    };
    Some((path, line.max(1) - 1, column.max(1) - 1, rest))
}

/// What a plugin's matcher has read of a problem so far.
#[derive(Default)]
struct Partial {
    file: Option<String>,
    line: Option<String>,
    column: Option<String>,
    severity: Option<String>,
    code: Option<String>,
    message: Option<String>,
}

impl Partial {
    fn take(&mut self, line: &MatcherLine, caps: &Captures) {
        let group = |i: Option<usize>| {
            i.and_then(|i| caps.get(i))
                .map(|m| m.as_str().trim().to_string())
        };
        let parts = [
            (&mut self.file, line.file),
            (&mut self.line, line.line),
            (&mut self.column, line.column),
            (&mut self.severity, line.severity),
            (&mut self.code, line.code),
            (&mut self.message, line.message),
        ];
        for (part, i) in parts {
            if let Some(text) = group(i) {
                *part = Some(text);
            }
        }
    }

    /// The problem, if it says where.
    fn finish(self, link: Option<&str>) -> Option<Found> {
        let path = path_of(self.file.as_deref()?)?;
        let line: u32 = self.line?.parse().ok()?;
        let column: u32 = self.column.and_then(|c| c.parse().ok()).unwrap_or(1);
        let code = self.code.unwrap_or_default();
        Some(Found {
            path,
            line: line.max(1) - 1,
            column: column.max(1) - 1,
            severity: self
                .severity
                .and_then(|w| severity_of(&w))
                .unwrap_or(Severity::Error),
            message: self.message.unwrap_or_default(),
            link: match link {
                Some(link) if !code.is_empty() => link.replace("{code}", &code),
                _ => String::new(),
            },
            code,
        })
    }
}

/// Reads `line` with a plugin's matcher, `at` how far it is: whether the
/// line was the matcher's, and a problem when it's the last of one.
fn step(
    matcher: &ProblemMatcher,
    line: &str,
    at: &mut Option<(usize, Partial)>,
    found: &mut Vec<Found>,
) -> bool {
    let link = matcher.link.as_deref();
    if let Some((next, partial)) = at
        && let Some(caps) = matcher.lines[*next].regex.captures(line)
    {
        partial.take(&matcher.lines[*next], &caps);
        *next += 1;
        if *next == matcher.lines.len()
            && let Some((_, partial)) = at.take()
        {
            found.extend(partial.finish(link));
        }
        return true;
    }
    // A first line starts again, as a traceback's next frame does.
    if let Some(first) = matcher.lines.first()
        && let Some(caps) = first.regex.captures(line)
    {
        let mut partial = Partial::default();
        partial.take(first, &caps);
        *at = None;
        match matcher.lines.len() {
            1 => found.extend(partial.finish(link)),
            _ => *at = Some((1, partial)),
        }
        return true;
    }
    if at.is_some() && matcher.between.as_ref().is_some_and(|b| b.is_match(line)) {
        return true;
    }
    *at = None;
    false
}

/// Every problem in a task's output, in the order printed: the plugins'
/// matchers' first, and the line most tools print for the rest.
pub fn problems(text: &str, matchers: &[&ProblemMatcher]) -> Vec<Found> {
    let mut found = Vec::new();
    let mut progress: Vec<Option<(usize, Partial)>> = matchers.iter().map(|_| None).collect();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim_end();
        let mut theirs = false;
        for (matcher, at) in matchers.iter().zip(&mut progress) {
            theirs |= step(matcher, line, at, &mut found);
        }
        if theirs {
            continue;
        }
        // Most tools: `path:line:col: error: message`, or with no word,
        // an error, as linters often print.
        if let Some((path, l, c, rest)) = located(line.trim_start()) {
            let rest = rest.trim();
            if rest.is_empty() {
                continue;
            }
            let (severity, message) = match rest.split_once(':') {
                Some((word, message)) => match severity_of(word) {
                    Some(s) => (s, message.trim()),
                    None => (Severity::Error, rest),
                },
                None => (Severity::Error, rest),
            };
            // A note under another problem says more about it.
            if severity == Severity::Info && i > 0 && !found.is_empty() {
                continue;
            }
            found.push(Found {
                path,
                line: l,
                column: c,
                severity,
                message: message.to_string(),
                code: String::new(),
                link: String::new(),
            });
        }
    }
    found
}

/// Where a problem that points at one character ends: the end of the
/// word it starts, as tools point at words. In UTF-16 columns.
pub(crate) fn word_end(line: &str, column: u32) -> u32 {
    let mut col = 0;
    let mut chars = line.chars().peekable();
    while col < column {
        match chars.next() {
            Some(c) => col += c.len_utf16() as u32,
            None => return column + 1,
        }
    }
    let mut end = col;
    for c in chars {
        if !(c.is_alphanumeric() || c == '_') {
            break;
        }
        end += c.len_utf16() as u32;
    }
    end.max(column + 1)
}

impl Editor {
    /// A task finished: what its output says is wrong replaces what its
    /// last run said. A problem a language server already shows isn't
    /// listed twice.
    pub(crate) fn task_problems(&mut self, task: &str, text: &str) {
        let key = key(task);
        let mut paths: Vec<PathBuf> = self
            .diagnostics
            .keys()
            .filter(|(k, _)| *k == key)
            .map(|(_, p)| p.clone())
            .collect();
        self.diagnostics.retain(|(k, _), _| *k != key);
        let mut by_path: Vec<(PathBuf, Vec<Diagnostic>)> = Vec::new();
        let matchers: Vec<&ProblemMatcher> =
            self.problem_matchers.iter().map(|(_, m)| m).collect();
        for f in problems(text, &matchers) {
            let first = f.message.lines().next().unwrap_or_default();
            let shown_already = self.diagnostics.iter().any(|((k, p), list)| {
                !k.starts_with("task:")
                    && *p == f.path
                    && list
                        .iter()
                        .any(|d| d.start.0 == f.line && d.message.starts_with(first))
            });
            if shown_already {
                continue;
            }
            let d = Diagnostic {
                start: (f.line, f.column),
                end: (f.line, f.column + 1),
                severity: f.severity,
                message: f.message,
                source: task.to_string(),
                code: f.code,
                link: f.link,
            };
            match by_path.iter_mut().find(|(p, _)| *p == f.path) {
                Some((_, list)) => list.push(d),
                None => by_path.push((f.path, vec![d])),
            }
        }
        for (path, list) in by_path {
            paths.push(path.clone());
            self.diagnostics.insert((key.clone(), path), list);
        }
        self.diagnostics_version += 1;
        paths.sort();
        paths.dedup();
        for path in paths {
            self.place_marks(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(regex: &str, groups: [Option<usize>; 6]) -> MatcherLine {
        let [file, line, column, severity, code, message] = groups;
        MatcherLine {
            regex: regex::Regex::new(regex).unwrap(),
            file,
            line,
            column,
            severity,
            code,
            message,
        }
    }

    /// rustc's, as the Rust plugin has it.
    fn rustc() -> ProblemMatcher {
        ProblemMatcher {
            lines: vec![
                line(
                    r"^(error|warning)(?:\[(E\d{4})\])?: (.+)$",
                    [None, None, None, Some(1), Some(2), Some(3)],
                ),
                line(
                    r"^\s*--> (.+):(\d+):(\d+)$",
                    [Some(1), Some(2), Some(3), None, None, None],
                ),
            ],
            between: None,
            link: Some("https://doc.rust-lang.org/error_codes/{code}.html".into()),
        }
    }

    #[test]
    fn a_plugins_matcher_reads_its_lines_in_turn() {
        let out = "   Compiling app v0.1.0\n\
error[E0308]: mismatched types\n \
 --> src/main.rs:5:23\n  \
|\n\
5 |     let count: u32 = steps.len();\n\
warning: unused variable: `x`\n \
 --> src/lib.rs:3:9\n\
error: could not compile `app` (bin \"app\") due to 1 previous error\n";
        let found = problems(out, &[&rustc()]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].path, PathBuf::from("src/main.rs"));
        assert_eq!((found[0].line, found[0].column), (4, 22));
        assert_eq!(found[0].code, "E0308");
        assert_eq!(found[0].message, "mismatched types");
        assert_eq!(
            found[0].link,
            "https://doc.rust-lang.org/error_codes/E0308.html"
        );
        assert_eq!(found[1].severity, Severity::Warning);
        assert_eq!(found[1].link, "", "no code, no link");
        assert!(problems(out, &[]).is_empty(), "without the plugin");
    }

    #[test]
    fn what_comes_between_lines_waits_and_a_first_line_starts_again() {
        let traceback = ProblemMatcher {
            lines: vec![
                line(
                    r#"^\s*File "([^/"][^"]*)", line (\d+)"#,
                    [Some(1), Some(2), None, None, None, None],
                ),
                line(
                    r"^[A-Za-z_][\w.]*(?:: .*)?$",
                    [None, None, None, None, None, Some(0)],
                ),
            ],
            between: Some(regex::Regex::new(r"^\s").unwrap()),
            link: None,
        };
        let out = "Traceback (most recent call last):\n  \
File \"app/main.py\", line 9, in <module>\n    main()\n  \
File \"app/main.py\", line 4, in main\n    print(nme)\n  \
File \"/usr/lib/python3/x.py\", line 1, in f\n\
NameError: name 'nme' is not defined\n";
        let found = problems(out, &[&traceback]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, PathBuf::from("app/main.py"));
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].message, "NameError: name 'nme' is not defined");
    }

    #[test]
    fn compilers_and_linters_print_path_line_col() {
        let out = "main.c:12:5: error: expected ';' before 'return'\n\
main.c:3:1: warning: unused function 'f'\n\
pkg/x.go:7:2: undefined: foo\n\
See https://example.com:443/x for more\n";
        let found = problems(out, &[]);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[0].message, "expected ';' before 'return'");
        assert_eq!(found[1].severity, Severity::Warning);
        assert_eq!(found[2].path, PathBuf::from("pkg/x.go"));
        assert_eq!(found[2].message, "undefined: foo");
    }

    #[test]
    fn a_point_widens_to_its_word() {
        assert_eq!(word_end("    accent + step", 4), 10);
        assert_eq!(word_end("    accent + step", 11), 12);
        assert_eq!(word_end("short", 40), 41);
    }

    #[test]
    fn plain_output_finds_nothing() {
        assert!(
            problems(
                "   Finished `dev` profile in 0.1s\nhello: world\n12:30 done\n",
                &[&rustc()]
            )
            .is_empty()
        );
    }
}
