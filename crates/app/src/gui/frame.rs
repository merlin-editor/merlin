//! The window's frame, laid out and drawn in pixels as the design has it:
//! a slim title bar, the activity rail, the side bar, editor and panel as
//! rounded islands on a darker ground, and a thin status bar. Drawing also
//! records what each clickable part does.

use merlin_core::editor::{
    BlameCardView, ChangeView, DiffBarView, MergeBarView, MergeRow, MergeSide, MergeView, PeekItem,
    PeekView, RenameView,
};
use merlin_core::lang::{ActionKind, CodeAction};
use merlin_core::panel::{Check, Entry, Node, Swap, Tone};
use merlin_core::{
    ContextItem, ContextView, DialogView, FindAction, FindView, HoverButton, Key, KeyCode,
    MenuView, PaletteView, PopupKind, PopupView, ProblemView, Severity, Side, SideView,
    SignatureView, TabView, TerminalView, TextView, ToastView, UiEvent, WindowView,
    editor::GroupView,
};

use super::canvas::{Canvas, Face, Pictures, Rect, Text};
use super::scene::Prim;
use crate::packs::{Fallback, Icon, Icons, Tint};
use crate::prefs::SettingsAction;
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
    /// A terminal's screen, where the mouse selects.
    Terminal(u64),
    /// Makes the panel as tall as it goes, or back.
    MaximizePanel,
    /// A part of the Settings tab.
    Settings(SettingsAction),
    /// The Settings tab, where the wheel scrolls it.
    SettingsBody,
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
    /// In the merge editor: Incoming's and Current's panes, and the
    /// Result's around the code.
    pub merge: Option<[Rect; 3]>,
    /// Over the code: the bar saying the file changed on disk while merlin
    /// was closed.
    pub away: Option<Rect>,
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
    /// Each terminal pane drawn, by session, and the rect its cells go in.
    pub term_panes: Vec<(u64, Rect)>,
    /// Whether the panel is maximised, for its button.
    pub panel_max: bool,
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
    /// The panel's height before it was maximised.
    pub bottom_before: Option<f32>,
}

impl Default for Sizes {
    fn default() -> Self {
        Self {
            left: SIDE_W,
            right: RIGHT_W,
            bottom: BOTTOM_H,
            scrollbar: super::overview::SCROLLBAR_W,
            minimap_scale: 1.0,
            bottom_before: None,
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
                self.bottom_before = None;
            }
        }
    }

    /// Makes the panel as tall as it goes, or back to its height before.
    pub fn maximize_panel(&mut self, h: f32) {
        match self.bottom_before.take() {
            Some(before) => self.bottom = before,
            None => {
                self.bottom_before = Some(self.bottom);
                self.bottom = (h - STATUS_H - TITLE_H) * 0.75;
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
            let merging = groups.get(g).is_some_and(|v| v.merging);
            let mut area = GroupArea::of(Rect::new(x, y, w, h), crumbs, minimap, sizes);
            if merging {
                let inline = groups.get(g).is_some_and(|v| v.merge_inline);
                area.split_for_merge(sizes, inline);
            } else if groups.get(g).is_some_and(|v| v.compare.is_some()) {
                area.under_titles();
            } else if groups.get(g).is_some_and(|v| v.away.is_some()) {
                area.under_away();
            }
            areas.push(area);
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
            merge: None,
            away: None,
        }
    }

    /// The merge editor's panes, as in VS Code: Incoming and Current side
    /// by side, over the Result, whose code is the group's text; or the
    /// Result alone.
    fn split_for_merge(&mut self, sizes: &Sizes, inline: bool) {
        let body = Rect::new(
            self.text.x + 8.0,
            self.text.y,
            (self.bar.right() - self.text.x - 16.0).max(0.0),
            (self.text.h - 8.0).max(0.0),
        );
        let top = ((body.h - 8.0) / 2.0).floor().max(0.0);
        let half = ((body.w - 8.0) / 2.0).floor().max(0.0);
        let incoming = Rect::new(body.x, body.y, half, top);
        let current = Rect::new(body.x + half + 8.0, body.y, body.w - half - 8.0, top);
        let result = if inline {
            body
        } else {
            Rect::new(body.x, body.y + top + 8.0, body.w, body.h - top - 8.0)
        };
        let code_y = result.y + MERGE_HEAD_H;
        let code_h = (result.h - MERGE_HEAD_H - 1.0).max(0.0);
        let bar_w = sizes.scrollbar;
        self.bar = Rect::new(result.right() - 1.0 - bar_w, code_y, bar_w, code_h);
        self.text = Rect::new(
            result.x + 1.0,
            code_y,
            (self.bar.x - result.x - 1.0).max(0.0),
            code_h,
        );
        self.minimap = None;
        self.merge = Some([incoming, current, result]);
    }
}

impl GroupArea {
    /// Room over the code for each compared file's title.
    /// Room over the code for the bar saying it changed on disk.
    fn under_away(&mut self) {
        let h = AWAY_H.min(self.text.h);
        self.away = Some(Rect::new(
            self.text.x,
            self.text.y - 4.0,
            self.bar.right() - self.text.x,
            h,
        ));
        for r in [&mut self.text, &mut self.bar] {
            r.y += h;
            r.h -= h;
        }
        if let Some(m) = &mut self.minimap {
            m.y += h;
            m.h -= h;
        }
    }

    fn under_titles(&mut self) {
        let h = MERGE_HEAD_H.min(self.text.h);
        for r in [&mut self.text, &mut self.bar] {
            r.y += h;
            r.h -= h;
        }
        if let Some(m) = &mut self.minimap {
            m.y += h;
            m.h -= h;
        }
    }
}

/// A merge editor pane's title row.
const MERGE_HEAD_H: f32 = 30.0;

/// The bar over a file that changed on disk while merlin was closed.
const AWAY_H: f32 = 38.0;

