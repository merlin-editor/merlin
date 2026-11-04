//! The window's frame, laid out and drawn in pixels as the design has it:
//! a slim title bar, the activity rail, the side bar, editor and panel as
//! rounded islands on a darker ground, and a thin status bar. Drawing also
//! records what each clickable part does.

use merlin_core::panel::{Entry, Node, Tone};
use merlin_core::{Key, KeyCode, PaletteView, Side, SideView, ToastView, UiEvent, WindowView};

use super::canvas::{Canvas, Face, Rect, Text};
use crate::theme::{self, Palette, mix};

const TITLE_H: f32 = 40.0;
const STATUS_H: f32 = 26.0;
const RAIL_W: f32 = 44.0;
const SIDE_W: f32 = 260.0;
const RIGHT_W: f32 = 320.0;
const BOTTOM_H: f32 = 240.0;
const TABS_H: f32 = 38.0;
const CRUMBS_H: f32 = 28.0;
const HEADER_H: f32 = 38.0;
const ROW_H: f32 = 26.0;
const PALETTE_ROWS: usize = 12;

/// What a click on part of the frame does.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Ui(UiEvent),
    Command(&'static str),
    Key(Key),
    /// The code, where the editor takes the mouse itself.
    Text,
    /// Inside a side's content, where the wheel scrolls it.
    Side(Side),
    /// A floating box that takes clicks without doing anything.
    Block,
}

/// Where the islands are, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Areas {
    pub left: Option<Rect>,
    pub right: Option<Rect>,
    pub bottom: Option<Rect>,
    pub editor: Rect,
    /// The code inside the editor island.
    pub text: Rect,
}

/// What the frame keeps between frames: how far each side is scrolled.
#[derive(Default)]
pub struct State {
    pub scroll: [f32; 3],
    /// The selection each side last showed, to scroll to it when it moves.
    selected: [Option<usize>; 3],
}

fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
        Side::Bottom => 2,
    }
}

pub struct Frame<'a> {
    pub canvas: Canvas<'a>,
    pub text: &'a mut Text,
    pub pointer: Option<(f32, f32)>,
    pub hits: Vec<(Rect, Hit)>,
    pub radius: f32,
    pub p: Palette,
}

pub fn areas(view: &WindowView, w: f32, h: f32, gap: f32) -> Areas {
    let top = TITLE_H;
    let bottom_y = h - STATUS_H;
    let body_h = (bottom_y - top).max(0.0);
    let mut x = RAIL_W;
    let left = view.left.as_ref().map(|_| {
        let r = Rect::new(x, top, SIDE_W.min(w * 0.4), body_h);
        x += r.w + gap;
        r
    });
    let mut x1 = w - gap;
    let right = view.right.as_ref().map(|_| {
        let rw = RIGHT_W.min(w * 0.35);
        x1 -= rw;
        let r = Rect::new(x1, top, rw, body_h);
        x1 -= gap;
        r
    });
    let main_w = (x1 - x).max(0.0);
    let bottom = view.bottom.as_ref().map(|_| {
        let bh = BOTTOM_H.min(body_h * 0.45);
        Rect::new(x, bottom_y - bh, main_w, bh)
    });
    let editor_h = body_h - bottom.map_or(0.0, |b| b.h + gap);
    let editor = Rect::new(x, top, main_w, editor_h.max(0.0));
    let head = TABS_H
        + if view.crumbs.is_empty() {
            0.0
        } else {
            CRUMBS_H
        };
    let text = Rect::new(
        editor.x,
        editor.y + head + 4.0,
        editor.w,
        (editor.h - head - 4.0).max(0.0),
    );
    Areas {
        left,
        right,
        bottom,
        editor,
        text,
    }
}

