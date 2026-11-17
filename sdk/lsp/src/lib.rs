//! Shared Language Server Protocol client for Merlin language plugins.
//!
//! A language plugin brings its own servers and embeds it:
//! `WithLsp::new(Rust::new(), "Rust", &[RUST_SERVER])`.

mod check;
mod features;
mod handlers;
mod hierarchy;
mod protocol;
mod server;
mod spec;
mod symbols;

use std::collections::{BTreeMap, BTreeSet};

use merlin_plugin::{
    BufferEvent, Button, Command, LogLevel, PageSize, Panel, Plugin, ProcessEvent, RgbaPicture,
    StatusItem, TextBox, Tile, Token, host,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::features::Formatting;
use crate::protocol::{file_edits, uri};
use crate::server::{Request, Server, Status};
pub use crate::spec::{Scanned, Spec};

#[derive(Serialize, Deserialize)]
pub struct Lsp {
    // The plugin's own, put back after a restore.
    #[serde(skip)]
    pub(crate) title: &'static str,
    #[serde(skip)]
    pub(crate) specs: &'static [Spec],
    pub(crate) servers: Vec<Server>,
    pub(crate) signature: bool,
    /// What `report_status` sent last, so it sends only changes.
    #[serde(skip)]
    pub(crate) reported: String,
    /// Code actions most recently offered to the picker, by id, with the
    /// workspace edit each applies.
    /// The code actions last shown, by id: the server each came from,
    /// and the action as it sent it.
    pub(crate) actions: BTreeMap<String, (String, Value)>,
    pub(crate) next_action: u64,
    /// Multi-range format requests still waiting for all their ranges:
    /// batch id -> `(path, ranges left, edits so far)`.
    #[serde(skip)]
    pub(crate) batches: BTreeMap<u64, (String, usize, Vec<Value>)>,
    #[serde(skip)]
    pub(crate) next_batch: u64,
    /// `(path, why)` pairs a save has already warned about.
    pub(crate) warned: BTreeSet<String>,
    /// The file and `(line, column)` of the name the rename box is open on.
    pub(crate) rename_at: Option<(String, (u32, u32))>,
    /// The Hierarchy view's items.
    #[serde(default)]
    pub(crate) hierarchy: hierarchy::Hierarchy,
    /// Problems' Check Project while it runs, and the files it closed
    /// whose empty publish is still to come.
    #[serde(skip)]
    pub(crate) check: Option<check::ProjectCheck>,
    #[serde(skip)]
    pub(crate) closing: BTreeSet<String>,
    /// Check Project has run, so a header's save runs it again.
    #[serde(default)]
    pub(crate) checked_once: bool,
}

pub use check::flags_line;

impl Lsp {
    pub fn new(title: &'static str, specs: &'static [Spec]) -> Self {
        Self {
            title,
            specs,
            servers: Vec::new(),
            signature: false,
            reported: String::new(),
            actions: BTreeMap::new(),
            next_action: 0,
            batches: BTreeMap::new(),
            next_batch: 0,
            warned: BTreeSet::new(),
            rename_at: None,
            hierarchy: hierarchy::Hierarchy::default(),
            check: None,
            closing: BTreeSet::new(),
            checked_once: false,
        }
    }

    fn go_to(&mut self, method: &str, what: &str) {
        self.ask(method, Request::GoTo { what: what.into() });
    }

    /// Opens the header for the source file, or the source for the header,
    /// as clangd finds it.
    fn switch_source_header(&mut self) {
        let Some((path, spec, language)) = self.active() else {
            return;
        };
        self.catch_up(&path, spec, language);
        let server = self.ensure(spec);
        server.request(
            "textDocument/switchSourceHeader",
            json!({"uri": uri(&path)}),
            Request::SwitchSourceHeader,
        );
    }

    pub fn commands(&self) -> Vec<Command> {
        let prefix = self.title;
        let clangd = self.specs.iter().any(|s| s.program == "clangd");
        let mut commands = vec![
            Command::new("hover", format!("{prefix}: Show Hover")).key("alt+k"),
            Command::new("definition", format!("{prefix}: Go to Definition")).key("alt+d"),
            Command::new("declaration", format!("{prefix}: Go to Declaration")),
            Command::new("implementation", format!("{prefix}: Go to Implementations")),
            Command::new(
                "type-definition",
                format!("{prefix}: Go to Type Definition"),
            ),
            Command::new("complete", format!("{prefix}: Complete")).key("ctrl+space"),
            Command::new("restart", format!("{prefix}: Restart Servers")),
            // Ctrl+. in the editor, which offers them its own way.
            Command::new("code-actions", format!("{prefix}: Code Actions")),
            // The editor's Ctrl+Shift+I, Ctrl+K Ctrl+F and saves.
            Command::new("format", format!("{prefix}: Format the File")),
            Command::new("format-on-save", format!("{prefix}: Format for Saving")),
            Command::new(
                "format-selection",
                format!("{prefix}: Format the Selection"),
            ),
            Command::new(
                "format-changed",
                format!("{prefix}: Format the Changed Lines"),
            ),
            // The editor's F2, which shows its box over the name.
            Command::new("rename", format!("{prefix}: Rename")),
            // The editor's Shift+F12, which peeks at them.
            Command::new("references", format!("{prefix}: Find References")),
            // The editor's Shift+Alt+H, and its Hierarchy view's.
            Command::new("call-hierarchy", format!("{prefix}: Show Call Hierarchy")),
            Command::new("type-hierarchy", format!("{prefix}: Show Type Hierarchy")),
        ];
        // Problems' Check Project, for a server that reports only on the
        // files open.
        if self.checks_projects() {
            commands.push(Command::new(
                "check-project",
                format!("{prefix}: Check Whole Project"),
            ));
        }
        // clangd's own, with the key its VS Code extension gives it.
        if clangd {
            commands.push(Command::new(
                "switch-source-header",
                format!("{prefix}: Switch Header/Source"),
            ));
        }
        commands
    }

    pub fn run_command(&mut self, id: &str) {
        match id {
            "hover" => self.ask("textDocument/hover", Request::Hover { manual: true }),
            // The editor's hover as the mouse rests, which it doesn't list.
            "hover-rest" => self.ask("textDocument/hover", Request::Hover { manual: false }),
            "definition" => self.ask("textDocument/definition", Request::Definition),
            "declaration" => self.go_to("textDocument/declaration", "Declaration"),
            "implementation" => self.go_to("textDocument/implementation", "Implementations"),
            "type-definition" => self.go_to("textDocument/typeDefinition", "Type Definition"),
            "switch-source-header" => self.switch_source_header(),
            // The editor's Ctrl over a name, which it doesn't list.
            "definition-link" => self.ask("textDocument/definition", Request::DefinitionLink),
            "code-actions" => self.ask("textDocument/codeAction", Request::CodeActions),
            "format" => self.format(Formatting::File, false),
            "format-on-save" => self.format(Formatting::File, true),
            "format-selection" => self.format(Formatting::Selection, false),
            "format-changed" => self.format(Formatting::Changed, true),
            "rename" => self.prepare_rename(),
            "references" => self.ask("textDocument/references", Request::Peek),
            id if self.hierarchy_command(id) => {}
            "check-project" => self.check_project(),
            // The editor's, as the cursor comes to a name; it doesn't list it.
            "highlight" => self.highlight(),
            // The palette's symbol search, as the manifest's `symbols`.
            "project-symbols" => self.project_symbols(),
            "complete" => self.ask(
                "textDocument/completion",
                Request::Completion { manual: true },
            ),
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

    /// A setting a server gets changed: those servers start again with it.
    pub fn setting_changed(&mut self, key: &str) {
        self.restart_where(key, |s| {
            s.options.iter().chain(s.flags).any(|(k, _)| *k == key) || s.restarts.contains(&key)
        });
    }

    /// A shared value a server is set up from changed: those servers start
    /// again with it.
    pub fn shared_changed(&mut self, key: &str) {
        self.restart_where(key, |s| s.shared.contains(&key));
    }

    /// Starts again the running servers whose spec `uses` says, because of
    /// what `key` names.
    fn restart_where(&mut self, key: &str, uses: impl Fn(&Spec) -> bool) {
        let affected: Vec<&str> = self
            .specs
            .iter()
            .filter(|s| uses(s))
            .map(|s| s.program)
            .collect();
        if affected.is_empty() {
            return;
        }
        let mut stopped = false;
        self.servers.retain(|server| {
            if !affected.contains(&server.program.as_str()) {
                return true;
            }
            if let Some(process) = server.process {
                host::kill(process);
            }
            stopped = true;
            false
        });
        if stopped {
            host::log(
                LogLevel::Info,
                &format!("Restarting {} with the new {key}", affected.join(", ")),
            );
            self.buffer(BufferEvent::Switched);
        }
    }

    pub fn submit(&mut self, input: &str, text: &str) {
        let review = match input {
            "rename" => false,
            "rename-review" => true,
            _ => return,
        };
        let Some((path, at)) = self.rename_at.take() else {
            return;
        };
        let Some((_, spec, _)) = self.active() else {
            return;
        };
        let server = self.ensure(spec);
        server.request(
            "textDocument/rename",
            json!({
                "textDocument": {"uri": uri(&path)},
                "position": {"line": at.0, "character": at.1},
                "newName": text,
            }),
            Request::Rename { review },
        );
    }

    pub fn pick(&mut self, picker: &str, item: &str) {
        if picker != "code-actions" {
            return;
        }
        let Some((program, action)) = self.actions.get(item).cloned() else {
            return host::notice(
                LogLevel::Info,
                "That code action is out of date: try again",
                &[],
            );
        };
        // An action says its edit, or a command the server runs, which
        // sends its edit back; or both, the edit first. A bare command has
        // the command's name as a string.
        let command = match &action["command"] {
            Value::String(_) => Some(action.clone()),
            c if c.is_object() => Some(c.clone()),
            _ => None,
        };
        if let Some(edit) = action.get("edit") {
            let edits = file_edits(edit);
            if edits.is_empty() && command.is_none() {
                return host::notice(LogLevel::Info, "That code action changes nothing", &[]);
            }
            if let Err(err) = host::apply_edits(&edits) {
                return host::notice(LogLevel::Error, &err, &[]);
            }
        }
        if let Some(command) = command
            && let Some(server) = self.server(&program)
        {
            let mut params = json!({"command": command["command"]});
            if let Some(arguments) = command.get("arguments") {
                params["arguments"] = arguments.clone();
            }
            server.request("workspace/executeCommand", params, Request::ExecuteCommand);
        }
    }

    pub fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Changed | BufferEvent::Switched) {
            self.actions.clear();
        }
        let Some((path, spec, language)) = self.active() else {
            return;
        };
        // A header saved can break the files that include it: once the
        // project has been checked, it's checked again.
        if let BufferEvent::Saved(saved) = &event {
            let header = saved.rsplit_once('.').is_some_and(|(_, e)| {
                matches!(
                    e,
                    "h" | "hh" | "hpp" | "hxx" | "h++" | "inl" | "ipp" | "tpp"
                )
            });
            if header && self.checked_once && self.check.is_none() && self.checks_projects() {
                self.check_project();
            }
        }
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
        let changes = self.catch_up(&path, spec, language);
        if matches!(event, BufferEvent::Changed) {
            self.typed(changes);
        }
    }

    pub fn process(&mut self, process: u32, event: ProcessEvent) {
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
            // clangd's log says when it guesses a file's flags.
            ProcessEvent::Stderr(line) => {
                if let Some((path, guessed)) = flags_line(&line)
                    && let Some(server) = self.server(&program)
                {
                    if guessed {
                        server.guessed.insert(path.to_string());
                    } else {
                        server.guessed.remove(path);
                    }
                }
            }
            ProcessEvent::Exit(code) => {
                if let Some(server) = self.server(&program) {
                    server.process = None;
                    server.opened.clear();
                    let code = code.map_or("a signal".into(), |c| format!("code {c}"));
                    server.status = Status::Failed(format!("exited ({code})"));
                    host::notice(
                        LogLevel::Error,
                        &format!("{program} stopped: it exited with {code}"),
                        &[Button {
                            id: "command:restart".into(),
                            label: "Restart".into(),
                        }],
                    );
                }
            }
        }
    }

    /// How each server is doing, for the language card, sent when it
    /// changes.
    pub fn report_status(&mut self) {
        let open = self.active().map(|(path, _, _)| path);
        let lines: Vec<StatusItem> = self
            .servers
            .iter()
            .filter_map(|server| {
                let spec = self.specs.iter().find(|s| s.program == server.program)?;
                let guessing = open.as_ref().is_some_and(|p| server.guessed.contains(p));
                Some(server.status_item(spec, guessing))
            })
            .collect();
        let sent = format!("{lines:?}");
        if sent != self.reported {
            self.reported = sent;
            host::set_status("servers", &lines);
        }
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(self).ok()
    }

    pub fn restore_state(&mut self, state: Vec<u8>) {
        let (title, specs) = (self.title, self.specs);
        if let Ok(mut lsp) = serde_json::from_slice::<Self>(&state) {
            lsp.title = title;
            lsp.specs = specs;
            *self = lsp;
        }
    }
}

