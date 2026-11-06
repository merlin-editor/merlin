//! Loads plugins as WebAssembly components and calls them.
//!
//! A plugin can only call the functions in `wit/merlin.wit`, plus a WASI
//! context with no files, environment or network. It never touches editor
//! state: it reads a [`Snapshot`] taken before the call, and what it asks for
//! comes back as [`Event`]s for the editor to apply. Each call has a fuel
//! budget, and a plugin that traps or runs out is disabled, not the editor.
//!
//! The one way out of the sandbox is spawning a program the manifest lists.
//! Its output is read on background threads and delivered to the plugin by
//! [`PluginHost::poll_processes`], between editor events.
//!
//! The first time a plugin asks for permissions, it shows a card in its
//! panel instead of loading, and loads once the user allows it. The grant
//! is remembered until the plugin asks for more.

mod process;
mod secrets;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow, bail};
use merlin_core::{
    Button, Command, Completion, Diagnostic, Event, Node, Panel, PickItem, Severity, Tone,
};
use serde::{Deserialize, Serialize};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use process::{Output, Processes};
use secrets::Secrets;

mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "plugin" });
}

use bindings::Plugin as Bindings;
use bindings::merlin::plugin::{host, types};

/// The API version this host implements. A manifest names the version it was
/// built for and loads only on an exact match while the API is 0.x.
pub const API_VERSION: &str = "0.8";

/// Roughly a few hundred million instructions per call.
const FUEL_PER_CALL: u64 = 500_000_000;

/// Process messages delivered per poll, so a chatty process can't starve
/// the editor of input.
const MESSAGES_PER_POLL: usize = 256;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub title: String,
    pub api: String,
    /// The component file, relative to the plugin folder. A plugin that
    /// only adds data, such as colour themes, has none.
    #[serde(default)]
    pub component: Option<PathBuf>,
    #[serde(default)]
    pub permissions: Permissions,
    /// Default settings, which the user's and the workspace's
    /// `[plugins.<name>]` override.
    #[serde(default)]
    pub settings: toml::Table,
    /// Languages the plugin adds, as a VS Code extension contributes them.
    #[serde(default)]
    pub languages: Vec<ManifestLanguage>,
    /// Colour themes the plugin adds.
    #[serde(default)]
    pub themes: Vec<ManifestTheme>,
    /// Theme packs the plugin adds: icons, layout, fonts and colours, any
    /// of them, layered over a theme. A theme is a pack too.
    #[serde(default)]
    pub packs: Vec<ManifestTheme>,
    /// Debuggers the plugin brings, for languages it adds or others.
    #[serde(default)]
    pub debuggers: Vec<ManifestDebugger>,
}

/// A debugger, as `[[debuggers]]` in a manifest declares it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestDebugger {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    /// The program and its arguments. Empty for one the plugin drives
    /// itself.
    #[serde(default)]
    pub command: Vec<String>,
    /// How to install it, as a command to copy.
    #[serde(default)]
    pub install: String,
    /// Launch settings sent with every start.
    #[serde(default)]
    pub launch: toml::Table,
    /// The plugin that speaks to it: `dap` unless it says.
    #[serde(default)]
    pub driver: Option<String>,
    /// More to stop on, as functions to break in.
    #[serde(default)]
    pub stop_on: Vec<ManifestStop>,
}

/// Something more a debugger can stop on: a function to break in, such as
/// `rust_panic` for Rust's panics.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestStop {
    pub id: String,
    pub label: String,
    /// Every name it may have, as versions of the language differ.
    pub functions: Vec<String>,
    #[serde(default)]
    pub default: bool,
}

impl ManifestDebugger {
    fn to_core(&self, plugin: &str) -> merlin_core::Debugger {
        merlin_core::Debugger {
            id: self.id.clone(),
            name: self.name.clone(),
            plugin: plugin.to_string(),
            languages: self.languages.clone(),
            command: self.command.clone(),
            install: self.install.clone(),
            launch: serde_json::to_string(&self.launch).unwrap_or_else(|_| "{}".into()),
            stops: self
                .stop_on
                .iter()
                .map(|s| merlin_core::FunctionStop {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    functions: s.functions.clone(),
                    on: s.default,
                })
                .collect(),
            driver: self.driver.clone().unwrap_or_else(|| {
                if self.command.is_empty() {
                    plugin.to_string()
                } else {
                    merlin_core::DAP.to_string()
                }
            }),
        }
    }
}

/// The events that say what a manifest adds: its languages and its
/// debuggers.
fn manifest_events(manifest: &Manifest) -> Vec<Event> {
    let mut events = Vec::new();
    let plugin = manifest.name.clone();
    let languages: Vec<_> = manifest
        .languages
        .iter()
        .map(ManifestLanguage::to_core)
        .collect();
    if !languages.is_empty() {
        events.push(Event::RegisterLanguages {
            plugin: plugin.clone(),
            languages,
        });
    }
    if !manifest.debuggers.is_empty() {
        events.push(Event::RegisterDebuggers {
            debuggers: manifest
                .debuggers
                .iter()
                .map(|d| d.to_core(&plugin))
                .collect(),
            plugin,
        });
    }
    events
}

/// A theme or pack: its name in the pickers, and its file, relative to the
/// plugin folder.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestTheme {
    pub label: String,
    pub file: PathBuf,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ManifestLanguage {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub line_comment: Option<String>,
    #[serde(default)]
    pub block_comment: Option<(String, String)>,
}

impl ManifestLanguage {
    fn to_core(&self) -> merlin_core::lang::Language {
        merlin_core::lang::Language {
            id: self.id.clone(),
            name: self.name.clone(),
            extensions: self
                .extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_string())
                .collect(),
            line_comment: self.line_comment.clone(),
            block_comment: self.block_comment.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Permissions {
    #[serde(default)]
    pub editor_read: bool,
    #[serde(default)]
    pub editor_write: bool,
    /// Programs the plugin may start, by name as found on `PATH`.
    #[serde(default)]
    pub processes: Vec<String>,
    /// Drives debuggers: starts the ones plugins declare when the user
    /// starts a run, and reports the run.
    #[serde(default)]
    pub debugger: bool,
}

impl Permissions {
    fn is_empty(&self) -> bool {
        *self == Permissions::default()
    }

    /// Whether these allow everything `wanted` asks for.
    fn covers(&self, wanted: &Permissions) -> bool {
        (self.editor_read || !wanted.editor_read)
            && (self.editor_write || !wanted.editor_write)
            && (self.debugger || !wanted.debugger)
            && wanted.processes.iter().all(|p| self.processes.contains(p))
    }
}

/// Permissions the user allowed, by plugin folder.
#[derive(Default, Deserialize, Serialize)]
struct Grants {
    #[serde(default)]
    plugins: BTreeMap<String, Permissions>,
}

pub struct Config {
    /// Where remembered permissions and file-backed secrets are kept.
    pub data_dir: PathBuf,
    /// Ask before granting a plugin's permissions the first time. When off,
    /// they're granted as the manifest says and nothing is remembered.
    pub ask: bool,
    /// Keep secrets in the system keyring when `secret-tool` is installed.
    pub keyring: bool,
}

impl Config {
    /// `$XDG_DATA_HOME/merlin` (`~/.local/share/merlin`), asking, with
    /// the keyring.
    pub fn standard() -> Self {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir);
        Self {
            data_dir: data.join("merlin"),
            ask: true,
            keyring: true,
        }
    }
}

/// A change to the open files, for plugins' `on-buffer`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BufferEvent {
    Switched,
    Changed,
    Saved(String),
}