impl Frame<'_> {
    fn hit(&mut self, r: Rect, hit: Hit) {
        self.hits.push((r, hit));
    }

    fn hovered(&self, r: Rect) -> bool {
        self.pointer.is_some_and(|(x, y)| r.contains(x, y))
    }

    /// Everything under the code: ground, title bar, rail, islands and
    /// their contents, and the status bar.
    pub fn base(&mut self, view: &WindowView, a: &Areas, state: &mut State, w: f32, h: f32) {
        let p = self.p;
        self.canvas.fill(Rect::new(0.0, 0.0, w, h), p.ground);
        self.title_bar(view, w);
        self.rail(view, h);
        for (side, area) in [
            (Side::Left, a.left),
            (Side::Right, a.right),
            (Side::Bottom, a.bottom),
        ] {
            let sv = match side {
                Side::Left => &view.left,
                Side::Right => &view.right,
                Side::Bottom => &view.bottom,
            };
            if let (Some(area), Some(sv)) = (area, sv) {
                self.canvas.round(area, self.radius, p.island);
                self.side(sv, side, area, state);
            }
        }
        self.canvas.round(a.editor, self.radius, p.island);
        self.tabs(view, a.editor);
        if !view.crumbs.is_empty() {
            self.crumbs(view, a.editor);
        }
        self.hit(a.text, Hit::Text);
        self.status(view, w, h);
    }

    /// What floats over the code: notices and the palette.
    pub fn overlays(&mut self, view: &WindowView, a: &Areas, w: f32, h: f32) {
        if let Some(toast) = &view.toast {
            self.toast(toast, a.editor);
        }
        if let Some(palette) = &view.palette {
            self.palette(palette, w, h);
        }
    }

    fn icon_button(&mut self, r: Rect, glyph: char, on: bool, hit: Hit) {
        let p = self.p;
        let hovered = self.hovered(r);
        if hovered || on {
            self.canvas
                .round(r, 6.0, if on { p.raised } else { p.hover });
        }
        let colour = if on || hovered { p.bright } else { p.muted };
        let face = Face::mono(15.0);
        let gw = self.text.width(&glyph.to_string(), face);
        let clip = r;
        self.text.draw_in_row(
            &mut self.canvas,
            &glyph.to_string(),
            face,
            r.x + (r.w - gw) / 2.0,
            r.y,
            r.h,
            colour,
            clip,
        );
        self.hit(r, hit);
    }

    fn title_bar(&mut self, view: &WindowView, w: f32) {
        let p = self.p;
        let bar = Rect::new(0.0, 0.0, w, TITLE_H);
        let ui = Face::ui(13.0);
        // The menus, behind one button.
        self.icon_button(
            Rect::new(10.0, 6.0, 30.0, 28.0),
            '\u{f0c9}',
            false,
            Hit::Key(Key::plain(KeyCode::F(10))),
        );
        // The project, with a mark in the accent.
        let mut x = 46.0;
        let mark = Rect::new(x + 6.0, 11.0, 18.0, 18.0);
        let name_w = self.text.width(&view.project, ui.bold());
        let project = Rect::new(x, 6.0, name_w + 40.0, 28.0);
        if self.hovered(project) {
            self.canvas.round(project, 6.0, p.hover);
        }
        self.canvas.round(mark, 5.0, p.accent);
        let m = "m";
        let mw = self.text.width(m, Face::ui(11.0).bold());
        self.text.draw_in_row(
            &mut self.canvas,
            m,
            Face::ui(11.0).bold(),
            mark.x + (18.0 - mw) / 2.0,
            mark.y,
            18.0,
            p.ground,
            mark,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &view.project,
            ui.bold(),
            x + 32.0,
            6.0,
            28.0,
            p.bright,
            bar,
        );
        self.hit(project, Hit::Command("file.open"));
        x += project.w + 4.0;
        if let Some(branch) = &view.branch {
            let label = format!("\u{e725} {branch}");
            let bw = self.text.width(&label, ui);
            let r = Rect::new(x, 6.0, bw + 16.0, 28.0);
            if self.hovered(r) {
                self.canvas.round(r, 6.0, p.hover);
            }
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                ui,
                x + 8.0,
                6.0,
                28.0,
                p.muted,
                bar,
            );
            self.hit(r, Hit::Ui(UiEvent::Rail(2)));
        }
        // The one search box.
        let sw = 440.0_f32.min(w * 0.42);
        let search = Rect::new((w - sw) / 2.0, 6.0, sw, 28.0);
        let hovered = self.hovered(search);
        self.canvas
            .round(search, 7.0, if hovered { p.widget } else { p.raised });
        self.text.draw_in_row(
            &mut self.canvas,
            "\u{ea6d}",
            Face::mono(13.0),
            search.x + 10.0,
            search.y,
            28.0,
            p.muted,
            search,
        );
        let hint = self.text.fit(
            "Search files, commands and symbols",
            ui,
            (sw - 100.0).max(0.0),
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &hint,
            ui,
            search.x + 30.0,
            search.y,
            28.0,
            p.muted,
            search,
        );
        self.chip("Ctrl P", search.right() - 8.0, search.y + 6.0, true);
        self.hit(search, Hit::Command("file.open"));
        // Layout toggles: side bar, panel, secondary side bar.
        let mut bx = w - 10.0 - 3.0 * 32.0;
        for (i, (glyph, command)) in [
            ('\u{ebf3}', "view.sidebar"),
            ('\u{ebf2}', "view.panel"),
            ('\u{ebf4}', "view.secondary"),
        ]
        .into_iter()
        .enumerate()
        {
            self.icon_button(
                Rect::new(bx, 6.0, 30.0, 28.0),
                glyph,
                view.shown[i],
                Hit::Command(command),
            );
            bx += 32.0;
        }
    }

    /// A key or label in a small grey box, ending at `right`.
    fn chip(&mut self, label: &str, right: f32, y: f32, end: bool) -> f32 {
        let p = self.p;
        let face = Face::ui(11.0);
        let tw = self.text.width(label, face);
        let x = if end { right - tw - 12.0 } else { right };
        let r = Rect::new(x, y, tw + 12.0, 16.0);
        self.canvas.round(r, 4.0, p.chip);
        self.text.draw_in_row(
            &mut self.canvas,
            label,
            face,
            x + 6.0,
            y,
            16.0,
            p.chip_text,
            r,
        );
        r.w
    }

    fn rail(&mut self, view: &WindowView, h: f32) {
        let p = self.p;
        let mut y = TITLE_H + 2.0;
        for (i, item) in view.rail.iter().enumerate() {
            let r = Rect::new(4.0, y, 36.0, 36.0);
            let hovered = self.hovered(r);
            if item.active {
                self.canvas.round(r, 8.0, p.raised);
            } else if hovered {
                self.canvas.round(r, 8.0, p.hover);
            }
            let face = Face::mono(18.0);
            let g = item.glyph.to_string();
            let gw = self.text.width(&g, face);
            let colour = if item.active || hovered {
                p.bright
            } else {
                p.muted
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &g,
                face,
                r.x + (36.0 - gw) / 2.0,
                y,
                36.0,
                colour,
                r,
            );
            if item.badge > 0 {
                let label = if item.badge > 99 {
                    "99+".to_string()
                } else {
                    item.badge.to_string()
                };
                let face = Face::ui(10.0).bold();
                let bw = self.text.width(&label, face).max(7.0) + 8.0;
                let b = Rect::new(r.right() - bw + 1.0, y + 3.0, bw, 15.0);
                self.canvas.round(b, 7.5, p.accent);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    b.x + 4.0,
                    b.y,
                    15.0,
                    p.ground,
                    b,
                );
            }
            self.hit(r, Hit::Ui(UiEvent::Rail(i)));
            y += 40.0;
            if y + 40.0 > h - STATUS_H {
                break;
            }
        }
    }

    fn side(&mut self, sv: &SideView, side: Side, area: Rect, state: &mut State) {
        let p = self.p;
        let header = Rect::new(area.x, area.y, area.w, HEADER_H);
        // The rail picks what the side bars show; the panel under the editor
        // has its own tabs: Problems and Output.
        if side == Side::Bottom && sv.panels.len() > 1 {
            // Panels that share the side, as tabs: Problems and Output.
            let mut x = area.x + 10.0;
            for (name, title, on) in &sv.panels {
                let face = Face::ui(12.5).medium();
                let tw = self.text.width(title, face);
                let r = Rect::new(x, area.y + 7.0, tw + 20.0, 24.0);
                if *on {
                    self.canvas.round(r, 6.0, p.raised);
                } else if self.hovered(r) {
                    self.canvas.round(r, 6.0, p.hover);
                }
                let colour = if *on { p.bright } else { p.muted };
                self.text.draw_in_row(
                    &mut self.canvas,
                    title,
                    face,
                    x + 10.0,
                    r.y,
                    24.0,
                    colour,
                    header,
                );
                self.hit(
                    r,
                    Hit::Ui(UiEvent::PanelTitle {
                        side,
                        name: name.clone(),
                    }),
                );
                x += r.w + 4.0;
            }
        } else {
            let title = title_case(&sv.title);
            self.text.draw_in_row(
                &mut self.canvas,
                &title,
                Face::ui(13.0).bold(),
                area.x + 14.0,
                area.y,
                HEADER_H,
                p.bright,
                header,
            );
        }
        let body = Rect::new(
            area.x,
            area.y + HEADER_H,
            area.w,
            (area.h - HEADER_H - 6.0).max(0.0),
        );
        self.hit(body, Hit::Side(side));

        // Lay the nodes out first, to keep the selection in view.
        let inner = area.w - 28.0;
        let mut rows = Vec::with_capacity(sv.nodes.len());
        let mut y = 0.0;
        for node in &sv.nodes {
            let h = self.node_height(node, inner);
            rows.push((y, h));
            y += h;
        }
        let total = y;
        let k = side_index(side);
        if sv.focused && state.selected[k] != Some(sv.selected) {
            if let Some(i) = sv.targets.iter().position(|t| *t == Some(sv.selected)) {
                let (ry, rh) = rows[i];
                let scroll = &mut state.scroll[k];
                if ry < *scroll {
                    *scroll = ry;
                } else if ry + rh > *scroll + body.h {
                    *scroll = ry + rh - body.h;
                }
            }
            state.selected[k] = Some(sv.selected);
        }
        let scroll = state.scroll[k].clamp(0.0, (total - body.h).max(0.0));
        state.scroll[k] = scroll;

        for (i, node) in sv.nodes.iter().enumerate() {
            let (ry, rh) = rows[i];
            let top = body.y + ry - scroll;
            if top + rh < body.y || top > body.bottom() {
                continue;
            }
            let r = Rect::new(area.x, top, area.w, rh);
            let target = sv.targets[i];
            let selected = target.is_some() && target == Some(sv.selected);
            self.node(node, sv, side, r, target, selected, body);
        }
    }

    fn node_height(&mut self, node: &Node, inner: f32) -> f32 {
        match node {
            Node::Heading(_) => 30.0,
            Node::Text(t) | Node::Styled(t, _) => {
                self.text.wrapped_height(t, Face::ui(12.5), inner) + 8.0
            }
            Node::Markdown(t) => self.text.wrapped_height(&plain(t), Face::ui(12.5), inner) + 8.0,
            Node::Button(_) | Node::Buttons(_) => 38.0,
            Node::Item { .. } | Node::Entry(_) => ROW_H,
            Node::Input { .. } => 40.0,
            Node::Divider => 13.0,
            Node::Footer => 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        node: &Node,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        selected: bool,
        clip: Rect,
    ) {
        let p = self.p;
        let inner = r.w - 28.0;
        let row_bg = |frame: &mut Self, row: Rect| {
            let bg = if selected && sv.focused {
                p.focus_row
            } else if selected {
                p.selected
            } else if frame.hovered(row) {
                p.hover
            } else {
                return;
            };
            frame.canvas.round(row, 6.0, bg);
        };
        match node {
            Node::Heading(t) => {
                let face = Face::ui(11.0).bold();
                self.text.draw_in_row(
                    &mut self.canvas,
                    &t.to_uppercase(),
                    face,
                    r.x + 14.0,
                    r.y + 4.0,
                    26.0,
                    p.muted,
                    clip,
                );
            }
            Node::Text(t) => {
                self.text.draw(
                    &mut self.canvas,
                    t,
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    p.text,
                    clip,
                    Some(inner),
                );
            }
            Node::Styled(t, tone) => {
                self.text.draw(
                    &mut self.canvas,
                    t,
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    tone_colour(&p, *tone),
                    clip,
                    Some(inner),
                );
            }
            Node::Markdown(t) => {
                self.text.draw(
                    &mut self.canvas,
                    &plain(t),
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    p.text,
                    clip,
                    Some(inner),
                );
            }
            Node::Item { label, tone, .. } => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                let label = self.text.fit(label, Face::ui(13.0), inner);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    Face::ui(13.0),
                    r.x + 14.0,
                    r.y,
                    r.h,
                    tone_colour(&p, *tone),
                    clip,
                );
                if let Some(index) = target {
                    self.hit(row, Hit::Ui(UiEvent::Target { side, index }));
                }
            }
            Node::Entry(e) => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                self.entry(e, row, clip);
                if let Some(index) = target {
                    self.hit(row, Hit::Ui(UiEvent::Target { side, index }));
                    if e.action.is_some() {
                        let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
                        self.hit(
                            a,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index: index + 1,
                            }),
                        );
                    }
                }
            }
            Node::Button(b) => {
                let bw = self.button(
                    &b.label,
                    r.x + 14.0,
                    r.y + 4.0,
                    selected && sv.focused,
                    true,
                );
                if let Some(index) = target {
                    self.hit(
                        Rect::new(r.x + 14.0, r.y + 4.0, bw, 30.0),
                        Hit::Ui(UiEvent::Target { side, index }),
                    );
                }
            }
            Node::Buttons(bs) => {
                let mut x = r.x + 14.0;
                for (k, b) in bs.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let bw = self.button(&b.label, x, r.y + 4.0, focused, k == 0);
                    if let Some(index) = index {
                        self.hit(
                            Rect::new(x, r.y + 4.0, bw, 30.0),
                            Hit::Ui(UiEvent::Target { side, index }),
                        );
                    }
                    x += bw + 8.0;
                }
            }
            Node::Input {
                id,
                placeholder,
                secret,
            } => {
                let b = Rect::new(r.x + 10.0, r.y + 5.0, r.w - 20.0, 30.0);
                let focused = selected && sv.focused;
                let border = if focused { p.accent } else { p.border };
                self.canvas.outline(b, 7.0, 1.0, border, p.raised);
                let typed = sv
                    .inputs
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or_default();
                let shown = if *secret {
                    "•".repeat(typed.chars().count())
                } else {
                    typed.to_string()
                };
                let face = Face::ui(13.0);
                let (label, colour) = if shown.is_empty() {
                    (placeholder.as_str(), p.muted)
                } else {
                    (shown.as_str(), p.text)
                };
                let tw = self.text.draw_in_row(
                    &mut self.canvas,
                    label,
                    face,
                    b.x + 10.0,
                    b.y,
                    b.h,
                    colour,
                    b.inset(1.0),
                );
                if focused {
                    let cx = if shown.is_empty() {
                        b.x + 10.0
                    } else {
                        b.x + 10.0 + tw
                    };
                    self.canvas
                        .fill(Rect::new(cx, b.y + 7.0, 1.5, b.h - 14.0), p.accent);
                }
                if let Some(index) = target {
                    self.hit(b, Hit::Ui(UiEvent::Target { side, index }));
                }
            }
            Node::Divider => {
                self.canvas
                    .fill(Rect::new(r.x + 14.0, r.y + 6.0, r.w - 28.0, 1.0), p.line);
            }
            Node::Footer => {}
        }
    }

    /// A tree row: indent, arrow, icon, name, detail, and a badge at the end.
    fn entry(&mut self, e: &Entry, row: Rect, clip: Rect) {
        let p = self.p;
        let mut x = row.x + 8.0 + e.depth as f32 * 14.0;
        let mid = row.y + row.h / 2.0;
        // Folders have an open or closed arrow; files a blank in its place.
        let folder = match e.twisty {
            Some(t @ ('▾' | '▸' | '\u{eab4}' | '\u{eab6}')) => {
                let open = matches!(t, '▾' | '\u{eab4}');
                self.canvas.chevron(x + 5.0, mid, 7.0, open, p.muted);
                x += 16.0;
                true
            }
            Some(_) => {
                x += 16.0;
                false
            }
            None => false,
        };
        if let Some((glyph, hue)) = e.icon {
            let ext = e
                .label
                .rsplit_once('.')
                .map(|(_, ext)| ext)
                .filter(|ext| !ext.is_empty() && ext.len() <= 4 && !folder);
            let colour = theme::hue(&p, hue);
            match ext {
                Some(ext) => self.badge(ext, x, mid - 8.0, 16.0, colour),
                None => {
                    let g = glyph.to_string();
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &g,
                        Face::mono(13.0),
                        x,
                        row.y,
                        row.h,
                        colour,
                        clip,
                    );
                }
            }
            x += 22.0;
        }
        let face = Face::ui(13.0);
        let badge_w = if e.badge.is_empty() {
            0.0
        } else {
            self.text.width(&e.badge, Face::ui(11.0).bold()) + 14.0
        };
        let room = (row.right() - x - badge_w - if e.action.is_some() { 26.0 } else { 0.0 } - 6.0)
            .max(0.0);
        let label = self.text.fit(&e.label, face, room);
        let colour = match e.tone {
            Tone::Normal => p.text,
            tone => tone_colour(&p, tone),
        };
        let lw = self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            row.y,
            row.h,
            colour,
            clip,
        );
        if !e.detail.is_empty() && lw + 12.0 < room {
            let detail = self.text.fit(&e.detail, Face::ui(12.0), room - lw - 8.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &detail,
                Face::ui(12.0),
                x + lw + 8.0,
                row.y,
                row.h,
                p.muted,
                clip,
            );
        }
        let mut right = row.right() - 8.0;
        if !e.badge.is_empty() {
            let face = Face::ui(11.0).bold();
            let bw = self.text.width(&e.badge, face);
            let colour = tone_colour(&p, e.badge_tone);
            if e.badge_tone == Tone::Badge {
                let pill = Rect::new(right - bw - 10.0, mid - 8.0, bw + 10.0, 16.0);
                self.canvas.round(pill, 8.0, p.chip);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &e.badge,
                    face,
                    pill.x + 5.0,
                    pill.y,
                    16.0,
                    p.text,
                    clip,
                );
            } else {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &e.badge,
                    face,
                    right - bw,
                    row.y,
                    row.h,
                    colour,
                    clip,
                );
            }
            right -= bw + 14.0;
        }
        if let Some((glyph, _)) = &e.action
            && self.hovered(row)
        {
            let a = Rect::new(right - 20.0, row.y + 3.0, 20.0, 20.0);
            self.canvas.round(a, 5.0, p.chip);
            let g = glyph.to_string();
            let gw = self.text.width(&g, Face::mono(13.0));
            self.text.draw_in_row(
                &mut self.canvas,
                &g,
                Face::mono(13.0),
                a.x + (20.0 - gw) / 2.0,
                a.y,
                20.0,
                p.bright,
                clip,
            );
        }
    }

    /// A file type's letters in a small rounded square of its colour.
    fn badge(&mut self, ext: &str, x: f32, y: f32, size: f32, colour: u32) {
        let p = self.p;
        let r = Rect::new(x, y, size, size);
        self.canvas.round(r, 4.0, mix(colour, p.island, 0.22));
        let label: String = ext.chars().take(2).collect::<String>().to_uppercase();
        let face = Face::ui(size * 0.5).bold();
        let lw = self.text.width(&label, face);
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x + (size - lw) / 2.0,
            y,
            size,
            colour,
            r,
        );
    }

    fn button(&mut self, label: &str, x: f32, y: f32, focused: bool, primary: bool) -> f32 {
        let p = self.p;
        let face = Face::ui(13.0).medium();
        let tw = self.text.width(label, face);
        let r = Rect::new(x, y, tw + 28.0, 30.0);
        let hovered = self.hovered(r);
        let (bg, fg) = if primary {
            (
                if hovered {
                    mix(p.accent, 0xffffff, 0.88)
                } else {
                    p.accent
                },
                p.ground,
            )
        } else {
            (if hovered { p.border } else { p.chip }, p.bright)
        };
        if focused {
            self.canvas.outline(
                Rect::new(x - 2.0, y - 2.0, r.w + 4.0, 34.0),
                9.0,
                1.5,
                p.accent,
                p.island,
            );
        }
        self.canvas.round(r, 7.0, bg);
        self.text
            .draw_in_row(&mut self.canvas, label, face, x + 14.0, y, 30.0, fg, r);
        r.w
    }

    fn tabs(&mut self, view: &WindowView, editor: Rect) {
        let p = self.p;
        let row = Rect::new(editor.x, editor.y, editor.w, TABS_H);
        self.canvas.fill(
            Rect::new(editor.x, editor.y + TABS_H - 1.0, editor.w, 1.0),
            p.line,
        );
        let mut x = editor.x + 6.0;
        let end = editor.right() - 70.0;
        for (i, tab) in view.tabs.iter().enumerate() {
            let face = if tab.preview {
                Face::ui(13.0).italic()
            } else {
                Face::ui(13.0)
            };
            let nw = self.text.width(&tab.name, face);
            let w = 12.0 + 14.0 + 7.0 + nw + 7.0 + 16.0 + 10.0;
            if x + w > end && i > 0 {
                break;
            }
            let r = Rect::new(x, editor.y, w, TABS_H - 1.0);
            let hovered = self.hovered(r);
            if tab.active {
                self.canvas.fill(r, p.raised);
                self.canvas.round(
                    Rect::new(r.x + 8.0, r.bottom() - 1.0, r.w - 16.0, 2.0),
                    1.0,
                    p.accent,
                );
            } else if hovered {
                self.canvas.fill(r, p.hover);
            }
            let ext = tab.name.rsplit_once('.').map_or("", |(_, e)| e);
            let colour = theme::hue(&p, tab.icon.1);
            if ext.is_empty() {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &tab.icon.0.to_string(),
                    Face::mono(12.0),
                    x + 12.0,
                    r.y,
                    r.h,
                    colour,
                    row,
                );
            } else {
                self.badge(ext, x + 12.0, r.y + (r.h - 14.0) / 2.0, 14.0, colour);
            }
            let fg = if tab.active { p.bright } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                &tab.name,
                face,
                x + 33.0,
                r.y,
                r.h,
                fg,
                row,
            );
            let close = Rect::new(x + 33.0 + nw + 5.0, r.y + (r.h - 18.0) / 2.0, 18.0, 18.0);
            let mark = if tab.dirty && !self.hovered(close) {
                Some('●')
            } else if tab.active || hovered {
                Some('×')
            } else {
                None
            };
            if let Some(mark) = mark {
                if mark == '×' && self.hovered(close) {
                    self.canvas.round(close, 4.0, p.chip);
                }
                let face = Face::ui(if mark == '●' { 10.0 } else { 14.0 });
                let mw = self.text.width(&mark.to_string(), face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &mark.to_string(),
                    face,
                    close.x + (18.0 - mw) / 2.0,
                    close.y,
                    18.0,
                    p.muted,
                    row,
                );
            }
            self.hit(
                r,
                Hit::Ui(UiEvent::Tab {
                    index: i,
                    clicks: 1,
                }),
            );
            self.hit(close, Hit::Ui(UiEvent::CloseTab(i)));
            x += w + 2.0;
        }
        // Split the editor, at the right end.
        self.icon_button(
            Rect::new(editor.right() - 66.0, editor.y + 5.0, 28.0, 28.0),
            '\u{eb56}',
            false,
            Hit::Command("view.split"),
        );
        self.icon_button(
            Rect::new(editor.right() - 34.0, editor.y + 5.0, 28.0, 28.0),
            '\u{ea7c}',
            false,
            Hit::Command("palette.open"),
        );
    }

    fn crumbs(&mut self, view: &WindowView, editor: Rect) {
        let p = self.p;
        let row = Rect::new(editor.x, editor.y + TABS_H, editor.w, CRUMBS_H);
        let face = Face::ui(12.0);
        let mut x = editor.x + 16.0;
        let last = view.crumbs.len().saturating_sub(1);
        for (i, crumb) in view.crumbs.iter().enumerate() {
            let tw = self.text.width(crumb, face);
            let r = Rect::new(x - 3.0, row.y + 4.0, tw + 6.0, CRUMBS_H - 8.0);
            let hovered = self.hovered(r);
            if hovered {
                self.canvas.round(r, 4.0, p.hover);
            }
            let colour = if i == last || hovered {
                p.text
            } else {
                p.muted
            };
            self.text
                .draw_in_row(&mut self.canvas, crumb, face, x, row.y, row.h, colour, row);
            self.hit(r, Hit::Ui(UiEvent::Crumb(i)));
            x += tw;
            if i < last {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "›",
                    face,
                    x + 6.0,
                    row.y,
                    row.h,
                    p.muted,
                    row,
                );
                x += 20.0;
            }
            if x > row.right() - 20.0 {
                break;
            }
        }
    }

    fn status(&mut self, view: &WindowView, w: f32, h: f32) {
        let p = self.p;
        let bar = Rect::new(0.0, h - STATUS_H, w, STATUS_H);
        let face = Face::ui(12.0);
        let mut x = RAIL_W + 12.0;
        let mut left_end = x;
        let mut right_items = Vec::new();
        for (i, item) in view.status.iter().enumerate() {
            if item.right {
                right_items.push((i, item));
                continue;
            }
            let tw = self.text.width(&item.text, face);
            let r = Rect::new(x - 6.0, bar.y + 3.0, tw + 12.0, STATUS_H - 6.0);
            if item.clickable && self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let colour = if item.accent { p.accent } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.text,
                face,
                x,
                bar.y,
                STATUS_H,
                colour,
                bar,
            );
            if item.clickable {
                self.hit(r, Hit::Ui(UiEvent::Status(i)));
            }
            x += tw + 18.0;
            left_end = x;
        }
        let mut rx = w - 14.0;
        for (i, item) in right_items.into_iter().rev() {
            let tw = self.text.width(&item.text, face);
            rx -= tw;
            let r = Rect::new(rx - 6.0, bar.y + 3.0, tw + 12.0, STATUS_H - 6.0);
            if item.clickable && self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let last = i + 1 == view.status.len();
            let colour = if last { p.chip_text } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.text,
                face,
                rx,
                bar.y,
                STATUS_H,
                colour,
                bar,
            );
            if item.clickable {
                self.hit(r, Hit::Ui(UiEvent::Status(i)));
            }
            rx -= 18.0;
        }
        if let Some((message, error)) = &view.problem {
            let room = rx - left_end - 12.0;
            if room > 60.0 {
                let text = self.text.fit(message, face, room);
                let colour = if *error { p.error } else { p.warning };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    face,
                    left_end + 6.0,
                    bar.y,
                    STATUS_H,
                    colour,
                    bar,
                );
            }
        }
    }

    fn toast(&mut self, toast: &ToastView, editor: Rect) {
        let p = self.p;
        let face = Face::ui(13.0);
        let w = 420.0_f32.min(editor.w - 32.0);
        let text_w = w - 56.0;
        let th = self.text.wrapped_height(&toast.text, face, text_w);
        let h = th + 28.0 + if toast.buttons.is_empty() { 0.0 } else { 40.0 };
        let r = Rect::new(editor.right() - w - 16.0, editor.bottom() - h - 16.0, w, h);
        self.canvas.shadow(r, 10.0, theme::is_light(&p));
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        // Later hits win, so the box goes under its buttons.
        self.hit(r, Hit::Block);
        let colour = if toast.error { p.error } else { p.accent };
        let dot = Rect::new(r.x + 16.0, r.y + 17.0, 10.0, 10.0);
        self.canvas.round(dot, 5.0, colour);
        self.text.draw(
            &mut self.canvas,
            &toast.text,
            face,
            r.x + 38.0,
            r.y + 12.0,
            p.text,
            r,
            Some(text_w),
        );
        let mut bx = r.right() - 14.0;
        for (i, label) in toast.buttons.iter().enumerate().rev() {
            let bf = Face::ui(12.5).medium();
            let bw = self.text.width(label, bf) + 24.0;
            bx -= bw;
            let b = Rect::new(bx, r.bottom() - 40.0, bw, 28.0);
            let primary = i + 1 == toast.buttons.len();
            let hovered = self.hovered(b);
            let (bg, fg) = if primary {
                (
                    if hovered {
                        mix(p.accent, 0xffffff, 0.88)
                    } else {
                        p.accent
                    },
                    p.ground,
                )
            } else {
                (if hovered { p.border } else { p.chip }, p.bright)
            };
            self.canvas.round(b, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, bf, b.x + 12.0, b.y, 28.0, fg, b);
            self.hit(b, Hit::Ui(UiEvent::ToastButton(i)));
            bx -= 8.0;
        }
    }

    fn palette(&mut self, pv: &PaletteView, w: f32, h: f32) {
        let p = self.p;
        self.canvas
            .tint(Rect::new(0.0, 0.0, w, h), 0.0, p.ground, 140);
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::ClosePalette));
        let pw = 640.0_f32.min(w - 32.0);
        let shown = pv.items.len().clamp(1, PALETTE_ROWS);
        let list_h = shown as f32 * 34.0 + 12.0;
        let ph = 48.0 + list_h + 34.0;
        let r = Rect::new((w - pw) / 2.0, 56.0, pw, ph);
        self.canvas.shadow(r, 12.0, theme::is_light(&p));
        self.canvas.outline(r, 12.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);

        // The query, with what's being searched.
        let input = Rect::new(r.x, r.y, r.w, 48.0);
        self.text.draw_in_row(
            &mut self.canvas,
            "\u{ea6d}",
            Face::mono(15.0),
            r.x + 16.0,
            r.y,
            48.0,
            p.muted,
            input,
        );
        let mut qx = r.x + 42.0;
        if let Some(title) = &pv.title {
            let face = Face::ui(14.0).medium();
            qx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("{title}:"),
                face,
                qx,
                r.y,
                48.0,
                p.muted,
                input,
            ) + 8.0;
        }
        let face = Face::ui(15.0);
        let qw = self.text.draw_in_row(
            &mut self.canvas,
            &pv.query,
            face,
            qx,
            r.y,
            48.0,
            p.bright,
            input,
        );
        self.canvas
            .fill(Rect::new(qx + qw + 1.0, r.y + 14.0, 1.5, 20.0), p.accent);
        self.chip("Esc", r.right() - 14.0, r.y + 16.0, true);
        let kind_w = self.text.width(pv.kind, Face::ui(11.0)) + 12.0;
        self.chip(pv.kind, r.right() - 14.0 - 40.0 - kind_w, r.y + 16.0, false);
        self.canvas
            .fill(Rect::new(r.x, r.y + 48.0, r.w, 1.0), p.chip);

        // The items, scrolled to keep the selection in view.
        let first = pv.selected.saturating_sub(PALETTE_ROWS - 1);
        let list = Rect::new(r.x, r.y + 49.0, r.w, list_h);
        if pv.items.is_empty() {
            self.text.draw_in_row(
                &mut self.canvas,
                "Nothing matches",
                Face::ui(13.0),
                r.x + 20.0,
                list.y + 6.0,
                34.0,
                p.muted,
                list,
            );
        }
        for (k, (label, key)) in pv.items.iter().enumerate().skip(first).take(PALETTE_ROWS) {
            let y = list.y + 6.0 + (k - first) as f32 * 34.0;
            let row = Rect::new(r.x + 6.0, y, r.w - 12.0, 34.0);
            let on = k == pv.selected;
            if on {
                self.canvas.round(row, 7.0, p.selected);
            } else if self.hovered(row) {
                self.canvas.round(row, 7.0, p.hover);
            }
            self.matched(label, &pv.query, row.x + 12.0, row, row.w - 140.0);
            if let Some(key) = key {
                self.chip(key, row.right() - 10.0, y + 9.0, true);
            }
            self.hit(row, Hit::Ui(UiEvent::PaletteRow(k)));
        }

        // How to use it.
        let foot = Rect::new(r.x, r.bottom() - 34.0, r.w, 34.0);
        self.canvas
            .fill(Rect::new(r.x + 1.0, foot.y, r.w - 2.0, 1.0), p.chip);
        let mut fx = r.x + 16.0;
        for (keys, what) in [("↑ ↓", "to move"), ("Enter", "to run"), ("Esc", "to close")] {
            let f = Face::ui(11.5);
            fx += self.text.draw_in_row(
                &mut self.canvas,
                keys,
                f,
                fx,
                foot.y,
                34.0,
                p.chip_text,
                foot,
            ) + 5.0;
            fx += self
                .text
                .draw_in_row(&mut self.canvas, what, f, fx, foot.y, 34.0, p.muted, foot)
                + 18.0;
        }
    }

    /// A label with the letters of `query` it matches in the accent.
    fn matched(&mut self, label: &str, query: &str, x: f32, row: Rect, max: f32) {
        let p = self.p;
        let face = Face::ui(13.5);
        let strong = Face::ui(13.5).bold();
        let label = self.text.fit(label, face, max);
        let mut wanted = query
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .peekable();
        let mut x = x;
        let mut run = String::new();
        let mut run_match = false;
        let flush = |frame: &mut Self, run: &mut String, matched: bool, x: &mut f32| {
            if run.is_empty() {
                return;
            }
            let (f, c) = if matched {
                (strong, p.accent)
            } else {
                (face, p.text)
            };
            *x += frame
                .text
                .draw_in_row(&mut frame.canvas, run, f, *x, row.y, row.h, c, row);
            run.clear();
        };
        for c in label.chars() {
            let hit = wanted
                .peek()
                .is_some_and(|w| c.to_lowercase().next() == Some(*w));
            if hit {
                wanted.next();
            }
            if hit != run_match {
                flush(self, &mut run, run_match, &mut x);
                run_match = hit;
            }
            run.push(c);
        }
        flush(self, &mut run, run_match, &mut x);
    }
}

fn tone_colour(p: &Palette, tone: Tone) -> u32 {
    match tone {
        Tone::Normal => p.text,
        Tone::Muted => p.muted,
        Tone::Strong => p.bright,
        Tone::Warning => p.warning,
        Tone::Error => p.error,
        Tone::Success => p.success,
        Tone::Badge => p.text,
    }
}

/// `EXPLORER` as `Explorer`; titles already in mixed case stay.
fn title_case(title: &str) -> String {
    if title.chars().any(|c| c.is_lowercase()) {
        return title.to_string();
    }
    let mut out = String::new();
    let mut start = true;
    for c in title.chars() {
        if start {
            out.push(c);
        } else {
            out.extend(c.to_lowercase());
        }
        start = !c.is_alphanumeric();
    }
    out
}

/// Markdown as plain text, for panels until it gets its own drawing.
fn plain(markdown: &str) -> String {
    markdown
        .lines()
        .map(|l| l.trim_start_matches('#').trim_start())
        .filter(|l| !l.starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("**", "")
        .replace('`', "")
}