impl Plugin for Lsp {
    fn commands(&self) -> Vec<Command> {
        Lsp::commands(self)
    }

    fn run_command(&mut self, id: &str) {
        Lsp::run_command(self, id);
        self.report_status();
    }

    fn submit(&mut self, input: &str, text: &str) {
        Lsp::submit(self, input, text);
    }

    fn pick(&mut self, picker: &str, item: &str) {
        Lsp::pick(self, picker, item);
    }

    fn setting_changed(&mut self, key: &str) {
        Lsp::setting_changed(self, key);
        self.report_status();
    }

    fn shared_changed(&mut self, key: &str) {
        Lsp::shared_changed(self, key);
        self.report_status();
    }

    fn buffer(&mut self, event: BufferEvent) {
        Lsp::buffer(self, event);
        self.report_status();
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        Lsp::process(self, process, event);
        self.report_status();
    }

    fn cancel(&mut self, id: &str) {
        if id == "check-project" {
            self.stop_check();
        }
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        Lsp::save_state(self)
    }

    fn restore_state(&mut self, state: Vec<u8>) {
        Lsp::restore_state(self, state);
    }
}

/// Composes a syntax/language plugin `P` (such as `merlin_grammar::Grammar` or
/// a custom lexer) with an [`Lsp`] client so a full language plugin requires
/// no method-forwarding boilerplate.
pub struct WithLsp<P> {
    pub inner: P,
    pub lsp: Lsp,
}

