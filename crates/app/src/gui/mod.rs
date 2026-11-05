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
mod frame;
mod gpu;
mod grid;
mod overview;
mod scene;
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
}

/// How far a tab moves before it's dragged rather than clicked.
const DRAG_START: f32 = 5.0;

/// How long the mouse rests on a word before its hover shows, as in VS
/// Code.
const HOVER_DELAY: Duration = Duration::from_millis(500);
/// How long the mouse rests on an icon before its tooltip shows.
const TIP_DELAY: Duration = Duration::from_millis(600);
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
    drawn: Option<(WindowView, Option<usize>, [f32; 3], (u32, u32), Look)>,
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

    /// A press on a scrollbar or the minimap: on the thumb it's held where
    /// it was grabbed; elsewhere the view jumps there and it's held by its
    /// middle.
    fn press_bar(&mut self, g: usize, what: Bar, at: (f32, f32)) {
        let (Some(area), Some(v)) = (self.areas.groups.get(g).copied(), self.view_of(g)) else {
            return;
        };
        let grab = match what {
            Bar::Down => {
                let Some(t) = overview::vthumb(area.bar, &v) else {
                    return;
                };
                if t.rect.contains(at.0, at.1) {
                    at.1 - t.rect.y
                } else {
                    t.rect.h / 2.0
                }
            }
            Bar::Along => {
                let track = overview::htrack(area.text, &v, self.cell_w());
                let Some(t) = overview::hthumb(track, &v) else {
                    return;
                };
                if t.rect.contains(at.0, at.1) {
                    at.0 - t.rect.x
                } else {
                    t.rect.w / 2.0
                }
            }
            Bar::Map => {
                let (Some(map), Some(rect)) = (&v.minimap, area.minimap) else {
                    return;
                };
                let geom = overview::Map::new(rect, &v, map);
                if geom.slider.contains(at.0, at.1) {
                    at.1 - geom.slider.y
                } else {
                    // A click elsewhere centres the view on that line.
                    let line = geom.line_at(rect, at.1);
                    let row = line.saturating_sub(v.area.h as usize / 2) as f64;
                    self.dispatch(Event::ScrollTo { group: g, row });
                    self.bar_drag = Some(BarDrag {
                        group: g,
                        what,
                        grab: geom.slider.h / 2.0,
                    });
                    return;
                }
            }
        };
        self.bar_drag = Some(BarDrag {
            group: g,
            what,
            grab,
        });
        self.drag_bar(at);
    }

    /// A held bar follows the mouse.
    fn drag_bar(&mut self, at: (f32, f32)) {
        let Some(drag) = self.bar_drag else { return };
        let g = drag.group;
        let (Some(area), Some(v)) = (self.areas.groups.get(g).copied(), self.view_of(g)) else {
            return;
        };
        self.show_bars(g);
        match drag.what {
            Bar::Down => {
                if let Some(t) = overview::vthumb(area.bar, &v) {
                    let row = t.position(at.1 - drag.grab - area.bar.y);
                    self.dispatch(Event::ScrollTo { group: g, row });
                }
            }
            Bar::Along => {
                let track = overview::htrack(area.text, &v, self.cell_w());
                if let Some(t) = overview::hthumb(track, &v) {
                    let col = t.position(at.0 - drag.grab - track.x).round();
                    let cols = (col - v.hscroll as f64) as f32;
                    if cols != 0.0 {
                        self.dispatch(Event::Scroll {
                            group: g,
                            rows: 0.0,
                            cols,
                        });
                    }
                }
            }
            Bar::Map => {
                if let (Some(map), Some(rect)) = (&v.minimap, area.minimap) {
                    let geom = overview::Map::new(rect, &v, map);
                    let row = geom.position(at.1 - drag.grab - rect.y);
                    self.dispatch(Event::ScrollTo { group: g, row });
                }
            }
        }
    }

    /// Group `g`'s thumbs show for a while.
    fn show_bars(&mut self, g: usize) {
        let until = Instant::now() + BARS_LINGER;
        match self.bars_until.iter_mut().find(|(group, _)| *group == g) {
            Some(entry) => entry.1 = until,
            None => self.bars_until.push((g, until)),
        }
    }

    /// The mouse near a group's code or bars shows its thumbs.
    fn hover_bars(&mut self, at: (f32, f32)) {
        let near = self.areas.groups.iter().position(|a| {
            let right = a.bar.right();
            Rect::new(a.text.x, a.text.y, right - a.text.x, a.text.h).contains(at.0, at.1)
        });
        if let Some(g) = near {
            self.show_bars(g);
            self.redraw();
        }
    }

    /// How group `g`'s bars look now.
    fn bars_shown(&self, g: usize) -> overview::Shown {
        let dragging = self.bar_drag.is_some_and(|d| d.group == g);
        let alpha = match self.bars_until.iter().find(|(group, _)| *group == g) {
            _ if dragging => 1.0,
            Some(&(_, until)) => {
                let now = Instant::now();
                if now < until {
                    1.0
                } else {
                    1.0 - (now - until).as_secs_f32() / BARS_FADE.as_secs_f32()
                }
            }
            None => 0.0,
        }
        .clamp(0.0, 1.0);
        let area = self.areas.groups.get(g).copied().unwrap_or_default();
        let on = |r: Rect| self.pointer.is_some_and(|(x, y)| r.contains(x, y));
        overview::Shown {
            alpha,
            thumb_hovered: on(area.bar),
            dragging,
            map_hovered: area.minimap.is_some_and(on),
        }
    }

    /// When a fading thumb next needs drawing.
    fn bars_due(&self) -> Option<Instant> {
        let now = Instant::now();
        self.bars_until
            .iter()
            .map(|&(_, until)| until)
            .filter(|&until| until + BARS_FADE > now)
            .map(|until| {
                if until > now {
                    until
                } else {
                    now + Duration::from_millis(16)
                }
            })
            .min()
    }

    /// The group whose code is under a point.
    fn group_at(&self, (x, y): (f32, f32)) -> Option<usize> {
        self.areas.groups.iter().position(|g| g.text.contains(x, y))
    }

    /// A point in group `g`'s code as the editor counts it: rows from the
    /// top of the view, the part scrolled off included, and columns from
    /// the group's left edge.
    fn text_point(&self, g: usize, (x, y): (f32, f32)) -> (f32, f32) {
        let (Some(grid), Some(area)) = (&self.grid, self.areas.groups.get(g)) else {
            return (0.0, 0.0);
        };
        let s = self.scale();
        let frac = self
            .drawn_text
            .iter()
            .find(|v| v.group == g)
            .map_or(0.0, |v| v.top_frac);
        let a = area.text;
        (
            (y - a.y) * s / grid.cell_h as f32 + frac,
            (x - a.x) * s / grid.cell_w as f32,
        )
    }

    /// Tells the editor what the mouse did in group `g`'s code.
    fn pointer_event(&mut self, g: usize, kind: MouseKind, at: (f32, f32), clicks: u8) {
        let (row, col) = self.text_point(g, at);
        self.dispatch(Event::Pointer(Pointer {
            group: g,
            kind,
            row,
            col,
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            ctrl: self.modifiers.control_key(),
            clicks,
        }));
    }

    /// The cell of group `g`'s code a point is nearest the gap before, as
    /// a press or drag picks it.
    fn gap_cell(&self, g: usize, at: (f32, f32)) -> (i32, i32) {
        let (row, col) = self.text_point(g, at);
        (row.floor() as i32, (col + 0.5).floor() as i32)
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

    fn press(&mut self, at: (f32, f32)) {
        let clicks = self.clicks.count(at);
        let Some((_, hit)) = self.hit_at(at) else {
            return;
        };
        match hit.clone() {
            Hit::Text => {
                if let Some(g) = self.group_at(at) {
                    self.pressed_group = Some(g);
                    self.dragged_to = Some(self.gap_cell(g, at));
                    self.pointer_event(g, MouseKind::Down, at, clicks);
                }
            }
            Hit::Scrollbar { group, horizontal } => {
                let what = if horizontal { Bar::Along } else { Bar::Down };
                self.press_bar(group, what, at);
            }
            Hit::Minimap(group) => self.press_bar(group, Bar::Map, at),
            Hit::Terminal(id) => {
                self.dispatch(Event::Ui(UiEvent::Terminal(id)));
                if !self.app.terminals.wants_mouse(id)
                    && let Some(cell) = self.term_cell(id, at)
                {
                    self.app
                        .terminals
                        .select(id, cell, terminal::SelectKind::Start(clicks.min(3)));
                    self.term_drag = Some(id);
                    self.redraw();
                }
            }
            Hit::MaximizePanel => {
                if let Some(shown) = &self.window {
                    let h = shown.window.inner_size().height as f32 / self.scale();
                    self.sizes.maximize_panel(h);
                }
                self.fit_text();
                self.redraw();
            }
            // A double click puts the groups back to the same sizes.
            Hit::GroupSash(_) if clicks >= 2 => {
                let even = vec![(1.0, 1.0); self.areas.groups.len()];
                self.dispatch(Event::Ui(UiEvent::GroupSizes(even)));
            }
            Hit::GroupSash(sash) => {
                self.group_drag = Some((sash, at, self.areas.groups.clone()));
            }
            Hit::Ui(UiEvent::Tab { index, .. }) => {
                self.hold_tab(None, index, at);
                self.dispatch(Event::Ui(UiEvent::Tab { index, clicks }))
            }
            Hit::Ui(UiEvent::GroupTab { group, index, .. }) => {
                self.hold_tab(Some(group), index, at);
                self.dispatch(Event::Ui(UiEvent::GroupTab {
                    group,
                    index,
                    clicks,
                }))
            }
            // A file opens on release, so it can be dragged first.
            Hit::Ui(UiEvent::Target { side, index, .. })
                if self.hold_entry(side, index, clicks, at) => {}
            Hit::Ui(UiEvent::Target { side, index, .. }) => {
                self.dispatch(Event::Ui(UiEvent::Target {
                    side,
                    index,
                    clicks,
                }))
            }
            Hit::Ui(event) => self.dispatch(Event::Ui(event)),
            Hit::Command(command) => self.dispatch(Event::Command(command.into())),
            Hit::Key(key) => self.dispatch(Event::Key(key)),
            Hit::Sash(side) => self.sash = Some(side),
            // A double click on a strip's empty end opens a new file there.
            Hit::TabStrip(group) if clicks >= 2 => {
                let view = self.app.editor.window_view();
                let g = group.unwrap_or(0);
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
                self.dispatch(Event::Command("file.new".into()));
            }
            Hit::Side(_) | Hit::Block | Hit::TabStrip(_) => {}
        }
    }

    /// The cell of terminal `id` under `at`, kept inside its screen.
    fn term_cell(&self, id: u64, at: (f32, f32)) -> Option<(usize, usize)> {
        let (_, r) = self.term_panes.iter().find(|(i, _)| *i == id)?;
        let grid = self.grid.as_ref()?;
        let s = self.scale();
        let col = ((at.0 - r.x) * s / grid.cell_w as f32).max(0.0) as usize;
        let line = ((at.1 - r.y) * s / grid.cell_h as f32).max(0.0) as usize;
        let cols = (r.w * s / grid.cell_w as f32).floor().max(1.0) as usize;
        let lines = (r.h * s / grid.cell_h as f32).floor().max(1.0) as usize;
        Some((col.min(cols - 1), line.min(lines - 1)))
    }

    /// Remembers the tab pressed on, so moving the mouse can drag it.
    fn hold_tab(&mut self, group: Option<usize>, index: usize, at: (f32, f32)) {
        let view = self.app.editor.window_view();
        let tabs = match group {
            Some(g) => view.groups.get(g).map(|g| &g.tabs),
            None => Some(&view.tabs),
        };
        if let Some(tab) = tabs.and_then(|t| t.get(index)) {
            self.tab_drag = Some(TabDrag {
                name: tab.name.clone(),
                icon: Some(tab.icon),
                from: at,
                moved: false,
                entry: None,
            });
        }
    }

    /// A file in a side's view pressed on, its `index`th element: held, to
    /// drag into the editor, if it's a file. Whether it was.
    fn hold_entry(&mut self, side: Side, index: usize, clicks: u8, at: (f32, f32)) -> bool {
        let view = self.app.editor.window_view();
        let sv = match side {
            Side::Left => view.left.as_ref(),
            Side::Right => view.right.as_ref(),
            Side::Bottom => view.bottom.as_ref(),
        };
        let Some(sv) = sv else { return false };
        let file = sv
            .nodes
            .iter()
            .zip(&sv.targets)
            .find_map(|(node, t)| match node {
                Node::Entry(e) if *t == Some(index) && e.id.starts_with("f:") => Some(e),
                _ => None,
            });
        let Some(file) = file else { return false };
        self.tab_drag = Some(TabDrag {
            name: file.label.clone(),
            icon: file.icon,
            from: at,
            moved: false,
            entry: Some((side, index, clicks)),
        });
        true
    }

    /// Where a tab or file dragged to `at` would land: on a tab or the
    /// strip, or on a group's code, on the side it's nearest if it's near
    /// one. A tab dropped on its own group's middle, or its group's only
    /// tab on any of it, goes nowhere new.
    fn drop_at(&self, at: (f32, f32)) -> Option<Landing> {
        let view = self.app.editor.window_view();
        let focused = view.groups.iter().position(|g| g.focused).unwrap_or(0);
        let strip = match self.hit_at(at).map(|(_, h)| h.clone()) {
            Some(Hit::Ui(UiEvent::Tab { index, .. } | UiEvent::CloseTab(index))) => {
                Some((focused, Some(index)))
            }
            Some(Hit::Ui(
                UiEvent::GroupTab { group, index, .. } | UiEvent::CloseGroupTab { group, index },
            )) => Some((group, Some(index))),
            Some(Hit::TabStrip(group)) => Some((group.unwrap_or(focused), None)),
            _ => None,
        };
        if let Some((group, at)) = strip {
            return Some(Landing::Strip { group, at });
        }
        let (x, y) = at;
        let (group, area) = self.areas.groups.iter().enumerate().find(|(_, a)| {
            let body = a.body();
            body.contains(x, y)
        })?;
        let body = area.body();
        let (fx, fy) = ((x - body.x) / body.w, (y - body.y) / body.h);
        let near = [
            (fx, Edge::Left),
            (1.0 - fx, Edge::Right),
            (fy, Edge::Up),
            (1.0 - fy, Edge::Down),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let edge = (near.0 < 0.3).then_some(near.1);
        let dragging_tab = self.tab_drag.as_ref().is_some_and(|d| d.entry.is_none());
        if dragging_tab && group == focused {
            let alone = view.groups.get(group).is_some_and(|g| g.tabs.len() <= 1);
            if edge.is_none() || alone {
                return None;
            }
        }
        let (hw, hh) = (body.w / 2.0, body.h / 2.0);
        let zone = match edge {
            None => body,
            Some(Edge::Left) => Rect::new(body.x, body.y, hw, body.h),
            Some(Edge::Right) => Rect::new(body.x + hw, body.y, hw, body.h),
            Some(Edge::Up) => Rect::new(body.x, body.y, body.w, hh),
            Some(Edge::Down) => Rect::new(body.x, body.y + hh, body.w, hh),
        };
        Some(Landing::Body { group, edge, zone })
    }

    /// A dragged tab or file let go at `at`, where [`Self::drop_at`]
    /// says.
    fn drop_tab(&mut self, at: (f32, f32), entry: Option<(Side, usize, u8)>) {
        let Some(target) = self.drop_at(at) else {
            return;
        };
        let event = match (target, entry) {
            (Landing::Strip { group, at }, None) => UiEvent::DropTab { group, at },
            (
                Landing::Body {
                    group, edge: None, ..
                },
                None,
            ) => UiEvent::DropTab { group, at: None },
            (
                Landing::Body {
                    group,
                    edge: Some(edge),
                    ..
                },
                None,
            ) => UiEvent::SplitTab { group, edge },
            (Landing::Strip { group, .. }, Some((side, index, _))) => UiEvent::DropEntry {
                side,
                index,
                group,
                edge: None,
            },
            (Landing::Body { group, edge, .. }, Some((side, index, _))) => UiEvent::DropEntry {
                side,
                index,
                group,
                edge,
            },
        };
        self.dispatch(Event::Ui(event));
    }

    /// The pointer's shape for what it's over: an I-beam over the code, a
    /// grabbing hand while dragging a tab.
    fn set_pointer(&mut self, icon: CursorIcon) {
        if self.cursor_icon != icon
            && let Some(shown) = &self.window
        {
            shown.window.set_cursor(icon);
            self.cursor_icon = icon;
        }
    }

    fn wheel(&mut self, lines: f64, sideways: bool) {
        let Some(at) = self.pointer else {
            return;
        };
        let over = self.hit_at(at).map(|(_, h)| h.clone());
        // Over a terminal, it scrolls back through the history.
        if let Some(Hit::Terminal(id)) = over {
            let lines = (lines * 3.0).round() as i32;
            if lines != 0 {
                self.app.terminals.scroll(id, lines);
                self.redraw();
            }
            return;
        }
        // Over a tab strip, it scrolls the tabs along.
        let strip = match over {
            Some(Hit::TabStrip(group)) => Some(group.unwrap_or(0)),
            Some(Hit::Ui(
                UiEvent::GroupTab { group, .. } | UiEvent::CloseGroupTab { group, .. },
            )) => Some(group),
            _ => None,
        };
        if let Some(g) = strip {
            if let Some(strip) = self.strips.get_mut(g) {
                strip.0 -= lines as f32 * 40.0;
            }
            self.redraw();
            return;
        }
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
        // Three rows a step, or six columns sideways, as VS Code scrolls,
        // times the settings' sensitivity, and again with Alt.
        let group = self.group_at(at).unwrap_or(self.focused_group());
        let steps = lines as f32 * self.sensitivity();
        let (rows, cols) = if sideways || self.modifiers.shift_key() {
            (0.0, -steps * 6.0)
        } else {
            (-steps * 3.0, 0.0)
        };
        self.dispatch(Event::Scroll { group, rows, cols });
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

    /// A touchpad moved the text by (`dx`, `dy`) physical pixels: the text
    /// follows it exactly, parts of a row and column too.
    fn touch_scroll(&mut self, dx: f64, dy: f64) {
        let Some(at) = self.pointer else { return };
        let over_text = matches!(self.hit_at(at), None | Some((_, Hit::Text)));
        let (cw, ch) = self
            .grid
            .as_ref()
            .map_or((10.0, 20.0), |g| (g.cell_w as f64, g.cell_h as f64));
        let group = self.group_at(at).filter(|_| over_text);
        // Off the text: as many wheel steps as it adds up to.
        let Some(group) = group else {
            self.scroll += if dy.abs() >= dx.abs() { dy } else { dx };
            let step = ch * 3.0;
            let steps = (self.scroll / step).trunc();
            if steps != 0.0 {
                self.scroll -= steps * step;
                self.wheel(steps, dy.abs() < dx.abs());
            }
            return;
        };
        self.touched = true;
        let k = self.sensitivity();
        self.dispatch(Event::Scroll {
            group,
            rows: (-dy / ch) as f32 * k,
            cols: (-dx / cw) as f32 * k,
        });
    }

    /// How far the wheel and touchpad scroll the code, times their own.
    fn sensitivity(&self) -> f32 {
        let w = &self.app.window;
        if self.modifiers.alt_key() {
            w.wheel_sensitivity * w.fast_sensitivity
        } else {
            w.wheel_sensitivity
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
        let cell_h = self.grid.as_ref().map_or(1, |g| g.cell_h) as f32;
        let texts = self.app.editor.text_views();
        let caret_on = self.caret_level();
        let caret = code::Caret {
            style: self.app.window.cursor_style,
            on: caret_on,
            expand: self.app.window.cursor_blinking == Blinking::Expand,
        };
        // A scroll since the last frame glides there from where it was, but
        // a touchpad's follows the fingers already.
        let mut tops = Vec::new();
        let mut moved_groups = Vec::new();
        for v in &texts {
            if tops.iter().any(|(g, _)| *g == v.group) {
                continue;
            }
            let was = self
                .last_top
                .iter()
                .find(|(g, _)| *g == v.group)
                .map(|t| t.1);
            // Scrolling shows the thumbs.
            if was.is_some_and(|was| was != v.top_row) {
                moved_groups.push(v.group);
            }
            if let Some(was) = was
                && was != v.top_row
                && !self.touched
                && self.app.window.smooth_scrolling
            {
                let rows = (v.top_row - was) as f32;
                let from = self.glide_offset(v.group) + rows * cell_h;
                self.glide =
                    (rows.abs() <= v.overscan as f32).then(|| (v.group, Instant::now(), from));
            }
            tops.push((v.group, v.top_row));
        }
        self.last_top = tops;
        for g in moved_groups {
            self.show_bars(g);
        }
        let bars: Vec<(usize, overview::Shown)> = (0..self.areas.groups.len())
            .map(|g| (g, self.bars_shown(g)))
            .collect();
        let fading = bars.iter().any(|(_, b)| b.alpha > 0.0 && b.alpha < 1.0);
        self.touched = false;
        let glide = self.glide.map_or(0.0, |(g, ..)| self.glide_offset(g));
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
        let mut view = self.app.editor.window_view();
        // Code to colour asks as it's laid out: ask, then lay it out again.
        if self.app.serve_colours() {
            view = self.app.editor.window_view();
        }
        let hovered = self.pointer.and_then(|p| {
            self.hits
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (r, _))| r.contains(p.0, p.1))
                .map(|(i, _)| i)
        });
        let drawn = Some((
            view.clone(),
            hovered,
            self.state.scroll,
            (size.width, size.height),
            self.app.look.clone(),
        ));
        let gliding = self.glide.is_some();
        // A dragged tab follows the mouse, so every move draws.
        let dragging = self.tab_drag.as_ref().is_some_and(|d| d.moved);
        let tip = self
            .tip
            .as_ref()
            .filter(|t| t.0 <= Instant::now())
            .map(|t| t.1.clone());
        let tip_changed = tip != self.tip_drawn;
        if self.drawn == drawn
            && self.bars_drawn == bars
            && !tip_changed
            && self.drawn_text == texts
            && caret_on == self.caret_drawn
            && self.term_drawn == self.app.terminals.version
            && !gliding
            && !dragging
        {
            return;
        }
        // Packs changed: their pictures may have too. Too many bitmaps:
        // start again.
        if self.drawn.as_ref().map(|d| &d.4) != Some(&self.app.look)
            || self.bitmaps.bytes() > Bitmaps::LIMIT
        {
            self.bitmaps.clear();
        }
        let (w, h) = (pw as f32 / s, ph as f32 / s);
        let areas = frame::areas(&view, w, h, self.app.look.layout.gap, &self.sizes);
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
            sash_hits: Vec::new(),
            tips: Vec::new(),
            tip: tip.clone(),
            tab_drag: self
                .tab_drag
                .as_ref()
                .filter(|d| d.moved)
                .map(|d| (d.name.clone(), d.icon)),
            drop_zone: match self.drop {
                Some(Landing::Body { zone, .. }) => Some(zone),
                _ => None,
            },
            strips: std::mem::take(&mut self.strips),
            term_panes: Vec::new(),
            panel_max: self.sizes.bottom_before.is_some(),
        };
        f.base(&view, &areas, &mut self.state, w, h);
        for text in &texts {
            let Some(area) = areas.groups.get(text.group) else {
                continue;
            };
            let t = area.text;
            let glide = match self.glide {
                Some((g, ..)) if g == text.group => glide,
                _ => 0.0,
            };
            grid.draw_code(
                text,
                &mut f.canvas,
                [t.x * s, t.y * s, t.w * s, t.h * s],
                &self.app.look.palette,
                caret,
                glide,
            );
        }
        // The terminals' screens, each fitted to its pane first.
        let colours = self.app.terminal_colours();
        let term = view.terminal.as_ref();
        let focused = term.filter(|t| t.focused).and_then(|t| t.active);
        for &(id, r) in &f.term_panes {
            let cols = (r.w * s / grid.cell_w as f32).floor() as usize;
            let lines = (r.h * s / grid.cell_h as f32).floor() as usize;
            let cell = (grid.cell_w as u16, grid.cell_h as u16);
            self.app.terminals.resize(id, cols, lines, cell);
            let ended = term
                .and_then(|t| t.sessions.iter().find(|s| s.id == id))
                .is_some_and(|s| s.exited.is_some());
            if let Some(mut screen) = self.app.terminals.screen(id, &colours) {
                if ended {
                    screen.cursor = None;
                }
                let at = ((r.x * s).round(), (r.y * s).round());
                grid.draw_terminal(&screen, &mut f.canvas, at, &colours, focused == Some(id));
            }
        }
        self.term_panes = f.term_panes.clone();
        self.term_drawn = self.app.terminals.version;
        // Beside each group's code, its minimap and scrollbars, and a
        // shadow under the tabs once it's scrolled; the review side by
        // side has them on its new side.
        let p = self.app.look.palette;
        let cell_w = grid.cell_w as f32 / s;
        for (g, area) in areas.groups.iter().enumerate() {
            let Some(v) = texts.iter().rfind(|v| v.group == g) else {
                continue;
            };
            let shown = bars.get(g).map_or_else(Default::default, |b| b.1);
            if let Some(rect) = area.minimap {
                overview::draw_minimap(&mut f.canvas, &p, rect, v, shown);
                f.hits.push((rect, Hit::Minimap(g)));
            }
            overview::draw_vbar(&mut f.canvas, &p, area.bar, v, shown);
            f.hits.push((
                area.bar,
                Hit::Scrollbar {
                    group: g,
                    horizontal: false,
                },
            ));
            let track = overview::htrack(area.text, v, cell_w);
            if overview::hthumb(track, v).is_some() {
                overview::draw_hbar(&mut f.canvas, &p, track, v, shown);
                f.hits.push((
                    track,
                    Hit::Scrollbar {
                        group: g,
                        horizontal: true,
                    },
                ));
            }
            overview::draw_shadow(&mut f.canvas, area.text, v);
        }
        let cell = (grid.cell_w as f32 / s, grid.cell_h as f32 / s);
        f.floating(&view, &areas, cell, self.context_at, w, h);
        f.overlays(&view, &areas, w, h);
        self.hits = std::mem::take(&mut f.hits);
        self.strips = std::mem::take(&mut f.strips);
        self.tips = std::mem::take(&mut f.tips);
        self.tip_drawn = tip;
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
        self.drawn = drawn.map(|d| (d.0, hovered, d.2, d.3, d.4));
        // The input method's candidates open by the cursor.
        if let Some(v) = texts.last()
            && let Some((cx, cy)) = v.carets.first().and_then(|c| c.cell)
            && let Some(area) = self.areas.groups.get(v.group)
        {
            let (cw, ch) = (grid.cell_w as f32 / s, grid.cell_h as f32 / s);
            let x = area.text.x + (v.gutter as f32 + cx as f32) * cw;
            let y = area.text.y + (cy as f32 - v.top_frac + 1.0) * ch;
            let spot = (x.round() as i32, y.round() as i32);
            if self.ime_spot != Some(spot) {
                self.ime_spot = Some(spot);
                shown.window.set_ime_cursor_area(
                    winit::dpi::LogicalPosition::new(x, y - ch),
                    winit::dpi::LogicalSize::new(cw, ch),
                );
            }
        }
        self.drawn_text = texts;
        self.bars_drawn = bars;
        self.caret_drawn = caret_on;
        if fading {
            self.redraw();
        }
        // Until the glide ends, every frame moves it on.
        if self.glide.is_some() {
            if glide == 0.0 {
                self.glide = None;
            }
            self.redraw();
        }
    }

    /// Whether the caret blinks now: for a while after each input, then
    /// it's on for good once the user has been away a while.
    fn blinking(&self) -> bool {
        self.focused
            && self.blink_from.elapsed() < BLINK_FOR
            && self.app.window.cursor_blinking != Blinking::Solid
    }

    /// Where the blink is, in blinks since it was last on from the start:
    /// from 0 up to 2, on for the first and off for the second.
    fn blink_phase(&self) -> f32 {
        let blink = BLINK.as_secs_f32();
        (self.blink_from.elapsed().as_secs_f32() / blink) % 2.0
    }

    /// How far on the caret is now, from 0 to 1, as the settings have it
    /// blink.
    fn caret_level(&self) -> f32 {
        if !self.blinking() {
            return 1.0;
        }
        let t = self.blink_phase();
        let level = match self.app.window.cursor_blinking {
            Blinking::Blink | Blinking::Solid => (t < 1.0) as u8 as f32,
            Blinking::Phase => 0.5 + 0.5 * (t * std::f32::consts::PI).cos(),
            // On, fading out, off, fading in.
            Blinking::Smooth | Blinking::Expand => {
                if t < 1.0 - FADE {
                    1.0
                } else if t < 1.0 {
                    (1.0 - t) / FADE
                } else if t < 2.0 - FADE {
                    0.0
                } else {
                    (t - (2.0 - FADE)) / FADE
                }
            }
        };
        // In steps, so frames that look the same aren't drawn twice.
        (level * 32.0).round() / 32.0
    }

    /// When the caret next changes, if it's still blinking: its next
    /// blink, or its next frame while it fades.
    fn next_blink(&self) -> Option<Instant> {
        if !self.blinking() {
            return None;
        }
        let now = Instant::now();
        let t = self.blink_phase();
        let at = |phase: f32| now + BLINK.mul_f32(phase - t);
        Some(match self.app.window.cursor_blinking {
            Blinking::Blink | Blinking::Solid => at(if t < 1.0 { 1.0 } else { 2.0 }),
            Blinking::Phase => now + FADE_FRAME,
            Blinking::Smooth | Blinking::Expand => {
                let fading = (1.0 - FADE..1.0).contains(&t) || t >= 2.0 - FADE;
                if fading {
                    now + FADE_FRAME
                } else if t < 1.0 - FADE {
                    at(1.0 - FADE)
                } else {
                    at(2.0 - FADE)
                }
            }
        })
    }

    /// How far, in physical pixels, the lines are from where they're
    /// gliding to: eased out, so it starts quick and settles.
    fn glide_offset(&self, group: usize) -> f32 {
        let Some((g, start, from)) = self.glide else {
            return 0.0;
        };
        if g != group {
            return 0.0;
        }
        let t = start.elapsed().as_secs_f32() / GLIDE.as_secs_f32();
        if t >= 1.0 {
            return 0.0;
        }
        from * (1.0 - t).powi(3)
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
            if self.caret_level() != self.caret_drawn {
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
            WindowEvent::RedrawRequested => self.paint(),
            // Files dropped from the desktop open, as in any editor.
            WindowEvent::DroppedFile(path) => {
                self.app.open_dropped(&path);
                self.fit_text();
                self.redraw();
            }
            WindowEvent::Focused(on) => {
                self.focused = on;
                if !on {
                    self.dispatch(Event::AutoSave(merlin_core::AutoSaveWhy::Window));
                }
                self.blink_from = Instant::now();
                self.redraw();
            }
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
                    self.drop = None;
                    if drag.moved {
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
                    self.dispatch(Event::Key(Key::plain(KeyCode::Char(c))));
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
        let wake = [
            self.rest.map(|(when, _)| when),
            self.next_blink(),
            tip,
            self.app.due(),
            self.bars_due(),
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

    fn moved(&mut self, position: PhysicalPosition<f64>) {
        let s = self.scale() as f64;
        let at = ((position.x / s) as f32, (position.y / s) as f32);
        let before = self.pointer.and_then(|p| self.hit_at(p).map(|(i, _)| i));
        self.pointer = Some(at);
        if self.bar_drag.is_some() {
            self.drag_bar(at);
            return;
        }
        self.hover_bars(at);
        if let Some((sash, from, areas)) = &self.group_drag {
            let sizes = group_sizes(*sash, &areas.clone(), (at.0 - from.0, at.1 - from.1));
            self.dispatch(Event::Ui(UiEvent::GroupSizes(sizes)));
            return;
        }
        if let Some(side) = self.sash {
            let s = self.scale();
            if let Some(shown) = &self.window {
                let size = shown.window.inner_size();
                let (w, h) = (size.width as f32 / s, size.height as f32 / s);
                self.sizes.drag(side, at, w, h);
            }
            self.fit_text();
            self.redraw();
            return;
        }
        if let Some(drag) = &mut self.tab_drag {
            let (dx, dy) = (at.0 - drag.from.0, at.1 - drag.from.1);
            if !drag.moved && dx.hypot(dy) > DRAG_START {
                drag.moved = true;
            }
            if drag.moved {
                self.set_pointer(CursorIcon::Grabbing);
                self.drop = self.drop_at(at);
                self.redraw();
            }
            return;
        }
        if let Some(id) = self.term_drag {
            if let Some(cell) = self.term_cell(id, at) {
                self.app
                    .terminals
                    .select(id, cell, terminal::SelectKind::Extend);
                self.redraw();
            }
            return;
        }
        if let Some(g) = self.pressed_group {
            let cell = self.gap_cell(g, at);
            if self.dragged_to != Some(cell) {
                self.dragged_to = Some(cell);
                self.pointer_event(g, MouseKind::Drag, at, 1);
            }
            return;
        }
        let over = self.hit_at(at).map(|(i, h)| (i, h.clone()));
        // Resting on an icon brings up what it is.
        let tip = self
            .tips
            .iter()
            .find(|(r, _)| r.contains(at.0, at.1))
            .cloned();
        match (&self.tip, tip) {
            (Some((_, (r, _))), Some((now, _))) if *r == now => {}
            (_, Some(tip)) => self.tip = Some((Instant::now() + TIP_DELAY, tip)),
            (Some(_), None) => {
                self.tip = None;
                self.redraw();
            }
            (None, None) => {}
        }
        let icon = match over {
            Some((_, Hit::Text | Hit::Terminal(_))) => CursorIcon::Text,
            Some((_, Hit::Sash(Side::Bottom))) => CursorIcon::RowResize,
            Some((_, Hit::Sash(_))) => CursorIcon::ColResize,
            Some((_, Hit::GroupSash(frame::GroupSash::Column(_)))) => CursorIcon::ColResize,
            Some((_, Hit::GroupSash(frame::GroupSash::Row(_)))) => CursorIcon::RowResize,
            _ => CursorIcon::Default,
        };
        self.set_pointer(icon);
        // Over a menu's title, its menu opens, as menus do.
        if let Some((_, Hit::Ui(UiEvent::MenuTitle(i)))) = over {
            self.dispatch(Event::Ui(UiEvent::MenuTitle(i)));
        }
        if let (Some((_, Hit::Text)), Some(g)) = (&over, self.group_at(at)) {
            // The editor lights up what's under the mouse in the code, and
            // a rest on a word brings up its hover.
            let (row, col) = self.text_point(g, at);
            let cell = (g, row.floor() as i32, col.floor() as i32);
            if self.over_cell != Some(cell) {
                self.over_cell = Some(cell);
                self.pointer_event(g, MouseKind::Move, at, 1);
                self.rest = Some((Instant::now() + HOVER_DELAY, at));
            }
            return;
        }
        self.over_cell = None;
        self.rest = None;
        if before != over.map(|(i, _)| i) {
            self.redraw();
        }
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