/// What a plugin call can see of the editor.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub path: Option<String>,
    /// Shared with the editor, not copied: a call that doesn't read it
    /// costs nothing.
    pub text: merlin_core::buffer::TextSnapshot,
    pub root: PathBuf,
    /// Line and UTF-16 column.
    pub cursor: (u32, u32),
    pub selection: Option<Selection>,
    /// The lines changed since git's copy, as first and last line, or
    /// none when the file isn't in git.
    pub changed: Option<Vec<(u32, u32)>>,
}

/// Selected text, and where it starts and ends as lines and UTF-16 columns.
#[derive(Clone, Debug)]
pub struct Selection {
    pub start: (u32, u32),
    pub end: (u32, u32),
    pub text: String,
}

impl Snapshot {
    /// A path a plugin gave, relative to the root when it's inside it, as
    /// the editor names files.
    fn local(&self, path: &str) -> PathBuf {
        let path = Path::new(path);
        path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
    }
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    name: String,
    permissions: Permissions,
    snapshot: Snapshot,
    events: Vec<Event>,
    processes: Processes,
    secrets: Arc<Secrets>,
    /// The plugin's settings, defaults and all.
    settings: toml::Table,
    /// How much memory the plugin's instance has, for its page.
    limits: Limits,
    /// What the editor last asked this plugin to debug, and the
    /// breakpoints, for `debug-start` and `breakpoints`.
    debug_start: Option<merlin_core::DebugStart>,
    breakpoints: Vec<merlin_core::Breakpoint>,
}

/// Counts the memory an instance grows to. It allows any growth: fuel and
/// the user's eyes are the limits.
#[derive(Default)]
struct Limits {
    memory: usize,
}

impl wasmtime::ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.memory = self.memory + desired - current.min(desired);
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

impl State {
    /// A report on a run, for a plugin that drives debuggers.
    fn debug(&mut self, update: merlin_core::DebugUpdate) -> Result<(), String> {
        self.need(self.permissions.debugger, "debugger")?;
        self.events.push(Event::Debug {
            plugin: self.name.clone(),
            update,
        });
        Ok(())
    }

    fn need(&self, granted: bool, permission: &str) -> Result<(), String> {
        if granted {
            Ok(())
        } else {
            Err(format!("permission denied: {permission}"))
        }
    }

    /// A plugin's file edits as the editor's, with workspace paths.
    fn file_edits(&self, edits: Vec<types::FileEdit>) -> Vec<merlin_core::lang::FileEdit> {
        let at = |p: types::Position| (p.line, p.column);
        edits
            .into_iter()
            .map(|f| merlin_core::lang::FileEdit {
                path: self.snapshot.local(&f.path),
                edits: f
                    .edits
                    .into_iter()
                    .map(|e| (at(e.start), at(e.end), e.text))
                    .collect(),
            })
            .collect()
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl types::Host for State {}

impl host::Host for State {
    fn active_text(&mut self) -> Result<String, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        Ok(self.snapshot.text.to_string())
    }

    fn active_version(&mut self) -> Result<u64, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        self.snapshot
            .text
            .version()
            .ok_or_else(|| "no version for this text".to_string())
    }