impl<P> WithLsp<P> {
    pub fn new(inner: P, title: &'static str, specs: &'static [Spec]) -> Self {
        Self {
            inner,
            lsp: Lsp::new(title, specs),
        }
    }
}

impl<P: Plugin> Plugin for WithLsp<P> {
    fn commands(&self) -> Vec<Command> {
        let mut cmds = self.inner.commands();
        cmds.extend(self.lsp.commands());
        cmds
    }

    /// A command either lists goes to it alone; one neither lists, such as
    /// the editor's `find-tests`, to both.
    fn run_command(&mut self, id: &str) {
        let lists = |commands: Vec<Command>| commands.iter().any(|c| c.id == id);
        let (inner, lsp) = (lists(self.inner.commands()), lists(self.lsp.commands()));
        if inner || !lsp {
            self.inner.run_command(id);
        }
        if lsp || !inner {
            self.lsp.run_command(id);
            self.lsp.report_status();
        }
    }

    fn submit(&mut self, input: &str, text: &str) {
        self.inner.submit(input, text);
        self.lsp.submit(input, text);
    }

    fn pick(&mut self, picker: &str, item: &str) {
        self.inner.pick(picker, item);
        self.lsp.pick(picker, item);
    }

    fn review(&mut self, id: &str, accepted: bool) {
        self.inner.review(id, accepted);
    }

