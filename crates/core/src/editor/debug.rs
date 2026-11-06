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
    /// A test's file, when its binary is found in what the build printed.
    pub binary_for: Option<String>,
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

/// Merges JSON objects, later keys over earlier ones; text that isn't an
/// object is left out.
fn merge_json(parts: &[&str]) -> String {
    let mut out = String::from("{");
    let mut first = true;
    let mut seen: Vec<String> = Vec::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for part in parts {
        for (key, value) in json_fields(part) {
            match seen.iter().position(|k| *k == key) {
                Some(i) => fields[i].1 = value,
                None => {
                    seen.push(key.clone());
                    fields.push((key, value));
                }
            }
        }
    }
    for (key, value) in fields {
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!("{key}:{value}"));
    }
    out.push('}');
    out
}

/// The top-level fields of a JSON object, each as its key (quoted) and its
/// value's text. Enough to merge objects without a JSON library.
fn json_fields(text: &str) -> Vec<(String, String)> {
    let text = text.trim();
    let Some(body) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0;
    let mut parts = Vec::new();
    for (i, c) in body.char_indices() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        // The key is a string, so the first colon outside it splits.
        let mut quoted = false;
        let mut escaped = false;
        let split = part.char_indices().find(|&(_, c)| {
            if quoted {
                match (escaped, c) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, '"') => quoted = false,
                    _ => {}
                }
                return false;
            }
            if c == '"' {
                quoted = true;
            }
            c == ':'
        });
        if let Some((at, _)) = split {
            fields.push((
                part[..at].trim().to_string(),
                part[at + 1..].trim().to_string(),
            ));
        }
    }
    fields
}

