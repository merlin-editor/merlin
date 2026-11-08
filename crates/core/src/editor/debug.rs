//! Debugging, driven by plugins: breakpoints, the Run and Debug view, the
//! Debug Console, and a run's state as its debugger plugin reports it.
//!
//! Plugins declare debuggers in their manifests, and list what can be
//! debugged with `set-debug-targets`. F5 picks a target, runs its build,
//! and asks the debugger's driver, the Debugger plugin for DAP ones, to
//! start it. The driver reports back with [`DebugUpdate`]s, and the user's
//! buttons and keys go to it as commands.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::*;
use crate::panel::{Button, Entry, Node, Tone};
use crate::style::Hue;

mod breakpoints;
mod inline;
mod launch;
mod view;
use view::*;

/// The Run and Debug side view.
pub(crate) const DEBUG: &str = "merlin.debug";
/// The Debug Console, in the bottom panel.
pub(crate) const CONSOLE: &str = "merlin.debug-console";
/// The plugin that drives a debugger unless its manifest names another.
pub const DAP: &str = "dap";
/// Your own configurations.
pub(crate) const LAUNCH_FILE: &str = ".merlin/launch.toml";
/// What a new launch.toml starts with.
const EXAMPLE: &str = "\
# What Run and Debug can start, besides what plugins find. Keys merlin
# doesn't know go to the debugger as they are.
[[debug]]
name = \"my program\"
program = \"target/debug/my-program\"
args = []
# A task's name or a command, run first.
build = \"cargo build\"
# env = { RUST_LOG = \"info\" }

# [[debug]]
# name = \"attach\"
# attach = 12345        # a process id
";
/// Lines the Debug Console keeps.
const CONSOLE_LINES: usize = 2000;

/// A debugger a plugin's manifest declares.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Debugger {
    pub id: String,
    pub name: String,
    /// The plugin that declares it.
    pub plugin: String,
    pub languages: Vec<String>,
    /// The program and its arguments.
    pub command: Vec<String>,
    /// How to install it, as a command to copy.
    pub install: String,
    /// Launch settings sent with every start, as a JSON object.
    pub launch: String,
    /// The plugin that speaks to it: [`DAP`], or the declaring plugin for
    /// one that isn't DAP.
    pub driver: String,
    /// More it can stop on, as functions: `on` is the default.
    pub stops: Vec<FunctionStop>,
}

/// Something a debugger can stop on that's a function to break in, such
/// as `rust_panic`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FunctionStop {
    pub id: String,
    pub label: String,
    pub functions: Vec<String>,
    pub on: bool,
}

/// Something that can be debugged.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DebugTarget {
    pub name: String,
    pub debugger: String,
    pub language: String,
    pub program: String,
    pub args: Vec<String>,
    pub build: String,
    pub detail: String,
    pub launch: String,
    /// The plugin that found it.
    pub plugin: Option<String>,
}

/// A breakpoint the user set. Lines are from 0.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Breakpoint {
    pub id: u32,
    pub path: PathBuf,
    pub line: u32,
    pub condition: String,
    pub hit: String,
    pub log: String,
    pub enabled: bool,
}

