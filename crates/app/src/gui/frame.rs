//! The window's frame, laid out and drawn in pixels as the design has it:
//! a slim title bar, the activity rail, the side bar, editor and panel as
//! rounded islands on a darker ground, and a thin status bar. Drawing also
//! records what each clickable part does.

use merlin_core::panel::{Entry, Node, Tone};
use merlin_core::{
    ContextItem, ContextView, DialogView, FindAction, FindView, Key, KeyCode, MenuView,
    PaletteView, PopupKind, PopupView, Side, SideView, TabView, ToastView, UiEvent, WindowView,
    editor::GroupView,
};

use super::canvas::{Canvas, Face, Pictures, Rect, Text};
use super::scene::Prim;
use crate::packs::{Fallback, Icon, Icons, Tint};
use crate::theme::{self, Palette, mix};

const TITLE_H: f32 = 40.0;
const STATUS_H: f32 = 26.0;
const RAIL_W: f32 = 44.0;
const SIDE_W: f32 = 260.0;
const RIGHT_W: f32 = 320.0;
const BOTTOM_H: f32 = 240.0;
const TABS_H: f32 = 32.0;
/// A tab: how far it sits in from the strip's top and bottom, its corners,
/// the room before its icon, a pinned one's width, and the gap between.
const TAB_INSET: f32 = 3.0;
const TAB_RADIUS: f32 = 6.0;
const TAB_PAD: f32 = 10.0;
const PINNED_W: f32 = 36.0;
const TAB_GAP: f32 = 3.0;
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
    /// The edge between a side bar or the panel and the code, to drag.
    Sash(Side),
    /// A tab strip, past its tabs: a dragged tab dropped there goes at the
    /// end, of group `n` when there are several.
    TabStrip(Option<usize>),
    /// An edge between editor groups, to drag.
    GroupSash(GroupSash),
    /// A group's scrollbar: down the side, or along the bottom.
    Scrollbar {
        group: usize,
        horizontal: bool,
    },
    /// A group's minimap.
    Minimap(usize),
}

/// An edge between editor groups: after column `n`, or under group `n` in
/// its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupSash {
    Column(usize),
    Row(usize),
}

/// Where an editor group's parts go, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GroupArea {
    pub tabs: Rect,
    pub crumbs: Option<Rect>,
    /// The code: gutter and text.
    pub text: Rect,
    /// Right of the code: the minimap, when it's on, then the scrollbar.
    pub minimap: Option<Rect>,
    pub bar: Rect,
}

/// The line between groups, and the room a sash takes on either side.
pub const DIVIDER: f32 = 1.0;
const SASH: f32 = 3.0;

/// Where the islands are, in logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Areas {
    pub left: Option<Rect>,
    pub right: Option<Rect>,
    pub bottom: Option<Rect>,
    pub editor: Rect,
    /// Each editor group's parts, in the editor island.
    pub groups: Vec<GroupArea>,
    /// The edges between groups.
    pub group_sashes: Vec<(Rect, GroupSash)>,
    /// The focused group's code, for what floats over it.
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
    pub icons: &'a Icons,
    pub pictures: &'a mut Pictures,
    /// Rows a little shorter, for more on screen.
    pub compact: bool,
    /// The name and icon of a tab or file being dragged.
    pub tab_drag: Option<(String, Option<(char, merlin_core::style::Hue)>)>,
    /// Where it would land, shaded.
    pub drop_zone: Option<Rect>,
    /// Each group's tab strip: how far it's scrolled, and the active tab
    /// it last scrolled to. Drawing keeps them in range.
    pub strips: Vec<(f32, usize)>,
    /// The side edges' hits, added after the code's so they win.
    pub sash_hits: Vec<(Rect, Hit)>,
    /// What each icon is, for a tooltip when the mouse rests on it.
    pub tips: Vec<(Rect, String)>,
    /// The tooltip showing, under the icon it's for.
    pub tip: Option<(Rect, String)>,
}

/// How wide the side bars are and how tall the panel is, as the user last
/// dragged them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    /// The scrollbar's width, and the minimap's scale, from the settings.
    pub scrollbar: f32,
    pub minimap_scale: f32,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            left: SIDE_W,
            right: RIGHT_W,
            bottom: BOTTOM_H,
            scrollbar: super::overview::SCROLLBAR_W,
            minimap_scale: 1.0,
        }
    }
}

impl Sizes {
    /// The size `side` takes with its edge dragged to (`x`, `y`) in a
    /// window `w` by `h`, kept to what leaves the code room.
    pub fn drag(&mut self, side: Side, (x, y): (f32, f32), w: f32, h: f32) {
        match side {
            Side::Left => self.left = (x - RAIL_W).clamp(160.0, w * 0.5),
            Side::Right => self.right = (w - x).clamp(200.0, w * 0.5),
            Side::Bottom => {
                let body = h - STATUS_H - TITLE_H;
                self.bottom = (h - STATUS_H - y).clamp(100.0, body * 0.75);
            }
        }
    }
}