    fn active_changes(&mut self, since: u64) -> Result<Option<Vec<types::TextChange>>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        let at = |(line, column)| types::Position { line, column };
        Ok(self.snapshot.text.changes_since(since).map(|changes| {
            changes
                .into_iter()
                .map(|c| types::TextChange {
                    start: at(c.start),
                    end: at(c.end),
                    text: c.text,
                })
                .collect()
        }))
    }

    fn active_path(&mut self) -> Result<Option<String>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        Ok(self.snapshot.path.clone())
    }

    fn active_cursor(&mut self) -> Result<types::Position, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        let (line, column) = self.snapshot.cursor;
        Ok(types::Position { line, column })
    }

    fn active_selection(&mut self) -> Result<Option<types::Selection>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        let at = |(line, column)| types::Position { line, column };
        Ok(self.snapshot.selection.clone().map(|s| types::Selection {
            start: at(s.start),
            end: at(s.end),
            text: s.text,
        }))
    }

    fn changed_lines(&mut self) -> Result<Option<Vec<types::LineRange>>, String> {
        self.need(self.permissions.editor_read, "editor-read")?;
        Ok(self.snapshot.changed.as_ref().map(|lines| {
            lines
                .iter()
                .map(|&(start, end)| types::LineRange { start, end })
                .collect()
        }))
    }

    fn show_diff(&mut self, id: String, path: String, old: String, new: String) {
        self.events.push(Event::ShowReview {
            plugin: self.name.clone(),
            id,
            path: self.snapshot.local(&path),
            old,
            new,
        });
    }

    fn close_diff(&mut self, id: String) {
        self.events.push(Event::CloseReview {
            plugin: self.name.clone(),
            id,
        });
    }

    fn set_diagnostics(&mut self, path: String, diagnostics: Vec<types::Diagnostic>) {
        let diagnostics = diagnostics
            .into_iter()
            .map(|d| diagnostic(d, String::new(), String::new(), String::new()))
            .collect();
        self.events.push(Event::SetDiagnostics {
            plugin: self.name.clone(),
            path: self.snapshot.local(&path),
            diagnostics,
        });
    }

    fn set_detailed_diagnostics(
        &mut self,
        path: String,
        diagnostics: Vec<types::DetailedDiagnostic>,
    ) {
        let diagnostics = diagnostics
            .into_iter()
            .map(|d| diagnostic(d.diagnostic, d.source, d.code, d.link))
            .collect();
        self.events.push(Event::SetDiagnostics {
            plugin: self.name.clone(),
            path: self.snapshot.local(&path),
            diagnostics,
        });
    }

    fn set_tokens(
        &mut self,
        path: String,
        layer: types::TokenLayer,
        lines: Option<types::LineRange>,
        tokens: Vec<types::Token>,
    ) {
        self.events.push(Event::SetTokens {
            path: self.snapshot.local(&path),
            layer: match layer {
                types::TokenLayer::Base => merlin_core::layers::Layer::Base,
                types::TokenLayer::Semantic => merlin_core::layers::Layer::Semantic,
            },
            lines: lines.map(|r| (r.start, r.end)),
            tokens: tokens.into_iter().map(plugin_token).collect(),
        });
    }

    fn set_symbols(&mut self, path: String, symbols: Vec<types::DocSymbol>) {
        let at = |p: types::Position| (p.line, p.column);
        self.events.push(Event::SetSymbols {
            path: self.snapshot.local(&path),
            symbols: symbols
                .into_iter()
                .map(|s| merlin_core::lang::PluginSymbol {
                    name: s.name,
                    kind: s.kind,
                    detail: s.detail,
                    start: at(s.start),
                    end: at(s.end),
                    depth: s.depth,
                })
                .collect(),
        });
    }

    fn set_folds(&mut self, path: String, ranges: Vec<types::FoldRange>) {
        self.events.push(Event::SetFolds {
            path: self.snapshot.local(&path),
            ranges: ranges.into_iter().map(|r| (r.first, r.last)).collect(),
        });
    }

    fn set_inlay_hints(&mut self, path: String, hints: Vec<types::InlayHint>) {
        self.events.push(Event::SetInlayHints {
            path: self.snapshot.local(&path),
            hints: hints
                .into_iter()
                .map(|h| merlin_core::InlayHint {
                    at: (h.at.line, h.at.column),
                    label: h.label,
                    kind: match h.kind {
                        types::InlayKind::Type => merlin_core::InlayKind::Type,
                        types::InlayKind::Parameter => merlin_core::InlayKind::Parameter,
                        types::InlayKind::Other => merlin_core::InlayKind::Other,
                    },
                })
                .collect(),
        });
    }

    fn show_signature(&mut self, signature: Option<types::Signature>) {
        self.events.push(Event::ShowSignature(signature.map(|s| {
            merlin_core::Signature {
                label: s.label,
                active: s.active.map(|(a, b)| (a as usize, b as usize)),
                doc: s.doc,
                index: s.index,
                count: s.count,
            }
        })));
    }

    fn show_hover(&mut self, text: String) {
        self.events.push(Event::ShowHover(text));
    }

    fn show_completions(&mut self, items: Vec<types::Completion>) {
        self.events.push(Event::ShowCompletions(
            items
                .into_iter()
                .map(|c| Completion {
                    label: c.label,
                    detail: c.detail,
                    insert: c.insert,
                })
                .collect(),
        ));
    }

    fn open_at(&mut self, path: String, at: types::Position) {
        self.events.push(Event::OpenAt {
            path: self.snapshot.local(&path),
            line: at.line,
            column: at.column,
        });
    }

    fn show_code_actions(&mut self, actions: Vec<types::CodeAction>) {
        use merlin_core::lang::{ActionKind, CodeAction};
        self.events.push(Event::ShowCodeActions(
            actions
                .into_iter()
                .map(|a| CodeAction {
                    id: a.id,
                    title: a.title,
                    kind: match a.kind {
                        types::ActionKind::QuickFix => ActionKind::QuickFix,
                        types::ActionKind::Refactor => ActionKind::Refactor,
                        types::ActionKind::Source => ActionKind::Source,
                        types::ActionKind::Other => ActionKind::Other,
                    },
                    preferred: a.preferred,
                })
                .collect(),
        ));
    }

    fn apply_edits(&mut self, edits: Vec<types::FileEdit>) -> Result<(), String> {
        self.need(self.permissions.editor_write, "editor-write")?;
        let edits = self.file_edits(edits);
        self.events.push(Event::ApplyEdits(edits));
        Ok(())
    }

    fn show_rename(&mut self, info: Result<types::RenameInfo, String>) {
        let at = |p: types::Position| (p.line, p.column);
        self.events.push(Event::ShowRename(info.map(|i| {
            merlin_core::lang::RenameInfo {
                start: at(i.start),
                end: at(i.end),
                places: i.places,
                files: i.files,
            }
        })));
    }

    fn show_rename_review(&mut self, edits: Vec<types::FileEdit>) {
        let edits = self.file_edits(edits);
        self.events.push(Event::ShowRenameReview(edits));
    }

    fn show_references(&mut self, places: Vec<types::Location>) {
        let at = |p: types::Position| (p.line, p.column);
        let places = places
            .into_iter()
            .map(|l| merlin_core::lang::Location {
                path: self.snapshot.local(&l.path),
                start: at(l.start),
                end: at(l.end),
            })
            .collect();
        self.events.push(Event::ShowReferences(places));
    }

    fn insert_text(&mut self, text: String) -> Result<(), String> {
        self.need(self.permissions.editor_write, "editor-write")?;
        self.events.push(Event::InsertText(text));
        Ok(())
    }

    fn workspace_root(&mut self) -> String {
        self.snapshot.root.display().to_string()
    }

    fn set_tasks(&mut self, tasks: Vec<types::Task>) {
        self.events.push(Event::SetTasks {
            plugin: self.name.clone(),
            tasks: tasks
                .into_iter()
                .map(|t| merlin_core::Task {
                    name: t.name,
                    command: t.command,
                    group: match t.group {
                        types::TaskGroup::Test => merlin_core::TaskGroup::Test,
                        types::TaskGroup::Build => merlin_core::TaskGroup::Build,
                        types::TaskGroup::Other => merlin_core::TaskGroup::Other,
                    },
                    source: t.detail,
                    plugin: None,
                })
                .collect(),
        });
    }

    fn set_tests(&mut self, path: String, tests: Vec<types::TestItem>) {
        self.events.push(Event::SetTests {
            plugin: self.name.clone(),
            path: self.snapshot.local(&path),
            tests: tests
                .into_iter()
                .map(|t| merlin_core::PluginTest {
                    id: t.id,
                    label: t.label,
                    line: t.line,
                    command: t.command,
                })
                .collect(),
        });
    }

    fn set_test_results(&mut self, results: Vec<types::TestResult>) {
        self.events.push(Event::SetTestResults {
            plugin: self.name.clone(),
            results: results
                .into_iter()
                .map(|r| merlin_core::PluginTestResult {
                    id: r.id,
                    outcome: match r.outcome {
                        types::TestOutcome::Passed => Some(merlin_core::Outcome::Passed),
                        types::TestOutcome::Failed => Some(merlin_core::Outcome::Failed),
                        types::TestOutcome::Skipped => Some(merlin_core::Outcome::Skipped),
                        types::TestOutcome::Running => None,
                    },
                    message: r.message,
                    line: r.line,
                })
                .collect(),
        });
    }

    fn set_debug_targets(&mut self, targets: Vec<types::DebugTarget>) {
        self.events.push(Event::SetDebugTargets {
            plugin: self.name.clone(),
            targets: targets
                .into_iter()
                .map(|t| merlin_core::DebugTarget {
                    name: t.name,
                    debugger: t.debugger,
                    language: t.language,
                    program: t.program,
                    args: t.args,
                    build: t.build,
                    detail: t.detail,
                    launch: t.launch,
                    plugin: None,
                })
                .collect(),
        });
    }

    fn debug_start(&mut self) -> Result<Option<types::DebugLaunch>, String> {
        self.need(self.permissions.debugger, "debugger")?;
        Ok(self.debug_start.clone().map(|s| types::DebugLaunch {
            name: s.name,
            debugger: s.debugger,
            command: s.command,
            program: s.program,
            args: s.args,
            cwd: s.cwd,
            env: s.env,
            launch: s.launch,
            stop_on: s.stop_on,
            function_stops: s
                .function_stops
                .into_iter()
                .map(|f| types::FunctionStop {
                    id: f.id,
                    label: f.label,
                    functions: f.functions,
                    on: f.on,
                })
                .collect(),
        }))
    }

    fn breakpoints(&mut self) -> Result<Vec<types::Breakpoint>, String> {
        self.need(self.permissions.debugger, "debugger")?;
        let root = self.snapshot.root.clone();
        Ok(self
            .breakpoints
            .iter()
            .map(|b| types::Breakpoint {
                id: b.id,
                path: match b.path.is_absolute() {
                    true => b.path.display().to_string(),
                    false => root.join(&b.path).display().to_string(),
                },
                line: b.line,
                condition: b.condition.clone(),
                hit_condition: b.hit.clone(),
                log_message: b.log.clone(),
                enabled: b.enabled,
            })
            .collect())
    }

    fn set_debug_state(&mut self, state: types::DebugState) -> Result<(), String> {
        use merlin_core::DebugState as S;
        let state = match state {
            types::DebugState::Starting => S::Starting,
            types::DebugState::Running => S::Running,
            types::DebugState::Paused(p) => S::Paused {
                reason: p.reason,
                description: p.description,
                thread: p.thread,
            },
            types::DebugState::Ended(code) => S::Ended(code),
        };
        self.debug(merlin_core::DebugUpdate::State(state))
    }

    fn set_threads(&mut self, threads: Vec<types::DebugThread>) -> Result<(), String> {
        self.debug(merlin_core::DebugUpdate::Threads(
            threads
                .into_iter()
                .map(|t| merlin_core::DebugThread {
                    id: t.id,
                    name: t.name,
                    paused: t.paused,
                })
                .collect(),
        ))
    }

    fn set_stack(&mut self, thread: u32, frames: Vec<types::StackFrame>) -> Result<(), String> {
        let frames = frames
            .into_iter()
            .map(|f| merlin_core::StackFrame {
                id: f.id,
                name: f.name,
                path: (!f.path.is_empty()).then(|| PathBuf::from(f.path)),
                line: f.line,
                column: f.column,
                library: f.library,
            })
            .collect();
        self.debug(merlin_core::DebugUpdate::Stack { thread, frames })
    }

    fn set_variables(&mut self, parent: u32, variables: Vec<types::DebugVariable>) -> Result<(), String> {
        let variables = variables
            .into_iter()
            .map(|v| merlin_core::DebugVariable {
                name: v.name,
                value: v.value,
                kind: v.type_name,
                children: v.children,
            })
            .collect();
        self.debug(merlin_core::DebugUpdate::Variables { parent, variables })
    }

    fn set_breakpoint_states(&mut self, states: Vec<types::BreakpointState>) -> Result<(), String> {
        self.debug(merlin_core::DebugUpdate::Breakpoints(
            states
                .into_iter()
                .map(|s| merlin_core::BreakpointState {
                    id: s.id,
                    set: s.set,
                    line: s.line,
                    message: s.message,
                })
                .collect(),
        ))
    }

    fn set_stop_filters(&mut self, filters: Vec<types::StopFilter>) -> Result<(), String> {
        self.debug(merlin_core::DebugUpdate::StopFilters(
            filters
                .into_iter()
                .map(|f| merlin_core::StopFilter {
                    id: f.id,
                    label: f.label,
                    on: f.on,
                })
                .collect(),
        ))
    }

    fn set_inline_values(&mut self, path: String, values: Vec<types::InlineValue>) -> Result<(), String> {
        let path = PathBuf::from(path);
        let values = values.into_iter().map(|v| (v.line, v.text)).collect();
        self.debug(merlin_core::DebugUpdate::InlineValues { path, values })
    }

    fn set_watch(&mut self, index: u32, value: types::DebugVariable) -> Result<(), String> {
        self.debug(merlin_core::DebugUpdate::Watch {
            index,
            value: merlin_core::DebugVariable {
                name: value.name,
                value: value.value,
                kind: value.type_name,
                children: value.children,
            },
        })
    }

    fn debug_output(&mut self, kind: types::OutputKind, text: String) -> Result<(), String> {
        use merlin_core::OutputKind as K;
        let kind = match kind {
            types::OutputKind::Console => K::Console,
            types::OutputKind::Stdout => K::Stdout,
            types::OutputKind::Stderr => K::Stderr,
            types::OutputKind::Important => K::Important,
        };
        self.debug(merlin_core::DebugUpdate::Output(kind, text))
    }

    fn run_task(&mut self, name: String) {
        self.events.push(Event::RunTask(name));
    }

    fn notify(&mut self, message: String) {
        self.events.push(Event::Notify(message));
    }

    fn focus_panel(&mut self) {
        self.events.push(Event::FocusPanel(self.name.clone()));
    }

    fn show_picker(&mut self, id: String, title: String, items: Vec<types::PickItem>) {
        self.events.push(Event::ShowPicker {
            plugin: self.name.clone(),
            id,
            title,
            items: items
                .into_iter()
                .map(|i| PickItem {
                    id: i.id,
                    label: i.label,
                })
                .collect(),
        });
    }

    fn spawn(
        &mut self,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        framing: types::Framing,
    ) -> Result<u32, String> {
        // A debugger plugin may start the debugger it was asked to.
        let debugger = self.permissions.debugger
            && self
                .debug_start
                .as_ref()
                .and_then(|s| s.command.first())
                .is_some_and(|p| *p == program);
        if !self.permissions.processes.contains(&program) && !debugger {
            return Err(format!("permission denied: processes ({program})"));
        }
        let root = self.snapshot.root.clone();
        self.processes.spawn(&program, &args, &env, &root, framing)
    }

    fn send(&mut self, process: u32, message: String) -> Result<(), String> {
        self.processes.send(process, &message)
    }

    fn kill(&mut self, process: u32) {
        self.processes.kill(process);
    }

    fn get_setting(&mut self, key: String) -> Option<types::Setting> {
        let mut value: &toml::Value = self.settings.get(key.split('.').next()?)?;
        for part in key.split('.').skip(1) {
            value = value.as_table()?.get(part)?;
        }
        Some(match value {
            toml::Value::String(s) => types::Setting::Text(s.clone()),
            toml::Value::Boolean(b) => types::Setting::Flag(*b),
            toml::Value::Integer(i) => types::Setting::Integer(*i),
            toml::Value::Float(f) => types::Setting::Float(*f),
            other => types::Setting::Other(other.to_string()),
        })
    }

    fn get_secret(&mut self, key: String) -> Option<String> {
        self.secrets.get(&self.name, &key)
    }

    fn set_secret(&mut self, key: String, value: String) -> Result<(), String> {
        self.secrets.set(&self.name, &key, &value)
    }

    fn delete_secret(&mut self, key: String) {
        self.secrets.delete(&self.name, &key);
    }
}