/// What a driver is asked to start.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct DebugStart {
    pub name: String,
    pub debugger: String,
    pub command: Vec<String>,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: Vec<(String, String)>,
    pub launch: String,
    pub stop_on: Vec<String>,
    pub function_stops: Vec<FunctionStop>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugState {
    Starting,
    Running,
    Paused {
        reason: String,
        description: String,
        thread: u32,
    },
    Ended(Option<i32>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugThread {
    pub id: u32,
    pub name: String,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackFrame {
    pub id: u32,
    pub name: String,
    pub path: Option<PathBuf>,
    pub line: u32,
    pub column: u32,
    pub library: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugVariable {
    pub name: String,
    pub value: String,
    pub kind: String,
    pub children: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakpointState {
    pub id: u32,
    pub set: bool,
    pub line: u32,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopFilter {
    pub id: String,
    pub label: String,
    pub on: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Console,
    Stdout,
    Stderr,
    Important,
}

/// What a driver reports about its run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DebugUpdate {
    State(DebugState),
    Threads(Vec<DebugThread>),
    Stack {
        thread: u32,
        frames: Vec<StackFrame>,
    },
    Variables {
        parent: u32,
        variables: Vec<DebugVariable>,
    },
    Breakpoints(Vec<BreakpointState>),
    StopFilters(Vec<StopFilter>),
    InlineValues {
        path: PathBuf,
        values: Vec<(u32, String)>,
    },
    Output(OutputKind, String),
    /// A watch expression's value, by its place in Watch.
    Watch {
        index: u32,
        value: DebugVariable,
    },
}

/// The right-click menu on the breakpoint room.
pub(crate) const BREAK_ITEMS: &[(&str, &str)] = &[
    ("debug.var_set", "Set Value…"),
    ("debug.var_copy", "Copy Value"),
    ("debug.var_watch", "Add to Watch"),
    ("debug.bp_add", "Add Breakpoint"),
    ("debug.bp_condition", "Add Conditional Breakpoint…"),
    ("debug.bp_log", "Add Logpoint…"),
    ("debug.bp_edit", "Edit Breakpoint…"),
    ("debug.bp_disable", "Disable Breakpoint"),
    ("debug.bp_enable", "Enable Breakpoint"),
    ("debug.bp_remove", "Remove Breakpoint"),
];

/// What the breakpoint box sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakField {
    Condition,
    Hit,
    Log,
}

impl BreakField {
    fn label(self) -> &'static str {
        match self {
            BreakField::Condition => "Condition",
            BreakField::Hit => "Hit Count",
            BreakField::Log => "Log Message",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            BreakField::Condition => "Stops when this is true",
            BreakField::Hit => {
                "A count of hits, as the debugger takes it: gdb stops after skipping that many"
            }
            BreakField::Log => "Prints this, with {names} filled in, and doesn't stop",
        }
    }
}

/// The box between the lines that edits a breakpoint.
#[derive(Debug)]
pub(crate) struct BreakBox {
    pub doc: u64,
    pub line: u32,
    pub field: BreakField,
    pub text: String,
    pub caret: usize,
}

/// The breakpoint box, to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakBoxView {
    pub field: String,
    pub text: String,
    pub caret: usize,
    pub hint: String,
}

/// A breakpoint's mark in the gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakMark {
    Set,
    /// With a condition or a hit count.
    Conditional,
    /// Logs and doesn't stop.
    Log,
    /// The debugger couldn't set it.
    Unset,
    Disabled,
}

/// The debug toolbar, while a run is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebugBarView {
    pub paused: bool,
    /// Building first: only Stop works.
    pub building: bool,
}

/// A button of the debug toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugButton {
    Continue,
    Pause,
    StepOver,
    StepInto,
    StepOut,
    Restart,
    Stop,
}

impl DebugButton {
    pub fn command(self) -> &'static str {
        match self {
            DebugButton::Continue => "debug.continue",
            DebugButton::Pause => "debug.pause",
            DebugButton::StepOver => "debug.step_over",
            DebugButton::StepInto => "debug.step_into",
            DebugButton::StepOut => "debug.step_out",
            DebugButton::Restart => "debug.restart",
            DebugButton::Stop => "debug.stop",
        }
    }
}

/// A run, from F5 until it ends.
#[derive(Debug)]
pub(crate) struct Session {
    pub driver: String,
    pub start: DebugStart,
    pub state: DebugState,
    /// The build's terminal while it runs first.
    pub building: Option<u64>,
    pub threads: Vec<DebugThread>,
    pub stacks: HashMap<u32, Vec<StackFrame>>,
    /// The frame shown, by thread and frame id.
    pub frame: Option<(u32, u32)>,
    pub variables: HashMap<u32, Vec<DebugVariable>>,
    pub inline: HashMap<PathBuf, Vec<(u32, String)>>,
    /// Each watch's value at this pause, by its place in Watch.
    pub watched: Vec<Option<DebugVariable>>,
}