impl Frame<'_> {
    pub(super) fn hit(&mut self, r: Rect, hit: Hit) {
        self.hits.push((r, hit));
    }

    pub(super) fn hovered(&self, r: Rect) -> bool {
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
                let term = view.terminal.as_ref().filter(|_| side == Side::Bottom);
                self.side(sv, side, area, state, term);
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
        if let Some(bar) = &view.debug {
            self.debug_bar(bar, a.editor);
        }
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
        if let Some(signature) = &view.signature {
            self.signature(signature, a, cell);
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
        let buttons_w = 124.0;
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
            let edge = if find.invalid && !is_replace {
                p.error
            } else if active {
                p.accent
            } else {
                p.border
            };
            self.canvas.outline(field, 5.0, 1.0, edge, p.raised);
            self.hit(field, Hit::Ui(UiEvent::Find(FindAction::Field(is_replace))));
            // Match Case, Whole Word and Regex, at the query's right end.
            let toggles_w = if is_replace {
                0.0
            } else {
                self.find_toggles(find, field)
            };
            let info_w = if is_replace || count.is_empty() {
                0.0
            } else {
                self.text.width(&count, Face::ui(12.0)) + 12.0
            };
            let room = field.w - 16.0 - info_w - toggles_w;
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
                    field.right() - info_w - toggles_w,
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
                    ("", FindAction::InSelection),
                    ("✕", FindAction::Close),
                ]
            };
            for &(label, action) in buttons {
                let lw = self.text.width(label, ui) + 14.0;
                let b = Rect::new(bx, field.y, lw.max(26.0), field.h);
                bx = b.right() + 2.0;
                self.hit(b, Hit::Ui(UiEvent::Find(action)));
                if action == FindAction::InSelection {
                    self.in_selection_button(b, find.in_selection);
                    continue;
                }
                if self.hovered(b) {
                    self.canvas.round(b, 5.0, p.hover);
                }
                let lx = b.x + (b.w - self.text.width(label, ui)) / 2.0;
                self.text
                    .draw_in_row(&mut self.canvas, label, ui, lx, b.y, b.h, p.text, b);
            }
        }
    }

    /// The query's toggles, drawn at the right of `field`; returns the
    /// width they take.
    fn find_toggles(&mut self, find: &FindView, field: Rect) -> f32 {
        let p = self.p;
        let toggles = [
            (
                "Aa",
                find.match_case,
                FindAction::MatchCase,
                "Match Case (Alt+C)",
            ),
            (
                "ab",
                find.whole_word,
                FindAction::WholeWord,
                "Match Whole Word (Alt+W)",
            ),
            (
                ".*",
                find.regex,
                FindAction::Regex,
                "Use Regular Expression (Alt+R)",
            ),
        ];
        let size = field.h - 6.0;
        let mut x = field.right() - 3.0 - toggles.len() as f32 * (size + 2.0);
        let start = x;
        for (label, on, action, tip) in toggles {
            let b = Rect::new(x, field.y + 3.0, size, size);
            x += size + 2.0;
            if on {
                self.canvas.outline(b, 4.0, 1.0, p.accent, p.chip);
            } else if self.hovered(b) {
                self.canvas.round(b, 4.0, p.hover);
            }
            let face = Face::ui(12.0);
            let lw = self.text.width(label, face);
            let colour = if on { p.bright } else { p.muted };
            let lx = b.x + (b.w - lw) / 2.0;
            self.text
                .draw_in_row(&mut self.canvas, label, face, lx, b.y, b.h, colour, b);
            // Whole Word's letters sit on a line, as VS Code draws it.
            if action == FindAction::WholeWord {
                let y = (b.y + b.h * 0.78).round();
                self.canvas
                    .fill(Rect::new(lx - 1.0, y, lw + 2.0, 1.0), colour);
            }
            self.hit(b, Hit::Ui(UiEvent::Find(action)));
            self.tips.push((b, tip.to_string()));
        }
        field.right() - start
    }

    /// Find in Selection's button: lines in a box, lit when on.
    fn in_selection_button(&mut self, b: Rect, on: bool) {
        let p = self.p;
        if on {
            self.canvas.outline(b, 5.0, 1.0, p.accent, p.chip);
        } else if self.hovered(b) {
            self.canvas.round(b, 5.0, p.hover);
        }
        let colour = if on { p.bright } else { p.text };
        let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        for (k, w) in [(-1.0, 10.0), (0.0, 7.0), (1.0, 10.0)] {
            let y = (cy + k * 3.5).round();
            self.canvas
                .fill(Rect::new((cx - 5.0).round(), y, w, 1.5), colour);
        }
        self.tips.push((b, "Find in Selection (Alt+L)".to_string()));
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
        let anchor = (
            a.text.x + popup.anchor.0 as f32 * cw,
            a.text.y + popup.anchor.1 as f32 * ch,
        );
        match &popup.kind {
            PopupKind::Rename(view) => self.rename_box(view, anchor, a, (cw, ch)),
            PopupKind::Hover {
                problems,
                lines,
                definition,
                colours,
                fix,
                more_fixes,
            } => {
                let card = HoverCard {
                    problems,
                    lines,
                    colours,
                    definition: *definition,
                    fix: fix.as_deref(),
                    more_fixes: *more_fixes,
                };
                self.hover_card(&card, anchor, a, ch)
            }
            PopupKind::Actions { actions, selected } => {
                self.actions_card(actions, *selected, anchor, a, ch)
            }
            PopupKind::Completion { items, selected } => {
                self.completions(items, *selected, anchor, a, ch)
            }
        }
    }

    /// The references peek view, in the rows `text` opens for it from `y`:
    /// a header, the chosen reference's lines on the left, and every
    /// reference by file on the right.
    /// A change opened in place from its gutter bar, in the `rows` after
    /// screen position `y`: which change, Stage, Revert, the arrows and
    /// close, then what the lines were and are.
    pub fn change_peek(&mut self, change: &ChangeView, text: Rect, y: f32, ch: f32, rows: usize) {
        let p = self.p;
        let r = Rect::new(
            text.x + 36.0,
            y + 2.0,
            (text.w - 36.0 - 16.0).max(200.0),
            rows as f32 * ch - 4.0,
        );
        let clip = Rect::new(text.x, text.y, text.w, text.h);
        if r.bottom() < clip.y || r.y > clip.bottom() {
            return;
        }
        self.hit(r, Hit::Block);
        self.canvas
            .outline(r, 8.0, 1.0, p.border, mix(p.ground, p.island, 0.5));
        let inner = Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0);
        let clip = Rect::new(
            inner.x,
            inner.y.max(clip.y),
            inner.w,
            (inner.bottom().min(clip.bottom()) - inner.y.max(clip.y)).max(0.0),
        );
        let head = Rect::new(inner.x, inner.y, inner.w, 30.0);
        self.canvas.round(head, 7.0, p.widget);
        self.canvas.fill(
            Rect::new(head.x, head.bottom() - 7.0, head.w, 7.0),
            p.widget,
        );
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.border);
        let face = Face::ui(12.0);
        let title = format!("Change {} of {}", change.index, change.count);
        let tw = self.text.draw_in_row(
            &mut self.canvas,
            &title,
            face,
            head.x + 10.0,
            head.y,
            30.0,
            p.text,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            "· since the last commit",
            face,
            head.x + 10.0 + tw + 8.0,
            head.y,
            30.0,
            p.muted,
            clip,
        );
        // Close, the arrows, then Revert and Stage, from the right.
        let mut x = head.right() - 8.0;
        for (glyph, event, tip) in [
            ("✕", UiEvent::ChangeClose, "Close (Esc)"),
            ("↓", UiEvent::ChangeStep(true), "Next Change (Alt+F5)"),
            ("↑", UiEvent::ChangeStep(false), "Previous Change (Alt+F3)"),
        ] {
            x -= 22.0;
            let b = Rect::new(x, head.y + 4.0, 22.0, 22.0);
            let hovered = self.hovered(b);
            if hovered {
                self.canvas.round(b, 5.0, p.hover);
            }
            let gw = self.text.width(glyph, face);
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                face,
                b.x + (22.0 - gw) / 2.0,
                b.y,
                22.0,
                if hovered { p.bright } else { p.muted },
                clip,
            );
            self.tips.push((b, tip.into()));
            self.hit(b, Hit::Ui(event));
        }
        x -= 6.0;
        let stage = if change.staged { "Unstage" } else { "Stage" };
        for (label, event) in [
            ("Revert", UiEvent::ChangeRevert),
            (stage, UiEvent::ChangeStage),
        ] {
            let w = self.text.width(label, face) + 18.0;
            x -= w;
            let b = Rect::new(x, head.y + 4.0, w, 22.0);
            let hovered = self.hovered(b);
            self.canvas.outline(
                b,
                6.0,
                1.0,
                p.border,
                if hovered { p.border } else { p.chip },
            );
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                face,
                b.x + 9.0,
                b.y,
                22.0,
                p.bright,
                clip,
            );
            self.hit(b, Hit::Ui(event));
            x -= 6.0;
        }
        // The lines: what they were in red, what they are in green.
        let mono = Face::mono(13.0);
        let mut ly = head.bottom() + 1.0;
        for (sign, line) in &change.lines {
            if ly + ch > inner.bottom() + 0.5 {
                break;
            }
            let (band, colour) = if *sign == '-' {
                (mix(p.error, p.island, 0.16), p.error)
            } else {
                (mix(p.success, p.island, 0.14), p.success)
            };
            let row = Rect::new(inner.x, ly, inner.w, ch);
            self.canvas.fill(row, band);
            let sw = self.text.width(&sign.to_string(), mono);
            self.text.draw_in_row(
                &mut self.canvas,
                &sign.to_string(),
                mono,
                inner.x + 24.0 - sw / 2.0,
                ly,
                ch,
                colour,
                clip,
            );
            let room = inner.right() - (inner.x + 44.0) - 8.0;
            let fitted = self.text.fit(line, mono, room);
            self.text.draw_in_row(
                &mut self.canvas,
                &fitted,
                mono,
                inner.x + 44.0,
                ly,
                ch,
                p.text,
                clip,
            );
            ly += ch;
        }
        if change.more > 0 && ly + ch <= inner.bottom() + 0.5 {
            let more = format!("{} more lines", change.more);
            self.text.draw_in_row(
                &mut self.canvas,
                &more,
                face,
                inner.x + 44.0,
                ly,
                ch,
                p.muted,
                clip,
            );
        }
    }

    /// A commit's card over the blame column, from `y` down: who and when,
    /// its hash, its message, its size, and Open Changes, Blame Before
    /// This and Copy Hash.
    pub fn blame_card(&mut self, card: &BlameCardView, text: Rect, y: f32) {
        let p = self.p;
        let face = Face::ui(12.5);
        let w = (text.w - 24.0).clamp(200.0, 470.0);
        let x = text.x + 12.0;
        let body_lines: Vec<String> = if card.body.is_empty() {
            Vec::new()
        } else {
            vec![card.body.replace('\n', " ")]
        };
        let body_h = if body_lines.is_empty() {
            0.0
        } else {
            self.text
                .wrapped_height(&body_lines[0], face, w - 28.0)
                .min(80.0)
                + 6.0
        };
        let h = 12.0 + 22.0 + 10.0 + 18.0 + body_h + 10.0 + 38.0;
        // It opens below the line, or above it near the bottom.
        let y = if y + h > text.bottom() && y - h > text.y {
            y - h - 22.0
        } else {
            y
        };
        let r = Rect::new(x, y, w, h);
        self.hit(r, Hit::Block);
        self.canvas.shadow(r, 10.0, false);
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        let clip = r;
        // Who, when, and the hash.
        let top = r.y + 12.0;
        let av = Rect::new(r.x + 14.0, top, 22.0, 22.0);
        self.canvas.round(av, 11.0, mix(p.accent, p.widget, 0.3));
        let initial: String = card
            .author
            .chars()
            .take(1)
            .collect::<String>()
            .to_uppercase();
        let iw = self.text.width(&initial, Face::ui(11.0).bold());
        self.text.draw_in_row(
            &mut self.canvas,
            &initial,
            Face::ui(11.0).bold(),
            av.x + (22.0 - iw) / 2.0,
            av.y,
            22.0,
            p.bright,
            clip,
        );
        let aw = self.text.draw_in_row(
            &mut self.canvas,
            &card.author,
            face.bold(),
            av.right() + 8.0,
            top,
            22.0,
            p.bright,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &card.when,
            face,
            av.right() + 8.0 + aw + 8.0,
            top,
            22.0,
            p.muted,
            clip,
        );
        let mono = Face::mono(11.5);
        let hw = self.text.width(&card.hash, mono);
        self.text.draw_in_row(
            &mut self.canvas,
            &card.hash,
            mono,
            r.right() - 14.0 - hw,
            top,
            22.0,
            p.accent,
            clip,
        );
        // The summary, then the rest of the message.
        let mut ly = top + 22.0 + 10.0;
        let summary = self.text.fit(&card.summary, face.medium(), w - 28.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &summary,
            face.medium(),
            r.x + 14.0,
            ly,
            18.0,
            p.bright,
            clip,
        );
        ly += 18.0;
        if let Some(body) = body_lines.first() {
            let inner = Rect::new(r.x + 14.0, ly + 4.0, w - 28.0, body_h - 6.0);
            self.text.draw(
                &mut self.canvas,
                body,
                face,
                inner.x,
                inner.y,
                p.muted,
                inner,
                Some(inner.w),
            );
            ly += body_h;
        }
        ly += 10.0;
        // A rule, the size, and the buttons.
        self.canvas
            .fill(Rect::new(r.x + 1.0, ly, w - 2.0, 1.0), p.border);
        let row = Rect::new(r.x, ly + 1.0, w, 37.0);
        if let Some((added, removed, files)) = card.stats {
            let small = Face::ui(11.5);
            let mut sx = r.x + 14.0;
            sx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("+{added}"),
                small,
                sx,
                row.y,
                row.h,
                p.success,
                clip,
            ) + 4.0;
            sx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("−{removed}"),
                small,
                sx,
                row.y,
                row.h,
                p.error,
                clip,
            ) + 4.0;
            let s = if files == 1 { "" } else { "s" };
            self.text.draw_in_row(
                &mut self.canvas,
                &format!("in {files} file{s}"),
                small,
                sx,
                row.y,
                row.h,
                p.muted,
                clip,
            );
        }
        let mut bx = r.right() - 10.0;
        for (label, id) in [
            ("Copy Hash", "copy"),
            ("Blame Before This", "before"),
            ("Open Changes", "open"),
        ] {
            let bw = self.text.width(label, Face::ui(12.0)) + 18.0;
            bx -= bw;
            let b = Rect::new(bx, row.y + 7.5, bw, 22.0);
            let hovered = self.hovered(b);
            self.canvas.outline(
                b,
                6.0,
                1.0,
                p.border,
                if hovered { p.border } else { p.chip },
            );
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                Face::ui(12.0),
                b.x + 9.0,
                b.y,
                22.0,
                p.bright,
                clip,
            );
            self.hit(b, Hit::Ui(UiEvent::BlameButton(id.into())));
            bx -= 6.0;
        }
    }

    pub fn peek(&mut self, peek: &PeekView, text: Rect, y: f32, ch: f32) {
        let p = self.p;
        let r = Rect::new(
            text.x + 40.0,
            y + 4.0,
            (text.w - 40.0 - 16.0).max(200.0),
            peek.rows as f32 * ch - 8.0,
        );
        let clip = Rect::new(text.x, text.y, text.w, text.h);
        if r.bottom() < clip.y || r.y > clip.bottom() {
            return;
        }
        self.hit(r, Hit::Block);
        let border = if peek.focused { p.accent } else { p.border };
        self.canvas
            .outline(r, 8.0, 1.0, border, mix(p.ground, p.island, 0.5));
        let inner = Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0);
        let clip = Rect::new(
            inner.x,
            inner.y.max(clip.y),
            inner.w,
            (inner.bottom().min(clip.bottom()) - inner.y.max(clip.y)).max(0.0),
        );
        // The header: the name, how many, and its buttons.
        let head = Rect::new(inner.x, inner.y, inner.w, 30.0);
        self.canvas.round(head, 7.0, p.widget);
        self.canvas.fill(
            Rect::new(head.x, head.bottom() - 7.0, head.w, 7.0),
            p.widget,
        );
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.border);
        let bold = Face::ui(12.5).bold();
        let nw = self.text.draw_in_row(
            &mut self.canvas,
            &peek.name,
            bold,
            head.x + 12.0,
            head.y,
            30.0,
            p.bright,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &peek.summary,
            Face::ui(12.5),
            head.x + 12.0 + nw + 10.0,
            head.y,
            30.0,
            p.muted,
            clip,
        );
        let close = Rect::new(head.right() - 30.0, head.y + 3.0, 24.0, 24.0);
        let search = Rect::new(close.x - 26.0, close.y, 24.0, 24.0);
        for (b, event) in [(search, UiEvent::PeekToSearch), (close, UiEvent::PeekClose)] {
            let hovered = self.hovered(b);
            if hovered {
                self.canvas.round(b, 5.0, p.hover);
            }
            let c = if hovered { p.bright } else { p.muted };
            let (cx, cy) = (b.x + 12.0, b.y + 12.0);
            if event == UiEvent::PeekClose {
                self.canvas
                    .line((cx - 3.5, cy - 3.5), (cx + 3.5, cy + 3.5), 1.5, c);
                self.canvas
                    .line((cx + 3.5, cy - 3.5), (cx - 3.5, cy + 3.5), 1.5, c);
            } else {
                self.canvas
                    .frame(Rect::new(cx - 5.0, cy - 5.0, 10.0, 10.0), 1.5, 1.3, c, 0);
                self.canvas
                    .line((cx - 2.0, cy - 5.0), (cx - 2.0, cy + 5.0), 1.3, c);
            }
            self.hit(b, Hit::Ui(event));
        }
        // The list, on the right.
        let list_w = (inner.w * 0.32).clamp(220.0, 330.0);
        let body = Rect::new(
            inner.x,
            head.bottom() + 1.0,
            inner.w,
            inner.bottom() - head.bottom() - 1.0,
        );
        let list = Rect::new(body.right() - list_w, body.y, list_w, body.h);
        let code = Rect::new(body.x, body.y, body.w - list_w - 1.0, body.h);
        self.canvas.fill(code, p.island);
        self.canvas
            .fill(Rect::new(list.x - 1.0, list.y, 1.0, list.h), p.border);
        let code_clip = Rect::new(
            code.x,
            clip.y.max(code.y),
            code.w,
            clip.bottom().min(code.bottom()) - clip.y.max(code.y),
        );
        let mono = Face::mono(13.0);
        let cw = self.text.width("0", mono);
        let x = code.x + 60.0;
        let end = code_clip.right() - 12.0;
        // The lines' shared indent is left out, and long lines slide left
        // so the chosen reference shows.
        let indent = peek
            .preview
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| l.text.chars().take_while(|c| c.is_whitespace()).count())
            .min()
            .unwrap_or(0);
        let room = ((end - x) / cw).max(1.0) as usize;
        let skip = peek
            .preview
            .iter()
            .find(|l| l.current)
            .and_then(|l| l.marks.first())
            .filter(|&&(_, b)| b.saturating_sub(indent) > room)
            .map_or(indent, |&(a, _)| a.saturating_sub(room / 3).max(indent));
        let mut ly = code.y + 6.0;
        for line in &peek.preview {
            if ly + ch > code.bottom() {
                break;
            }
            let number = line.number.to_string();
            let nw = self.text.width(&number, mono);
            self.text.draw_in_row(
                &mut self.canvas,
                &number,
                mono,
                code.x + 40.0 - nw,
                ly,
                ch,
                p.muted,
                code_clip,
            );
            let chars: Vec<char> = line.text.chars().skip(skip).collect();
            let shift = |(a, b): (usize, usize)| (a.saturating_sub(skip), b.saturating_sub(skip));
            for &mark in &line.marks {
                let (a, b) = shift(mark);
                if b == 0 {
                    continue;
                }
                let before: String = chars[..a.min(chars.len())].iter().collect();
                let word: String = chars[a.min(chars.len())..b.min(chars.len())]
                    .iter()
                    .collect();
                let mx = x + self.text.width(&before, mono);
                let mw = self.text.width(&word, mono).max(cw).min(end - mx);
                if mw > 0.0 {
                    self.canvas
                        .round(Rect::new(mx, ly + 2.0, mw, ch - 4.0), 2.0, p.find_match);
                }
            }
            let text: String = chars.iter().collect();
            let colours: Spans = line
                .colours
                .iter()
                .map(|&(a, b, k)| {
                    let (a, b) = shift((a, b));
                    (a, b, k)
                })
                .filter(|&(a, b, _)| b > a)
                .collect();
            self.code_line_in(&text, &colours, x, ly, ch, code_clip, mono);
            ly += ch;
        }
        // Files and their references.
        let list_clip = Rect::new(
            list.x,
            clip.y.max(list.y),
            list.w,
            clip.bottom().min(list.bottom()) - clip.y.max(list.y),
        );
        let mut iy = list.y + 4.0;
        for (i, item) in peek.items.iter().enumerate() {
            let h = if matches!(item, PeekItem::File { .. }) {
                26.0
            } else {
                24.0
            };
            if iy + h > list.bottom() {
                break;
            }
            let row = Rect::new(list.x, iy, list.w, h);
            if peek.selected == Some(i) {
                self.canvas.fill(
                    row,
                    if peek.focused {
                        p.focus_row
                    } else {
                        p.selected
                    },
                );
            } else if self.hovered(row) {
                self.canvas.fill(row, p.hover);
            }
            match item {
                PeekItem::File {
                    name,
                    icon,
                    folder,
                    count,
                    open,
                } => {
                    let mid = iy + h / 2.0;
                    self.canvas.chevron(row.x + 15.0, mid, 7.0, *open, p.muted);
                    let mut x = row.x + 24.0;
                    x += self.file_icon(name, Some(*icon), x, mid - 8.0, 16.0, list_clip);
                    let face = Face::ui(12.5);
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        name,
                        face,
                        x,
                        iy,
                        h,
                        p.text,
                        list_clip,
                    );
                    let badge = count.to_string();
                    let bf = Face::ui(11.0);
                    let bw = self.text.width(&badge, bf) + 12.0;
                    let pill = Rect::new(row.right() - 10.0 - bw, mid - 8.0, bw, 16.0);
                    let room = (pill.x - x - 14.0).max(0.0);
                    let folder = self.text.fit(folder, Face::ui(11.5), room);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &folder,
                        Face::ui(11.5),
                        x + 6.0,
                        iy,
                        h,
                        p.muted,
                        list_clip,
                    );
                    self.canvas.round(pill, 8.0, p.chip);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &badge,
                        bf,
                        pill.x + 6.0,
                        pill.y,
                        16.0,
                        p.muted,
                        list_clip,
                    );
                }
                PeekItem::Place { line, text, mark } => {
                    let face = Face::mono(12.0);
                    let colour = if peek.selected == Some(i) {
                        p.bright
                    } else {
                        p.text
                    };
                    let number = format!("{line}  ");
                    let x = row.x + 26.0;
                    let nw = self.text.draw_in_row(
                        &mut self.canvas,
                        &number,
                        face,
                        x,
                        iy,
                        h,
                        p.muted,
                        list_clip,
                    );
                    let end = row.right() - 10.0;
                    let chars: Vec<char> = text.chars().collect();
                    let (a, b) = (mark.0.min(chars.len()), mark.1.min(chars.len()));
                    let before: String = chars[..a].iter().collect();
                    let word: String = chars[a..b].iter().collect();
                    let mx = x + nw + self.text.width(&before, face);
                    let mw = self.text.width(&word, face).min(end - mx);
                    if mw > 0.0 {
                        self.canvas
                            .round(Rect::new(mx, iy + 4.0, mw, h - 8.0), 2.0, p.find_match);
                    }
                    let shown = self.text.fit(text, face, (end - x - nw).max(0.0));
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &shown,
                        face,
                        x + nw,
                        iy,
                        h,
                        colour,
                        list_clip,
                    );
                }
            }
            self.hit(
                row,
                Hit::Ui(UiEvent::PeekRow {
                    index: i,
                    clicks: 1,
                }),
            );
            iy += h;
        }
    }

    /// The rename box (F2) under the name, which is outlined: the new name
    /// in a field, and what Enter and Shift+Enter do.
    fn rename_box(
        &mut self,
        view: &RenameView,
        anchor: (f32, f32),
        a: &Areas,
        (cw, ch): (f32, f32),
    ) {
        let p = self.p;
        let name = Rect::new(anchor.0 - 1.0, anchor.1, view.name as f32 * cw + 2.0, ch);
        self.canvas.frame(name, 2.0, 1.0, p.accent, 0);
        let mono = Face::mono(13.0);
        let note = Face::ui(11.5);
        let text_w = self.text.width(&view.text, mono);
        let note_w = self.text.width(&view.note, note);
        let w = (text_w + 40.0).max(note_w + 16.0).clamp(280.0, 560.0);
        let h = 6.0 + 26.0 + 6.0 + 16.0 + 6.0;
        let r = self.card_place((anchor.0 - 7.0, anchor.1), w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let field = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, 26.0);
        self.canvas.outline(field, 5.0, 1.0, p.accent, p.raised);
        let x = field.x + 8.0;
        if view.selected && !view.text.is_empty() {
            let sel = Rect::new(x - 1.0, field.y + 5.0, text_w + 2.0, 16.0);
            self.canvas.round(sel, 2.0, p.selection);
        }
        self.text.draw_in_row(
            &mut self.canvas,
            &view.text,
            mono,
            x,
            field.y,
            field.h,
            p.bright,
            field.inset(1.0),
        );
        let before: String = view.text.chars().take(view.caret).collect();
        let cx = x + self.text.width(&before, mono);
        self.canvas
            .fill(Rect::new(cx, field.y + 5.5, 1.5, 15.0), p.accent);
        let shown = self.text.fit(&view.note, note, r.w - 16.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &shown,
            note,
            r.x + 8.0,
            field.bottom() + 4.0,
            16.0,
            p.muted,
            r,
        );
    }

    /// Where a card of `w` by `h` goes for text at `anchor`: under its
    /// line, or over it when there's no room below.
    fn card_place(&self, (ax, ay): (f32, f32), w: f32, h: f32, a: &Areas, ch: f32) -> Rect {
        let below = ay + ch + 4.0;
        let y = if below + h > a.editor.bottom() - 4.0 && ay - h - 4.0 > a.editor.y {
            ay - h - 4.0
        } else {
            below
        };
        let x = ax.min(a.editor.right() - w - 8.0).max(a.editor.x + 4.0);
        Rect::new(x, y, w, h)
    }

    fn completions(
        &mut self,
        items: &[(String, String)],
        selected: usize,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let mono = Face::mono(12.5);
        let ui = Face::ui(13.0);
        let first = selected.saturating_sub(7);
        let rows: Vec<&(String, String)> = items.iter().skip(first).take(8).collect();
        if rows.is_empty() {
            return;
        }
        let row_h = 21.0;
        let widest = rows
            .iter()
            .map(|(t, d)| {
                self.text.width(t, mono)
                    + if d.is_empty() {
                        0.0
                    } else {
                        self.text.width(d, ui) + 24.0
                    }
            })
            .fold(0.0, f32::max);
        let pw = (widest + 24.0).clamp(160.0, 560.0);
        let ph = rows.len() as f32 * row_h + 14.0;
        let r = self.card_place(anchor, pw, ph, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        for (k, (text, detail)) in rows.iter().enumerate() {
            let ry = r.y + 7.0 + k as f32 * row_h;
            let row = Rect::new(r.x + 4.0, ry, r.w - 8.0, row_h);
            if selected - first == k {
                self.canvas.round(row, 4.0, p.focus_row);
            }
            let shown = self.text.fit(text, mono, row.w - 16.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &shown,
                mono,
                row.x + 8.0,
                ry,
                row_h,
                p.bright,
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

    /// The hover card: the problems at the spot first, each with its
    /// severity's badge, its message and notes, what reported it and its
    /// code; then, under a divider, the signature in code and the first of
    /// the docs, and Go to definition.
    fn hover_card(&mut self, card: &HoverCard, anchor: (f32, f32), a: &Areas, ch: f32) {
        let HoverCard {
            problems,
            lines,
            colours,
            definition,
            fix,
            more_fixes,
        } = *card;
        let p = self.p;
        let parts = hover_parts(lines, colours);
        if problems.is_empty() && parts.is_empty() {
            return;
        }
        // As wide as the widest line, up to a limit past which prose wraps.
        let mut widest: f32 = 0.0;
        for pr in problems {
            for (k, line) in pr.message.lines().enumerate() {
                let face = if k == 0 { MESSAGE } else { PROSE };
                widest = widest.max(self.rich_width(line.trim(), face) + 50.0);
            }
        }
        for part in &parts {
            widest = widest.max(match part {
                Part::Code(line, _) => self.text.width(line, CODE) + 24.0,
                Part::Prose(text) => self.rich_width(text, PROSE) + 24.0,
                Part::Gap => 0.0,
            });
        }
        // Room for the fix, More fixes, Copy and Problems in a row.
        if let Some(title) = fix {
            let tw = self.rich_width(title, (Face::ui(12.0), Face::mono(11.5)));
            widest = widest.max(38.0 + tw + 36.0 + 6.0 + 120.0 + 130.0);
        }
        let min = if problems.is_empty() { 200.0 } else { 340.0 };
        let w = widest.clamp(min, 560.0);
        let layout = self.hover_layout(problems, &parts, definition, w);
        let h = layout.last().map_or(0.0, |(y, h, _)| y + h) + 10.0;
        let r = self.card_place(anchor, w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        // The gap between the line and the card holds it open too, so the
        // mouse can cross into it.
        let gap = if r.y > anchor.1 {
            (anchor.1 + ch, r.y)
        } else {
            (r.bottom(), anchor.1)
        };
        self.hit(
            Rect::new(r.x, gap.0, r.w, (gap.1 - gap.0).max(0.0)),
            Hit::Block,
        );
        let clip = r;
        let text_x = r.x + 38.0;
        for (y, h, op) in layout {
            let y = r.y + y;
            match op {
                Op::Badge(severity) => self.badge_icon(severity, r.x + 12.0, y + 2.0),
                Op::Message(line) => {
                    self.draw_rich(&line, text_x, y, h, MESSAGE, p.bright, p.type_, clip)
                }
                Op::Note { line, help } => {
                    let mut x = text_x;
                    if help {
                        let face = PROSE.0;
                        self.text.draw_in_row(
                            &mut self.canvas,
                            "help:",
                            face,
                            x,
                            y,
                            h,
                            p.text,
                            clip,
                        );
                        x += self.text.width("help: ", face);
                    }
                    self.draw_rich(&line, x, y, h, PROSE, p.muted, p.type_, clip);
                }
                Op::Source(i) => {
                    let problem = &problems[i];
                    let mut x = text_x;
                    if !problem.source.is_empty() {
                        x += self.chip(&problem.source, x, y + (h - 16.0) / 2.0, false) + 6.0;
                    }
                    let face = Face::ui(11.5);
                    if !problem.code.is_empty() {
                        let cw = self.text.width(&problem.code, face);
                        let link = !problem.link.is_empty();
                        let code = Rect::new(x, y, cw, h);
                        let colour = match (link, self.hovered(code)) {
                            (true, true) => p.bright,
                            (true, false) => p.accent,
                            (false, _) => p.muted,
                        };
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &problem.code,
                            face,
                            x,
                            y,
                            h,
                            colour,
                            clip,
                        );
                        if link {
                            self.hit(code, Hit::Ui(UiEvent::Hover(HoverButton::Link(i))));
                        }
                        x += cw + 6.0;
                    }
                    if !problem.also.is_empty() {
                        let also = format!("·  also from {}", problem.also.join(", "));
                        let shown = self.text.fit(&also, face, r.right() - 12.0 - x);
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &shown,
                            face,
                            x,
                            y,
                            h,
                            p.muted,
                            clip,
                        );
                    }
                }
                Op::Buttons => {
                    // The likeliest fix and the rest on the left, quiet
                    // links on the right.
                    let links = Face::ui(12.0);
                    let links_w =
                        self.text.width("Problems", links) + self.text.width("Copy", links) + 24.0;
                    let label = if fix.is_some() { "More fixes" } else { "Fixes" };
                    let more_w = if more_fixes {
                        self.more_width(label) + 6.0
                    } else {
                        0.0
                    };
                    let mut fx = text_x;
                    if let Some(title) = fix {
                        let room = r.right() - 8.0 - links_w - 8.0 - more_w - fx;
                        fx += self.fix_button(title, fx, y, h, room) + 6.0;
                    }
                    if more_fixes {
                        self.more_button(label, fx, y, h);
                    }
                    let mut x = r.right() - 8.0;
                    for (label, button) in [
                        ("Problems", HoverButton::Problems),
                        ("Copy", HoverButton::Copy),
                    ] {
                        let face = Face::ui(12.0);
                        let bw = self.text.width(label, face) + 12.0;
                        x -= bw;
                        let b = Rect::new(x, y, bw, h);
                        let hovered = self.hovered(b);
                        if hovered {
                            self.canvas.round(b, 5.0, p.hover);
                        }
                        let colour = if hovered { p.bright } else { p.muted };
                        self.text.draw_in_row(
                            &mut self.canvas,
                            label,
                            face,
                            x + 6.0,
                            y,
                            h,
                            colour,
                            clip,
                        );
                        self.hit(b, Hit::Ui(UiEvent::Hover(button)));
                    }
                }
                Op::Divider => self
                    .canvas
                    .fill(Rect::new(r.x + 1.0, y, r.w - 2.0, 1.0), p.border),
                Op::Code(line, spans) => self.code_line(&line, &spans, r.x + 12.0, y, h, clip),
                Op::Prose(line) => {
                    self.draw_rich(&line, r.x + 12.0, y, h, PROSE, p.muted, p.text, clip)
                }
                Op::Definition => {
                    let face = Face::ui(11.5);
                    let label = "Go to definition";
                    let lw = self.text.width(label, face);
                    let link = Rect::new(r.x + 12.0, y, lw + 8.0 + 30.0, h);
                    let colour = if self.hovered(link) {
                        p.bright
                    } else {
                        p.accent
                    };
                    self.text.draw_in_row(
                        &mut self.canvas,
                        label,
                        face,
                        r.x + 12.0,
                        y,
                        h,
                        colour,
                        clip,
                    );
                    self.chip("F12", r.x + 12.0 + lw + 8.0, y + (h - 16.0) / 2.0, false);
                    self.hit(link, Hit::Ui(UiEvent::Hover(HoverButton::Definition)));
                }
            }
        }
    }

    /// The hover card's likeliest fix: a bulb and its title, as the
    /// primary button. Returns its width.
    fn fix_button(&mut self, title: &str, x: f32, y: f32, h: f32, room: f32) -> f32 {
        let p = self.p;
        let face = (Face::ui(12.0), Face::mono(11.5));
        let tw = self.rich_width(title, face).min((room - 36.0).max(60.0));
        let b = Rect::new(x, y, tw + 36.0, h);
        let hovered = self.hovered(b);
        let fill = if hovered {
            mix(p.accent, p.widget, 0.32)
        } else {
            mix(p.accent, p.widget, 0.2)
        };
        self.canvas
            .outline(b, 5.0, 1.0, mix(p.accent, p.widget, 0.45), fill);
        self.bulb_icon(x + 8.0, y + (h - 12.0) / 2.0, 12.0, p.warning);
        let line = self
            .wrap_rich(title, face, f32::MAX)
            .into_iter()
            .next()
            .unwrap_or_default();
        // `draw_rich` stops 12 short of the clip, so this ends 10 inside.
        let clip = Rect::new(x, y, b.w + 2.0, h);
        self.draw_rich(&line, x + 26.0, y, h, face, p.bright, p.bright, clip);
        self.hit(b, Hit::Ui(UiEvent::Hover(HoverButton::Fix)));
        b.w
    }

    fn more_width(&mut self, label: &str) -> f32 {
        let key_w = self.text.width("Ctrl .", Face::ui(11.0)) + 12.0;
        self.text.width(label, Face::ui(12.0)) + key_w + 24.0
    }

    /// More fixes, with its key.
    fn more_button(&mut self, label: &str, x: f32, y: f32, h: f32) {
        let p = self.p;
        let face = Face::ui(12.0);
        let tw = self.text.width(label, face);
        let b = Rect::new(x, y, self.more_width(label), h);
        let hovered = self.hovered(b);
        self.canvas
            .round(b, 5.0, if hovered { p.border } else { p.chip });
        self.text
            .draw_in_row(&mut self.canvas, label, face, x + 9.0, y, h, p.text, b);
        self.chip("Ctrl .", x + tw + 15.0, y + (h - 16.0) / 2.0, false);
        self.hit(b, Hit::Ui(UiEvent::Hover(HoverButton::MoreFixes)));
    }

    /// A lightbulb in a `size` square at (`x`, `y`): its glass and base.
    fn bulb_icon(&mut self, x: f32, y: f32, size: f32, colour: u32) {
        let (cx, cy, r) = (x + size / 2.0, y + size * 0.4, size * 0.3);
        self.canvas
            .round(Rect::new(cx - r, cy - r, r * 2.0, r * 2.0), r, colour);
        let w = r * 0.6;
        self.canvas.round(
            Rect::new(cx - w, cy + r * 0.6, w * 2.0, r * 1.0),
            1.0,
            colour,
        );
    }

    /// Code actions by the cursor, grouped as Quick Fix, Refactor, Source
    /// and More Actions, the one Enter takes lit up and the server's
    /// preferred fix marked with a bulb.
    fn actions_card(
        &mut self,
        actions: &[CodeAction],
        selected: usize,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let face = (Face::ui(12.5), Face::mono(12.0));
        const SHOWN: usize = 14;
        let first = selected.saturating_sub(SHOWN - 1);
        let shown: Vec<(usize, &CodeAction)> =
            actions.iter().enumerate().skip(first).take(SHOWN).collect();
        if shown.is_empty() {
            return;
        }
        let mut widest: f32 = 0.0;
        for (_, action) in &shown {
            widest = widest.max(self.rich_width(&action.title, face));
        }
        let w = (widest + 52.0).clamp(260.0, 520.0);
        // Rows: a group's title before its first action, a rule between.
        let (row_h, title_h, rule_h) = (28.0, 22.0, 9.0);
        let mut layout: Vec<(f32, Option<usize>, Option<ActionKind>)> = Vec::new();
        let mut y = 4.0;
        let mut group = None;
        for &(i, action) in &shown {
            if group != Some(action.kind) {
                if group.is_some() {
                    layout.push((y, None, None));
                    y += rule_h;
                }
                // Formatting is the editor's own, under the rule alone.
                if action.kind != ActionKind::Format {
                    layout.push((y, None, Some(action.kind)));
                    y += title_h;
                }
                group = Some(action.kind);
            }
            layout.push((y, Some(i), None));
            y += row_h;
        }
        let r = self.card_place(anchor, w, y + 4.0, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        for (top, row, title) in layout {
            let ry = r.y + top;
            match (row, title) {
                (None, Some(kind)) => {
                    let label = match kind {
                        ActionKind::QuickFix => "Quick Fix",
                        ActionKind::Refactor => "Refactor",
                        ActionKind::Source => "Source",
                        ActionKind::Other => "More Actions",
                        ActionKind::Format => "Format",
                    };
                    let f = Face::ui(11.0);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        label,
                        f,
                        r.x + 14.0,
                        ry,
                        title_h,
                        p.muted,
                        r,
                    );
                }
                (None, None) => self.canvas.fill(
                    Rect::new(r.x + 6.0, ry + rule_h / 2.0, r.w - 12.0, 1.0),
                    p.border,
                ),
                (Some(i), _) => {
                    let action = &actions[i];
                    let b = Rect::new(r.x + 4.0, ry, r.w - 8.0, row_h);
                    let hovered = self.hovered(b);
                    if i == selected {
                        self.canvas.round(b, 5.0, p.focus_row);
                    } else if hovered {
                        self.canvas.round(b, 5.0, p.hover);
                    }
                    if action.preferred {
                        self.bulb_icon(b.x + 8.0, ry + (row_h - 12.0) / 2.0, 12.0, p.warning);
                    }
                    let format = action.kind == ActionKind::Format;
                    let colour = match (i == selected, format) {
                        (true, _) => p.bright,
                        (false, true) => p.muted,
                        (false, false) => p.text,
                    };
                    if format {
                        let key = if action.id == merlin_core::editor::FORMAT_SELECTION {
                            "Ctrl K Ctrl F"
                        } else {
                            "Ctrl Shift I"
                        };
                        self.chip(key, b.right() - 8.0, ry + (row_h - 16.0) / 2.0, true);
                    }
                    let line = self
                        .wrap_rich(&action.title, face, f32::MAX)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let clip = Rect::new(b.x, ry, b.w - 4.0, row_h);
                    self.draw_rich(&line, b.x + 28.0, ry, row_h, face, colour, colour, clip);
                    self.hit(b, Hit::Ui(UiEvent::Action(i)));
                }
            }
        }
    }

    /// Lays the hover card out at width `w`: each thing to draw, with its
    /// top and height from the card's top.
    fn hover_layout(
        &mut self,
        problems: &[ProblemView],
        parts: &[Part],
        definition: bool,
        w: f32,
    ) -> Vec<(f32, f32, Op)> {
        let mut ops = Vec::new();
        let mut y = 10.0;
        let text_w = w - 38.0 - 12.0;
        for (i, problem) in problems.iter().enumerate() {
            ops.push((y, 20.0, Op::Badge(problem.severity)));
            let mut lines = problem.message.lines().filter(|l| !l.trim().is_empty());
            let first = lines.next().unwrap_or_default();
            for line in self.wrap_rich(first, MESSAGE, text_w).into_iter().take(3) {
                ops.push((y, 20.0, Op::Message(line)));
                y += 20.0;
            }
            for note in lines.take(6) {
                let note = note.trim();
                let (help, rest) = match note.strip_prefix("help:") {
                    Some(rest) => (true, rest.trim_start()),
                    None => (false, note),
                };
                let indent = if help {
                    self.text.width("help: ", PROSE.0)
                } else {
                    0.0
                };
                for (k, line) in self
                    .wrap_rich(rest, PROSE, text_w - indent)
                    .into_iter()
                    .take(3)
                    .enumerate()
                {
                    ops.push((
                        y,
                        18.0,
                        Op::Note {
                            line,
                            help: help && k == 0,
                        },
                    ));
                    y += 18.0;
                }
            }
            if !problem.source.is_empty() || !problem.code.is_empty() {
                y += 4.0;
                ops.push((y, 20.0, Op::Source(i)));
                y += 20.0;
            }
            y += 8.0;
        }
        if !problems.is_empty() {
            ops.push((y - 4.0, 24.0, Op::Buttons));
            y += 20.0 + 8.0;
        }
        if !parts.is_empty() || definition {
            if !problems.is_empty() {
                ops.push((y, 1.0, Op::Divider));
                y += 9.0;
            }
            for part in parts {
                match part {
                    Part::Code(line, spans) => {
                        ops.push((y, 19.0, Op::Code(line.clone(), spans.clone())));
                        y += 19.0;
                    }
                    Part::Prose(text) => {
                        for line in self.wrap_rich(text, PROSE, w - 24.0).into_iter().take(4) {
                            ops.push((y, 18.0, Op::Prose(line)));
                            y += 18.0;
                        }
                    }
                    Part::Gap => y += 6.0,
                }
            }
            if definition {
                y += 6.0;
                ops.push((y, 20.0, Op::Definition));
            }
        }
        ops
    }

    /// A line of code in a card, in the colours its language's plugin
    /// gave, by character.
    #[allow(clippy::too_many_arguments)]
    fn code_line(&mut self, line: &str, spans: &Spans, x: f32, y: f32, h: f32, clip: Rect) {
        self.code_line_in(line, spans, x, y, h, clip, CODE);
    }

    /// A line of code in `face`, coloured by `spans`.
    #[allow(clippy::too_many_arguments)]
    fn code_line_in(
        &mut self,
        line: &str,
        spans: &Spans,
        mut x: f32,
        y: f32,
        h: f32,
        clip: Rect,
        face: Face,
    ) {
        let p = self.p;
        let chars: Vec<char> = line.chars().collect();
        let mut runs: Vec<(String, u32)> = Vec::new();
        let mut at = 0;
        for &(start, end, syntax) in spans {
            let (start, end) = (start.min(chars.len()), end.min(chars.len()));
            if start < at || start >= end {
                continue;
            }
            if start > at {
                runs.push((chars[at..start].iter().collect(), p.text));
            }
            let colour = theme::code(&p, syntax).unwrap_or(p.text);
            runs.push((chars[start..end].iter().collect(), colour));
            at = end;
        }
        if at < chars.len() {
            runs.push((chars[at..].iter().collect(), p.text));
        }
        for (text, colour) in runs {
            let max = clip.right() - 12.0 - x;
            if max <= 0.0 {
                break;
            }
            let shown = self.text.fit(&text, face, max);
            self.text
                .draw_in_row(&mut self.canvas, &shown, face, x, y, h, colour, clip);
            x += self.text.width(&shown, face);
        }
    }

    /// A severity's badge: its mark in a round chip of its colour.
    fn badge_icon(&mut self, severity: Severity, x: f32, y: f32) {
        let p = self.p;
        let colour = theme::severity(&p, severity);
        let r = Rect::new(x, y, 16.0, 16.0);
        self.canvas.round(r, 8.0, mix(colour, p.widget, 0.2));
        let mark = match severity {
            Severity::Error => "×",
            Severity::Warning => "!",
            _ => "i",
        };
        let face = Face::ui(if severity == Severity::Error {
            13.0
        } else {
            11.0
        })
        .bold();
        let mw = self.text.width(mark, face);
        self.text.draw_in_row(
            &mut self.canvas,
            mark,
            face,
            x + (16.0 - mw) / 2.0,
            y,
            16.0,
            colour,
            r,
        );
    }

    /// The width of a line whose `backticked` parts are code.
    fn rich_width(&mut self, line: &str, (face, code): (Face, Face)) -> f32 {
        code_spans(line)
            .iter()
            .map(|(text, is_code)| self.text.width(text, if *is_code { code } else { face }))
            .sum()
    }

    /// A line with backticked code wrapped at `w`, as runs of text and code.
    fn wrap_rich(
        &mut self,
        line: &str,
        (face, code): (Face, Face),
        w: f32,
    ) -> Vec<Vec<(String, bool)>> {
        let mut out: Vec<Vec<(String, bool)>> = vec![Vec::new()];
        let mut width = 0.0;
        for (span, is_code) in code_spans(line) {
            for word in span.split_inclusive(' ') {
                let ww = self.text.width(word, if is_code { code } else { face });
                let line = out.last_mut().unwrap();
                if width + ww > w && !line.is_empty() {
                    out.push(Vec::new());
                    width = 0.0;
                }
                let line = out.last_mut().unwrap();
                match line.last_mut() {
                    Some((text, c)) if *c == is_code => text.push_str(word),
                    _ => line.push((word.to_string(), is_code)),
                }
                width += ww;
            }
        }
        out.retain(|l| !l.is_empty());
        out
    }

    /// Runs of text and code side by side, cut off at the card's edge.
    #[allow(clippy::too_many_arguments)]
    fn draw_rich(
        &mut self,
        line: &[(String, bool)],
        mut x: f32,
        y: f32,
        h: f32,
        (face, code): (Face, Face),
        colour: u32,
        code_colour: u32,
        clip: Rect,
    ) {
        for (text, is_code) in line {
            let (f, c) = if *is_code {
                (code, code_colour)
            } else {
                (face, colour)
            };
            let max = clip.right() - 12.0 - x;
            if max <= 0.0 {
                break;
            }
            let shown = self.text.fit(text, f, max);
            self.text
                .draw_in_row(&mut self.canvas, &shown, f, x, y, h, c, clip);
            x += self.text.width(&shown, f);
        }
    }

    /// Signature help: a card over the line of the call being typed, or
    /// under it when there's no room above, so it keeps clear of the
    /// completions below. The parameter the cursor is in stands out.
    fn signature(&mut self, view: &SignatureView, a: &Areas, (cw, ch): (f32, f32)) {
        let p = self.p;
        let sig = &view.signature;
        let (ax, ay) = (
            a.text.x + view.anchor.0 as f32 * cw,
            a.text.y + view.anchor.1 as f32 * ch,
        );
        let mono = Face::mono(12.5);
        let ui = Face::ui(12.0);
        let chars: Vec<char> = sig.label.chars().collect();
        let (s, e) = sig.active.unwrap_or((0, 0));
        let (s, e) = (
            s.min(chars.len()),
            e.min(chars.len()).max(s.min(chars.len())),
        );
        let parts: [String; 3] = [
            chars[..s].iter().collect(),
            chars[s..e].iter().collect(),
            chars[e..].iter().collect(),
        ];
        let count = format!("{} of {}", sig.index + 1, sig.count.max(1));
        let label_w: f32 = parts
            .iter()
            .enumerate()
            .map(|(k, t)| self.text.width(t, if k == 1 { mono.bold() } else { mono }))
            .sum();
        let count_w = self.text.width(&count, ui);
        let pw = (label_w + count_w + 48.0).clamp(220.0, 620.0);
        let doc_w = pw - 24.0;
        let doc_h = if sig.doc.is_empty() {
            0.0
        } else {
            self.text
                .wrapped_height(&sig.doc, ui, doc_w)
                .min(3.0 * 17.0)
                + 6.0
        };
        let ph = 16.0 + 20.0 + doc_h;
        let above = ay - ph - 4.0;
        let y = if above > a.editor.y {
            above
        } else {
            ay + ch + 4.0
        };
        let x = ax.min(a.editor.right() - pw - 8.0).max(a.editor.x + 4.0);
        let r = Rect::new(x, y, pw, ph);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let row = Rect::new(r.x + 12.0, r.y + 8.0, r.w - 24.0 - count_w - 12.0, 20.0);
        let mut tx = row.x;
        for (k, part) in parts.iter().enumerate() {
            let (face, colour) = if k == 1 {
                (mono.bold(), p.bright)
            } else {
                (mono, p.text)
            };
            let w =
                self.text
                    .draw_in_row(&mut self.canvas, part, face, tx, row.y, row.h, colour, row);
            if k == 1 && !part.is_empty() {
                let under = Rect::new(tx, row.y + row.h - 2.0, w.min(row.right() - tx), 1.5);
                self.canvas.fill(under, p.accent);
            }
            tx += w;
        }
        self.text.draw_in_row(
            &mut self.canvas,
            &count,
            ui,
            r.right() - 12.0 - count_w,
            row.y,
            row.h,
            p.muted,
            r,
        );
        if !sig.doc.is_empty() {
            let clip = Rect::new(r.x + 12.0, row.bottom() + 4.0, doc_w, doc_h - 6.0);
            self.text.draw(
                &mut self.canvas,
                &sig.doc,
                ui,
                clip.x,
                clip.y,
                p.muted,
                clip,
                Some(doc_w),
            );
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
    pub(super) fn file_icon(
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

    /// A plugin's name in a blue chip, ending at `right`: what a task or
    /// test came from. Its width.
    fn plugin_chip(&mut self, label: &str, right: f32, y: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(11.0);
        let tw = self.text.width(label, face);
        let r = Rect::new(right - tw - 12.0, y, tw + 12.0, 16.0);
        self.canvas.round(r, 4.0, mix(p.accent, p.widget, 0.18));
        self.text.draw_in_row(
            &mut self.canvas,
            label,
            face,
            r.x + 6.0,
            y,
            16.0,
            p.accent,
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

    fn side(
        &mut self,
        sv: &SideView,
        side: Side,
        area: Rect,
        state: &mut State,
        term: Option<&TerminalView>,
    ) {
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
            // The Tests view's passed and failed, at the right.
            if let Some((passed, failed)) = sv.counts.filter(|(a, b)| a + b > 0) {
                let face = Face::ui(12.0).medium();
                let mut x = header.right() - 14.0;
                for (text, colour) in [
                    (format!("{failed} ✕"), p.error),
                    (format!("{passed} ✓"), p.success),
                ] {
                    let w = self.text.width(&text, face);
                    x -= w;
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &text,
                        face,
                        x,
                        area.y,
                        HEADER_H,
                        colour,
                        header,
                    );
                    x -= 8.0;
                }
            }
        }
        let body = Rect::new(
            area.x,
            area.y + HEADER_H,
            area.w,
            (area.h - HEADER_H - 6.0).max(0.0),
        );
        if let Some(term) = term {
            return self.terminal_panel(term, header, body);
        }
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

    /// The terminal panel: its buttons in the header, the shells showing
    /// side by side, and the list of shells when there are several. The
    /// shells' cells are drawn after, in each of [`Frame::term_panes`].
    fn terminal_panel(&mut self, tv: &TerminalView, header: Rect, body: Rect) {
        let p = self.p;
        // The header's buttons, right to left.
        let mut x = header.right() - 8.0;
        let close = (u8::MAX, "Close Panel (Ctrl+J)", Hit::Command("view.panel"));
        let buttons = [
            close,
            if self.panel_max {
                (5, "Restore Panel Size", Hit::MaximizePanel)
            } else {
                (4, "Maximise Panel Size", Hit::MaximizePanel)
            },
            (3, "Kill the Terminal", Hit::Command("terminal.kill")),
            (
                2,
                "Split Terminal (Ctrl+Shift+5)",
                Hit::Command("terminal.split"),
            ),
            (
                1,
                "New Terminal (Ctrl+Shift+`)",
                Hit::Command("terminal.new"),
            ),
        ];
        for (icon, tip, hit) in buttons {
            let r = Rect::new(x - 26.0, header.y + 6.0, 26.0, 26.0);
            x = r.x - 2.0;
            let hovered = self.hovered(r);
            if hovered {
                self.canvas.round(r, 6.0, p.hover);
            }
            let c = if hovered { p.bright } else { p.muted };
            self.toolbar_icon(icon, r, c);
            self.hit(r, hit);
            self.tips.push((r, tip.to_string()));
        }
        // The list of shells, when there's more than one.
        let list_w = if tv.sessions.len() > 1 { 168.0 } else { 0.0 };
        if list_w > 0.0 {
            let lx = body.right() - list_w;
            self.canvas.fill(Rect::new(lx, body.y, 1.0, body.h), p.line);
            for (k, session) in tv.sessions.iter().enumerate() {
                let r = Rect::new(
                    lx + 6.0,
                    body.y + 2.0 + k as f32 * 28.0,
                    list_w - 12.0,
                    26.0,
                );
                if r.bottom() > body.bottom() {
                    break;
                }
                let on = tv.active == Some(session.id);
                if on {
                    self.canvas.round(r, 6.0, p.chip);
                } else if self.hovered(r) {
                    self.canvas.round(r, 6.0, p.hover);
                }
                let dot = if session.exited.is_some() {
                    p.faint
                } else {
                    p.success
                };
                let cy = r.y + r.h / 2.0;
                self.canvas
                    .round(Rect::new(r.x + 9.0, cy - 3.5, 7.0, 7.0), 3.5, dot);
                let face = Face::ui(12.5);
                let title = self.text.fit(&session.title, face, r.w - 46.0);
                let colour = if on { p.bright } else { p.text };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &title,
                    face,
                    r.x + 24.0,
                    r.y,
                    r.h,
                    colour,
                    r,
                );
                let n = (k + 1).to_string();
                let nw = self.text.width(&n, Face::ui(11.0));
                self.text.draw_in_row(
                    &mut self.canvas,
                    &n,
                    Face::ui(11.0),
                    r.right() - 8.0 - nw,
                    r.y,
                    r.h,
                    p.muted,
                    r,
                );
                self.hit(r, Hit::Ui(UiEvent::Terminal(session.id)));
            }
        }
        // The panes, side by side with a hairline between.
        let panes = Rect::new(body.x, body.y, body.w - list_w, body.h);
        let n = tv.panes.len().max(1) as f32;
        let w = panes.w / n;
        for (k, &id) in tv.panes.iter().enumerate() {
            let r = Rect::new(panes.x + k as f32 * w, panes.y, w, panes.h);
            if k > 0 {
                self.canvas.fill(Rect::new(r.x, r.y, 1.0, r.h), p.line);
            }
            // A focused split pane gets a quiet edge along its top.
            if tv.panes.len() > 1 && tv.focused && tv.active == Some(id) {
                self.canvas
                    .fill(Rect::new(r.x + 8.0, r.y, r.w - 16.0, 1.5), p.accent);
            }
            let inner = Rect::new(r.x + 12.0, r.y + 6.0, r.w - 20.0, r.h - 8.0);
            self.hit(r, Hit::Terminal(id));
            if let Some((code, task)) = tv
                .sessions
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| Some((s.exited?, s.task)))
            {
                let note = match (task, code) {
                    (true, 0) => "The task is done. Run it again from Run Task.".to_string(),
                    (true, code) => format!("The task failed with code {code}."),
                    (false, code) => {
                        format!("The shell ended with code {code}. The bin closes it.")
                    }
                };
                let face = Face::ui(12.0);
                let y = r.bottom() - 26.0;
                self.text
                    .draw_in_row(&mut self.canvas, &note, face, inner.x, y, 22.0, p.muted, r);
                self.term_panes.push((
                    id,
                    Rect::new(inner.x, inner.y, inner.w, (inner.h - 28.0).max(0.0)),
                ));
            } else {
                self.term_panes.push((id, inner));
            }
        }
    }

    /// The terminal panel's button icons, drawn in strokes: 1 new, 2
    /// split, 3 kill, 4 maximise, 5 restore, else close.
    fn toolbar_icon(&mut self, icon: u8, r: Rect, c: u32) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let line = |canvas: &mut Canvas, a: (f32, f32), b: (f32, f32)| {
            canvas.line((cx + a.0, cy + a.1), (cx + b.0, cy + b.1), 1.4, c)
        };
        match icon {
            1 => {
                line(&mut self.canvas, (-5.0, 0.0), (5.0, 0.0));
                line(&mut self.canvas, (0.0, -5.0), (0.0, 5.0));
            }
            2 => {
                self.canvas
                    .frame(Rect::new(cx - 6.0, cy - 5.0, 12.0, 10.0), 2.0, 1.3, c, 0);
                line(&mut self.canvas, (0.0, -5.0), (0.0, 5.0));
            }
            3 => {
                line(&mut self.canvas, (-5.5, -3.5), (5.5, -3.5));
                line(&mut self.canvas, (-2.0, -5.5), (2.0, -5.5));
                line(&mut self.canvas, (-4.0, -3.5), (-3.2, 5.5));
                line(&mut self.canvas, (4.0, -3.5), (3.2, 5.5));
                line(&mut self.canvas, (-3.2, 5.5), (3.2, 5.5));
            }
            4 => {
                line(&mut self.canvas, (-4.5, 2.5), (0.0, -2.0));
                line(&mut self.canvas, (0.0, -2.0), (4.5, 2.5));
            }
            5 => {
                line(&mut self.canvas, (-4.5, -2.0), (0.0, 2.5));
                line(&mut self.canvas, (0.0, 2.5), (4.5, -2.0));
            }
            _ => {
                line(&mut self.canvas, (-4.0, -4.0), (4.0, 4.0));
                line(&mut self.canvas, (4.0, -4.0), (-4.0, 4.0));
            }
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
            Node::Tools(_) => 32.0,
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
            Node::Header { .. } => 38.0,
            Node::Banner { .. } => 96.0,
            Node::Wide { .. } => 38.0,
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
                let hits = self.entry(e, row, clip);
                let check = hits.check;
                if let Some(index) = target {
                    self.hit(
                        row,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                    if let Some(b) = check {
                        self.hit(b, Hit::Ui(UiEvent::Check { side, index }));
                    }
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
                    if let Some(f) = hits.fold {
                        self.hit(f, Hit::Ui(UiEvent::Fold { side, index }));
                    }
                    let first_link = index + 1 + usize::from(e.action.is_some());
                    for (j, l) in hits.links.iter().enumerate() {
                        self.hit(
                            *l,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index: first_link + j,
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
            Node::Tools(bs) => {
                let mut x = r.x + 12.0;
                let face = Face::ui(12.0);
                for (k, b) in bs.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let tw = self.text.width(&b.label, face);
                    let br = Rect::new(x, r.y + 4.0, tw + 16.0, 24.0);
                    if x + br.w > r.right() - 8.0 {
                        break;
                    }
                    let bg = if self.hovered(br) { p.border } else { p.chip };
                    if focused {
                        self.canvas
                            .outline(br.inset(-2.0), 7.0, 1.5, p.accent, p.island);
                    }
                    self.canvas.round(br, 5.0, bg);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &b.label,
                        face,
                        x + 8.0,
                        br.y,
                        br.h,
                        p.text,
                        br,
                    );
                    if let Some(index) = index {
                        self.hit(
                            br,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index,
                                clicks: 1,
                            }),
                        );
                    }
                    x += br.w + 6.0;
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
            Node::Header {
                title,
                detail,
                buttons,
            } => self.header(title, detail, buttons, r, clip),
            Node::Banner {
                title,
                detail,
                tone,
                buttons,
            } => {
                // A box tinted by its tone: the title, what's left, then
                // its buttons.
                let colour = tone_colour(&p, *tone);
                let b = Rect::new(r.x + 10.0, r.y + 2.0, r.w - 20.0, r.h - 10.0);
                self.canvas.outline(
                    b,
                    8.0,
                    1.0,
                    mix(colour, p.island, 0.22),
                    mix(colour, p.island, 0.07),
                );
                let inside = b.inset(1.0).intersect(clip);
                let mut x = b.x + 10.0;
                for (text, strong) in title {
                    let face = if *strong {
                        Face::ui(12.5).medium()
                    } else {
                        Face::ui(12.5)
                    };
                    let colour = if *strong { p.bright } else { p.text };
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        text,
                        face,
                        x,
                        b.y + 7.0,
                        18.0,
                        colour,
                        inside,
                    );
                }
                self.text.draw_in_row(
                    &mut self.canvas,
                    detail,
                    Face::ui(12.0),
                    b.x + 10.0,
                    b.y + 25.0,
                    18.0,
                    p.muted,
                    inside,
                );
                let mut x = b.x + 10.0;
                for (k, button) in buttons.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let bw = self.button(&button.label, x, b.y + 49.0, focused, false);
                    if let Some(index) = index {
                        self.hit(
                            Rect::new(x, b.y + 49.0, bw, 30.0),
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
            Node::Wide { button, ready } => {
                // The main button across the panel, faded until it's ready.
                let b = Rect::new(r.x + 14.0, r.y + 4.0, r.w - 28.0, 30.0);
                let focused = selected && sv.focused;
                if focused {
                    self.canvas.outline(
                        Rect::new(b.x - 2.0, b.y - 2.0, b.w + 4.0, 34.0),
                        9.0,
                        1.5,
                        p.accent,
                        p.island,
                    );
                }
                let bg = if !*ready {
                    mix(p.accent, p.island, 0.45)
                } else if self.hovered(b) {
                    mix(p.accent, 0xffffff, 0.88)
                } else {
                    p.accent
                };
                self.canvas.round(b, 7.0, bg);
                let face = Face::ui(13.0).medium();
                let label = self.text.fit(&button.label, face, b.w - 16.0);
                let tw = self.text.width(&label, face);
                let fg = if *ready {
                    p.ground
                } else {
                    mix(p.ground, p.island, 0.6)
                };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    b.x + (b.w - tw) / 2.0,
                    b.y,
                    30.0,
                    fg,
                    b,
                );
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
        }
    }

    /// A title, toned parts as code, with buttons and their keys at the
    /// right end, the last the main one.
    fn header(
        &mut self,
        title: &[(String, Tone)],
        detail: &str,
        buttons: &[(merlin_core::panel::Button, String)],
        r: Rect,
        clip: Rect,
    ) {
        let p = self.p;
        let (y, h) = (r.y + 4.0, 26.0);
        let mut right = r.right() - 14.0;
        for (k, (button, key)) in buttons.iter().enumerate().rev() {
            let w = self.key_button(&button.label, key, right, y, k + 1 == buttons.len());
            self.hit(
                Rect::new(right - w, y, w, h),
                Hit::Ui(UiEvent::Press(button.id.clone())),
            );
            right -= w + 8.0;
        }
        let mut x = r.x + 14.0;
        for (text, tone) in title {
            let (face, colour) = match tone {
                Tone::Normal => (Face::ui(13.0), p.text),
                tone => (Face::mono(12.5), tone_colour(&p, *tone)),
            };
            x += self
                .text
                .draw_in_row(&mut self.canvas, text, face, x, y, h, colour, clip);
        }
        if !detail.is_empty() {
            let face = Face::ui(13.0);
            let room = (right - x - 24.0).max(0.0);
            let shown = self.text.fit(&format!("  ·  {detail}"), face, room);
            self.text
                .draw_in_row(&mut self.canvas, &shown, face, x, y, h, p.muted, clip);
        }
    }

    /// A button of `label` and its key, ending at `right`: its width.
    fn key_button(&mut self, label: &str, key: &str, right: f32, y: f32, primary: bool) -> f32 {
        let p = self.p;
        let (face, chip) = (Face::ui(12.5), Face::ui(11.0));
        let (lw, kw) = (self.text.width(label, face), self.text.width(key, chip));
        let w = lw + kw + 12.0 + 8.0 + 20.0;
        let r = Rect::new(right - w, y, w, 26.0);
        let hovered = self.hovered(r);
        let (bg, fg, chip_bg, chip_fg) = if primary {
            let bg = if hovered {
                mix(p.accent, 0xffffff, 0.88)
            } else {
                p.accent
            };
            (bg, p.ground, mix(p.ground, bg, 0.18), p.ground)
        } else {
            let bg = if hovered { p.border } else { p.chip };
            (bg, p.bright, mix(p.chip_text, bg, 0.12), p.chip_text)
        };
        self.canvas.round(r, 6.0, bg);
        self.text
            .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, 26.0, fg, r);
        let k = Rect::new(r.x + 10.0 + lw + 8.0, y + 5.0, kw + 12.0, 16.0);
        self.canvas.round(k, 4.0, chip_bg);
        self.text.draw_in_row(
            &mut self.canvas,
            key,
            chip,
            k.x + 6.0,
            k.y,
            16.0,
            chip_fg,
            r,
        );
        w
    }

    /// A checkbox at `x`, centred on `mid`.
    fn checkbox(&mut self, check: Check, x: f32, mid: f32) {
        let p = self.p;
        let b = Rect::new(x, mid - 7.0, 14.0, 14.0);
        match check {
            Check::On => {
                self.canvas.round(b, 4.0, p.accent);
                self.canvas
                    .line((x + 3.5, mid + 0.2), (x + 6.0, mid + 2.8), 1.8, p.ground);
                self.canvas
                    .line((x + 6.0, mid + 2.8), (x + 10.8, mid - 2.6), 1.8, p.ground);
            }
            Check::Some => {
                self.canvas.frame(b, 4.0, 1.5, p.accent, 0);
                self.canvas
                    .round(Rect::new(x + 3.5, mid - 1.0, 7.0, 2.0), 1.0, p.accent);
            }
            Check::Off => {
                self.canvas
                    .frame(b, 4.0, 1.5, mix(p.muted, p.island, 0.5), 0);
            }
        }
    }

    /// A line with a name changed: its number, the old name struck through
    /// and tinted, then the new.
    fn swap(&mut self, s: &Swap, muted: bool, x: f32, row: Rect, room: f32, clip: Rect) -> f32 {
        let p = self.p;
        let mono = Face::mono(12.5);
        let number = s.line.to_string();
        let nw = self.text.width(&number, mono);
        let num_x = x + 26.0 - nw;
        self.text.draw_in_row(
            &mut self.canvas,
            &number,
            mono,
            num_x,
            row.y,
            row.h,
            p.muted,
            clip,
        );
        let start = x + 34.0;
        let end = start + (room - 34.0).max(0.0);
        let clip = Rect::new(clip.x, clip.y, (end - clip.x).max(0.0), clip.h);
        let text = if muted { p.muted } else { p.text };
        let mid = row.y + row.h / 2.0;
        let mut at = start;
        at += self.text.draw_in_row(
            &mut self.canvas,
            &s.before,
            mono,
            at,
            row.y,
            row.h,
            text,
            clip,
        );
        let ow = self.text.width(&s.old, mono);
        if !muted {
            let tint = Rect::new(at, mid - 8.0, ow, 16.0);
            self.canvas.tint(tint, 2.0, p.error, 77);
        }
        self.text
            .draw_in_row(&mut self.canvas, &s.old, mono, at, row.y, row.h, text, clip);
        if at < end {
            let strike = Rect::new(at, mid, ow.min(end - at), 1.0);
            self.canvas.fill(strike, text);
        }
        at += ow;
        if !s.new.is_empty() {
            let w = self.text.width(&s.new, mono);
            self.canvas
                .tint(Rect::new(at, mid - 8.0, w, 16.0), 2.0, p.success, 71);
            at +=
                self.text
                    .draw_in_row(&mut self.canvas, &s.new, mono, at, row.y, row.h, text, clip);
        }
        // What's past the room is cut with an ellipsis.
        let after = self.text.fit(&s.after, mono, (end - at).max(0.0));
        at += self
            .text
            .draw_in_row(&mut self.canvas, &after, mono, at, row.y, row.h, text, clip);
        at - x
    }

    /// A tree row: indent, arrow, checkbox, icon, name, detail, counts,
    /// and a badge and buttons at the end. Returns where its checkbox,
    /// arrow and buttons are, to click.
    fn entry(&mut self, e: &Entry, row: Rect, clip: Rect) -> EntryHits {
        let p = self.p;
        let mut x = row.x + 8.0 + e.depth as f32 * 14.0;
        let mid = row.y + row.h / 2.0;
        let mut hits = EntryHits::default();
        // A section's heading: its title in capitals and its count, with
        // its button on hover.
        if e.heading {
            let face = Face::ui(11.0).bold();
            let colour = match e.tone {
                Tone::Normal => p.muted,
                tone => tone_colour(&p, tone),
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &e.label.to_uppercase(),
                face,
                x,
                row.y,
                row.h,
                colour,
                clip,
            );
            let bw = self.text.width(&e.badge, face);
            let right = row.right() - 8.0;
            self.text.draw_in_row(
                &mut self.canvas,
                &e.badge,
                face,
                right - bw,
                row.y,
                row.h,
                p.muted,
                clip,
            );
            if let Some((glyph, _)) = &e.action
                && self.hovered(row)
            {
                let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
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
            return hits;
        }
        if let Some(open) = e.fold {
            self.canvas.chevron(x + 5.0, mid, 7.0, open, p.muted);
            hits.fold = Some(Rect::new(x - 4.0, row.y, 20.0, row.h));
            x += 16.0;
        }
        // Folders have an open or closed arrow; files a blank in its place.
        let folder = match e.twisty.filter(|_| e.fold.is_none()) {
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
        hits.check = e.check.map(|check| {
            self.checkbox(check, x, mid);
            x += 22.0;
            Rect::new(x - 26.0, row.y, 26.0, row.h)
        });
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
                // A test's mark stays a mark, whatever its file's icon.
                let mark = matches!(e.icon, Some(('✓' | '✕' | '○' | '⟳' | '●', _)));
                let name = if mark { "" } else { e.label.as_str() };
                x += self.file_icon(name, e.icon, x, mid - 8.0, 16.0, clip);
            }
            None => {}
        }
        let face = if e.code {
            Face::mono(12.0)
        } else {
            Face::ui(13.0)
        };
        let detail_face = if e.code {
            Face::mono(11.5)
        } else {
            Face::ui(12.0)
        };
        let badge_w = if e.badge.is_empty() {
            0.0
        } else {
            self.text.width(&e.badge, Face::ui(11.0).bold()) + 14.0
        };
        let link_face = Face::ui(11.5);
        let hovered = self.hovered(row);
        let links: Vec<&(String, String)> = e
            .links
            .iter()
            .enumerate()
            .filter(|(j, _)| *j == 0 || hovered)
            .map(|(_, l)| l)
            .collect();
        let links_w: f32 = links
            .iter()
            .map(|(l, _)| self.text.width(l, link_face) + 12.0)
            .sum();
        let room = (row.right()
            - x
            - badge_w
            - links_w
            - if e.action.is_some() { 26.0 } else { 0.0 }
            - 6.0)
            .max(0.0);
        let colour = match e.tone {
            _ if e.strike => p.muted,
            Tone::Normal => p.text,
            tone => tone_colour(&p, tone),
        };
        let lw = match &e.swap {
            Some(s) => {
                let room = room - if e.detail.is_empty() { 0.0 } else { 70.0 };
                self.swap(s, e.tone == Tone::Muted, x, row, room, clip)
            }
            None => {
                let label = self.text.fit(&e.label, face, room);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    x,
                    row.y,
                    row.h,
                    colour,
                    clip,
                )
            }
        };
        let mut used = lw;
        // A deleted file's name is struck through.
        if e.strike {
            self.canvas.fill(Rect::new(x, mid, lw, 1.0), p.muted);
        }
        // A chip after the name, as a mode change's.
        if let Some(chip) = &e.chip {
            let face = Face::mono(10.5);
            let cw = self.text.width(chip, face) + 10.0;
            if used + 8.0 + cw < room {
                let r = Rect::new(x + used + 8.0, mid - 8.0, cw, 16.0);
                self.canvas.round(r, 4.0, p.chip);
                self.text.draw_in_row(
                    &mut self.canvas,
                    chip,
                    face,
                    r.x + 5.0,
                    r.y,
                    16.0,
                    p.muted,
                    clip,
                );
                used += 8.0 + cw;
            }
        }
        // A note after the name in its tone, as a conflict's "2 left".
        if let Some((note, tone)) = &e.note {
            let face = Face::ui(11.5);
            let nw = self.text.width(note, face);
            if used + 8.0 + nw < room {
                self.text.draw_in_row(
                    &mut self.canvas,
                    note,
                    face,
                    x + used + 8.0,
                    row.y,
                    row.h,
                    tone_colour(&p, *tone),
                    clip,
                );
                used += 8.0 + nw;
            }
        }
        if !e.detail.is_empty() && used + 12.0 < room {
            let detail = self.text.fit(&e.detail, detail_face, room - used - 8.0);
            used += 8.0
                + self.text.draw_in_row(
                    &mut self.canvas,
                    &detail,
                    detail_face,
                    x + used + 8.0,
                    row.y,
                    row.h,
                    p.muted,
                    clip,
                );
        }
        // Lines added and removed, in green and red.
        if let Some((added, removed)) = e.counts {
            let count = Face::ui(11.5);
            let mut at = x + used + 8.0;
            for (n, sign, colour) in [(added, '+', p.success), (removed, '−', p.error)] {
                if n == 0 {
                    continue;
                }
                let text = format!("{sign}{n}");
                if at + self.text.width(&text, count) > x + room {
                    break;
                }
                at += self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    count,
                    at,
                    row.y,
                    row.h,
                    colour,
                    clip,
                ) + 6.0;
            }
        }
        let mut right = row.right() - 8.0;
        // Buttons as words: the first always, in green, at the end, so it
        // stays put; the rest before it while the row is hovered.
        for (j, (label, _)) in links.iter().enumerate() {
            let w = self.text.width(label, link_face);
            let r = Rect::new(right - w - 6.0, row.y + 3.0, w + 12.0, row.h - 6.0);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.chip);
            }
            let colour = if j == 0 { p.success } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                link_face,
                right - w,
                row.y,
                row.h,
                colour,
                clip,
            );
            hits.links.push(r);
            right -= w + 12.0;
        }
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
        hits
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
    fn tab_strip(
        &mut self,
        tabs: &[TabView],
        row: Rect,
        group: Option<usize>,
        focused: bool,
        bar: Option<&DiffBarView>,
        merge: Option<&MergeBarView>,
    ) {
        let p = self.p;
        // Empty strip takes drops too, at the end, and a double click.
        self.hit(row, Hit::TabStrip(group));
        // A changes tab's bar takes the strip's right end, before its
        // buttons.
        let bar_w = match (bar, merge) {
            (Some(b), _) => self.diff_bar(b, row, row.right() - 72.0),
            (_, Some(m)) => self.merge_bar(m, row, row.right() - 72.0),
            _ => 0.0,
        };
        // A tab in the merge editor says so after its name.
        let merging_w = self.text.width("Merging", Face::ui(11.0)) + 6.0;
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
                    let nw = self.text.width(&t.name, face_of(t))
                        + if t.merging { merging_w } else { 0.0 };
                    TAB_PAD + 14.0 + 7.0 + nw + 5.0 + 18.0 + 6.0
                }
            })
            .collect();
        let (left, right) = (row.x + 6.0, row.right() - 72.0 - bar_w);
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
            // A tab named "theme.rs ↔ Saved" or "theme.rs (changes)" is
            // its file's: the icon goes by the first word.
            let file = tab.name.split(' ').next().unwrap_or(&tab.name);
            self.file_icon(
                file,
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
            let mut nw = self.text.width(&tab.name, face);
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
            if tab.merging {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "Merging",
                    Face::ui(11.0),
                    x + TAB_PAD + 21.0 + nw + 6.0,
                    r.y,
                    r.h,
                    p.error,
                    clip,
                );
                nw += merging_w;
            }
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

    /// The changes tab's bar, ending at `right`: which change, arrows to
    /// the next and previous, side by side or inline, and Stage File and
    /// Revert File. Returns its width.
    /// The merge editor's bar: what's left, the arrows, Accept All
    /// Incoming and Complete Merge, ending at `right`. Its width.
    /// Over a file that changed on disk while merlin was closed: what
    /// happened, and Compare, Use the Disk's and Keep Mine.
    fn away_bar(&mut self, id: u64, name: &str, r: Rect) {
        let p = self.p;
        let face = Face::ui(12.5);
        self.canvas.fill(r, mix(p.warning, p.island, 0.12));
        self.canvas.fill(
            Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0),
            mix(p.warning, p.island, 0.22),
        );
        let (h, y) = (26.0, r.y + (r.h - 26.0) / 2.0);
        let mut x = r.right() - 12.0;
        for (label, what, primary) in [
            ("Keep Mine", "keep", true),
            ("Use the Disk's", "disk", false),
            ("Compare", "compare", false),
        ] {
            let face = if primary { face.medium() } else { face };
            let w = self.text.width(label, face) + 20.0;
            x -= w;
            let b = Rect::new(x, y, w, h);
            let hovered = self.hovered(b);
            let (bg, fg) = match (primary, hovered) {
                (true, false) => (p.accent, p.on_accent),
                (true, true) => (mix(p.accent, 0xffffff, 0.88), p.on_accent),
                (false, false) => (p.chip, p.bright),
                (false, true) => (p.border, p.bright),
            };
            self.canvas.round(b, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, face, b.x + 10.0, y, h, fg, b);
            self.hit(b, Hit::Ui(UiEvent::Press(format!("away:{what}:{id}"))));
            x -= 8.0;
        }
        let text_r = Rect::new(r.x + 12.0, r.y, (x - r.x - 12.0).max(0.0), r.h);
        self.text.draw_in_row(
            &mut self.canvas,
            "⚠",
            face,
            text_r.x,
            r.y,
            r.h,
            p.warning,
            text_r,
        );
        let bold = face.medium();
        let tx = text_r.x + 22.0;
        self.text
            .draw_in_row(&mut self.canvas, name, bold, tx, r.y, r.h, p.bright, text_r);
        let tx = tx + self.text.width(name, bold);
        self.text.draw_in_row(
            &mut self.canvas,
            " changed on disk since you left it. Your unsaved changes are kept.",
            face,
            tx,
            r.y,
            r.h,
            p.text,
            text_r,
        );
    }

    fn merge_bar(&mut self, bar: &MergeBarView, row: Rect, right: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(12.0);
        let h = 24.0;
        let y = row.y + (row.h - h) / 2.0;
        let mut x = right - 6.0;
        // A file with markers opened as text: only the way in.
        let buttons: &[(&str, &'static str, bool)] = if bar.offer {
            &[("Open in Merge Editor", "merge.open_here", true)]
        } else if bar.inline {
            &[("Complete", "merge.complete", true)]
        } else {
            &[
                ("Complete Merge", "merge.complete", true),
                ("Accept All Incoming", "merge.accept_all_incoming", false),
            ]
        };
        for &(label, command, primary) in buttons {
            let face = if primary { face.medium() } else { face };
            let w = self.text.width(label, face) + 20.0;
            x -= w;
            let r = Rect::new(x, y, w, h);
            let hovered = self.hovered(r);
            let (bg, fg) = match (primary, hovered) {
                (true, false) => (p.accent, p.on_accent),
                (true, true) => (mix(p.accent, 0xffffff, 0.88), p.on_accent),
                (false, false) => (p.chip, p.bright),
                (false, true) => (p.border, p.bright),
            };
            self.canvas.round(r, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, h, fg, r);
            self.hit(r, Hit::Command(command));
            x -= 6.0;
        }
        if bar.offer {
            return right - x;
        }
        x -= 4.0;
        for (glyph, command) in [("↓", "merge.next"), ("↑", "merge.prev")] {
            x -= 20.0;
            let r = Rect::new(x, y, 20.0, h);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let gw = self.text.width(glyph, face);
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                face,
                r.x + (20.0 - gw) / 2.0,
                y,
                h,
                p.muted,
                r,
            );
            self.hit(r, Hit::Command(command));
        }
        let (label, colour) = match (bar.left, bar.inline) {
            (0, _) => ("No conflicts left".to_string(), p.success),
            (n, true) => (format!("{n} left"), p.error),
            (1, false) => ("1 conflict left".to_string(), p.error),
            (n, false) => (format!("{n} conflicts left"), p.error),
        };
        let lw = self.text.width(&label, face);
        x -= lw + 10.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            y,
            h,
            colour,
            Rect::new(x, y, lw + 1.0, h),
        );
        right - x
    }

    /// The merge editor around the Result's code: Incoming and Current,
    /// each line of a conflict with its box and Accept under it, the
    /// panes' titles, and the Result's notes under its conflicts, from
    /// `y0`, the Result's first row.
    pub fn merge_editor(
        &mut self,
        merge: &MergeView,
        area: &GroupArea,
        y0: f32,
        row_h: f32,
        gutter: f32,
    ) {
        let p = self.p;
        let Some([incoming, current, result]) = area.merge else {
            return;
        };
        for (pane, r) in merge.panes.iter().zip([incoming, current]) {
            self.merge_pane(pane, r, row_h);
        }
        // The Result's frame and title, around the code drawn already.
        self.canvas.frame(result, 8.0, 1.0, p.line, 0);
        let head = Rect::new(
            result.x + 1.0,
            result.y + 1.0,
            result.w - 2.0,
            MERGE_HEAD_H - 1.0,
        );
        self.canvas.fill(head, p.widget);
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.line);
        let face = Face::ui(12.0);
        let mut x = head.x + 10.0;
        x += self.text.draw_in_row(
            &mut self.canvas,
            "Result",
            face.medium(),
            x,
            head.y,
            head.h,
            p.bright,
            head,
        ) + 8.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &if merge.inline {
                "with each conflict in place".to_string()
            } else {
                format!("{} · you can edit here", merge.name)
            },
            face,
            x,
            head.y,
            head.h,
            p.muted,
            head,
        );
        let hint = "Base shows through until you choose";
        let hw = self.text.width(hint, face);
        if head.w > 520.0 && !merge.inline {
            self.text.draw_in_row(
                &mut self.canvas,
                hint,
                face,
                head.right() - 10.0 - hw,
                head.y,
                head.h,
                p.muted,
                head,
            );
        }
        // Each conflict's note and links, in the row under it.
        let code = area.text;
        let link = Face::ui(11.5);
        let clip = Rect::new(code.x, code.y, code.w, code.h);
        let text_x = code.x + gutter;
        for note in &merge.notes {
            let y = y0 + note.row as f32 * row_h;
            let rows = 1 + note.sides.len();
            if y + rows as f32 * row_h <= code.y || y >= code.bottom() {
                continue;
            }
            self.canvas.fill(
                Rect::new(code.x, y, code.w, rows as f32 * row_h).intersect(clip),
                p.island,
            );
            let mut x = text_x;
            if !note.text.is_empty() {
                x += self.text.draw_in_row(
                    &mut self.canvas,
                    &note.text,
                    link,
                    x,
                    y,
                    row_h,
                    p.muted,
                    clip,
                ) + 6.0;
            }
            for (k, (label, id)) in note.links.iter().enumerate() {
                if k > 0 || !note.text.is_empty() {
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        "·",
                        link,
                        x,
                        y,
                        row_h,
                        p.muted,
                        clip,
                    ) + 6.0;
                }
                let w = self.text.width(label, link);
                let r = Rect::new(x - 2.0, y + 2.0, w + 4.0, row_h - 4.0);
                if self.hovered(r) {
                    self.canvas.round(r, 4.0, p.hover);
                }
                // Side by side changes the view, not the text: muted.
                let colour = if id.starts_with("merge:side") {
                    p.muted
                } else {
                    p.accent
                };
                self.text
                    .draw_in_row(&mut self.canvas, label, link, x, y, row_h, colour, clip);
                self.hit(r.intersect(clip), Hit::Ui(UiEvent::Press(id.clone())));
                x += w + 6.0;
            }
            // In the Result alone: both sides under their titles.
            for (k, row) in note.sides.iter().enumerate() {
                let ry = y + (k + 1) as f32 * row_h;
                if ry >= code.bottom() {
                    break;
                }
                let line = Rect::new(code.x, ry, code.w, row_h).intersect(clip);
                match row {
                    MergeRow::Title(side, label) => {
                        let (who, edge) = match side {
                            MergeSide::Incoming => ("Incoming", p.git_changed),
                            MergeSide::Current => ("Current", p.success),
                        };
                        self.canvas.fill(line, mix(edge, p.island, 0.10));
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &format!("{who} · {label}"),
                            Face::ui(11.0),
                            text_x,
                            ry,
                            row_h,
                            mix(edge, p.bright, 0.7),
                            clip,
                        );
                    }
                    MergeRow::Line {
                        number,
                        text,
                        colours,
                        ..
                    } => {
                        let incoming = note.sides[..k]
                            .iter()
                            .rfind(|r| matches!(r, MergeRow::Title(..)))
                            .is_some_and(|r| matches!(r, MergeRow::Title(MergeSide::Incoming, _)));
                        let edge = if incoming { p.git_changed } else { p.success };
                        let tint = if incoming { 0.16 } else { 0.15 };
                        self.canvas.fill(line, mix(edge, p.island, tint));
                        self.canvas.fill(
                            Rect::new(text_x - 6.0, ry, 3.0, row_h).intersect(clip),
                            edge,
                        );
                        if incoming {
                            let n = number.to_string();
                            let nw = self.text.width(&n, CODE);
                            self.text.draw_in_row(
                                &mut self.canvas,
                                &n,
                                CODE,
                                text_x - 14.0 - nw,
                                ry,
                                row_h,
                                p.faint,
                                clip,
                            );
                        }
                        self.code_line(text, colours, text_x, ry, row_h, clip);
                    }
                    MergeRow::Accept(_) => {}
                }
            }
        }
    }

    /// A text view's test rows: each test's ▶ Run Test and Debug and how
    /// it last went, in the row over its `fn`; a failure's message in a
    /// red box under the line that failed; and ✓ or ✕ in the gutter.
    pub fn test_rows(&mut self, text: &TextView, code: Rect, y0: f32, row_h: f32, cell_w: f32) {
        let p = self.p;
        let text_x = code.x + (text.gutter as f32 - text.hscroll as f32) * cell_w;
        let clip = Rect::new(
            code.x + text.gutter as f32 * cell_w,
            code.y,
            code.w - text.gutter as f32 * cell_w,
            code.h,
        );
        let link = Face::ui(11.5);
        for lens in &text.tests {
            let y = y0 + lens.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let mut x = text_x + lens.col as f32 * cell_w;
            for (label, id) in &lens.links {
                let w = self.text.width(label, link);
                let r = Rect::new(x - 3.0, y + 2.0, w + 6.0, row_h - 4.0);
                let hovered = self.hovered(r);
                if hovered {
                    self.canvas.round(r.intersect(clip), 4.0, p.hover);
                }
                let colour = if hovered { p.bright } else { p.muted };
                self.text
                    .draw_in_row(&mut self.canvas, label, link, x, y, row_h, colour, clip);
                self.hit(r.intersect(clip), Hit::Ui(UiEvent::Press(id.clone())));
                x += w + 14.0;
            }
            if let Some((status, failed)) = &lens.status {
                let colour = if *failed { p.error } else { p.success };
                self.text
                    .draw_in_row(&mut self.canvas, status, link, x, y, row_h, colour, clip);
            }
        }
        for failure in &text.failures {
            let y = y0 + failure.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let x = text_x + failure.col as f32 * cell_w;
            let face = Face::mono(11.5);
            let max = (clip.right() - x - 20.0).max(40.0);
            let shown = self.text.fit(&failure.text, face, max);
            let w = self.text.width(&shown, face);
            let r = Rect::new(x - 2.0, y + 2.0, w + 16.0, row_h - 4.0);
            self.canvas
                .round(r.intersect(clip), 4.0, mix(p.error, p.island, 0.18));
            self.text.draw_in_row(
                &mut self.canvas,
                &shown,
                face,
                x + 6.0,
                y,
                row_h,
                mix(p.error, p.bright, 0.35),
                clip,
            );
        }
        // ✓ or ✕ at the gutter's left, on each test's line.
        for line in text.lines.iter().filter(|l| l.test.is_some()) {
            let y = y0 + line.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let passed = line.test == Some(true);
            let (glyph, colour) = if passed {
                ("✓", p.success)
            } else {
                ("✕", p.error)
            };
            let x = code.x + text.blame_cols as f32 * cell_w + 4.0;
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                Face::ui(10.5).bold(),
                x,
                y,
                row_h,
                colour,
                code,
            );
        }
    }

    /// Two compared files' titles, over the halves of the code at `halves`.
    pub fn compare_titles(&mut self, titles: &[(String, String); 2], halves: &[Rect]) {
        let p = self.p;
        // Inline, one view: the right file's title over it all.
        let pairs: Vec<(&(String, String), Rect)> = match halves {
            [one] => vec![(&titles[1], *one)],
            _ => titles.iter().zip(halves.iter().copied()).collect(),
        };
        for ((name, note), r) in pairs {
            let head = Rect::new(r.x, r.y - MERGE_HEAD_H, r.w, MERGE_HEAD_H - 4.0);
            self.canvas.round(head, 6.0, p.widget);
            let face = Face::ui(12.0);
            let x = head.x + 10.0;
            let w = self.text.draw_in_row(
                &mut self.canvas,
                name,
                face.medium(),
                x,
                head.y,
                head.h,
                p.bright,
                head,
            );
            self.text.draw_in_row(
                &mut self.canvas,
                note,
                face,
                x + w + 8.0,
                head.y,
                head.h,
                p.muted,
                head,
            );
        }
    }

    /// Incoming or Current: its title, then its lines from the one level
    /// with the Result's top.
    fn merge_pane(&mut self, pane: &merlin_core::MergePaneView, r: Rect, row_h: f32) {
        let p = self.p;
        self.hit(r, Hit::Block);
        self.canvas.outline(r, 8.0, 1.0, p.line, p.island);
        let head = Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, MERGE_HEAD_H - 1.0);
        self.canvas.fill(head, p.widget);
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.line);
        let face = Face::ui(12.0);
        let (who, tint, edge, accept, key) = match pane.side {
            MergeSide::Incoming => (
                "Incoming",
                mix(p.git_changed, p.island, 0.16),
                p.git_changed,
                "Accept Incoming",
                "i",
            ),
            MergeSide::Current => (
                "Current",
                mix(p.success, p.island, 0.15),
                p.success,
                "Accept Current",
                "c",
            ),
        };
        let x = head.x + 10.0;
        let w = self.text.draw_in_row(
            &mut self.canvas,
            who,
            face.medium(),
            x,
            head.y,
            head.h,
            p.bright,
            head,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &pane.detail,
            face,
            x + w + 8.0,
            head.y,
            head.h,
            p.muted,
            head,
        );
        let code = Rect::new(
            r.x + 1.0,
            head.bottom() + 1.0,
            r.w - 2.0,
            r.bottom() - head.bottom() - 2.0,
        );
        let mono = CODE;
        let digits = pane
            .rows
            .iter()
            .filter_map(|row| match row {
                MergeRow::Line { number, .. } => Some(number.to_string().len()),
                _ => None,
            })
            .max()
            .unwrap_or(1)
            .max(2);
        let cw = self.text.width("0", mono);
        let num_w = digits as f32 * cw + 14.0;
        let box_x = code.x + num_w;
        let text_x = box_x + 22.0;
        let mut y = code.y + 4.0;
        for row in &pane.rows {
            if y >= code.bottom() {
                break;
            }
            let line = Rect::new(code.x, y, code.w, row_h);
            match row {
                MergeRow::Line {
                    number,
                    text,
                    colours,
                    conflict,
                } => {
                    if conflict.is_some() {
                        self.canvas.fill(line.intersect(code), tint);
                        self.canvas
                            .fill(Rect::new(code.x, y, 3.0, row_h).intersect(code), edge);
                    }
                    let n = number.to_string();
                    let nw = self.text.width(&n, mono);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &n,
                        mono,
                        box_x - 10.0 - nw,
                        y,
                        row_h,
                        p.faint,
                        code,
                    );
                    if let Some((i, k, ticked)) = conflict {
                        let b = Rect::new(box_x, y + (row_h - 14.0) / 2.0, 14.0, 14.0);
                        let hovered = self.hovered(b);
                        if *ticked {
                            self.canvas.round(b, 3.0, p.accent);
                            let tw = self.text.width("✓", Face::ui(11.0));
                            self.text.draw_in_row(
                                &mut self.canvas,
                                "✓",
                                Face::ui(11.0),
                                b.x + (14.0 - tw) / 2.0,
                                b.y,
                                14.0,
                                p.on_accent,
                                code,
                            );
                        } else {
                            let border = if hovered { p.accent } else { p.faint };
                            self.canvas.outline(b, 3.0, 1.0, border, p.island);
                        }
                        let id = format!("merge:box:{key}:{i}:{k}");
                        self.hit(line.intersect(code), Hit::Ui(UiEvent::Press(id)));
                    }
                    self.code_line(text, colours, text_x, y, row_h, code);
                }
                MergeRow::Title(..) => {}
                MergeRow::Accept(i) => {
                    let link = Face::ui(11.5);
                    let w = self.text.width(accept, link);
                    let r = Rect::new(text_x - 2.0, y + 2.0, w + 4.0, row_h - 4.0);
                    if self.hovered(r) {
                        self.canvas.round(r, 4.0, p.hover);
                    }
                    self.text.draw_in_row(
                        &mut self.canvas,
                        accept,
                        link,
                        text_x,
                        y,
                        row_h,
                        p.accent,
                        code,
                    );
                    let what = if key == "i" { "incoming" } else { "current" };
                    self.hit(
                        r.intersect(code),
                        Hit::Ui(UiEvent::Press(format!("merge:{what}:{i}"))),
                    );
                }
            }
            y += row_h;
        }
    }

    fn diff_bar(&mut self, bar: &DiffBarView, row: Rect, right: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(12.0);
        let h = 22.0;
        let y = row.y + (row.h - h) / 2.0;
        let mut x = right - 6.0;
        let button = |f: &mut Self, label: &str, command: &'static str, x: &mut f32| {
            let w = f.text.width(label, face) + 18.0;
            *x -= w;
            let r = Rect::new(*x, y, w, h);
            let hovered = f.hovered(r);
            f.canvas
                .round(r, 6.0, if hovered { p.border } else { p.chip });
            f.text
                .draw_in_row(&mut f.canvas, label, face, r.x + 9.0, y, h, p.bright, r);
            f.hit(r, Hit::Command(command));
            *x -= 6.0;
        };
        // A narrow strip keeps its room for the tabs; the file's buttons
        // are in Source Control too.
        if bar.compare {
            if bar.swap {
                button(self, "⇄ Swap", "compare.swap", &mut x);
            }
        } else if row.w >= 900.0 {
            button(self, "Revert File", "diff.revert_file", &mut x);
            button(self, "Stage File", "diff.stage_file", &mut x);
        }
        // Side by side or inline, the one shown lit.
        let segs = [("Inline", true), ("Side by side", false)];
        let widths: Vec<f32> = segs
            .iter()
            .map(|(l, _)| self.text.width(l, face) + 20.0)
            .collect();
        let total: f32 = widths.iter().sum();
        x -= total;
        let seg = Rect::new(x, y, total, h);
        self.canvas.frame(seg, 6.0, 1.0, p.border, 0);
        let mut sx = seg.right();
        for ((label, inline), w) in segs.iter().zip(&widths) {
            sx -= w;
            let r = Rect::new(sx, y, *w, h);
            let on = bar.inline == *inline;
            if on {
                self.canvas.round(r.inset(1.0), 5.0, p.raised);
            } else {
                self.hit(r, Hit::Command("diff.inline"));
            }
            let colour = if on { p.bright } else { p.muted };
            self.text
                .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, h, colour, r);
        }
        x -= 10.0;
        // The arrows, then which change it is.
        for (glyph, command) in [("↓", "diff.next"), ("↑", "diff.prev")] {
            x -= 20.0;
            let r = Rect::new(x, y, 20.0, h);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let gw = self.text.width(glyph, face);
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                face,
                r.x + (20.0 - gw) / 2.0,
                y,
                h,
                p.muted,
                r,
            );
            self.hit(r, Hit::Command(command));
        }
        // A note such as "Moved from x.rs, no changes" says it already.
        let label = match bar.changes {
            0 if bar.note.is_some() => String::new(),
            0 => "No changes".to_string(),
            n => format!("Change {} of {n}", bar.change.max(1)),
        };
        let lw = self.text.width(&label, face);
        x -= lw + 6.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            y,
            h,
            p.text,
            Rect::new(x, y, lw + 1.0, h),
        );
        // What kind of change, when it isn't plain edits, while there's
        // room for the tabs.
        if let Some(note) = &bar.note {
            let nw = self.text.width(note, face);
            if right - x + nw + 16.0 < row.w - 260.0 {
                x -= nw + 14.0;
                self.text.draw_in_row(
                    &mut self.canvas,
                    note,
                    face,
                    x,
                    y,
                    h,
                    p.muted,
                    Rect::new(x, y, nw + 1.0, h),
                );
            }
        }
        right - x
    }

    /// A changes tab's boxes and « buttons in the gap between its halves,
    /// each changed row's box before the new side's numbers, and the rows
    /// with no line hatched. `text` is the group's text area, and `y0` the
    /// top of the view's first row.
    pub fn diff_marks(&mut self, view: &TextView, text: Rect, cell_w: f32, row_h: f32, y0: f32) {
        let Some(marks) = &view.diff else { return };
        let p = self.p;
        let col_x = |c: u16| text.x + c as f32 * cell_w;
        let row_y = |r: i32| y0 + r as f32 * row_h;
        let side = Rect::new(
            col_x(view.area.x),
            text.y,
            view.area.w as f32 * cell_w,
            text.h,
        );
        // Hatching where a side has no line.
        for &r in &marks.fillers {
            let row = Rect::new(side.x, row_y(r), side.w, row_h);
            if row.bottom() <= text.y || row.y >= text.bottom() {
                continue;
            }
            self.canvas.fill(row, p.island);
            let colour = mix(p.line, p.island, 0.6);
            let mut x0 = row.x - row.h;
            while x0 < row.right() {
                let (mut a, mut b) = ((x0, row.bottom()), (x0 + row.h, row.y));
                if a.0 < row.x {
                    a = (row.x, row.bottom() - (row.x - a.0));
                }
                if b.0 > row.right() {
                    b = (row.right(), row.y + (b.0 - row.right()));
                }
                self.canvas.line(a, b, 1.0, colour);
                x0 += 6.0;
            }
        }
        let gap = Rect::new(
            col_x(marks.gap.0),
            text.y,
            marks.gap.1 as f32 * cell_w,
            text.h,
        );
        for m in &marks.rows {
            let y = row_y(m.row);
            if y + row_h <= text.y || y >= text.bottom() {
                continue;
            }
            let mid = y + row_h / 2.0;
            if let Some((index, check, rows)) = m.change {
                // The change's band across the gap, its « and its box.
                let band = Rect::new(gap.x, y, gap.w, rows as f32 * row_h);
                let band = Rect::new(band.x, band.y.max(text.y), band.w, band.h);
                self.canvas.fill(band, mix(p.accent, p.island, 0.1));
                let revert = Rect::new(gap.x + gap.w / 2.0 - 24.0, mid - 9.0, 22.0, 18.0);
                let hovered = self.hovered(revert);
                self.canvas
                    .round(revert, 4.0, if hovered { p.border } else { p.chip });
                let face = Face::ui(12.0);
                let gw = self.text.width("«", face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    "«",
                    face,
                    revert.x + (22.0 - gw) / 2.0,
                    revert.y,
                    18.0,
                    p.bright,
                    revert,
                );
                self.tips
                    .push((revert, "Revert to the last commit (Alt+R)".into()));
                self.hit(revert, Hit::Ui(UiEvent::DiffRevert(index)));
                let bx = gap.x + gap.w / 2.0 + 4.0;
                self.checkbox(check, bx, mid);
                let b = Rect::new(bx - 3.0, y, 20.0, row_h);
                self.tips.push((b, "Stage this change (Alt+S)".into()));
                self.hit(b, Hit::Ui(UiEvent::DiffStage(index)));
            }
            if let Some((line, staged)) = m.line {
                let x = col_x(view.area.x) + cell_w * 0.5;
                // A staged row has a blue edge.
                if staged {
                    self.canvas
                        .fill(Rect::new(col_x(view.area.x), y, 3.0, row_h), p.accent);
                }
                let check = if staged { Check::On } else { Check::Off };
                self.checkbox(check, x, mid);
                self.hit(
                    Rect::new(x - 3.0, y, 20.0, row_h),
                    Hit::Ui(UiEvent::DiffLine(line)),
                );
            }
        }
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
            let bar = view.diff_bar.as_ref().filter(|_| group.focused);
            let merge = view.merge_bar.as_ref().filter(|_| group.focused);
            self.tab_strip(&group.tabs, area.tabs, Some(g), group.focused, bar, merge);
            if let Some(row) = area.crumbs {
                self.crumb_row(&group.crumbs, row, Some(g));
            }
            if let (Some((id, name)), Some(r)) = (&group.away, area.away) {
                self.away_bar(*id, name, r);
            }
        }
        // Where a dragged tab or file would land: a side of a group, or
        // all of it.
        if let Some(zone) = self.drop_zone {
            self.canvas.frame(zone, 8.0, 1.0, self.p.accent, 36);
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
        // While debugging, the bar takes the debug colour, as VS Code's.
        let debugging = view.debug.is_some();
        if debugging {
            self.canvas.fill(bar, theme::mix(p.warning, p.ground, 0.14));
        }
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
            let colour = if item.error {
                p.error
            } else if item.accent {
                p.accent
            } else if debugging {
                p.warning
            } else {
                p.muted
            };
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

    /// The debug toolbar, floating at the top of the code: Continue or
    /// Pause, the steps, Restart and Stop; only Stop while building.
    fn debug_bar(&mut self, bar: &merlin_core::DebugBarView, editor: Rect) {
        use merlin_core::DebugButton as B;
        let p = self.p;
        let buttons: Vec<(B, &str, &str, u32)> = if bar.building {
            vec![(B::Stop, "■", "Stop (Shift+F5)", p.error)]
        } else {
            let first = if bar.paused {
                (B::Continue, "▶", "Continue (F5)", p.success)
            } else {
                (B::Pause, "❚❚", "Pause (F6)", p.warning)
            };
            vec![
                first,
                (B::StepOver, "↷", "Step Over (F10)", p.accent),
                (B::StepInto, "↓", "Step Into (F11)", p.accent),
                (B::StepOut, "↑", "Step Out (Shift+F11)", p.accent),
                (B::Restart, "↻", "Restart (Ctrl+Shift+F5)", p.success),
                (B::Stop, "■", "Stop (Shift+F5)", p.error),
            ]
        };
        let (bw, bh, pad) = (30.0, 26.0, 4.0);
        let w = buttons.len() as f32 * (bw + 2.0) - 2.0 + pad * 2.0;
        let r = Rect::new(
            (editor.x + (editor.w - w) / 2.0).round(),
            editor.y + 6.0,
            w,
            bh + pad * 2.0,
        );
        self.canvas.shadow(r, 9.0, theme::is_light(&p));
        self.canvas.outline(r, 9.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let face = Face::ui(14.0).bold();
        let mut x = r.x + pad;
        for (button, glyph, tip, colour) in buttons {
            let b = Rect::new(x, r.y + pad, bw, bh);
            // Steps wait while it runs.
            let waiting = !bar.paused
                && !bar.building
                && matches!(button, B::StepOver | B::StepInto | B::StepOut);
            if !waiting {
                if self.hovered(b) {
                    self.canvas.round(b, 6.0, p.hover);
                }
                self.hit(b, Hit::Ui(UiEvent::Debug(button)));
            }
            self.tips.push((b, tip.to_string()));
            let colour = if waiting { theme::mix(colour, p.widget, 0.65) } else { colour };
            // A step is drawn as its arrow just over a dot, the line it
            // lands on, as VS Code draws them.
            if matches!(button, B::StepOver | B::StepInto | B::StepOut) {
                self.step_icon(button, b.x + bw / 2.0, b.y + bh / 2.0, colour);
            } else {
                let gw = self.text.width(glyph, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    glyph,
                    face,
                    b.x + (bw - gw) / 2.0,
                    b.y,
                    bh,
                    colour,
                    b,
                );
            }
            x += bw + 2.0;
        }
    }

    /// A step button's icon, in a 16 px square centred at (`cx`, `cy`):
    /// an arc over the line for Step Over, an arrow down into it or up out
    /// of it, and the dot under each.
    fn step_icon(&mut self, button: merlin_core::DebugButton, cx: f32, cy: f32, colour: u32) {
        use merlin_core::DebugButton as B;
        let w = 1.7;
        let c = &mut self.canvas;
        match button {
            B::StepOver => {
                // An arc over the top, from left to right, then its head.
                let (ox, oy, r) = (cx, cy - 0.5, 5.5);
                let steps = 10;
                let at = |i: usize| {
                    let t = std::f32::consts::PI * (1.0 + i as f32 / steps as f32);
                    (ox + r * t.cos(), oy + r * t.sin())
                };
                for i in 0..steps {
                    c.line(at(i), at(i + 1), w, colour);
                }
                let end = (ox + r, oy);
                c.line(end, (end.0 - 3.6, end.1 - 1.2), w, colour);
                c.line(end, (end.0 + 0.9, end.1 - 3.6), w, colour);
            }
            B::StepInto => {
                let (top, tip) = (cy - 7.5, cy + 1.0);
                c.line((cx, top), (cx, tip), w, colour);
                c.line((cx - 3.5, tip - 3.5), (cx, tip), w, colour);
                c.line((cx + 3.5, tip - 3.5), (cx, tip), w, colour);
            }
            _ => {
                let (bottom, tip) = (cy + 1.0, cy - 7.5);
                c.line((cx, bottom), (cx, tip), w, colour);
                c.line((cx - 3.5, tip + 3.5), (cx, tip), w, colour);
                c.line((cx + 3.5, tip + 3.5), (cx, tip), w, colour);
            }
        }
        let r = 2.1;
        let dy = cy + 5.2;
        c.round(Rect::new(cx - r, dy - r, r * 2.0, r * 2.0), r, colour);
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
        let colour = if toast.error {
            p.error
        } else if toast.done {
            p.success
        } else if toast.warning {
            p.warning
        } else {
            p.accent
        };
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
        // The items scrolled to keep the selection in view, and the group
        // headings over them, as Run Task has.
        let first = pv.selected.saturating_sub(PALETTE_ROWS - 1);
        let heading = |k: usize| pv.extras.get(k).and_then(|e| e.heading.as_deref());
        let headings = (first..pv.items.len().min(first + PALETTE_ROWS))
            .filter(|&k| heading(k).is_some())
            .count();
        let list_h = shown as f32 * 34.0 + headings as f32 * 26.0 + 12.0;
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
            let title = match title.ends_with('›') {
                true => title.clone(),
                false => format!("{title}:"),
            };
            qx += self.text.draw_in_row(
                &mut self.canvas,
                &title,
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
        let mut y = list.y + 6.0;
        for (k, (label, key)) in pv.items.iter().enumerate().skip(first).take(PALETTE_ROWS) {
            if let Some(heading) = heading(k) {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &heading.to_uppercase(),
                    Face::ui(11.0).medium(),
                    r.x + 18.0,
                    y,
                    26.0,
                    p.muted,
                    list,
                );
                y += 26.0;
            }
            let row = Rect::new(r.x + 6.0, y, r.w - 12.0, 34.0);
            y += 34.0;
            let on = k == pv.selected;
            if on {
                self.canvas.round(row, 7.0, p.selected);
            } else if self.hovered(row) {
                self.canvas.round(row, 7.0, p.hover);
            }
            let extra = pv.extras.get(k);
            // At the right: the key, then the plugin's chip, then where
            // it's from.
            let mut right = row.right() - 10.0;
            if let Some(key) = key {
                right -= self.chip(key, right, row.y + 9.0, true) + 8.0;
            }
            if let Some(chip) = extra.and_then(|e| e.chip.as_deref()) {
                right -= self.plugin_chip(chip, right, row.y + 9.0) + 8.0;
            }
            if let Some(detail) = extra.map(|e| e.detail.as_str()).filter(|d| !d.is_empty()) {
                let face = Face::ui(12.0);
                let w = self.text.width(detail, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    detail,
                    face,
                    right - w,
                    row.y,
                    row.h,
                    p.muted,
                    row,
                );
                right -= w + 12.0;
            }
            let max = (right - row.x - 12.0).min(row.w - 140.0);
            if extra.is_some_and(|e| e.muted) {
                self.text.draw_in_row(
                    &mut self.canvas,
                    label,
                    Face::ui(13.5),
                    row.x + 12.0,
                    row.y,
                    row.h,
                    p.muted,
                    row,
                );
            } else {
                self.matched(label, &pv.query, row.x + 12.0, row, max);
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

/// Where a tree row's checkbox, arrow and buttons are, to click.
#[derive(Default)]
struct EntryHits {
    check: Option<Rect>,
    fold: Option<Rect>,
    /// The buttons shown, in order; ones hidden till hover aren't here.
    links: Vec<Rect>,
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

/// What a hover card shows.
struct HoverCard<'a> {
    problems: &'a [ProblemView],
    lines: &'a [String],
    colours: &'a [Spans],
    definition: bool,
    fix: Option<&'a str>,
    more_fixes: bool,
}

/// The faces of a hover's message, and of its prose, each with the face
/// of its code.
const MESSAGE: (Face, Face) = (Face::ui(13.0).medium(), Face::mono(12.5));
const PROSE: (Face, Face) = (Face::ui(12.0), Face::mono(11.5));
const CODE: Face = Face::mono(12.5);

/// Coloured runs of a line, by character.
type Spans = Vec<(usize, usize, merlin_core::Syntax)>;

/// What a language server's hover says, to draw: its code, its prose
/// joined into paragraphs, and room between.
#[derive(Clone, Debug, PartialEq)]
enum Part {
    /// A line of code, and its colours by character.
    Code(String, Spans),
    Prose(String),
    Gap,
}

/// The code in a hover, then its docs up to their first heading, at most
/// two paragraphs: the signature and what it does, as the card has room
/// for. Rules and runs of blank lines are room between.
fn hover_parts(lines: &[String], colours: &[Spans]) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let mut code = false;
    let mut paragraphs = 0;
    let mut code_lines = 0;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            code = !code;
            if !code {
                parts.push(Part::Gap);
            }
            continue;
        }
        if code {
            code_lines += 1;
            if code_lines <= 8 {
                let spans = colours.get(i).cloned().unwrap_or_default();
                parts.push(Part::Code(line.trim_end().to_string(), spans));
            }
            continue;
        }
        if trimmed.is_empty() || trimmed == "---" {
            if !matches!(parts.last(), None | Some(Part::Gap)) {
                parts.push(Part::Gap);
            }
            continue;
        }
        if trimmed.starts_with('#') && !parts.is_empty() {
            break;
        }
        match parts.last_mut() {
            Some(Part::Prose(text)) => {
                text.push(' ');
                text.push_str(trimmed);
            }
            _ => {
                if paragraphs == 2 {
                    break;
                }
                paragraphs += 1;
                parts.push(Part::Prose(trimmed.to_string()));
            }
        }
    }
    while matches!(parts.last(), Some(Part::Gap)) {
        parts.pop();
    }
    parts
}

/// A line's runs of text and `backticked` code.
fn code_spans(line: &str) -> Vec<(&str, bool)> {
    line.split('`')
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, s)| (s, i % 2 == 1))
        .collect()
}

