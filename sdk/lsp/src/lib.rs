//! Shared Language Server Protocol client for Merlin language plugins.
//!
//! Can be embedded into a single-language plugin (`Lsp::new("Rust", &[RUST_SERVER])`)
//! or used standalone across all configured servers (`Lsp::default()`).

mod features;
mod handlers;
mod protocol;
mod server;
mod spec;

use std::collections::{BTreeMap, BTreeSet};

use merlin_plugin::{
    BufferEvent, Command, Node, Panel, Plugin, Position, ProcessEvent, Token, Tone, host,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::features::Formatting;
use crate::protocol::{file_edits, uri};
use crate::server::{Problem, Request, Server, Status};
pub use crate::spec::{
    CPP_SERVER, GO_SERVER, JS_SERVER, PYTHON_SERVER, RUST_SERVER, SERVERS, Spec,
};

#[derive(Serialize, Deserialize)]
pub struct Lsp {
    #[serde(skip, default = "default_title")]
    pub(crate) title: &'static str,
    #[serde(skip, default = "default_specs")]
    pub(crate) specs: &'static [Spec],
    pub(crate) servers: Vec<Server>,
    pub(crate) signature: bool,
    pub(crate) problems: BTreeMap<String, Vec<Problem>>,
    /// Code actions most recently offered to the picker, by id, with the
    /// workspace edit each applies.
    pub(crate) actions: BTreeMap<String, Value>,
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
}

fn default_title() -> &'static str {
    "Language Servers"
}

fn default_specs() -> &'static [Spec] {
    SERVERS
}

impl Default for Lsp {
    fn default() -> Self {
        Self::new("Language Servers", SERVERS)
    }
}

impl Lsp {
    pub fn new(title: &'static str, specs: &'static [Spec]) -> Self {
        Self {
            title,
            specs,
            servers: Vec::new(),
            signature: false,
            problems: BTreeMap::new(),
            actions: BTreeMap::new(),
            next_action: 0,
            batches: BTreeMap::new(),
            next_batch: 0,
            warned: BTreeSet::new(),
            rename_at: None,
        }
    }

    pub fn commands(&self) -> Vec<Command> {
        let prefix = self.title;
        vec![
            Command::new("hover", format!("{prefix}: Show Hover")).key("alt+k"),
            Command::new("definition", format!("{prefix}: Go to Definition")).key("alt+d"),
            Command::new("complete", format!("{prefix}: Complete")).key("ctrl+space"),
            Command::new("problems", format!("{prefix}: Show Problems")).key("alt+p"),
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
        ]
    }

    pub fn run_command(&mut self, id: &str) {
        match id {
            "hover" => self.ask("textDocument/hover", Request::Hover { manual: true }),
            // The editor's hover as the mouse rests, which it doesn't list.
            "hover-rest" => self.ask("textDocument/hover", Request::Hover { manual: false }),
            "definition" => self.ask("textDocument/definition", Request::Definition),
            // The editor's Ctrl over a name, which it doesn't list.
            "definition-link" => self.ask("textDocument/definition", Request::DefinitionLink),
            "code-actions" => self.ask("textDocument/codeAction", Request::CodeActions),
            "format" => self.format(Formatting::File, false),
            "format-on-save" => self.format(Formatting::File, true),
            "format-selection" => self.format(Formatting::Selection, false),
            "format-changed" => self.format(Formatting::Changed, true),
            "rename" => self.prepare_rename(),
            "references" => self.ask("textDocument/references", Request::Peek),
            // The palette's symbol search, as the manifest's `symbols`.
            "project-symbols" => self.project_symbols(),
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
        let Some(edit) = self.actions.get(item) else {
            return host::notify("That code action is out of date: try again");
        };
        let edits = file_edits(edit);
        if edits.is_empty() {
            return host::notify("That code action changes nothing");
        }
        if let Err(err) = host::apply_edits(&edits) {
            host::notify(&err);
        }
    }

    pub fn buffer(&mut self, event: BufferEvent) {
        if matches!(event, BufferEvent::Changed | BufferEvent::Switched) {
            self.actions.clear();
        }
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

    pub fn panel(&mut self) -> Option<Panel> {
        let mut panel = Panel::new(self.title);
        if self.servers.is_empty() {
            let names: Vec<&str> = self.specs.iter().map(|s| s.program).collect();
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

    pub fn click(&mut self, button: &str) {
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
    }

    fn submit(&mut self, input: &str, text: &str) {
        Lsp::submit(self, input, text);
    }

    fn pick(&mut self, picker: &str, item: &str) {
        Lsp::pick(self, picker, item);
    }

    fn buffer(&mut self, event: BufferEvent) {
        Lsp::buffer(self, event);
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        Lsp::process(self, process, event);
    }

    fn panel(&mut self) -> Option<Panel> {
        Lsp::panel(self)
    }

    fn click(&mut self, button: &str) {
        Lsp::click(self, button);
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

    fn terminal(&mut self, id: &str, output: &str, code: Option<i32>) {
        self.inner.terminal(id, output, code);
    }

    fn buffer(&mut self, event: BufferEvent) {
        self.inner.buffer(event.clone());
        self.lsp.buffer(event);
    }

    fn process(&mut self, process: u32, event: ProcessEvent) {
        self.inner.process(process, event.clone());
        self.lsp.process(process, event);
    }

    /// The inner plugin's panel, if it has one, over the client's.
    fn panel(&mut self) -> Option<Panel> {
        let lsp = self.lsp.panel();
        match (self.inner.panel(), lsp) {
            (Some(mut inner), Some(lsp)) => {
                inner = inner.divider();
                inner.nodes.extend(lsp.nodes);
                Some(inner)
            }
            (inner, lsp) => inner.or(lsp),
        }
    }

    fn click(&mut self, button: &str) {
        self.inner.click(button);
        self.lsp.click(button);
    }

    fn colour_text(&mut self, language: &str, text: &str) -> Vec<Token> {
        self.inner.colour_text(language, text)
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