pub fn areas(view: &WindowView, w: f32, h: f32, gap: f32, sizes: &Sizes) -> Areas {
    let top = TITLE_H;
    let bottom_y = h - STATUS_H;
    let body_h = (bottom_y - top).max(0.0);
    let mut x = RAIL_W;
    let left = view.left.as_ref().map(|_| {
        let r = Rect::new(x, top, sizes.left.min(w * 0.5), body_h);
        x += r.w + gap;
        r
    });
    let mut x1 = w - gap;
    let right = view.right.as_ref().map(|_| {
        let rw = sizes.right.min(w * 0.5);
        x1 -= rw;
        let r = Rect::new(x1, top, rw, body_h);
        x1 -= gap;
        r
    });
    let main_w = (x1 - x).max(0.0);
    let bottom = view.bottom.as_ref().map(|_| {
        let bh = sizes.bottom.min(body_h * 0.75);
        Rect::new(x, bottom_y - bh, main_w, bh)
    });
    let editor_h = body_h - bottom.map_or(0.0, |b| b.h + gap);
    let editor = Rect::new(x, top, main_w, editor_h.max(0.0));
    let (groups, group_sashes) = group_areas(editor, &view.groups, view.minimap, sizes);
    let text = groups
        .iter()
        .zip(&view.groups)
        .find(|(_, g)| g.focused)
        .or_else(|| groups.iter().zip(&view.groups).next())
        .map_or(editor, |(a, _)| a.text);
    Areas {
        left,
        right,
        bottom,
        editor,
        groups,
        group_sashes,
        text,
    }
}

/// The editor groups' places in `editor`: columns left to right by their
/// shares of the width, each column's groups top to bottom by theirs of
/// its height, a divider between each.
pub fn group_areas(
    editor: Rect,
    groups: &[GroupView],
    minimap: bool,
    sizes: &Sizes,
) -> (Vec<GroupArea>, Vec<(Rect, GroupSash)>) {
    let n = groups.len().max(1);
    let heads: Vec<usize> = (0..groups.len())
        .filter(|&g| g == 0 || !groups[g].below)
        .collect();
    let heads = if heads.is_empty() { vec![0] } else { heads };
    let share = |g: usize, wide: bool| {
        groups
            .get(g)
            .map_or(1.0, |v| if wide { v.width } else { v.height })
    };
    let total_w: f32 = heads.iter().map(|&h| share(h, true)).sum();
    let room_w = (editor.w - (heads.len() - 1) as f32 * DIVIDER).max(0.0);
    let mut areas = Vec::with_capacity(n);
    let mut sashes = Vec::new();
    let mut x = editor.x;
    for (c, &head) in heads.iter().enumerate() {
        let end = heads.get(c + 1).copied().unwrap_or(n);
        let last_column = c + 1 == heads.len();
        let w = if last_column {
            editor.right() - x
        } else {
            room_w * share(head, true) / total_w
        };
        let total_h: f32 = (head..end).map(|g| share(g, false)).sum();
        let room_h = (editor.h - (end - head - 1) as f32 * DIVIDER).max(0.0);
        let mut y = editor.y;
        for g in head..end {
            let h = if g + 1 == end {
                editor.bottom() - y
            } else {
                room_h * share(g, false) / total_h
            };
            let crumbs = groups.get(g).is_some_and(|v| !v.crumbs.is_empty());
            areas.push(GroupArea::of(Rect::new(x, y, w, h), crumbs, minimap, sizes));
            if g + 1 < end {
                let r = Rect::new(x, y + h - SASH, w, DIVIDER + 2.0 * SASH);
                sashes.push((r, GroupSash::Row(g)));
            }
            y += h + DIVIDER;
        }
        if !last_column {
            let r = Rect::new(x + w - SASH, editor.y, DIVIDER + 2.0 * SASH, editor.h);
            sashes.push((r, GroupSash::Column(c)));
        }
        x += w + DIVIDER;
    }
    (areas, sashes)
}

impl GroupArea {
    /// The group under its tabs: breadcrumbs, code, minimap and
    /// scrollbar, where a dragged tab or file can land.
    pub fn body(&self) -> Rect {
        let top = self.crumbs.map_or(self.text.y, |c| c.y);
        Rect::new(
            self.text.x,
            top,
            self.bar.right() - self.text.x,
            self.text.bottom() - top,
        )
    }

    /// A group's tabs on top, then its breadcrumbs, then its code with
    /// the minimap and scrollbar on its right. A narrow group drops the
    /// minimap first.
    fn of(r: Rect, crumbs: bool, minimap: bool, sizes: &Sizes) -> Self {
        let head = if crumbs { TABS_H + CRUMBS_H } else { TABS_H };
        let (y, h) = (r.y + head + 4.0, (r.h - head - 4.0).max(0.0));
        let bar_w = sizes.scrollbar;
        let map_w = super::overview::MINIMAP_W * sizes.minimap_scale;
        let bar = Rect::new(r.right() - bar_w, y, bar_w, h);
        let minimap = (minimap && r.w > 3.0 * map_w).then(|| Rect::new(bar.x - map_w, y, map_w, h));
        let code_w = (minimap.map_or(bar.x, |m| m.x) - r.x).max(0.0);
        Self {
            tabs: Rect::new(r.x, r.y, r.w, TABS_H.min(r.h)),
            crumbs: crumbs.then(|| Rect::new(r.x, r.y + TABS_H, r.w, CRUMBS_H)),
            text: Rect::new(r.x, y, code_w, h),
            minimap,
            bar,
        }
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
        // The edges that resize the sides, lit while the mouse is on them.
        let gap = (a.editor.x - a.left.map_or(a.editor.x, |l| l.right())).max(4.0);
        let sashes = [
            a.left
                .map(|l| (Side::Left, Rect::new(l.right() - 2.0, l.y, gap + 4.0, l.h))),
            a.right
                .map(|r| (Side::Right, Rect::new(r.x - gap - 2.0, r.y, gap + 4.0, r.h))),
            a.bottom.map(|b| {
                (
                    Side::Bottom,
                    Rect::new(b.x, b.y - gap - 2.0, b.w, gap + 4.0),
                )
            }),
        ];
        for (side, r) in sashes.into_iter().flatten() {
            if self.hovered(r) {
                let line = match side {
                    Side::Bottom => Rect::new(r.x, r.y + r.h / 2.0 - 1.0, r.w, 2.0),
                    _ => Rect::new(r.x + r.w / 2.0 - 1.0, r.y, 2.0, r.h),
                };
                self.canvas.fill(line, p.accent);
            }
            self.sash_hits.push((r, Hit::Sash(side)));
        }
        for &(r, sash) in &a.group_sashes {
            self.sash_hits.push((r, Hit::GroupSash(sash)));
        }
        for group in &a.groups {
            self.hit(group.text, Hit::Text);
        }
        // Over the code's edge, the sash wins.
        let sashes = std::mem::take(&mut self.sash_hits);
        self.hits.extend(sashes);
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
        if let Some(dialog) = &view.dialog {
            self.dialog(dialog, w, h);
        }
    }