/// A string as JSON writes it.
pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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

    /// F5: reads launch.toml first, unless a run is on, then goes on.
    fn debug_read_then(&mut self, without_debugger: bool, fx: &mut Vec<Effect>) {
        if self.debug.running() || self.debug_building().is_some() {
            return self.debug_start(without_debugger, fx);
        }
        self.debug.reading = true;
        self.debug.run_plain = without_debugger;
        fx.push(Effect::ReadLaunch);
    }

    /// launch.toml read: Run and Debug lists it, and a waiting F5 goes on.
    pub(crate) fn launch_configs(
        &mut self,
        configs: Result<Vec<DebugTarget>, String>,
        fx: &mut Vec<Effect>,
    ) {
        match configs {
            Ok(mut list) => {
                let language = self
                    .plugin_language()
                    .map(|l| l.id.clone())
                    .unwrap_or_default();
                for t in &mut list {
                    if t.language.is_empty() && t.debugger.is_empty() {
                        t.language = language.clone();
                    }
                    t.detail = LAUNCH_FILE.into();
                }
                self.debug.launch = list;
                self.debug.launch_error = None;
            }
            Err(e) => self.debug.launch_error = Some(e),
        }
        if std::mem::take(&mut self.debug.reading) {
            if let Some(e) = self.debug.launch_error.clone() {
                self.error(format!("{LAUNCH_FILE} can't be read: {e}"));
                self.add_action("Open It", "debug.configure");
                return self.build_views();
            }
            let plain = self.debug.run_plain;
            self.debug_start(plain, fx);
        }
        self.build_views();
    }

    /// A file opened: launch.toml new for Add a Configuration gets the
    /// example.
    pub(crate) fn launch_opened(&mut self, path: &Path) {
        if !std::mem::take(&mut self.debug.example) || path != Path::new(LAUNCH_FILE) {
            return;
        }
        if self.doc().buffer.path.as_deref() == Some(path) && self.doc().buffer.text().is_empty() {
            self.type_text(EXAMPLE, crate::history::Kind::Other);
        }
    }

    /// F5: continues a paused run, or starts the chosen target, asking
    /// which the first time.
    fn debug_start(&mut self, without_debugger: bool, fx: &mut Vec<Effect>) {
        if self.debug.paused().is_some() {
            return self.send_debug("debug.continue", fx);
        }
        if self.debug.running() || self.debug_building().is_some() {
            return;
        }
        let chosen = self
            .debug
            .chosen
            .clone()
            .filter(|c| self.debug.all_targets().any(|t| t.name == *c));
        let Some(name) = chosen else {
            return self.debug_pick();
        };
        let Some(target) = self.debug.all_targets().find(|t| t.name == name).cloned() else {
            return;
        };
        if without_debugger {
            // Ctrl+F5 runs it as a task would.
            let mut command = target.program.clone();
            for a in &target.args {
                command.push(' ');
                command.push_str(a);
            }
            let command = match target.build.is_empty() {
                true => command,
                false => format!("{} && {command}", target.build),
            };
            self.start_command(&format!("Run {}", target.name), &command, fx);
            return;
        }
        self.launch_target(target, fx);
    }

    /// Asks what to debug.
    fn debug_pick(&mut self) {
        let items: Vec<(String, String)> = self
            .debug
            .all_targets()
            .map(|t| (format!("t:{}", t.name), t.name.clone()))
            .collect();
        if items.is_empty() {
            self.notify(
                "Nothing to debug was found. Open a file whose language plugin finds programs, \
                 such as a Rust main.rs"
                    .into(),
            );
            return;
        }
        self.palette = Some(Palette::new(
            Source::Picker {
                plugin: DEBUG.into(),
                id: "targets".into(),
            },
            Some("Debug what?".into()),
            items,
        ));
        self.focus = Focus::Palette;
    }

    /// The processes to attach to, in a picker. Linux allows it only to a
    /// program's parent when `ptrace_scope` is 1 or more, and merlin isn't
    /// one, so then it says so instead.
    pub(crate) fn pick_process(&mut self, list: Vec<(u32, String, String)>, ptrace_scope: u8) {
        if list.is_empty() {
            return self.notify("No processes of yours to attach to".into());
        }
        self.debug.ptrace_scope = ptrace_scope;
        let items = list
            .into_iter()
            .map(|(pid, name, command)| {
                (
                    format!("p:{pid}:{name}"),
                    format!("{name}  {pid}  {command}"),
                )
            })
            .collect();
        self.palette = Some(Palette::new(
            Source::Picker {
                plugin: DEBUG.into(),
                id: "attach".into(),
            },
            Some("Attach to which process?".into()),
            items,
        ));
        self.focus = Focus::Palette;
    }

    /// A target chosen from the picker or the view.
    pub(crate) fn debug_picked(&mut self, item: &str, fx: &mut Vec<Effect>) {
        if let Some((pid, name)) = item.strip_prefix("p:").and_then(|p| p.split_once(':')) {
            if self.debug.ptrace_scope >= 1 {
                self.error(format!(
                    "Linux won't let merlin attach to {pid}: ptrace_scope is {}, so only a \
                     program's parent may debug it. Start it from merlin instead, or change \
                     /proc/sys/kernel/yama/ptrace_scope.",
                    self.debug.ptrace_scope
                ));
                // The last button is the main one.
                self.add_action("What This Means", "debug.ptrace_help");
                self.add_action("Start It from merlin", "debug.pick");
                return;
            }
            let language = self
                .plugin_language()
                .map(|l| l.id.clone())
                .unwrap_or_default();
            let target = DebugTarget {
                name: format!("{name} ({pid})"),
                language,
                launch: format!("{{\"pid\":{pid}}}"),
                ..Default::default()
            };
            return self.launch_target(target, fx);
        }
        if let Some(name) = item.strip_prefix("t:") {
            self.debug.chosen = Some(name.to_string());
            self.debug_start(false, fx);
        }
    }

    /// Builds a target if it says to, then starts it.
    fn launch_target(&mut self, target: DebugTarget, fx: &mut Vec<Effect>) {
        let debuggers: Vec<Debugger> = self
            .debug
            .debuggers_for(&target)
            .into_iter()
            .cloned()
            .collect();
        let Some(debugger) = debuggers.first().cloned() else {
            let what = match target.language.as_str() {
                "" => target.name.clone(),
                l => self.language_name(l),
            };
            self.error(format!(
                "No plugin adds a debugger for {what}. A language plugin declares one in its \
                 manifest's [[debuggers]]"
            ));
            return;
        };
        if !self.debug.is_installed(&debugger) {
            return self.debugger_missing(&target, &debuggers);
        }
        let root = self.root.display().to_string();
        let program = match Path::new(&target.program) {
            p if p.is_absolute() || target.program.is_empty() => target.program.clone(),
            p => self.root.join(p).display().to_string(),
        };
        let mut base = format!(
            "{{\"name\":{},\"program\":{},\"args\":[{}],\"cwd\":{}}}",
            json_string(&target.name),
            json_string(&program),
            target
                .args
                .iter()
                .map(|a| json_string(a))
                .collect::<Vec<_>>()
                .join(","),
            json_string(&root),
        );
        // Nothing to start yet, to attach or for a test's binary to come:
        // no program, but the arguments all the same.
        if target.program.is_empty() {
            base = format!(
                "{{\"name\":{},\"args\":[{}],\"cwd\":{}}}",
                json_string(&target.name),
                target
                    .args
                    .iter()
                    .map(|a| json_string(a))
                    .collect::<Vec<_>>()
                    .join(","),
                json_string(&root)
            );
        }
        let start = DebugStart {
            name: target.name.clone(),
            debugger: debugger.id.clone(),
            command: debugger.command.clone(),
            program,
            args: target.args.clone(),
            cwd: root,
            env: Vec::new(),
            launch: merge_json(&[&base, &debugger.launch, &target.launch]),
            stop_on: self.debug.stop_on.iter().cloned().collect(),
            function_stops: debugger
                .stops
                .iter()
                .map(|f| FunctionStop {
                    on: !self.debug.stop_off.contains(&f.id)
                        && (f.on || self.debug.stop_on.contains(&f.id)),
                    ..f.clone()
                })
                .collect(),
        };
        self.debug.console.clear();
        self.debug.states.clear();
        self.debug.expanded.clear();
        self.debug.last = Some((target.name.clone(), start.clone()));
        let mut session = Session {
            driver: debugger.driver.clone(),
            start,
            state: DebugState::Starting,
            building: None,
            binary_for: None,
            threads: Vec::new(),
            stacks: HashMap::new(),
            frame: None,
            variables: HashMap::new(),
            inline: HashMap::new(),
            watched: Vec::new(),
        };
        if !target.build.is_empty() {
            let term = self.start_command(&format!("Build {}", target.name), &target.build, fx);
            session.building = Some(term);
            self.debug.session = Some(session);
            self.build_views();
            return;
        }
        self.debug.session = Some(session);
        self.start_driver(fx);
    }

    /// Asks the driver to start the run.
    fn start_driver(&mut self, fx: &mut Vec<Effect>) {
        let Some(session) = &mut self.debug.session else {
            return;
        };
        session.building = None;
        let (driver, start) = (session.driver.clone(), session.start.clone());
        if !self.plugins.iter().any(|p| p.name == driver) {
            self.debug.session = None;
            self.error(format!(
                "Debugging needs the {driver} plugin, which isn't loaded or is turned off"
            ));
            return;
        }
        fx.push(Effect::Debug {
            plugin: driver,
            command: "debug.start".into(),
            start: Some(Box::new(start)),
            breakpoints: self.debug.breakpoints.clone(),
        });
        self.show_view(DEBUG, fx);
        // The console comes to the front, as VS Code's does, but the keys
        // stay in the code.
        let bottom = self.bar_mut(Side::Bottom);
        bottom.visible = true;
        bottom.active = Some(CONSOLE.into());
        if self.focus == Focus::SideBar {
            self.focus = Focus::Editor;
        }
        self.build_views();
    }

    /// No debugger for the target is installed: says how to get one, and
    /// offers another that is.
    fn debugger_missing(&mut self, target: &DebugTarget, debuggers: &[Debugger]) {
        let names: Vec<&str> = debuggers.iter().map(|d| d.name.as_str()).collect();
        let what = self.language_name(&target.language);
        let list = match names.as_slice() {
            [one] => one.to_string(),
            [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
            [] => String::new(),
        };
        let install = debuggers
            .iter()
            .find(|d| !d.install.is_empty())
            .map(|d| d.install.clone());
        let mut text = format!("Debugging {what} needs {list}, and merlin found none of them.");
        if let Some(install) = &install {
            text.push_str(&format!(" To install one: {install}"));
        }
        self.notify_warning(text);
        if let Some(install) = install {
            self.add_action("Copy Install Command", &format!("debug.copy:{install}"));
        }
        self.add_action("Choose the Path…", "settings.open");
    }

    pub(crate) fn has_debugger_for(&self, language: &str) -> bool {
        self.debug
            .debuggers
            .iter()
            .any(|d| d.languages.iter().any(|l| l == language))
    }

    /// Debugs a test: its binary comes from what its build prints.
    pub(crate) fn debug_test(&mut self, target: DebugTarget, file: String, fx: &mut Vec<Effect>) {
        if self.debug.running() || self.debug_building().is_some() {
            return self.notify("A run is on already: Shift+F5 stops it".into());
        }
        self.launch_target(target, fx);
        if let Some(session) = &mut self.debug.session {
            session.binary_for = Some(file);
        }
    }

    /// All a build printed, when it ends: a test's binary is found in it,
    /// from cargo's `Executable unittests src/lib.rs (target/debug/…)`.
    pub(crate) fn debug_build_output(&mut self, term: u64, text: &str) {
        if self.debug_building() != Some(term) {
            return;
        }
        let root = self.root.clone();
        let Some(session) = &mut self.debug.session else {
            return;
        };
        let Some(file) = session.binary_for.clone() else {
            return;
        };
        let binaries = executables(text);
        // The file's own target, or the crate's library, then its binary.
        let pick = binaries
            .iter()
            .find(|(src, _)| *src == file)
            .or_else(|| match file.starts_with("tests/") {
                true => None,
                false => ["src/lib.rs", "src/main.rs"]
                    .iter()
                    .find_map(|want| binaries.iter().find(|(src, _)| src == want)),
            })
            .or(binaries.first());
        if let Some((_, bin)) = pick {
            let program = root.join(bin).display().to_string();
            let launch = format!("{{\"program\":{}}}", json_string(&program));
            session.start.launch = merge_json(&[&session.start.launch, &launch]);
            session.start.program = program;
        }
    }

    /// A build run first ended: the run starts if it worked.
    pub(crate) fn debug_build_exited(
        &mut self,
        term: u64,
        code: Option<i32>,
        fx: &mut Vec<Effect>,
    ) {
        if self.debug_building() != Some(term) {
            return;
        }
        if code == Some(0) {
            let no_binary = self
                .debug
                .session
                .as_ref()
                .is_some_and(|s| s.binary_for.is_some() && s.start.program.is_empty());
            if no_binary {
                self.debug.session = None;
                self.build_views();
                return self
                    .error("The test built, but cargo didn't say where its binary is".into());
            }
            return self.start_driver(fx);
        }
        let session = self.debug.session.take();
        let name = session
            .as_ref()
            .map(|s| s.start.name.clone())
            .unwrap_or_default();
        self.build_views();
        self.error(format!("{name} didn't build, so debugging didn't start"));
        self.add_action("Show Problems", "view.problems");
        // A program built before can still be debugged, as it was.
        if let Some(session) = session.filter(|s| !s.start.program.is_empty()) {
            self.debug.failed = Some(session);
            self.add_action("Debug the Last Build", "debug.last_build");
        }
    }

    pub(crate) fn toggle_breakpoint(&mut self, path: &Path, line: u32, fx: &mut Vec<Effect>) {
        match self.debug.breakpoint_at(path, line) {
            Some(i) => {
                let b = self.debug.breakpoints.remove(i);
                self.debug.states.remove(&b.id);
            }
            None => {
                self.debug.next_id += 1;
                let id = self.debug.next_id;
                self.debug.breakpoints.push(Breakpoint {
                    id,
                    path: path.to_path_buf(),
                    line,
                    enabled: true,
                    ..Default::default()
                });
            }
        }
        self.breakpoints_changed(fx);
    }

    fn menu_breakpoint_index(&self) -> Option<usize> {
        let path = self.doc().buffer.path.as_ref()?;
        self.debug.breakpoint_at(path, self.debug.menu_line?)
    }

    fn menu_breakpoint(&self) -> Option<&Breakpoint> {
        self.menu_breakpoint_index()
            .map(|i| &self.debug.breakpoints[i])
    }

    /// The right-click menu on the breakpoint room of line `line`: add
    /// one, or change the one there.
    pub(crate) fn break_menu(&mut self, line: usize) {
        self.debug.menu_line = Some(line as u32);
        let items: Vec<&'static str> = match self.menu_breakpoint() {
            Some(b) => vec![
                "debug.bp_edit",
                if b.enabled {
                    "debug.bp_disable"
                } else {
                    "debug.bp_enable"
                },
                "debug.bp_remove",
            ],
            None => vec!["debug.bp_add", "debug.bp_condition", "debug.bp_log"],
        };
        self.context = Some(super::context::ContextMenu {
            items,
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    /// Opens the box between the lines on the menu's line, with what the
    /// breakpoint there already has.
    fn open_break_box(&mut self, field: BreakField) {
        let Some(line) = self.debug.menu_line else {
            return;
        };
        let text = self
            .menu_breakpoint()
            .map_or(String::new(), |b| match field {
                BreakField::Condition => b.condition.clone(),
                BreakField::Hit => b.hit.clone(),
                BreakField::Log => b.log.clone(),
            });
        self.debug.break_box = Some(BreakBox {
            doc: self.doc().id,
            line,
            field,
            caret: text.chars().count(),
            text,
        });
        self.focus = Focus::Editor;
    }

    /// The box, to draw, and the position it's under.
    pub(crate) fn break_box_view(&self) -> Option<(BreakBoxView, usize)> {
        let b = self
            .debug
            .break_box
            .as_ref()
            .filter(|b| b.doc == self.doc().id)?;
        let line = (b.line as usize).min(self.doc().last_line());
        Some((
            BreakBoxView {
                field: b.field.label().into(),
                text: b.text.clone(),
                caret: b.caret,
                hint: format!(
                    "{} · Tab for another kind · Enter sets it · Esc leaves it",
                    b.field.hint()
                ),
            },
            self.doc().line_start(line),
        ))
    }

    /// Keys while the breakpoint box is open: they edit its text. Tab
    /// goes to the next kind, Enter sets it, Esc leaves it as it was.
    pub(crate) fn break_box_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let id = self.doc().id;
        let Some(b) = self.debug.break_box.as_mut().filter(|b| b.doc == id) else {
            return false;
        };
        let byte = |text: &str, i: usize| text.char_indices().nth(i).map_or(text.len(), |c| c.0);
        let len = b.text.chars().count();
        match key.code {
            KeyCode::Esc => self.debug.break_box = None,
            KeyCode::Tab | KeyCode::BackTab => {
                b.field = match (b.field, key.code == KeyCode::BackTab) {
                    (BreakField::Condition, false) | (BreakField::Log, true) => BreakField::Hit,
                    (BreakField::Hit, false) | (BreakField::Condition, true) => BreakField::Log,
                    (BreakField::Log, false) | (BreakField::Hit, true) => BreakField::Condition,
                };
                b.text.clear();
                b.caret = 0;
            }
            KeyCode::Enter => {
                let b = self.debug.break_box.take().expect("open");
                self.set_break_field(b.line, b.field, b.text.trim(), fx);
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                let at = byte(&b.text, b.caret);
                b.text.insert(at, c);
                b.caret += 1;
            }
            KeyCode::Backspace if b.caret > 0 => {
                let at = byte(&b.text, b.caret - 1);
                b.text.remove(at);
                b.caret -= 1;
            }
            KeyCode::Delete if b.caret < len => {
                let at = byte(&b.text, b.caret);
                b.text.remove(at);
            }
            KeyCode::Left => b.caret = b.caret.saturating_sub(1),
            KeyCode::Right => b.caret = (b.caret + 1).min(len),
            KeyCode::Home => b.caret = 0,
            KeyCode::End => b.caret = len,
            _ => {}
        }
        true
    }

    /// Sets a breakpoint's condition, hit count or message, making the
    /// breakpoint if the line has none.
    fn set_break_field(&mut self, line: u32, field: BreakField, text: &str, fx: &mut Vec<Effect>) {
        let Some(path) = self.doc().buffer.path.clone() else {
            return;
        };
        let i = match self.debug.breakpoint_at(&path, line) {
            Some(i) => i,
            None => {
                self.debug.next_id += 1;
                self.debug.breakpoints.push(Breakpoint {
                    id: self.debug.next_id,
                    path,
                    line,
                    enabled: true,
                    ..Default::default()
                });
                self.debug.breakpoints.len() - 1
            }
        };
        let b = &mut self.debug.breakpoints[i];
        match field {
            BreakField::Condition => b.condition = text.into(),
            BreakField::Hit => b.hit = text.into(),
            BreakField::Log => b.log = text.into(),
        }
        self.breakpoints_changed(fx);
    }

    /// Breakpoints and the chosen target, kept with the workspace.
    pub(crate) fn restore_breakpoints(
        &mut self,
        mut list: Vec<Breakpoint>,
        target: Option<String>,
    ) {
        for b in &mut list {
            self.debug.next_id += 1;
            b.id = self.debug.next_id;
        }
        self.debug.breakpoints = list;
        if target.is_some() {
            self.debug.chosen = target;
        }
        self.build_views();
    }

    fn breakpoints_changed(&mut self, fx: &mut Vec<Effect>) {
        if self.debug.running() {
            self.send_debug("debug.breakpoints", fx);
        }
        self.build_views();
    }

    pub(crate) fn debug_paused(&self) -> bool {
        self.debug.paused().is_some()
    }

    /// While paused, whether the word from `start` to `end` is a variable
    /// of the frame shown; it becomes the hover's, and what's in it is
    /// asked for.
    pub(crate) fn hover_variable(
        &mut self,
        start: usize,
        end: usize,
        fx: &mut Vec<Effect>,
    ) -> bool {
        self.debug.hover = None;
        if self.debug.paused().is_none() {
            return false;
        }
        let name = self.doc().slice(start, end);
        let Some(v) = self.frame_variable(&name).cloned() else {
            return false;
        };
        let loaded = self
            .debug
            .session
            .as_ref()
            .is_some_and(|s| s.variables.contains_key(&v.children));
        if v.children != 0 && !loaded {
            self.send_debug(&format!("debug.expand:{}", v.children), fx);
        }
        self.debug.hover = Some(name);
        true
    }

    pub(crate) fn clear_hover_variable(&mut self) {
        self.debug.hover = None;
    }

    /// A variable of the frame shown by name, from its scopes as loaded.
    fn frame_variable(&self, name: &str) -> Option<&DebugVariable> {
        let session = self.debug.session.as_ref()?;
        let scopes = session.variables.get(&0)?;
        scopes
            .iter()
            .filter_map(|s| session.variables.get(&s.children))
            .flatten()
            .find(|v| v.name == name)
    }

    /// The hover's value, as a code block: the variable with its type and
    /// value, then what's in it, a line each.
    pub(crate) fn hover_value_lines(&self) -> Vec<String> {
        let Some(name) = &self.debug.hover else {
            return Vec::new();
        };
        let Some(v) = self.frame_variable(name) else {
            return Vec::new();
        };
        let session = self.debug.session.as_ref().expect("a variable is in a run");
        let typed = match v.kind.is_empty() {
            true => v.name.clone(),
            false => format!("{}: {}", v.name, short_type(&v.kind)),
        };
        let mut lines = vec!["```".to_string()];
        let children = session
            .variables
            .get(&v.children)
            .filter(|_| v.children != 0);
        match (children, v.value.is_empty()) {
            (Some(_), _) | (None, true) => lines.push(typed),
            (None, false) => lines.push(format!("{typed} = {}", short_value(&v.value))),
        }
        if let Some(children) = children {
            for c in children.iter().take(12) {
                // Items of an array or a vector, as Rust indexes them.
                let name = match c.name.chars().all(|d| d.is_ascii_digit()) {
                    true => format!("[{}]", c.name),
                    false => c.name.clone(),
                };
                lines.push(format!("    {name} = {}", short_value(&c.value)));
            }
            if children.len() > 12 {
                lines.push(format!("    … {} more", children.len() - 12));
            }
        } else if v.children != 0 {
            lines.push("    …".into());
        }
        lines.push("```".into());
        lines
    }

    /// Whether the shown file can have breakpoints: a debugger is declared
    /// for its language, or it has some already.
    pub(crate) fn can_break_here(&self) -> bool {
        let Some(path) = self.doc().buffer.path.as_ref() else {
            return false;
        };
        let language = self.plugin_language().map(|l| l.id.clone());
        self.debug.breakpoints.iter().any(|b| b.path == *path)
            || language.is_some_and(|l| {
                self.debug
                    .debuggers
                    .iter()
                    .any(|d| d.languages.contains(&l))
            })
    }

    /// The gutter's mark for a line of the shown file.
    pub(crate) fn break_mark(&self, line: usize) -> Option<BreakMark> {
        let path = self.doc().buffer.path.as_ref()?;
        let b = self
            .debug
            .breakpoints
            .iter()
            .find(|b| b.path == *path && b.line as usize == line)?;
        Some(if !b.enabled {
            BreakMark::Disabled
        } else if self.debug.states.get(&b.id).is_some_and(|s| !s.set) {
            BreakMark::Unset
        } else if !b.log.is_empty() {
            BreakMark::Log
        } else if !b.condition.is_empty() || !b.hit.is_empty() {
            BreakMark::Conditional
        } else {
            BreakMark::Set
        })
    }

    /// Whether the shown file's line is where the run is paused, and if
    /// so whether on an exception.
    pub(crate) fn paused_line(&self, line: usize) -> Option<bool> {
        let frame = self.debug.current_frame()?;
        let path = self.doc().buffer.path.as_ref()?;
        let same = frame.path.as_ref().is_some_and(|p| self.same_file(p, path));
        (same && frame.line as usize == line).then(|| self.paused_on_exception())
    }

    /// What to write after a line while paused: the plugin's values, or
    /// the frame's variables that the line names.
    pub(crate) fn inline_value(&self, line: usize, text: &str) -> Option<String> {
        let session = self.debug.session.as_ref()?;
        self.debug.paused()?;
        let path = self.doc().buffer.path.as_ref()?;
        if let Some((_, values)) = session.inline.iter().find(|(p, _)| self.same_file(p, path)) {
            return values
                .iter()
                .find(|(l, _)| *l as usize == line)
                .map(|(_, t)| t.clone());
        }
        let frame = self.debug.current_frame()?;
        if !frame.path.as_ref().is_some_and(|p| self.same_file(p, path))
            || line > frame.line as usize
            || line < self.function_start(frame.line as usize)
        {
            return None;
        }
        let scopes = session.variables.get(&0)?;
        let locals: Vec<&DebugVariable> = scopes
            .iter()
            .filter_map(|s| session.variables.get(&s.children))
            .flatten()
            .collect();
        let words: HashSet<&str> = text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
            .collect();
        let shown: Vec<String> = locals
            .iter()
            .filter(|v| !v.value.is_empty() && words.contains(v.name.as_str()))
            .take(4)
            .map(|v| format!("{} = {}", v.name, short_value(&v.value)))
            .collect();
        (!shown.is_empty()).then(|| shown.join(", "))
    }

    /// The first line of the function a line is in, as far as 40 lines
    /// up: where its `fn`, `def` or `function` is. Values show only from
    /// there, as the frame's variables are only that function's.
    fn function_start(&self, line: usize) -> usize {
        let doc = self.doc();
        let top = line.saturating_sub(40);
        (top..=line)
            .rev()
            .find(|&l| {
                let text: String = doc.line_chars(l).into_iter().collect();
                let mut words = text.split_whitespace().skip_while(|w| {
                    w.starts_with("pub")
                        || matches!(*w, "async" | "const" | "unsafe" | "static" | "export")
                });
                matches!(words.next(), Some("fn" | "def" | "function" | "func"))
            })
            .unwrap_or(top)
    }

    /// While paused on an exception in the shown file: what it says, and
    /// the start of its line, for the card under it.
    pub(crate) fn exception_card(&self) -> Option<(String, usize)> {
        let session = self.debug.session.as_ref()?;
        let DebugState::Paused {
            reason,
            description,
            ..
        } = &session.state
        else {
            return None;
        };
        if reason != "exception" {
            return None;
        }
        let frame = self.debug.current_frame()?;
        let path = self.doc().buffer.path.as_ref()?;
        if !frame.path.as_ref().is_some_and(|p| self.same_file(p, path)) {
            return None;
        }
        let line = (frame.line as usize).min(self.doc().last_line());
        let message = match description.is_empty() {
            true => "An exception".to_string(),
            false => description.clone(),
        };
        Some((message, self.doc().line_start(line)))
    }

    fn paused_on_exception(&self) -> bool {
        matches!(
            self.debug.session.as_ref().map(|s| &s.state),
            Some(DebugState::Paused { reason, .. }) if reason == "exception"
        )
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

    /// The toolbar, while a run is on.
    pub(crate) fn debug_bar(&self) -> Option<DebugBarView> {
        let session = self.debug.session.as_ref()?;
        Some(DebugBarView {
            paused: matches!(session.state, DebugState::Paused { .. }),
            building: session.building.is_some(),
        })
    }

    /// The status bar's words while a run is on.
    pub(crate) fn debug_status(&self) -> Option<String> {
        let session = self.debug.session.as_ref()?;
        let name = &session.start.name;
        Some(match &session.state {
            _ if session.building.is_some() => format!("⟳ Building {name}"),
            DebugState::Starting => format!("● Starting {name}"),
            DebugState::Running => format!("● Debugging {name}"),
            DebugState::Paused { .. } => match self.debug.current_frame() {
                Some(f) => {
                    let file = f
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| f.name.clone());
                    format!("● Debugging {name} · Paused at {file}:{}", f.line + 1)
                }
                None => format!("● Debugging {name} · Paused"),
            },
            DebugState::Ended(_) => return None,
        })
    }

    /// The Run and Debug view.
    pub(crate) fn debug_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        let heading = |label: &str| {
            Node::Entry(Entry {
                id: format!("dbg:h:{label}"),
                label: label.into(),
                heading: true,
                ..Default::default()
            })
        };
        match &self.debug.session {
            None => {
                let chosen = self
                    .debug
                    .chosen
                    .clone()
                    .filter(|c| self.debug.all_targets().any(|t| t.name == *c));
                let label = match &chosen {
                    Some(name) => format!("▶ Start Debugging {name}"),
                    None => "▶ Start Debugging".into(),
                };
                nodes.push(Node::Wide {
                    button: Button {
                        id: "dbg:start".into(),
                        label,
                    },
                    ready: true,
                });
                let target_entry = |t: &DebugTarget| {
                    Node::Entry(Entry {
                        id: format!("dbg:t:{}", t.name),
                        label: t.name.clone(),
                        icon: Some((
                            '▶',
                            if chosen.as_ref() == Some(&t.name) {
                                Hue::Green
                            } else {
                                Hue::Grey
                            },
                        )),
                        chip: t.plugin.clone(),
                        badge: match t.plugin {
                            Some(_) => t.detail.clone(),
                            None if !t.build.is_empty() => format!("{} first", t.build),
                            None => String::new(),
                        },
                        ..Default::default()
                    })
                };
                if !self.debug.launch.is_empty() || self.debug.launch_error.is_some() {
                    nodes.push(Node::Entry(Entry {
                        id: "dbg:h:launch".into(),
                        label: "From launch.toml".into(),
                        heading: true,
                        action: Some(('✎', "dbg:configure".into())),
                        ..Default::default()
                    }));
                    if let Some(e) = &self.debug.launch_error {
                        nodes.push(Node::Styled(format!("Can't be read: {e}"), Tone::Error));
                    }
                    for t in &self.debug.launch {
                        nodes.push(target_entry(t));
                    }
                }
                nodes.push(heading("Found in the project"));
                let targets: Vec<&DebugTarget> = self
                    .debug
                    .targets
                    .iter()
                    .flat_map(|(_, t)| t.iter())
                    .collect();
                if targets.is_empty() {
                    nodes.push(Node::Styled(
                        "Nothing yet. Language plugins list what can be debugged when they see \
                         it, as rust-analyzer does for a crate's binaries."
                            .into(),
                        Tone::Muted,
                    ));
                }
                for t in targets {
                    nodes.push(target_entry(t));
                }
                nodes.push(Node::Tools(vec![
                    Button {
                        id: "dbg:attach".into(),
                        label: "Attach to Process…".into(),
                    },
                    Button {
                        id: "dbg:configure".into(),
                        label: "Add a Configuration…".into(),
                    },
                ]));
            }
            Some(session) => {
                let state = match &session.state {
                    _ if session.building.is_some() => ("Building".to_string(), Tone::Warning),
                    DebugState::Starting => ("Starting".into(), Tone::Muted),
                    DebugState::Running => ("Running".into(), Tone::Normal),
                    DebugState::Paused {
                        reason,
                        description,
                        ..
                    } => {
                        let why = match reason.as_str() {
                            "breakpoint" | "function breakpoint" => "Paused on breakpoint".into(),
                            "exception" => "Paused on an exception".into(),
                            "step" => "Paused after a step".into(),
                            "entry" => "Paused at the start".into(),
                            "pause" => "Paused".into(),
                            other => format!("Paused: {other}"),
                        };
                        // The card under the line says what went wrong.
                        let _ = description;
                        (
                            why,
                            if reason == "exception" {
                                Tone::Error
                            } else {
                                Tone::Warning
                            },
                        )
                    }
                    DebugState::Ended(_) => ("Ended".into(), Tone::Muted),
                };
                nodes.push(Node::Entry(Entry {
                    id: "dbg:session".into(),
                    label: session.start.name.clone(),
                    icon: Some((
                        '●',
                        match state.1 {
                            Tone::Error => Hue::Red,
                            Tone::Warning => Hue::Yellow,
                            _ => Hue::Green,
                        },
                    )),
                    detail: session.start.debugger.clone(),
                    badge: state.0,
                    badge_tone: state.1,
                    ..Default::default()
                }));
                nodes.push(heading("Variables"));
                match session.variables.get(&0) {
                    Some(scopes) => self.variable_nodes(session, scopes, 0, 0, &mut nodes),
                    None => nodes.push(Node::Styled(
                        match self.debug.paused() {
                            Some(_) => "Reading…".into(),
                            None => "Shown while paused".into(),
                        },
                        Tone::Muted,
                    )),
                }
                nodes.push(Node::Entry(Entry {
                    id: "dbg:h:watch".into(),
                    label: "Watch".into(),
                    heading: true,
                    ..Default::default()
                }));
                for (i, expr) in self.debug.watches.iter().enumerate() {
                    let value = session.watched.get(i).cloned().flatten();
                    let open = value.as_ref().is_some_and(|v| {
                        v.children != 0 && self.debug.expanded.contains(&v.children)
                    });
                    nodes.push(Node::Entry(Entry {
                        id: format!("dbg:w:{i}:{}", value.as_ref().map_or(0, |v| v.children)),
                        label: match &value {
                            Some(v) => format!("{expr} = {}", short_value(&v.value)),
                            None => expr.clone(),
                        },
                        twisty: value
                            .as_ref()
                            .filter(|v| v.children != 0)
                            .map(|_| if open { '▾' } else { '▸' }),
                        badge: value
                            .as_ref()
                            .map_or(String::new(), |v| short_type(&v.kind)),
                        tone: if value.is_some() {
                            Tone::Normal
                        } else {
                            Tone::Muted
                        },
                        code: true,
                        action: Some(('✕', format!("dbg:rw:{i}"))),
                        ..Default::default()
                    }));
                    if let Some(v) = value.filter(|_| open)
                        && let Some(children) = session.variables.get(&v.children)
                    {
                        self.variable_nodes(session, children, v.children, 1, &mut nodes);
                    }
                }
                nodes.push(Node::Input {
                    id: "watch".into(),
                    placeholder: "An expression to watch, then Enter".into(),
                    secret: false,
                });
                nodes.push(heading("Call Stack"));
                self.stack_nodes(session, &mut nodes);
            }
        }
        nodes.push(heading("Breakpoints"));
        if self.debug.breakpoints.is_empty() {
            nodes.push(Node::Styled(
                "Click left of a line's number, or F9, to stop there.".into(),
                Tone::Muted,
            ));
        }
        for b in &self.debug.breakpoints {
            let state = self.debug.states.get(&b.id);
            let unset = state.is_some_and(|s| !s.set);
            let file = b
                .path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let detail = if unset {
                state
                    .map(|s| s.message.clone())
                    .filter(|m| !m.is_empty())
                    .unwrap_or("not set".into())
            } else if !b.log.is_empty() {
                format!("log \"{}\"", b.log)
            } else if !b.condition.is_empty() {
                b.condition.clone()
            } else {
                String::new()
            };
            nodes.push(Node::Entry(Entry {
                id: format!("dbg:b:{}", b.id),
                label: file,
                check: Some(if b.enabled {
                    crate::panel::Check::On
                } else {
                    crate::panel::Check::Off
                }),
                icon: Some((
                    if b.log.is_empty() { '●' } else { '◆' },
                    if unset || !b.enabled {
                        Hue::Grey
                    } else {
                        Hue::Red
                    },
                )),
                detail,
                badge: (b.line + 1).to_string(),
                tone: if unset || !b.enabled {
                    Tone::Muted
                } else {
                    Tone::Normal
                },
                action: Some(('✕', format!("dbg:rb:{}", b.id))),
                ..Default::default()
            }));
        }
        if !self.debug.filters.is_empty() {
            nodes.push(heading("Stop on"));
            for f in &self.debug.filters {
                nodes.push(Node::Entry(Entry {
                    id: format!("dbg:f:{}", f.id),
                    label: f.label.clone(),
                    check: Some(if f.on {
                        crate::panel::Check::On
                    } else {
                        crate::panel::Check::Off
                    }),
                    ..Default::default()
                }));
            }
        }
        nodes
    }

    fn variable_nodes(
        &self,
        session: &Session,
        list: &[DebugVariable],
        parent: u32,
        depth: usize,
        nodes: &mut Vec<Node>,
    ) {
        for v in list {
            let open = v.children != 0 && self.debug.expanded.contains(&v.children);
            let scope = depth == 0;
            if self.debug.setting.as_ref() == Some(&(parent, v.name.clone())) {
                nodes.push(Node::Input {
                    id: "set".into(),
                    placeholder: format!("A new value for {}, then Enter", v.name),
                    secret: false,
                });
            }
            nodes.push(Node::Entry(Entry {
                id: format!("dbg:v:{}:{}:{parent}:{}", v.children, nodes.len(), v.name),
                label: if scope || v.value.is_empty() {
                    v.name.clone()
                } else {
                    format!("{} = {}", v.name, short_value(&v.value))
                },
                depth,
                twisty: (v.children != 0).then_some(if open { '▾' } else { '▸' }),
                badge: short_type(&v.kind),
                code: !scope,
                ..Default::default()
            }));
            if open {
                match session.variables.get(&v.children) {
                    Some(children) => {
                        self.variable_nodes(session, children, v.children, depth + 1, nodes)
                    }
                    None => nodes.push(Node::Entry(Entry {
                        id: format!("dbg:wait:{}", v.children),
                        label: "…".into(),
                        depth: depth + 1,
                        tone: Tone::Muted,
                        ..Default::default()
                    })),
                }
            }
        }
    }

    fn stack_nodes(&self, session: &Session, nodes: &mut Vec<Node>) {
        let several = session.threads.len() > 1;
        let shown = self.debug.current_frame().map(|f| f.id);
        let threads: Vec<DebugThread> = match session.threads.is_empty() {
            true => session
                .stacks
                .keys()
                .map(|&id| DebugThread {
                    id,
                    name: format!("Thread {id}"),
                    paused: true,
                })
                .collect(),
            false => session.threads.clone(),
        };
        if threads.is_empty() {
            nodes.push(Node::Styled("Shown while paused".into(), Tone::Muted));
        }
        for t in &threads {
            let depth = several as usize;
            if several {
                nodes.push(Node::Entry(Entry {
                    id: format!("dbg:th:{}", t.id),
                    label: t.name.clone(),
                    icon: Some(('●', if t.paused { Hue::Yellow } else { Hue::Green })),
                    badge: if t.paused {
                        "Paused".into()
                    } else {
                        "Running".into()
                    },
                    links: if t.paused {
                        Vec::new()
                    } else {
                        vec![("Pause".into(), format!("dbg:pt:{}", t.id))]
                    },
                    ..Default::default()
                }));
            }
            let Some(frames) = session.stacks.get(&t.id) else {
                continue;
            };
            let unfolded = self.debug.unfolded.contains(&t.id);
            let mut i = 0;
            while i < frames.len() {
                let f = &frames[i];
                // A run of library frames folds into one row.
                if f.library && !unfolded {
                    let run = frames[i..].iter().take_while(|f| f.library).count();
                    if run > 1 {
                        nodes.push(Node::Entry(Entry {
                            id: format!("dbg:lib:{}", t.id),
                            label: format!("{run} frames in libraries"),
                            depth,
                            tone: Tone::Muted,
                            ..Default::default()
                        }));
                        i += run;
                        continue;
                    }
                }
                let at = f
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| format!("{}:{}", n.to_string_lossy(), f.line + 1))
                    .unwrap_or_default();
                nodes.push(Node::Entry(Entry {
                    id: format!("dbg:fr:{}:{}", t.id, f.id),
                    label: f.name.clone(),
                    depth,
                    badge: at,
                    tone: if f.library { Tone::Muted } else { Tone::Normal },
                    icon: (shown == Some(f.id)).then_some(('▸', Hue::Yellow)),
                    ..Default::default()
                }));
                i += 1;
            }
        }
    }

    /// The Debug Console: what the program and the debugger printed, and
    /// a box to evaluate in while paused.
    pub(crate) fn console_nodes(&self) -> Vec<Node> {
        let mut nodes = Vec::new();
        if self.debug.console.is_empty() {
            nodes.push(Node::Styled(
                "What the program prints shows here while it's debugged.".into(),
                Tone::Muted,
            ));
        }
        for (kind, line) in &self.debug.console {
            let line = line.trim_end_matches('\u{0}').to_string();
            let tone = match kind {
                OutputKind::Stderr => Tone::Error,
                OutputKind::Important => Tone::Warning,
                OutputKind::Console => Tone::Muted,
                OutputKind::Stdout => Tone::Normal,
            };
            nodes.push(Node::Styled(line, tone));
        }
        if !self.debug.running() && self.debug.last.is_some() {
            nodes.push(Node::Tools(vec![Button {
                id: "dbg:again".into(),
                label: "↻ Run Again".into(),
            }]));
        }
        if self.debug.running() {
            nodes.push(Node::Footer);
            nodes.push(Node::Input {
                id: "console".into(),
                placeholder: "Evaluate while paused".into(),
                secret: false,
            });
        }
        nodes
    }

    /// A click in Run and Debug or the Debug Console.
    pub(crate) fn debug_click(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let rest = id.strip_prefix("dbg:").unwrap_or(id);
        if rest == "start" {
            return self.debug_read_then(false, fx);
        }
        if rest == "attach" {
            return self.debug_command("debug.attach", fx);
        }
        if rest == "configure" {
            return self.debug_command("debug.configure", fx);
        }
        if rest == "again" {
            return self.debug_command("debug.restart", fx);
        }
        if let Some(name) = rest.strip_prefix("t:") {
            self.debug.chosen = Some(name.to_string());
            return self.debug_read_then(false, fx);
        }
        if let Some(id) = rest.strip_prefix("rb:").and_then(|i| i.parse::<u32>().ok()) {
            self.debug.breakpoints.retain(|b| b.id != id);
            self.debug.states.remove(&id);
            return self.breakpoints_changed(fx);
        }
        if let Some(id) = rest.strip_prefix("b:").and_then(|i| i.parse::<u32>().ok()) {
            if let Some(b) = self.debug.breakpoints.iter().find(|b| b.id == id).cloned() {
                let path = b
                    .path
                    .strip_prefix(&self.root)
                    .map(Path::to_path_buf)
                    .unwrap_or(b.path);
                self.open_at(path, b.line, 0, fx);
            }
            return;
        }
        if let Some(r) = rest
            .strip_prefix("v:")
            .and_then(|v| v.split(':').next()?.parse::<u32>().ok())
        {
            if r == 0 {
                return;
            }
            if !self.debug.expanded.remove(&r) {
                self.debug.expanded.insert(r);
                let asked = self
                    .debug
                    .session
                    .as_ref()
                    .is_some_and(|s| s.variables.contains_key(&r));
                if !asked {
                    self.send_debug(&format!("debug.expand:{r}"), fx);
                }
            }
            return self.build_views();
        }
        if let Some(i) = rest
            .strip_prefix("rw:")
            .and_then(|i| i.parse::<usize>().ok())
        {
            if i < self.debug.watches.len() {
                self.debug.watches.remove(i);
                if let Some(session) = &mut self.debug.session {
                    session.watched.clear();
                }
                self.ask_watches(fx);
            }
            return self.build_views();
        }
        if let Some(r) = rest
            .strip_prefix("w:")
            .and_then(|w| w.split(':').nth(1)?.parse::<u32>().ok())
        {
            if r != 0 && !self.debug.expanded.remove(&r) {
                self.debug.expanded.insert(r);
                self.send_debug(&format!("debug.expand:{r}"), fx);
            }
            return self.build_views();
        }
        if let Some(t) = rest
            .strip_prefix("lib:")
            .and_then(|t| t.parse::<u32>().ok())
        {
            self.debug.unfolded.insert(t);
            return self.build_views();
        }
        if let Some(t) = rest.strip_prefix("pt:") {
            return self.send_debug(&format!("debug.pause-thread:{t}"), fx);
        }
        if let Some((t, f)) = rest
            .strip_prefix("fr:")
            .and_then(|r| r.split_once(':'))
            .and_then(|(t, f)| Some((t.parse::<u32>().ok()?, f.parse::<u32>().ok()?)))
        {
            if let Some(session) = &mut self.debug.session {
                session.frame = Some((t, f));
                session.variables.clear();
                session.inline.clear();
            }
            self.send_debug(&format!("debug.frame:{f}"), fx);
            self.reveal_frame(fx);
            self.build_views();
        }
    }

    /// A box ticked in Run and Debug: a breakpoint or a stop filter.
    pub(crate) fn debug_check(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let rest = id.strip_prefix("dbg:").unwrap_or(id);
        if let Some(id) = rest.strip_prefix("b:").and_then(|i| i.parse::<u32>().ok()) {
            if let Some(b) = self.debug.breakpoints.iter_mut().find(|b| b.id == id) {
                b.enabled = !b.enabled;
            }
            return self.breakpoints_changed(fx);
        }
        if let Some(f) = rest.strip_prefix("f:") {
            let on = !self.debug.filters.iter().any(|x| x.id == f && x.on);
            if on {
                self.debug.stop_on.insert(f.to_string());
                self.debug.stop_off.remove(f);
            } else {
                self.debug.stop_on.remove(f);
                self.debug.stop_off.insert(f.to_string());
            }
            if let Some(filter) = self.debug.filters.iter_mut().find(|x| x.id == f) {
                filter.on = on;
            }
            self.send_debug(&format!("debug.stop-on:{f}"), fx);
            self.build_views();
        }
    }

    /// Asks the driver for each watch's value at the frame shown.
    fn ask_watches(&mut self, fx: &mut Vec<Effect>) {
        let Some(session) = &self.debug.session else {
            return;
        };
        if self.debug.paused().is_none() {
            return;
        }
        let plugin = session.driver.clone();
        for (i, expr) in self.debug.watches.iter().enumerate() {
            fx.push(Effect::PluginSubmit {
                plugin: plugin.clone(),
                input: format!("debug-watch:{i}"),
                text: expr.clone(),
            });
        }
    }

    /// A right-click on a row of Run and Debug: a variable's menu.
    pub(crate) fn debug_row_menu(&mut self, row: Option<String>) {
        let Some(row) = row else {
            return;
        };
        let Some(rest) = row.strip_prefix("dbg:v:") else {
            return;
        };
        let mut parts = rest.splitn(4, ':');
        let (_, _, parent, name) = (parts.next(), parts.next(), parts.next(), parts.next());
        let (Some(parent), Some(name)) = (parent.and_then(|p| p.parse::<u32>().ok()), name) else {
            return;
        };
        let value = self
            .debug
            .session
            .as_ref()
            .and_then(|s| s.variables.get(&parent))
            .and_then(|l| l.iter().find(|v| v.name == name))
            .map(|v| v.value.clone());
        // A scope has no value to set or copy.
        let Some(value) = value.filter(|_| parent != 0) else {
            return;
        };
        self.debug.menu_var = Some((parent, name.to_string(), value));
        self.context = Some(super::context::ContextMenu {
            items: vec!["debug.var_set", "debug.var_copy", "debug.var_watch"],
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    /// The variable menu's items.
    fn var_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let Some((parent, name, value)) = self.debug.menu_var.clone() else {
            return;
        };
        match id {
            "debug.var_copy" => {
                fx.push(Effect::Copy(value));
                self.notify_done("Copied".into());
            }
            "debug.var_watch" => {
                self.debug.watches.push(name);
                self.ask_watches(fx);
                self.build_views();
            }
            _ => {
                self.debug.setting = Some((parent, name));
                self.build_views();
                self.focus_input(Side::Left, "set");
            }
        }
    }

    /// Focuses a view's input by its id.
    fn focus_input(&mut self, side: Side, id: &str) {
        let at = self
            .panel_targets(side)
            .iter()
            .position(|t| matches!(t, crate::panel::Target::Input(i) if i == id));
        if let Some(at) = at {
            self.focus = side.focus();
            self.select(side, at);
        }
    }

    /// Enter in one of Run and Debug's boxes: a watch to add, or a
    /// variable's new value.
    pub(crate) fn debug_submit(&mut self, input: &str, text: &str, fx: &mut Vec<Effect>) {
        self.inputs.remove(&(DEBUG.to_string(), input.to_string()));
        match input {
            "watch" => {
                self.debug.watches.push(text.trim().to_string());
                self.ask_watches(fx);
            }
            "set" => {
                if let (Some((parent, name)), Some(session)) =
                    (self.debug.setting.take(), &self.debug.session)
                {
                    fx.push(Effect::PluginSubmit {
                        plugin: session.driver.clone(),
                        input: format!("debug-set:{parent}:{name}"),
                        text: text.trim().to_string(),
                    });
                    self.debug.rewatch = true;
                }
                self.focus = Focus::Editor;
            }
            _ => {}
        }
        self.build_views();
    }

    /// Enter in the Debug Console's box.
    pub(crate) fn console_submit(&mut self, text: &str, fx: &mut Vec<Effect>) {
        self.inputs
            .remove(&(CONSOLE.to_string(), "console".to_string()));
        let Some(session) = &self.debug.session else {
            return;
        };
        let plugin = session.driver.clone();
        self.debug
            .push_console(OutputKind::Console, &format!("› {text}\n"));
        fx.push(Effect::PluginSubmit {
            plugin,
            input: "debug-console".into(),
            text: text.to_string(),
        });
        self.build_views();
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

    /// Breakpoints, for the workspace to keep.
    pub fn breakpoints(&self) -> Vec<(PathBuf, u32, String, String, String, bool)> {
        self.debug
            .breakpoints
            .iter()
            .map(|b| {
                (
                    b.path.clone(),
                    b.line,
                    b.condition.clone(),
                    b.hit.clone(),
                    b.log.clone(),
                    b.enabled,
                )
            })
            .collect()
    }
}

/// The test binaries cargo's `--no-run` printed, as source and binary:
/// `Executable unittests src/lib.rs (target/debug/deps/x-1a2b)`. A
/// terminal may have wrapped the line at its width, so the binary runs on
/// to its closing bracket across lines.
fn executables(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("Executable ") {
        rest = &rest[at + "Executable ".len()..];
        let Some(close) = rest.find(')') else { break };
        let entry: String = rest[..close]
            .chars()
            .filter(|c| *c != '\n' && *c != '\r')
            .collect();
        rest = &rest[close..];
        let entry = entry.strip_prefix("unittests ").unwrap_or(&entry);
        if let Some((src, bin)) = entry.split_once(" (") {
            found.push((src.trim().to_string(), bin.trim().to_string()));
        }
    }
    found
}

/// A type without its modules' paths, and shortened, to fit beside a
/// variable: `core::array::iter::IntoIter<&str, 3>` is `IntoIter<&str, 3>`.
fn short_type(kind: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    let mut chars = kind.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ':' && chars.peek() == Some(&':') {
            chars.next();
            word.clear();
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            out.push_str(&word);
            word.clear();
            out.push(c);
        }
    }
    out.push_str(&word);
    if out.chars().count() > 24 {
        format!("{}…", out.chars().take(23).collect::<String>())
    } else {
        out
    }
}

/// A value shortened for one line.
fn short_value(value: &str) -> String {
    let one: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        format!("{}…", one.chars().take(59).collect::<String>())
    } else {
        one
    }
}