#[derive(Debug, Default)]
pub(crate) struct Debugging {
    pub debuggers: Vec<Debugger>,
    /// Programs found on `PATH`, by name, once looked for.
    pub installed: HashMap<String, bool>,
    pub targets: Vec<(String, Vec<DebugTarget>)>,
    /// launch.toml's, and why it couldn't be read.
    pub launch: Vec<DebugTarget>,
    pub launch_error: Option<String>,
    /// F5 waits for launch.toml to be read.
    pub reading: bool,
    /// Add a Configuration opened launch.toml: it gets the example if new.
    pub example: bool,
    /// The target F5 starts, by name, once chosen.
    pub chosen: Option<String>,
    pub breakpoints: Vec<Breakpoint>,
    next_id: u32,
    pub states: HashMap<u32, BreakpointState>,
    pub session: Option<Session>,
    /// The last run, kept after it ends for Restart and the console.
    pub last: Option<(String, DebugStart)>,
    pub console: Vec<(OutputKind, String)>,
    pub filters: Vec<StopFilter>,
    /// Stop filters the user turned on, and off, by id, kept across runs.
    pub stop_on: HashSet<String>,
    pub stop_off: HashSet<String>,
    pub expanded: HashSet<u32>,
    /// The line the breakpoint menu was opened on.
    pub menu_line: Option<u32>,
    pub break_box: Option<BreakBox>,
    /// Watch's expressions, kept across runs.
    pub watches: Vec<String>,
    /// A value was set: the watches are asked again once it's read back.
    pub rewatch: bool,
    /// The variable whose value is being set: what lists it, and its name.
    pub setting: Option<(u32, String)>,
    /// The variable the menu was opened on: what lists it, its name and
    /// its value.
    pub menu_var: Option<(u32, String, String)>,
    /// The variable the hover card is about, by name.
    pub hover: Option<String>,
    /// Linux's ptrace_scope, as last read.
    pub ptrace_scope: u8,
    /// A run whose build failed, for Debug the Last Build.
    pub failed: Option<Session>,
    /// Ctrl+F5 is waiting for launch.toml.
    pub run_plain: bool,
    /// Restart asked for: the run starts again once it ends.
    pub restart: bool,
    /// Library frames unfolded in Call Stack, by thread.
    pub unfolded: HashSet<u32>,
}

impl Debugging {
    fn breakpoint_at(&self, path: &Path, line: u32) -> Option<usize> {
        self.breakpoints
            .iter()
            .position(|b| b.path == path && b.line == line)
    }

    /// Every target: plugins', in the order they came.
    pub(crate) fn all_targets(&self) -> impl Iterator<Item = &DebugTarget> {
        self.launch
            .iter()
            .chain(self.targets.iter().flat_map(|(_, t)| t.iter()))
    }

    /// The debuggers that can debug a target, in the manifest's order,
    /// installed ones first.
    fn debuggers_for(&self, target: &DebugTarget) -> Vec<&Debugger> {
        let mut list: Vec<&Debugger> = self
            .debuggers
            .iter()
            .filter(|d| {
                if target.debugger.is_empty() {
                    d.languages.contains(&target.language)
                } else {
                    d.id == target.debugger
                }
            })
            .collect();
        list.sort_by_key(|d| !self.is_installed(d));
        list
    }

    /// Whether a debugger's program was found. One a plugin drives itself,
    /// with no command, always is.
    fn is_installed(&self, d: &Debugger) -> bool {
        match d.command.first() {
            None => true,
            Some(program) => self.installed.get(program).copied().unwrap_or(true),
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| !matches!(s.state, DebugState::Ended(_)))
    }

    pub(crate) fn paused(&self) -> Option<u32> {
        match &self.session.as_ref()?.state {
            DebugState::Paused { thread, .. } => Some(*thread),
            _ => None,
        }
    }

