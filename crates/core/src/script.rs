//! Scripts drive the editor without a window, for tests and `--exec`.
//!
//! Actions are separated by `;` or newlines:
//!
//! - `type TEXT` presses a key per character (`\n` is Enter, `\t` is Tab)
//! - `key SPEC...` presses keys such as `ctrl+s` or `down`
//! - `paste TEXT` pastes into whatever has focus
//! - `cmd ID` runs a command, such as `file.save`
//! - `click COL ROW` and `shift-click COL ROW` press and release the left
//!   mouse button in a group's text, at a column from its left edge (the
//!   gutter's included) and a row from the top of the view; `press`, `drag`
//!   and `release` do one step at a time; `move COL ROW` moves the mouse
//!   with no button down, `rest COL ROW` rests it there for a hover, and
//!   `right-click COL ROW` opens the menu there. They're in the focused group, or in group `N`
//!   (from 1) after `group N`
//! - `wheel up`, `wheel down`, `wheel left` and `wheel right` scroll the
//!   first group, or group `N`
//! - `resize WxH`
//! - `wait MS` pauses, and `wait-for TEXT [MS]` waits until TEXT is on
//!   screen, for output from plugin processes (only the app runs these)
//!
//! In TEXT, `\;`, `\n`, `\t` and `\\` are escapes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::editor::{Editor, Effect, Event, MouseKind, Pointer};
use crate::key::{Key, KeyCode};
use crate::style::Cell;

pub enum Step {
    Event(Event),
    Wait(u64),
    WaitFor { text: String, timeout_ms: u64 },
}

