//! The window: the editor's screen drawn with real fonts into a software
//! buffer, and the window's keys and mouse turned into editor events.
//!
//! The screen is still the core's grid of cells, as the terminal showed
//! it. The window sleeps until something happens: a key, the mouse, output
//! from a plugin's process, or the half-second check for changed files.

use std::collections::HashMap;
use std::env;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache,
    SwashContent, Weight,
};
use merlin_core::screen::{Screen, WIDE_TAIL};
use merlin_core::{Event, Key, KeyCode, Mouse, MouseKind};
use merlin_plugins::PluginHost;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{Window, WindowId};

use crate::App;
use crate::theme;

/// How often open files are checked for changes on disk, and plugins for
/// rebuilds.
const DISK_CHECK: Duration = Duration::from_millis(500);
/// Points, before the display's scale.
const FONT_SIZE: f32 = 14.0;
/// Rows are this many times the font size.
const LINE_HEIGHT: f32 = 1.35;
const BACKGROUND: u32 = 0x1f1f1f;
const TEXT: u32 = 0xcccccc;
const CURSOR: u32 = 0xaeafad;
/// Families tried in order; the first one installed is used. The Nerd
/// Fonts carry the icons the Explorer and tabs show.
const FAMILIES: &[&str] = &[
    "JetBrainsMono Nerd Font",
    "JetBrainsMono NF",
    "JetBrains Mono",
    "FiraCode Nerd Font",
    "DejaVu Sans Mono",
];

pub fn run(app: &mut App) -> Result<()> {
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .context("opening a window")?;
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    let mut gui = Gui {
        app,
        window: None,
        fonts: None,
        modifiers: ModifiersState::empty(),
        pointer: None,
        pressed: false,
        clicks: Clicks::default(),
        scroll: 0.0,
        frame: Vec::new(),
        next_check: Instant::now() + DISK_CHECK,
    };
    event_loop.run_app(&mut gui).context("running the window")?;
    Ok(())
}

/// The window's event loop, once it's open. EventLoopProxy is Send but not
/// Sync, so it sits behind a lock.
static PROXY: OnceLock<Mutex<EventLoopProxy<()>>> = OnceLock::new();

/// Lets plugins' processes wake the window when they have output. Called
/// before plugins load, as they keep the waker they were loaded with.
pub fn set_waker(host: &mut PluginHost) {
    host.set_waker(|| {
        if let Some(proxy) = PROXY.get()
            && let Ok(proxy) = proxy.lock()
        {
            let _ = proxy.send_event(());
        }
    });
}

struct Gui<'a> {
    app: &'a mut App,
    window: Option<Shown>,
    fonts: Option<Fonts>,
    modifiers: ModifiersState,
    /// The cell under the mouse.
    pointer: Option<(u16, u16)>,
    pressed: bool,
    clicks: Clicks,
    /// Touchpad scrolling not yet turned into wheel steps, in pixels.
    scroll: f64,
    /// The pixels last drawn, kept so a redraw that changes nothing is free.
    frame: Vec<u32>,
    next_check: Instant,
}

struct Shown {
    window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
}

