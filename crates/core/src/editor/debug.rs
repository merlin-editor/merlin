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
}

#[derive(Debug, Default)]
pub(crate) struct Debugging {
    pub debuggers: Vec<Debugger>,
    /// Programs found on `PATH`, by name, once looked for.
    pub installed: HashMap<String, bool>,
    pub targets: Vec<(String, Vec<DebugTarget>)>,
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
    /// Stop filters the user turned on, by id, kept across runs.
    pub stop_on: HashSet<String>,
    pub expanded: HashSet<u32>,
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
        self.targets.iter().flat_map(|(_, t)| t.iter())
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
            "debug.start" => self.debug_start(false, fx),
            "debug.run" => self.debug_start(true, fx),
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
            "debug.continue" if !self.debug.running() => self.debug_start(false, fx),
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

    /// A target chosen from the picker or the view.
    pub(crate) fn debug_picked(&mut self, item: &str, fx: &mut Vec<Effect>) {
        if let Some(name) = item.strip_prefix("t:") {
            self.debug.chosen = Some(name.to_string());
            self.debug_start(false, fx);
        }
    }

    /// Builds a target if it says to, then starts it.
    fn launch_target(&mut self, target: DebugTarget, fx: &mut Vec<Effect>) {
        let debuggers: Vec<Debugger> =
            self.debug.debuggers_for(&target).into_iter().cloned().collect();
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
        if target.program.is_empty() {
            base = format!("{{\"name\":{},\"cwd\":{}}}", json_string(&target.name), json_string(&root));
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
            threads: Vec::new(),
            stacks: HashMap::new(),
            frame: None,
            variables: HashMap::new(),
            inline: HashMap::new(),
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

    /// A build run first ended: the run starts if it worked.
    pub(crate) fn debug_build_exited(&mut self, term: u64, code: Option<i32>, fx: &mut Vec<Effect>) {
        if self.debug_building() != Some(term) {
            return;
        }
        if code == Some(0) {
            return self.start_driver(fx);
        }
        let name = self
            .debug
            .session
            .take()
            .map(|s| s.start.name)
            .unwrap_or_default();
        self.build_views();
        self.error(format!("{name} didn't build, so debugging didn't start"));
        self.add_action("Show Problems", "view.problems");
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

    fn breakpoints_changed(&mut self, fx: &mut Vec<Effect>) {
        if self.debug.running() {
            self.send_debug("debug.breakpoints", fx);
        }
        self.build_views();
    }

    pub(crate) fn debug_paused(&self) -> bool {
        self.debug.paused().is_some()
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
            || frame.line as usize - line > 40
        {
            return None;
        }
        let scopes = session.variables.get(&0)?;
        let locals = scopes
            .first()
            .and_then(|s| session.variables.get(&s.children))?;
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
                if parent == 0 {
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
                    } else {
                        self.debug.stop_on.remove(&f.id);
                    }
                }
            }
            DebugUpdate::InlineValues { path, values } => {
                session.inline.insert(path, values);
            }
            DebugUpdate::Output(kind, text) => self.debug.push_console(kind, &text),
        }
        self.build_views();
    }

