//! The window. It draws its own frame in pixels, from the editor's window
//! view, and the code from the editor's screen of cells inside the editor
//! island. Clicks on the frame go back as UI events; the mouse in the code
//! goes back as cells.
//!
//! Each frame is recorded as a [`scene::Scene`] and drawn by the GPU
//! (`gpu.rs`), or by the CPU (`cpu.rs`) when there's no adapter or
//! `MERLIN_RENDERER=cpu` asks.
//!
//! The window sleeps until something happens: a key, the mouse, output
//! from a plugin's process, or the half-second check for changed files.

mod canvas;
mod cpu;
mod frame;
mod gpu;
mod grid;
mod scene;

use std::env;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use merlin_core::screen::Cell;
use merlin_core::{Event, Key, KeyCode, Mouse, MouseKind, Side, UiEvent, WindowView};
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
use crate::packs::{self, Look};
use canvas::{Canvas, Pictures, Rect, Text};
use frame::{Areas, Frame, Hit, State};
use gpu::Gpu;
use grid::Fonts;
use scene::{Bitmaps, Scene};

/// How often open files are checked for changes on disk, and plugins for
/// rebuilds.
const DISK_CHECK: Duration = Duration::from_millis(500);

pub fn run(app: &mut App) -> Result<()> {
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .context("opening a window")?;
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    app.editor.set_windowed(true);
    let mut gui = Gui {
        app,
        window: None,
        text: None,
        grid: None,
        modifiers: ModifiersState::empty(),
        pointer: None,
        pressed_in_text: false,
        clicks: Clicks::default(),
        scroll: 0.0,
        state: State::default(),
        hits: Vec::new(),
        areas: Areas::default(),
        drawn: None,
        next_check: Instant::now() + DISK_CHECK,
        pictures: Pictures::default(),
        bitmaps: Bitmaps::default(),
        scene: Scene::default(),
        fonts: None,
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
    text: Option<Text>,
    grid: Option<Fonts>,
    modifiers: ModifiersState,
    /// Where the mouse is, in logical pixels.
    pointer: Option<(f32, f32)>,
    /// The left button went down in the code, so moves are drags there.
    pressed_in_text: bool,
    clicks: Clicks,
    /// Touchpad scrolling not yet turned into wheel steps, in pixels.
    scroll: f64,
    state: State,
    /// What each clickable part of the last frame does; later ones are on
    /// top.
    hits: Vec<(Rect, Hit)>,
    areas: Areas,
    /// What was last drawn, so a redraw that changes nothing costs nothing.
    #[allow(clippy::type_complexity)]
    drawn: Option<(
        WindowView,
        Vec<Cell>,
        Option<(u16, u16)>,
        Option<usize>,
        [f32; 3],
        (u32, u32),
        Look,
    )>,
    next_check: Instant,
    pictures: Pictures,
    /// The glyphs and pictures drawn so far, as scenes name them.
    bitmaps: Bitmaps,
    /// The last frame's drawing, kept for its room.
    scene: Scene,
    /// The fonts the text and code were set up with, to set them up again
    /// when packs change them.
    fonts: Option<packs::Fonts>,
}

struct Shown {
    window: Arc<Window>,
    backend: Backend,
}

enum Backend {
    Gpu(Box<Gpu>),
    Cpu(Surface<Arc<Window>, Arc<Window>>),
}

impl Backend {
    /// The GPU's, unless `MERLIN_RENDERER=cpu` asks for the CPU's or
    /// there's no adapter; the CPU's otherwise.
    fn new(window: &Arc<Window>) -> Result<Self, String> {
        if env::var("MERLIN_RENDERER").as_deref() != Ok("cpu") {
            match Gpu::new(window.clone()) {
                Ok(gpu) => return Ok(Self::Gpu(Box::new(gpu))),
                Err(err) => eprintln!("merlin: drawing on the CPU: {err}"),
            }
        }
        Context::new(window.clone())
            .and_then(|context| Surface::new(&context, window.clone()))
            .map(Self::Cpu)
            .map_err(|e| e.to_string())
    }
}

impl Gui<'_> {
    fn scale(&self) -> f32 {
        self.window
            .as_ref()
            .map_or(1.0, |s| s.window.scale_factor() as f32)
    }

    fn dispatch(&mut self, event: Event) {
        self.app.dispatch(event);
        self.fit_text();
        self.redraw();
    }

    fn redraw(&self) {
        if let Some(shown) = &self.window {
            shown.window.request_redraw();
        }
    }

    /// Sets up the fonts for the display's scale and what the packs ask
    /// for, when they've changed or `now`.
    fn set_fonts(&mut self, now: bool) {
        let wanted = &self.app.look.fonts;
        if !now && self.fonts.as_ref() == Some(wanted) {
            return;
        }
        let scale = self.scale();
        self.text = Some(Text::new(
            scale,
            wanted.ui.as_deref(),
            wanted.code.as_deref(),
        ));
        self.grid = Some(Fonts::new(scale, wanted.code.as_deref(), wanted.code_size));
        self.fonts = Some(wanted.clone());
        // The old fonts' glyphs go with them.
        self.bitmaps.clear();
        self.drawn = None;
    }

    /// Lays the frame out for the window's size and what shows, and tells
    /// the editor how many cells the code has room for.
    fn fit_text(&mut self) {
        self.set_fonts(false);
        let (Some(shown), Some(grid)) = (&self.window, &self.grid) else {
            return;
        };
        let size = shown.window.inner_size();
        let s = self.scale();
        let (w, h) = (size.width as f32 / s, size.height as f32 / s);
        let view = self.app.editor.window_view();
        self.areas = frame::areas(&view, w, h, self.app.look.layout.gap);
        let t = self.areas.text;
        let cols = ((t.w * s) as u32 / grid.cell_w).clamp(1, u16::MAX as u32) as u16;
        let rows = ((t.h * s) as u32 / grid.cell_h).clamp(1, u16::MAX as u32) as u16;
        if self.app.editor.render().width != cols || self.app.editor.render().height != rows {
            self.app.dispatch(Event::Resize(cols, rows));
        }
    }

    /// The topmost clickable part under the mouse.
    fn hit_at(&self, (x, y): (f32, f32)) -> Option<(usize, &Hit)> {
        self.hits
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (r, _))| r.contains(x, y))
            .map(|(i, (_, hit))| (i, hit))
    }

    /// The code's cell under a point, clamped to the code.
    fn cell_at(&self, (x, y): (f32, f32)) -> (u16, u16) {
        let Some(grid) = &self.grid else {
            return (0, 0);
        };
        let s = self.scale();
        let t = self.areas.text;
        let cx = (((x - t.x).max(0.0) * s) as u32 / grid.cell_w).min(u16::MAX as u32) as u16;
        let cy = (((y - t.y).max(0.0) * s) as u32 / grid.cell_h).min(u16::MAX as u32) as u16;
        (cx, cy)
    }

    fn text_mouse(&mut self, kind: MouseKind, at: (f32, f32), clicks: u8) {
        let (x, y) = self.cell_at(at);
        self.dispatch(Event::Mouse(Mouse {
            kind,
            x,
            y,
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            clicks,
        }));
    }

    fn press(&mut self, at: (f32, f32)) {
        let clicks = self.clicks.count(at);
        let Some((_, hit)) = self.hit_at(at) else {
            return;
        };
        match hit.clone() {
            Hit::Text => {
                self.pressed_in_text = true;
                self.text_mouse(MouseKind::Down, at, clicks);
            }
            Hit::Ui(UiEvent::Tab { index, .. }) => {
                self.dispatch(Event::Ui(UiEvent::Tab { index, clicks }))
            }
            Hit::Ui(event) => self.dispatch(Event::Ui(event)),
            Hit::Command(command) => self.dispatch(Event::Command(command.into())),
            Hit::Key(key) => self.dispatch(Event::Key(key)),
            Hit::Side(_) | Hit::Block => {}
        }
    }

    fn wheel(&mut self, lines: f64, sideways: bool) {
        let Some(at) = self.pointer else {
            return;
        };
        let over = self.hit_at(at).map(|(_, h)| h.clone());
        let side = match over {
            Some(Hit::Side(side)) => Some(side),
            Some(Hit::Ui(UiEvent::Target { side, .. })) => Some(side),
            _ => None,
        };
        // A side's content scrolls in pixels, three rows a step.
        if let Some(side) = side
            && self.app.editor.window_view().palette.is_none()
        {
            let k = match side {
                Side::Left => 0,
                Side::Right => 1,
                Side::Bottom => 2,
            };
            self.state.scroll[k] -= lines as f32 * 78.0;
            self.redraw();
            return;
        }
        let shift = self.modifiers.shift_key();
        let kind = match (sideways || shift, lines > 0.0) {
            (false, true) => MouseKind::ScrollUp,
            (false, false) => MouseKind::ScrollDown,
            (true, true) => MouseKind::ScrollLeft,
            (true, false) => MouseKind::ScrollRight,
        };
        for _ in 0..(lines.abs().round() as u32).max(1) {
            self.text_mouse(kind, at, 1);
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
        let s = self.scale();
        let (Some(shown), Some(text), Some(grid)) =
            (&mut self.window, &mut self.text, &mut self.grid)
        else {
            return;
        };
        let size = shown.window.inner_size();
        let (pw, ph) = (size.width as usize, size.height as usize);
        if pw == 0 || ph == 0 {
            return;
        }
        let mut screen = self.app.editor.render();
        // Code blocks ask for their colours while drawn; draw again with them.
        if self.app.serve_colours() {
            screen = self.app.editor.render();
        }
        let view = self.app.editor.window_view();
        let hovered = self.pointer.and_then(|p| {
            self.hits
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (r, _))| r.contains(p.0, p.1))
                .map(|(i, _)| i)
        });
        let cells: Vec<Cell> = (0..screen.height)
            .flat_map(|y| (0..screen.width).map(move |x| (x, y)))
            .map(|(x, y)| screen.cell(x, y))
            .collect();
        let drawn = Some((
            view.clone(),
            cells,
            screen.cursor,
            hovered,
            self.state.scroll,
            (size.width, size.height),
            self.app.look.clone(),
        ));
        if self.drawn == drawn {
            return;
        }
        // Packs changed: their pictures may have too. Too many bitmaps:
        // start again.
        if self.drawn.as_ref().map(|d| &d.6) != Some(&self.app.look)
            || self.bitmaps.bytes() > Bitmaps::LIMIT
        {
            self.bitmaps.clear();
        }
        let (w, h) = (pw as f32 / s, ph as f32 / s);
        let areas = frame::areas(&view, w, h, self.app.look.layout.gap);
        self.scene.clear();
        let mut f = Frame {
            canvas: Canvas {
                scene: &mut self.scene,
                bitmaps: &mut self.bitmaps,
                width: pw,
                height: ph,
                scale: s,
            },
            text,
            pointer: self.pointer,
            hits: Vec::new(),
            radius: self.app.look.layout.radius,
            p: self.app.look.palette,
            icons: &self.app.look.icons,
            pictures: &mut self.pictures,
            compact: self.app.look.layout.compact,
        };
        f.base(&view, &areas, &mut self.state, w, h);
        let t = areas.text;
        grid.draw(
            &screen,
            &mut f.canvas,
            ((t.x * s) as usize, (t.y * s) as usize),
            &self.app.look.palette,
        );
        f.overlays(&view, &areas, w, h);
        self.hits = std::mem::take(&mut f.hits);
        let shown_ok = match &mut shown.backend {
            Backend::Gpu(gpu) => gpu.draw(&self.scene, &self.bitmaps, &shown.window),
            Backend::Cpu(surface) => match surface.buffer_mut() {
                Ok(mut buffer) if buffer.len() == pw * ph => {
                    cpu::draw(&self.scene, &self.bitmaps, &mut buffer, pw, ph);
                    shown.window.pre_present_notify();
                    buffer.present().is_ok()
                }
                _ => false,
            },
        };
        self.areas = areas;
        if !shown_ok {
            // Not shown: draw again when asked.
            self.drawn = None;
            return;
        }
        // The hover index belongs to the hits just made.
        let hovered = self.pointer.and_then(|p| {
            self.hits
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (r, _))| r.contains(p.0, p.1))
                .map(|(i, _)| i)
        });
        self.drawn = drawn.map(|d| (d.0, d.1, d.2, hovered, d.4, d.5, d.6));
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
            self.fit_text();
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
            .with_inner_size(LogicalSize::new(1440.0, 900.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(err) => {
                eprintln!("merlin: can't open a window: {err}");
                event_loop.exit();
                return;
            }
        };
        let backend = match Backend::new(&window) {
            Ok(b) => b,
            Err(err) => {
                eprintln!("merlin: can't draw in the window: {err}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.window = Some(Shown { window, backend });
        self.set_fonts(true);
        self.resized(size);
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.tick();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, (): ()) {
        if self.app.poll_processes() {
            self.fit_text();
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
                let _ = scale_factor;
                self.set_fonts(true);
                if let Some(shown) = &self.window {
                    let size = shown.window.inner_size();
                    self.resized(size);
                }
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } => self.key(event),
            WindowEvent::CursorMoved { position, .. } => self.moved(position),
            WindowEvent::CursorLeft { .. } => {
                self.pointer = None;
                self.redraw();
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let Some(at) = self.pointer else {
                    return;
                };
                if state == ElementState::Pressed {
                    self.press(at);
                } else if self.pressed_in_text {
                    self.pressed_in_text = false;
                    self.text_mouse(MouseKind::Up, at, 1);
                }
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
                    let step = self.grid.as_ref().map_or(60, |g| g.cell_h * 3) as f64;
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

impl Gui<'_> {
    fn resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(shown) = &mut self.window
            && let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            match &mut shown.backend {
                Backend::Gpu(gpu) => gpu.resize(w.get(), h.get()),
                Backend::Cpu(surface) => {
                    let _ = surface.resize(w, h);
                }
            }
        }
        self.drawn = None;
        self.fit_text();
        self.redraw();
    }

    fn moved(&mut self, position: PhysicalPosition<f64>) {
        let s = self.scale() as f64;
        let at = ((position.x / s) as f32, (position.y / s) as f32);
        let before = self.pointer.and_then(|p| self.hit_at(p).map(|(i, _)| i));
        let old_cell = self.pointer.map(|p| self.cell_at(p));
        self.pointer = Some(at);
        if self.pressed_in_text {
            if old_cell != Some(self.cell_at(at)) {
                self.text_mouse(MouseKind::Drag, at, 1);
            }
            return;
        }
        let over = self.hit_at(at).map(|(i, h)| (i, h.clone()));
        if let Some((_, Hit::Text)) = over {
            // The editor lights up what's under the mouse in the code.
            if old_cell != Some(self.cell_at(at)) {
                self.text_mouse(MouseKind::Move, at, 1);
            }
            return;
        }
        if before != over.map(|(i, _)| i) {
            self.redraw();
        }
    }
}

/// Counts presses in the same place in quick succession, so the editor can
/// tell a double or triple click: a fourth starts again at one.
#[derive(Default)]
struct Clicks {
    last: Option<(Instant, (f32, f32), u8)>,
}

impl Clicks {
    const GAP: Duration = Duration::from_millis(400);

    fn count(&mut self, at: (f32, f32)) -> u8 {
        let now = Instant::now();
        let near = |a: (f32, f32)| (a.0 - at.0).abs() < 4.0 && (a.1 - at.1).abs() < 4.0;
        let clicks = match self.last {
            Some((when, where_, n)) if now - when < Self::GAP && near(where_) => n % 3 + 1,
            _ => 1,
        };
        self.last = Some((now, at, clicks));
        clicks
    }
}
