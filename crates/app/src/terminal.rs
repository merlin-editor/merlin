//! The integrated terminal's shells: each on its own pty, read on its own
//! thread by alacritty's terminal, whose screen the window draws.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermDamage, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use merlin_core::{Event, Key, KeyCode};

use crate::jobs::Waker;

/// Lines kept above the screen.
const HISTORY: usize = 10_000;

/// Hands the terminal's events to the window's thread and wakes it.
#[derive(Clone)]
struct Proxy {
    id: u64,
    queue: Arc<Mutex<Vec<(u64, TermEvent)>>>,
    wake: Waker,
}

impl EventListener for Proxy {
    fn send_event(&self, event: TermEvent) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.push((self.id, event));
        }
        (self.wake)();
    }
}

/// A terminal's size in cells.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Size {
    cols: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Session {
    term: Arc<FairMutex<Term<Proxy>>>,
    tx: EventLoopSender,
    size: Size,
    /// A cell's size in pixels, for programs that ask.
    cell: (u16, u16),
    /// A task's command line, which the editor wants the output of.
    task: bool,
    /// The task's exit code, kept until all it printed is read.
    code: Option<i32>,
}

/// A shell or task waiting for its size to start.
struct Pending {
    root: PathBuf,
    command: Option<String>,
}

/// One cell of a terminal's screen, as the window draws it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    pub c: char,
    pub fg: u32,
    /// `None` for the panel's own background.
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// The first half of a double-width character.
    pub wide: bool,
}

/// A terminal's screen, copied out for drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub cols: usize,
    pub lines: usize,
    /// Row by row.
    pub cells: Vec<Cell>,
    /// Where the cursor is, and whether it's a bar, a block or an
    /// underline; `None` when it's hidden or scrolled away.
    pub cursor: Option<(usize, usize, CursorShape)>,
    /// Lines scrolled back from the bottom.
    pub scrolled: usize,
}

/// Colours a screen is drawn in: the 16 ANSI colours, then the text and
/// the background.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colours {
    pub ansi: [u32; 16],
    pub text: u32,
    pub background: u32,
}

impl Colours {
    /// ANSI colours that sit with merlin's palette.
    pub fn for_palette(text: u32, background: u32, light: bool) -> Self {
        let ansi = if light {
            [
                0x2e3035, 0xc0392b, 0x2e8b3a, 0x9a6b00, 0x2f5fc4, 0x8a4fb8, 0x16808c, 0x6b6e76,
                0x5f626b, 0xd9443a, 0x3a9e48, 0xb07d00, 0x3d6fe0, 0x9c5fcc, 0x1a95a3, 0x2e3035,
            ]
        } else {
            // The eight are mid-toned, so white or black text reads on
            // them, as prompts' segments put it; the bright eight are
            // merlin's own accents.
            [
                0x26272c, 0xd9534c, 0x4fa456, 0xc9962b, 0x4677d6, 0xa45fbf, 0x2b98ad, 0xd4d5da,
                0x6b6e76, 0xf0807a, 0x6cc070, 0xe3b341, 0x7c9cff, 0xc4a7e7, 0x7fc8d1, 0xeef0f3,
            ]
        };
        Self {
            ansi,
            text,
            background,
        }
    }

    fn indexed(&self, i: u8, custom: &alacritty_terminal::term::color::Colors) -> u32 {
        if let Some(rgb) = custom[i as usize] {
            return rgb_u32(rgb);
        }
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let i = i - 16;
                let step = |v: u8| if v == 0 { 0 } else { 55 + v as u32 * 40 };
                step(i / 36) << 16 | step(i / 6 % 6) << 8 | step(i % 6)
            }
            _ => {
                let v = 8 + (i as u32 - 232) * 10;
                v << 16 | v << 8 | v
            }
        }
    }
}