struct Instance {
    store: Store<State>,
    bindings: Bindings,
}

struct Loaded {
    manifest: Manifest,
    /// The plugin's folder, which its permissions are remembered by.
    dir: String,
    wasm: PathBuf,
    modified: Option<SystemTime>,
    /// None while waiting for the user to allow its permissions.
    instance: Option<Instance>,
    /// Bumped on every reload, so output from an old instance's processes
    /// isn't delivered to the new one.
    generation: u64,
    failed: bool,
    /// Turned off by the user: loaded, but not running.
    disabled: bool,
    /// Time spent in the plugin's calls since it loaded.
    busy: Duration,
}

/// What a plugin's page shows about it.
#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub name: String,
    pub title: String,
    /// Where it was loaded from.
    pub dir: PathBuf,
    pub api: String,
    /// It adds data only, and runs nothing.
    pub data_only: bool,
    pub enabled: bool,
    /// Waiting for the user to allow its permissions.
    pub waiting: bool,
    pub failed: bool,
    pub permissions: Permissions,
    pub languages: Vec<String>,
    /// Themes and packs, by name.
    pub looks: Vec<String>,
    /// Its settings with the user's over its defaults.
    pub settings: toml::Table,
    /// Its manifest's defaults, to tell what the user changed.
    pub defaults: toml::Table,
    /// Bytes of memory its instance has.
    pub memory: usize,
    pub busy: Duration,
    /// Programs it's running, with their process ids.
    pub processes: Vec<(String, u32)>,
}

