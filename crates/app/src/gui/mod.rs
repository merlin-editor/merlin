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
mod code;
mod cpu;
mod fonts;
mod frame;
mod gpu;
mod grid;
mod mouse;
mod overview;
mod paint;
mod scene;
mod settings;
mod termview;

use std::env;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use merlin_core::editor::Edge;
use merlin_core::panel::Node;
use merlin_core::{Event, Key, KeyCode, MouseKind, Pointer, Side, TextView, UiEvent, WindowView};
use merlin_plugins::PluginHost;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::CursorIcon;
use winit::window::{Window, WindowId};

use crate::App;
use crate::packs::{self, Look};
use crate::prefs::{self, Category, SettingsAction, SettingsView};
use crate::settings::Blinking;
use crate::terminal;
use canvas::{Canvas, Pictures, Rect, Text};
use frame::{Areas, Frame, Hit, State};
use gpu::Gpu;
use grid::Fonts;
use scene::{Bitmaps, Scene};

/// How often open files are checked for changes on disk, and plugins for
/// rebuilds.
/// A tab or a file of a side's view held down: its name and icon, where
/// the press was, and whether it has moved far enough to be a drag.
struct TabDrag {
    name: String,
    icon: Option<(char, merlin_core::style::Hue)>,
    from: (f32, f32),
    moved: bool,
    /// A file's element in a side's view, and the clicks pressed on it,
    /// which go to it on release unless it was dragged.
    entry: Option<(Side, usize, u8)>,
    /// A plugin view's icon or tab: the view, and the click it is unless
    /// it's dragged somewhere else.
    view: Option<(String, UiEvent)>,
}

/// Where a dragged tab or file would land.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    /// Before a tab of a group's strip, or at its end.
    Strip { group: usize, at: Option<usize> },
    /// On a group's code: on a side of it, to split, or in its middle,
    /// shading `zone`.
    Body {
        group: usize,
        edge: Option<Edge>,
        zone: Rect,
    },
    /// A plugin view's new place, shading where it goes.
    View {
        place: merlin_core::Place,
        zone: Rect,
    },
}

/// How far a tab moves before it's dragged rather than clicked.
const DRAG_START: f32 = 5.0;

/// How long the mouse rests on a word before its hover shows, as in VS
/// Code.
const HOVER_DELAY: Duration = Duration::from_millis(500);
/// How long the mouse rests on an icon before its tooltip shows.
const TIP_DELAY: Duration = Duration::from_millis(600);
/// How long the pointer rests on a tab before the open hand shows.
const GRAB_DELAY: Duration = Duration::from_millis(400);
/// The caret's blink, each way, and how long after the last input it
/// keeps blinking before it stays on and the window sleeps.
const BLINK: Duration = Duration::from_millis(530);
const BLINK_FOR: Duration = Duration::from_secs(10);
/// A smooth blink's fade, out or in, as a part of a blink.
const FADE: f32 = 0.35;
/// How often a fading caret is drawn again.
const FADE_FRAME: Duration = Duration::from_millis(33);
/// How long a scroll takes to glide to where it's going.
const GLIDE: Duration = Duration::from_millis(110);