fn rgb_u32(c: Rgb) -> u32 {
    (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 0xff) as f32, ((b >> s) & 0xff) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Every open terminal.
pub struct Terminals {
    sessions: HashMap<u64, Session>,
    queue: Arc<Mutex<Vec<(u64, TermEvent)>>>,
    wake: Waker,
    /// Shells to start in a folder once their size is known.
    pending: HashMap<u64, Pending>,
    /// Shells that couldn't start, and why.
    failed: Vec<(u64, String)>,
    /// Goes up whenever a screen may have changed, for the window to know
    /// to draw again.
    pub version: u64,
    /// Until when to look for new output each frame. A shell printing
    /// without a pause says nothing until it stops, as alacritty reads
    /// all there is before it wakes the window, so the screen is looked
    /// at a frame at a time while output comes.
    busy: Option<Instant>,
}

/// How often a busy terminal is looked at: each frame, at 60 a second.
const FRAME: Duration = Duration::from_millis(16);

/// How long after its last output a terminal counts as busy.
const BUSY: Duration = Duration::from_millis(250);

impl Terminals {
    pub fn new(wake: Waker) -> Self {
        Self {
            sessions: HashMap::new(),
            queue: Arc::default(),
            wake,
            pending: HashMap::new(),
            failed: Vec::new(),
            version: 0,
            busy: None,
        }
    }

    /// Starts the user's shell in `root`, the project's folder, once its
    /// pane is first drawn: a shell started at a guessed size and then
    /// resized redraws its prompt over lines the terminal has rewrapped,
    /// which leaves bits of coloured prompts behind.
    pub fn spawn(&mut self, id: u64, root: &Path, command: Option<String>) {
        self.pending.insert(
            id,
            Pending {
                root: root.to_path_buf(),
                command,
            },
        );
    }

    fn start(
        &mut self,
        id: u64,
        job: &Pending,
        size: Size,
        cell: (u16, u16),
    ) -> Result<(), String> {
        let root = &job.root;
        // A task's command line, under the line that says what it is.
        let shell = job.command.as_ref().map(|command| {
            let script = "printf '\\033[2m❯ %s\\033[0m\\n' \"$1\"; eval \"$1\"";
            tty::Shell::new(
                "/bin/sh".into(),
                vec!["-c".into(), script.into(), "merlin".into(), command.clone()],
            )
        });
        let env = [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "merlin"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let options = tty::Options {
            shell,
            working_directory: Some(root.to_path_buf()),
            drain_on_exit: true,
            env,
            // Quote the shell's arguments for Windows' command line, as
            // Alacritty does by default.
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, window_size(size, cell), id).map_err(|e| e.to_string())?;
        let proxy = Proxy {
            id,
            queue: self.queue.clone(),
            wake: self.wake.clone(),
        };
        let config = Config {
            scrolling_history: HISTORY,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, proxy.clone())));
        let event_loop =
            EventLoop::new(term.clone(), proxy, pty, true, false).map_err(|e| e.to_string())?;
        let tx = event_loop.channel();
        event_loop.spawn();
        self.sessions.insert(
            id,
            Session {
                term,
                tx,
                size,
                cell,
                task: job.command.is_some(),
                code: None,
            },
        );
        Ok(())
    }

    pub fn kill(&mut self, id: u64) {
        self.pending.remove(&id);
        if let Some(s) = self.sessions.remove(&id) {
            let _ = s.tx.send(Msg::Shutdown);
        }
    }

    /// Shuts every shell down, as the editor quits.
    pub fn kill_all(&mut self) {
        let ids: Vec<u64> = self.sessions.keys().copied().collect();
        for id in ids {
            self.kill(id);
        }
    }

    /// Fits a terminal to `cols` by `lines` cells of `cell` pixels.
    pub fn resize(&mut self, id: u64, cols: usize, lines: usize, cell: (u16, u16)) {
        let size = Size {
            cols: cols.max(2),
            lines: lines.max(1),
        };
        // Tasks started out of view start at this size too: a task runs
        // whether its terminal shows or not.
        let tasks: Vec<u64> = self
            .pending
            .iter()
            .filter(|(i, p)| **i != id && p.command.is_some())
            .map(|(&i, _)| i)
            .collect();
        for task in tasks {
            if let Some(job) = self.pending.remove(&task)
                && let Err(err) = self.start(task, &job, size, cell)
            {
                self.failed.push((task, err));
            }
        }
        if let Some(job) = self.pending.remove(&id) {
            if let Err(err) = self.start(id, &job, size, cell) {
                self.failed.push((id, err));
            }
            self.version += 1;
            return;
        }
        let Some(s) = self.sessions.get_mut(&id) else {
            return;
        };
        if s.size == size {
            return;
        }
        s.size = size;
        s.cell = cell;
        let _ = s.tx.send(Msg::Resize(window_size(size, cell)));
        s.term.lock().resize(size);
        self.version += 1;
    }

    fn write(&self, id: u64, bytes: Vec<u8>) {
        if let Some(s) = self.sessions.get(&id) {
            // Typing goes back to the bottom, as in any terminal.
            s.term.lock().scroll_display(Scroll::Bottom);
            let _ = s.tx.send(Msg::Input(bytes.into()));
        }
    }

    pub fn key(&mut self, id: u64, key: Key) {
        let Some(s) = self.sessions.get(&id) else {
            return;
        };
        let mode = *s.term.lock().mode();
        if let Some(bytes) = encode(key, mode) {
            s.term.lock().selection = None;
            self.write(id, bytes);
            self.version += 1;
        }
    }

    pub fn paste(&mut self, id: u64, text: &str) {
        let Some(s) = self.sessions.get(&id) else {
            return;
        };
        let bracketed = s.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        let bytes = if bracketed {
            // A paste can't end the bracket early.
            let inner = text.replace("\x1b[201~", "");
            format!("\x1b[200~{inner}\x1b[201~").into_bytes()
        } else {
            text.into_bytes()
        };
        self.write(id, bytes);
    }

    /// Scrolls back (positive) or forward through the history; in a
    /// full-screen program, the wheel sends arrow keys instead, as other
    /// terminals do.
    pub fn scroll(&mut self, id: u64, lines: i32) {
        let Some(s) = self.sessions.get(&id) else {
            return;
        };
        let mode = *s.term.lock().mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let arrow = if lines > 0 { b'A' } else { b'B' };
            let lead = if mode.contains(TermMode::APP_CURSOR) {
                b'O'
            } else {
                b'['
            };
            let bytes: Vec<u8> = (0..lines.unsigned_abs())
                .flat_map(|_| [0x1b, lead, arrow])
                .collect();
            let _ = s.tx.send(Msg::Input(bytes.into()));
        } else {
            s.term.lock().scroll_display(Scroll::Delta(lines));
        }
        self.version += 1;
    }

    pub fn page(&mut self, id: u64, up: bool) {
        if let Some(s) = self.sessions.get(&id) {
            let page = if up { Scroll::PageUp } else { Scroll::PageDown };
            s.term.lock().scroll_display(page);
            self.version += 1;
        }
    }

    /// Starts, extends or ends a selection with the mouse at a cell of the
    /// screen as drawn.
    pub fn select(&mut self, id: u64, (col, line): (usize, usize), kind: SelectKind) {
        let Some(s) = self.sessions.get(&id) else {
            return;
        };
        let mut term = s.term.lock();
        let offset = term.grid().display_offset() as i32;
        let point = Point::new(Line(line as i32 - offset), Column(col));
        match kind {
            SelectKind::Start(clicks) => {
                let ty = match clicks {
                    2 => SelectionType::Semantic,
                    3 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                term.selection = Some(Selection::new(ty, point, Side::Left));
            }
            SelectKind::Extend => {
                if let Some(sel) = &mut term.selection {
                    sel.update(point, Side::Right);
                }
            }
        }
        drop(term);
        self.version += 1;
    }

    /// The selected text, if any.
    pub fn selected(&self, id: u64) -> Option<String> {
        let term = self.sessions.get(&id)?.term.lock();
        term.selection_to_string().filter(|t| !t.is_empty())
    }

    /// Whether the program in the terminal wants the mouse itself.
    pub fn wants_mouse(&self, id: u64) -> bool {
        self.sessions
            .get(&id)
            .is_some_and(|s| s.term.lock().mode().intersects(TermMode::MOUSE_MODE))
    }

    /// The events the shells sent since last asked, as the editor's, and
    /// whether any screen changed. Some are answered here: the shell asking
    /// for its size or colours, or to write back.
    pub fn take_events(
        &mut self,
        colours: &Colours,
        clipboard: &mut dyn FnMut(Option<String>) -> String,
    ) -> (Vec<Event>, bool) {
        let events = std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()));
        let mut out = Vec::new();
        for (id, err) in self.failed.drain(..) {
            out.push(Event::Error(format!("Can't start a shell: {err}")));
            out.push(Event::TermExited { id, code: None });
        }
        let mut changed = false;
        for (id, event) in events {
            match event {
                TermEvent::Wakeup => {
                    changed = true;
                    self.busy = Some(Instant::now() + BUSY);
                }
                TermEvent::MouseCursorDirty | TermEvent::CursorBlinkingChange => changed = true,
                TermEvent::Title(title) => out.push(Event::TermTitle { id, title }),
                TermEvent::ResetTitle => out.push(Event::TermTitle {
                    id,
                    title: "shell".into(),
                }),
                // A task's output is all read once the terminal exits,
                // after its child.
                TermEvent::ChildExit(status) => match self.sessions.get_mut(&id) {
                    Some(s) if s.task => s.code = status.code(),
                    _ => out.push(Event::TermExited {
                        id,
                        code: status.code(),
                    }),
                },
                TermEvent::Exit => {
                    if let Some(s) = self.sessions.get(&id).filter(|s| s.task) {
                        out.push(Event::TaskOutput {
                            id,
                            text: text_of(&s.term.lock()),
                        });
                        out.push(Event::TermExited { id, code: s.code });
                    }
                }
                TermEvent::Bell => {}
                TermEvent::PtyWrite(text) => self.write(id, text.into_bytes()),
                TermEvent::ClipboardStore(_, text) => {
                    clipboard(Some(text));
                }
                TermEvent::ClipboardLoad(_, reply) => {
                    let text = clipboard(None);
                    self.write(id, reply(&text).into_bytes());
                }
                TermEvent::ColorRequest(i, reply) => {
                    let rgb = match i {
                        256 => colours.text,
                        257 => colours.background,
                        _ => colours.ansi.get(i).copied().unwrap_or(colours.text),
                    };
                    let rgb = Rgb {
                        r: (rgb >> 16) as u8,
                        g: (rgb >> 8) as u8,
                        b: rgb as u8,
                    };
                    self.write(id, reply(rgb).into_bytes());
                }
                TermEvent::TextAreaSizeRequest(reply) => {
                    if let Some(s) = self.sessions.get(&id) {
                        let text = reply(window_size(s.size, s.cell));
                        self.write(id, text.into_bytes());
                    }
                }
            }
        }
        if changed || !out.is_empty() {
            self.version += 1;
        }
        (out, changed)
    }

    /// When to look at the screens next, while output comes.
    pub fn due(&self) -> Option<Instant> {
        self.busy.map(|_| Instant::now() + FRAME)
    }

    /// Looks for output the shells printed without saying, while they're
    /// busy. Returns whether a screen changed.
    pub fn poll_busy(&mut self) -> bool {
        let Some(until) = self.busy else {
            return false;
        };
        let mut changed = false;
        for s in self.sessions.values() {
            let mut term = s.term.lock();
            // alacritty always counts the cursor's cell as damaged, so
            // that alone isn't output: else a terminal is busy for good.
            let cursor = term.grid().cursor.point;
            let at = (cursor.line.0.max(0) as usize, cursor.column.0);
            changed |= match term.damage() {
                TermDamage::Full => true,
                TermDamage::Partial(lines) => is_output(lines, at),
            };
            term.reset_damage();
        }
        let now = Instant::now();
        if changed {
            self.version += 1;
            self.busy = Some(now + BUSY);
        } else if now >= until {
            self.busy = None;
        }
        changed
    }

    /// A terminal's screen as it shows now.
    pub fn screen(&self, id: u64, colours: &Colours) -> Option<Screen> {
        let s = self.sessions.get(&id)?;
        let term = s.term.lock();
        let content = term.renderable_content();
        let (cols, lines) = (s.size.cols, s.size.lines);
        let blank = Cell {
            c: ' ',
            fg: colours.text,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            wide: false,
        };
        let mut cells = vec![blank; cols * lines];
        let offset = content.display_offset as i32;
        let selection = content.selection;
        let custom = content.colors;
        let colour = |c: Color| -> Option<u32> {
            match c {
                Color::Spec(rgb) => Some(rgb_u32(rgb)),
                Color::Indexed(i) => Some(colours.indexed(i, custom)),
                Color::Named(n) => match n {
                    NamedColor::Foreground | NamedColor::BrightForeground => {
                        custom[n].map(rgb_u32).or(Some(colours.text))
                    }
                    NamedColor::Background => custom[n].map(rgb_u32),
                    NamedColor::DimForeground => Some(mix(colours.text, colours.background, 0.35)),
                    // Bold stays its colour, as kitty and Alacritty draw it,
                    // not the bright one, as xterm used to.
                    n if (n as usize) < 16 => Some(colours.indexed(n as u8, custom)),
                    _ => Some(colours.text),
                },
            }
        };
        for indexed in content.display_iter {
            let row = indexed.point.line.0 + offset;
            let col = indexed.point.column.0;
            if row < 0 || row as usize >= lines || col >= cols {
                continue;
            }
            let cell = indexed.cell;
            let flags = cell.flags;
            if flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let bold = flags.contains(Flags::BOLD);
            let mut fg = colour(cell.fg).unwrap_or(colours.text);
            let mut bg = colour(cell.bg);
            if flags.contains(Flags::INVERSE) {
                let back = bg.unwrap_or(colours.background);
                bg = Some(fg);
                fg = back;
            }
            if flags.contains(Flags::DIM) {
                fg = mix(fg, bg.unwrap_or(colours.background), 0.4);
            }
            if selection.is_some_and(|sel| sel.contains(indexed.point)) {
                bg = Some(mix(colours.background, 0x7c9cff, 0.35));
            }
            let c = if flags.contains(Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            cells[row as usize * cols + col] = Cell {
                c,
                fg,
                bg,
                bold,
                italic: flags.contains(Flags::ITALIC),
                underline: flags.intersects(Flags::ALL_UNDERLINES),
                strike: flags.contains(Flags::STRIKEOUT),
                wide: flags.contains(Flags::WIDE_CHAR),
            };
        }
        let cursor = content.cursor;
        let shown =
            cursor.shape != CursorShape::Hidden && term.mode().contains(TermMode::SHOW_CURSOR);
        let row = cursor.point.line.0 + offset;
        let cursor = (shown && row >= 0 && (row as usize) < lines).then_some((
            cursor.point.column.0,
            row as usize,
            cursor.shape,
        ));
        Some(Screen {
            cols,
            lines,
            cells,
            cursor,
            scrolled: content.display_offset,
        })
    }
}

