//! The integrated terminal: which shells are open, which show side by
//! side in the panel, and where keys go. The host runs the shells and
//! keeps their screens; the core only names them.

use super::*;

/// The terminal's view in the panel under the editor.
pub(crate) const TERMINAL: &str = "merlin.terminal";

/// Commands only the workbench runs, even while a terminal has the keys,
/// as VS Code's `commandsToSkipShell` has them.
const SKIP_SHELL: &[&str] = &[
    "palette.open",
    "file.open",
    "file.open_file",
    "app.quit",
    "settings.open",
    "view.sidebar",
    "view.explorer",
    "view.search",
    "view.changes",
    "view.panel",
    "view.problems",
    "view.output",
    "panel.toggle",
    "view.zoom_in",
    "view.zoom_out",
    "view.zoom_reset",
    "view.group1",
    "view.group2",
    "view.group3",
    "file.next",
    "file.prev",
];

/// Keys the terminal itself answers, before the shell sees them.
const TERMINAL_KEYS: &[(&str, &str)] = &[
    ("ctrl+shift+c", "terminal.copy"),
    ("ctrl+shift+v", "terminal.paste"),
    ("shift+pageup", "terminal.scroll_up"),
    ("shift+pagedown", "terminal.scroll_down"),
    ("ctrl+shift+5", "terminal.split"),
    ("ctrl+shift+`", "terminal.new"),
    ("ctrl+`", "terminal.toggle"),
];

/// What the host does with a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermOp {
    /// Starts a shell in `cwd`, the project's folder.
    Spawn {
        id: u64,
    },
    /// A key, for the host to encode as the shell's modes want it.
    Key {
        id: u64,
        key: Key,
    },
    /// Text, bracketed if the shell asked for that.
    Paste {
        id: u64,
        text: String,
    },
    /// Scrolls back through the history by lines, up for positive.
    Scroll {
        id: u64,
        lines: i32,
    },
    /// Scrolls back or forward a page, as tall as the host shows it.
    Page {
        id: u64,
        up: bool,
    },
    /// Copies the selection to the clipboard.
    Copy {
        id: u64,
    },
    Kill {
        id: u64,
    },
}

/// One open shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TermSession {
    pub id: u64,
    pub title: String,
    /// Its exit code once it's ended; it stays listed until killed.
    pub exited: Option<i32>,
}

/// The terminal panel, as the window draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalView {
    pub sessions: Vec<TermSession>,
    /// The sessions showing side by side, left to right.
    pub panes: Vec<u64>,
    /// The one keys go to.
    pub active: Option<u64>,
    /// Whether keys go to the terminal now.
    pub focused: bool,
}

#[derive(Debug, Default)]
pub(crate) struct Terminals {
    pub sessions: Vec<TermSession>,
    /// Groups of sessions split side by side; the shown one is the active
    /// session's.
    pub splits: Vec<Vec<u64>>,
    pub active: Option<u64>,
    next: u64,
}

impl Terminals {
    fn split_of(&self, id: u64) -> Option<usize> {
        self.splits.iter().position(|s| s.contains(&id))
    }
}

impl Editor {
    /// Whether keys go to a terminal now.
    pub(crate) fn terminal_focused(&self) -> bool {
        self.focus == Focus::Bottom
            && self.bar(Side::Bottom).visible
            && self.active_panel_name(Side::Bottom).as_deref() == Some(TERMINAL)
            && self.terminals.active.is_some()
    }