impl Gui<'_> {
    fn dispatch(&mut self, event: Event) {
        self.app.dispatch(event);
        self.redraw();
    }

    fn redraw(&self) {
        if let Some(shown) = &self.window {
            shown.window.request_redraw();
        }
    }

    /// The grid that fits the window, in cells.
    fn grid(&self, size: PhysicalSize<u32>) -> (u16, u16) {
        let Some(fonts) = &self.fonts else {
            return (80, 24);
        };
        (
            (size.width / fonts.cell_w).clamp(1, u16::MAX as u32) as u16,
            (size.height / fonts.cell_h).clamp(1, u16::MAX as u32) as u16,
        )
    }

    fn resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(shown) = &mut self.window
            && let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            let _ = shown.surface.resize(w, h);
        }
        self.frame.clear();
        let (w, h) = self.grid(size);
        self.dispatch(Event::Resize(w, h));
    }

    fn cell_at(&self, at: PhysicalPosition<f64>) -> (u16, u16) {
        let Some(fonts) = &self.fonts else {
            return (0, 0);
        };
        let x = (at.x.max(0.0) as u32 / fonts.cell_w).min(u16::MAX as u32) as u16;
        let y = (at.y.max(0.0) as u32 / fonts.cell_h).min(u16::MAX as u32) as u16;
        (x, y)
    }

    fn mouse(&mut self, kind: MouseKind) {
        let (x, y) = self.pointer.unwrap_or((0, 0));
        let mut mouse = Mouse {
            kind,
            x,
            y,
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            clicks: 1,
        };
        self.clicks.count(&mut mouse);
        self.dispatch(Event::Mouse(mouse));
    }

    fn wheel(&mut self, lines: f64, sideways: bool) {
        let shift = self.modifiers.shift_key();
        let kind = match (sideways || shift, lines > 0.0) {
            (false, true) => MouseKind::ScrollUp,
            (false, false) => MouseKind::ScrollDown,
            (true, true) => MouseKind::ScrollLeft,
            (true, false) => MouseKind::ScrollRight,
        };
        for _ in 0..(lines.abs().round() as u32).max(1) {
            self.mouse(kind);
        }
    }

    fn key(&mut self, event: KeyEvent) {
        if event.state != ElementState::Pressed {
            return;
        }
        let m = self.modifiers;
        let (ctrl, alt) = (m.control_key(), m.alt_key());
        // Typed text, unless Ctrl or Alt makes it a shortcut.
        if !ctrl
            && !alt
            && let Some(text) = &event.text
            && text.chars().all(|c| !c.is_control())
        {
            for c in text.chars() {
                self.dispatch(Event::Key(Key {
                    code: KeyCode::Char(c),
                    ctrl: false,
                    alt: false,
                    shift: false,
                }));
            }
            return;
        }
        let code = match event.key_without_modifiers() {
            WinitKey::Character(s) => match s.chars().next() {
                Some(c) => KeyCode::Char(c.to_ascii_lowercase()),
                None => return,
            },
            WinitKey::Named(named) => match named {
                NamedKey::Enter => KeyCode::Enter,
                NamedKey::Backspace => KeyCode::Backspace,
                NamedKey::Delete => KeyCode::Delete,
                NamedKey::ArrowLeft => KeyCode::Left,
                NamedKey::ArrowRight => KeyCode::Right,
                NamedKey::ArrowUp => KeyCode::Up,
                NamedKey::ArrowDown => KeyCode::Down,
                NamedKey::Home => KeyCode::Home,
                NamedKey::End => KeyCode::End,
                NamedKey::PageUp => KeyCode::PageUp,
                NamedKey::PageDown => KeyCode::PageDown,
                NamedKey::Tab if m.shift_key() => KeyCode::BackTab,
                NamedKey::Tab => KeyCode::Tab,
                NamedKey::Escape => KeyCode::Esc,
                NamedKey::Space => KeyCode::Char(' '),
                NamedKey::F1 => KeyCode::F(1),
                NamedKey::F2 => KeyCode::F(2),
                NamedKey::F3 => KeyCode::F(3),
                NamedKey::F4 => KeyCode::F(4),
                NamedKey::F5 => KeyCode::F(5),
                NamedKey::F6 => KeyCode::F(6),
                NamedKey::F7 => KeyCode::F(7),
                NamedKey::F8 => KeyCode::F(8),
                NamedKey::F9 => KeyCode::F(9),
                NamedKey::F10 => KeyCode::F(10),
                NamedKey::F11 => KeyCode::F(11),
                NamedKey::F12 => KeyCode::F(12),
                _ => return,
            },
            _ => return,
        };
        // The core reports Shift only where it isn't in the character, as
        // with Ctrl+Shift+P.
        let shift = m.shift_key() && (ctrl || !matches!(code, KeyCode::Char(_)));
        let shift = shift && code != KeyCode::BackTab;
        self.dispatch(Event::Key(Key {
            code,
            ctrl,
            alt,
            shift,
        }));
    }

    fn paint(&mut self) {
        let (Some(shown), Some(fonts)) = (&mut self.window, &mut self.fonts) else {
            return;
        };
        let size = shown.window.inner_size();
        let (w, h) = (size.width as usize, size.height as usize);
        if w == 0 || h == 0 {
            return;
        }
        let screen = self.app.editor.render();
        let mut frame = vec![BACKGROUND; w * h];
        fonts.draw(&screen, &mut frame, w, h);
        if frame == self.frame {
            return;
        }
        let Ok(mut buffer) = shown.surface.buffer_mut() else {
            return;
        };
        if buffer.len() == frame.len() {
            buffer.copy_from_slice(&frame);
            shown.window.pre_present_notify();
            let _ = buffer.present();
            self.frame = frame;
        }
    }

    fn tick(&mut self) {
        let mut changed = self.app.poll_processes();
        if Instant::now() >= self.next_check {
            self.app.reload_plugins();
            self.app.check_disk();
            self.next_check = Instant::now() + DISK_CHECK;
            changed = true;
        }
        if changed {
            self.redraw();
        }
    }
}