    fn setting_changed(&mut self, key: &str) {
        self.inner.setting_changed(key);
        self.lsp.setting_changed(key);
        self.lsp.report_status();
    }

    fn shared_changed(&mut self, key: &str) {
        self.inner.shared_changed(key);
        self.lsp.shared_changed(key);
        self.lsp.report_status();
    }

    fn terminal(&mut self, id: &str, output: &str, code: Option<i32>) {
        self.inner.terminal(id, output, code);
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.inner.buffer(event.clone());
        self.lsp.buffer(event);
        self.lsp.report_status();
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        self.inner.process(process, event.clone());
        self.lsp.process(process, event);
        self.lsp.report_status();
    }

    /// The inner plugin's panel, if it has one, over the client's.
    /// The inner plugin's panel: the client's servers are in the
    /// language card instead.
    fn view(&mut self, id: &str) -> Option<Panel> {
        self.inner.view(id)
    }

    fn click(&mut self, button: &str) {
        self.inner.click(button);
    }

    fn cancel(&mut self, id: &str) {
        self.inner.cancel(id);
        if id == "check-project" {
            self.lsp.stop_check();
        }
    }

    fn colour_text(&mut self, language: &str, text: &str) -> Vec<Token> {
        self.inner.colour_text(language, text)
    }

    fn open_document(
        &mut self,
        surface: u32,
        editor: &str,
        path: &str,
        data: Vec<u8>,
    ) -> Result<Vec<PageSize>, String> {
        self.inner.open_document(surface, editor, path, data)
    }

    fn render_tile(&mut self, surface: u32, tile: Tile) -> Result<RgbaPicture, String> {
        self.inner.render_tile(surface, tile)
    }

    fn page_text(&mut self, surface: u32, page: u32) -> Vec<TextBox> {
        self.inner.page_text(surface, page)
    }

    fn close_document(&mut self, surface: u32) {
        self.inner.close_document(surface)
    }

    /// Both states, as a pair.
    fn save_state(&self) -> Option<Vec<u8>> {
        let pair = (self.lsp.save_state(), self.inner.save_state());
        serde_json::to_vec(&pair).ok()
    }

    /// A state saved before the pair was the client's alone.
    fn restore_state(&mut self, state: Vec<u8>) {
        match serde_json::from_slice::<(Option<Vec<u8>>, Option<Vec<u8>>)>(&state) {
            Ok((lsp, inner)) => {
                if let Some(lsp) = lsp {
                    self.lsp.restore_state(lsp);
                }
                if let Some(inner) = inner {
                    self.inner.restore_state(inner);
                }
            }
            Err(_) => self.lsp.restore_state(state),
        }
    }
}