    /// A question over the dimmed window: what it's about, why it
    /// matters, and the buttons, the first the main one, right-aligned as
    /// a desktop's dialogs have them. Nothing else takes a click.
    fn dialog(&mut self, dv: &DialogView, w: f32, h: f32) {
        let p = self.p;
        let all = Rect::new(0.0, 0.0, w, h);
        self.canvas.tint(all, 0.0, p.ground, 150);
        self.hit(all, Hit::Block);
        let dw = 460.0_f32.min(w - 32.0);
        let inner = dw - 48.0;
        let title_face = Face::ui(15.0).medium();
        let detail_face = Face::ui(13.0);
        let title_h = self.text.wrapped_height(&dv.title, title_face, inner);
        let detail_h = self.text.wrapped_height(&dv.detail, detail_face, inner);
        let dh = 24.0 + title_h + 10.0 + detail_h + 28.0 + 30.0 + 22.0;
        let r = Rect::new((w - dw) / 2.0, ((h - dh) / 2.0 - 40.0).max(16.0), dw, dh);
        self.canvas.shadow(r, 12.0, theme::is_light(&p));
        self.canvas.outline(r, 12.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let (x, mut y) = (r.x + 24.0, r.y + 24.0);
        self.text.draw(
            &mut self.canvas,
            &dv.title,
            title_face,
            x,
            y,
            p.bright,
            r,
            Some(inner),
        );
        y += title_h + 10.0;
        self.text.draw(
            &mut self.canvas,
            &dv.detail,
            detail_face,
            x,
            y,
            p.muted,
            r,
            Some(inner),
        );
        // The buttons, right to left from the edge, so the main one ends
        // up on the left of the row as GNOME and VS Code put it.
        let face = Face::ui(13.0).medium();
        let widths: Vec<f32> = dv
            .buttons
            .iter()
            .map(|b| self.text.width(b, face) + 28.0)
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0 * (widths.len().saturating_sub(1)) as f32;
        let mut bx = r.right() - 24.0 - total;
        let by = r.bottom() - 22.0 - 30.0;
        for (i, label) in dv.buttons.iter().enumerate() {
            let bw = self.button(label, bx, by, i == dv.selected, i == dv.primary);
            self.hit(
                Rect::new(bx, by, bw, 30.0),
                Hit::Ui(UiEvent::DialogButton(i)),
            );
            bx += bw + 8.0;
        }
    }

    /// What floats over the code: the right-click menu where it was
    /// opened, and hover or completion by the text they're about. `cell`
    /// is a cell of the code, in logical pixels.
    pub fn floating(
        &mut self,
        view: &WindowView,
        a: &Areas,
        cell: (f32, f32),
        context_at: Option<(f32, f32)>,
        w: f32,
        h: f32,
    ) {
        self.group_heads(view, a);
        if let Some(find) = &view.find {
            self.find(find, a);
        }
        if let Some(popup) = &view.popup {
            self.popup(popup, a, cell);
        }
        if let (Some(menu), Some(at)) = (&view.context, context_at) {
            self.context(menu, at, w, h);
        }
        if let Some(menu) = &view.menu {
            self.menu(menu, w, h);
        }
        if let Some((r, tip)) = self.tip.clone() {
            self.tooltip(r, &tip, w);
        }
    }

    /// A small label under `r`, or beside it on the rail.
    fn tooltip(&mut self, r: Rect, tip: &str, w: f32) {
        let p = self.p;
        let face = Face::ui(12.0);
        let tw = self.text.width(tip, face) + 16.0;
        let (x, y) = if r.x < RAIL_W {
            (r.right() + 6.0, r.y + (r.h - 24.0) / 2.0)
        } else {
            (
                (r.x + r.w / 2.0 - tw / 2.0).clamp(6.0, w - tw - 6.0),
                r.bottom() + 6.0,
            )
        };
        let b = Rect::new(x, y, tw, 24.0);
        self.canvas.shadow(b, 5.0, theme::is_light(&p));
        self.canvas.outline(b, 5.0, 1.0, p.border, p.widget);
        self.text
            .draw_in_row(&mut self.canvas, tip, face, b.x + 8.0, b.y, b.h, p.text, b);
    }

    /// The menus behind the title bar's button: their titles in one card,
    /// and the open one's items in another beside it.
    fn menu(&mut self, menu: &MenuView, w: f32, h: f32) {
        let p = self.p;
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::CloseMenu));
        let row_h = 28.0;
        let titles = Rect::new(
            10.0,
            TITLE_H - 4.0,
            180.0,
            menu.titles.len() as f32 * row_h + 10.0,
        );
        self.canvas.shadow(titles, 8.0, theme::is_light(&p));
        self.canvas.outline(titles, 8.0, 1.0, p.border, p.widget);
        self.hit(titles, Hit::Block);
        let face = Face::ui(13.0);
        let mut open_y = titles.y;
        for (i, title) in menu.titles.iter().enumerate() {
            let row = Rect::new(
                titles.x + 5.0,
                titles.y + 5.0 + i as f32 * row_h,
                titles.w - 10.0,
                row_h,
            );
            let open = i == menu.open;
            if open {
                self.canvas.round(row, 5.0, p.focus_row);
                open_y = row.y - 5.0;
            } else if self.hovered(row) {
                self.canvas.round(row, 5.0, p.hover);
            }
            let colour = if open { p.bright } else { p.text };
            self.text.draw_in_row(
                &mut self.canvas,
                title,
                face,
                row.x + 12.0,
                row.y,
                row_h,
                colour,
                row,
            );
            self.canvas
                .chevron(row.right() - 14.0, row.y + row_h / 2.0, 7.0, false, p.muted);
            self.hit(row, Hit::Ui(UiEvent::MenuTitle(i)));
        }
        let at = (titles.right() + 4.0, open_y);
        self.items_card(&menu.items, menu.selected, at, w, h, UiEvent::MenuItem);
    }

    /// The find box at the top right of the code, as VS Code has it: the
    /// query with its count and arrows, and the replacement below it.
    fn find(&mut self, find: &FindView, a: &Areas) {
        let p = self.p;
        let rows = 1.0 + find.replace.is_some() as u8 as f32;
        let bw = 440.0_f32.min(a.text.w - 24.0);
        if bw < 200.0 {
            return;
        }
        let r = Rect::new(
            a.text.right() - bw - 22.0,
            a.text.y + 4.0,
            bw,
            rows * 34.0 + 8.0,
        );
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let ui = Face::ui(13.0);
        // The arrow that shows and hides the replacement.
        let toggle = Rect::new(r.x + 4.0, r.y + 4.0, 20.0, rows * 34.0);
        if self.hovered(toggle) {
            self.canvas.round(toggle, 4.0, p.hover);
        }
        self.canvas.chevron(
            toggle.x + 10.0,
            r.y + 4.0 + 17.0,
            8.0,
            find.replace.is_some(),
            p.muted,
        );
        self.hit(toggle, Hit::Ui(UiEvent::Find(FindAction::ToggleReplace)));
        let field_x = toggle.right() + 4.0;
        let buttons_w = 96.0;
        let field_w = r.w - (field_x - r.x) - buttons_w - 8.0;
        let count = match (find.current, find.total) {
            (_, 0) if find.query.is_empty() => String::new(),
            (_, 0) => "No results".to_string(),
            (Some(i), n) => format!("{i} of {n}"),
            (None, n) => format!("{n} found"),
        };
        let fields: Vec<(&str, bool, &str)> = std::iter::once((find.query.as_str(), false, "Find"))
            .chain(find.replace.as_deref().map(|t| (t, true, "Replace")))
            .collect();
        for (k, (value, is_replace, hint)) in fields.into_iter().enumerate() {
            let y = r.y + 4.0 + k as f32 * 34.0;
            let field = Rect::new(field_x, y + 4.0, field_w, 26.0);
            let active = find.focused && find.in_replace == is_replace;
            self.canvas.outline(
                field,
                5.0,
                1.0,
                if active { p.accent } else { p.border },
                p.raised,
            );
            self.hit(field, Hit::Ui(UiEvent::Find(FindAction::Field(is_replace))));
            let info_w = if is_replace || count.is_empty() {
                0.0
            } else {
                self.text.width(&count, Face::ui(12.0)) + 12.0
            };
            let room = field.w - 16.0 - info_w;
            let shown = self.text.fit(value, ui, room);
            let (text, colour) = if value.is_empty() {
                (hint.to_string(), p.muted)
            } else {
                (shown, p.bright)
            };
            let tw = self.text.draw_in_row(
                &mut self.canvas,
                &text,
                ui,
                field.x + 8.0,
                field.y,
                field.h,
                colour,
                field,
            );
            if active {
                let cx = field.x + 8.0 + if value.is_empty() { 0.0 } else { tw } + 1.0;
                self.canvas
                    .fill(Rect::new(cx, field.y + 5.0, 1.5, field.h - 10.0), p.accent);
            }
            if info_w > 0.0 {
                let colour = if find.total == 0 { p.error } else { p.muted };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &count,
                    Face::ui(12.0),
                    field.right() - info_w,
                    field.y,
                    field.h,
                    colour,
                    field,
                );
            }
            // Buttons right of the field.
            let mut bx = field.right() + 6.0;
            let buttons: &[(&str, FindAction)] = if is_replace {
                &[
                    ("Replace", FindAction::ReplaceOne),
                    ("All", FindAction::ReplaceAll),
                ]
            } else {
                &[
                    ("↑", FindAction::Previous),
                    ("↓", FindAction::Next),
                    ("✕", FindAction::Close),
                ]
            };
            for &(label, action) in buttons {
                let lw = self.text.width(label, ui) + 14.0;
                let b = Rect::new(bx, field.y, lw.max(26.0), field.h);
                if self.hovered(b) {
                    self.canvas.round(b, 5.0, p.hover);
                }
                let lx = b.x + (b.w - self.text.width(label, ui)) / 2.0;
                self.text
                    .draw_in_row(&mut self.canvas, label, ui, lx, b.y, b.h, p.text, b);
                self.hit(b, Hit::Ui(UiEvent::Find(action)));
                bx = b.right() + 2.0;
            }
        }
    }

    fn context(&mut self, menu: &ContextView, at: (f32, f32), w: f32, h: f32) {
        // A click anywhere else closes it.
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::CloseContext));
        self.items_card(&menu.items, menu.selected, at, w, h, UiEvent::ContextItem);
    }

    /// A card of commands with their keys at `at`, or up and left of it at
    /// the window's edges; a click on the `k`th item sends `event(k)`.
    fn items_card(
        &mut self,
        items: &[ContextItem],
        selected: Option<usize>,
        (x, y): (f32, f32),
        w: f32,
        h: f32,
        event: fn(usize) -> UiEvent,
    ) {
        let p = self.p;
        let (label_f, key_f) = (Face::ui(13.0), Face::ui(12.0));
        let widest = items
            .iter()
            .map(|i| {
                self.text.width(&i.label, label_f)
                    + i.key
                        .as_ref()
                        .map_or(0.0, |k| self.text.width(k, key_f) + 32.0)
            })
            .fold(0.0, f32::max);
        let mw = (widest + 40.0).max(220.0);
        let height = |label: &str| if label.is_empty() { 9.0 } else { 28.0 };
        let mh: f32 = items.iter().map(|i| height(&i.label)).sum::<f32>() + 10.0;
        let x = if x + mw > w - 8.0 {
            (x - mw).max(8.0)
        } else {
            x
        };
        let y = if y + mh > h - 8.0 {
            (h - 8.0 - mh).max(8.0)
        } else {
            y
        };
        let r = Rect::new(x, y, mw, mh);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let mut ry = r.y + 5.0;
        for (k, item) in items.iter().enumerate() {
            let rh = height(&item.label);
            if item.label.is_empty() {
                self.canvas
                    .fill(Rect::new(r.x + 10.0, ry + 4.0, r.w - 20.0, 1.0), p.chip);
                ry += rh;
                continue;
            }
            let row = Rect::new(r.x + 5.0, ry, r.w - 10.0, rh);
            let on = selected == Some(k) || self.hovered(row);
            if on {
                self.canvas.round(row, 5.0, p.focus_row);
            }
            let colour = if on { p.bright } else { p.text };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.label,
                label_f,
                row.x + 12.0,
                ry,
                rh,
                colour,
                row,
            );
            if let Some(key) = &item.key {
                let kw = self.text.width(key, key_f);
                self.text.draw_in_row(
                    &mut self.canvas,
                    key,
                    key_f,
                    row.right() - 12.0 - kw,
                    ry,
                    rh,
                    p.muted,
                    row,
                );
            }
            self.hit(row, Hit::Ui(event(k)));
            ry += rh;
        }
    }

    /// Hover and completion as a card under the text they're about, or
    /// over it when there's no room below.
    fn popup(&mut self, popup: &PopupView, a: &Areas, (cw, ch): (f32, f32)) {
        let p = self.p;
        let (ax, ay) = (
            a.text.x + popup.anchor.0 as f32 * cw,
            a.text.y + popup.anchor.1 as f32 * ch,
        );
        let mono = Face::mono(12.5);
        let ui = Face::ui(13.0);
        // The rows to draw: text, whether it's code, and a detail.
        let mut rows: Vec<(String, bool, String)> = Vec::new();
        let mut selected = None;
        match &popup.kind {
            PopupKind::Hover(lines) => {
                let mut code = false;
                for line in lines {
                    if line.trim_start().starts_with("```") {
                        code = !code;
                        continue;
                    }
                    if line.trim() == "---" {
                        rows.push((String::new(), false, String::new()));
                        continue;
                    }
                    rows.push((line.clone(), code, String::new()));
                }
                // Blank lines at the ends say nothing.
                while rows.last().is_some_and(|r| r.0.trim().is_empty()) {
                    rows.pop();
                }
                rows.truncate(16);
            }
            PopupKind::Completion { items, selected: s } => {
                let first = s.saturating_sub(7);
                selected = Some(s - first);
                rows = items
                    .iter()
                    .skip(first)
                    .take(8)
                    .map(|(label, detail)| (label.clone(), true, detail.clone()))
                    .collect();
            }
        }
        if rows.is_empty() {
            return;
        }
        let row_h = 21.0;
        let widest = rows
            .iter()
            .map(|(t, code, d)| {
                self.text.width(t, if *code { mono } else { ui })
                    + if d.is_empty() {
                        0.0
                    } else {
                        self.text.width(d, ui) + 24.0
                    }
            })
            .fold(0.0, f32::max);
        let pw = (widest + 24.0).clamp(160.0, 560.0);
        let ph = rows.len() as f32 * row_h + 14.0;
        let below = ay + ch + 4.0;
        let y = if below + ph > a.editor.bottom() - 4.0 && ay - ph - 4.0 > a.editor.y {
            ay - ph - 4.0
        } else {
            below
        };
        let x = ax.min(a.editor.right() - pw - 8.0).max(a.editor.x + 4.0);
        let r = Rect::new(x, y, pw, ph);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        for (k, (text, code, detail)) in rows.iter().enumerate() {
            let ry = r.y + 7.0 + k as f32 * row_h;
            let row = Rect::new(r.x + 4.0, ry, r.w - 8.0, row_h);
            if selected == Some(k) {
                self.canvas.round(row, 4.0, p.focus_row);
            }
            if text.is_empty() && !code {
                self.canvas.fill(
                    Rect::new(r.x + 8.0, ry + row_h / 2.0, r.w - 16.0, 1.0),
                    p.chip,
                );
                continue;
            }
            let face = if *code { mono } else { ui };
            let colour = if *code { p.bright } else { p.text };
            let shown = self.text.fit(text, face, row.w - 16.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &shown,
                face,
                row.x + 8.0,
                ry,
                row_h,
                colour,
                row,
            );
            if !detail.is_empty() {
                let dw = self.text.width(detail, ui).min(row.w * 0.5);
                let shown = self.text.fit(detail, ui, dw);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &shown,
                    ui,
                    row.right() - 8.0 - dw,
                    ry,
                    row_h,
                    p.muted,
                    row,
                );
            }
        }
    }

    /// A button showing the icon theme's `id` icon, or `glyph`.
    fn icon_button(&mut self, r: Rect, id: &str, glyph: char, on: bool, hit: Hit) {
        let tip = match id {
            "menu" => Some("Application Menu (F10)"),
            "layout-left" => Some("Toggle Primary Side Bar (Ctrl+B)"),
            "layout-panel" => Some("Toggle Panel (Ctrl+J)"),
            "layout-right" => Some("Toggle Secondary Side Bar"),
            "split" => Some("Split Editor Right (Ctrl+\\)"),
            "more" => Some("More Actions…"),
            _ => None,
        };
        if let Some(tip) = tip {
            self.tips.push((r, tip.to_string()));
        }
        let p = self.p;
        let hovered = self.hovered(r);
        if hovered || on {
            self.canvas
                .round(r, 6.0, if on { p.raised } else { p.hover });
        }
        let colour = if on || hovered { p.bright } else { p.muted };
        self.hit(r, hit);
        if let Some(icon) = self.icons.ui(id).cloned() {
            let size = (r.h - 12.0).max(10.0);
            let (x, y) = (r.x + (r.w - size) / 2.0, r.y + (r.h - size) / 2.0);
            if self.draw_icon(&icon, x, y, size, colour, r) {
                return;
            }
        }
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
    }

    /// Draws an icon in a `size` square at (`x`, `y`), in `colour` unless
    /// it has its own. Returns whether it could be drawn.
    #[allow(clippy::too_many_arguments)]
    fn draw_icon(
        &mut self,
        icon: &Icon,
        x: f32,
        y: f32,
        size: f32,
        colour: u32,
        clip: Rect,
    ) -> bool {
        match icon {
            Icon::Svg { file, tint } => {
                let px = (size * self.canvas.scale).round() as u32;
                let Some(picture) = self.pictures.get(file, px, self.canvas.bitmaps) else {
                    return false;
                };
                let tint = match tint {
                    Tint::Own => None,
                    Tint::Colour(c) => Some(*c),
                    Tint::Current => Some(colour),
                };
                self.canvas.picture(picture, x, y, tint, clip);
                true
            }
            Icon::Glyph { text, colour: own } => {
                let face = Face::mono(size * 0.85);
                let w = self.text.width(text, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    face,
                    x + (size - w) / 2.0,
                    y,
                    size,
                    own.unwrap_or(colour),
                    clip,
                );
                true
            }
            Icon::Badge { text, colour: own } => {
                self.badge(text, x, y, size, own.unwrap_or(colour));
                true
            }
        }
    }

    /// A file's icon from the icon theme, else as the theme's fallback has
    /// it. Returns the room it took.
    #[allow(clippy::too_many_arguments)]
    fn file_icon(
        &mut self,
        name: &str,
        core: Option<(char, merlin_core::style::Hue)>,
        x: f32,
        y: f32,
        size: f32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let colour = core.map_or(p.muted, |(_, hue)| theme::hue(&p, hue));
        if let Some(icon) = self.icons.file(name).cloned()
            && self.draw_icon(&icon, x, y, size, colour, clip)
        {
            return size + 6.0;
        }
        let ext = name
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .filter(|ext| !ext.is_empty() && ext.len() <= 4);
        match (self.icons.fallback, ext, core) {
            (Fallback::None, ..) => 0.0,
            (Fallback::Badge, Some(ext), _) => {
                self.badge(ext, x, y, size, colour);
                size + 6.0
            }
            (_, _, Some((glyph, _))) => {
                let icon = Icon::Glyph {
                    text: glyph.to_string(),
                    colour: Some(colour),
                };
                self.draw_icon(&icon, x, y, size, colour, clip);
                size + 6.0
            }
            _ => 0.0,
        }
    }

    fn title_bar(&mut self, view: &WindowView, w: f32) {
        let p = self.p;
        let bar = Rect::new(0.0, 0.0, w, TITLE_H);
        let ui = Face::ui(13.0);
        // The menus, behind one button.
        self.icon_button(
            Rect::new(10.0, 6.0, 30.0, 28.0),
            "menu",
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
        for (i, (id, glyph, command)) in [
            ("layout-left", '\u{ebf3}', "view.sidebar"),
            ("layout-panel", '\u{ebf2}', "view.panel"),
            ("layout-right", '\u{ebf4}', "view.secondary"),
        ]
        .into_iter()
        .enumerate()
        {
            self.icon_button(
                Rect::new(bx, 6.0, 30.0, 28.0),
                id,
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
            let themed = self.icons.ui(item.id).cloned();
            let drawn = themed
                .is_some_and(|icon| self.draw_icon(&icon, r.x + 8.0, y + 8.0, 20.0, colour, r));
            if !drawn {
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
            }
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
            self.tips.push((r, item.label.clone()));
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
            Node::Item { .. } | Node::Entry(_) => {
                if self.compact {
                    22.0
                } else {
                    ROW_H
                }
            }
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
                    self.hit(
                        row,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                }
            }
            Node::Entry(e) => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                self.entry(e, row, clip);
                if let Some(index) = target {
                    self.hit(
                        row,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                    if e.action.is_some() {
                        let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
                        self.hit(
                            a,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index: index + 1,
                                clicks: 1,
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
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
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
                            Hit::Ui(UiEvent::Target {
                                side,
                                index,
                                clicks: 1,
                            }),
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
                    self.hit(
                        b,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
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
                Some(open)
            }
            Some(_) => {
                x += 16.0;
                None
            }
            None => None,
        };
        match folder {
            // Folders get an icon only from an icon theme that has them.
            Some(open) => {
                if let Some(icon) = self.icons.folder(&e.label, open).cloned()
                    && self.draw_icon(&icon, x, mid - 8.0, 16.0, p.muted, clip)
                {
                    x += 22.0;
                }
            }
            None if e.icon.is_some() => {
                x += self.file_icon(&e.label, e.icon, x, mid - 8.0, 16.0, clip);
            }
            None => {}
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

    /// A strip of tabs in `row`, for group `group` when there are several.
    /// With more tabs than fit, it starts late enough for the active one to
    /// show. While a tab is dragged, a bar marks where it would go.
    /// A group's tabs as rounded pills, pinned ones first as only their
    /// icons, scrolled along when they don't all fit, with the split and
    /// more buttons at the right end.
    fn tab_strip(&mut self, tabs: &[TabView], row: Rect, group: Option<usize>, focused: bool) {
        let p = self.p;
        // Empty strip takes drops too, at the end, and a double click.
        self.hit(row, Hit::TabStrip(group));
        let face_of = |tab: &TabView| {
            if tab.preview {
                Face::ui(13.0).italic()
            } else {
                Face::ui(13.0)
            }
        };
        let widths: Vec<f32> = tabs
            .iter()
            .map(|t| {
                if t.pinned {
                    PINNED_W
                } else {
                    let nw = self.text.width(&t.name, face_of(t));
                    TAB_PAD + 14.0 + 7.0 + nw + 5.0 + 18.0 + 6.0
                }
            })
            .collect();
        let (left, right) = (row.x + 6.0, row.right() - 72.0);
        let room = (right - left).max(0.0);
        let starts: Vec<f32> = widths
            .iter()
            .scan(0.0, |x, w| {
                let at = *x;
                *x += w + TAB_GAP;
                Some(at)
            })
            .collect();
        let total = widths.iter().map(|w| w + TAB_GAP).sum::<f32>() - TAB_GAP;
        // Scrolled to the active tab when it changes, else where the wheel
        // left it.
        let g = group.unwrap_or(0);
        if self.strips.len() <= g {
            self.strips.resize(g + 1, (0.0, usize::MAX));
        }
        let active = tabs.iter().position(|t| t.active).unwrap_or(0);
        let (mut offset, revealed) = self.strips[g];
        if revealed != active
            && let (Some(&x), Some(&w)) = (starts.get(active), widths.get(active))
        {
            if x < offset {
                offset = x;
            } else if x + w > offset + room {
                offset = x + w - room;
            }
        }
        offset = offset.clamp(0.0, (total - room).max(0.0));
        self.strips[g] = (offset, active);

        let first_prim = self.canvas.scene.prims.len();
        let clip = Rect::new(left, row.y, room, row.h);
        let pill_h = row.h - 2.0 * TAB_INSET;
        for (i, tab) in tabs.iter().enumerate() {
            let w = widths[i];
            let x = left + starts[i] - offset;
            if x + w < left || x > right {
                continue;
            }
            let r = Rect::new(x, row.y + TAB_INSET, w, pill_h);
            let shown = Rect::new(x.max(left), r.y, (x + w).min(right) - x.max(left), r.h);
            let hovered = self.hovered(shown) && self.tab_drag.is_none();
            // The active tab stands out from a hovered one by its edge,
            // in the focused group.
            if tab.active && focused {
                self.canvas.outline(r, TAB_RADIUS, 1.0, p.border, p.chip);
            } else if tab.active {
                self.canvas.round(r, TAB_RADIUS, p.raised);
            } else if hovered {
                self.canvas.round(r, TAB_RADIUS, p.hover);
            }
            if self.tab_drag.is_some() && self.hovered(shown) {
                self.canvas.round(
                    Rect::new(r.x - 2.5, r.y + 3.0, 2.0, r.h - 6.0),
                    1.0,
                    p.accent,
                );
            }
            let icon_x = if tab.pinned {
                x + (w - 14.0) / 2.0
            } else {
                x + TAB_PAD
            };
            self.file_icon(
                &tab.name,
                Some(tab.icon),
                icon_x,
                r.y + (r.h - 14.0) / 2.0,
                14.0,
                clip,
            );
            let (tab_hit, close_hit) = match group {
                Some(group) => (
                    UiEvent::GroupTab {
                        group,
                        index: i,
                        clicks: 1,
                    },
                    UiEvent::CloseGroupTab { group, index: i },
                ),
                None => (
                    UiEvent::Tab {
                        index: i,
                        clicks: 1,
                    },
                    UiEvent::CloseTab(i),
                ),
            };
            self.hit(shown, Hit::Ui(tab_hit));
            if tab.pinned {
                // Only its icon: the name is in its tooltip, and changes a
                // dot in its corner.
                self.tips.push((shown, tab.name.clone()));
                if tab.dirty {
                    self.canvas.round(
                        Rect::new(r.right() - 9.0, r.y + 4.0, 5.0, 5.0),
                        2.5,
                        p.muted,
                    );
                }
                continue;
            }
            let face = face_of(tab);
            let nw = self.text.width(&tab.name, face);
            let fg = match (tab.active, focused) {
                (true, true) => p.bright,
                (true, false) => p.text,
                _ => p.muted,
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &tab.name,
                face,
                x + TAB_PAD + 21.0,
                r.y,
                r.h,
                fg,
                clip,
            );
            let close = Rect::new(
                x + TAB_PAD + 21.0 + nw + 5.0,
                r.y + (r.h - 18.0) / 2.0,
                18.0,
                18.0,
            );
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
                    clip,
                );
            }
            if close.x >= left && close.right() <= right {
                self.hit(close, Hit::Ui(close_hit));
            }
        }
        // Tabs scrolled part way out are cut at the strip's ends.
        let s = self.canvas.scale;
        clip_cols(
            &mut self.canvas.scene.prims[first_prim..],
            left * s,
            right * s,
        );
        // Split the editor, at the right end.
        let mid = row.y + (row.h - 28.0) / 2.0;
        self.icon_button(
            Rect::new(row.right() - 66.0, mid, 28.0, 28.0),
            "split",
            '\u{eb56}',
            false,
            Hit::Command("view.split"),
        );
        self.icon_button(
            Rect::new(row.right() - 34.0, mid, 28.0, 28.0),
            "more",
            '\u{ea7c}',
            false,
            Hit::Command("palette.open"),
        );
    }

    fn crumb_row(&mut self, crumbs: &[String], row: Rect, group: Option<usize>) {
        let p = self.p;
        let face = Face::ui(12.0);
        let mut x = row.x + 16.0;
        let last = crumbs.len().saturating_sub(1);
        for (i, crumb) in crumbs.iter().enumerate() {
            let tw = self.text.width(crumb, face);
            let r = Rect::new(x - 3.0, row.y + 4.0, tw + 6.0, row.h - 8.0);
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
            let hit = match group {
                Some(group) => UiEvent::GroupCrumb { group, index: i },
                None => UiEvent::Crumb(i),
            };
            self.hit(r, Hit::Ui(hit));
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

    /// Each group's tabs and breadcrumbs above its code, and the tab being
    /// dragged under the mouse.
    fn group_heads(&mut self, view: &WindowView, a: &Areas) {
        // Hairlines between the groups, over the code so nothing hides
        // them, lit while dragged or under the mouse.
        let p = self.p;
        for &(r, sash) in &a.group_sashes {
            let line = match sash {
                GroupSash::Column(_) => Rect::new(r.x + SASH, r.y, DIVIDER, r.h),
                GroupSash::Row(_) => Rect::new(r.x, r.y + SASH, r.w, DIVIDER),
            };
            let lit = self.hovered(r);
            self.canvas.fill(line, if lit { p.accent } else { p.line });
        }
        for (g, (group, area)) in view.groups.iter().zip(&a.groups).enumerate() {
            self.tab_strip(&group.tabs, area.tabs, Some(g), group.focused);
            if let Some(row) = area.crumbs {
                self.crumb_row(&group.crumbs, row, Some(g));
            }
        }
        // Where a dragged tab or file would land: a side of a group, or
        // all of it.
        if let Some(z) = self.drop_zone {
            let p = self.p;
            self.canvas.tint(z, 8.0, p.accent, 36);
            for edge in [
                Rect::new(z.x + 8.0, z.y, z.w - 16.0, 1.0),
                Rect::new(z.x + 8.0, z.bottom() - 1.0, z.w - 16.0, 1.0),
                Rect::new(z.x, z.y + 8.0, 1.0, z.h - 16.0),
                Rect::new(z.right() - 1.0, z.y + 8.0, 1.0, z.h - 16.0),
            ] {
                self.canvas.tint(edge, 0.0, p.accent, 150);
            }
        }
        if let (Some((name, icon)), Some((px, py))) = (self.tab_drag.clone(), self.pointer) {
            let p = self.p;
            let face = Face::ui(13.0);
            let r = Rect::new(
                px + 12.0,
                py + 8.0,
                TAB_PAD + 21.0 + self.text.width(&name, face) + TAB_PAD,
                26.0,
            );
            self.canvas.shadow(r, TAB_RADIUS, theme::is_light(&p));
            self.canvas.outline(r, TAB_RADIUS, 1.0, p.accent, p.raised);
            self.file_icon(&name, icon, r.x + TAB_PAD, r.y + 6.0, 14.0, r);
            self.text.draw_in_row(
                &mut self.canvas,
                &name,
                face,
                r.x + TAB_PAD + 21.0,
                r.y,
                r.h,
                p.bright,
                r,
            );
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

/// Cuts what was drawn to between physical `left` and `right`.
fn clip_cols(prims: &mut [Prim], left: f32, right: f32) {
    for prim in prims {
        match prim {
            Prim::Quad { rect, .. } | Prim::Shadow { rect, .. } => {
                if rect[2] <= left || rect[0] >= right {
                    *rect = [0.0; 4];
                } else {
                    rect[0] = rect[0].max(left);
                    rect[2] = rect[2].min(right);
                }
            }
            Prim::Line { a, b, .. } => {
                if a[0].max(b[0]) < left || a[0].min(b[0]) > right {
                    (*a, *b) = ([-64.0, -64.0], [-64.0, -64.0]);
                }
            }
            Prim::Glyph { clip, .. } | Prim::Image { clip, .. } => {
                clip[0] = clip[0].max(left.floor() as i32);
                clip[2] = clip[2].min(right.ceil() as i32);
            }
        }
    }
}
