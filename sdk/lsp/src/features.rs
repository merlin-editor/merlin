//! Outgoing LSP request dispatch, document sync, formatting, and typing triggers.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use merlin_plugin::{FileEdit, Framing, LogLevel, Position, TextChange, host};
use serde_json::{Value, json};

use crate::Lsp;
use crate::protocol::{TOKEN_TYPES, text_changes, uri};
use crate::server::{Request, Server, Status};
use crate::spec::Spec;

/// What to format.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Formatting {
    File,
    Selection,
    /// Only the lines git says changed, for a save.
    Changed,
}

/// Answers a format with the server's edits for `path`, none if it gave
/// none: a save waits for this.
pub(crate) fn formatted(path: &str, edits: &Value) {
    let file = FileEdit {
        path: path.to_string(),
        edits: text_changes(edits),
    };
    if let Err(err) = host::apply_edits(&[file]) {
        host::notice(LogLevel::Error, &err, &[]);
    }
}

impl Lsp {
    /// The server and language id for a path, by its extension.
    pub(crate) fn spec_for(&self, path: &str) -> Option<(&'static Spec, &'static str)> {
        let ext = path.rsplit_once('.')?.1;
        self.specs.iter().find_map(|s| {
            s.files
                .iter()
                .find(|(e, _)| e.eq_ignore_ascii_case(ext))
                .map(|(_, id)| (s, *id))
        })
    }

    /// The active file as an absolute path, with its server and language.
    pub(crate) fn active(&self) -> Option<(String, &'static Spec, &'static str)> {
        let path = host::active_path().ok()??;
        let (spec, language) = self.spec_for(&path)?;
        let path = if path.starts_with('/') {
            path
        } else {
            format!("{}/{path}", host::workspace_root())
        };
        Some((path, spec, language))
    }

