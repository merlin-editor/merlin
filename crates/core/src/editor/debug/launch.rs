//! Starting a run: `.merlin/launch.toml`'s configurations, picking a
//! target or a process, its build, and handing it to the debugger's driver.

use super::*;

/// Merges JSON objects, later keys over earlier ones; text that isn't an
/// object is left out.
pub(super) fn merge_json(parts: &[&str]) -> String {
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
pub(super) fn json_fields(text: &str) -> Vec<(String, String)> {
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

/// The test binaries cargo's `--no-run` printed, as source and binary:
/// `Executable unittests src/lib.rs (target/debug/deps/x-1a2b)`. A
/// terminal may have wrapped the line at its width, so the binary runs on
/// to its closing bracket across lines.
pub(super) fn executables(text: &str) -> Vec<(String, String)> {
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

impl Editor {
    /// F5: reads launch.toml first, unless a run is on, then goes on.
    pub(super) fn debug_read_then(&mut self, without_debugger: bool, fx: &mut Vec<Effect>) {
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
    pub(super) fn debug_start(&mut self, without_debugger: bool, fx: &mut Vec<Effect>) {
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
    pub(super) fn debug_pick(&mut self) {
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
    pub(super) fn launch_target(&mut self, target: DebugTarget, fx: &mut Vec<Effect>) {
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
    pub(super) fn start_driver(&mut self, fx: &mut Vec<Effect>) {
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
    pub(super) fn debugger_missing(&mut self, target: &DebugTarget, debuggers: &[Debugger]) {
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
}
