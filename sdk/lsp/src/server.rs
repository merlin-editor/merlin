//! Per-server process state, JSON-RPC framing, and request cancellation.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use merlin_plugin::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::protocol::uri;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Status {
    Starting,
    Ready,
    Failed(String),
}

/// What a request's response is for.
#[derive(Serialize, Deserialize)]
pub(crate) enum Request {
    Initialize,
    /// Whether the user asked, so an empty answer is worth saying: not
    /// when the mouse rests on a name.
    Hover {
        manual: bool,
    },
    Definition,
    /// Where the name under Ctrl is defined, for the editor to show as a
    /// link: every place, without going there.
    DefinitionLink,
    CodeActions,
    /// The whole file or the selection formatted: its edits, for `path`.
    Format {
        path: String,
    },
    /// One of the changed ranges a save formats, in batch `batch`.
    FormatPart {
        batch: u64,
    },
    /// Whether the user asked, so an empty answer is worth saying.
    Completion {
        manual: bool,
    },
    /// Diagnostics for a document at a version.
    Diagnostics {
        path: String,
        version: i32,
    },
    /// Semantic colours, symbols or folds for a document at a version.
    Tokens {
        path: String,
        version: i32,
    },
    Symbols {
        path: String,
        version: i32,
    },
    Folds {
        path: String,
        version: i32,
    },
    InlayHints {
        path: String,
        version: i32,
    },
    /// rust-analyzer's tests and binaries in a document at a version.
    Runnables {
        path: String,
        version: i32,
    },
    Signature,
    /// Whether the name at the cursor can be renamed, and where it is.
    PrepareRename,
    /// The places that use the name at the cursor, for the rename box to
    /// count; with the name's place, if preparing found it.
    References {
        name: Option<((u32, u32), (u32, u32))>,
    },
    /// Where the name at the cursor is used, for the peek view.
    Peek,
    /// The project's symbols matching what the palette searches for.
    ProjectSymbols {
        query: String,
    },
    /// A rename's edits: made, or sent to review.
    Rename {
        review: bool,
    },
}