    /// The frame shown while paused: the chosen one, or the paused
    /// thread's top frame.
    pub(crate) fn current_frame(&self) -> Option<&StackFrame> {
        let session = self.session.as_ref()?;
        let thread = self.paused()?;
        let frames = session.stacks.get(&thread)?;
        match session.frame {
            Some((t, id)) => session
                .stacks
                .get(&t)?
                .iter()
                .find(|f| f.id == id)
                .or(frames.first()),
            None => frames.iter().find(|f| !f.library).or(frames.first()),
        }
    }

    fn push_console(&mut self, kind: OutputKind, text: &str) {
        // Output comes in pieces; a piece without a line break joins the
        // next.
        let mut rest = text;
        if let Some((last_kind, last)) = self.console.last_mut()
            && *last_kind == kind
            && last.ends_with('\u{0}')
        {
            last.pop();
            match rest.split_once('\n') {
                Some((head, tail)) => {
                    last.push_str(head);
                    rest = tail;
                }
                None => {
                    last.push_str(rest);
                    last.push('\u{0}');
                    return;
                }
            }
        }
        let mut lines: Vec<&str> = rest.split('\n').collect();
        let open = !rest.ends_with('\n');
        if !open {
            lines.pop();
        }
        let count = lines.len();
        for (i, line) in lines.into_iter().enumerate() {
            let mut line = line.trim_end_matches('\r').to_string();
            if open && i + 1 == count {
                line.push('\u{0}');
            }
            self.console.push((kind, line));
        }
        if self.console.len() > CONSOLE_LINES {
            let extra = self.console.len() - CONSOLE_LINES;
            self.console.drain(..extra);
        }
    }
}