impl ApplicationHandler for Gui<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let project = self
            .app
            .root
            .file_name()
            .map(|n| format!("{} — merlin", n.to_string_lossy()))
            .unwrap_or_else(|| "merlin".into());
        let attributes = Window::default_attributes()
            .with_title(project)
            .with_inner_size(LogicalSize::new(1280.0, 800.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Rc::new(w),
            Err(err) => {
                eprintln!("merlin: can't open a window: {err}");
                event_loop.exit();
                return;
            }
        };
        let surface =
            Context::new(window.clone()).and_then(|context| Surface::new(&context, window.clone()));
        let surface = match surface {
            Ok(s) => s,
            Err(err) => {
                eprintln!("merlin: can't draw in the window: {err}");
                event_loop.exit();
                return;
            }
        };
        self.fonts = Some(Fonts::new(window.scale_factor() as f32));
        let size = window.inner_size();
        self.window = Some(Shown { window, surface });
        self.resized(size);
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.tick();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, (): ()) {
        if self.app.poll_processes() {
            self.redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                // As File › Exit: unsaved files ask first.
                self.dispatch(Event::Command("app.quit".into()));
            }
            WindowEvent::Resized(size) => self.resized(size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.fonts = Some(Fonts::new(scale_factor as f32));
                if let Some(shown) = &self.window {
                    let size = shown.window.inner_size();
                    self.resized(size);
                }
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => self.key(event),
            WindowEvent::CursorMoved { position, .. } => {
                let cell = self.cell_at(position);
                if self.pointer != Some(cell) {
                    self.pointer = Some(cell);
                    let kind = if self.pressed {
                        MouseKind::Drag
                    } else {
                        MouseKind::Move
                    };
                    self.mouse(kind);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.pressed = state == ElementState::Pressed;
                let kind = if self.pressed {
                    MouseKind::Down
                } else {
                    MouseKind::Up
                };
                self.mouse(kind);
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                MouseScrollDelta::LineDelta(x, y) => {
                    if y != 0.0 {
                        self.wheel(y as f64, false);
                    } else if x != 0.0 {
                        self.wheel(x as f64, true);
                    }
                }
                MouseScrollDelta::PixelDelta(p) => {
                    // A wheel step scrolls three rows.
                    let step = self.fonts.as_ref().map_or(60, |f| f.cell_h * 3) as f64;
                    self.scroll += p.y;
                    let steps = (self.scroll / step).trunc();
                    if steps != 0.0 {
                        self.scroll -= steps * step;
                        self.wheel(steps, false);
                    }
                }
            },
            _ => {}
        }
        if self.app.quit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_check));
    }
}