    /// A key while the terminal has the keys: the terminal's own keys, the
    /// workbench's few, and the rest to the shell.
    pub(crate) fn terminal_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if !self.terminal_focused() || self.chord.is_some() {
            return false;
        }
        if let Some((_, id)) = TERMINAL_KEYS
            .iter()
            .find(|(k, _)| Key::parse(k) == Some(key))
        {
            self.run(id, fx);
            return true;
        }
        if let Some(binding) = self.binding_for(&[key]) {
            let from_user = binding.from == keymap::From::Settings;
            if from_user || SKIP_SHELL.contains(&binding.command.as_str()) {
                return false;
            }
        }
        let Some(id) = self.terminals.active else {
            return false;
        };
        fx.push(Effect::Terminal(TermOp::Key { id, key }));
        true
    }

    /// Pasted text goes to the shell when it has the keys.
    pub(crate) fn terminal_paste(&mut self, text: &str, fx: &mut Vec<Effect>) -> bool {
        match self.terminals.active {
            Some(id) if self.terminal_focused() => {
                fx.push(Effect::Terminal(TermOp::Paste {
                    id,
                    text: text.to_string(),
                }));
                true
            }
            _ => false,
        }
    }

    /// The `terminal.` commands.
    pub(crate) fn terminal_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let active = self.terminals.active;
        match id {
            "terminal.new" => self.new_terminal(false, fx),
            "terminal.split" => self.new_terminal(true, fx),
            "terminal.toggle" => {
                if self.terminal_focused() {
                    self.toggle_bottom();
                } else if self.terminals.sessions.is_empty() {
                    self.new_terminal(false, fx);
                } else {
                    self.show_terminal();
                }
            }
            "terminal.kill" => {
                if let Some(id) = active {
                    self.kill_terminal(id, fx);
                }
            }
            "terminal.copy" => {
                if let Some(id) = active {
                    fx.push(Effect::Terminal(TermOp::Copy { id }));
                }
            }
            "terminal.paste" => fx.push(Effect::ReadClipboard),
            "terminal.scroll_up" | "terminal.scroll_down" => {
                if let Some(session) = active {
                    fx.push(Effect::Terminal(TermOp::Page {
                        id: session,
                        up: id == "terminal.scroll_up",
                    }));
                }
            }
            "terminal.focus_next" | "terminal.focus_prev" => {
                let ids: Vec<u64> = self.terminals.sessions.iter().map(|s| s.id).collect();
                if let Some(at) = active.and_then(|a| ids.iter().position(|&i| i == a)) {
                    let n = ids.len();
                    let to = if id == "terminal.focus_next" {
                        (at + 1) % n
                    } else {
                        (at + n - 1) % n
                    };
                    self.terminals.active = Some(ids[to]);
                }
            }
            _ => {}
        }
    }

    /// Starts a shell, beside the active one with `split`, and shows it.
    fn new_terminal(&mut self, split: bool, fx: &mut Vec<Effect>) {
        let t = &mut self.terminals;
        t.next += 1;
        let id = t.next;
        t.sessions.push(TermSession {
            id,
            title: "shell".into(),
            exited: None,
        });
        match t.active.and_then(|a| t.split_of(a)) {
            Some(at) if split => {
                let row = &mut t.splits[at];
                let after = row
                    .iter()
                    .position(|&i| Some(i) == t.active)
                    .map_or(row.len(), |p| p + 1);
                row.insert(after, id);
            }
            _ => t.splits.push(vec![id]),
        }
        t.active = Some(id);
        fx.push(Effect::Terminal(TermOp::Spawn { id }));
        self.show_terminal();
    }

    /// Shows the terminal panel and gives it the keys.
    fn show_terminal(&mut self) {
        let bar = self.bar_mut(Side::Bottom);
        bar.visible = true;
        self.show_bottom(TERMINAL);
    }

    pub(crate) fn kill_terminal(&mut self, id: u64, fx: &mut Vec<Effect>) {
        let t = &mut self.terminals;
        let Some(at) = t.sessions.iter().position(|s| s.id == id) else {
            return;
        };
        fx.push(Effect::Terminal(TermOp::Kill { id }));
        t.sessions.remove(at);
        for row in &mut t.splits {
            row.retain(|&i| i != id);
        }
        t.splits.retain(|row| !row.is_empty());
        if t.active == Some(id) {
            t.active = t
                .sessions
                .get(at.min(t.sessions.len().saturating_sub(1)))
                .map(|s| s.id);
        }
        if t.sessions.is_empty() && self.focus == Focus::Bottom {
            self.focus = Focus::Editor;
            self.bar_mut(Side::Bottom).visible = false;
        }
    }

    /// A shell's title changed, as shells set it for the running command.
    pub(crate) fn terminal_title(&mut self, id: u64, title: String) {
        if let Some(s) = self.terminals.sessions.iter_mut().find(|s| s.id == id) {
            s.title = title;
        }
    }

    pub(crate) fn terminal_exited(&mut self, id: u64, code: Option<i32>) {
        if let Some(s) = self.terminals.sessions.iter_mut().find(|s| s.id == id) {
            s.exited = Some(code.unwrap_or(0));
        }
    }

    /// A click on a session in the list, or in a pane: it gets the keys.
    pub(crate) fn focus_terminal(&mut self, id: u64) {
        if self.terminals.sessions.iter().any(|s| s.id == id) {
            self.terminals.active = Some(id);
            self.show_terminal();
        }
    }

    pub(crate) fn terminal_view(&self) -> Option<TerminalView> {
        let bar = self.bar(Side::Bottom);
        if !bar.visible || self.active_panel_name(Side::Bottom).as_deref() != Some(TERMINAL) {
            return None;
        }
        let t = &self.terminals;
        let panes = t
            .active
            .and_then(|a| t.split_of(a))
            .map(|at| t.splits[at].clone())
            .unwrap_or_default();
        Some(TerminalView {
            sessions: t.sessions.clone(),
            panes,
            active: t.active,
            focused: self.terminal_focused(),
        })
    }
}