/// One thing the hover card draws.
enum Op {
    Badge(Severity),
    Message(Vec<(String, bool)>),
    Note {
        line: Vec<(String, bool)>,
        help: bool,
    },
    Source(usize),
    Buttons,
    Divider,
    Code(String, Spans),
    Prose(Vec<(String, bool)>),
    Definition,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hover_keeps_its_code_and_the_first_of_its_docs() {
        // As rust-analyzer sends it.
        let hover: Vec<String> = "```rust\ncore::slice\n```\n\n```rust\nimpl<T> [T]\npub const fn len(&self) -> usize\n```\n\n---\n\nReturns the number of elements\nin the slice.\n\n# Examples\n\n```\nlet a = [1, 2, 3];\n```"
            .lines()
            .map(String::from)
            .collect();
        assert_eq!(
            hover_parts(&hover, &[]),
            [
                Part::Code("core::slice".into(), Vec::new()),
                Part::Gap,
                Part::Code("impl<T> [T]".into(), Vec::new()),
                Part::Code("pub const fn len(&self) -> usize".into(), Vec::new()),
                Part::Gap,
                Part::Prose("Returns the number of elements in the slice.".into()),
            ]
        );
    }

    #[test]
    fn backticks_mark_code() {
        assert_eq!(
            code_spans("expected `u32`, found `usize`"),
            [
                ("expected ", false),
                ("u32", true),
                (", found ", false),
                ("usize", true)
            ]
        );
    }
}
