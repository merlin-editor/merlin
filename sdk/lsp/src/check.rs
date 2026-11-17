//! Problems' Check Project: every file of the project opened in the
//! running server, a few at a time, so its problems (warnings and
//! clang-tidy's findings too, which `clangd --check` leaves out) come as
//! for an open file, then closed again with the problems kept.
//!
//! clangd's files are the ones `compile_commands.json` lists, so each is
//! checked with the flags it's built with.

use std::collections::{BTreeSet, VecDeque};

use merlin_plugin::{LogLevel, host};
use serde_json::{Value, json};

use crate::Lsp;
use crate::protocol::uri;

/// How many files are open for the check at once.
const AT_ONCE: usize = 3;

#[derive(Default)]
pub(crate) struct ProjectCheck {
    program: String,
    /// Full paths still to open, with their language.
    waiting: VecDeque<(String, String)>,
    /// Open for the check, waiting for their problems.
    open: BTreeSet<String>,
    total: usize,
    done: usize,
}

/// What clangd's log says of a file's flags: the file, and whether
/// they're guessed. `ASTWorker building file /w/a.cpp version 1 with
/// command clangd fallback` is guessed; with a real command, not.
pub fn flags_line(line: &str) -> Option<(&str, bool)> {
    if let Some(path) = line
        .split("Failed to find compilation database for ")
        .nth(1)
    {
        return Some((path.trim(), true));
    }
    let rest = line.split("ASTWorker building file ").nth(1)?;
    let (path, after) = rest.split_once(" version ")?;
    Some((path.trim(), after.contains("with command clangd fallback")))
}

/// `/w/build/../a.cpp` as `/w/a.cpp`, as servers name it.
fn normal(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The files a `compile_commands.json` lists, each once, with full paths.
pub(crate) fn listed_files(json: &str) -> Vec<String> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut files: Vec<String> = entries
        .iter()
        .filter_map(|e| {
            let file = e["file"].as_str()?;
            let full = if file.starts_with('/') {
                file.to_string()
            } else {
                format!("{}/{file}", e["directory"].as_str()?.trim_end_matches('/'))
            };
            Some(normal(&full))
        })
        .collect();
    files.sort();
    files.dedup();
    files
}

impl Lsp {
    /// Whether it offers Check Project: for clangd, which reports only on
    /// open files.
    pub(crate) fn checks_projects(&self) -> bool {
        self.specs.iter().any(|s| s.program == "clangd")
    }

    /// Where `compile_commands.json` is: the folder clangd was given, else
    /// the top of the workspace or its `build`.
    pub(crate) fn compile_commands(&self) -> Option<String> {
        let root = host::workspace_root();
        let root = root.trim_end_matches('/');
        let mut options = json!({});
        if let Some(adjust) = self
            .specs
            .iter()
            .find(|s| s.program == "clangd")
            .and_then(|s| s.adjust)
        {
            adjust(&mut options);
        }
        let given = options["compilationDatabasePath"]
            .as_str()
            .map(String::from);
        given
            .into_iter()
            .chain([root.to_string(), format!("{root}/build")])
            .find_map(|dir| {
                let path = format!("{}/compile_commands.json", dir.trim_end_matches('/'));
                let local = path.strip_prefix(&format!("{root}/")).unwrap_or(&path);
                host::read_file(local).ok()
            })
    }

    /// Check Project: every file the compile commands list, opened in
    /// clangd a few at a time.
    pub(crate) fn check_project(&mut self) {
        if self.check.is_some() {
            return;
        }
        let Some(spec) = self.specs.iter().find(|s| s.program == "clangd") else {
            return;
        };
        let Some(json) = self.compile_commands() else {
            return host::notice(
                LogLevel::Info,
                "There's no compile_commands.json to check the project from: configure its build first",
                &[],
            );
        };
        // Not what the project excludes, as vendored code.
        let waiting: VecDeque<(String, String)> = listed_files(&json)
            .into_iter()
            .filter(|path| !host::is_excluded(path, false))
            .filter_map(|path| {
                let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
                let language = spec.files.iter().find(|(e, _)| *e == ext)?.1;
                Some((path, language.to_string()))
            })
            .collect();
        self.ensure(spec);
        self.checked_once = true;
        host::start_progress("check-project", "Checking the project", true);
        self.check = Some(ProjectCheck {
            program: spec.program.to_string(),
            total: waiting.len(),
            waiting,
            ..ProjectCheck::default()
        });
        self.check_next();
    }

