//! Incoming LSP server message, request, notification, and response handlers.

use std::collections::BTreeSet;

use merlin_plugin::{
    ActionKind, CodeAction, Completion, DetailedDiagnostic, Diagnostic, FoldRange, InlayHint,
    Location, LogLevel, Position, ProjectSymbol, RenameInfo, Severity, Task, TaskGroup, TokenLayer,
    host,
};
use serde_json::{Value, json};

use crate::Lsp;
use crate::features::formatted;
use crate::protocol::{
    debug_targets, file_edits, flatten_symbols, hover_text, inlay_hint, path_of, position,
    runnables, semantic_tokens, signature, symbol_kind,
};
use crate::server::{Request, Status};

impl Lsp {
    pub(crate) fn on_message(&mut self, program: &str, message: Value) {
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
                let items = params["items"].as_array();
                let list: Vec<Value> = items
                    .into_iter()
                    .flatten()
                    .map(|item| {
                        let Some(section) = item["section"].as_str().filter(|s| !s.is_empty())
                        else {
                            return server.settings.clone();
                        };
                        // A server asking for its own name's section, as
                        // rust-analyzer does, gets them all.
                        if section == server.program {
                            return server.settings.clone();
                        }
                        section
                            .split('.')
                            .try_fold(&server.settings, |cur, key| cur.get(key))
                            .cloned()
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                Value::Array(list)
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
        // rust-analyzer's word for a file in no crate it loaded.
        if list.iter().any(|d| d["code"] == "unlinked-file") {
            server.add_folder_for(path);
        }
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
        let detailed: Vec<DetailedDiagnostic> = list
            .iter()
            .zip(diagnostics)
            .map(|(d, diagnostic)| DetailedDiagnostic {
                diagnostic,
                source: d["source"].as_str().unwrap_or_default().to_string(),
                // A code is a string or a number.
                code: match &d["code"] {
                    Value::String(code) => code.clone(),
                    Value::Number(code) => code.to_string(),
                    _ => String::new(),
                },
                link: d["codeDescription"]["href"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect();
        host::set_detailed_diagnostics(path, &detailed);
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
            // An error the server shows is a notice; the rest of what it
            // says goes to the plugin's Output channel.
            "window/showMessage" | "window/logMessage" => {
                let text = format!(
                    "{program}: {}",
                    params["message"].as_str().unwrap_or_default()
                );
                let level = match params["type"].as_u64() {
                    Some(1) => LogLevel::Error,
                    Some(2) => LogLevel::Warn,
                    Some(3) => LogLevel::Info,
                    _ => LogLevel::Debug,
                };
                if method == "window/showMessage" && level == LogLevel::Error {
                    host::notice(level, &text, &[]);
                } else {
                    host::log(level, &text);
                }
            }
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
            // A save waits for its formatting, so it hears either way.
            match &request {
                Request::Format { path } => formatted(path, &Value::Null),
                Request::FormatPart { batch } => self.part_formatted(*batch, &Value::Null),
                // The rename box waits to hear, and says why.
                Request::PrepareRename | Request::References { .. } => {
                    let why = error["message"].as_str().unwrap_or("it can't be renamed");
                    return host::show_rename(Err(&format!("Can't rename this: {why}")));
                }
                // The palette waits to hear, quietly.
                Request::ProjectSymbols { query } => {
                    return host::show_project_symbols(query, &[]);
                }
                // So does Ctrl over a name.
                Request::DefinitionLink => return host::show_definitions(&[]),
                _ => {}
            }
            // Servers cancel requests that newer edits made pointless.
            let cancelled = matches!(error["code"].as_i64(), Some(-32802..=-32800));
            // What comes by itself as the mouse rests says nothing either.
            let quiet = matches!(
                request,
                Request::Hover { manual: false }
                    | Request::Completion { manual: false }
                    | Request::Diagnostics { .. }
                    | Request::Tokens { .. }
                    | Request::Symbols { .. }
                    | Request::Folds { .. }
                    | Request::InlayHints { .. }
                    | Request::Runnables { .. }
                    | Request::Signature
            );
            let text = format!(
                "{program}: {}",
                error["message"].as_str().unwrap_or("error")
            );
            // Only what the user asked for is a notice; what came by
            // itself is logged.
            if cancelled {
                host::log(LogLevel::Debug, &text);
            } else if quiet {
                host::log(LogLevel::Warn, &text);
            } else {
                host::notice(LogLevel::Warn, &text, &[]);
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
                server.workspace_symbols = offered(&caps["workspaceSymbolProvider"]);
                server.folds = offered(&caps["foldingRangeProvider"]);
                server.inlay_hints = offered(&caps["inlayHintProvider"]);
                server.formats = offered(&caps["documentFormattingProvider"]);
                server.formats_ranges = offered(&caps["documentRangeFormattingProvider"]);
                server.renames = offered(&caps["renameProvider"]);
                server.prepares_renames = caps["renameProvider"]["prepareProvider"] == true;
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
            Request::Hover { manual } => match result.get("contents").map(hover_text) {
                Some(text) if !text.is_empty() => host::show_hover(&text),
                _ if manual => host::log(LogLevel::Info, "Hover: nothing to show here"),
                _ => {}
            },
            Request::Format { path } => formatted(&path, result),
            Request::FormatPart { batch } => {
                let result = result.clone();
                self.part_formatted(batch, &result)
            }
            Request::CodeActions => {
                let list = result.as_array().cloned().unwrap_or_default();
                let mut actions = Vec::new();
                for item in list {
                    // Only actions that say their edit: a bare command
                    // would need the server to run it.
                    let Some(edit) = item.get("edit").filter(|_| item.get("disabled").is_none())
                    else {
                        continue;
                    };
                    self.next_action += 1;
                    let id = self.next_action.to_string();
                    let kind = item["kind"].as_str().unwrap_or_default();
                    actions.push(CodeAction {
                        id: id.clone(),
                        title: item["title"].as_str().unwrap_or_default().to_string(),
                        kind: match kind.split('.').next().unwrap_or_default() {
                            "quickfix" => ActionKind::QuickFix,
                            "refactor" => ActionKind::Refactor,
                            "source" => ActionKind::Source,
                            _ => ActionKind::Other,
                        },
                        preferred: item["isPreferred"].as_bool().unwrap_or(false),
                    });
                    self.actions.insert(id, edit.clone());
                }
                host::show_code_actions(&actions);
            }
            Request::PrepareRename => {
                // A range, a range with a placeholder, or "the word here".
                let range = result.get("range").unwrap_or(result);
                let at = |p: Position| (p.line, p.column);
                let name = range
                    .get("start")
                    .map(|start| (at(position(start)), at(position(&range["end"]))));
                if result.is_null() {
                    return host::show_rename(Err("You can't rename this"));
                }
                self.find_references(program, name);
            }
            Request::References { name } => {
                let list = result.as_array().cloned().unwrap_or_default();
                let Some((path, cursor)) = self.rename_at.clone() else {
                    return;
                };
                let mut files = BTreeSet::new();
                let mut here = None;
                for location in &list {
                    let file = location["uri"].as_str().and_then(path_of);
                    let at = |p: Position| (p.line, p.column);
                    let (start, end) = (
                        at(position(&location["range"]["start"])),
                        at(position(&location["range"]["end"])),
                    );
                    if file.as_deref() == Some(path.as_str()) && (start..=end).contains(&cursor) {
                        here = Some((start, end));
                    }
                    files.insert(file);
                }
                let Some((start, end)) = name.or(here) else {
                    return host::show_rename(Err("There's no name here to rename"));
                };
                let position = |(line, column)| Position { line, column };
                host::show_rename(Ok(&RenameInfo {
                    start: position(start),
                    end: position(end),
                    places: list.len().max(1) as u32,
                    files: files.len().max(1) as u32,
                }));
            }
            Request::ProjectSymbols { query } => {
                let symbols: Vec<ProjectSymbol> = result
                    .as_array()
                    .into_iter()
                    .flatten()
                    .take(200)
                    .filter_map(|s| {
                        let location = &s["location"];
                        Some(ProjectSymbol {
                            name: s["name"].as_str()?.to_string(),
                            kind: symbol_kind(s["kind"].as_u64().unwrap_or(0)).into(),
                            container: s["containerName"].as_str().unwrap_or_default().to_string(),
                            path: path_of(location["uri"].as_str()?)?,
                            start: position(&location["range"]["start"]),
                        })
                    })
                    .collect();
                host::show_project_symbols(&query, &symbols);
            }
            Request::Peek => {
                let places: Vec<Location> = result
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| {
                        Some(Location {
                            path: path_of(l["uri"].as_str()?)?,
                            start: position(&l["range"]["start"]),
                            end: position(&l["range"]["end"]),
                        })
                    })
                    .collect();
                host::show_references(&places);
            }
            Request::Rename { review } => {
                let edits = file_edits(result);
                if edits.is_empty() {
                    return host::notice(LogLevel::Info, "That rename changes nothing", &[]);
                }
                if review {
                    host::show_rename_review(&edits);
                } else if let Err(err) = host::apply_edits(&edits) {
                    host::notice(LogLevel::Error, &err, &[]);
                }
            }
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
                    None => host::log(LogLevel::Info, "Go to Definition: none found"),
                }
            }
            Request::DefinitionLink => {
                let list = match result {
                    Value::Array(list) => list.iter().collect(),
                    Value::Null => Vec::new(),
                    one => vec![one],
                };
                // A Location, or a LocationLink with the name's own range.
                let places: Vec<Location> = list
                    .into_iter()
                    .filter_map(|l| {
                        let uri = l.get("uri").or(l.get("targetUri"))?.as_str()?;
                        let range = l.get("range").or(l.get("targetSelectionRange"))?;
                        Some(Location {
                            path: path_of(uri)?,
                            start: position(&range["start"]),
                            end: position(&range["end"]),
                        })
                    })
                    .collect();
                host::show_definitions(&places);
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
            Request::Runnables { path, version } => {
                if server.opened.get(&path) == Some(&version)
                    && let Some(list) = result.as_array()
                {
                    let tasks = runnables(list);
                    let before = server.tasks.len();
                    server.tasks.extend(tasks);
                    if server.tasks.len() != before {
                        let tasks: Vec<Task> = server
                            .tasks
                            .iter()
                            .map(|(name, (command, detail))| Task {
                                name: name.clone(),
                                command: Some(command.clone()),
                                group: TaskGroup::Build,
                                detail: detail.clone(),
                            })
                            .collect();
                        host::set_tasks(program, &tasks);
                        host::set_debug_targets(&debug_targets(&server.tasks));
                    }
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
}