impl Request {
    /// Whether this request makes an earlier one still in `waiting`
    /// obsolete, so the server can cancel it.
    pub(crate) fn supersedes(&self, earlier: &Self) -> bool {
        use Request::*;
        match (self, earlier) {
            (Diagnostics { path: a, .. }, Diagnostics { path: b, .. })
            | (Tokens { path: a, .. }, Tokens { path: b, .. })
            | (Symbols { path: a, .. }, Symbols { path: b, .. })
            | (Folds { path: a, .. }, Folds { path: b, .. })
            | (InlayHints { path: a, .. }, InlayHints { path: b, .. })
            | (Runnables { path: a, .. }, Runnables { path: b, .. }) => a == b,
            (Completion { .. }, Completion { .. })
            | (Signature, Signature)
            | (DefinitionLink, DefinitionLink)
            | (ProjectSymbols { .. }, ProjectSymbols { .. }) => true,
            _ => false,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Server {
    pub program: String,
    pub process: Option<u32>,
    pub status: Status,
    pub next_id: u64,
    pub waiting: HashMap<u64, Request>,
    /// Open documents by absolute path, with their version.
    pub opened: HashMap<String, i32>,
    /// Messages held until the server is initialized.
    pub queue: Vec<Value>,
    /// Work in progress by token, such as indexing.
    pub progress: BTreeMap<String, String>,
    /// Whether the server gives diagnostics when asked (LSP 3.17 pull
    /// diagnostics). rust-analyzer gives its own only this way, and pushes
    /// just `cargo check`'s.
    #[serde(default)]
    pub pull: bool,
    /// Diagnostics by path, as pushed and as pulled.
    #[serde(default)]
    pub pushed: HashMap<String, Vec<Value>>,
    #[serde(default)]
    pub pulled: HashMap<String, Vec<Value>>,
    /// The newest completion request. Typing `a.b.` asks twice, and the
    /// answer for `a.` mustn't replace the one for `a.b.`.
    #[serde(default)]
    pub latest_completion: u64,
    /// The names of the server's semantic token types, by index; empty if
    /// it has none.
    #[serde(default)]
    pub legend: Vec<String>,
    /// Whether it gives symbols and folding ranges.
    #[serde(default)]
    pub symbols: bool,
    /// Whether it searches the project's symbols.
    #[serde(default)]
    pub workspace_symbols: bool,
    #[serde(default)]
    pub folds: bool,
    #[serde(default)]
    pub inlay_hints: bool,
    /// Whether it formats files, and parts of them.
    #[serde(default)]
    pub formats: bool,
    #[serde(default)]
    pub formats_ranges: bool,
    /// Whether it renames, and says first whether a name can be.
    #[serde(default)]
    pub renames: bool,
    #[serde(default)]
    pub prepares_renames: bool,
    /// The last line of each open document, for asking about all of it.
    #[serde(default)]
    pub last_lines: HashMap<String, u32>,
    /// Whether it takes changes as ranges, rather than the whole text each
    /// time.
    #[serde(default)]
    pub incremental: bool,
    /// The editor's version of each open document the server has, to send
    /// only what changed since.
    #[serde(default)]
    pub synced: HashMap<String, u64>,
    /// rust-analyzer's binaries and benches to run, as tasks, by name.
    #[serde(default)]
    pub tasks: BTreeMap<String, (String, String)>,
    /// Server settings for `workspace/configuration` requests.
    #[serde(default)]
    pub settings: Value,
    /// Folders added to the workspace root, for files outside what the
    /// server loaded from it.
    #[serde(default)]
    pub folders: BTreeSet<String>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Problem {
    pub line: u32,
    pub column: u32,
    pub severity: u8,
    pub message: String,
}

impl Server {
    pub(crate) fn write(&mut self, message: Value) {
        let setup = matches!(
            message["method"].as_str(),
            Some("initialize" | "initialized")
        );
        if self.status == Status::Starting && !setup {
            self.queue.push(message);
            return;
        }
        if let Some(process) = self.process
            && let Err(err) = host::send(process, &message.to_string())
        {
            self.status = Status::Failed(err);
        }
    }

    pub(crate) fn request(&mut self, method: &str, params: Value, request: Request) {
        let stale: Vec<u64> = self
            .waiting
            .iter()
            .filter(|(_, r)| request.supersedes(r))
            .map(|(&id, _)| id)
            .collect();
        for id in stale {
            self.waiting.remove(&id);
            if self.status == Status::Ready {
                self.notify("$/cancelRequest", json!({"id": id}));
            } else {
                self.queue
                    .retain(|m| m.get("id").and_then(Value::as_u64) != Some(id));
            }
        }
        self.next_id += 1;
        self.waiting.insert(self.next_id, request);
        let id = self.next_id;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    pub(crate) fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    /// Adds the project around `path` to the workspace, when the server
    /// says the file belongs to nothing it loaded: a crate of its own
    /// beside the workspace's, say. Plugins can't look for its manifest,
    /// so the project is the folder above its `src`, or the file's own.
    pub(crate) fn add_folder_for(&mut self, path: &str) {
        let folder = match path.rfind("/src/") {
            Some(i) => &path[..i],
            None => path.rsplit_once('/').map_or(path, |(dir, _)| dir),
        };
        if folder.is_empty() || !self.folders.insert(folder.to_string()) {
            return;
        }
        let name = folder.rsplit('/').next().unwrap_or(folder);
        self.notify(
            "workspace/didChangeWorkspaceFolders",
            json!({"event": {"added": [{"uri": uri(folder), "name": name}], "removed": []}}),
        );
    }

    pub(crate) fn respond(&mut self, id: Value, result: Value) {
        if let Some(process) = self.process {
            let message = json!({"jsonrpc": "2.0", "id": id, "result": result});
            let _ = host::send(process, &message.to_string());
        }
    }

    /// Asks for a document's diagnostics, if the server gives them that way.
    pub(crate) fn pull(&mut self, path: &str) {
        let Some(&version) = self.opened.get(path).filter(|_| self.pull) else {
            return;
        };
        self.request(
            "textDocument/diagnostic",
            json!({"textDocument": {"uri": uri(path)}}),
            Request::Diagnostics {
                path: path.into(),
                version,
            },
        );
    }

    /// Tells the server what changed in a file it has open, as ranges.
    pub(crate) fn send_changes(&mut self, path: &str, changes: &[TextChange]) {
        let Some(version) = self.opened.get_mut(path) else {
            return;
        };
        *version += 1;
        let version = *version;
        if let Some(last) = self.last_lines.get_mut(path) {
            for c in changes {
                let added = c.text.matches('\n').count() as u32;
                *last = (*last + added).saturating_sub(c.end.line - c.start.line);
            }
        }
        let at = |p: &Position| json!({"line": p.line, "character": p.column});
        let changes: Vec<Value> = changes
            .iter()
            .map(|c| json!({"range": {"start": at(&c.start), "end": at(&c.end)}, "text": c.text}))
            .collect();
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri(path), "version": version},
                "contentChanges": changes,
            }),
        );
        self.pull(path);
        self.describe(path);
    }

    /// Tells the server about the file's current text.
    pub(crate) fn sync(&mut self, path: &str, language: &str, text: &str) {
        let uri = uri(path);
        self.last_lines
            .insert(path.to_string(), text.matches('\n').count() as u32);
        match self.opened.get_mut(path) {
            Some(version) => {
                *version += 1;
                let version = *version;
                self.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": uri, "version": version},
                        "contentChanges": [{"text": text}],
                    }),
                );
            }
            None => {
                self.opened.insert(path.to_string(), 1);
                self.notify(
                    "textDocument/didOpen",
                    json!({"textDocument": {
                        "uri": uri, "languageId": language, "version": 1, "text": text,
                    }}),
                );
            }
        }
        self.pull(path);
        self.describe(path);
    }

    /// Asks for what the editor shows about a document's structure: its
    /// semantic colours, symbols and folds, for whichever the server has.
    pub(crate) fn describe(&mut self, path: &str) {
        let Some(&version) = self.opened.get(path) else {
            return;
        };
        let doc = json!({"textDocument": {"uri": uri(path)}});
        let path = path.to_string();
        if !self.legend.is_empty() {
            let request = Request::Tokens {
                path: path.clone(),
                version,
            };
            self.request("textDocument/semanticTokens/full", doc.clone(), request);
        }
        if self.symbols {
            let request = Request::Symbols {
                path: path.clone(),
                version,
            };
            self.request("textDocument/documentSymbol", doc.clone(), request);
        }
        if self.inlay_hints {
            // To the end of the last line: servers take a column past a
            // line's end as its end, as the protocol says.
            let last = self.last_lines.get(&path).copied().unwrap_or(0);
            let mut params = doc.clone();
            params["range"] = json!({
                "start": {"line": 0, "character": 0},
                "end": {"line": last, "character": 1 << 20},
            });
            let request = Request::InlayHints {
                path: path.clone(),
                version,
            };
            self.request("textDocument/inlayHint", params, request);
        }
        // rust-analyzer's tests and binaries, as tests and tasks.
        if self.program == "rust-analyzer" && path.ends_with(".rs") {
            let request = Request::Runnables {
                path: path.clone(),
                version,
            };
            self.request("experimental/runnables", doc.clone(), request);
        }
        if self.folds {
            self.request(
                "textDocument/foldingRange",
                doc,
                Request::Folds { path, version },
            );
        }
    }

    pub(crate) fn status_line(&self) -> (String, Tone) {
        match &self.status {
            Status::Starting => ("starting…".into(), Tone::Muted),
            Status::Failed(err) => (err.clone(), Tone::Error),
            Status::Ready if self.progress.is_empty() => ("ready".into(), Tone::Muted),
            Status::Ready => {
                let work: Vec<&str> = self.progress.values().map(String::as_str).collect();
                (work.join(", "), Tone::Warning)
            }
        }
    }
}
