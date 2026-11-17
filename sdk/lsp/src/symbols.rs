//! The palette's search of the project's symbols: the servers' own, from
//! any file, a note while they index, and a quick scan of the sources
//! when the server can't search them all, as clangd without compile
//! commands.

use merlin_plugin::{Button, ProjectSymbol, SymbolsNote};
use merlin_plugin::{Position, host};
use serde_json::json;

use crate::server::{Request, Status};
use crate::{Lsp, Spec};

/// How many files a scan reads at most.
const SCAN_FILES: usize = 5_000;
/// How many symbols one answer gives at most.
const SHOWN: usize = 200;

/// `412/1030` as `412 of 1,030 files`.
fn files_done(message: &str) -> Option<String> {
    let (done, total) = message.trim().split_once('/')?;
    let (done, total): (u64, u64) = (done.trim().parse().ok()?, total.trim().parse().ok()?);
    Some(format!("{} of {} files", grouped(done), grouped(total)))
}

/// A note that says nothing.
fn no_note() -> SymbolsNote {
    SymbolsNote {
        indexing: String::new(),
        warning: String::new(),
        action: None,
    }
}

/// `1030` as `1,030`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Whether `name` matches the query: its letters in order, ignoring case.
fn matches(query: &str, name: &str) -> bool {
    let mut wanted = query.chars().flat_map(char::to_lowercase).peekable();
    for c in name.chars().flat_map(char::to_lowercase) {
        if wanted.peek() == Some(&c) {
            wanted.next();
        }
    }
    wanted.peek().is_none()
}

impl Lsp {
    /// The palette asked: each server that's running is asked, and with
    /// none, the current file's or else the first is started for it.
    /// Where a scan stands in, its symbols go at once.
    pub(crate) fn project_symbols(&mut self) {
        let query = host::symbol_query();
        if query.is_empty() {
            return;
        }
        if self.servers.iter().all(|s| s.process.is_none()) {
            let spec = self
                .active()
                .map(|(_, spec, _)| spec)
                .or_else(|| self.specs.first());
            if let Some(spec) = spec {
                self.ensure(spec);
            }
        }
        let mut asked = false;
        for server in &mut self.servers {
            // One still starting is asked too: its answer comes once it's up.
            let can = match server.status {
                Status::Starting => true,
                Status::Ready => server.workspace_symbols,
                Status::Failed(_) => false,
            };
            if server.process.is_some() && can {
                server.request(
                    "workspace/symbol",
                    json!({"query": query}),
                    Request::ProjectSymbols {
                        query: query.clone(),
                    },
                );
                asked = true;
            }
        }
        // The scan's now; the server's join them when they come.
        let scanning = self.specs.iter().find(|s| self.scans(s));
        if let Some(spec) = scanning {
            let found = self.scanned(spec, &query);
            host::show_project_symbols(&query, &found, &self.symbols_note(spec.program));
        } else if !asked {
            // Nothing to ask: the palette hears there's nothing.
            host::show_project_symbols(&query, &[], &no_note());
        }
    }

    /// A server's answer, with the scan's for files it said nothing of.
    pub(crate) fn answer_symbols(
        &mut self,
        program: &str,
        query: &str,
        mut symbols: Vec<ProjectSymbol>,
    ) {
        if let Some(spec) = self
            .specs
            .iter()
            .find(|s| s.program == program && self.scans(s))
        {
            let known: Vec<String> = symbols.iter().map(|s| s.path.clone()).collect();
            let opened: Vec<String> = self
                .server(program)
                .map(|s| s.opened.keys().cloned().collect())
                .unwrap_or_default();
            let full = |p: &str| -> String {
                if p.starts_with('/') {
                    p.to_string()
                } else {
                    format!("{}/{p}", host::workspace_root().trim_end_matches('/'))
                }
            };
            symbols.extend(self.scanned(spec, query).into_iter().filter(|s| {
                let path = full(&s.path);
                !known.iter().any(|k| full(k) == path) && !opened.contains(&path)
            }));
        }
        symbols.truncate(SHOWN);
        host::show_project_symbols(query, &symbols, &self.symbols_note(program));
    }

    /// Whether the server's search needs the scan: it has one, and the
    /// server can't see the whole project.
    fn scans(&self, spec: &Spec) -> bool {
        spec.scan.is_some() && spec.program == "clangd" && self.compile_commands().is_none()
    }

    /// The project's symbols the scan finds matching `query`, from the
    /// files the server takes, kept from the last time when unchanged.
    fn scanned(&mut self, spec: &Spec, query: &str) -> Vec<ProjectSymbol> {
        let Some(scan) = spec.scan else {
            return Vec::new();
        };
        let Ok(files) = host::list_files() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for path in files
            .into_iter()
            .filter(|p| {
                let name = p.rsplit('/').next().unwrap_or(p);
                let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
                spec.files
                    .iter()
                    .any(|(e, _)| Some(*e) == ext.as_deref() || *e == name)
            })
            .take(SCAN_FILES)
        {
            let Ok(text) = host::read_file(&path) else {
                continue;
            };
            for s in scan(&text) {
                if !matches(query, &s.name) {
                    continue;
                }
                found.push(ProjectSymbol {
                    name: s.name,
                    kind: s.kind.into(),
                    container: s.container,
                    path: path.clone(),
                    start: Position {
                        line: s.line,
                        column: s.column,
                    },
                    scan: true,
                });
                if found.len() >= SHOWN {
                    return found;
                }
            }
        }
        found
    }

    /// What the palette says under the symbols: the server still indexing,
    /// or that it can't see the whole project and a scan stands in.
    fn symbols_note(&self, program: &str) -> SymbolsNote {
        let mut note = no_note();
        if let Some(server) = self.servers.iter().find(|s| s.program == program) {
            let indexing = server
                .progress
                .iter()
                .find(|(_, what)| what.to_lowercase().starts_with("index"));
            if let Some((token, what)) = indexing {
                let how_far = server
                    .progress_message
                    .get(token)
                    .and_then(|m| files_done(m))
                    .or_else(|| what.split_once(": ").map(|(_, done)| done.to_string()));
                note.indexing = match how_far {
                    Some(far) => format!("{program} is still indexing: {far}"),
                    None => format!("{program} is still indexing"),
                };
            }
        }
        if let Some(spec) = self.specs.iter().find(|s| s.program == program)
            && self.scans(spec)
        {
            note.warning = format!(
                "No compile_commands.json: {program} only knows open files. SCAN rows come from a quick scan."
            );
            // A build plugin's setup is what makes the compile commands.
            note.action = merlin_plugin::shared::all::<merlin_plugin::shared::BuildSetup>()
                .into_iter()
                .next()
                .map(|(_, s)| Button {
                    id: format!("run:{}", s.command),
                    label: s.label,
                });
        }
        note
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_matching() {
        assert_eq!(
            files_done("412/1030").as_deref(),
            Some("412 of 1,030 files")
        );
        assert_eq!(files_done("indexing"), None);
        assert_eq!(grouped(1234567), "1,234,567");
        assert!(matches("area", "total_area"));
        assert!(matches("ca", "Circle::area"));
        assert!(!matches("zz", "area"));
    }
}