/// All a terminal shows and keeps above, as plain text: lines that wrap
/// joined again.
fn text_of(term: &Term<Proxy>) -> String {
    let grid = term.grid();
    let cols = grid.columns();
    let mut out = String::new();
    for i in grid.topmost_line().0..=grid.bottommost_line().0 {
        let row = &grid[Line(i)];
        let mut line = String::new();
        for c in 0..cols {
            let cell = &row[Column(c)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                line.push(cell.c);
            }
        }
        if row[Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
            out.push_str(&line);
        } else {
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// Whether damage is more than the cursor's own cell, at (line, column),
/// which alacritty always counts.
fn is_output(
    mut lines: impl Iterator<Item = alacritty_terminal::term::LineDamageBounds>,
    at: (usize, usize),
) -> bool {
    lines.any(|d| (d.line, d.left, d.right) != (at.0, at.1, at.1))
}

/// How the mouse acts on a selection.
#[derive(Clone, Copy, Debug)]
pub enum SelectKind {
    /// Pressed, with the clicks so far: one for characters, two for words,
    /// three for lines.
    Start(u8),
    Extend,
}

fn window_size(size: Size, (cell_width, cell_height): (u16, u16)) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.cols as u16,
        cell_width,
        cell_height,
    }
}

/// The bytes a key sends to the shell, as xterm sends them.
pub fn encode(key: Key, mode: TermMode) -> Option<Vec<u8>> {
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    // xterm's modifier parameter: 1 plus shift 1, alt 2, ctrl 4.
    let modifier = 1 + key.shift as u8 + 2 * key.alt as u8 + 4 * key.ctrl as u8;
    let csi = |end: &str| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[1;{modifier}{end}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{end}").into_bytes()
        } else {
            format!("\x1b[{end}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if modifier > 1 {
            format!("\x1b[{n};{modifier}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let alt = |mut bytes: Vec<u8>| {
        if key.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    Some(match key.code {
        KeyCode::Char(c) if key.ctrl => {
            let byte = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                '@' | ' ' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '8' | '?' => 0x7f,
                _ => return None,
            };
            alt(vec![byte])
        }
        KeyCode::Char(c) => alt(c.to_string().into_bytes()),
        KeyCode::Enter => alt(vec![b'\r']),
        KeyCode::Backspace if key.ctrl => alt(vec![0x08]),
        KeyCode::Backspace => alt(vec![0x7f]),
        KeyCode::Tab => alt(vec![b'\t']),
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Esc => alt(vec![0x1b]),
        KeyCode::Up => csi("A"),
        KeyCode::Down => csi("B"),
        KeyCode::Right => csi("C"),
        KeyCode::Left => csi("D"),
        KeyCode::Home => csi("H"),
        KeyCode::End => csi("F"),
        KeyCode::Delete => tilde(3),
        KeyCode::PageUp => tilde(5),
        KeyCode::PageDown => tilde(6),
        KeyCode::F(n @ 1..=4) => {
            let end = ["P", "Q", "R", "S"][n as usize - 1];
            if modifier > 1 {
                format!("\x1b[1;{modifier}{end}").into_bytes()
            } else {
                format!("\x1bO{end}").into_bytes()
            }
        }
        KeyCode::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][n as usize - 5]),
        KeyCode::F(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursors_own_cell_isnt_output() {
        use alacritty_terminal::term::LineDamageBounds as D;
        let cursor = D {
            line: 3,
            left: 7,
            right: 7,
        };
        assert!(!is_output([cursor].into_iter(), (3, 7)));
        assert!(!is_output(std::iter::empty(), (3, 7)));
        let printed = D {
            line: 3,
            left: 0,
            right: 7,
        };
        assert!(is_output([cursor, printed].into_iter(), (3, 7)));
        // The cursor moved: its old cell is damaged too.
        assert!(is_output([cursor].into_iter(), (4, 0)));
    }

    fn key(code: KeyCode, ctrl: bool, alt: bool, shift: bool) -> Key {
        Key {
            code,
            ctrl,
            alt,
            shift,
        }
    }

    #[test]
    fn keys_encode_as_xterm_sends_them() {
        let none = TermMode::empty();
        let plain = |c| key(c, false, false, false);
        assert_eq!(encode(plain(KeyCode::Char('a')), none), Some(b"a".to_vec()));
        assert_eq!(
            encode(key(KeyCode::Char('c'), true, false, false), none),
            Some(vec![3])
        );
        assert_eq!(
            encode(key(KeyCode::Char('b'), false, true, false), none),
            Some(b"\x1bb".to_vec())
        );
        assert_eq!(encode(plain(KeyCode::Enter), none), Some(b"\r".to_vec()));
        assert_eq!(encode(plain(KeyCode::Backspace), none), Some(vec![0x7f]));
        assert_eq!(encode(plain(KeyCode::Up), none), Some(b"\x1b[A".to_vec()));
        assert_eq!(
            encode(plain(KeyCode::Up), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            encode(key(KeyCode::Right, true, false, false), none),
            Some(b"\x1b[1;5C".to_vec())
        );
        assert_eq!(
            encode(plain(KeyCode::Delete), none),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(encode(plain(KeyCode::F(1)), none), Some(b"\x1bOP".to_vec()));
        assert_eq!(
            encode(plain(KeyCode::F(5)), none),
            Some(b"\x1b[15~".to_vec())
        );
    }

    #[test]
    fn the_256_colours() {
        let c = Colours::for_palette(0xffffff, 0, false);
        let none = alacritty_terminal::term::color::Colors::default();
        assert_eq!(c.indexed(16, &none), 0x000000);
        assert_eq!(c.indexed(196, &none), 0xff0000);
        assert_eq!(c.indexed(232, &none), 0x080808);
        assert_eq!(c.indexed(1, &none), c.ansi[1]);
    }
}

#[cfg(test)]
mod replay {
    use super::*;

    /// Feeds the bytes in `MERLIN_REPLAY` to a terminal `MERLIN_COLS`
    /// wide and prints each row with its backgrounds, for looking at
    /// what a program drew.
    #[test]
    #[ignore = "a tool, run by hand"]
    fn replay() {
        let bytes = std::fs::read(std::env::var("MERLIN_REPLAY").unwrap()).unwrap();
        let cols: usize = std::env::var("MERLIN_COLS").unwrap().parse().unwrap();
        #[derive(Clone)]
        struct Quiet;
        impl EventListener for Quiet {}
        let size = Size { cols, lines: 8 };
        let mut term = Term::new(Config::default(), &size, Quiet);
        let mut parser: alacritty_terminal::vte::ansi::Processor = Default::default();
        parser.advance(&mut term, &bytes);
        for line in 0..8 {
            let row = &term.grid()[Line(line)];
            let text: String = (0..cols).map(|c| row[Column(c)].c).collect();
            let bg: String = (0..cols)
                .map(|c| match row[Column(c)].bg {
                    Color::Named(NamedColor::Background) => '.',
                    Color::Named(n) => char::from_digit(n as u32 % 16, 16).unwrap_or('?'),
                    _ => '#',
                })
                .collect();
            println!("{text}|\n{bg}|");
        }
    }
}
