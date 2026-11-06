//! Problems from a task's output, as VS Code's problem matchers find them:
//! rustc's and cargo's, the `path:line:col: message` most compilers and
//! linters print, TypeScript's `path(line,col)`, and Python's tracebacks.
//! Each task's go in with the language servers' problems, under the task,
//! and its next run replaces them.

use std::path::PathBuf;

use super::Editor;
use crate::lang::{Diagnostic, Severity};

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

/// rustc's header: `error[E0308]: mismatched types`.
fn rust_header(line: &str) -> Option<(Severity, String, String)> {
    let (head, message) = line.split_once(": ")?;
    let (word, code) = match head.split_once('[') {
        Some((w, c)) => (w, c.strip_suffix(']')?.to_string()),
        None => (head, String::new()),
    };
    let severity = match word {
        "error" => Severity::Error,
        "warning" => Severity::Warning,
        _ => return None,
    };
    Some((severity, code, message.trim().to_string()))
}

/// Every problem in a task's output, in the order printed.
pub fn problems(text: &str) -> Vec<Found> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    let mut header: Option<(Severity, String, String)> = None;
    // A Python traceback's last frame in the project, waiting for its
    // exception's line.
    let mut frame: Option<(PathBuf, u32)> = None;
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        // rustc: the header, then `  --> path:line:col` under it.
        if let Some(h) = rust_header(line) {
            header = Some(h);
            continue;
        }
        if let Some(at) = line.trim_start().strip_prefix("--> ") {
            if let (Some((severity, code, message)), Some((path, l, c, _))) =
                (header.take(), located(at))
            {
                found.push(Found {
                    path,
                    line: l,
                    column: c,
                    severity,
                    message,
                    code,
                });
            }
            continue;
        }
        // Python: `  File "app/x.py", line 3, in main`.
        if let Some(rest) = line.trim_start().strip_prefix("File \"")
            && let Some((path, rest)) = rest.split_once("\", line ")
        {
            let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let (Some(path), Ok(n)) = (path_of(path), n.parse::<u32>()) {
                // The project's own files, not the libraries'.
                if path.is_relative() {
                    frame = Some((path, n.max(1) - 1));
                }
            }
            continue;
        }
        if let Some((path, l)) = &frame
            && !line.starts_with(' ')
            && !line.is_empty()
            && !line.starts_with("Traceback")
        {
            found.push(Found {
                path: path.clone(),
                line: *l,
                column: 0,
                severity: Severity::Error,
                message: line.to_string(),
                code: String::new(),
            });
            frame = None;
            continue;
        }
        // TypeScript: `src/a.ts(3,5): error TS2322: message`.
        if let Some((path, rest)) = line.split_once('(')
            && let Some((at, rest)) = rest.split_once("): ")
            && let Some((l, c)) = at.split_once(',')
            && let (Some(path), Ok(l), Ok(c)) = (path_of(path), l.parse::<u32>(), c.parse::<u32>())
        {
            let (word, message) = rest.split_once(": ").unwrap_or(("error", rest));
            let (word, code) = word.split_once(' ').unwrap_or((word, ""));
            if let Some(severity) = severity_of(word) {
                found.push(Found {
                    path,
                    line: l.max(1) - 1,
                    column: c.max(1) - 1,
                    severity,
                    message: message.trim().to_string(),
                    code: code.to_string(),
                });
            }
            continue;
        }
        // Most others: `path:line:col: error: message`, or with no word,
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
        for f in problems(text) {
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
            let link = if f.code.starts_with('E') && f.code.len() == 5 {
                format!("https://doc.rust-lang.org/error_codes/{}.html", f.code)
            } else {
                String::new()
            };
            let d = Diagnostic {
                start: (f.line, f.column),
                end: (f.line, f.column + 1),
                severity: f.severity,
                message: f.message,
                source: task.to_string(),
                code: f.code,
                link,
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

    #[test]
    fn rustc_says_where_under_its_header() {
        let out = "   Compiling app v0.1.0\n\
error[E0308]: mismatched types\n \
 --> src/main.rs:5:23\n  \
|\n\
5 |     let count: u32 = steps.len();\n\
warning: unused variable: `x`\n \
 --> src/lib.rs:3:9\n\
error: could not compile `app` (bin \"app\") due to 1 previous error\n";
        let found = problems(out);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].path, PathBuf::from("src/main.rs"));
        assert_eq!((found[0].line, found[0].column), (4, 22));
        assert_eq!(found[0].code, "E0308");
        assert_eq!(found[0].message, "mismatched types");
        assert_eq!(found[1].severity, Severity::Warning);
    }

    #[test]
    fn compilers_and_linters_print_path_line_col() {
        let out = "main.c:12:5: error: expected ';' before 'return'\n\
main.c:3:1: warning: unused function 'f'\n\
pkg/x.go:7:2: undefined: foo\n\
See https://example.com:443/x for more\n";
        let found = problems(out);
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].severity, Severity::Error);
        assert_eq!(found[0].message, "expected ';' before 'return'");
        assert_eq!(found[1].severity, Severity::Warning);
        assert_eq!(found[2].path, PathBuf::from("pkg/x.go"));
        assert_eq!(found[2].message, "undefined: foo");
    }

    #[test]
    fn typescript_puts_the_place_in_brackets() {
        let found = problems("src/a.ts(3,5): error TS2322: Type 'string' is not 'number'.\n");
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].line, found[0].column), (2, 4));
        assert_eq!(found[0].code, "TS2322");
    }

    #[test]
    fn a_python_traceback_points_at_the_projects_last_frame() {
        let out = "Traceback (most recent call last):\n  \
File \"app/main.py\", line 9, in <module>\n    main()\n  \
File \"app/main.py\", line 4, in main\n    print(nme)\n  \
File \"/usr/lib/python3/x.py\", line 1, in f\n\
NameError: name 'nme' is not defined\n";
        let found = problems(out);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, PathBuf::from("app/main.py"));
        assert_eq!(found[0].line, 3);
        assert_eq!(found[0].message, "NameError: name 'nme' is not defined");
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
            problems("   Finished `dev` profile in 0.1s\nhello: world\n12:30 done\n").is_empty()
        );
    }
}