/// Counts presses in the same cell in quick succession, so the editor can
/// tell a double or triple click: a fourth starts again at one.
#[derive(Default)]
struct Clicks {
    last: Option<(Instant, u16, u16, u8)>,
}

impl Clicks {
    const GAP: Duration = Duration::from_millis(400);

    fn count(&mut self, m: &mut Mouse) {
        if m.kind != MouseKind::Down {
            return;
        }
        let now = Instant::now();
        let clicks = match self.last {
            Some((at, x, y, n)) if now - at < Self::GAP && (x, y) == (m.x, m.y) => n % 3 + 1,
            _ => 1,
        };
        self.last = Some((now, m.x, m.y, clicks));
        m.clicks = clicks;
    }
}

/// A character's picture, rasterised once and kept.
struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    /// Colour glyphs (emoji) are RGBA; the rest are coverage.
    colour: bool,
    data: Vec<u8>,
}

struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    family: Option<String>,
    size: f32,
    cell_w: u32,
    cell_h: u32,
    /// From the top of a row.
    baseline: i32,
    line: Buffer,
    glyphs: HashMap<(char, bool, bool), Option<Glyph>>,
}

impl Fonts {
    fn new(scale: f32) -> Self {
        let mut system = FontSystem::new();
        let size = env::var("MERLIN_FONT_SIZE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(FONT_SIZE)
            * scale;
        let wanted = env::var("MERLIN_FONT").ok();
        let installed = |name: &str| {
            system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == name))
        };
        let family = wanted
            .iter()
            .map(String::as_str)
            .chain(FAMILIES.iter().copied())
            .find(|name| installed(name))
            .map(str::to_string);
        let cell_h = (size * LINE_HEIGHT).round().max(1.0);
        let line = Buffer::new(&mut system, Metrics::new(size, cell_h));
        let mut fonts = Self {
            system,
            cache: SwashCache::new(),
            family,
            size,
            cell_w: 1,
            cell_h: cell_h as u32,
            baseline: 0,
            line,
            glyphs: HashMap::new(),
        };
        fonts.measure();
        fonts
    }
}

/// The face for a cell: the chosen family, bold or italic as it says.
fn attrs(family: &Option<String>, bold: bool, italic: bool) -> Attrs<'_> {
    let family = match family {
        Some(name) => Family::Name(name),
        None => Family::Monospace,
    };
    let mut attrs = Attrs::new().family(family);
    if bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

impl Fonts {
    /// The cell's width and the baseline, from the font's `M`.
    fn measure(&mut self) {
        let attrs = attrs(&self.family, false, false);
        let line = &mut self.line;
        line.set_text("M", &attrs, Shaping::Advanced, None);
        line.shape_until_scroll(&mut self.system, false);
        if let Some(run) = line.layout_runs().next() {
            if let Some(g) = run.glyphs.first() {
                self.cell_w = g.w.round().max(1.0) as u32;
            }
            self.baseline = run.line_y.round() as i32;
        }
    }

    fn glyph(&mut self, ch: char, bold: bool, italic: bool) -> Option<&Glyph> {
        if !self.glyphs.contains_key(&(ch, bold, italic)) {
            let glyph = self.rasterise(ch, bold, italic);
            self.glyphs.insert((ch, bold, italic), glyph);
        }
        self.glyphs.get(&(ch, bold, italic))?.as_ref()
    }

