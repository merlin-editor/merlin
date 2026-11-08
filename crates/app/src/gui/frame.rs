//! The window's frame, laid out and drawn in pixels as the design has it:
//! a slim title bar, the activity rail, the side bar, editor and panel as
//! rounded islands on a darker ground, and a thin status bar. Drawing also
//! records what each clickable part does.

use std::collections::HashMap;

use merlin_core::editor::{
    BlameCardView, ChangeView, DiffBarView, MergeBarView, MergeRow, MergeSide, MergeView, PeekItem,
    PeekView, RenameView,
};
use merlin_core::lang::{ActionKind, CodeAction};
use merlin_core::panel::{Check, Entry, Node, Swap, Tone};
use merlin_core::{
    ContextItem, ContextView, DialogView, FindAction, FindView, HoverButton, Key, KeyCode,
    MenuView, PaletteIcon, PaletteView, PopupKind, PopupView, ProblemView, Severity, Side,
    SideView, SignatureView, TabView, TerminalView, TextView, ToastView, UiEvent, WindowView,
    editor::GroupView,
};

mod cards;
mod chat;
mod chrome;
mod diff;
mod find;
mod group;
mod hover;
mod markdown;
mod palette;
mod peek;
mod side;
mod widgets;

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

/// What the pointer shows over a hit, when it isn't the arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Drawn in the link colour: the hand.
    Link,
    /// A box to type in: the text bar.
    Input,
    /// A tab that can be dragged: the open hand, after a rest.
    Tab,
}

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
    pub scroll: [f32; 4],
    /// The selection each side last showed, to scroll to it when it moves.
    selected: [Option<usize>; 4],
    /// How far each multi-line input has scrolled.
    pub input_scroll: HashMap<String, f32>,
}

pub(super) fn side_index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
        Side::Bottom => 2,
        Side::Tab => 3,
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
    /// Where a side's tabs must end, for what's drawn over their right.
    pub tabs_end: Option<f32>,
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
    /// The hits, by their place in `hits`, the pointer shows differently.
    pub marks: Vec<(usize, Mark)>,
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
            Side::Tab => {}
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

    /// A hit, marked for the pointer's shape over it.
    pub(super) fn marked_hit(&mut self, r: Rect, hit: Hit, mark: Mark) {
        self.marks.push((self.hits.len(), mark));
        self.hits.push((r, hit));
    }

    /// A box to type in, for the text bar over it. Clicks go to what's
    /// under it, as before.
    pub(super) fn input_box(&mut self, r: Rect) {
        let under = self
            .hits
            .iter()
            .rev()
            .find(|(h, _)| h.contains(r.x + r.w / 2.0, r.y + r.h / 2.0))
            .map_or(Hit::Block, |(_, hit)| hit.clone());
        self.marked_hit(r, under, Mark::Input);
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
                _ => &view.bottom,
            };
            if let (Some(area), Some(sv)) = (area, sv) {
                self.canvas.round(area, self.radius, p.island);
                let term = view.terminal.as_ref().filter(|_| side == Side::Bottom);
                // The Output panel's choices sit over the tabs' right end.
                self.tabs_end = view
                    .output_bar
                    .as_ref()
                    .filter(|_| side == Side::Bottom)
                    .map(|bar| area.right() - self.output_bar_width(bar) - 8.0);
                self.side(sv, side, area, state, term);
                self.tabs_end = None;
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
        // The Output panel's choices, over its tabs, and the list one
        // opened.
        if let (Some(bar), Some(panel)) = (&view.output_bar, a.bottom) {
            let under = self.output_bar(bar, panel);
            self.output_list(bar, under, w, h);
        }
        if let Some(toast) = &view.toast {
            self.toast(toast, a.editor);
        }
        if let Some(card) = &view.language_card {
            self.language_card(card, w, h);
        }
        if let Some(palette) = &view.palette {
            self.palette(palette, w, h);
        }
        if let Some(dialog) = &view.dialog {
            self.dialog(dialog, w, h);
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
        if let Some(line) = &view.vim_line {
            self.vim_line(line, a);
        }
        if let Some(card) = &view.which_key {
            self.which_key(card, a);
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
        // An open menu is what's being read: no tip over it.
        if let Some((r, tip)) = self.tip.clone()
            && view.context.is_none()
            && view.menu.is_none()
        {
            self.tooltip(r, &tip, w);
        }
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