pub fn run(app: &mut App) -> Result<()> {
    let event_loop = EventLoop::<()>::with_user_event()
        .build()
        .context("opening a window")?;
    let _ = PROXY.set(Mutex::new(event_loop.create_proxy()));
    // Plugins' processes may have answered before there was a loop to
    // wake, as a language server started for the file opened first does.
    let _ = event_loop.create_proxy().send_event(());
    let mut gui = Gui {
        app,
        window: None,
        text: None,
        grid: None,
        modifiers: ModifiersState::empty(),
        pointer: None,
        pressed_group: None,
        dragged_to: None,
        rest: None,
        over_cell: None,
        tip_drawn: None,
        tips: Vec::new(),
        tip: None,
        marks: Vec::new(),
        pointer_link: false,
        busy: false,
        grab: None,
        zoomed: (0.0, 1.0),
        sizes: frame::Sizes::default(),
        sash: None,
        group_drag: None,
        bar_drag: None,
        ime_spot: None,
        bars_until: Vec::new(),
        bars_drawn: Vec::new(),
        viewports: Vec::new(),
        resized_to: (0, 0),
        tab_drag: None,
        drop: None,
        strips: Vec::new(),
        cursor_icon: CursorIcon::Default,
        blink_from: Instant::now(),
        caret_drawn: 1.0,
        focused: true,
        glide: None,
        last_top: Vec::new(),
        context_at: None,
        clicks: Clicks::default(),
        scroll: 0.0,
        touched: false,
        state: State::default(),
        hits: Vec::new(),
        areas: Areas::default(),
        drawn: None,
        drawn_text: Vec::new(),
        term_drawn: 0,
        term_panes: Vec::new(),
        term_drag: None,
        pictures: Pictures::default(),
        bitmaps: Bitmaps::default(),
        scene: Scene::default(),
        fonts: None,
        settings_drawn: None,
        settings_due: None,
        hover_card: None,
    };
    event_loop.run_app(&mut gui).context("running the window")?;
    Ok(())
}

/// The window's event loop, once it's open. EventLoopProxy is Send but not
/// Sync, so it sits behind a lock.
static PROXY: OnceLock<Mutex<EventLoopProxy<()>>> = OnceLock::new();