    fn rasterise(&mut self, ch: char, bold: bool, italic: bool) -> Option<Glyph> {
        let attrs = attrs(&self.family, bold, italic);
        let line = &mut self.line;
        line.set_text(&ch.to_string(), &attrs, Shaping::Advanced, None);
        line.shape_until_scroll(&mut self.system, false);
        let physical = line
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.first())
            .map(|g| g.physical((0.0, 0.0), 1.0));
        let physical = physical?;
        let image = self
            .cache
            .get_image_uncached(&mut self.system, physical.cache_key)?;
        Some(Glyph {
            left: image.placement.left + physical.x,
            top: image.placement.top - physical.y,
            width: image.placement.width as usize,
            height: image.placement.height as usize,
            colour: image.content == SwashContent::Color,
            data: match image.content {
                // Subpixel masks would need the panel's layout; their
                // average is plain coverage.
                SwashContent::SubpixelMask => image
                    .data
                    .chunks(4)
                    .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                    .collect(),
                _ => image.data,
            },
        })
    }

    fn draw(&mut self, screen: &Screen, frame: &mut [u32], w: usize, h: usize) {
        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let cells = |x: u16, y: u16| (x as usize * cw, y as usize * ch);
        // Backgrounds first, so a glyph wider than its cell isn't covered
        // by the next one's.
        for y in 0..screen.height {
            for x in 0..screen.width {
                let look = theme::dark_cell(&screen.cell(x, y));
                let (px, py) = cells(x, y);
                fill(frame, w, h, px, py, cw, ch, look.bg.unwrap_or(BACKGROUND));
            }
        }
        for y in 0..screen.height {
            for x in 0..screen.width {
                let cell = screen.cell(x, y);
                if cell.ch == ' ' || cell.ch == WIDE_TAIL {
                    continue;
                }
                let look = theme::dark_cell(&cell);
                let fg = look.fg.unwrap_or(TEXT);
                let (px, py) = cells(x, y);
                let wide = x + 1 < screen.width && screen.cell(x + 1, y).ch == WIDE_TAIL;
                let span = if wide { 2 * cw } else { cw };
                if box_drawing(cell.ch, frame, w, h, px, py, cw, ch, fg) {
                    continue;
                }
                // Room for an icon wider than its cell, when the next is blank.
                let room = if !wide && x + 1 < screen.width && screen.cell(x + 1, y).ch == ' ' {
                    2 * cw
                } else {
                    span
                };
                let baseline = self.baseline;
                let Some(glyph) = self.glyph(cell.ch, look.bold, look.italic) else {
                    continue;
                };
                // Centre glyphs that are narrower than a wide character's
                // two cells, as CJK and emoji are drawn.
                let offset = if wide && glyph.width < span {
                    ((span - glyph.width) / 2) as i32 - glyph.left.max(0)
                } else {
                    0
                };
                blit(
                    frame,
                    w,
                    (px, py, room, ch),
                    px as i32 + glyph.left + offset,
                    py as i32 + baseline - glyph.top,
                    glyph,
                    fg,
                );
            }
        }
        for y in 0..screen.height {
            for x in 0..screen.width {
                let look = theme::dark_cell(&screen.cell(x, y));
                if !look.underline {
                    continue;
                }
                let colour = look.underline_colour.or(look.fg).unwrap_or(TEXT);
                let (px, py) = cells(x, y);
                let under = py + (self.baseline as usize + 2).min(ch - 1);
                let thick = (self.size / 14.0).round().max(1.0) as usize;
                for dx in 0..cw {
                    // A wave for problems, two pixels up and down.
                    let lift = if look.curly {
                        let phase = ((px + dx) as f32 / (cw as f32 / 2.0)) * std::f32::consts::PI;
                        ((phase.sin() + 1.0) * thick as f32).round() as usize
                    } else {
                        0
                    };
                    let row = (under + 2 * thick).saturating_sub(lift);
                    fill(frame, w, h, px + dx, row, 1, thick, colour);
                }
            }
        }
        if let Some((x, y)) = screen.cursor {
            let (px, py) = cells(x, y);
            let beam = (self.size / 7.0).round().max(2.0) as usize;
            fill(frame, w, h, px, py, beam, ch, CURSOR);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill(frame: &mut [u32], w: usize, h: usize, x: usize, y: usize, fw: usize, fh: usize, c: u32) {
    for row in y.min(h)..(y + fh).min(h) {
        let start = row * w;
        frame[start + x.min(w)..start + (x + fw).min(w)].fill(c);
    }
}

/// Draws `glyph` with its origin at (`gx`, `gy`), inside `clip`.
fn blit(
    frame: &mut [u32],
    w: usize,
    clip: (usize, usize, usize, usize),
    gx: i32,
    gy: i32,
    glyph: &Glyph,
    fg: u32,
) {
    let h = frame.len() / w;
    let (cx, cy, cw, ch) = clip;
    let (x0, x1) = (cx as i32, ((cx + cw).min(w)) as i32);
    let (y0, y1) = (cy as i32, ((cy + ch).min(h)) as i32);
    for row in 0..glyph.height as i32 {
        let y = gy + row;
        if y < y0 || y >= y1 {
            continue;
        }
        for col in 0..glyph.width as i32 {
            let x = gx + col;
            if x < x0 || x >= x1 {
                continue;
            }
            let i = (row as usize) * glyph.width + col as usize;
            let at = y as usize * w + x as usize;
            frame[at] = if glyph.colour {
                let p = &glyph.data[i * 4..i * 4 + 4];
                let rgb = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                blend(frame[at], rgb, p[3])
            } else {
                blend(frame[at], fg, glyph.data[i])
            };
        }
    }
}

fn blend(bg: u32, fg: u32, alpha: u8) -> u32 {
    if alpha == 0 {
        return bg;
    }
    if alpha == 255 {
        return fg;
    }
    let a = alpha as u32;
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (f, b) = ((fg >> shift) & 0xff, (bg >> shift) & 0xff);
        out | (((f * a + b * (255 - a)) / 255) << shift)
    })
}