    /// Opens files until enough are open, or says it's done.
    fn check_next(&mut self) {
        let Some(check) = &mut self.check else {
            return;
        };
        let program = check.program.clone();
        let root = host::workspace_root();
        let root = root.trim_end_matches('/').to_string();
        let mut opening = Vec::new();
        while check.open.len() + opening.len() < AT_ONCE {
            let Some((path, language)) = check.waiting.pop_front() else {
                break;
            };
            opening.push((path, language));
        }
        for (path, language) in opening {
            let Some(server) = self.servers.iter_mut().find(|s| s.program == program) else {
                break;
            };
            let check = self.check.as_mut().expect("checking");
            // An open file's problems are live already.
            if server.opened.contains_key(&path) {
                check.done += 1;
                continue;
            }
            let local = path.strip_prefix(&format!("{root}/")).unwrap_or(&path);
            let Ok(text) = host::read_file(local) else {
                check.done += 1;
                continue;
            };
            server.notify(
                "textDocument/didOpen",
                json!({"textDocument": {
                    "uri": uri(&path), "languageId": language, "version": 1, "text": text,
                }}),
            );
            check.open.insert(path);
        }
        let Some(check) = &self.check else {
            return;
        };
        let fraction = check.done as f32 / check.total.max(1) as f32;
        host::report_progress(
            "check-project",
            &format!("{} of {} files", check.done, check.total),
            Some(fraction),
        );
        if check.open.is_empty() && check.waiting.is_empty() {
            self.check = None;
            host::end_progress("check-project");
        }
    }

    /// Whether to drop a publish: the empty one a server sends as a file
    /// the check opened closes, which would clear the problems it found.
    pub(crate) fn closing_publish(&mut self, path: &str, empty: bool) -> bool {
        self.closing.remove(path) && empty
    }

    /// A file's problems came: one the check opened is closed again, and
    /// the next opened.
    pub(crate) fn checked(&mut self, program: &str, path: &str) {
        let Some(check) = &mut self.check else {
            return;
        };
        if check.program != program || !check.open.remove(path) {
            return;
        }
        check.done += 1;
        if let Some(server) = self.server(program) {
            server.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": uri(path)}}),
            );
        }
        self.closing.insert(path.to_string());
        self.check_next();
    }

    /// Stop: the files still open closed, the rest left.
    pub(crate) fn stop_check(&mut self) {
        let Some(check) = self.check.take() else {
            return;
        };
        if let Some(server) = self.server(&check.program) {
            for path in &check.open {
                server.notify(
                    "textDocument/didClose",
                    json!({"textDocument": {"uri": uri(path)}}),
                );
            }
        }
        self.closing.extend(check.open);
        host::end_progress("check-project");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clangds_log_and_the_listed_files() {
        assert_eq!(
            flags_line(
                "I[12:00:00.000] ASTWorker building file /w/a.cpp version 1 with command clangd fallback"
            ),
            Some(("/w/a.cpp", true))
        );
        assert_eq!(
            flags_line("I[12:00:00.000] ASTWorker building file /w/b.cpp version 3 with command "),
            Some(("/w/b.cpp", false))
        );
        assert_eq!(
            flags_line("I[12:00:00.000] Failed to find compilation database for /w/c.cpp"),
            Some(("/w/c.cpp", true))
        );
        assert_eq!(
            flags_line("I[12:00:00.000] Loading compilation database..."),
            None
        );
        let json = r#"[{"directory": "/w/build", "file": "../a.cpp"},
                       {"directory": "/w", "file": "/w/b.cpp"},
                       {"directory": "/w", "file": "/w/b.cpp"}]"#;
        assert_eq!(listed_files(json), ["/w/a.cpp", "/w/b.cpp"]);
    }
}