impl Editor {
    pub(crate) fn debug_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        match id {
            "debug.start" => self.debug_read_then(false, fx),
            "debug.run" => self.debug_read_then(true, fx),
            "debug.attach" => fx.push(Effect::ListProcesses),
            "debug.last_build" => {
                if let Some(session) = self.debug.failed.take()
                    && !self.debug.running()
                {
                    self.message = None;
                    self.debug.session = Some(session);
                    self.start_driver(fx);
                }
            }
            "debug.ptrace_help" => fx.push(Effect::OpenUrl(
                "https://www.kernel.org/doc/html/latest/admin-guide/LSM/Yama.html".into(),
            )),
            "debug.configure" => {
                self.debug.example = true;
                self.open(PathBuf::from(LAUNCH_FILE), fx);
            }
            "debug.pick" => self.debug_pick(),
            "debug.toggle_breakpoint" => {
                let line = self.doc().line_col(self.cursor()).0 as u32;
                if let Some(path) = self.doc().buffer.path.clone() {
                    self.toggle_breakpoint(&path, line, fx);
                }
            }
            "debug.remove_breakpoints" => {
                self.debug.breakpoints.clear();
                self.debug.states.clear();
                self.breakpoints_changed(fx);
            }
            "debug.toggle_breakpoints" => {
                let on = self.debug.breakpoints.iter().any(|b| !b.enabled);
                for b in &mut self.debug.breakpoints {
                    b.enabled = on;
                }
                self.breakpoints_changed(fx);
            }
            "debug.bp_add" | "debug.bp_remove" => {
                if let (Some(line), Some(path)) =
                    (self.debug.menu_line, self.doc().buffer.path.clone())
                {
                    self.toggle_breakpoint(&path, line, fx);
                }
            }
            "debug.bp_condition" => self.open_break_box(BreakField::Condition),
            "debug.bp_log" => self.open_break_box(BreakField::Log),
            "debug.bp_edit" => {
                let field = self.menu_breakpoint().map_or(BreakField::Condition, |b| {
                    if !b.log.is_empty() {
                        BreakField::Log
                    } else if !b.hit.is_empty() && b.condition.is_empty() {
                        BreakField::Hit
                    } else {
                        BreakField::Condition
                    }
                });
                self.open_break_box(field);
            }
            "debug.bp_disable" | "debug.bp_enable" => {
                let on = id == "debug.bp_enable";
                if let Some(i) = self.menu_breakpoint_index() {
                    self.debug.breakpoints[i].enabled = on;
                    self.breakpoints_changed(fx);
                }
            }
            "debug.var_set" | "debug.var_copy" | "debug.var_watch" => self.var_command(id, fx),
            "view.debug" => self.show_view(DEBUG, fx),
            "view.debug_console" => self.show_bottom(CONSOLE),
            "debug.stop" if self.debug_building().is_some() => {
                if let Some(term) = self.debug_building() {
                    self.kill_terminal(term, fx);
                }
                self.debug.session = None;
                self.build_views();
            }
            "debug.restart" => {
                if self.debug.running() {
                    // Stopped and started again, so the build runs too.
                    self.debug.restart = true;
                    self.send_debug("debug.stop", fx);
                } else if let Some((target, _)) = self.debug.last.clone() {
                    self.debug.chosen = Some(target);
                    self.debug_start(false, fx);
                }
            }
            "debug.continue" if !self.debug.running() => self.debug_read_then(false, fx),
            _ if self.debug.running() => self.send_debug(id, fx),
            _ => {}
        }
    }

    fn debug_building(&self) -> Option<u64> {
        self.debug.session.as_ref()?.building
    }

    /// Whether a run is building or its debugger starting.
    pub(crate) fn debug_busy(&self) -> bool {
        self.debug
            .session
            .as_ref()
            .is_some_and(|s| s.building.is_some() || s.state == DebugState::Starting)
    }

    /// Sends a command to the driver of the run.
    fn send_debug(&mut self, command: &str, fx: &mut Vec<Effect>) {
        let Some(session) = &self.debug.session else {
            return;
        };
        fx.push(Effect::Debug {
            plugin: session.driver.clone(),
            command: command.replace('_', "-"),
            start: None,
            breakpoints: self.debug.breakpoints.clone(),
        });
    }

    /// Whether two paths are the same file, one perhaps relative to the
    /// project.
    fn same_file(&self, a: &Path, b: &Path) -> bool {
        let full = |p: &Path| match p.is_absolute() {
            true => p.to_path_buf(),
            false => self.root.join(p),
        };
        a == b || full(a) == full(b)
    }

    fn language_name(&self, id: &str) -> String {
        self.languages
            .iter()
            .find(|(_, l)| l.id == id)
            .map_or(id.to_string(), |(_, l)| l.name.clone())
    }

    pub(crate) fn register_debuggers(
        &mut self,
        plugin: String,
        debuggers: Vec<Debugger>,
        fx: &mut Vec<Effect>,
    ) {
        self.debug.debuggers.retain(|d| d.plugin != plugin);
        let programs: Vec<String> = debuggers
            .iter()
            .filter_map(|d| d.command.first().cloned())
            .filter(|p| !self.debug.installed.contains_key(p))
            .collect();
        self.debug.debuggers.extend(debuggers);
        if !programs.is_empty() {
            fx.push(Effect::LocateDebuggers(programs));
        }
    }

    pub(crate) fn set_debug_targets(&mut self, plugin: String, mut targets: Vec<DebugTarget>) {
        for t in &mut targets {
            t.plugin = Some(plugin.clone());
        }
        self.debug.targets.retain(|(p, _)| *p != plugin);
        if !targets.is_empty() {
            self.debug.targets.push((plugin, targets));
        }
        self.build_views();
    }

    pub(crate) fn debug_update(&mut self, plugin: &str, update: DebugUpdate, fx: &mut Vec<Effect>) {
        let Some(session) = &mut self.debug.session else {
            // Output after the end still reaches the console.
            if let DebugUpdate::Output(kind, text) = update {
                self.debug.push_console(kind, &text);
                self.build_views();
            }
            return;
        };
        if session.driver != plugin {
            return;
        }
        match update {
            DebugUpdate::State(state) => {
                let paused = matches!(state, DebugState::Paused { .. });
                if !paused {
                    session.watched.clear();
                    session.stacks.clear();
                    session.variables.clear();
                    session.inline.clear();
                    session.frame = None;
                }
                if let DebugState::Ended(code) = state {
                    let name = session.start.name.clone();
                    let line = match code {
                        Some(c) => format!("{name} ended with code {c}"),
                        None => format!("{name} ended"),
                    };
                    let kind = match code {
                        Some(0) | None => OutputKind::Console,
                        _ => OutputKind::Stderr,
                    };
                    self.debug.push_console(kind, &format!("{line}\n"));
                    self.debug.session = None;
                    self.debug.filters.clear();
                    if std::mem::take(&mut self.debug.restart)
                        && let Some((target, _)) = self.debug.last.clone()
                    {
                        self.debug.chosen = Some(target);
                        self.debug_start(false, fx);
                    }
                    self.build_views();
                    return;
                }
                session.state = state;
                if paused {
                    self.debug.expanded.clear();
                }
            }
            DebugUpdate::Threads(threads) => session.threads = threads,
            DebugUpdate::Stack { thread, frames } => {
                let first = session.stacks.is_empty();
                session.stacks.insert(thread, frames);
                // The paused thread's frame shows its file, as VS Code does.
                if first && self.debug.paused() == Some(thread) {
                    self.reveal_frame(fx);
                }
            }
            DebugUpdate::Variables { parent, variables } => {
                if std::mem::take(&mut self.debug.rewatch) {
                    self.ask_watches(fx);
                }
                if parent == 0 {
                    // The frame is ready: the watches are asked about it.
                    self.ask_watches(fx);
                    // Locals open, to show what's in them at once.
                    if let Some(first) = variables.first() {
                        self.debug.expanded.insert(first.children);
                    }
                }
                let session = self.debug.session.as_mut().expect("checked");
                session.variables.insert(parent, variables);
            }
            DebugUpdate::Breakpoints(states) => {
                for s in states {
                    if let Some(b) = self.debug.breakpoints.iter_mut().find(|b| b.id == s.id)
                        && s.set
                        && s.line != u32::MAX
                    {
                        // The debugger moved it to the nearest line with code.
                        b.line = s.line;
                    }
                    self.debug.states.insert(s.id, s);
                }
            }
            DebugUpdate::StopFilters(filters) => {
                self.debug.filters = filters;
                for f in &self.debug.filters {
                    if f.on {
                        self.debug.stop_on.insert(f.id.clone());
                        self.debug.stop_off.remove(&f.id);
                    } else {
                        self.debug.stop_on.remove(&f.id);
                        self.debug.stop_off.insert(f.id.clone());
                    }
                }
            }
            DebugUpdate::InlineValues { path, values } => {
                session.inline.insert(path, values);
            }
            DebugUpdate::Output(kind, text) => self.debug.push_console(kind, &text),
            DebugUpdate::Watch { index, value } => {
                let i = index as usize;
                if session.watched.len() <= i {
                    session.watched.resize(i + 1, None);
                }
                session.watched[i] = Some(value);
            }
        }
        self.build_views();
    }

    /// Opens the shown frame's file at its line.
    fn reveal_frame(&mut self, fx: &mut Vec<Effect>) {
        let Some(frame) = self.debug.current_frame().cloned() else {
            return;
        };
        if let Some(path) = frame.path {
            let path = path
                .strip_prefix(&self.root)
                .map(Path::to_path_buf)
                .unwrap_or(path);
            self.open_at(path, frame.line, frame.column, fx);
        }
    }

    /// Keys while a run is on, as VS Code has them: F5 continues, F6
    /// pauses, F10, F11 and Shift+F11 step, Shift+F5 stops.
    pub(crate) fn debug_key(&self, key: &Key) -> Option<&'static str> {
        if !self.debug.running() && self.debug_building().is_none() {
            return None;
        }
        let paused = self.debug.paused().is_some();
        Some(match (key.code, key.shift, key.ctrl) {
            (KeyCode::F(5), true, true) => "debug.restart",
            (KeyCode::F(5), true, false) => "debug.stop",
            (KeyCode::F(6), false, false) if !paused => "debug.pause",
            (KeyCode::F(10), false, false) if paused => "debug.step_over",
            (KeyCode::F(11), false, false) if paused => "debug.step_into",
            (KeyCode::F(11), true, false) if paused => "debug.step_out",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::launch::*;
    use super::*;

    #[test]
    fn launch_settings_merge_later_over_earlier() {
        let merged = merge_json(&[
            r#"{"program":"a","args":["x, y"],"cwd":"/r"}"#,
            r#"{"stopOnEntry":false}"#,
            r#"{"program":"b","env":{"A":"1,2"}}"#,
            "not json",
        ]);
        assert_eq!(
            merged,
            r#"{"program":"b","args":["x, y"],"cwd":"/r","stopOnEntry":false,"env":{"A":"1,2"}}"#
        );
    }

    #[test]
    fn console_output_in_pieces_joins_its_lines() {
        let mut d = Debugging::default();
        d.push_console(OutputKind::Stdout, "0: #a6");
        d.push_console(OutputKind::Stdout, "e3a1\n1: #89b4fa\n");
        d.push_console(OutputKind::Stderr, "boom\n");
        let lines: Vec<&str> = d.console.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(lines, ["0: #a6e3a1", "1: #89b4fa", "boom"]);
    }

    #[test]
    fn a_plugin_starts_debugging_or_its_fallback_without_a_debugger() {
        let mut e = Editor::new("fn main() {}\n", Some(PathBuf::from("src/main.rs")));
        let target = DebugTarget {
            name: "Test parses".into(),
            language: "rust".into(),
            program: "target/debug/deps/demo-1a2b".into(),
            args: vec!["parses".into()],
            ..Default::default()
        };
        let fx = e.update(Event::StartDebugging {
            plugin: "rust".into(),
            target: Box::new(target.clone()),
            fallback: Some("rust-gdb --args target/debug/deps/demo-1a2b parses".into()),
        });
        assert!(
            fx.iter().any(|f| matches!(f, Effect::Terminal(_))),
            "without a debugger, the fallback runs in a terminal: {fx:?}"
        );
        e.update(Event::RegisterDebuggers {
            plugin: "rust".into(),
            debuggers: vec![Debugger {
                id: "gdb".into(),
                name: "gdb".into(),
                languages: vec!["rust".into()],
                command: vec!["gdb".into()],
                launch: "{}".into(),
                driver: "dap".into(),
                ..Default::default()
            }],
        });
        e.update(Event::RegisterPlugin {
            plugin: "dap".into(),
            commands: Vec::new(),
        });
        let fx = e.update(Event::StartDebugging {
            plugin: "rust".into(),
            target: Box::new(target),
            fallback: None,
        });
        let start = fx.iter().find_map(|f| match f {
            Effect::Debug {
                start: Some(start), ..
            } => Some(start.clone()),
            _ => None,
        });
        let start = start.expect("the run starts");
        assert!(
            start.program.ends_with("target/debug/deps/demo-1a2b"),
            "{}",
            start.program
        );
        assert!(
            start.launch.contains("\"args\":[\"parses\"]"),
            "{}",
            start.launch
        );
    }

    #[test]
    fn types_drop_their_paths() {
        assert_eq!(
            short_type("core::array::iter::IntoIter<&str, 3>"),
            "IntoIter<&str, 3>"
        );
        assert_eq!(short_type("usize"), "usize");
        assert_eq!(
            short_type("core::iter::adapters::enumerate::Enumerate<core::slice::iter::Iter<&str>>"),
            "Enumerate<Iter<&str>>"
        );
    }

    #[test]
    fn json_strings_escape_quotes_and_breaks() {
        assert_eq!(json_string("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
    }
}