    /// Opens the shown frame's file at its line.
    fn reveal_frame(&mut self, fx: &mut Vec<Effect>) {
        let Some(frame) = self.debug.current_frame().cloned() else {
            return;
        };
        if let Some(path) = frame.path {
            let path = path.strip_prefix(&self.root).map(Path::to_path_buf).unwrap_or(path);
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
                nodes.push(heading("Found in the project"));
                let targets: Vec<&DebugTarget> = self.debug.all_targets().collect();
                if targets.is_empty() {
                    nodes.push(Node::Styled(
                        "Nothing yet. Language plugins list what can be debugged when they see \
                         it, as rust-analyzer does for a crate's binaries."
                            .into(),
                        Tone::Muted,
                    ));
                }
                for t in targets {
                    nodes.push(Node::Entry(Entry {
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
                        badge: t.detail.clone(),
                        ..Default::default()
                    }));
                }
            }
            Some(session) => {
                let state = match &session.state {
                    _ if session.building.is_some() => ("Building".to_string(), Tone::Warning),
                    DebugState::Starting => ("Starting".into(), Tone::Muted),
                    DebugState::Running => ("Running".into(), Tone::Normal),
                    DebugState::Paused { reason, description, .. } => {
                        let why = match reason.as_str() {
                            "breakpoint" | "function breakpoint" => "Paused on breakpoint".into(),
                            "exception" => "Paused on an exception".into(),
                            "step" => "Paused after a step".into(),
                            "entry" => "Paused at the start".into(),
                            "pause" => "Paused".into(),
                            other => format!("Paused: {other}"),
                        };
                        let why = match description.is_empty() || reason != "exception" {
                            true => why,
                            false => format!("{why}: {description}"),
                        };
                        (why, Tone::Warning)
                    }
                    DebugState::Ended(_) => ("Ended".into(), Tone::Muted),
                };
                nodes.push(Node::Entry(Entry {
                    id: "dbg:session".into(),
                    label: session.start.name.clone(),
                    icon: Some(('●', if state.1 == Tone::Warning { Hue::Yellow } else { Hue::Green })),
                    detail: session.start.debugger.clone(),
                    badge: state.0,
                    badge_tone: state.1,
                    ..Default::default()
                }));
                nodes.push(heading("Variables"));
                match session.variables.get(&0) {
                    Some(scopes) => self.variable_nodes(session, scopes, 0, &mut nodes),
                    None => nodes.push(Node::Styled(
                        match self.debug.paused() {
                            Some(_) => "Reading…".into(),
                            None => "Shown while paused".into(),
                        },
                        Tone::Muted,
                    )),
                }
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
                state.map(|s| s.message.clone()).filter(|m| !m.is_empty()).unwrap_or("not set".into())
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
                    if unset || !b.enabled { Hue::Grey } else { Hue::Red },
                )),
                detail,
                badge: (b.line + 1).to_string(),
                tone: if unset || !b.enabled { Tone::Muted } else { Tone::Normal },
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
        depth: usize,
        nodes: &mut Vec<Node>,
    ) {
        for v in list {
            let open = v.children != 0 && self.debug.expanded.contains(&v.children);
            let scope = depth == 0;
            nodes.push(Node::Entry(Entry {
                id: format!("dbg:v:{}:{}", v.children, nodes.len()),
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
                    Some(children) => self.variable_nodes(session, children, depth + 1, nodes),
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
                    badge: if t.paused { "Paused".into() } else { "Running".into() },
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
            return self.debug_start(false, fx);
        }
        if rest == "again" {
            return self.debug_command("debug.restart", fx);
        }
        if let Some(name) = rest.strip_prefix("t:") {
            self.debug.chosen = Some(name.to_string());
            return self.debug_start(false, fx);
        }
        if let Some(id) = rest.strip_prefix("rb:").and_then(|i| i.parse::<u32>().ok()) {
            self.debug.breakpoints.retain(|b| b.id != id);
            self.debug.states.remove(&id);
            return self.breakpoints_changed(fx);
        }
        if let Some(id) = rest.strip_prefix("b:").and_then(|i| i.parse::<u32>().ok()) {
            if let Some(b) = self.debug.breakpoints.iter().find(|b| b.id == id).cloned() {
                let path = b.path.strip_prefix(&self.root).map(Path::to_path_buf).unwrap_or(b.path);
                self.open_at(path, b.line, 0, fx);
            }
            return;
        }
        if let Some(r) = rest.strip_prefix("v:").and_then(|v| v.split(':').next()?.parse::<u32>().ok()) {
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
        if let Some(t) = rest.strip_prefix("lib:").and_then(|t| t.parse::<u32>().ok()) {
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
            let on = !self.debug.stop_on.contains(f);
            if on {
                self.debug.stop_on.insert(f.to_string());
            } else {
                self.debug.stop_on.remove(f);
            }
            if let Some(filter) = self.debug.filters.iter_mut().find(|x| x.id == f) {
                filter.on = on;
            }
            self.send_debug(&format!("debug.stop-on:{f}"), fx);
            self.build_views();
        }
    }

    /// Enter in the Debug Console's box.
    pub(crate) fn console_submit(&mut self, text: &str, fx: &mut Vec<Effect>) {
        self.inputs.remove(&(CONSOLE.to_string(), "console".to_string()));
        let Some(session) = &self.debug.session else {
            return;
        };
        let plugin = session.driver.clone();
        self.debug.push_console(OutputKind::Console, &format!("› {text}\n"));
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
    fn types_drop_their_paths() {
        assert_eq!(short_type("core::array::iter::IntoIter<&str, 3>"), "IntoIter<&str, 3>");
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
