//! What a plugin's instance holds, and the calls it makes to the editor:
//! the host functions in `wit/merlin.wit`, turned into events.

use super::*;

pub(super) struct State {
    pub(super) wasi: WasiCtx,
    pub(super) table: ResourceTable,
    pub(super) name: String,
    pub(super) permissions: Permissions,
    pub(super) snapshot: Snapshot,
    pub(super) events: Vec<Event>,
    pub(super) processes: Processes,
    pub(super) secrets: Arc<Secrets>,
    /// The plugin's settings, defaults and all.
    pub(super) settings: toml::Table,
    /// How much memory the plugin's instance has, for its page.
    pub(super) limits: Limits,
    /// What the editor last asked this plugin to debug, and the
    /// breakpoints, for `debug-start` and `breakpoints`.
    pub(super) debug_start: Option<merlin_core::DebugStart>,
    pub(super) breakpoints: Vec<merlin_core::Breakpoint>,
    /// What the palette's symbol search looks for, for `symbol-query`.
    pub(super) symbol_query: String,
}

/// Most memory one plugin instance may grow to: 128 MB is plenty for
/// colouring a 512 KB file or holding a language server's symbols, and
/// stops a runaway allocation from taking the machine's RAM before fuel
/// runs out.
const MAX_MEMORY: usize = 128 * 1024 * 1024;

/// Counts the memory an instance grows to, up to [`MAX_MEMORY`].
#[derive(Default)]
pub(super) struct Limits {
    pub(super) memory: usize,
}