pub struct PluginHost {
    engine: Engine,
    linker: Linker<State>,
    plugins: Vec<Loaded>,
    output_tx: Sender<Output>,
    output_rx: Receiver<Output>,
    /// Called from background threads when process output is queued, so a
    /// frontend that sleeps between events knows to poll.
    waker: Option<Waker>,
    config: Config,
    secrets: Arc<Secrets>,
    /// `[plugins]` from the user's and the workspace's settings.
    settings: toml::Table,
    /// Plugins the user turned off, by name, which load without running.
    disabled: Vec<String>,
}

const ALLOW: &str = "merlin:allow";
const DENY: &str = "merlin:deny";

/// Wakes the frontend from another thread.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

impl PluginHost {
    /// Wakes the frontend when processes of plugins loaded after this have
    /// output waiting.
    pub fn set_waker(&mut self, wake: impl Fn() + Send + Sync + 'static) {
        self.waker = Some(Arc::new(wake));
    }

    pub fn new(config: Config) -> Result<Self> {
        let mut wasm_config = wasmtime::Config::new();
        wasm_config.consume_fuel(true);
        let engine = Engine::new(&wasm_config).map_err(wt)?;
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(wt)?;
        Bindings::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s).map_err(wt)?;
        let (output_tx, output_rx) = channel();
        let secrets = Arc::new(Secrets::new(
            config.data_dir.join("secrets"),
            config.keyring,
        ));
        Ok(Self {
            engine,
            linker,
            plugins: Vec::new(),
            output_tx,
            output_rx,
            waker: None,
            config,
            secrets,
            settings: toml::Table::new(),
            disabled: Vec::new(),
        })
    }

    /// Loads the plugin in `dir` and returns the events that register it,
    /// or that show the permission card if the user hasn't allowed it yet.
    pub fn load(&mut self, dir: &Path) -> Result<Vec<Event>> {
        let manifest = manifest(dir)?;
        let valid = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
        if manifest.name.is_empty() || !manifest.name.chars().all(valid) {
            bail!(
                "plugin name {:?} must be lowercase letters, digits, - and _",
                manifest.name
            );
        }
        if manifest.api != API_VERSION {
            bail!(
                "{} needs plugin API {}, this editor has {API_VERSION}",
                manifest.name,
                manifest.api
            );
        }
        if self
            .plugins
            .iter()
            .any(|p| p.manifest.name == manifest.name)
        {
            bail!("a plugin named {} is already loaded", manifest.name);
        }
        let folder = dir.to_path_buf();
        let dir = dir
            .canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .display()
            .to_string();
        // Data only: nothing runs, so there's nothing to allow.
        let Some(component) = &manifest.component else {
            if !manifest.permissions.is_empty() {
                bail!(
                    "{} asks for permissions but has no component to use them",
                    manifest.name
                );
            }
            let events = manifest_events(&manifest);
            self.plugins.push(Loaded {
                manifest,
                dir,
                modified: None,
                wasm: PathBuf::new(),
                instance: None,
                generation: 0,
                failed: false,
                disabled: false,
                busy: Duration::ZERO,
            });
            return Ok(events);
        };
        let wasm = folder.join(component);
        let ask = self.config.ask
            && !manifest.permissions.is_empty()
            && !self
                .grants()
                .plugins
                .get(&dir)
                .is_some_and(|g| g.covers(&manifest.permissions));
        let off = self.disabled.contains(&manifest.name);
        let instance = if ask || off {
            None
        } else {
            Some(self.instantiate(&manifest, &wasm, 0)?)
        };
        self.plugins.push(Loaded {
            manifest,
            dir,
            modified: modified(&wasm),
            wasm,
            instance,
            generation: 0,
            failed: false,
            disabled: off,
            busy: Duration::ZERO,
        });
        let index = self.plugins.len() - 1;
        if off {
            return Ok(Vec::new());
        }
        if ask {
            let plugin = &self.plugins[index].manifest;
            return Ok(vec![
                Event::SetPanel {
                    plugin: plugin.name.clone(),
                    panel: Some(permission_card(plugin)),
                },
                Event::FocusPanel(plugin.name.clone()),
            ]);
        }
        Ok(self.activate(index, None))
    }

    /// Every loaded plugin's themes and packs: name and file.
    pub fn packs(&self) -> Vec<(String, PathBuf)> {
        self.plugins
            .iter()
            .flat_map(|p| {
                p.manifest
                    .themes
                    .iter()
                    .chain(&p.manifest.packs)
                    .map(|t| (t.label.clone(), Path::new(&p.dir).join(&t.file)))
            })
            .collect()
    }

    fn grants(&self) -> Grants {
        read_grants(&self.config.data_dir)
    }

    /// Answers a permission card: loads the plugin and remembers the grant,
    /// or drops it.
    fn answer(&mut self, index: usize, allow: bool) -> Vec<Event> {
        let name = self.plugins[index].manifest.name.clone();
        if !allow {
            let plugin = self.plugins.remove(index);
            return vec![
                Event::SetPanel {
                    plugin: name,
                    panel: None,
                },
                Event::Notify(format!("{} was not loaded", plugin.manifest.title)),
            ];
        }
        let plugin = &self.plugins[index];
        let instance = match self.instantiate(&plugin.manifest, &plugin.wasm, 0) {
            Ok(instance) => instance,
            Err(err) => {
                self.plugins.remove(index);
                return vec![Event::PluginFailed {
                    plugin: name,
                    message: format!("{err:#}"),
                }];
            }
        };
        let saved = save_grant(
            &self.config.data_dir,
            &plugin.dir,
            &plugin.manifest.permissions,
        );
        self.plugins[index].instance = Some(instance);
        let mut events = self.activate(index, None);
        if let Err(err) = saved {
            events.push(Event::Notify(format!(
                "Couldn't remember the permission: {err:#}"
            )));
        }
        events
    }

    /// Compiles a component, or loads it compiled from the cache in the
    /// data folder, since compiling every plugin at every start is slow.
    /// The cache is keyed by the component's bytes and this build of the
    /// editor, so a rebuilt plugin or editor compiles afresh.
    fn compile(&self, wasm: &Path) -> Result<Component> {
        use std::hash::{Hash, Hasher};
        let bytes = fs::read(wasm)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        std::env::current_exe()
            .ok()
            .and_then(|e| fs::metadata(e).ok()?.modified().ok())
            .hash(&mut hasher);
        let dir = self.config.data_dir.join("compiled");
        let cached = dir.join(format!("{:016x}.cwasm", hasher.finish()));
        if cached.exists() {
            // Safe as long as the file is what `serialize` wrote for this
            // engine: it's in the user's own data folder, named by the
            // component and the editor that compiled it.
            if let Ok(component) = unsafe { Component::deserialize_file(&self.engine, &cached) } {
                return Ok(component);
            }
        }
        let component = Component::new(&self.engine, &bytes).map_err(wt)?;
        // Written aside and renamed into place: another editor may have the
        // file mapped, and changing it under them would crash them.
        if let Ok(compiled) = component.serialize() {
            let aside = dir.join(format!(
                "{:016x}.{}.tmp",
                hasher.finish(),
                std::process::id()
            ));
            let _ = fs::create_dir_all(&dir)
                .and_then(|()| fs::write(&aside, compiled))
                .and_then(|()| fs::rename(&aside, &cached));
        }
        Ok(component)
    }

    fn instantiate(&self, manifest: &Manifest, wasm: &Path, generation: u64) -> Result<Instance> {
        let component = self
            .compile(wasm)
            .with_context(|| format!("loading {}", wasm.display()))?;
        let state = State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            name: manifest.name.clone(),
            permissions: manifest.permissions.clone(),
            snapshot: Snapshot::default(),
            events: Vec::new(),
            processes: Processes::new(
                manifest.name.clone(),
                generation,
                self.output_tx.clone(),
                self.waker.clone(),
            ),
            secrets: self.secrets.clone(),
            settings: plugin_settings(manifest, &self.settings),
            limits: Limits::default(),
            debug_start: None,
            breakpoints: Vec::new(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        store.set_fuel(FUEL_PER_CALL).map_err(wt)?;
        let bindings = Bindings::instantiate(&mut store, &component, &self.linker)
            .map_err(wt)
            .with_context(|| format!("instantiating {}", manifest.name))?;
        Ok(Instance { store, bindings })
    }

    fn activate(&mut self, index: usize, state: Option<Vec<u8>>) -> Vec<Event> {
        let plugin = self.plugins[index].manifest.name.clone();
        let (commands, mut events) = self.call(index, Snapshot::default(), |b, s| {
            b.call_activate(s, state.as_deref())
        });
        if let Some(commands) = commands {
            events.insert(
                0,
                Event::RegisterPlugin {
                    plugin,
                    commands: commands
                        .into_iter()
                        .map(|c| Command {
                            id: c.id,
                            title: c.title,
                            key: c.key,
                        })
                        .collect(),
                },
            );
            for (i, event) in manifest_events(&self.plugins[index].manifest)
                .into_iter()
                .enumerate()
            {
                events.insert(1 + i, event);
            }
        }
        events
    }

    /// Calls into a plugin and collects the events it asked for. A trap
    /// disables the plugin and stops its processes until it is reloaded.
    fn call<R>(
        &mut self,
        index: usize,
        snapshot: Snapshot,
        f: impl FnOnce(&Bindings, &mut Store<State>) -> wasmtime::Result<R>,
    ) -> (Option<R>, Vec<Event>) {
        let plugin = &mut self.plugins[index];
        let Some(instance) = plugin.instance.as_mut().filter(|_| !plugin.failed) else {
            return (None, Vec::new());
        };
        let store = &mut instance.store;
        store.data_mut().snapshot = snapshot;
        let _ = store.set_fuel(FUEL_PER_CALL);
        let started = std::time::Instant::now();
        let result = f(&instance.bindings, store);
        plugin.busy += started.elapsed();
        let mut events = std::mem::take(&mut store.data_mut().events);
        match result {
            Ok(value) => (Some(value), events),
            Err(err) => {
                plugin.failed = true;
                store.data_mut().processes.kill_all();
                events.push(Event::PluginFailed {
                    plugin: plugin.manifest.name.clone(),
                    message: format!("{err:#}"),
                });
                (None, events)
            }
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.plugins.iter().position(|p| p.manifest.name == name)
    }

    /// Takes new settings for plugins: the `[plugins]` table of the user's
    /// settings with the workspace's over it.
    pub fn set_settings(&mut self, plugins: toml::Table) {
        self.settings = plugins;
        for plugin in &mut self.plugins {
            let settings = plugin_settings(&plugin.manifest, &self.settings);
            if let Some(instance) = &mut plugin.instance {
                instance.store.data_mut().settings = settings;
            }
        }
    }

    /// Gives a debugger plugin what `debug-start` and `breakpoints` read,
    /// before a debug command. `start` replaces what it had when given.
    pub fn set_debug(
        &mut self,
        plugin: &str,
        start: Option<merlin_core::DebugStart>,
        breakpoints: Vec<merlin_core::Breakpoint>,
    ) {
        let Some(i) = self.index(plugin) else {
            return;
        };
        if let Some(instance) = &mut self.plugins[i].instance {
            let state = instance.store.data_mut();
            if start.is_some() {
                state.debug_start = start;
            }
            state.breakpoints = breakpoints;
        }
    }

    pub fn run_command(&mut self, plugin: &str, command: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_run_command(s, command))
            .1
    }

    pub fn click(&mut self, plugin: &str, button: &str, snapshot: Snapshot) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        // Data only or turned off: nothing to click or answer.
        if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
            return Vec::new();
        }
        if self.plugins[i].instance.is_none() {
            return match button {
                ALLOW => self.answer(i, true),
                DENY => self.answer(i, false),
                _ => Vec::new(),
            };
        }
        self.call(i, snapshot, |b, s| b.call_on_click(s, button)).1
    }

    /// Tells every working plugin that the open files changed.
    pub fn buffer_event(&mut self, event: &BufferEvent, snapshot: &Snapshot) -> Vec<Event> {
        let event = match event {
            BufferEvent::Switched => types::BufferEvent::Switched,
            BufferEvent::Changed => types::BufferEvent::Changed,
            BufferEvent::Saved(path) => types::BufferEvent::Saved(path.clone()),
        };
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            let (_, mut more) = self.call(i, snapshot.clone(), |b, s| b.call_on_buffer(s, &event));
            events.append(&mut more);
        }
        events
    }

    pub fn submit(
        &mut self,
        plugin: &str,
        input: &str,
        text: &str,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_submit(s, input, text))
            .1
    }

    pub fn review(
        &mut self,
        plugin: &str,
        id: &str,
        accepted: bool,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_review(s, id, accepted))
            .1
    }

    /// Colours `text` as a language, named by its id or an extension, with
    /// the plugin that added it. `None` if no plugin did.
    pub fn colour_text(
        &mut self,
        language: &str,
        text: &str,
    ) -> Option<Vec<merlin_core::lang::PluginToken>> {
        let name = language.trim_start_matches('.').to_ascii_lowercase();
        let i = self.plugins.iter().position(|p| {
            p.instance.is_some()
                && p.manifest
                    .languages
                    .iter()
                    .any(|l| l.id == name || l.extensions.contains(&name))
        })?;
        let (tokens, _) = self.call(i, Snapshot::default(), |b, s| {
            b.call_colour_text(s, &name, text)
        });
        Some(tokens?.into_iter().map(plugin_token).collect())
    }

    pub fn pick(
        &mut self,
        plugin: &str,
        picker: &str,
        item: &str,
        snapshot: Snapshot,
    ) -> Vec<Event> {
        let Some(i) = self.index(plugin) else {
            return Vec::new();
        };
        self.call(i, snapshot, |b, s| b.call_on_pick(s, picker, item))
            .1
    }

    /// Asks every working plugin for its panel.
    pub fn render_panels(&mut self, snapshot: &Snapshot) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            // Data only or turned off: no panel, and nothing to allow.
            if self.plugins[i].manifest.component.is_none() || self.plugins[i].disabled {
                continue;
            }
            if self.plugins[i].instance.is_none() {
                events.push(Event::SetPanel {
                    plugin: self.plugins[i].manifest.name.clone(),
                    panel: Some(permission_card(&self.plugins[i].manifest)),
                });
                continue;
            }
            let (panel, mut more) = self.call(i, snapshot.clone(), |b, s| b.call_render_panel(s));
            events.append(&mut more);
            if let Some(panel) = panel {
                events.push(Event::SetPanel {
                    plugin: self.plugins[i].manifest.name.clone(),
                    panel: panel.map(convert_panel),
                });
            }
        }
        events
    }

    /// Delivers process output that arrived since the last poll. `snapshot`
    /// is only called if there is output to deliver. Returns the events the
    /// plugins asked for, and whether anything was delivered.
    pub fn poll_processes(&mut self, snapshot: impl Fn() -> Snapshot) -> (Vec<Event>, bool) {
        let pending: Vec<Output> = self.output_rx.try_iter().take(MESSAGES_PER_POLL).collect();
        if pending.is_empty() {
            return (Vec::new(), false);
        }
        let snapshot = snapshot();
        let mut events = Vec::new();
        for output in pending {
            let Some(i) = self.index(&output.plugin) else {
                continue;
            };
            if self.plugins[i].generation != output.generation {
                continue;
            }
            if let (types::ProcessEvent::Exit(_), Some(instance)) =
                (&output.event, &mut self.plugins[i].instance)
            {
                instance.store.data_mut().processes.forget(output.process);
            }
            let (_, mut more) = self.call(i, snapshot.clone(), |b, s| {
                b.call_on_process(s, output.process, &output.event)
            });
            events.append(&mut more);
        }
        (events, true)
    }

    /// Reloads plugins whose component file changed since it was loaded.
    /// A plugin that fails to reload keeps running its old code. One that
    /// saves its state gets it back, and keeps its processes.
    /// The component files of the plugins loaded, which reload when they
    /// change.
    pub fn components(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .filter(|p| !p.wasm.as_os_str().is_empty())
            .map(|p| p.wasm.clone())
            .collect()
    }

    pub fn reload_changed(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for i in 0..self.plugins.len() {
            let now = modified(&self.plugins[i].wasm);
            if now.is_none()
                || now == self.plugins[i].modified
                || self.plugins[i].instance.is_none()
            {
                continue;
            }
            self.plugins[i].modified = now;
            events.extend(self.reload_at(i));
        }
        events
    }

    /// Starts a plugin again from its component, keeping its state if it
    /// saves it.
    pub fn reload(&mut self, name: &str) -> Vec<Event> {
        match self.index(name) {
            Some(i) if self.plugins[i].instance.is_some() => self.reload_at(i),
            _ => Vec::new(),
        }
    }

    fn reload_at(&mut self, i: usize) -> Vec<Event> {
        let mut events = Vec::new();
        let name = self.plugins[i].manifest.title.clone();
        let (state, mut more) = self.call(i, Snapshot::default(), |b, s| b.call_save_state(s));
        events.append(&mut more);
        let state = state.flatten();
        let plugin = &self.plugins[i];
        // Output from the old instance's processes goes to the new one
        // only if they're handed over.
        let generation = plugin.generation + state.is_none() as u64;
        match self.instantiate(&plugin.manifest, &plugin.wasm, generation) {
            Ok(mut instance) => {
                let plugin = &mut self.plugins[i];
                if let Some(old) = &mut plugin.instance
                    && state.is_some()
                {
                    std::mem::swap(
                        &mut old.store.data_mut().processes,
                        &mut instance.store.data_mut().processes,
                    );
                }
                // Dropping the old store stops any processes it still has.
                plugin.instance = Some(instance);
                plugin.generation = generation;
                plugin.failed = false;
                events.extend(self.activate(i, state));
                events.push(Event::Notify(format!("Reloaded {name}")));
            }
            Err(err) => events.push(Event::Notify(format!("Reload of {name} failed: {err:#}"))),
        }
        events
    }

    /// Plugins to load turned off, by name, from the settings.
    pub fn set_disabled(&mut self, names: Vec<String>) {
        self.disabled = names;
    }

    /// Turns a plugin on or off. Off, it stops: its processes end, and its
    /// panel and commands go.
    pub fn set_enabled(&mut self, name: &str, on: bool) -> Vec<Event> {
        let Some(i) = self.index(name) else {
            return Vec::new();
        };
        self.disabled.retain(|n| n != name);
        if !on {
            self.disabled.push(name.to_string());
        }
        let plugin = &mut self.plugins[i];
        if plugin.manifest.component.is_none() || plugin.disabled == !on {
            return Vec::new();
        }
        plugin.disabled = !on;
        if !on {
            // Dropping the store stops its processes.
            plugin.instance = None;
            return vec![
                Event::SetPanel {
                    plugin: name.to_string(),
                    panel: None,
                },
                Event::RegisterPlugin {
                    plugin: name.to_string(),
                    commands: Vec::new(),
                },
            ];
        }
        let plugin = &self.plugins[i];
        match self.instantiate(&plugin.manifest, &plugin.wasm, plugin.generation + 1) {
            Ok(instance) => {
                let plugin = &mut self.plugins[i];
                plugin.instance = Some(instance);
                plugin.generation += 1;
                plugin.failed = false;
                self.activate(i, None)
            }
            Err(err) => vec![Event::PluginFailed {
                plugin: name.to_string(),
                message: format!("{err:#}"),
            }],
        }
    }

    /// Stops a plugin and forgets it, as uninstalling does.
    pub fn unload(&mut self, name: &str) -> Vec<Event> {
        let Some(i) = self.index(name) else {
            return Vec::new();
        };
        self.plugins.remove(i);
        vec![
            Event::SetPanel {
                plugin: name.to_string(),
                panel: None,
            },
            Event::RegisterPlugin {
                plugin: name.to_string(),
                commands: Vec::new(),
            },
        ]
    }

    /// What each plugin's page shows.
    pub fn plugins(&self) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .map(|p| {
                let m = &p.manifest;
                let (memory, processes) = match &p.instance {
                    Some(i) => (
                        i.store.data().limits.memory,
                        i.store.data().processes.running(),
                    ),
                    None => (0, Vec::new()),
                };
                PluginInfo {
                    name: m.name.clone(),
                    title: m.title.clone(),
                    dir: PathBuf::from(&p.dir),
                    api: m.api.clone(),
                    data_only: m.component.is_none(),
                    enabled: !p.disabled,
                    waiting: m.component.is_some() && !p.disabled && p.instance.is_none(),
                    failed: p.failed,
                    permissions: m.permissions.clone(),
                    languages: m.languages.iter().map(|l| l.name.clone()).collect(),
                    looks: m
                        .themes
                        .iter()
                        .chain(&m.packs)
                        .map(|t| t.label.clone())
                        .collect(),
                    settings: plugin_settings(m, &self.settings),
                    defaults: m.settings.clone(),
                    memory,
                    busy: p.busy,
                    processes,
                }
            })
            .collect()
    }
}

