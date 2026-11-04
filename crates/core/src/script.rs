//! Scripts drive the editor without a terminal, for tests and `--exec`.
//!
//! Actions are separated by `;` or newlines:
//!
//! - `type TEXT` presses a key per character (`\n` is Enter, `\t` is Tab)
//! - `key SPEC...` presses keys such as `ctrl+s` or `down`
//! - `paste TEXT` pastes into whatever has focus
//! - `cmd ID` runs a command, such as `file.save`
//! - `click X Y` and `shift-click X Y` press and release the left mouse
//!   button; `press X Y`, `drag X Y` and `release X Y` do one step at a time;
//!   `wheel up X Y` and `wheel down X Y` scroll; `move X Y` moves the
//!   mouse with no button down
//! - `resize WxH`
//! - `wait MS` pauses, and `wait-for TEXT [MS]` waits until TEXT is on
//!   screen, for output from plugin processes (only the app runs these)
//!
//! In TEXT, `\;`, `\n`, `\t` and `\\` are escapes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::editor::{Editor, Effect, Event, Mouse, MouseKind};
use crate::key::{Key, KeyCode};

pub enum Step {
    Event(Event),
    Wait(u64),
    WaitFor { text: String, timeout_ms: u64 },
}

pub fn parse(script: &str) -> Result<Vec<Step>, String> {
    let mut events = Vec::new();
    for action in split(script) {
        let action = action.trim();
        if action.is_empty() {
            continue;
        }
        let (verb, rest) = action.split_once(' ').unwrap_or((action, ""));
        match verb {
            "type" => {
                for c in unescape(rest).chars() {
                    let code = match c {
                        '\n' => KeyCode::Enter,
                        '\t' => KeyCode::Tab,
                        c => KeyCode::Char(c),
                    };
                    events.push(Step::Event(Event::Key(Key::plain(code))));
                }
            }
            "key" => {
                for spec in rest.split_whitespace() {
                    let key = Key::parse(spec).ok_or_else(|| format!("unknown key: {spec}"))?;
                    events.push(Step::Event(Event::Key(key)));
                }
            }
            "paste" => events.push(Step::Event(Event::Paste(unescape(rest)))),
            "cmd" => events.push(Step::Event(Event::Command(rest.trim().to_string()))),
            "click" | "shift-click" | "alt-click" | "ctrl-click" | "double-click"
            | "triple-click" | "press" | "box-press" | "double-press" | "drag" | "release"
            | "wheel" | "move" => {
                let mut args = rest.split_whitespace();
                let kind = match verb {
                    "wheel" => match args.next() {
                        Some("up") => MouseKind::ScrollUp,
                        Some("down") => MouseKind::ScrollDown,
                        Some("left") => MouseKind::ScrollLeft,
                        Some("right") => MouseKind::ScrollRight,
                        _ => return Err(format!("bad wheel: {rest}")),
                    },
                    "drag" => MouseKind::Drag,
                    "release" => MouseKind::Up,
                    "move" => MouseKind::Move,
                    _ => MouseKind::Down,
                };
                let mut number = || -> Result<u16, String> {
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(|| format!("bad position: {action}"))
                };
                let (x, y) = (number()?, number()?);
                // A box press is Shift+Alt, to drag a box selection.
                let shift = matches!(verb, "shift-click" | "box-press");
                let alt = matches!(verb, "alt-click" | "box-press");
                let clicks = match verb {
                    "double-click" | "double-press" => 2,
                    "triple-click" => 3,
                    _ => 1,
                };
                events.push(Step::Event(Event::Mouse(Mouse {
                    kind,
                    x,
                    y,
                    shift,
                    alt,
                    ctrl: verb == "ctrl-click",
                    clicks,
                })));
                if verb.ends_with("click") {
                    let up = Mouse {
                        kind: MouseKind::Up,
                        x,
                        y,
                        shift,
                        alt,
                        ctrl: verb == "ctrl-click",
                        clicks: 1,
                    };
                    events.push(Step::Event(Event::Mouse(up)));
                }
            }
            "resize" => {
                let (w, h) = rest
                    .trim()
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or_else(|| format!("bad size: {rest}"))?;
                events.push(Step::Event(Event::Resize(w, h)));
            }
            "wait" => {
                let ms = rest
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad wait: {rest}"))?;
                events.push(Step::Wait(ms));
            }
            "wait-for" => {
                let (text, timeout_ms) = match rest.rsplit_once(' ') {
                    Some((text, ms)) if ms.parse::<u64>().is_ok() => (text, ms.parse().unwrap()),
                    _ => (rest, 5000),
                };
                events.push(Step::WaitFor {
                    text: unescape(text),
                    timeout_ms,
                });
            }
            other => return Err(format!("unknown action: {other}")),
        }
    }
    Ok(events)
}