/// Wakes the window from another thread, as when a job is done.
pub fn waker() -> crate::jobs::Waker {
    std::sync::Arc::new(|| {
        if let Some(proxy) = PROXY.get()
            && let Ok(proxy) = proxy.lock()
        {
            let _ = proxy.send_event(());
        }
    })
}

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
    /// The group whose code the left button went down in, so moves are
    /// drags there; and the cell last dragged to.
    pressed_group: Option<usize>,
    dragged_to: Option<(i32, i32)>,
    /// When the mouse, resting on the text, asks for a hover, and where.
    rest: Option<(Instant, (f32, f32))>,
    /// The cell of the text the mouse was last over, by group.
    over_cell: Option<(usize, i32, i32)>,
    /// Where the right-click menu was opened, in logical pixels.
    context_at: Option<(f32, f32)>,
    /// Each icon's tooltip, as last drawn.
    tips: Vec<(Rect, String)>,
    /// The tooltip the mouse rests for: when it shows, and what.
    tip: Option<(Instant, (Rect, String))>,
    /// The tooltip last drawn.
    tip_drawn: Option<(Rect, String)>,
    /// The hits the pointer shows differently, as last drawn.
    marks: Vec<(usize, frame::Mark)>,
    /// From the last view: whether the pointer is on a link, and whether
    /// something is on its way.
    pointer_link: bool,
    busy: bool,
    /// The tab the pointer rests on, by its hit, and when the open hand
    /// shows over it.
    grab: Option<(usize, Instant)>,
    /// The zoom the fonts were made for.
    zoomed: (f32, f32),
    /// How big the sides are, as last dragged.
    sizes: frame::Sizes,
    /// The side whose edge is being dragged.
    sash: Option<Side>,
    /// A scrollbar thumb or the minimap's slider held.
    bar_drag: Option<BarDrag>,
    /// Where the input method was last told the cursor is.
    ime_spot: Option<(i32, i32)>,
    /// Until when each group's scrollbar thumbs show before they fade.
    bars_until: Vec<(usize, Instant)>,
    /// How each group's bars were last drawn.
    bars_drawn: Vec<(usize, overview::Shown)>,
    /// The edge between groups being dragged: which, where it began, and
    /// the groups' sizes and places then.
    group_drag: Option<(frame::GroupSash, (f32, f32), Vec<frame::GroupArea>)>,
    /// Each group's text area, in columns and rows, as last told the
    /// editor; and the whole editor area's.
    viewports: Vec<(u16, u16)>,
    resized_to: (u16, u16),
    /// A tab pressed on, and whether it's being dragged yet.
    tab_drag: Option<TabDrag>,
    /// Where it would land now.
    drop: Option<Landing>,
    /// Each group's tab strip's scroll, as the frame last kept it.
    strips: Vec<(f32, usize)>,
    /// The mouse pointer's shape now.
    cursor_icon: CursorIcon,
    /// The last input, which restarts the caret's blink.
    blink_from: Instant,
    /// How far on the caret was drawn, for redrawing when it blinks.
    caret_drawn: f32,
    /// Whether the window has the keyboard; the caret blinks only then.
    focused: bool,
    /// A scroll gliding: the group, when it began and how far, in physical
    /// pixels, its lines started from where they are now.
    glide: Option<(usize, Instant, f32)>,
    /// Each group's top row as last drawn, to tell how far a scroll went.
    last_top: Vec<(usize, f64)>,
    clicks: Clicks,
    /// Touchpad scrolling off the text, short of a wheel step, in physical
    /// pixels.
    scroll: f64,
    /// The touchpad scrolled since the last frame: it moves the text
    /// exactly, so no glide.
    touched: bool,
    state: State,
    /// What each clickable part of the last frame does; later ones are on
    /// top.
    hits: Vec<(Rect, Hit)>,
    areas: Areas,
    /// What was last drawn, so a redraw that changes nothing costs nothing.
    #[allow(clippy::type_complexity)]
    drawn: Option<(WindowView, Option<usize>, [f32; 4], (u32, u32), Look)>,
    /// The text last drawn, beside `drawn`.
    drawn_text: Vec<TextView>,
    /// The terminals' version last drawn, and where each pane was.
    term_drawn: u64,
    term_panes: Vec<(u64, canvas::Rect)>,
    /// The terminal the left button went down in, to select in.
    term_drag: Option<u64>,
    pictures: Pictures,
    /// The glyphs and pictures drawn so far, as scenes name them.
    bitmaps: Bitmaps,
    /// The last frame's drawing, kept for its room.
    scene: Scene,
    /// The fonts the text and code were set up with, and the code's line
    /// height and ligatures, to set them up again when packs or settings
    /// change them.
    fonts: Option<(packs::Fonts, (Option<f32>, bool))>,
    /// The Settings tab as last drawn.
    settings_drawn: Option<SettingsView>,
    /// When the Plugins page reads programs' memory again.
    settings_due: Option<Instant>,
    /// The theme card the pointer is on, which the window previews.
    hover_card: Option<String>,
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
    /// Physical pixels to a point: the display's scale, times the
    /// window's own text size over its usual 13.
    fn scale(&self) -> f32 {
        let display = self
            .window
            .as_ref()
            .map_or(1.0, |s| s.window.scale_factor() as f32);
        display * self.ui_zoom()
    }

    /// How much bigger the frame is drawn than usual, for `ui_font_size`.
    fn ui_zoom(&self) -> f32 {
        self.app.window.ui_font_size.map_or(1.0, |n| n / 13.0)
    }

    fn dispatch(&mut self, event: Event) {
        let resting = matches!(
            event,
            Event::Pointer(Pointer {
                kind: MouseKind::Rest | MouseKind::Move,
                ..
            })
        );
        if !resting {
            self.blink_from = Instant::now();
        }
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
        // The settings' code font over the packs'.
        let w = &self.app.window;
        let wanted = packs::Fonts {
            code: w.font_family.clone().or(self.app.look.fonts.code.clone()),
            code_size: w.font_size.or(self.app.look.fonts.code_size),
            ..self.app.look.fonts.clone()
        };
        let shape = (w.line_height, w.font_ligatures);
        let zoom = (self.app.zoom, self.ui_zoom());
        if !now && self.fonts.as_ref() == Some(&(wanted.clone(), shape)) && self.zoomed == zoom {
            return;
        }
        self.zoomed = zoom;
        let scale = self.scale();
        self.text = Some(Text::new(
            scale,
            wanted.ui.as_deref(),
            wanted.code.as_deref(),
        ));
        let size = (wanted.code_size.unwrap_or(grid::FONT_SIZE) + zoom.0).max(6.0);
        // The frame's zoom leaves the code as big as it says.
        self.grid = Some(Fonts::new(
            scale / zoom.1,
            wanted.code.as_deref(),
            Some(size),
            shape.0,
            shape.1,
        ));
        self.fonts = Some((wanted, shape));
        // The old fonts' glyphs go with them.
        self.bitmaps.clear();
        self.drawn = None;
    }

    /// Lays the frame out for the window's size and what shows, and tells
    /// the editor how many cells the code has room for.
    fn fit_text(&mut self) {
        self.set_fonts(false);
        self.sizes.scrollbar = self.app.window.scrollbar_size;
        self.sizes.minimap_scale = self.app.window.minimap_scale;
        let (Some(shown), Some(grid)) = (&self.window, &self.grid) else {
            return;
        };
        let size = shown.window.inner_size();
        let s = self.scale();
        let (w, h) = (size.width as f32 / s, size.height as f32 / s);
        let view = self.app.editor.window_view();
        self.areas = frame::areas(&view, w, h, self.app.look.layout.gap, &self.sizes);
        let (cw, ch) = (grid.cell_w, grid.cell_h);
        let cells = |r: Rect| {
            (
                ((r.w * s) as u32 / cw).clamp(1, u16::MAX as u32) as u16,
                ((r.h * s) as u32 / ch).clamp(1, u16::MAX as u32) as u16,
            )
        };
        let viewports: Vec<(u16, u16)> = self.areas.groups.iter().map(|g| cells(g.text)).collect();
        let whole = cells(self.areas.editor);
        if whole != self.resized_to {
            self.resized_to = whole;
            self.app.dispatch(Event::Resize(whole.0, whole.1));
        }
        if viewports != self.viewports {
            self.viewports = viewports.clone();
            self.app.dispatch(Event::Viewports(viewports));
        }
    }

    /// Group `g`'s text as the editor has it now: its last view, the new
    /// side of a review side by side.
    fn view_of(&self, g: usize) -> Option<TextView> {
        self.app
            .editor
            .text_views()
            .into_iter()
            .rfind(|v| v.group == g)
    }

    /// The code's cell width in logical pixels.
    fn cell_w(&self) -> f32 {
        self.grid.as_ref().map_or(8.0, |g| g.cell_w as f32) / self.scale()
    }

    fn focused_group(&self) -> usize {
        self.app
            .editor
            .window_view()
            .groups
            .iter()
            .position(|g| g.focused)
            .unwrap_or(0)
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
                self.send_key(Key {
                    code: KeyCode::Char(c),
                    ctrl: false,
                    alt: false,
                    shift: false,
                });
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
        self.send_key(Key {
            code,
            ctrl,
            alt,
            shift,
        });
    }

    /// A key to the Settings tab while it has the keyboard and wants it,
    /// else to the editor.
    fn send_key(&mut self, key: Key) {
        if self.app.editor.page_focused() == Some(merlin_core::SETTINGS_PAGE)
            && let Some(events) = self.app.settings_key(key)
        {
            self.blink_from = Instant::now();
            for event in events {
                self.app.dispatch(event);
            }
            self.fit_text();
            self.redraw();
            return;
        }
        self.dispatch(Event::Key(key));
    }

    /// A click on the Settings tab gives it the keyboard.
    fn focus_page(&mut self, at: (f32, f32)) {
        if self.app.editor.page_focused().is_some() {
            return;
        }
        let Some(g) = self.group_at(at) else {
            return;
        };
        let view = self.app.editor.window_view();
        if let Some(index) = view
            .groups
            .get(g)
            .and_then(|g| g.tabs.iter().position(|t| t.active))
        {
            self.dispatch(Event::Ui(UiEvent::GroupTab {
                group: g,
                index,
                clicks: 1,
            }));
        }
    }

    fn tick(&mut self) {
        if self.app.tick() {
            self.fit_text();
            self.redraw();
        }
    }
}