fn convert_panel(panel: types::Panel) -> Panel {
    let button = |b: types::Button| Button {
        id: b.id,
        label: b.label,
    };
    let tone = |tone| match tone {
        types::Tone::Normal => Tone::Normal,
        types::Tone::Muted => Tone::Muted,
        types::Tone::Strong => Tone::Strong,
        types::Tone::Warning => Tone::Warning,
        types::Tone::Error => Tone::Error,
    };
    Panel {
        title: panel.title,
        nodes: panel
            .nodes
            .into_iter()
            .map(|node| match node {
                types::Node::Heading(text) => Node::Heading(text),
                types::Node::Text(text) => Node::Text(text),
                types::Node::Styled(s) => Node::Styled(s.text, tone(s.tone)),
                types::Node::Item(i) => Node::Item {
                    id: i.id,
                    label: i.label,
                    tone: tone(i.tone),
                },
                types::Node::Markdown(text) => Node::Markdown(text),
                types::Node::Button(b) => Node::Button(button(b)),
                types::Node::Buttons(bs) => Node::Buttons(bs.into_iter().map(button).collect()),
                types::Node::Input(i) => Node::Input {
                    id: i.id,
                    placeholder: i.placeholder,
                    secret: i.secret,
                },
                types::Node::Divider => Node::Divider,
                types::Node::Footer => Node::Footer,
            })
            .collect(),
    }
}