    pub(crate) fn server(&mut self, program: &str) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.program == program)
    }

    /// The server for `spec`, starting it if it isn't running.
    pub(crate) fn ensure(&mut self, spec: &Spec) -> &mut Server {
        let index = match self.servers.iter().position(|s| s.program == spec.program) {
            Some(i) if self.servers[i].process.is_some() => i,
            found => {
                let args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();
                let process = host::spawn(spec.program, &args, &[], Framing::ContentLength);
                let settings = server_settings(spec);
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
                    workspace_symbols: false,
                    folds: false,
                    inlay_hints: false,
                    formats: false,
                    formats_ranges: false,
                    renames: false,
                    prepares_renames: false,
                    last_lines: HashMap::new(),
                    incremental: false,
                    synced: HashMap::new(),
                    tasks: BTreeMap::new(),
                    settings: settings.clone(),
                    folders: BTreeSet::new(),
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
                            "configuration": true,
                            "workspaceFolders": true,
                            "inlayHint": {"refreshSupport": true},
                            "semanticTokens": {"refreshSupport": true},
                            "diagnostics": {"refreshSupport": true},
                        },
                        "textDocument": {
                            "synchronization": {"didSave": true},
                            "hover": {"contentFormat": ["markdown", "plaintext"]},
                            "completion": {"completionItem": {"snippetSupport": false}},
                            "definition": {},
                            "formatting": {},
                            "rangeFormatting": {},
                            "rename": {"prepareSupport": true},
                            "references": {},
                            "codeAction": {
                                "codeActionLiteralSupport": {
                                    "codeActionKind": {
                                        "valueSet": [
                                            "", "quickfix", "refactor", "refactor.extract",
                                            "refactor.inline", "refactor.rewrite", "source",
                                            "source.organizeImports",
                                        ],
                                    },
                                },
                                "isPreferredSupport": true,
                                "disabledSupport": true,
                            },
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
                    let mut params = json!({
                        "processId": null,
                        "rootUri": uri(&root),
                        "workspaceFolders": [{"uri": uri(&root), "name": "workspace"}],
                        "capabilities": capabilities,
                    });
                    if !settings.is_null() {
                        params["initializationOptions"] = settings;
                    }
                    self.servers[i].request("initialize", params, Request::Initialize);
                }
                i
            }
        };
        &mut self.servers[index]
    }

    /// Brings the server's copy of the active file up to the editor's, if
    /// it's behind, so positions in it mean the same to both. Returns the
    /// incremental changes when that's how it synced.
    pub(crate) fn catch_up(
        &mut self,
        path: &str,
        spec: &'static Spec,
        language: &str,
    ) -> Option<Vec<TextChange>> {
        let now = host::active_version().ok();
        let server = self.ensure(spec);
        let open = server.opened.contains_key(path);
        let was = server.synced.get(path).copied();
        // The server has it as it is.
        if open && (now.is_none() || now == was) {
            return None;
        }
        // Only what changed, when the server takes ranges and the editor
        // still has them; else the whole text.
        let changes = match (server.incremental && open, was) {
            (true, Some(was)) => host::active_changes(was).ok().flatten(),
            _ => None,
        };
        match &changes {
            Some(changes) if changes.is_empty() => {}
            Some(changes) => server.send_changes(path, changes),
            None => {
                let text = host::active_text().unwrap_or_default();
                server.sync(path, language, &text);
            }
        }
        if let Some(now) = now {
            server.synced.insert(path.to_string(), now);
        }
        changes
    }

    pub(crate) fn ask(&mut self, method: &str, request: Request) {
        // Most of these come by themselves, as a hover under the mouse
        // does: a file no server reads has nothing to say.
        let Some((path, spec, language)) = self.active() else {
            // The editor waits to hear about the name under Ctrl.
            if matches!(request, Request::DefinitionLink) {
                host::show_definitions(&[]);
            }
            return;
        };
        let Ok(cursor) = host::active_cursor() else {
            return;
        };
        // A position in text the server hasn't had yet would be wrong, or
        // past its end.
        self.catch_up(&path, spec, language);
        let server = self.ensure(spec);
        let here = json!({"line": cursor.line, "character": cursor.column});
        let params = if matches!(request, Request::CodeActions) {
            // With the problems on the cursor's line, which the fixes are for.
            let line = cursor.line as u64;
            let on_line = |d: &&Value| {
                let start = d["range"]["start"]["line"].as_u64().unwrap_or(u64::MAX);
                let end = d["range"]["end"]["line"].as_u64().unwrap_or(0);
                (start..=end).contains(&line)
            };
            let diagnostics: Vec<Value> = [&server.pushed, &server.pulled]
                .into_iter()
                .filter_map(|m| m.get(&path))
                .flatten()
                .filter(on_line)
                .cloned()
                .collect();
            json!({
                "textDocument": {"uri": uri(&path)},
                "range": {"start": here, "end": here},
                "context": {"diagnostics": diagnostics, "triggerKind": 1},
            })
        } else if matches!(request, Request::Peek) {
            // As VS Code's peek, without the name's own definition.
            json!({
                "textDocument": {"uri": uri(&path)},
                "position": here,
                "context": {"includeDeclaration": false},
            })
        } else {
            json!({
                "textDocument": {"uri": uri(&path)},
                "position": here,
            })
        };
        if matches!(request, Request::Completion { .. }) {
            server.latest_completion = server.next_id + 1;
        }
        server.request(method, params, request);
    }

    /// Asks each server that's running, or the current file's, for the
    /// project's symbols matching the palette's query.
    pub(crate) fn project_symbols(&mut self) {
        let query = host::symbol_query();
        if query.is_empty() {
            return;
        }
        if self.servers.iter().all(|s| s.process.is_none())
            && let Some((_, spec, _)) = self.active()
        {
            self.ensure(spec);
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
        // Nothing to ask: the palette hears there's nothing.
        if !asked {
            host::show_project_symbols(&query, &[]);
        }
    }

    /// The editor's F2: whether the name at the cursor can be renamed, and
    /// how far it reaches, for its rename box.
    pub(crate) fn prepare_rename(&mut self) {
        let Some((path, spec, language)) = self.active() else {
            // Nothing to say: the box just closes.
            return host::show_rename(Err(""));
        };
        let Ok(cursor) = host::active_cursor() else {
            return;
        };
        let text = host::active_text().unwrap_or_default();
        let server = self.ensure(spec);
        if let Status::Failed(err) = &server.status {
            let why = format!("{} isn't running: {err}", spec.program);
            return host::show_rename(Err(&why));
        }
        // Until it's started, it's asked anyway: the box hears if it can't.
        if server.status == Status::Ready && !server.renames {
            let why = format!("{} can't rename", spec.program);
            return host::show_rename(Err(&why));
        }
        if !server.opened.contains_key(&path) {
            server.sync(&path, language, &text);
        }
        self.rename_at = Some((path.clone(), (cursor.line, cursor.column)));
        let server = self.ensure(spec);
        if server.prepares_renames {
            server.request(
                "textDocument/prepareRename",
                json!({
                    "textDocument": {"uri": uri(&path)},
                    "position": {"line": cursor.line, "character": cursor.column},
                }),
                Request::PrepareRename,
            );
        } else {
            self.find_references(spec.program, None);
        }
    }

    /// Asks where the name being renamed is used, to count the places.
    pub(crate) fn find_references(
        &mut self,
        program: &str,
        name: Option<((u32, u32), (u32, u32))>,
    ) {
        let Some((path, at)) = self.rename_at.clone() else {
            return;
        };
        let Some(server) = self.server(program) else {
            return;
        };
        server.request(
            "textDocument/references",
            json!({
                "textDocument": {"uri": uri(&path)},
                "position": {"line": at.0, "character": at.1},
                "context": {"includeDeclaration": true},
            }),
            Request::References { name },
        );
    }

    /// Formats the active file, its selection, or its changed lines. A
    /// save waits for the answer, so there's always one, with no edits
    /// when it can't. Why a save didn't format is said once a file.
    pub(crate) fn format(&mut self, what: Formatting, saving: bool) {
        let active = host::active_path().ok().flatten().unwrap_or_default();
        let say = |lsp: &mut Self, why: String| {
            // The Format mock-up says when a save went out unformatted,
            // once a file.
            if !saving || lsp.warned.insert(format!("{active}\n{why}")) {
                host::notice(LogLevel::Info, &why, &[]);
            }
        };
        let Some((path, spec, language)) = self.active() else {
            let kind = active.rsplit('.').next().unwrap_or("these").to_uppercase();
            say(self, format!("Nothing formats {kind} files here"));
            return formatted(&active, &Value::Null);
        };
        let text = host::active_text().unwrap_or_default();
        let program = spec.program;
        let server = self.ensure(spec);
        if !server.opened.contains_key(&path) {
            server.sync(&path, language, &text);
        }
        // Started now: what it offers isn't known until it answers.
        let ready = server.status == Status::Ready;
        let (formats, ranges) = (server.formats || !ready, server.formats_ranges || !ready);
        let doc = json!({"uri": uri(&path)});
        let options = json!({"tabSize": 4, "insertSpaces": true});
        let name = active.rsplit('/').next().unwrap_or(&active).to_string();
        match what {
            Formatting::File if formats => server.request(
                "textDocument/formatting",
                json!({"textDocument": doc, "options": options}),
                Request::Format { path },
            ),
            Formatting::File => {
                say(self, format!("{program} doesn't format files"));
                formatted(&path, &Value::Null);
            }
            Formatting::Selection => {
                let selection = host::active_selection().ok().flatten();
                match selection {
                    Some(s) if ranges => {
                        let at = |p: Position| json!({"line": p.line, "character": p.column});
                        server.request(
                            "textDocument/rangeFormatting",
                            json!({
                                "textDocument": doc,
                                "range": {"start": at(s.start), "end": at(s.end)},
                                "options": options,
                            }),
                            Request::Format { path },
                        );
                    }
                    Some(_) => {
                        say(self, format!("{program} can't format a selection"));
                        formatted(&path, &Value::Null);
                    }
                    None => formatted(&path, &Value::Null),
                }
            }
            Formatting::Changed => {
                let changed = host::changed_lines().ok().flatten();
                match changed {
                    None => {
                        say(
                            self,
                            format!("{name} isn't in git, so it saved unformatted"),
                        );
                        formatted(&path, &Value::Null);
                    }
                    Some(lines) if !ranges => {
                        let _ = lines;
                        say(
                            self,
                            format!(
                                "{program} can't format only changed lines, so {name} saved unformatted"
                            ),
                        );
                        formatted(&path, &Value::Null);
                    }
                    Some(lines) if lines.is_empty() => formatted(&path, &Value::Null),
                    Some(lines) => {
                        self.next_batch += 1;
                        let batch = self.next_batch;
                        self.batches
                            .insert(batch, (path.clone(), lines.len(), Vec::new()));
                        let Some(server) = self.server(program) else {
                            return;
                        };
                        for range in lines {
                            server.request(
                                "textDocument/rangeFormatting",
                                json!({
                                    "textDocument": doc,
                                    "range": {
                                        "start": {"line": range.start, "character": 0},
                                        "end": {"line": range.end + 1, "character": 0},
                                    },
                                    "options": options,
                                }),
                                Request::FormatPart { batch },
                            );
                        }
                    }
                }
            }
        }
    }

    /// One changed range's formatting came: once they all have, the save
    /// gets their edits together.
    pub(crate) fn part_formatted(&mut self, batch: u64, edits: &Value) {
        let Some((_, left, all)) = self.batches.get_mut(&batch) else {
            return;
        };
        all.extend(edits.as_array().into_iter().flatten().cloned());
        *left -= 1;
        if *left == 0
            && let Some((path, _, all)) = self.batches.remove(&batch)
        {
            formatted(&path, &Value::Array(all));
        }
    }

    /// Asks for completions after `.` or `::`, as most editors do, and for
    /// signature help after `(` or `,`, or while it shows.
    pub(crate) fn typed(&mut self, changes: Option<Vec<TextChange>>) {
        if !self.signature
            && let Some(changes) = &changes
            && let Some(last) = changes.last()
            && !last.text.is_empty()
            && !last.text.ends_with(['.', ':', '(', ','])
        {
            return;
        }
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

/// The JSON settings a server gets: its spec's, with the plugin's own
/// settings it names put in.
pub(crate) fn server_settings(spec: &Spec) -> Value {
    let mut settings = spec
        .settings
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    for (key, path) in spec.options {
        let Some(value) = merlin_plugin::host::get_setting(key) else {
            continue;
        };
        let value = match value {
            merlin_plugin::Setting::Text(s) => Value::String(s),
            merlin_plugin::Setting::Flag(b) => Value::Bool(b),
            merlin_plugin::Setting::Integer(i) => Value::from(i),
            merlin_plugin::Setting::Float(f) => Value::from(f),
            // Lists of strings come as TOML, which reads as JSON too.
            merlin_plugin::Setting::Other(text) => {
                serde_json::from_str(&text).unwrap_or(Value::String(text))
            }
        };
        if !settings.is_object() {
            settings = Value::Object(Default::default());
        }
        let mut at = &mut settings;
        let parts: Vec<&str> = path.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            if !at[*part].is_object() {
                at[*part] = Value::Object(Default::default());
            }
            at = &mut at[*part];
        }
        at[parts[parts.len() - 1]] = value;
    }
    settings
}