impl ApplicationHandler for Gui<'_> {
    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.app.terminals.kill_all();
        // The clipboards go before the window they share.
        self.app.clipboard = None;
    }

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
        // The desktop's clipboards share the window's connection, and an
        // input method composes text into it.
        self.app.clipboard = crate::clipboard::Clipboard::for_window(&*window);
        window.set_ime_allowed(true);
        self.window = Some(Shown { window, backend });
        self.set_fonts(true);
        self.resized(size);
    }

    fn new_events(&mut self, _: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            if let Some((when, at)) = self.rest
                && Instant::now() >= when
            {
                self.rest = None;
                if let Some(g) = self.group_at(at) {
                    self.pointer_event(g, MouseKind::Rest, at, 1);
                }
            }
            if self
                .tip
                .as_ref()
                .is_some_and(|(when, _)| *when <= Instant::now())
            {
                self.redraw();
            }
            if self.grab.is_some_and(|(_, when)| when <= Instant::now()) {
                self.update_pointer();
            }
            if self.caret_level() != self.caret_drawn || self.app.terminals.poll_busy() {
                self.redraw();
            }
            if self.settings_due.is_some_and(|when| when <= Instant::now()) {
                self.settings_due = None;
                self.redraw();
            }
            self.tick();
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, (): ()) {
        if self.app.wake() {
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
            WindowEvent::RedrawRequested => {
                self.paint();
                // What's under the pointer may have changed with the view.
                self.update_pointer();
            }
            // Files dropped from the desktop open, as in any editor.
            WindowEvent::DroppedFile(path) => {
                self.app.open_dropped(&path);
                self.fit_text();
                self.redraw();
            }
            WindowEvent::Focused(on) => {
                self.focused = on;
                self.app.window_focused = on;
                // The desktop may have gone light or dark meanwhile.
                if on {
                    self.app.check_system_theme();
                }
                if !on {
                    self.dispatch(Event::AutoSave(merlin_core::AutoSaveWhy::Window));
                }
                self.blink_from = Instant::now();
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => {
                let ctrl = self.modifiers.control_key();
                self.modifiers = m.state();
                // Ctrl over a name makes it a link, without moving.
                if ctrl != self.modifiers.control_key()
                    && let Some(at) = self.pointer
                    && let Some((_, Hit::Text)) = self.hit_at(at)
                    && let Some(g) = self.group_at(at)
                {
                    self.pointer_event(g, MouseKind::Move, at, 1);
                    // Pressed, a rest shows the definition; let go, the
                    // link just goes.
                    if self.modifiers.control_key() {
                        self.rest = Some((Instant::now() + HOVER_DELAY, at));
                    }
                }
            }
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
                } else if let Some(id) = self.term_drag.take() {
                    // What's selected goes to the selection clipboard, as
                    // in any terminal.
                    if let (Some(text), Some(clipboard)) =
                        (self.app.terminals.selected(id), &mut self.app.clipboard)
                    {
                        clipboard.select(&text);
                    }
                } else if self.sash.take().is_some()
                    || self.group_drag.take().is_some()
                    || self.bar_drag.take().is_some()
                {
                    self.redraw();
                } else if let Some(drag) = self.tab_drag.take() {
                    let landing = self.drop.take();
                    if let Some((name, click)) = drag.view {
                        match landing {
                            Some(Landing::View { place, .. }) if drag.moved => {
                                self.dispatch(Event::Ui(UiEvent::MoveView { name, place }))
                            }
                            _ if !drag.moved => self.dispatch(Event::Ui(click)),
                            _ => {}
                        }
                    } else if drag.moved {
                        self.drop_tab(at, drag.entry);
                    } else if let Some((side, index, clicks)) = drag.entry {
                        // Not dragged: a click, which opens it.
                        self.dispatch(Event::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks,
                        }));
                    }
                    self.set_pointer(CursorIcon::Default);
                    self.redraw();
                } else if let Some(g) = self.pressed_group.take() {
                    self.dragged_to = None;
                    self.pointer_event(g, MouseKind::Up, at, 1);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                let Some(at) = self.pointer else { return };
                self.rest = None;
                match self.hit_at(at).map(|(_, h)| h.clone()) {
                    Some(Hit::Text) => {
                        if let Some(g) = self.group_at(at) {
                            self.context_at = Some(at);
                            self.pointer_event(g, MouseKind::Context, at, 1);
                        }
                    }
                    // An Explorer row's menu, or the menu for its empty space.
                    Some(Hit::Ui(UiEvent::Target { side, index, .. })) => {
                        self.context_at = Some(at);
                        self.dispatch(Event::Ui(UiEvent::TargetMenu { side, index }));
                    }
                    Some(Hit::Side(side)) => {
                        self.context_at = Some(at);
                        self.dispatch(Event::Ui(UiEvent::SideMenu(side)));
                    }
                    // A plugin's menu, from its row in Settings.
                    Some(Hit::Settings(crate::prefs::SettingsAction::Plugin(name))) => {
                        let menu = crate::prefs::SettingsAction::PluginMenu(Some(name));
                        self.settings_press(menu, 1, at);
                    }
                    // A plugin view's menu, from its rail icon or its tab.
                    Some(Hit::Ui(UiEvent::Rail(i))) => {
                        self.context_at = Some(at);
                        self.dispatch(Event::Ui(UiEvent::RailMenu(i)));
                    }
                    Some(Hit::Ui(UiEvent::PanelTitle { name, .. })) => {
                        self.context_at = Some(at);
                        self.dispatch(Event::Ui(UiEvent::Press(format!("view-menu:{name}"))));
                    }
                    // A tab's menu is about that tab, so it's chosen first.
                    Some(Hit::Ui(tab @ (UiEvent::Tab { .. } | UiEvent::GroupTab { .. }))) => {
                        self.dispatch(Event::Ui(tab));
                        self.context_at = Some(at);
                        self.dispatch(Event::Ui(UiEvent::TabMenu));
                    }
                    _ => {}
                }
            }
            // The middle button closes a tab, as in every editor.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Middle,
                ..
            } => {
                let Some(at) = self.pointer else { return };
                match self.hit_at(at).map(|(_, h)| h.clone()) {
                    Some(Hit::Ui(UiEvent::Tab { index, .. })) => {
                        self.dispatch(Event::Ui(UiEvent::CloseTab(index)))
                    }
                    Some(Hit::Ui(UiEvent::GroupTab { group, index, .. })) => {
                        self.dispatch(Event::Ui(UiEvent::CloseGroupTab { group, index }))
                    }
                    // In the text, it pastes what's selected anywhere.
                    Some(Hit::Text) => {
                        if let Some(g) = self.group_at(at) {
                            self.pointer_event(g, MouseKind::Middle, at, 1);
                        }
                    }
                    _ => {}
                }
            }
            // An input method's text: shown at the cursor while it's
            // composed, then typed.
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => {
                self.dispatch(Event::Preedit {
                    text,
                    cursor: cursor.map(|(at, _)| at),
                });
            }
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.dispatch(Event::Preedit {
                    text: String::new(),
                    cursor: None,
                });
                for c in text.chars() {
                    self.send_key(Key::plain(KeyCode::Char(c)));
                }
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.dispatch(Event::Preedit {
                    text: String::new(),
                    cursor: None,
                });
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                // A high-resolution wheel sends parts of a step: those
                // scroll smoothly, as a touchpad does.
                MouseScrollDelta::LineDelta(x, y) if y.fract() != 0.0 && x == 0.0 => {
                    let cell_h = self.grid.as_ref().map_or(20, |g| g.cell_h) as f64;
                    self.touch_scroll(0.0, y as f64 * 3.0 * cell_h);
                }
                MouseScrollDelta::LineDelta(x, y) => {
                    if y != 0.0 {
                        self.wheel(y as f64, false);
                    } else if x != 0.0 {
                        self.wheel(x as f64, true);
                    }
                }
                MouseScrollDelta::PixelDelta(p) => self.touch_scroll(p.x, p.y),
            },
            _ => {}
        }
        if self.app.quit {
            // Saves still being written finish first.
            self.app.finish_all();
            // The clipboards go before the window they share.
            self.app.clipboard = None;
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let tip = self
            .tip
            .as_ref()
            .map(|(when, _)| *when)
            .filter(|when| *when > Instant::now());
        // Nothing to wait for: sleep until something happens.
        let grab = self
            .grab
            .map(|(_, when)| when)
            .filter(|when| *when > Instant::now());
        let wake = [
            self.rest.map(|(when, _)| when),
            grab,
            self.next_blink(),
            tip,
            self.app.due(),
            self.bars_due(),
            self.settings_due,
        ]
        .into_iter()
        .flatten()
        .min();
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
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
}