/// What a plugin shows until the user allows its permissions.
fn permission_card(manifest: &Manifest) -> Panel {
    let p = &manifest.permissions;
    let mut nodes = vec![
        Node::Heading(format!("Allow {}?", manifest.title)),
        Node::Text("This plugin asks to:".into()),
    ];
    if p.editor_read {
        nodes.push(Node::Text("• read the open file".into()));
    }
    if p.editor_write {
        nodes.push(Node::Text("• change the open file".into()));
    }
    for program in &p.processes {
        nodes.push(Node::Styled(
            format!("• run `{program}`, outside the sandbox, with your rights"),
            Tone::Warning,
        ));
    }
    if p.debugger {
        nodes.push(Node::Styled(
            "• start the debuggers plugins name when you start a run, and drive them".into(),
            Tone::Warning,
        ));
    }
    nodes.push(Node::Text(String::new()));
    nodes.push(Node::Styled(
        "You'll be asked again only if it wants more.".into(),
        Tone::Muted,
    ));
    nodes.push(Node::Buttons(vec![
        Button {
            id: ALLOW.into(),
            label: "Allow".into(),
        },
        Button {
            id: DENY.into(),
            label: "Don't load".into(),
        },
    ]));
    Panel {
        title: manifest.title.clone(),
        nodes,
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn wt(err: wasmtime::Error) -> anyhow::Error {
    anyhow!("{err:#}")
}

/// A plugin's settings: its manifest's defaults, with what the user set
/// over them.
fn plugin_settings(manifest: &Manifest, plugins: &toml::Table) -> toml::Table {
    let mut settings = manifest.settings.clone();
    if let Some(toml::Value::Table(set)) = plugins.get(&manifest.name) {
        merge(&mut settings, set);
    }
    settings
}

/// Puts `over`'s values into `base`, table by table.
pub fn merge(base: &mut toml::Table, over: &toml::Table) {
    for (key, value) in over {
        match (base.get_mut(key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

fn plugin_token(t: types::Token) -> merlin_core::lang::PluginToken {
    use merlin_core::Syntax;
    use types::TokenKind as K;
    let at = |p: types::Position| (p.line, p.column);
    merlin_core::lang::PluginToken {
        start: at(t.start),
        end: at(t.end),
        kind: match t.kind {
            K::Plain => Syntax::Plain,
            K::Keyword => Syntax::Keyword,
            K::String => Syntax::String,
            K::Comment => Syntax::Comment,
            K::Function => Syntax::Function,
            K::Type => Syntax::Type,
            K::Constant => Syntax::Constant,
            K::Attribute => Syntax::Attribute,
            K::Property => Syntax::Property,
            K::Operator => Syntax::Operator,
            K::Punctuation => Syntax::Punctuation,
            K::Escape => Syntax::Escape,
            K::Variable => Syntax::Variable,
            K::Macro => Syntax::Macro,
        },
    }
}

fn read_grants(data_dir: &Path) -> Grants {
    fs::read_to_string(data_dir.join("grants.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remembers that the plugin in `dir` may do what `permissions` say.
fn save_grant(data_dir: &Path, dir: &str, permissions: &Permissions) -> Result<()> {
    let mut grants = read_grants(data_dir);
    grants.plugins.insert(dir.to_string(), permissions.clone());
    fs::create_dir_all(data_dir)?;
    fs::write(data_dir.join("grants.toml"), toml::to_string(&grants)?)?;
    Ok(())
}

/// Reads a plugin's manifest.
pub fn manifest(dir: &Path) -> Result<Manifest> {
    let path = dir.join("plugin.toml");
    toml::from_str(
        &fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))
}

/// Allows the plugin in `dir` what its manifest asks for, as installing it
/// does, and says what that is.
pub fn grant(config: &Config, dir: &Path) -> Result<(Manifest, Vec<String>)> {
    let manifest = manifest(dir)?;
    let dir = dir.canonicalize()?.display().to_string();
    save_grant(&config.data_dir, &dir, &manifest.permissions)?;
    let p = &manifest.permissions;
    let mut allowed = Vec::new();
    if p.editor_read {
        allowed.push("read the open file".to_string());
    }
    if p.editor_write {
        allowed.push("change the open file".to_string());
    }
    for program in &p.processes {
        allowed.push(format!("run `{program}`, outside the sandbox"));
    }
    if p.debugger {
        allowed.push("start the debuggers plugins name, and drive them".to_string());
    }
    Ok((manifest, allowed))
}

fn diagnostic(d: types::Diagnostic, source: String, code: String, link: String) -> Diagnostic {
    let at = |p: types::Position| (p.line, p.column);
    Diagnostic {
        start: at(d.start),
        end: at(d.end),
        severity: match d.severity {
            types::Severity::Error => Severity::Error,
            types::Severity::Warning => Severity::Warning,
            types::Severity::Info => Severity::Info,
            types::Severity::Hint => Severity::Hint,
        },
        message: d.message,
        source,
        code,
        link,
    }
}
