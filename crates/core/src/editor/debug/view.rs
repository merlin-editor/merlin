//! The Run and Debug view and the Debug Console: threads, the stack,
//! variables and watches as rows, the bar's buttons, and what they do.

use super::*;

/// A type without its modules' paths, and shortened, to fit beside a
/// variable: `core::array::iter::IntoIter<&str, 3>` is `IntoIter<&str, 3>`.
pub(super) fn short_type(kind: &str) -> String {
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
pub(super) fn short_value(value: &str) -> String {
    let one: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 60 {
        format!("{}…", one.chars().take(59).collect::<String>())
    } else {
        one
    }
}

/// An address cut in the middle, as `0x7ffe…a4c`.
fn short_address(address: &str) -> Option<String> {
    let hex = address.strip_prefix("0x").filter(|h| !h.is_empty())?;
    Some(if hex.len() > 8 {
        format!("0x{}…{}", &hex[..4], &hex[hex.len() - 3..])
    } else {
        address.to_string()
    })
}

impl Editor {
    /// The toolbar, while a run is on.
    pub(crate) fn debug_bar(&self) -> Option<DebugBarView> {
        let session = self.debug.session.as_ref()?;
        Some(DebugBarView {
            paused: matches!(session.state, DebugState::Paused { .. }),
            building: session.building.is_some(),
            by_instruction: self.debug.by_instruction,
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
            Node::entry(Entry {
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
                    Node::entry(Entry {
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
                    nodes.push(Node::entry(Entry {
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
                         it, such as a project's programs."
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
                nodes.push(Node::entry(Entry {
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
                nodes.push(Node::entry(Entry {
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
                    nodes.push(Node::entry(Entry {
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
                    multiline: false,
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
            nodes.push(Node::entry(Entry {
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
        // Breakpoints on instructions, from Disassembly.
        for address in &self.debug.instruction_breaks {
            nodes.push(Node::entry(Entry {
                id: format!("dbg:asm:{address}"),
                label: address.clone(),
                detail: "instruction".into(),
                code: true,
                icon: Some(('●', Hue::Red)),
                action: Some(('✕', format!("dbg:rib:{address}"))),
                ..Default::default()
            }));
        }
        // How Step goes: past library code, an instruction at a time.
        nodes.push(heading("Stepping"));
        for (id, label, on) in [
            (
                "dbg:skip-libraries",
                "Skip library code",
                !self.debug.into_libraries,
            ),
            (
                "dbg:by-instruction",
                "Step by instruction",
                self.debug.by_instruction,
            ),
        ] {
            nodes.push(Node::entry(Entry {
                id: id.into(),
                label: label.into(),
                switch: Some(on),
                ..Default::default()
            }));
        }
        if !self.debug.filters.is_empty() {
            nodes.push(heading("Stop on"));
            for f in &self.debug.filters {
                nodes.push(Node::entry(Entry {
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

    pub(super) fn variable_nodes(
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
                    multiline: false,
                });
            }
            nodes.push(Node::entry(Entry {
                id: format!("dbg:v:{}:{}:{parent}:{}", v.children, nodes.len(), v.name),
                label: if scope || v.value.is_empty() {
                    v.name.clone()
                } else {
                    format!("{} = {}", v.name, short_value(&v.value))
                },
                depth,
                twisty: (v.children != 0).then_some(if open { '▾' } else { '▸' }),
                // The type, and where it is: `int · 0x7ffe…a4c`.
                badge: match (short_type(&v.kind), short_address(&v.address)) {
                    (kind, None) => kind,
                    (kind, Some(at)) if kind.is_empty() => at,
                    (kind, Some(at)) => format!("{kind} · {at}"),
                },
                code: !scope,
                ..Default::default()
            }));
            if open {
                match session.variables.get(&v.children) {
                    Some(children) => {
                        self.variable_nodes(session, children, v.children, depth + 1, nodes)
                    }
                    None => nodes.push(Node::entry(Entry {
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

    pub(super) fn stack_nodes(&self, session: &Session, nodes: &mut Vec<Node>) {
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
                nodes.push(Node::entry(Entry {
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
                        nodes.push(Node::entry(Entry {
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
                nodes.push(Node::entry(Entry {
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
                multiline: false,
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
        if let Some(n) = rest.strip_prefix("unit:").and_then(|n| n.parse().ok()) {
            self.debug.memory_unit = n;
            return self.build_views();
        }
        if rest == "refresh" {
            let expr = self.debug.memory_expr.clone();
            if !expr.is_empty() {
                self.send_driver(&format!("debug.memory:{expr}"), fx);
            }
            return;
        }
        if let Some(address) = rest.strip_prefix("asm:") {
            let address = address.to_string();
            if !self.debug.instruction_breaks.contains(&address) {
                self.debug.instruction_breaks.push(address);
            } else {
                self.debug.instruction_breaks.retain(|a| *a != address);
            }
            let list = self.debug.instruction_breaks.join(",");
            self.send_driver(&format!("debug.instruction-breakpoints:{list}"), fx);
            return self.build_views();
        }
        if let Some(address) = rest.strip_prefix("rib:") {
            self.debug.instruction_breaks.retain(|a| a != address);
            let list = self.debug.instruction_breaks.join(",");
            self.send_driver(&format!("debug.instruction-breakpoints:{list}"), fx);
            return self.build_views();
        }
        if rest == "skip-libraries" {
            self.debug.into_libraries = !self.debug.into_libraries;
            let on = if self.debug.into_libraries {
                "off"
            } else {
                "on"
            };
            self.send_driver(&format!("debug.skip-libraries:{on}"), fx);
            return self.build_views();
        }
        if rest == "by-instruction" {
            self.debug.by_instruction = !self.debug.by_instruction;
            let on = if self.debug.by_instruction {
                "on"
            } else {
                "off"
            };
            self.send_driver(&format!("debug.by-instruction:{on}"), fx);
            return self.build_views();
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
    pub(super) fn ask_watches(&mut self, fx: &mut Vec<Effect>) {
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
        // A frame: Open Disassembly there.
        if let Some((thread, frame)) = row
            .strip_prefix("dbg:fr:")
            .and_then(|r| r.split_once(':'))
            .and_then(|(t, f)| Some((t.parse().ok()?, f.parse().ok()?)))
        {
            self.debug.menu_frame = Some((thread, frame));
            self.context = Some(super::context::ContextMenu {
                items: vec!["debug.frame_disassembly"],
                selected: None,
                title: None,
            });
            self.focus = Focus::Menu;
            return;
        }
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
            items: vec![
                "debug.var_set",
                "debug.var_copy",
                "debug.var_watch",
                "debug.var_memory",
            ],
            selected: None,
            title: None,
        });
        self.focus = Focus::Menu;
    }

    /// The variable menu's items.
    pub(super) fn var_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
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
    pub(super) fn focus_input(&mut self, side: Side, id: &str) {
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
        self.input_cursors
            .remove(&(DEBUG.to_string(), input.to_string()));
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

    /// The Disassembly view: the instructions around the pause, the
    /// source line over each run of them, the paused one marked.
    pub(crate) fn disassembly_nodes(&self) -> Vec<Node> {
        if self.debug.disassembly.is_empty() {
            let text = if self.debug.session.is_some() {
                "The instructions show when the program pauses."
            } else {
                "Start debugging, and the instructions where it pauses show here."
            };
            return vec![Node::Styled(text.into(), Tone::Muted)];
        }
        let mut nodes = Vec::new();
        for l in &self.debug.disassembly {
            if let Some((path, line)) = &l.source {
                let text = self
                    .docs
                    .iter()
                    .find(|d| {
                        d.buffer
                            .path
                            .as_deref()
                            .is_some_and(|p| self.same_file(p, path))
                    })
                    .map(|d| d.line_chars(*line as usize).into_iter().collect::<String>())
                    .map(|t| t.trim().to_string())
                    .unwrap_or_else(|| {
                        path.file_name()
                            .map_or(String::new(), |n| n.to_string_lossy().into_owned())
                    });
                nodes.push(Node::entry(Entry {
                    id: String::new(),
                    label: format!("{:>4}  {text}", line + 1),
                    code: true,
                    tone: Tone::Muted,
                    ..Default::default()
                }));
            }
            // A click puts a breakpoint on it, or takes it off.
            let breaks = self.debug.instruction_breaks.contains(&l.address);
            nodes.push(Node::entry(Entry {
                id: format!("dbg:asm:{}", l.address),
                label: format!("{:<14} {}", l.address, l.instruction),
                detail: l.symbol.clone(),
                code: true,
                tone: if l.current {
                    Tone::Warning
                } else {
                    Tone::Normal
                },
                icon: if l.current {
                    Some(('▶', Hue::Yellow))
                } else if breaks {
                    Some(('●', Hue::Red))
                } else {
                    Some((' ', Hue::Grey))
                },
                ..Default::default()
            }));
        }
        nodes
    }

    /// The Memory view: a box for an address or expression, then rows of
    /// sixteen bytes and their text, the expression's own tinted and the
    /// ones that changed since the last pause in amber.
    pub(crate) fn memory_nodes(&self) -> Vec<Node> {
        let mut nodes = vec![Node::Input {
            id: "memory".into(),
            placeholder: "An address or expression, such as &total, then Enter".into(),
            secret: false,
            multiline: false,
        }];
        let Some(m) = &self.debug.memory else {
            let text = if self.debug.session.is_none() {
                "Memory shows while the program is paused."
            } else {
                "Type an address or an expression above, or right-click a variable and Show in Memory."
            };
            nodes.push(Node::Styled(text.into(), Tone::Muted));
            return nodes;
        };
        if !m.error.is_empty() {
            nodes.push(Node::Styled(m.error.clone(), Tone::Error));
            return nodes;
        }
        let (from, len) = m.mark;
        nodes.push(Node::Styled(
            format!(
                "{} · 0x{:x} · {len} byte{}",
                m.expression,
                m.address + from as u64,
                if len == 1 { "" } else { "s" }
            ),
            Tone::Muted,
        ));
        // How the bytes go: one at a time, or as numbers of 2, 4 or 8, the
        // way the machine (little-endian) holds them; and Refresh.
        let unit = self.debug.memory_unit.clamp(1, 8);
        let chip = |id: &str, label: &str, on: bool| crate::panel::Chip {
            id: id.into(),
            label: label.into(),
            tone: if on { Tone::Strong } else { Tone::Muted },
        };
        let mut chips: Vec<crate::panel::Chip> =
            [(1, "Bytes"), (2, "16-bit"), (4, "32-bit"), (8, "64-bit")]
                .iter()
                .map(|(n, label)| chip(&format!("dbg:unit:{n}"), label, *n == unit))
                .collect();
        chips.push(chip("dbg:refresh", "Refresh", false));
        nodes.push(Node::Chips {
            chips,
            hint: String::new(),
        });
        for (row, chunk) in m.bytes.chunks(16).enumerate() {
            let base = row * 16;
            let mut label = format!("0x{:012x}  ", m.address + base as u64);
            let mut spans = Vec::new();
            for (k, group) in chunk.chunks(unit).enumerate() {
                let first = base + k * unit;
                if k * unit == 8 {
                    label.push(' ');
                }
                let at = label.chars().count();
                // Little-endian: the last byte is the highest.
                for b in group.iter().rev() {
                    label.push_str(&format!("{b:02x}"));
                }
                let indices = first..first + group.len();
                let tone = if indices.clone().any(|i| m.changed.contains(&i)) {
                    Some(Tone::Warning)
                } else if indices.clone().any(|i| (from..from + len).contains(&i)) {
                    Some(Tone::Strong)
                } else {
                    None
                };
                if let Some(tone) = tone {
                    spans.push((at, at + group.len() * 2, tone));
                }
                label.push(' ');
            }
            // Short last rows keep the text in its column.
            let shown = chunk.len();
            if shown < 16 {
                let groups = (16 - shown).div_ceil(unit);
                label.push_str(&" ".repeat(groups * (unit * 2 + 1) + usize::from(shown <= 8)));
            }
            label.push(' ');
            label.extend(chunk.iter().map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            }));
            nodes.push(Node::entry(Entry {
                id: String::new(),
                label,
                code: true,
                spans,
                ..Default::default()
            }));
        }
        nodes
    }
}
