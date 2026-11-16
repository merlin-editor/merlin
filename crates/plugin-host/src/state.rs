//! What a plugin's instance holds, and the calls it makes to the editor:
//! the host functions in `sdk/rust/wit/merlin.wit`, turned into events.

use super::*;
use std::collections::{BTreeMap, HashMap};

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
    /// The bytes each of its pictures takes, by id, against its budget.
    pub(super) images: HashMap<String, usize>,
    /// Ids for agents' requests and runs.
    pub(super) next_agent_id: u32,
    /// Each run's chat, by run.
    pub(super) agent_runs: HashMap<u32, u32>,
    /// The programs its manifest declares, for their install commands.
    pub(super) programs: BTreeMap<String, crate::needs::ManifestProgram>,
    /// The keys it shares and watches, from its manifest.
    pub(super) shared_keys: ManifestShared,
    /// What every plugin shares.
    pub(super) board: SharedBoard,
}

/// Milliseconds since the host started, for when agents said things.
pub(super) fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// What one plugin's pictures may take together, as RGBA.
const PICTURE_BUDGET: usize = 64 << 20;
/// The most pixels a picture may have a side.
const PICTURE_SIDE: u32 = 8192;

/// A picture as premultiplied RGBA, or why it can't be read.
pub(super) fn decode(picture: types::Picture) -> Result<(u32, u32, Vec<u8>), String> {
    let (width, height, mut pixels) = match picture {
        types::Picture::Rgba(p) => {
            if p.width == 0 || p.height == 0 || p.width > PICTURE_SIDE || p.height > PICTURE_SIDE {
                return Err(format!(
                    "a picture is 1 to {PICTURE_SIDE} pixels a side, not {}×{}",
                    p.width, p.height
                ));
            }
            if p.pixels.len() != p.width as usize * p.height as usize * 4 {
                return Err(format!(
                    "{}×{} RGBA is {} bytes, not {}",
                    p.width,
                    p.height,
                    p.width as usize * p.height as usize * 4,
                    p.pixels.len()
                ));
            }
            (p.width, p.height, p.pixels)
        }
        types::Picture::Png(bytes) => {
            let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            decoder.set_transformations(png::Transformations::normalize_to_color8());
            let mut reader = decoder.read_info().map_err(|e| format!("not a PNG: {e}"))?;
            let info = reader.info();
            if info.width > PICTURE_SIDE || info.height > PICTURE_SIDE {
                return Err(format!(
                    "a picture is at most {PICTURE_SIDE} pixels a side, not {}×{}",
                    info.width, info.height
                ));
            }
            let size = reader.output_buffer_size().ok_or("the PNG is too large")?;
            let mut buf = vec![0; size];
            let frame = reader
                .next_frame(&mut buf)
                .map_err(|e| format!("reading the PNG: {e}"))?;
            buf.truncate(frame.buffer_size());
            let rgba = match frame.color_type {
                png::ColorType::Rgba => buf,
                png::ColorType::Rgb => buf
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|c| [c[0], c[1], c[2], 255])
                    .collect(),
                png::ColorType::GrayscaleAlpha => buf
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .flat_map(|c| [c[0], c[0], c[0], c[1]])
                    .collect(),
                png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
                png::ColorType::Indexed => return Err("the PNG's palette wasn't expanded".into()),
            };
            (frame.width, frame.height, rgba)
        }
    };
    for px in pixels.as_chunks_mut::<4>().0 {
        let a = px[3] as u16;
        for c in &mut px[..3] {
            *c = ((*c as u16 * a + 127) / 255) as u8;
        }
    }
    Ok((width, height, pixels))
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
    /// A path within the workspace, however it's written, made whole.
    fn inside_workspace(&self, path: &str) -> Result<std::path::PathBuf, String> {
        let root = &self.snapshot.root;
        if root.as_os_str().is_empty() {
            return Err("no workspace yet".into());
        }
        let full = root.join(path).canonicalize().map_err(|e| e.to_string())?;
        let inside = root.canonicalize().is_ok_and(|r| full.starts_with(r));
        if !inside {
            return Err(format!("{path} is outside the workspace"));
        }
        Ok(full)
    }

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

    fn active_indent(&mut self) -> types::Indent {
        let (tabs, width) = self.snapshot.indent.unwrap_or((false, 4));
        types::Indent { tabs, width }
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

    fn menu_target(&mut self) -> Option<String> {
        self.snapshot.menu_target.clone()
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

    fn list_folder(&mut self, path: String) -> Result<Vec<String>, String> {
        self.need(self.permissions.workspace_read, "workspace-read")?;
        let full = self.inside_workspace(&path)?;
        let mut names: Vec<String> = std::fs::read_dir(&full)
            .map_err(|e| e.to_string())?
            .flatten()
            .take(10_000)
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                match e.file_type().is_ok_and(|t| t.is_dir()) {
                    true => format!("{name}/"),
                    false => name,
                }
            })
            .collect();
        names.sort();
        Ok(names)
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
                    // The manifest's command for this system wins over
                    // the plugin's own, which can't tell systems apart.
                    hint: self
                        .programs
                        .get(&i.name)
                        .and_then(|p| p.install.as_ref()?.here())
                        .map(String::from)
                        .or(i.hint),
                    name: i.name,
                    state: match i.state {
                        types::StatusState::Ready => StatusState::Ready,
                        types::StatusState::Busy => StatusState::Busy,
                        types::StatusState::Warning => StatusState::Warning,
                        types::StatusState::Error => StatusState::Error,
                    },
                    detail: i.detail,
                    progress: i.progress,
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
        self.notice(types::LogLevel::Info, message, Vec::new());
    }

    fn log(&mut self, level: types::LogLevel, message: String) {
        let level = log_level(level);
        if level < self.log_level() {
            return;
        }
        self.events.push(Event::PluginLog {
            plugin: self.name.clone(),
            level,
            text: message,
        });
    }

    fn notice(&mut self, level: types::LogLevel, message: String, buttons: Vec<types::Button>) {
        let level = log_level(level);
        let shown = match self.text_setting("notices").as_deref() {
            Some("none") => false,
            Some("errors") => level >= LogLevel::Error,
            _ => true,
        };
        if !shown {
            self.events.push(Event::PluginLog {
                plugin: self.name.clone(),
                level,
                text: message,
            });
            return;
        }
        self.events.push(Event::PluginNotice {
            plugin: self.name.clone(),
            level,
            text: message,
            buttons: buttons
                .into_iter()
                .map(|b| Button {
                    id: b.id,
                    label: b.label,
                })
                .collect(),
        });
    }

    fn start_progress(&mut self, id: String, title: String, cancellable: bool) {
        self.events.push(Event::ProgressStart {
            plugin: self.name.clone(),
            id,
            title,
            cancellable,
        });
    }

    fn report_progress(&mut self, id: String, message: String, done: Option<f32>) {
        self.events.push(Event::ProgressReport {
            plugin: self.name.clone(),
            id,
            message,
            done: done
                .filter(|d| d.is_finite())
                .map(|d| (d.clamp(0.0, 1.0) * 1000.0).round() as u16),
        });
    }

    fn end_progress(&mut self, id: String) {
        self.events.push(Event::ProgressEnd {
            plugin: self.name.clone(),
            id: Some(id),
        });
    }

    fn focus_view(&mut self, view: String) {
        self.events.push(Event::FocusView {
            plugin: self.name.clone(),
            view,
        });
    }

    fn set_image(&mut self, id: String, picture: Option<types::Picture>) -> Result<(), String> {
        let Some(picture) = picture else {
            self.images.remove(&id);
            self.events.push(Event::SetImage {
                plugin: self.name.clone(),
                id,
                image: None,
            });
            return Ok(());
        };
        let (width, height, pixels) = decode(picture)?;
        let others: usize = self
            .images
            .iter()
            .filter(|(k, _)| **k != id)
            .map(|(_, n)| n)
            .sum();
        if others + pixels.len() > PICTURE_BUDGET {
            return Err(format!(
                "its pictures would take {} MB, past the {} MB a plugin has",
                (others + pixels.len()) >> 20,
                PICTURE_BUDGET >> 20
            ));
        }
        self.images.insert(id.clone(), pixels.len());
        self.events.push(Event::SetImage {
            plugin: self.name.clone(),
            id,
            image: Some(Arc::new(merlin_core::PluginImage::new(
                width, height, pixels,
            ))),
        });
        Ok(())
    }

    fn request_frame(&mut self) {
        // One a call is enough.
        if !self
            .events
            .iter()
            .any(|e| matches!(e, Event::RequestFrame { .. }))
        {
            self.events.push(Event::RequestFrame {
                plugin: self.name.clone(),
            });
        }
    }

    fn set_view_badge(&mut self, view: String, count: u32) {
        self.events.push(Event::ViewBadge {
            plugin: self.name.clone(),
            view,
            count,
        });
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

    fn share(&mut self, key: String, value: Option<String>) -> Result<(), String> {
        if !self.shared_keys.provides.contains(&key) {
            return Err(format!(
                "{key:?} isn't in the manifest's shared.provides, so it can't be shared"
            ));
        }
        if let Some(value) = &value {
            if value.len() > MAX_SHARED {
                return Err(format!(
                    "a shared value is at most {} KB, and this is {} KB",
                    MAX_SHARED / 1024,
                    value.len().div_ceil(1024)
                ));
            }
            serde_json::from_str::<serde_json::Value>(value)
                .map_err(|e| format!("a shared value is JSON, and this isn't: {e}"))?;
        }
        if self.board.lock().unwrap().set(&key, &self.name, value) {
            self.processes.announce_shared(key);
        }
        Ok(())
    }

    fn shared(&mut self, key: String) -> Result<Vec<(String, String)>, String> {
        if !self.shared_keys.watches.contains(&key) {
            return Err(format!(
                "{key:?} isn't in the manifest's shared.watches, so it can't be read"
            ));
        }
        Ok(self.board.lock().unwrap().get(&key, &self.name))
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

    fn fetch(&mut self, request: types::FetchRequest) -> Result<u32, String> {
        if self.permissions.network.is_empty() {
            return Err("permission denied: network".into());
        }
        let allowed = self.permissions.network.clone();
        self.processes.fetch(request, allowed)
    }

    fn agent_say(&mut self, chat: u32, part: types::AgentPart) -> Result<(), String> {
        self.need(self.permissions.agent, "agent")?;
        self.agent(chat, merlin_core::AgentUpdate::Say(agent_part(part)));
        Ok(())
    }

    fn agent_header(
        &mut self,
        chat: u32,
        chips: Vec<types::Chip>,
        note: String,
    ) -> Result<(), String> {
        self.need(self.permissions.agent, "agent")?;
        let chips = convert_chips(chips);
        self.agent(chat, merlin_core::AgentUpdate::Header { chips, note });
        Ok(())
    }

    fn agent_commands(
        &mut self,
        chat: u32,
        commands: Vec<types::AgentCommand>,
    ) -> Result<(), String> {
        self.need(self.permissions.agent, "agent")?;
        let commands = commands
            .into_iter()
            .map(|c| (c.name.trim_start_matches('/').to_string(), c.description))
            .collect();
        self.agent(chat, merlin_core::AgentUpdate::Commands(commands));
        Ok(())
    }

    fn agent_modes(
        &mut self,
        chat: u32,
        modes: Vec<types::AgentMode>,
        current: String,
    ) -> Result<(), String> {
        self.need(self.permissions.agent, "agent")?;
        let modes = modes
            .into_iter()
            .map(|m| merlin_core::AgentMode {
                id: m.id,
                name: m.name,
                colour: m.colour,
                asks: match m.asks {
                    types::AgentAsks::Unsaid => merlin_core::AgentAsks::Unsaid,
                    types::AgentAsks::ReadsOnly => merlin_core::AgentAsks::ReadsOnly,
                    types::AgentAsks::AsksFirst => merlin_core::AgentAsks::AsksFirst,
                    types::AgentAsks::Edits => merlin_core::AgentAsks::Edits,
                    types::AgentAsks::EditsAndSafeCommands => {
                        merlin_core::AgentAsks::EditsAndSafeCommands
                    }
                    types::AgentAsks::Never => merlin_core::AgentAsks::Never,
                },
            })
            .collect();
        self.agent(chat, merlin_core::AgentUpdate::Modes { modes, current });
        Ok(())
    }

    fn agent_usage(&mut self, chat: u32, used: u64, size: u64, note: String) -> Result<(), String> {
        self.need(self.permissions.agent, "agent")?;
        self.agent(chat, merlin_core::AgentUpdate::Usage { used, size, note });
        Ok(())
    }

    fn agent_ask(&mut self, chat: u32, request: types::AgentRequest) -> Result<u32, String> {
        self.need(self.permissions.agent, "agent")?;
        let id = self.agent_id();
        let what = match request {
            types::AgentRequest::Edit((path, diff)) => {
                merlin_core::AgentRequest::Edit { path, diff }
            }
            types::AgentRequest::Run(c) => merlin_core::AgentRequest::Run(c),
            types::AgentRequest::Read(p) => merlin_core::AgentRequest::Read(p),
            types::AgentRequest::Other(w) => merlin_core::AgentRequest::Other(w),
            types::AgentRequest::Mode(m) => merlin_core::AgentRequest::Mode(m),
        };
        self.agent(chat, merlin_core::AgentUpdate::Ask { request: id, what });
        Ok(id)
    }

    fn agent_done(&mut self, chat: u32, outcome: types::AgentOutcome, message: Option<String>) {
        if !self.permissions.agent {
            return;
        }
        let outcome = match outcome {
            types::AgentOutcome::Finished => merlin_core::AgentOutcome::Finished,
            types::AgentOutcome::Cancelled => merlin_core::AgentOutcome::Cancelled,
            types::AgentOutcome::Failed => merlin_core::AgentOutcome::Failed,
        };
        self.agent(chat, merlin_core::AgentUpdate::Done { outcome, message });
    }

    fn agent_read(&mut self, chat: u32, path: String) -> Result<u32, String> {
        self.need(self.permissions.agent, "agent")?;
        let request = self.agent_id();
        self.agent(chat, merlin_core::AgentUpdate::Read { request, path });
        Ok(request)
    }

    fn agent_write(&mut self, chat: u32, path: String, text: String) -> Result<u32, String> {
        self.need(self.permissions.agent, "agent")?;
        let request = self.agent_id();
        self.agent(
            chat,
            merlin_core::AgentUpdate::Write {
                request,
                path,
                text,
            },
        );
        Ok(request)
    }

    fn agent_run(
        &mut self,
        chat: u32,
        command: String,
        cwd: Option<String>,
    ) -> Result<u32, String> {
        self.need(self.permissions.agent, "agent")?;
        let run = self.agent_id();
        self.agent_runs.insert(run, chat);
        self.agent(chat, merlin_core::AgentUpdate::Run { run, command, cwd });
        Ok(run)
    }

    fn agent_stop(&mut self, run: u32) {
        if let Some(&chat) = self.agent_runs.get(&run) {
            self.agent(chat, merlin_core::AgentUpdate::Stop { run });
        }
    }
}

impl State {
    fn agent_id(&mut self) -> u32 {
        self.next_agent_id += 1;
        self.next_agent_id
    }

    fn agent(&mut self, chat: u32, update: merlin_core::AgentUpdate) {
        self.events.push(Event::Agent {
            plugin: self.name.clone(),
            chat,
            update,
            at: now_ms(),
        });
    }
}

fn step_state(state: types::StepState) -> merlin_core::StepState {
    match state {
        types::StepState::Pending => merlin_core::StepState::Pending,
        types::StepState::Running => merlin_core::StepState::Running,
        types::StepState::Done => merlin_core::StepState::Done,
        types::StepState::Failed => merlin_core::StepState::Failed,
    }
}

fn agent_part(part: types::AgentPart) -> merlin_core::AgentPart {
    match part {
        types::AgentPart::Text(t) => merlin_core::AgentPart::Text(t),
        types::AgentPart::Thought(t) => merlin_core::AgentPart::Thought(t),
        types::AgentPart::Plan(steps) => merlin_core::AgentPart::Plan(
            steps
                .into_iter()
                .map(|s| merlin_core::PlanStep {
                    text: s.text,
                    state: step_state(s.state),
                })
                .collect(),
        ),
        types::AgentPart::Tool(t) => merlin_core::AgentPart::Tool(merlin_core::AgentTool {
            id: t.id,
            kind: match t.kind {
                types::ToolKind::Read => merlin_core::ToolKind::Read,
                types::ToolKind::Edit => merlin_core::ToolKind::Edit,
                types::ToolKind::Run => merlin_core::ToolKind::Run,
                types::ToolKind::Search => merlin_core::ToolKind::Search,
                types::ToolKind::Other => merlin_core::ToolKind::Other,
            },
            title: t.title,
            state: step_state(t.state),
            added: t.added,
            removed: t.removed,
            run: t.run,
        }),
        types::AgentPart::Panel(nodes) => merlin_core::AgentPart::Panel(
            convert_panel(types::Panel {
                title: String::new(),
                nodes,
            })
            .nodes,
        ),
    }
}

/// Chips as a panel's: the plugin's ids, merlin's tones.
fn convert_chips(chips: Vec<types::Chip>) -> Vec<merlin_core::Chip> {
    let panel = convert_panel(types::Panel {
        title: String::new(),
        nodes: vec![types::Node::Chips(types::ChipRow {
            chips,
            hint: String::new(),
        })],
    });
    match panel.nodes.into_iter().next() {
        Some(merlin_core::Node::Chips { chips, .. }) => chips,
        _ => Vec::new(),
    }
}

fn log_level(level: types::LogLevel) -> LogLevel {
    match level {
        types::LogLevel::Trace => LogLevel::Trace,
        types::LogLevel::Debug => LogLevel::Debug,
        types::LogLevel::Info => LogLevel::Info,
        types::LogLevel::Warn => LogLevel::Warn,
        types::LogLevel::Error => LogLevel::Error,
    }
}

impl State {
    /// One of the plugin's own settings, as text, if it is text.
    fn text_setting(&self, key: &str) -> Option<String> {
        self.settings.get(key)?.as_str().map(str::to_string)
    }

    /// The least a `log` line must be to keep: the plugin's `log-level`
    /// setting, or info.
    fn log_level(&self) -> LogLevel {
        self.text_setting("log-level")
            .and_then(|l| LogLevel::parse(&l))
            .unwrap_or(LogLevel::Info)
    }
}

pub(super) fn convert_panel(panel: types::Panel) -> Panel {
    use merlin_core::style::Hue;
    use merlin_core::{Chip, EditCard, Entry, Message};
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
    let chip = |c: types::Chip| Chip {
        id: c.id,
        label: c.label,
        tone: tone(c.tone),
    };
    // A row of a tree: its arrow, its glyph, and at its right its count,
    // or its first chip; the other chips follow its label.
    let row = |r: types::Row| {
        let mut chips = r.chips.into_iter().map(chip);
        let (badge, badge_tone) = if r.count.is_empty() {
            chips
                .next()
                .map_or((String::new(), Tone::Normal), |c| (c.label, c.tone))
        } else {
            (r.count, Tone::Muted)
        };
        let rest: Vec<Chip> = chips.collect();
        Node::entry(Entry {
            id: r.id,
            label: r.label,
            detail: r.detail,
            depth: r.depth.min(16) as usize,
            twisty: r.expanded.map(|open| if open { '▾' } else { '▸' }),
            icon: r.icon.chars().next().map(|c| (c, Hue::Grey)),
            badge,
            badge_tone,
            note: rest.first().map(|c| {
                let labels: Vec<&str> = rest.iter().map(|c| c.label.as_str()).collect();
                (labels.join(" · "), c.tone)
            }),
            menu: r.menu.into_iter().map(button).collect(),
            ..Entry::default()
        })
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
                    multiline: i.multiline,
                },
                types::Node::Chips(row) => Node::Chips {
                    chips: row.chips.into_iter().map(chip).collect(),
                    hint: row.hint,
                },
                types::Node::Row(r) => row(r),
                types::Node::Message(m) => Node::Message(Box::new(Message {
                    author: m.author,
                    mine: m.mine,
                    markdown: m.markdown,
                    chips: m.chips.into_iter().map(chip).collect(),
                    streaming: m.streaming,
                })),
                types::Node::EditCard(e) => Node::Edit(Box::new(EditCard {
                    path: e.path,
                    added: e.added,
                    removed: e.removed,
                    buttons: e.buttons.into_iter().map(button).collect(),
                })),
                types::Node::Progress(p) => Node::Progress {
                    label: p.label,
                    detail: p.detail,
                    done: p
                        .done
                        .filter(|d| d.is_finite())
                        .map(|d| (d.clamp(0.0, 1.0) * 1000.0).round() as u16),
                },
                types::Node::EmptyState(e) => Node::Empty {
                    title: e.title,
                    text: e.text,
                    buttons: e.buttons.into_iter().map(button).collect(),
                },
                types::Node::Image(i) => Node::Image(Box::new(merlin_core::ImageNode {
                    id: i.id,
                    alt: i.alt,
                    image: None,
                })),
                types::Node::Drawing(d) => Node::Drawing(Box::new(drawing(d))),
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

/// A plugin's drawing, its numbers made finite.
fn drawing(d: types::Drawing) -> merlin_core::Drawing {
    use merlin_core::{DrawColour, DrawItem, Shape};
    let n = |v: f32| if v.is_finite() { v } else { 0.0 };
    let pt = |p: types::DrawPoint| [n(p.x), n(p.y)];
    let colour = |c: types::Colour| match c {
        types::Colour::Theme(name) => DrawColour::Theme(name),
        types::Colour::Rgb(rgb) => DrawColour::Rgb(rgb & 0xff_ffff),
    };
    merlin_core::Drawing {
        width: n(d.width).max(1.0),
        height: n(d.height).max(0.0),
        items: d
            .items
            .into_iter()
            .map(|item| DrawItem {
                id: item.id,
                shape: match item.shape {
                    types::Shape::Rect(r) => Shape::Rect {
                        rect: [n(r.x), n(r.y), n(r.width), n(r.height)],
                        radius: n(r.radius),
                        fill: r.fill.map(colour),
                        stroke: r.stroke.map(colour),
                    },
                    types::Shape::Line(l) => Shape::Line {
                        from: pt(l.start),
                        to: pt(l.end),
                        width: n(l.width),
                        colour: colour(l.colour),
                    },
                    types::Shape::Curve(c) => Shape::Curve {
                        points: c.points.into_iter().map(pt).collect(),
                        width: n(c.width),
                        colour: colour(c.colour),
                    },
                    types::Shape::Text(t) => Shape::Text {
                        at: [n(t.x), n(t.y)],
                        text: t.text,
                        size: n(t.size),
                        colour: colour(t.colour),
                        mono: t.mono,
                    },
                    types::Shape::Image(i) => Shape::Image {
                        rect: [n(i.x), n(i.y), n(i.width), n(i.height)],
                        id: i.id,
                        image: None,
                    },
                },
            })
            .collect(),
    }
}