fn split(script: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = script.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Keep escapes for unescape(); only `\;` matters here.
                parts.last_mut().unwrap().push(c);
                if let Some(next) = chars.next() {
                    parts.last_mut().unwrap().push(next);
                }
            }
            ';' | '\n' => parts.push(String::new()),
            c => parts.last_mut().unwrap().push(c),
        }
    }
    parts
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// An editor with an in-memory host: files live in `files`, saves succeed
/// instantly, the system clipboard can't be read, and plugin effects are
/// only recorded. For tests.
pub struct Session {
    pub editor: Editor,
    pub files: BTreeMap<String, String>,
    pub saved: Vec<(PathBuf, String)>,
    pub effects: Vec<Effect>,
    pub quit: bool,
}

impl Session {
    pub fn new(text: &str, path: Option<&str>, width: u16, height: u16) -> Self {
        let mut editor = Editor::new(text, path.map(PathBuf::from));
        editor.update(Event::Resize(width, height));
        Self {
            editor,
            files: BTreeMap::new(),
            saved: Vec::new(),
            effects: Vec::new(),
            quit: false,
        }
    }

    pub fn run(&mut self, script: &str) -> &mut Self {
        for step in parse(script).unwrap() {
            if let Step::Event(event) = step {
                self.send(event);
            }
        }
        self
    }

    pub fn send(&mut self, event: Event) {
        for effect in self.editor.update(event) {
            self.effects.push(effect.clone());
            match effect {
                Effect::Save {
                    doc,
                    path,
                    text,
                    revision,
                } => {
                    self.files.insert(path.display().to_string(), text.clone());
                    self.saved.push((path.clone(), text));
                    self.send(Event::Saved {
                        doc,
                        revision,
                        path,
                    });
                }
                Effect::Open(path) => {
                    let event = match self.files.get(&path.display().to_string()) {
                        Some(text) => Event::Opened {
                            text: text.clone(),
                            path,
                        },
                        None => Event::Error(format!("no such file: {}", path.display())),
                    };
                    self.send(event);
                }
                Effect::ListFiles => self.send(Event::Files(self.files.keys().cloned().collect())),
                Effect::ListTree => self.send(Event::Tree(self.files.keys().cloned().collect())),
                Effect::Search(query) => {
                    let hits = self
                        .files
                        .iter()
                        .flat_map(|(path, text)| {
                            text.lines().enumerate().filter_map(|(line, text)| {
                                let column = text.find(&query)?;
                                Some(crate::SearchHit {
                                    path: path.clone(),
                                    line: line as u32,
                                    column: text[..column].encode_utf16().count() as u32,
                                    text: text.to_string(),
                                })
                            })
                        })
                        .collect();
                    self.send(Event::SearchResults { query, hits });
                }
                Effect::ReadClipboard => self.send(Event::Clipboard(None)),
                Effect::Quit => self.quit = true,
                Effect::Copy(_)
                | Effect::PluginCommand { .. }
                | Effect::PluginClick { .. }
                | Effect::PluginSubmit { .. }
                | Effect::PluginPick { .. }
                | Effect::PluginReview { .. }
                | Effect::SetSetting { .. }
                | Effect::OpenSettings { .. }
                | Effect::PreviewTheme(..)
                | Effect::SetTheme(..)
                | Effect::OpenSettingsWindow
                | Effect::Zoom(_)
                | Effect::GitStatus
                | Effect::OpenDiff(_)
                | Effect::Git(_) => {}
            }
        }
    }

    pub fn text(&self) -> String {
        self.editor.buffer().text()
    }

    pub fn screen(&self) -> String {
        self.editor.render().to_text()
    }
}
