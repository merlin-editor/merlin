//! Language servers: diagnostics, hover, go-to-definition, completion,
//! semantic colours, symbols, folding and inlay hints over the Language
//! Server Protocol.
//!
//! An ordinary plugin: it starts the server for the open file's language
//! through the process API, which frames messages with `Content-Length`, and
//! passes LSP positions straight through, since the plugin API counts
//! columns in UTF-16 as LSP does.

use std::collections::{BTreeMap, HashMap};

use merlin_plugin::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A language server, and the files it serves.
struct Spec {
    program: &'static str,
    args: &'static [&'static str],
    /// Extensions and the LSP language id of each.
    files: &'static [(&'static str, &'static str)],
}

const SERVERS: &[Spec] = &[
    Spec {
        program: "rust-analyzer",
        args: &[],
        files: &[("rs", "rust")],
    },
    Spec {
        program: "gopls",
        args: &[],
        files: &[("go", "go")],
    },
    Spec {
        program: "clangd",
        args: &[],
        files: &[
            ("c", "c"),
            ("h", "c"),
            ("cc", "cpp"),
            ("cpp", "cpp"),
            ("cxx", "cpp"),
            ("c++", "cpp"),
            ("hpp", "cpp"),
            ("hh", "cpp"),
            ("hxx", "cpp"),
            ("h++", "cpp"),
            ("ipp", "cpp"),
        ],
    },
    Spec {
        program: "pyright-langserver",
        args: &["--stdio"],
        files: &[("py", "python")],
    },
    Spec {
        program: "typescript-language-server",
        args: &["--stdio"],
        files: &[
            ("ts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("jsx", "javascriptreact"),
        ],
    },
];

/// The server and language id for a path, by its extension.
fn spec_for(path: &str) -> Option<(&'static Spec, &'static str)> {
    let ext = path.rsplit_once('.')?.1;
    SERVERS.iter().find_map(|s| {
        s.files
            .iter()
            .find(|(e, _)| *e == ext)
            .map(|(_, id)| (s, *id))
    })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum Status {
    Starting,
    Ready,
    Failed(String),
}

/// What a request's response is for.
#[derive(Serialize, Deserialize)]
enum Request {
    Initialize,
    Hover,
    Definition,
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
    Signature,
}

#[derive(Serialize, Deserialize)]
struct Server {
    program: String,
    process: Option<u32>,
    status: Status,
    next_id: u64,
    waiting: HashMap<u64, Request>,
    /// Open documents by absolute path, with their version.
    opened: HashMap<String, i32>,
    /// Messages held until the server is initialized.
    queue: Vec<Value>,
    /// Work in progress by token, such as indexing.
    progress: BTreeMap<String, String>,
    /// Whether the server gives diagnostics when asked (LSP 3.17 pull
    /// diagnostics). rust-analyzer gives its own only this way, and pushes
    /// just `cargo check`'s.
    #[serde(default)]
    pull: bool,
    /// Diagnostics by path, as pushed and as pulled.
    #[serde(default)]
    pushed: HashMap<String, Vec<Value>>,
    #[serde(default)]
    pulled: HashMap<String, Vec<Value>>,
    /// The newest completion request. Typing `a.b.` asks twice, and the
    /// answer for `a.` mustn't replace the one for `a.b.`.
    #[serde(default)]
    latest_completion: u64,
    /// The names of the server's semantic token types, by index; empty if
    /// it has none.
    #[serde(default)]
    legend: Vec<String>,
    /// Whether it gives symbols and folding ranges.
    #[serde(default)]
    symbols: bool,
    #[serde(default)]
    folds: bool,
    #[serde(default)]
    inlay_hints: bool,
    /// The last line of each open document, for asking about all of it.
    #[serde(default)]
    last_lines: HashMap<String, u32>,
    /// Whether it takes changes as ranges, rather than the whole text each
    /// time.
    #[serde(default)]
    incremental: bool,
    /// The editor's version of each open document the server has, to send
    /// only what changed since.
    #[serde(default)]
    synced: HashMap<String, u64>,
}

#[derive(Serialize, Deserialize)]
struct Problem {
    line: u32,
    column: u32,
    severity: u8,
    message: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Lsp {
    servers: Vec<Server>,
    /// Whether signature help shows, so typing asks again.
    #[serde(default)]
    signature: bool,
    /// Diagnostics by absolute path, for the panel.
    problems: BTreeMap<String, Vec<Problem>>,
}

impl Server {
    fn write(&mut self, message: Value) {
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

    fn request(&mut self, method: &str, params: Value, request: Request) {
        self.next_id += 1;
        self.waiting.insert(self.next_id, request);
        let id = self.next_id;
        self.write(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn respond(&mut self, id: Value, result: Value) {
        if let Some(process) = self.process {
            let message = json!({"jsonrpc": "2.0", "id": id, "result": result});
            let _ = host::send(process, &message.to_string());
        }
    }

    /// Asks for a document's diagnostics, if the server gives them that way.
    fn pull(&mut self, path: &str) {
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
    fn send_changes(&mut self, path: &str, changes: &[TextChange]) {
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
    fn sync(&mut self, path: &str, language: &str, text: &str) {
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
    fn describe(&mut self, path: &str) {
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
        if self.folds {
            self.request(
                "textDocument/foldingRange",
                doc,
                Request::Folds { path, version },
            );
        }
    }

    fn status_line(&self) -> (String, Tone) {
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

/// The semantic token types the plugin colours, as LSP names them.
const TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "concept",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
    "label",
];

/// The colour of a semantic token type.
fn token_kind(name: &str) -> Option<TokenKind> {
    Some(match name {
        "namespace" | "type" | "class" | "enum" | "interface" | "struct" | "typeParameter"
        | "concept" | "builtinType" | "typeAlias" => TokenKind::Type,
        "parameter" | "variable" | "selfKeyword" => TokenKind::Variable,
        "property" | "label" => TokenKind::Property,
        "enumMember" | "number" | "boolean" => TokenKind::Constant,
        "function" | "method" | "event" => TokenKind::Function,
        "macro" => TokenKind::Macro,
        "keyword" | "modifier" => TokenKind::Keyword,
        "comment" => TokenKind::Comment,
        "string" | "regexp" => TokenKind::String,
        "operator" => TokenKind::Operator,
        "decorator" | "attribute" => TokenKind::Attribute,
        _ => return None,
    })
}

/// Semantic tokens from LSP's relative encoding: five numbers a token,
/// its line and start relative to the token before, its length and type.
fn semantic_tokens(data: &[Value], legend: &[String]) -> Vec<Token> {
    let n = |v: &Value| v.as_u64().unwrap_or(0) as u32;
    let (mut line, mut start) = (0, 0);
    let mut tokens = Vec::new();
    for chunk in data.chunks_exact(5) {
        let delta_line = n(&chunk[0]);
        if delta_line > 0 {
            line += delta_line;
            start = 0;
        }
        start += n(&chunk[1]);
        let kind = legend
            .get(n(&chunk[3]) as usize)
            .and_then(|name| token_kind(name));
        if let Some(kind) = kind {
            tokens.push(Token {
                start: Position {
                    line,
                    column: start,
                },
                end: Position {
                    line,
                    column: start + n(&chunk[2]),
                },
                kind,
            });
        }
    }
    tokens
}

/// LSP's symbol kinds, by number.
const SYMBOL_KINDS: &[&str] = &[
    "file",
    "module",
    "namespace",
    "package",
    "class",
    "method",
    "property",
    "field",
    "constructor",
    "enum",
    "interface",
    "function",
    "variable",
    "constant",
    "string",
    "number",
    "boolean",
    "array",
    "object",
    "key",
    "null",
    "enum-member",
    "struct",
    "event",
    "operator",
    "type-parameter",
];

/// Symbols as a list, nested ones after the one they're in.
fn flatten_symbols(list: &[Value], depth: u32, out: &mut Vec<DocSymbol>) {
    for s in list {
        let kind = s["kind"]
            .as_u64()
            .and_then(|k| SYMBOL_KINDS.get(k.saturating_sub(1) as usize))
            .unwrap_or(&"symbol");
        // A DocumentSymbol has its range; a SymbolInformation a location.
        let range = if s["range"].is_object() {
            &s["range"]
        } else {
            &s["location"]["range"]
        };
        out.push(DocSymbol {
            name: s["name"].as_str().unwrap_or_default().to_string(),
            kind: kind.to_string(),
            detail: s["detail"].as_str().unwrap_or_default().to_string(),
            start: position(&range["start"]),
            end: position(&range["end"]),
            depth,
        });
        if let Some(children) = s["children"].as_array() {
            flatten_symbols(children, depth + 1, out);
        }
    }
}

/// A `file://` URI for an absolute path.
fn uri(path: &str) -> String {
    let mut out = String::from("file://");
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The path of a `file://` URI.
fn path_of(uri: &str) -> Option<String> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < encoded.len() {
        let hex = encoded
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok());
        match (encoded[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(b)) => {
                bytes.push(b);
                i += 3;
            }
            (b, _) => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// Hover text without markdown code fences or rules.
/// An LSP signature help answer, as the editor shows it: the active
/// signature, its active parameter as characters of the label, and the
/// first paragraph of its documentation.
fn signature(help: &Value) -> Option<Signature> {
    let list = help["signatures"].as_array().filter(|l| !l.is_empty())?;
    let index = help["activeSignature"].as_u64().unwrap_or(0) as usize;
    let index = index.min(list.len() - 1);
    let sig = &list[index];
    let label = sig["label"].as_str()?.to_string();
    let param = sig["activeParameter"]
        .as_u64()
        .or(help["activeParameter"].as_u64())
        .and_then(|i| sig["parameters"].as_array()?.get(i as usize).cloned());
    let active = param.and_then(|p| match &p["label"] {
        // UTF-16 offsets into the label.
        Value::Array(range) => {
            let chars = |n: u64| {
                let mut seen = 0;
                label
                    .chars()
                    .take_while(|c| {
                        seen += c.len_utf16() as u64;
                        seen <= n
                    })
                    .count() as u32
            };
            Some((
                chars(range.first()?.as_u64()?),
                chars(range.get(1)?.as_u64()?),
            ))
        }
        Value::String(name) => {
            let at = label.find(name.as_str())?;
            let start = label[..at].chars().count() as u32;
            Some((start, start + name.chars().count() as u32))
        }
        _ => None,
    });
    let doc = match &sig["documentation"] {
        Value::String(s) => s.clone(),
        other => other["value"].as_str().unwrap_or_default().to_string(),
    };
    // Its first paragraph, outside any code block.
    let doc = doc
        .split("\n\n")
        .find(|p| !p.trim().is_empty() && !p.trim_start().starts_with("```"))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Some(Signature {
        label,
        active,
        doc,
        index: index as u32,
        count: list.len() as u32,
    })
}

/// An LSP inlay hint: its label, whole or in parts, at its position.
fn inlay_hint(hint: &Value) -> Option<InlayHint> {
    let label = match &hint["label"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
        _ => return None,
    };
    let label = label.trim().to_string();
    let kind = match hint["kind"].as_u64() {
        Some(1) => InlayKind::Type,
        Some(2) => InlayKind::Parameter,
        _ => InlayKind::Other,
    };
    (!label.is_empty()).then(|| InlayHint {
        at: position(&hint["position"]),
        label,
        kind,
    })
}

fn hover_text(contents: &Value) -> String {
    let raw = match contents {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(hover_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(o) => o
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    };
    let mut lines: Vec<&str> = Vec::new();
    for line in raw.lines() {
        let line = if line.starts_with("```") || line.trim() == "---" {
            ""
        } else {
            line
        };
        if !(line.is_empty() && lines.last().is_none_or(|l| l.is_empty())) {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_string()
}

fn position(value: &Value) -> Position {
    Position {
        line: value["line"].as_u64().unwrap_or(0) as u32,
        column: value["character"].as_u64().unwrap_or(0) as u32,
    }
}

impl Lsp {
    /// The active file as an absolute path, with its server and language.
    fn active(&self) -> Option<(String, &'static Spec, &'static str)> {
        let path = host::active_path().ok()??;
        let (spec, language) = spec_for(&path)?;
        let path = if path.starts_with('/') {
            path
        } else {
            format!("{}/{path}", host::workspace_root())
        };
        Some((path, spec, language))
    }

    fn server(&mut self, program: &str) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.program == program)
    }

    /// The server for `spec`, starting it if it isn't running.
    fn ensure(&mut self, spec: &Spec) -> &mut Server {
        let index = match self.servers.iter().position(|s| s.program == spec.program) {
            Some(i) if self.servers[i].process.is_some() => i,
            found => {
                let args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();
                let process = host::spawn(spec.program, &args, &[], Framing::ContentLength);
                let server = Server {
                    program: spec.program.into(),
                    process: process.as_ref().ok().copied(),
                    status: match &process {
                        Ok(_) => Status::Starting,
                        Err(err) => Status::Failed(err.clone()),
                    },
                    next_id: 0,
                    waiting: HashMap::new(),
                    opened: HashMap::new(),
                    queue: Vec::new(),
                    progress: BTreeMap::new(),
                    pull: false,
                    pushed: HashMap::new(),
                    pulled: HashMap::new(),
                    latest_completion: 0,
                    legend: Vec::new(),
                    symbols: false,
                    folds: false,
                    inlay_hints: false,
                    last_lines: HashMap::new(),
                    incremental: false,
                    synced: HashMap::new(),
                };
                let i = match found {
                    Some(i) => {
                        self.servers[i] = server;
                        i
                    }
                    None => {
                        self.servers.push(server);
                        self.servers.len() - 1
                    }
                };
                if process.is_ok() {
                    let root = host::workspace_root();
                    let capabilities = json!({
                        "general": {"positionEncodings": ["utf-16"]},
                        "window": {"workDoneProgress": true},
                        "workspace": {
                            "inlayHint": {"refreshSupport": true},
                            "semanticTokens": {"refreshSupport": true},
                            "diagnostics": {"refreshSupport": true},
                        },
                        "textDocument": {
                            "synchronization": {"didSave": true},
                            "hover": {"contentFormat": ["plaintext", "markdown"]},
                            "completion": {"completionItem": {"snippetSupport": false}},
                            "definition": {},
                            "signatureHelp": {
                                "signatureInformation": {
                                    "documentationFormat": ["plaintext", "markdown"],
                                    "parameterInformation": {"labelOffsetSupport": true},
                                    "activeParameterSupport": true,
                                },
                            },
                            "publishDiagnostics": {},
                            "diagnostic": {"dynamicRegistration": false},
                            "semanticTokens": {
                                "requests": {"full": true},
                                "tokenTypes": TOKEN_TYPES,
                                "tokenModifiers": [],
                                "formats": ["relative"],
                            },
                            "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
                            "foldingRange": {"lineFoldingOnly": true},
                            "inlayHint": {},
                        },
                    });
                    self.servers[i].request(
                        "initialize",
                        json!({
                            "processId": null,
                            "rootUri": uri(&root),
                            "workspaceFolders": [{"uri": uri(&root), "name": "workspace"}],
                            "capabilities": capabilities,
                        }),
                        Request::Initialize,
                    );
                }
                i
            }
        };
        &mut self.servers[index]
    }

    /// Syncs the active file, then sends a request about the cursor.
    fn ask(&mut self, method: &str, request: Request) {
        let Some((path, spec, language)) = self.active() else {
            return host::notify("No language server for this file");
        };
        let Ok(cursor) = host::active_cursor() else {
            return;
        };
        let text = host::active_text().unwrap_or_default();
        let server = self.ensure(spec);
        if !server.opened.contains_key(&path) {
            server.sync(&path, language, &text);
        }
        let params = json!({
            "textDocument": {"uri": uri(&path)},
            "position": {"line": cursor.line, "character": cursor.column},
        });
        if matches!(request, Request::Completion { .. }) {
            server.latest_completion = server.next_id + 1;
        }
        server.request(method, params, request);
    }

    fn on_message(&mut self, program: &str, message: Value) {
        let method = message["method"].as_str().map(String::from);
        let id = message.get("id").cloned();
        match (method, id) {
            (Some(method), Some(id)) => self.on_request(program, &method, id, &message["params"]),
            (Some(method), None) => self.on_notification(program, &method, &message["params"]),
            (None, Some(id)) => self.on_response(program, &id, &message),
            (None, None) => {}
        }
    }

    fn on_request(&mut self, program: &str, method: &str, id: Value, params: &Value) {
        let Some(server) = self.server(program) else {
            return;
        };
        let result = match method {
            "workspace/configuration" => {
                let count = params["items"].as_array().map_or(0, Vec::len);
                Value::Array(vec![Value::Null; count])
            }
            "workspace/applyEdit" => json!({"applied": false}),
            _ => Value::Null,
        };
        server.respond(id, result);
        // The server knows more now, such as once it's indexed: ask again.
        let paths: Vec<String> = server.opened.keys().cloned().collect();
        match method {
            "workspace/diagnostic/refresh" => paths.iter().for_each(|p| server.pull(p)),
            "workspace/inlayHint/refresh" | "workspace/semanticTokens/refresh" => {
                paths.iter().for_each(|p| server.describe(p))
            }
            _ => {}
        }
    }

    /// Shows a file's pushed and pulled diagnostics together.
    fn publish(&mut self, program: &str, path: &str) {
        let Some(server) = self.server(program) else {
            return;
        };
        let list: Vec<Value> = [&server.pushed, &server.pulled]
            .into_iter()
            .filter_map(|m| m.get(path))
            .flatten()
            .cloned()
            .collect();
        let diagnostics: Vec<Diagnostic> = list
            .iter()
            .map(|d| Diagnostic {
                start: position(&d["range"]["start"]),
                end: position(&d["range"]["end"]),
                severity: match d["severity"].as_u64() {
                    Some(2) => Severity::Warning,
                    Some(3) => Severity::Info,
                    Some(4) => Severity::Hint,
                    _ => Severity::Error,
                },
                message: d["message"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        let mut problems: Vec<Problem> = Vec::new();
        for d in &diagnostics {
            let problem = Problem {
                line: d.start.line,
                column: d.start.column,
                severity: match d.severity {
                    Severity::Error => 1,
                    Severity::Warning => 2,
                    _ => continue,
                },
                message: d.message.lines().next().unwrap_or_default().to_string(),
            };
            // Servers repeat themselves, such as one syntax error per token.
            let seen = problems
                .iter()
                .any(|p| p.line == problem.line && p.message == problem.message);
            if !seen {
                problems.push(problem);
            }
        }
        if problems.is_empty() {
            self.problems.remove(path);
        } else {
            self.problems.insert(path.to_string(), problems);
        }
        host::set_diagnostics(path, &diagnostics);
    }

    fn on_notification(&mut self, program: &str, method: &str, params: &Value) {
        match method {
            "textDocument/publishDiagnostics" => {
                let Some(path) = params["uri"].as_str().and_then(path_of) else {
                    return;
                };
                let list = params["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                if let Some(server) = self.server(program) {
                    server.pushed.insert(path.clone(), list);
                }
                self.publish(program, &path);
            }
            "$/progress" => {
                let Some(server) = self.server(program) else {
                    return;
                };
                let token = params["token"].to_string();
                let value = &params["value"];
                match value["kind"].as_str() {
                    Some("end") => {
                        server.progress.remove(&token);
                    }
                    Some(kind) => {
                        let title = match (kind, server.progress.get(&token)) {
                            ("report", Some(title)) => {
                                title.split(':').next().unwrap_or_default().into()
                            }
                            _ => value["title"].as_str().unwrap_or("working").to_string(),
                        };
                        let detail = match value["percentage"].as_u64() {
                            Some(p) => format!("{title}: {p}%"),
                            None => title,
                        };
                        server.progress.insert(token, detail);
                    }
                    None => {}
                }
            }
            "window/showMessage" if params["type"].as_u64() == Some(1) => host::notify(&format!(
                "{program}: {}",
                params["message"].as_str().unwrap_or_default()
            )),
            _ => {}
        }
    }

    fn on_response(&mut self, program: &str, id: &Value, message: &Value) {
        let Some(server) = self.server(program) else {
            return;
        };
        let Some(request) = id.as_u64().and_then(|id| server.waiting.remove(&id)) else {
            return;
        };
        if matches!(request, Request::Completion { .. })
            && id.as_u64() != Some(server.latest_completion)
        {
            return;
        }
        if let Some(error) = message.get("error") {
            // Servers cancel requests that newer edits made pointless.
            let cancelled = matches!(error["code"].as_i64(), Some(-32802..=-32800));
            let quiet = matches!(
                request,
                Request::Completion { manual: false }
                    | Request::Diagnostics { .. }
                    | Request::Tokens { .. }
                    | Request::Symbols { .. }
                    | Request::Folds { .. }
                    | Request::InlayHints { .. }
                    | Request::Signature
            );
            if !cancelled && !quiet {
                host::notify(&format!(
                    "{program}: {}",
                    error["message"].as_str().unwrap_or("error")
                ));
            }
            return;
        }
        let result = &message["result"];
        match request {
            Request::Initialize => {
                let caps = &result["capabilities"];
                server.pull = !caps["diagnosticProvider"].is_null();
                server.legend = caps["semanticTokensProvider"]["legend"]["tokenTypes"]
                    .as_array()
                    .map(|types| {
                        let name = |t: &Value| t.as_str().unwrap_or_default().to_string();
                        types.iter().map(name).collect()
                    })
                    .unwrap_or_default();
                let offered = |v: &Value| !v.is_null() && *v != Value::Bool(false);
                server.symbols = offered(&caps["documentSymbolProvider"]);
                server.folds = offered(&caps["foldingRangeProvider"]);
                server.inlay_hints = offered(&caps["inlayHintProvider"]);
                // 2 is incremental, as a number or as `change` in an object.
                let sync = &caps["textDocumentSync"];
                server.incremental = sync.as_u64().or_else(|| sync["change"].as_u64()) == Some(2);
                server.status = Status::Ready;
                server.notify("initialized", json!({}));
                for queued in std::mem::take(&mut server.queue) {
                    server.write(queued);
                }
                // Files opened while it started didn't know to ask.
                let paths: Vec<String> = server.opened.keys().cloned().collect();
                for path in paths {
                    server.pull(&path);
                    server.describe(&path);
                }
            }
            Request::Hover => match result.get("contents").map(hover_text) {
                Some(text) if !text.is_empty() => host::show_hover(&text),
                _ => host::notify("Nothing to show here"),
            },
            Request::Definition => {
                let first = match result {
                    Value::Array(list) => list.first(),
                    Value::Null => None,
                    one => Some(one),
                };
                let target = first.and_then(|l| {
                    let uri = l.get("uri").or(l.get("targetUri"))?.as_str()?;
                    let range = l.get("range").or(l.get("targetSelectionRange"))?;
                    Some((path_of(uri)?, position(&range["start"])))
                });
                match target {
                    Some((path, at)) => host::open_at(&path, at),
                    None => host::notify("No definition found"),
                }
            }
            Request::Diagnostics { path, version } => {
                // An answer about older text, or "unchanged", keeps what's shown.
                let current = server.opened.get(&path) == Some(&version);
                if let (true, Some(items)) = (current, result["items"].as_array()) {
                    server.pulled.insert(path.clone(), items.clone());
                    self.publish(program, &path);
                }
            }
            Request::Tokens { path, version } => {
                if server.opened.get(&path) == Some(&version)
                    && let Some(data) = result["data"].as_array()
                {
                    let tokens = semantic_tokens(data, &server.legend);
                    host::set_tokens(&path, TokenLayer::Semantic, None, &tokens);
                }
            }
            Request::Symbols { path, version } => {
                if server.opened.get(&path) == Some(&version)
                    && let Some(list) = result.as_array()
                {
                    let mut symbols = Vec::new();
                    flatten_symbols(list, 0, &mut symbols);
                    host::set_symbols(&path, &symbols);
                }
            }
            Request::Folds { path, version } => {
                if server.opened.get(&path) == Some(&version)
                    && let Some(list) = result.as_array()
                {
                    let ranges: Vec<FoldRange> = list
                        .iter()
                        .filter_map(|r| {
                            let first = r["startLine"].as_u64()? as u32;
                            let last = r["endLine"].as_u64()? as u32;
                            (last > first).then_some(FoldRange { first, last })
                        })
                        .collect();
                    host::set_folds(&path, &ranges);
                }
            }
            Request::Signature => {
                let shown = signature(result);
                self.signature = shown.is_some();
                host::show_signature(shown.as_ref());
            }
            Request::InlayHints { path, version } => {
                if server.opened.get(&path) == Some(&version)
                    && let Some(list) = result.as_array()
                {
                    let hints: Vec<InlayHint> = list.iter().filter_map(inlay_hint).collect();
                    host::set_inlay_hints(&path, &hints);
                }
            }
            Request::Completion { manual } => {
                let items = match result {
                    Value::Array(items) => items.clone(),
                    _ => result["items"].as_array().cloned().unwrap_or_default(),
                };
                let mut items: Vec<(String, Completion)> = items
                    .iter()
                    .take(500)
                    .map(|item| {
                        let label = item["label"].as_str().unwrap_or_default().to_string();
                        let insert = item["textEdit"]["newText"]
                            .as_str()
                            .or(item["insertText"].as_str())
                            .unwrap_or(&label)
                            .to_string();
                        let detail = item["labelDetails"]["detail"]
                            .as_str()
                            .or(item["detail"].as_str())
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        let sort = item["sortText"].as_str().unwrap_or(&label).to_string();
                        (
                            sort,
                            Completion {
                                label,
                                detail,
                                insert,
                            },
                        )
                    })
                    .collect();
                items.sort_by(|a, b| a.0.cmp(&b.0));
                if manual || !items.is_empty() {
                    let items: Vec<Completion> = items.into_iter().map(|(_, c)| c).collect();
                    host::show_completions(&items);
                }
            }
        }
    }

    /// Asks for completions after `.` or `::`, as most editors do, and for
    /// signature help after `(` or `,`, or while it shows.
    fn typed(&mut self) {
        let (Ok(text), Ok(cursor)) = (host::active_text(), host::active_cursor()) else {
            return;
        };
        let Some(line) = text.lines().nth(cursor.line as usize) else {
            return;
        };
        let mut units = 0;
        let before: String = line
            .chars()
            .take_while(|c| {
                units += c.len_utf16() as u32;
                units <= cursor.column
            })
            .collect();
        if before.ends_with('.') && !before.ends_with("..") || before.ends_with("::") {
            self.ask(
                "textDocument/completion",
                Request::Completion { manual: false },
            );
        }
        if self.signature || before.ends_with('(') || before.ends_with(',') {
            self.ask("textDocument/signatureHelp", Request::Signature);
        }
    }
}

impl Plugin for Lsp {
    fn commands(&self) -> Vec<Command> {
        vec![
            Command::new("hover", "Language: Show Hover").key("alt+k"),
            Command::new("definition", "Language: Go to Definition").key("alt+d"),
            Command::new("complete", "Language: Complete").key("ctrl+space"),
            Command::new("problems", "Language: Show Problems").key("alt+p"),
            Command::new("restart", "Language: Restart Servers"),
        ]
    }

    fn run_command(&mut self, id: &str) {
        match id {
            "hover" => self.ask("textDocument/hover", Request::Hover),
            "definition" => self.ask("textDocument/definition", Request::Definition),
            "complete" => self.ask(
                "textDocument/completion",
                Request::Completion { manual: true },
            ),
            "problems" => host::focus_panel(),
            "restart" => {
                for server in self.servers.drain(..) {
                    if let Some(process) = server.process {
                        host::kill(process);
                    }
                }
                self.buffer(BufferEvent::Switched);
            }
            _ => {}
        }
    }

    fn buffer(&mut self, event: BufferEvent) {
        let Some((path, spec, language)) = self.active() else {
            return;
        };
        if let BufferEvent::Saved(_) = event {
            if let Some(server) = self.server(spec.program)
                && server.opened.contains_key(&path)
            {
                server.notify(
                    "textDocument/didSave",
                    json!({"textDocument": {"uri": uri(&path)}}),
                );
            }
            return;
        }
        let now = host::active_version().ok();
        let server = self.ensure(spec);
        let open = server.opened.contains_key(&path);
        let was = server.synced.get(&path).copied();
        // Back to a file the server has as it is.
        if matches!(event, BufferEvent::Switched) && open && (now.is_none() || now == was) {
            return;
        }
        // Only what changed, when the server takes ranges and the editor
        // still has them; else the whole text.
        let changes = match (server.incremental && open, was) {
            (true, Some(was)) => host::active_changes(was).ok().flatten(),
            _ => None,
        };
        match changes {
            Some(changes) if changes.is_empty() => {}
            Some(changes) => server.send_changes(&path, &changes),
            None => {
                let text = host::active_text().unwrap_or_default();
                server.sync(&path, language, &text);
            }
        }
        if let Some(now) = now {
            server.synced.insert(path.clone(), now);
        }
        if matches!(event, BufferEvent::Changed) {
            self.typed();
        }
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        let Some(program) = self
            .servers
            .iter()
            .find(|s| s.process == Some(process))
            .map(|s| s.program.clone())
        else {
            return;
        };
        match event {
            ProcessEvent::Message(text) => {
                if let Ok(message) = serde_json::from_str(&text) {
                    self.on_message(&program, message);
                }
            }
            ProcessEvent::Stderr(_) => {}
            ProcessEvent::Exit(code) => {
                if let Some(server) = self.server(&program) {
                    server.process = None;
                    server.opened.clear();
                    let code = code.map_or("a signal".into(), |c| format!("code {c}"));
                    server.status = Status::Failed(format!("exited ({code})"));
                }
            }
        }
    }

    fn panel(&mut self) -> Option<Panel> {
        let mut panel = Panel::new("Language");
        if self.servers.is_empty() {
            let names: Vec<&str> = SERVERS.iter().map(|s| s.program).collect();
            panel = panel
                .styled(
                    "Open a source file to start its language server.",
                    Tone::Muted,
                )
                .styled(format!("Knows: {}", names.join(", ")), Tone::Muted);
        }
        for server in &self.servers {
            let (status, tone) = server.status_line();
            panel = panel.node(Node::styled(format!("{}: {status}", server.program), tone));
        }
        let count: usize = self.problems.values().map(Vec::len).sum();
        panel = panel.divider().heading(match count {
            0 => "No problems".to_string(),
            1 => "1 problem".to_string(),
            n => format!("{n} problems"),
        });
        // The active file's problems first.
        let active = self.active().map(|(p, _, _)| p);
        let root = format!("{}/", host::workspace_root());
        let mut files: Vec<&String> = self.problems.keys().collect();
        files.sort_by_key(|p| Some(*p) != active.as_ref());
        for path in files.into_iter().take(10) {
            panel = panel.styled(path.strip_prefix(&root).unwrap_or(path), Tone::Strong);
            for p in self.problems[path].iter().take(20) {
                let (mark, tone) = if p.severity == 1 {
                    ("E", Tone::Error)
                } else {
                    ("W", Tone::Warning)
                };
                let label = format!("{mark} {}: {}", p.line + 1, p.message);
                panel = panel.item(format!("go:{}:{}:{path}", p.line, p.column), label, tone);
            }
        }
        Some(panel)
    }

    fn click(&mut self, button: &str) {
        let mut parts = button.splitn(4, ':');
        if let (Some("go"), Some(line), Some(column), Some(path)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        {
            let at = Position {
                line: line.parse().unwrap_or(0),
                column: column.parse().unwrap_or(0),
            };
            host::open_at(path, at);
        }
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(self).ok()
    }

    fn restore_state(&mut self, state: Vec<u8>) {
        if let Ok(lsp) = serde_json::from_slice(&state) {
            *self = lsp;
        }
    }
}

merlin_plugin::export!(Lsp);