pub fn parse(script: &str) -> Result<Vec<Step>, String> {
    let mut events = Vec::new();
    // The group mouse actions are in: the focused one, until `group N`.
    let mut group = usize::MAX;
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
            "group" => {
                let n: usize = rest
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad group: {rest}"))?;
                group = n.checked_sub(1).ok_or("groups count from 1")?;
            }
            "wheel" => {
                let mut args = rest.split_whitespace();
                let (rows, cols) = match args.next() {
                    Some("up") => (-3.0, 0.0),
                    Some("down") => (3.0, 0.0),
                    Some("left") => (0.0, -6.0),
                    Some("right") => (0.0, 6.0),
                    _ => return Err(format!("bad wheel: {rest}")),
                };
                let group = if group == usize::MAX { 0 } else { group };
                events.push(Step::Event(Event::Scroll { group, rows, cols }));
            }
            "click" | "shift-click" | "alt-click" | "ctrl-click" | "double-click"
            | "triple-click" | "press" | "box-press" | "double-press" | "drag" | "release"
            | "move" | "right-click" | "rest" => {
                let mut args = rest.split_whitespace();
                let kind = match verb {
                    "drag" => MouseKind::Drag,
                    "release" => MouseKind::Up,
                    "move" => MouseKind::Move,
                    "right-click" => MouseKind::Context,
                    "rest" => MouseKind::Rest,
                    _ => MouseKind::Down,
                };
                let mut number = || -> Result<f32, String> {
                    args.next()
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(|| format!("bad position: {action}"))
                };
                let (col, row) = (number()?, number()?);
                // A box press is Shift+Alt, to drag a box selection.
                let shift = matches!(verb, "shift-click" | "box-press");
                let alt = matches!(verb, "alt-click" | "box-press");
                let clicks = match verb {
                    "double-click" | "double-press" => 2,
                    "triple-click" => 3,
                    _ => 1,
                };
                let at = |kind, clicks| {
                    Step::Event(Event::Pointer(Pointer {
                        group,
                        kind,
                        row,
                        col,
                        shift,
                        alt,
                        ctrl: verb == "ctrl-click",
                        clicks,
                    }))
                };
                events.push(at(kind, clicks));
                if verb.ends_with("click") {
                    events.push(at(MouseKind::Up, 1));
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
                Effect::ReadFiles(paths) => {
                    let files = paths
                        .into_iter()
                        .map(|p| {
                            let text = self.files.get(&p.display().to_string()).cloned();
                            (p, text)
                        })
                        .collect();
                    self.send(Event::FilesRead(files));
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
                | Effect::OpenUrl(_)
                | Effect::Terminal(_)
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
                | Effect::File(_)
                | Effect::GitStatus
                | Effect::OpenDiff(_)
                | Effect::OpenMove { .. }
                | Effect::Git(_)
                | Effect::Blame { .. }
                | Effect::CommitInfo(_)
                | Effect::OpenCommit { .. }
                | Effect::PickFile(_)
                | Effect::OpenFolder(_)
                | Effect::SetSelection(_)
                | Effect::ReadSelection => {}
            }
        }
    }

    pub fn text(&self) -> String {
        self.editor.buffer().text()
    }

    /// What the window shows, as text: see [`Editor::dump`].
    pub fn screen(&self) -> String {
        self.editor.dump()
    }

    /// What's at column `x` and row `y` of the focused group's grid, the
    /// gutter's columns first: the character, its style, colour and any
    /// diagnostic under it. Past a line's end, or off the text, `None`.
    pub fn cell(&self, x: u16, y: u16) -> Option<Cell> {
        let view = self.editor.text_views().pop()?;
        let y = y as i32;
        let line = view
            .lines
            .iter()
            .find(|l| (l.row..l.row + l.shown as i32).contains(&y))?;
        if (x as usize) < view.gutter {
            let gutter: Vec<char> = line
                .sign
                .map(|(c, _)| c)
                .into_iter()
                .chain(line.number.chars())
                .collect();
            let style = match line.sign {
                Some((_, sign)) if x == 0 => sign,
                _ => line.number_style,
            };
            return Some(Cell {
                ch: *gutter.get(x as usize)?,
                style,
                syntax: crate::Syntax::Plain,
                mark: None,
                surface: crate::style::Surface::Editor,
                tint: None,
            });
        }
        let r = (y - line.row) as usize;
        let row = line.rows[r];
        let hscroll = if view.wrapping { 0 } else { view.hscroll };
        let target = (x as usize - view.gutter + hscroll + row.col).checked_sub(row.indent)?;
        let end = line
            .rows
            .get(r + 1)
            .map_or(line.chars.len(), |next| next.start);
        let mut d = row.col;
        for i in row.start..end {
            let w = crate::doc::advance(line.chars[i], d);
            if (d..d + w.max(1)).contains(&target) {
                return Some(Cell {
                    ch: line.chars[i],
                    style: line.styles[i],
                    syntax: line.syntax[i],
                    mark: line.marks[i],
                    surface: crate::style::Surface::Editor,
                    tint: line.brackets[i],
                });
            }
            d += w;
        }
        None
    }

    /// Where `text` first shows in the focused group's text: its column
    /// from the group's left edge and its row, as `click` takes them.
    pub fn find(&self, text: &str) -> Option<(u16, u16)> {
        let view = self.editor.text_views().pop()?;
        let want: Vec<char> = text.chars().collect();
        for line in &view.lines {
            for (r, row) in line.rows.iter().enumerate() {
                let y = line.row + r as i32;
                if !(0..view.area.h as i32).contains(&y) {
                    continue;
                }
                let end = line.rows.get(r + 1).map_or(line.chars.len(), |n| n.start);
                let chars = &line.chars[row.start..end];
                let Some(at) = chars.windows(want.len()).position(|w| w == want.as_slice()) else {
                    continue;
                };
                let mut d = row.col;
                for &c in &line.chars[row.start..row.start + at] {
                    d += crate::doc::advance(c, d);
                }
                let hscroll = if view.wrapping { 0 } else { view.hscroll };
                let x = (view.gutter + row.indent + d - row.col).checked_sub(hscroll)?;
                return Some((x as u16, y as u16));
            }
        }
        None
    }

    /// Clicks the entry, item or button in a side panel whose label holds
    /// `label`, as a window does: `clicks` 2 for a double click.
    pub fn click_entry(&mut self, label: &str, clicks: u8) {
        let Some((side, index)) = self.entry(label) else {
            panic!("no {label:?} in a side panel:\n{}", self.screen());
        };
        self.send(Event::Ui(crate::UiEvent::Target {
            side,
            index,
            clicks,
        }));
    }

    /// The side and target of the entry, item or button in a side panel
    /// whose label holds `label`.
    pub fn entry(&self, label: &str) -> Option<(crate::Side, usize)> {
        let view = self.editor.window_view();
        let sides = [
            (crate::Side::Left, &view.left),
            (crate::Side::Right, &view.right),
            (crate::Side::Bottom, &view.bottom),
        ];
        for (side, sv) in sides {
            let Some(sv) = sv else { continue };
            for (i, node) in sv.nodes.iter().enumerate() {
                // Each label, by how far it is after the node's first target.
                let labels: Vec<String> = match node {
                    crate::panel::Node::Entry(e) => vec![e.label.clone()],
                    crate::panel::Node::Item { label, .. } => vec![label.clone()],
                    crate::panel::Node::Button(b) => vec![b.label.clone()],
                    crate::panel::Node::Buttons(bs) => bs.iter().map(|b| b.label.clone()).collect(),
                    _ => continue,
                };
                let Some(k) = labels.iter().position(|l| l.contains(label)) else {
                    continue;
                };
                if let Some(first) = sv.targets.get(i).copied().flatten() {
                    return Some((side, first + k));
                }
            }
        }
        None
    }

    /// Where the primary cursor is drawn, in the focused group's grid.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        let view = self.editor.text_views().pop()?;
        let (x, y) = view.carets.first()?.cell?;
        Some((x + view.gutter as u16, y))
    }
}