/// Which of a group's bars is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bar {
    Down,
    Along,
    Map,
}

/// A bar held: its group, which, and how far into the thumb it was
/// grabbed, so the thumb stays under the mouse.
#[derive(Clone, Copy, Debug)]
struct BarDrag {
    group: usize,
    what: Bar,
    grab: f32,
}

/// How long the thumbs show after the mouse leaves or the text stops, and
/// how long they take to fade.
const BARS_LINGER: Duration = Duration::from_millis(1000);
const BARS_FADE: Duration = Duration::from_millis(300);

/// The groups' sizes after the edge `sash` is dragged by `by` logical
/// pixels from where it was, with the groups where `areas` had them: the
/// two either side of it share the room they had, neither under a few
/// columns.
fn group_sizes(
    sash: frame::GroupSash,
    areas: &[frame::GroupArea],
    by: (f32, f32),
) -> Vec<(f32, f32)> {
    const LEAST: f32 = 120.0;
    let whole = |a: &frame::GroupArea| {
        let top = a.tabs.y;
        (a.text.w, a.text.bottom() - top, a.text.x, top)
    };
    let mut sizes: Vec<(f32, f32)> = areas.iter().map(|a| (whole(a).0, whole(a).1)).collect();
    match sash {
        frame::GroupSash::Column(c) => {
            // The first group of each column speaks for it.
            let mut heads: Vec<usize> = Vec::new();
            for (g, a) in areas.iter().enumerate() {
                if heads
                    .last()
                    .is_none_or(|&h| whole(&areas[h]).2 != whole(a).2)
                {
                    heads.push(g);
                }
            }
            if let (Some(&l), Some(&r)) = (heads.get(c), heads.get(c + 1)) {
                let room = sizes[l].0 + sizes[r].0;
                let left = (sizes[l].0 + by.0).clamp(LEAST, (room - LEAST).max(LEAST));
                for (g, a) in areas.iter().enumerate() {
                    if whole(a).2 == whole(&areas[l]).2 {
                        sizes[g].0 = left;
                    } else if whole(a).2 == whole(&areas[r]).2 {
                        sizes[g].0 = room - left;
                    }
                }
            }
        }
        frame::GroupSash::Row(g) => {
            if g + 1 < sizes.len() {
                let room = sizes[g].1 + sizes[g + 1].1;
                let top = (sizes[g].1 + by.1).clamp(LEAST, (room - LEAST).max(LEAST));
                sizes[g].1 = top;
                sizes[g + 1].1 = room - top;
            }
        }
    }
    sizes
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