#[cfg(test)]
mod tests {
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
    fn a_tests_binary_comes_from_what_its_build_printed() {
        let mut e = Editor::new("fn main() {}\n", Some(PathBuf::from("src/main.rs")));
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
        let mut fx = Vec::new();
        let target = DebugTarget {
            name: "Test parses".into(),
            language: "rust".into(),
            args: vec!["parses".into()],
            build: "cargo test --no-run -p demo".into(),
            ..Default::default()
        };
        e.debug_test(target, "src/main.rs".into(), &mut fx);
        let term = e.debug_building().expect("the build runs");
        e.update(Event::TaskOutput {
            id: term,
            text: "   Compiling demo\n  Executable unittests src/main.rs (target/debug/deps/demo-1a2b)\n"
                .into(),
        });
        let fx = e.update(Event::TermExited {
            id: term,
            code: Some(0),
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
    fn a_binary_wrapped_by_the_terminal_is_joined() {
        let text = "  Executable unittests src/main.rs (target/debug/deps/dbgdemo-79f066730ab665\nf5)\n  Executable tests/find.rs (target/debug/deps/find-12)\n";
        assert_eq!(
            executables(text),
            [
                (
                    "src/main.rs".to_string(),
                    "target/debug/deps/dbgdemo-79f066730ab665f5".to_string()
                ),
                (
                    "tests/find.rs".to_string(),
                    "target/debug/deps/find-12".to_string()
                ),
            ]
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