/// Lines and blocks that must meet the next cell's, drawn by hand: a font's
/// versions stop short when rows are taller than the font.
#[allow(clippy::too_many_arguments)]
fn box_drawing(
    c: char,
    frame: &mut [u32],
    w: usize,
    h: usize,
    x: usize,
    y: usize,
    cw: usize,
    ch: usize,
    fg: u32,
) -> bool {
    let t = (cw / 8).max(1);
    let (mx, my) = (x + cw / 2 - t / 2, y + ch / 2 - t / 2);
    // Which arms reach out from the middle: left, right, up, down.
    let arms = match c {
        '─' => (true, true, false, false),
        '│' => (false, false, true, true),
        '├' => (false, true, true, true),
        '┤' => (true, false, true, true),
        '┼' => (true, true, true, true),
        '┌' | '╭' => (false, true, false, true),
        '┐' | '╮' => (true, false, false, true),
        '└' | '╰' => (false, true, true, false),
        '┘' | '╯' => (true, false, true, false),
        '┬' => (true, true, false, true),
        '┴' => (true, true, true, false),
        '▌' => {
            fill(frame, w, h, x, y, cw / 2, ch, fg);
            return true;
        }
        '▐' => {
            fill(frame, w, h, x + cw / 2, y, cw - cw / 2, ch, fg);
            return true;
        }
        '▕' => {
            fill(frame, w, h, x + cw - cw / 8, y, (cw / 8).max(1), ch, fg);
            return true;
        }
        '█' => {
            fill(frame, w, h, x, y, cw, ch, fg);
            return true;
        }
        _ => return false,
    };
    let (left, right, up, down) = arms;
    if left {
        fill(frame, w, h, x, my, mx + t - x, t, fg);
    }
    if right {
        fill(frame, w, h, mx, my, x + cw - mx, t, fg);
    }
    if up {
        fill(frame, w, h, mx, y, t, my + t - y, fg);
    }
    if down {
        fill(frame, w, h, mx, my, t, y + ch - my, fg);
    }
    true
}