impl wasmtime::ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > MAX_MEMORY {
            return Ok(false);
        }
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
    pub(super) fn debug(&mut self, update: merlin_core::DebugUpdate) -> Result<(), String> {
        self.need(self.permissions.debugger, "debugger")?;
        self.events.push(Event::Debug {
            plugin: self.name.clone(),
            update,
        });
        Ok(())
    }

    pub(super) fn need(&self, granted: bool, permission: &str) -> Result<(), String> {
        if granted {
            Ok(())
        } else {
            Err(format!("permission denied: {permission}"))
        }
    }

    /// A plugin's places as the editor's, with workspace paths.
    pub(super) fn locations(
        &self,
        places: Vec<types::Location>,
    ) -> Vec<merlin_core::lang::Location> {
        let at = |p: types::Position| (p.line, p.column);
        places
            .into_iter()
            .map(|l| merlin_core::lang::Location {
                path: self.snapshot.local(&l.path),
                start: at(l.start),
                end: at(l.end),
            })
            .collect()
    }

    /// A plugin's file edits as the editor's, with workspace paths.
    pub(super) fn file_edits(
        &self,
        edits: Vec<types::FileEdit>,
    ) -> Vec<merlin_core::lang::FileEdit> {
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
        let places = self.locations(places);
        self.events.push(Event::ShowReferences(places));
    }

    fn show_definitions(&mut self, places: Vec<types::Location>) {
        let places = self.locations(places);
        self.events.push(Event::Definitions(places));
    }

    fn symbol_query(&mut self) -> String {
        self.symbol_query.clone()
    }

    fn show_project_symbols(&mut self, query: String, symbols: Vec<types::ProjectSymbol>) {
        let symbols = symbols
            .into_iter()
            .map(|s| merlin_core::lang::ProjectSymbol {
                name: s.name,
                kind: s.kind,
                container: s.container,
                path: self.snapshot.local(&s.path),
                line: s.start.line,
                column: s.start.column,
            })
            .collect();
        self.events.push(Event::ProjectSymbols { query, symbols });
    }

    fn insert_text(&mut self, text: String) -> Result<(), String> {
        self.need(self.permissions.editor_write, "editor-write")?;
        self.events.push(Event::InsertText(text));
        Ok(())
    }

    fn workspace_root(&mut self) -> String {
        self.snapshot.root.display().to_string()
    }

    fn list_files(&mut self) -> Result<Vec<String>, String> {
        self.need(self.permissions.workspace_read, "workspace-read")?;
        let root = &self.snapshot.root;
        if root.as_os_str().is_empty() {
            return Err("no workspace yet".into());
        }
        const MAX: usize = 50_000;
        let mut files: Vec<String> = ignore::WalkBuilder::new(root)
            .build()
            .flatten()
            .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
            .filter_map(|e| Some(e.path().strip_prefix(root).ok()?.display().to_string()))
            .take(MAX)
            .collect();
        files.sort();
        Ok(files)
    }

    fn read_file(&mut self, path: String) -> Result<String, String> {
        self.need(self.permissions.workspace_read, "workspace-read")?;
        let root = &self.snapshot.root;
        if root.as_os_str().is_empty() {
            return Err("no workspace yet".into());
        }
        // Within the workspace only, however the path is written.
        let full = root.join(&path).canonicalize().map_err(|e| e.to_string())?;
        let inside = root.canonicalize().is_ok_and(|r| full.starts_with(r));
        if !inside {
            return Err(format!("{path} is outside the workspace"));
        }
        std::fs::read_to_string(&full).map_err(|e| e.to_string())
    }

    fn set_status(&mut self, set: String, items: Vec<types::StatusItem>) {
        use merlin_core::lang::{StatusLine, StatusState};
        self.events.push(Event::SetStatus {
            plugin: self.name.clone(),
            set,
            lines: items
                .into_iter()
                .map(|i| StatusLine {
                    name: i.name,
                    state: match i.state {
                        types::StatusState::Ready => StatusState::Ready,
                        types::StatusState::Busy => StatusState::Busy,
                        types::StatusState::Warning => StatusState::Warning,
                        types::StatusState::Error => StatusState::Error,
                    },
                    detail: i.detail,
                    progress: i.progress,
                    hint: i.hint,
                    action: i.action,
                    extensions: i.extensions,
                })
                .collect(),
        });
    }

    fn set_tasks(&mut self, set: String, tasks: Vec<types::Task>) {
        self.events.push(Event::SetTasks {
            plugin: self.name.clone(),
            set,
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

    fn set_tests(
        &mut self,
        path: String,
        group: String,
        label: String,
        tests: Vec<types::TestItem>,
    ) {
        self.events.push(Event::SetTests {
            plugin: self.name.clone(),
            path: self.snapshot.local(&path),
            group,
            label,
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

    fn set_test_run(&mut self, group: String, running: bool, seconds: Option<f64>) {
        self.events.push(Event::SetTestRun {
            plugin: self.name.clone(),
            group,
            running,
            seconds,
        });
    }

    fn run_in_terminal(&mut self, id: String, name: String, command: String) -> Result<(), String> {
        self.need(self.permissions.terminal, "terminal")?;
        self.events.push(Event::RunInTerminal {
            plugin: self.name.clone(),
            id,
            name,
            command,
        });
        Ok(())
    }

    fn start_debugging(&mut self, target: types::DebugTarget, fallback: Option<String>) {
        self.events.push(Event::StartDebugging {
            plugin: self.name.clone(),
            target: Box::new(debug_target(target)),
            fallback,
        });
    }

    fn set_debug_targets(&mut self, targets: Vec<types::DebugTarget>) {
        self.events.push(Event::SetDebugTargets {
            plugin: self.name.clone(),
            targets: targets.into_iter().map(debug_target).collect(),
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

    fn set_variables(
        &mut self,
        parent: u32,
        variables: Vec<types::DebugVariable>,
    ) -> Result<(), String> {
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

    fn set_inline_values(
        &mut self,
        path: String,
        values: Vec<types::InlineValue>,
    ) -> Result<(), String> {
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

pub(super) fn convert_panel(panel: types::Panel) -> Panel {
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
                    multiline: false,
                },
                types::Node::Divider => Node::Divider,
                types::Node::Footer => Node::Footer,
            })
            .collect(),
    }
}

pub(super) fn plugin_token(t: types::Token) -> merlin_core::lang::PluginToken {
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

pub(super) fn diagnostic(
    d: types::Diagnostic,
    source: String,
    code: String,
    link: String,
) -> Diagnostic {
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

/// A plugin's thing to debug as the editor's.
fn debug_target(t: types::DebugTarget) -> merlin_core::DebugTarget {
    merlin_core::DebugTarget {
        name: t.name,
        debugger: t.debugger,
        language: t.language,
        program: t.program,
        args: t.args,
        build: t.build,
        detail: t.detail,
        launch: t.launch,
        plugin: None,
    }
}
