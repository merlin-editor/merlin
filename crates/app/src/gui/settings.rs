//! The Settings tab, drawn from the app's [`SettingsView`]: its pages down
//! the left, search and the User and Workspace tabs on top, and under them
//! the page's rows, cards or lists, scrolled. Drawing records what each
//! part does as a [`SettingsAction`].

use super::canvas::{Face, Rect};
use super::frame::{Frame, Hit};
use crate::prefs::{
    Category, Control, KeyRow, KeysFilter, PageAction, PageView, Row, SettingsAction, SettingsView,
    ThemeCard,
};
use crate::theme::mix;

const NAV_W: f32 = 210.0;
const SEARCH_H: f32 = 32.0;
const MAX_W: f32 = 900.0;
const ROW_PAD: f32 = 12.0;
const KEY_ROW_H: f32 = 34.0;
const CARD_W: f32 = 170.0;
const CARD_GAP: f32 = 10.0;
/// Rows of keys a search shows, before it says how many more.
const SEARCH_KEYS: usize = 8;

/// Where the page is being laid out: its column, how far down, and what
/// shows of it.
struct Ctx {
    x: f32,
    w: f32,
    y: f32,
    clip: Rect,
    /// A choice's list to draw over the rest: under which control, and
    /// the setting.
    list: Option<(Rect, ListOf)>,
    /// Where the keys row picked was drawn.
    picked: Option<f32>,
}

enum ListOf {
    Row(Row),
    Packs(Vec<(String, String)>),
}

impl Ctx {
    fn shows(&self, h: f32) -> bool {
        self.y + h >= self.clip.y && self.y <= self.clip.bottom()
    }
}

/// A choice's value as people read it: `after_delay` is "After delay".
fn option_label(o: &str) -> String {
    let s = o.replace(['_', '-'], " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => s,
    }
}

/// A file size in a few figures.
fn bytes(n: u64) -> String {
    let mb = n as f64 / (1024.0 * 1024.0);
    if mb >= 100.0 {
        format!("{mb:.0} MB")
    } else if mb >= 0.05 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", n as f64 / 1024.0)
    }
}

impl<'a> Frame<'a> {
    // Drawing kept to what shows.

    fn s_fill(&mut self, r: Rect, clip: Rect, colour: u32) {
        let r = r.intersect(clip);
        if r.w > 0.0 && r.h > 0.0 {
            self.canvas.fill(r, colour);
        }
    }

    fn s_round(&mut self, r: Rect, radius: f32, clip: Rect, colour: u32) {
        if r.intersect(clip) == r {
            self.canvas.round(r, radius, colour);
        } else {
            self.s_fill(r, clip, colour);
        }
    }

    fn s_outline(&mut self, r: Rect, radius: f32, clip: Rect, colour: u32, inside: u32) {
        if r.intersect(clip) == r {
            self.canvas.outline(r, radius, 1.0, colour, inside);
        } else {
            self.s_fill(r, clip, inside);
        }
    }

    fn s_hit(&mut self, r: Rect, clip: Rect, hit: Hit) {
        let r = r.intersect(clip);
        if r.w > 0.0 && r.h > 0.0 {
            self.hit(r, hit);
        }
    }

    fn s_hovered(&self, r: Rect, clip: Rect) -> bool {
        self.hovered(r.intersect(clip))
    }

    fn s_text(&mut self, text: &str, face: Face, x: f32, y: f32, colour: u32, clip: Rect) -> f32 {
        self.text
            .draw(&mut self.canvas, text, face, x, y, colour, clip, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn s_row_text(
        &mut self,
        text: &str,
        face: Face,
        x: f32,
        y: f32,
        h: f32,
        colour: u32,
        clip: Rect,
    ) -> f32 {
        self.text
            .draw_in_row(&mut self.canvas, text, face, x, y, h, colour, clip)
    }

    /// The Settings tab in `r`, which has the keyboard when `focused`.
    /// Returns how far it scrolls.
    pub fn settings_page(&mut self, v: &'a SettingsView, r: Rect, focused: bool) -> f32 {
        let p = self.p;
        self.canvas.fill(r, p.island);
        self.hit(r, Hit::SettingsBody);
        let nav_w = NAV_W.min(r.w * 0.3);
        self.settings_nav(v, Rect::new(r.x, r.y, nav_w, r.h));
        let x = r.x + nav_w + 24.0;
        let w = (r.right() - 32.0 - x).clamp(160.0, MAX_W);
        let top = r.y + 18.0;
        let body = Rect::new(r.x + nav_w, top + SEARCH_H + 44.0, r.w - nav_w, 0.0);
        let body = Rect::new(body.x, body.y, body.w, (r.bottom() - body.y).max(0.0));
        let mut c = Ctx {
            x,
            w,
            y: body.y + 4.0 - v.ui.scroll,
            clip: body,
            list: None,
            picked: None,
        };
        let start = c.y;
        if let Some(err) = &v.error {
            self.settings_banner(err, &mut c);
        }
        let searching = !v.ui.search.is_empty() && !v.ui.record;
        if searching {
            self.settings_found(v, &mut c);
        } else {
            match v.ui.category {
                Category::Appearance => self.appearance(v, &mut c),
                Category::Editor | Category::Files => {
                    for s in &v.sections {
                        self.s_heading(s.title, None, &mut c);
                        for row in &s.rows {
                            self.setting_row(row, v.ui.open_choice == Some(row.key), &mut c);
                        }
                    }
                }
                Category::Keys => {
                    for s in &v.sections {
                        self.s_heading(s.title, None, &mut c);
                        for row in &s.rows {
                            self.setting_row(row, v.ui.open_choice == Some(row.key), &mut c);
                        }
                    }
                    self.keys_page(v, &mut c)
                }
                Category::Plugins => self.plugins_page(v, &mut c),
            }
        }
        let height = c.y - start + 24.0;
        // The top over what scrolls under it.
        self.canvas.fill(
            Rect::new(r.x + nav_w, r.y, r.w - nav_w, body.y - r.y),
            p.island,
        );
        self.settings_search(v, Rect::new(x, top, w, SEARCH_H), focused);
        self.settings_scope(v, Rect::new(x, top + SEARCH_H + 12.0, w, 30.0));
        // What floats: a choice's list, then the card catching keys.
        if let Some((under, of)) = c.list.take() {
            self.choice_list(under, &of, r);
        }
        if v.capture.is_some() {
            self.capture_card(v, &c, body);
        }
        (height - body.h).max(0.0)
    }

    fn settings_nav(&mut self, v: &SettingsView, r: Rect) {
        let p = self.p;
        let face = Face::ui(13.0);
        let mut y = r.y + 14.0;
        for (i, cat) in Category::ALL.into_iter().enumerate() {
            let row = Rect::new(r.x + 10.0, y, r.w - 20.0, 30.0);
            let on = cat == v.ui.category && (v.ui.search.is_empty() || v.ui.record);
            let bg = if on {
                Some(p.chip)
            } else if self.hovered(row) {
                Some(p.hover)
            } else {
                None
            };
            if let Some(bg) = bg {
                self.canvas.round(row, 6.0, bg);
            }
            let fg = if on { p.bright } else { p.chip_text };
            self.nav_icon(cat, row.x + 10.0, row.y + 7.5, fg);
            self.text.draw_in_row(
                &mut self.canvas,
                cat.title(),
                face,
                row.x + 35.0,
                y,
                30.0,
                fg,
                row,
            );
            let n = v.changed[i];
            if n > 0 {
                let label = n.to_string();
                let f = Face::ui(11.0);
                let tw = self.text.width(&label, f);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    f,
                    row.right() - 10.0 - tw,
                    y,
                    30.0,
                    p.muted,
                    row,
                );
            }
            self.hit(row, Hit::Settings(SettingsAction::Category(cat)));
            y += 32.0;
        }
        let open = Rect::new(r.x + 10.0, r.bottom() - 44.0, r.w - 20.0, 30.0);
        let fg = if self.hovered(open) { p.text } else { p.muted };
        let label = if v.ui.workspace {
            "Open the folder's settings.toml"
        } else {
            "Open settings.toml"
        };
        let label = self.text.fit(label, Face::ui(12.0), open.w - 20.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            Face::ui(12.0),
            open.x + 10.0,
            open.y,
            30.0,
            fg,
            open,
        );
        self.hit(open, Hit::Settings(SettingsAction::OpenFile));
    }

    /// A page's small picture, 15 points square from (`x`, `y`).
    fn nav_icon(&mut self, cat: Category, x: f32, y: f32, c: u32) {
        let p = self.p;
        let line = |f: &mut Self, a: (f32, f32), b: (f32, f32)| {
            f.canvas
                .line((x + a.0, y + a.1), (x + b.0, y + b.1), 1.4, c)
        };
        match cat {
            Category::Appearance => {
                self.canvas.outline(
                    Rect::new(x + 1.5, y + 1.5, 12.0, 12.0),
                    6.0,
                    1.4,
                    c,
                    p.island,
                );
                self.canvas
                    .round(Rect::new(x + 4.5, y + 4.5, 6.0, 6.0), 3.0, c);
            }
            Category::Editor => {
                line(self, (2.0, 4.0), (13.0, 4.0));
                line(self, (2.0, 7.5), (10.0, 7.5));
                line(self, (2.0, 11.0), (12.0, 11.0));
            }
            Category::Files => {
                self.canvas.outline(
                    Rect::new(x + 3.0, y + 1.5, 9.5, 12.0),
                    1.5,
                    1.4,
                    c,
                    p.island,
                );
            }
            Category::Keys => {
                self.canvas.outline(
                    Rect::new(x + 1.0, y + 3.5, 13.0, 8.5),
                    1.5,
                    1.4,
                    c,
                    p.island,
                );
                line(self, (4.0, 9.0), (11.0, 9.0));
            }
            Category::Plugins => {
                for (dx, dy) in [(1.5, 1.5), (8.5, 1.5), (1.5, 8.5), (8.5, 8.5)] {
                    self.canvas
                        .outline(Rect::new(x + dx, y + dy, 5.0, 5.0), 1.0, 1.3, c, p.island);
                }
            }
        }
    }

    fn settings_search(&mut self, v: &SettingsView, r: Rect, focused: bool) {
        let p = self.p;
        let keys = v.ui.category == Category::Keys && v.ui.search.is_empty();
        let border = if v.ui.record { p.accent } else { p.border };
        self.canvas.outline(r, 7.0, 1.0, border, p.raised);
        self.input_box(r);
        // The magnifier.
        let (cx, cy) = (r.x + 17.0, r.y + r.h / 2.0 - 1.0);
        self.canvas.outline(
            Rect::new(cx - 5.0, cy - 5.0, 10.0, 10.0),
            5.0,
            1.5,
            p.muted,
            p.raised,
        );
        self.canvas
            .line((cx + 3.5, cy + 3.5), (cx + 7.0, cy + 7.0), 1.5, p.muted);
        let face = Face::ui(13.0);
        let tx = r.x + 34.0;
        let mut right = r.right() - 8.0;
        // Record Keys, on the Keys page.
        if v.ui.category == Category::Keys {
            let label = if v.ui.record {
                "Recording"
            } else {
                "Record Keys"
            };
            let f = Face::ui(12.0);
            let bw = self.text.width(label, f) + 20.0;
            let b = Rect::new(right - bw, r.y + 4.0, bw, r.h - 8.0);
            let bg = if v.ui.record {
                mix(p.accent, p.raised, 0.25)
            } else if self.hovered(b) {
                p.border
            } else {
                p.chip
            };
            self.canvas.round(b, 5.0, bg);
            let fg = if v.ui.record { p.bright } else { p.chip_text };
            self.text
                .draw_in_row(&mut self.canvas, label, f, b.x + 10.0, b.y, b.h, fg, b);
            self.hit(b, Hit::Settings(SettingsAction::Record));
            right = b.x - 8.0;
        }
        if v.ui.search.is_empty() {
            let hint = if v.ui.record {
                "Press keys to see what they run"
            } else if keys {
                "Search commands or keys"
            } else {
                "Search settings"
            };
            self.text
                .draw_in_row(&mut self.canvas, hint, face, tx, r.y, r.h, p.muted, r);
            if focused {
                self.canvas
                    .fill(Rect::new(tx, r.y + 9.0, 1.5, r.h - 18.0), p.accent);
            }
            return;
        }
        let clip = Rect::new(tx, r.y, (right - tx).max(0.0), r.h);
        let shown = if v.ui.record {
            v.ui.search
                .split('+')
                .map(option_label)
                .collect::<Vec<_>>()
                .join("+")
        } else {
            v.ui.search.clone()
        };
        let tw =
            self.text
                .draw_in_row(&mut self.canvas, &shown, face, tx, r.y, r.h, p.bright, clip);
        if focused && !v.ui.record {
            self.canvas.fill(
                Rect::new(tx + tw + 1.0, r.y + 9.0, 1.5, r.h - 18.0),
                p.accent,
            );
        }
        // How much it found, and ✕ to clear it.
        let x = r.right() - 30.0;
        let b = Rect::new(x, r.y + 4.0, 24.0, r.h - 8.0);
        if v.ui.category != Category::Keys || !v.ui.record {
            let fg = if self.hovered(b) { p.bright } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                "✕",
                Face::ui(12.0),
                x + 7.0,
                r.y,
                r.h,
                fg,
                r,
            );
            self.hit(b, Hit::Settings(SettingsAction::ClearSearch));
            if !v.ui.record {
                let n: usize = v.sections.iter().map(|s| s.rows.len()).sum();
                let found = format!("{n} settings · {} keys", v.keys.len());
                let f = Face::ui(11.0);
                let fw = self.text.width(&found, f);
                if tx + tw + fw + 24.0 < x {
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &found,
                        f,
                        x - 8.0 - fw,
                        r.y,
                        r.h,
                        p.muted,
                        r,
                    );
                }
            }
        }
    }

    fn settings_scope(&mut self, v: &SettingsView, r: Rect) {
        let p = self.p;
        let face = Face::ui(12.5);
        self.canvas
            .fill(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), p.line);
        let mut x = r.x;
        let folder = format!("Workspace · {}", v.folder);
        for (label, workspace) in [("User", false), (folder.as_str(), true)] {
            let tw = self.text.width(label, face);
            let tab = Rect::new(x, r.y, tw, r.h);
            let on = v.ui.workspace == workspace;
            let fg = if on || self.hovered(tab) {
                p.bright
            } else {
                p.muted
            };
            self.text
                .draw_in_row(&mut self.canvas, label, face, x, r.y, r.h - 8.0, fg, r);
            if on {
                self.canvas
                    .fill(Rect::new(x, r.bottom() - 2.0, tw, 2.0), p.accent);
            }
            self.hit(tab, Hit::Settings(SettingsAction::Workspace(workspace)));
            x += tw + 20.0;
        }
    }

    fn settings_banner(&mut self, err: &str, c: &mut Ctx) {
        let p = self.p;
        let r = Rect::new(c.x, c.y + 6.0, c.w, 38.0);
        self.s_round(r, 7.0, c.clip, mix(p.error, p.island, 0.14));
        let face = Face::ui(12.5);
        let open = "Open";
        let bw = self.text.width(open, face) + 20.0;
        let b = Rect::new(r.right() - bw - 8.0, r.y + 6.0, bw, 26.0);
        self.s_round(
            b,
            6.0,
            c.clip,
            if self.s_hovered(b, c.clip) {
                p.border
            } else {
                p.chip
            },
        );
        self.s_row_text(open, face, b.x + 10.0, b.y, b.h, p.bright, c.clip);
        self.s_hit(b, c.clip, Hit::Settings(SettingsAction::OpenFile));
        let tclip = Rect::new(r.x, r.y, b.x - r.x - 8.0, r.h).intersect(c.clip);
        self.s_row_text("⊗", face, r.x + 12.0, r.y, r.h, p.error, tclip);
        let msg = format!("{err}. Changes wait until the file reads again.");
        let msg = self.text.fit(&msg, face, tclip.w - 34.0);
        self.s_row_text(&msg, face, r.x + 32.0, r.y, r.h, p.text, tclip);
        c.y += 50.0;
    }

    /// A heading over a page's part, with a link at its right.
    fn s_heading(&mut self, title: &str, link: Option<(&str, Hit)>, c: &mut Ctx) {
        let p = self.p;
        c.y += 20.0;
        let face = Face::ui(11.0).bold();
        self.s_text(&title.to_uppercase(), face, c.x, c.y, p.muted, c.clip);
        if let Some((label, hit)) = link {
            let f = Face::ui(12.0).medium();
            let tw = self.text.width(label, f);
            let r = Rect::new(c.x + c.w - tw, c.y - 2.0, tw, 18.0);
            let fg = if self.s_hovered(r, c.clip) {
                mix(p.accent, 0xffffff, 0.8)
            } else {
                p.accent
            };
            self.s_text(label, f, r.x, c.y - 1.0, fg, c.clip);
            self.s_hit(r, c.clip, hit);
        }
        c.y += 22.0;
    }

    /// A small tinted label beside a setting's name.
    fn tag(&mut self, label: &str, x: f32, mid: f32, clip: Rect) -> f32 {
        let p = self.p;
        let face = Face::ui(10.5).medium();
        let tw = self.text.width(label, face);
        let r = Rect::new(x, mid - 8.0, tw + 12.0, 16.0);
        self.s_round(r, 4.0, clip, mix(p.accent, p.island, 0.16));
        self.s_row_text(label, face, x + 6.0, r.y, r.h, p.accent, clip);
        r.w
    }

    /// A setting: its name and what it does on the left, its control on
    /// the right.
    fn setting_row(&mut self, row: &Row, open: bool, c: &mut Ctx) {
        let p = self.p;
        let (cw, seg) = self.control_width(&row.control);
        let reset_w = if row.changed { 28.0 } else { 0.0 };
        let text_w = (c.w - cw - reset_w - 24.0).max(80.0);
        let about = Face::ui(12.5);
        let about_h = self.text.wrapped_height(row.about, about, text_w);
        let note_h = row
            .note
            .as_ref()
            .map_or(0.0, |n| self.text.wrapped_height(n, about, text_w) + 2.0);
        let h = ROW_PAD * 2.0 + 20.0 + 3.0 + about_h + note_h;
        if c.shows(h) {
            let clip = c.clip;
            let label = Face::ui(13.5).medium();
            let mut x = c.x;
            x += self.s_row_text(row.label, label, x, c.y + ROW_PAD, 20.0, p.bright, clip) + 8.0;
            let mid = c.y + ROW_PAD + 10.0;
            if row.in_workspace {
                x += self.tag("Workspace", x, mid, clip) + 6.0;
            } else if row.changed {
                x += self.tag("Modified", x, mid, clip) + 6.0;
            }
            let key = Rect::new(x, c.y, c.x + text_w - x, h).intersect(clip);
            self.s_row_text(
                row.key,
                Face::mono(11.0),
                x + 2.0,
                c.y + ROW_PAD,
                20.0,
                p.faint,
                key,
            );
            let ty = c.y + ROW_PAD + 23.0;
            self.text.draw(
                &mut self.canvas,
                row.about,
                about,
                c.x,
                ty,
                p.muted,
                clip,
                Some(text_w),
            );
            if let Some(note) = &row.note {
                self.text.draw(
                    &mut self.canvas,
                    note,
                    about,
                    c.x,
                    ty + about_h + 2.0,
                    mix(p.accent, p.text, 0.45),
                    clip,
                    Some(text_w),
                );
            }
            let right = c.x + c.w;
            let ctl = Rect::new(right - cw, c.y + (h - 28.0) / 2.0, cw, 28.0);
            if row.changed {
                let b = Rect::new(ctl.x - 28.0, ctl.y + 2.0, 24.0, 24.0);
                let fg = if self.s_hovered(b, clip) {
                    mix(p.accent, 0xffffff, 0.8)
                } else {
                    p.accent
                };
                self.s_row_text("↺", Face::ui(14.0), b.x + 5.0, b.y, b.h, fg, clip);
                self.s_hit(b, clip, Hit::Settings(SettingsAction::Reset(row.key)));
            }
            self.control(row, ctl, seg, clip);
            if open {
                c.list = Some((ctl, ListOf::Row(row.clone())));
            }
            self.s_fill(Rect::new(c.x, c.y + h - 1.0, c.w, 1.0), clip, p.line);
        }
        c.y += h;
    }

    /// How wide a control is, and whether a choice is shown as a
    /// segmented row rather than a list.
    fn control_width(&mut self, control: &Control) -> (f32, bool) {
        match control {
            Control::Toggle(_) => (34.0, false),
            Control::Number(_) => (104.0, false),
            Control::Choice(options, _) => {
                let face = Face::ui(12.0);
                let seg: f32 = options
                    .iter()
                    .map(|o| self.text.width(&option_label(o), face) + 20.0)
                    .sum::<f32>()
                    + 6.0;
                if options.len() <= 5 && seg <= 300.0 {
                    (seg, true)
                } else {
                    let widest = options
                        .iter()
                        .map(|o| self.text.width(&option_label(o), Face::ui(12.5)))
                        .fold(0.0, f32::max);
                    ((widest + 44.0).max(150.0), false)
                }
            }
        }
    }

    fn control(&mut self, row: &Row, r: Rect, seg: bool, clip: Rect) {
        let p = self.p;
        match &row.control {
            Control::Toggle(on) => {
                let s = Rect::new(r.x, r.y + 4.0, 34.0, 20.0);
                self.switch(*on, s, clip, Hit::Settings(SettingsAction::Toggle(row.key)));
            }
            Control::Number(n) => {
                self.s_outline(r, 6.0, clip, p.border, p.raised);
                let face = Face::ui(12.5);
                let label = format!("{n:.1}");
                let tw = self.text.width(&label, face);
                self.s_row_text(
                    &label,
                    face,
                    r.x + (r.w - tw) / 2.0,
                    r.y,
                    r.h,
                    p.bright,
                    clip,
                );
                for (glyph, x, by) in [("−", r.x, -0.5), ("+", r.right() - 28.0, 0.5)] {
                    let b = Rect::new(x, r.y, 28.0, r.h);
                    let fg = if self.s_hovered(b, clip) {
                        p.bright
                    } else {
                        p.muted
                    };
                    self.s_row_text(glyph, Face::ui(14.0), x + 9.0, r.y, r.h, fg, clip);
                    self.s_hit(b, clip, Hit::Settings(SettingsAction::Step(row.key, by)));
                }
            }
            Control::Choice(options, now) if seg => {
                self.s_outline(r, 7.0, clip, p.border, p.raised);
                let face = Face::ui(12.0);
                let mut x = r.x + 3.0;
                for o in options {
                    let label = option_label(o);
                    let w = self.text.width(&label, face) + 20.0;
                    let b = Rect::new(x, r.y + 2.0, w, r.h - 4.0);
                    let on = o == now;
                    if on {
                        self.s_round(b, 5.0, clip, mix(p.accent, p.raised, 0.3));
                    } else if self.s_hovered(b, clip) {
                        self.s_round(b, 5.0, clip, p.hover);
                    }
                    let fg = if on { p.bright } else { p.chip_text };
                    self.s_row_text(&label, face, x + 10.0, b.y, b.h, fg, clip);
                    self.s_hit(b, clip, Hit::Settings(SettingsAction::Choose(row.key, o)));
                    x += w;
                }
            }
            Control::Choice(_, now) => {
                let bg = if self.s_hovered(r, clip) {
                    p.hover
                } else {
                    p.raised
                };
                self.s_outline(r, 6.0, clip, p.border, bg);
                let face = Face::ui(12.5);
                self.s_row_text(
                    &option_label(now),
                    face,
                    r.x + 10.0,
                    r.y,
                    r.h,
                    p.bright,
                    clip,
                );
                self.s_row_text("▾", face, r.right() - 20.0, r.y, r.h, p.muted, clip);
                self.s_hit(r, clip, Hit::Settings(SettingsAction::OpenChoice(row.key)));
            }
        }
    }

    fn switch(&mut self, on: bool, s: Rect, clip: Rect, hit: Hit) {
        let p = self.p;
        let hovered = self.s_hovered(s, clip);
        let track = match (on, hovered) {
            (true, false) => p.accent,
            (true, true) => mix(p.accent, 0xffffff, 0.88),
            (false, false) => p.border,
            (false, true) => mix(p.border, p.muted, 0.7),
        };
        self.s_round(s, 10.0, clip, track);
        let knob = if on {
            Rect::new(s.right() - 17.0, s.y + 3.0, 14.0, 14.0)
        } else {
            Rect::new(s.x + 3.0, s.y + 3.0, 14.0, 14.0)
        };
        self.s_round(knob, 7.0, clip, if on { p.on_accent } else { p.muted });
        self.s_hit(s, clip, hit);
    }

    /// A choice's list, open under its control.
    fn choice_list(&mut self, under: Rect, of: &ListOf, page: Rect) {
        let p = self.p;
        let items: Vec<(String, String, Hit, bool)> = match of {
            ListOf::Row(row) => match &row.control {
                Control::Choice(options, now) => options
                    .iter()
                    .map(|o| {
                        (
                            option_label(o),
                            String::new(),
                            Hit::Settings(SettingsAction::Choose(row.key, o)),
                            o == now,
                        )
                    })
                    .collect(),
                _ => return,
            },
            ListOf::Packs(packs) => packs
                .iter()
                .map(|(label, what)| {
                    (
                        label.clone(),
                        what.clone(),
                        Hit::Settings(SettingsAction::PackOn(label.clone())),
                        false,
                    )
                })
                .collect(),
        };
        if items.is_empty() {
            return;
        }
        let face = Face::ui(12.5);
        let w = items
            .iter()
            .map(|(l, d, ..)| self.text.width(l, face) + self.text.width(d, Face::ui(11.5)) + 48.0)
            .fold(under.w, f32::max);
        let h = items.len() as f32 * 28.0 + 8.0;
        let x = (under.right() - w).max(page.x + 8.0);
        let mut y = under.bottom() + 4.0;
        if y + h > page.bottom() - 8.0 {
            y = (under.y - 4.0 - h).max(page.y + 8.0);
        }
        let r = Rect::new(x, y, w, h);
        self.canvas.shadow(r, 8.0, false);
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let mut iy = r.y + 4.0;
        for (label, what, hit, on) in items {
            let row = Rect::new(r.x + 4.0, iy, r.w - 8.0, 28.0);
            if self.hovered(row) {
                self.canvas.round(row, 5.0, p.focus_row);
            }
            let fg = if on { p.accent } else { p.text };
            let lw = self.text.draw_in_row(
                &mut self.canvas,
                &label,
                face,
                row.x + 10.0,
                iy,
                28.0,
                fg,
                row,
            );
            if !what.is_empty() {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &what,
                    Face::ui(11.5),
                    row.x + 20.0 + lw,
                    iy,
                    28.0,
                    p.muted,
                    row,
                );
            }
            self.hit(row, hit);
            iy += 28.0;
        }
    }

    // Appearance ----------------------------------------------------------

    fn appearance(&mut self, v: &'a SettingsView, c: &mut Ctx) {
        let p = self.p;
        let more = Hit::Settings(SettingsAction::Category(Category::Plugins));
        self.s_heading("Colour theme", Some(("More themes in Plugins ›", more)), c);
        let follow = Row {
            label: "Follow the system",
            about: "One theme for a dark desktop, another for a light one.",
            key: "editor.theme_follows_system",
            control: Control::Toggle(v.follow),
            changed: false,
            in_workspace: false,
            note: None,
        };
        // The row's switch flips it, rather than writing it as a toggle.
        let at = self.hits.len();
        self.setting_row(&follow, false, c);
        for (_, hit) in &mut self.hits[at..] {
            if *hit == Hit::Settings(SettingsAction::Toggle("editor.theme_follows_system")) {
                *hit = Hit::Settings(SettingsAction::Follow);
            }
        }
        c.y += 12.0;
        let cols = (((c.w + CARD_GAP) / (CARD_W + CARD_GAP)).floor() as usize).clamp(2, 6);
        let cw = (c.w - CARD_GAP * (cols - 1) as f32) / cols as f32;
        let card_h = 62.0 + 28.0;
        for (i, card) in v.themes.iter().enumerate() {
            let (col, line) = (i % cols, i / cols);
            let r = Rect::new(
                c.x + col as f32 * (cw + CARD_GAP),
                c.y + line as f32 * (card_h + CARD_GAP),
                cw,
                card_h,
            );
            self.theme_card(card, r, v.ui.card == Some(i), v.follow, c.clip);
        }
        let lines = v.themes.len().div_ceil(cols);
        c.y += lines as f32 * (card_h + CARD_GAP);

        self.s_heading("Icons", None, c);
        for (i, card) in v.icons.iter().enumerate() {
            let (col, line) = (i % cols, i / cols);
            let r = Rect::new(
                c.x + col as f32 * (cw + CARD_GAP),
                c.y + line as f32 * (52.0 + CARD_GAP),
                cw,
                52.0,
            );
            let clip = c.clip;
            let hovered = self.s_hovered(r, clip);
            let bg = if hovered { p.hover } else { p.widget };
            let edge = if card.current { p.accent } else { p.border };
            self.card_ground(r, clip, bg, edge, card.current);
            let face = Face::ui(12.0);
            self.s_row_text(&card.label, face, r.x + 10.0, r.y + 6.0, 18.0, p.text, clip);
            let old = self.icons;
            self.icons = &card.icons;
            let mut x = r.x + 10.0;
            for name in ["main.rs", "README.md", "app.js", "main.py"] {
                let core = merlin_core::file_icon(name, true);
                x += self.file_icon(name, Some(core), x, r.y + 30.0, 14.0, clip.intersect(r));
            }
            self.icons = old;
            let from = Face::ui(10.5);
            let fw = self.text.width(&card.from, from);
            if x + fw + 8.0 < r.right() {
                self.s_row_text(
                    &card.from,
                    from,
                    r.right() - 10.0 - fw,
                    r.y + 28.0,
                    18.0,
                    p.muted,
                    clip,
                );
            }
            self.s_hit(
                r,
                clip,
                Hit::Settings(SettingsAction::Icons(card.label.clone())),
            );
        }
        c.y += v.icons.len().div_ceil(cols) as f32 * (52.0 + CARD_GAP);

        let add = Hit::Settings(SettingsAction::OpenChoice("packs.add"));
        let link = (!v.available.is_empty()).then_some(("Add a layout ▾", add));
        let link_at = c.y + 20.0;
        self.s_heading("Layouts", link, c);
        if v.ui.open_choice == Some("packs.add") {
            let under = Rect::new(c.x + c.w - 120.0, link_at - 2.0, 120.0, 18.0);
            c.list = Some((under, ListOf::Packs(v.available.clone())));
        }
        if v.layers.is_empty() {
            let clip = c.clip;
            self.s_row_text(
                "None stacked: merlin's own layout.",
                Face::ui(12.5),
                c.x,
                c.y,
                24.0,
                p.muted,
                clip,
            );
            c.y += 28.0;
        }
        let n = v.layers.len();
        for (i, (label, what)) in v.layers.iter().enumerate() {
            let clip = c.clip;
            let row = Rect::new(c.x, c.y, c.w, 36.0);
            let face = Face::ui(12.5);
            let lw = self.s_row_text(label, face.medium(), c.x, c.y, 36.0, p.bright, clip);
            self.s_row_text(
                what,
                Face::ui(12.0),
                c.x + 14.0 + lw,
                c.y,
                36.0,
                p.muted,
                clip,
            );
            let mut bx = row.right() - 24.0;
            for (glyph, action, shown) in [
                ("✕", SettingsAction::PackOff(i), true),
                ("↓", SettingsAction::PackDown(i), i + 1 < n),
                ("↑", SettingsAction::PackUp(i), i > 0),
            ] {
                let b = Rect::new(bx, c.y + 6.0, 24.0, 24.0);
                if shown {
                    let bg = if self.s_hovered(b, clip) {
                        p.border
                    } else {
                        p.raised
                    };
                    self.s_outline(b, 5.0, clip, p.border, bg);
                    self.s_row_text(
                        glyph,
                        Face::ui(11.5),
                        b.x + 7.0,
                        b.y,
                        b.h,
                        p.chip_text,
                        clip,
                    );
                    self.s_hit(b, clip, Hit::Settings(action));
                }
                bx -= 28.0;
            }
            if i + 1 < n {
                self.s_fill(Rect::new(c.x, c.y + 35.0, c.w, 1.0), clip, p.line);
            }
            c.y += 36.0;
        }
    }

    /// A card's ground in `bg`, edged in `edge`: 2 points just outside it
    /// when `strong`, else 1 inside.
    fn card_ground(&mut self, r: Rect, clip: Rect, bg: u32, edge: u32, strong: bool) {
        let (o, radius, w) = if strong {
            (
                Rect::new(r.x - 2.0, r.y - 2.0, r.w + 4.0, r.h + 4.0),
                10.0,
                2.0,
            )
        } else {
            (r, 8.0, 1.0)
        };
        if o.intersect(clip) == o {
            self.canvas.outline(o, radius, w, edge, bg);
        } else {
            self.s_fill(r, clip, bg);
        }
    }

    /// A theme's card: a few lines of code in its own colours, and its
    /// name.
    fn theme_card(&mut self, card: &ThemeCard, r: Rect, keyed: bool, follow: bool, clip: Rect) {
        let p = self.p;
        let t = card.palette;
        let pv = Rect::new(r.x, r.y, r.w, 62.0);
        let label = Rect::new(r.x, r.y + 62.0, r.w, 28.0);
        let hovered = self.s_hovered(r, clip);
        let bg = if hovered { p.hover } else { p.widget };
        let (edge, strong) = if card.mark.is_some() {
            (p.accent, true)
        } else if keyed {
            (mix(p.accent, p.island, 0.55), true)
        } else {
            (p.border, false)
        };
        self.card_ground(r, clip, bg, edge, strong);
        self.s_fill(pv.inset(1.0), clip, t.island);
        self.s_fill(
            Rect::new(pv.x + 1.0, pv.y + 1.0, 7.0, pv.h - 1.0),
            clip,
            t.ground,
        );
        let code = Face::mono(9.5);
        let lines: [&[(&str, u32)]; 3] = [
            &[
                ("fn", t.keyword),
                (" ", t.text),
                ("main", t.function),
                ("() {", t.text),
            ],
            &[
                ("    let", t.keyword),
                (" s = ", t.text),
                ("\"hi\"", t.string),
                (";", t.text),
            ],
            &[
                ("    Args", t.type_),
                ("::", t.operator),
                ("parse", t.function),
                ("()", t.text),
            ],
        ];
        let inner = pv.inset(1.0).intersect(clip);
        for (n, parts) in lines.iter().enumerate() {
            let mut x = pv.x + 18.0;
            let y = pv.y + 8.0 + n as f32 * 15.0;
            for (text, colour) in *parts {
                x += self
                    .text
                    .draw(&mut self.canvas, text, code, x, y, *colour, inner, None);
            }
        }
        let face = Face::ui(12.0);
        let name = self.text.fit(&card.label, face, r.w - 70.0);
        self.s_row_text(&name, face, label.x + 9.0, label.y, label.h, p.text, clip);
        let (tag, colour, f) = match card.mark {
            Some(mark) => (mark.to_string(), p.accent, Face::ui(10.5).bold()),
            None => (
                if card.light { "Light" } else { "Dark" }.to_string(),
                p.muted,
                Face::ui(10.5),
            ),
        };
        // Without Follow the system, Dark and Light only label the cards.
        let tag = if card.mark.is_none() && !follow && !card.light {
            String::new()
        } else {
            tag
        };
        let tw = self.text.width(&tag, f);
        self.s_row_text(
            &tag,
            f,
            label.right() - 9.0 - tw,
            label.y,
            label.h,
            colour,
            clip,
        );
        self.s_hit(
            r,
            clip,
            Hit::Settings(SettingsAction::Theme(card.label.clone())),
        );
    }

    // Keyboard Shortcuts ----------------------------------------------------

    fn keys_page(&mut self, v: &SettingsView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        // The filters, with how many each lists.
        c.y += 10.0;
        let face = Face::ui(12.0);
        let mut x = c.x + 3.0;
        let widths: Vec<(f32, String)> = KeysFilter::ALL
            .iter()
            .zip(v.key_counts)
            .map(|(f, n)| {
                let label = format!("{}  {n}", f.title());
                (self.text.width(&label, face) + 20.0, label)
            })
            .collect();
        let total: f32 = widths.iter().map(|(w, _)| w).sum::<f32>() + 6.0;
        let seg = Rect::new(c.x, c.y, total, 30.0);
        self.s_outline(seg, 7.0, clip, p.border, p.raised);
        for (f, (w, label)) in KeysFilter::ALL.iter().zip(widths) {
            let b = Rect::new(x, c.y + 2.0, w, 26.0);
            let on = *f == v.ui.keys;
            if on {
                self.s_round(b, 5.0, clip, mix(p.accent, p.raised, 0.3));
            } else if self.s_hovered(b, clip) {
                self.s_round(b, 5.0, clip, p.hover);
            }
            let conflicts = *f == KeysFilter::Conflicts && v.key_counts[3] > 0;
            let fg = match (on, conflicts) {
                (_, true) => p.warning,
                (true, _) => p.bright,
                _ => p.chip_text,
            };
            self.s_row_text(&label, face, x + 10.0, b.y, b.h, fg, clip);
            self.s_hit(b, clip, Hit::Settings(SettingsAction::KeysFilter(*f)));
            x += w;
        }
        c.y += 42.0;
        self.keys_header(c);
        if v.keys.is_empty() {
            let msg = match v.ui.keys {
                KeysFilter::Conflicts => "No two commands share keys.",
                KeysFilter::Changed => "You haven't changed any keys.",
                _ => "Nothing matches.",
            };
            self.s_row_text(msg, Face::ui(12.5), c.x + 6.0, c.y, 34.0, p.muted, clip);
            c.y += 34.0;
        }
        for (i, row) in v.keys.iter().enumerate() {
            self.key_row(row, i, v.ui.key_row == Some(i), c);
        }
    }

    fn keys_header(&mut self, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        let face = Face::ui(11.0).bold();
        let (keys_x, from_x) = key_columns(c);
        for (label, x) in [("COMMAND", c.x + 6.0), ("KEYS", keys_x), ("FROM", from_x)] {
            self.s_row_text(label, face, x, c.y, 28.0, p.muted, clip);
        }
        self.s_fill(Rect::new(c.x, c.y + 27.0, c.w, 1.0), clip, p.line);
        c.y += 28.0;
    }

    fn key_row(&mut self, row: &KeyRow, i: usize, picked: bool, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        if picked {
            c.picked = Some(c.y);
        }
        if !c.shows(KEY_ROW_H) {
            c.y += KEY_ROW_H;
            return;
        }
        let b = &row.binding;
        let r = Rect::new(c.x, c.y, c.w, KEY_ROW_H);
        let hovered = self.s_hovered(r, clip);
        if picked {
            self.s_round(r, 6.0, clip, p.focus_row);
        } else if hovered {
            self.s_round(r, 6.0, clip, p.hover);
        } else {
            self.s_fill(Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0), clip, p.line);
        }
        self.s_hit(r, clip, Hit::Settings(SettingsAction::KeyRow(i, 1)));
        let (keys_x, from_x) = key_columns(c);
        let face = Face::ui(12.5);
        let title_clip = Rect::new(r.x, r.y, keys_x - r.x - 12.0, r.h).intersect(clip);
        let mut x = r.x + 6.0;
        x += self.s_row_text(&b.title, face, x, r.y, r.h, p.bright, title_clip) + 8.0;
        let mid = r.y + r.h / 2.0;
        if b.changed {
            x += self.tag("Changed", x, mid, title_clip) + 6.0;
        }
        if row.conflict {
            let f = Face::ui(10.5).medium();
            let tw = self.text.width("Conflict", f);
            let t = Rect::new(x, mid - 8.0, tw + 12.0, 16.0);
            self.s_round(t, 4.0, title_clip, mix(p.warning, p.island, 0.16));
            self.s_row_text("Conflict", f, x + 6.0, t.y, t.h, p.warning, title_clip);
        }
        // Its keys: alternatives with "or", a chord's two with "then".
        let keys_clip = Rect::new(keys_x, r.y, from_x - keys_x - 12.0, r.h).intersect(clip);
        if b.keys.is_empty() {
            self.s_row_text("—", Face::ui(12.0), keys_x, r.y, r.h, p.faint, keys_clip);
        }
        let mut kx = keys_x;
        for (n, (_, shown)) in b.keys.iter().enumerate() {
            if n > 0 {
                kx += self.s_row_text("or", Face::ui(11.0), kx + 2.0, r.y, r.h, p.faint, keys_clip)
                    + 6.0;
            }
            kx = self.key_chips(shown, kx, mid, keys_clip);
        }
        let from_clip = Rect::new(from_x, r.y, r.right() - from_x - 76.0, r.h).intersect(clip);
        match &b.plugin {
            Some(plugin) => {
                let f = Face::ui(11.0);
                let tw = self.text.width(plugin, f);
                let t = Rect::new(from_x, mid - 8.0, tw + 12.0, 16.0);
                self.s_round(t, 4.0, from_clip, mix(p.accent, p.island, 0.16));
                self.s_row_text(plugin, f, from_x + 6.0, t.y, t.h, p.accent, from_clip);
            }
            None => {
                self.s_row_text(
                    "merlin",
                    Face::ui(11.5),
                    from_x,
                    r.y,
                    r.h,
                    p.muted,
                    from_clip,
                );
            }
        }
        // What can be done to it, while the pointer or the arrows are on it.
        if hovered || picked {
            let mut bx = r.right() - 28.0;
            let mut buttons = vec![("+", SettingsAction::Capture(b.command.clone()), "Add keys")];
            if !b.keys.is_empty() {
                buttons.push((
                    "✕",
                    SettingsAction::Unbind(b.command.clone()),
                    "Remove its keys",
                ));
            }
            if b.changed {
                buttons.push((
                    "↺",
                    SettingsAction::ResetKey(b.command.clone()),
                    "Back to merlin's",
                ));
            }
            for (glyph, action, tip) in buttons {
                let br = Rect::new(bx, r.y + 6.0, 22.0, 22.0);
                if self.s_hovered(br, clip) {
                    self.s_round(br, 5.0, clip, p.border);
                }
                let fg = if glyph == "↺" {
                    p.accent
                } else {
                    p.chip_text
                };
                self.s_row_text(glyph, Face::ui(13.0), br.x + 6.0, br.y, br.h, fg, clip);
                self.s_hit(br, clip, Hit::Settings(action));
                self.tips.push((br, tip.into()));
                bx -= 26.0;
            }
        }
        c.y += KEY_ROW_H;
    }

    /// Keys as people read them (`Ctrl+K Ctrl+M`) as small chips, from
    /// `x`. Returns where they end.
    fn key_chips(&mut self, shown: &str, mut x: f32, mid: f32, clip: Rect) -> f32 {
        let p = self.p;
        let face = Face::ui(11.0);
        for (n, stroke) in shown.split(' ').enumerate() {
            if n > 0 {
                x += self.s_row_text(
                    "then",
                    Face::ui(11.0),
                    x + 2.0,
                    mid - 10.0,
                    20.0,
                    p.faint,
                    clip,
                ) + 6.0;
            }
            // `Ctrl++` is Ctrl and +.
            let mut parts: Vec<&str> = stroke.split('+').collect();
            if stroke.ends_with("++") {
                parts.retain(|s| !s.is_empty());
                parts.push("+");
            }
            for part in parts.into_iter().filter(|s| !s.is_empty()) {
                let tw = self.text.width(part, face);
                let k = Rect::new(x, mid - 10.0, tw + 12.0, 20.0);
                self.s_outline(k, 4.0, clip, p.border, p.chip);
                self.s_row_text(part, face, x + 6.0, k.y, k.h, p.text, clip);
                x += k.w + 4.0;
            }
        }
        x
    }

    /// The card catching keys, under the row it's for.
    fn capture_card(&mut self, v: &SettingsView, c: &Ctx, body: Rect) {
        let p = self.p;
        let Some(cap) = &v.capture else {
            return;
        };
        let w = 380.0_f32.min(body.w - 16.0);
        let clash = cap.clash.as_ref().map(|clash| {
            let keys = cap.keys.join(" ");
            format!("⚠ {keys} is {clash}'s now. Adding it here takes it off {clash}.")
        });
        let clash_h = clash.as_ref().map_or(0.0, |m| {
            self.text.wrapped_height(m, Face::ui(11.5), w - 28.0) + 8.0
        });
        let h = 136.0 + clash_h;
        // Under the row picked, when it shows, else in the middle.
        let row_y = c.picked;
        let (keys_x, _) = key_columns(c);
        let y = match row_y {
            Some(y) if y + KEY_ROW_H + h < body.bottom() && y > body.y => y + KEY_ROW_H + 4.0,
            Some(y) if y - h - 4.0 > body.y && y < body.bottom() => y - h - 4.0,
            _ => body.y + (body.h - h) / 2.0,
        };
        let x = (keys_x - 40.0)
            .min(body.right() - w - 8.0)
            .max(body.x + 8.0);
        let r = Rect::new(x, y, w, h);
        self.canvas.shadow(r, 8.0, false);
        self.canvas.outline(r, 8.0, 1.0, p.accent, p.widget);
        self.hit(r, Hit::Block);
        let inner = r.inset(14.0);
        let title = format!("Press the keys for {}", cap.title);
        let title = self.text.fit(&title, Face::ui(12.5).medium(), inner.w);
        self.text.draw_in_row(
            &mut self.canvas,
            &title,
            Face::ui(12.5).medium(),
            inner.x,
            inner.y,
            18.0,
            p.bright,
            inner,
        );
        let field = Rect::new(inner.x, inner.y + 28.0, inner.w, 32.0);
        self.canvas.outline(field, 6.0, 1.0, p.border, p.ground);
        let mid = field.y + field.h / 2.0;
        if cap.keys.is_empty() {
            self.text.draw_in_row(
                &mut self.canvas,
                "A key, or two in a row",
                Face::ui(12.0),
                field.x + 10.0,
                field.y,
                field.h,
                p.muted,
                field,
            );
        } else {
            let shown = cap.keys.join(" ");
            let end = self.key_chips(&shown, field.x + 10.0, mid, field);
            self.canvas
                .fill(Rect::new(end + 2.0, mid - 7.0, 1.5, 14.0), p.accent);
        }
        let mut y = field.bottom() + 8.0;
        if let Some(msg) = &clash {
            self.text.draw(
                &mut self.canvas,
                msg,
                Face::ui(11.5),
                inner.x,
                y,
                p.warning,
                inner,
                Some(inner.w),
            );
            y += clash_h;
        }
        let by = y + 4.0;
        let face = Face::ui(12.0);
        let mut bx = inner.right();
        let ready = !cap.keys.is_empty();
        let mut buttons = vec![("Add", true, SettingsAction::CaptureDone { replace: false })];
        if cap.had {
            buttons.push((
                "Replace",
                false,
                SettingsAction::CaptureDone { replace: true },
            ));
        }
        for (label, primary, action) in buttons {
            let bw = self.text.width(label, face.medium()) + 20.0;
            bx -= bw;
            let b = Rect::new(bx, by, bw, 26.0);
            let hovered = self.hovered(b) && ready;
            let (bg, fg) = match (primary, ready, hovered) {
                (_, false, _) => (p.chip, p.faint),
                (true, _, false) => (p.accent, p.on_accent),
                (true, _, true) => (mix(p.accent, 0xffffff, 0.88), p.on_accent),
                (false, _, false) => (p.chip, p.bright),
                (false, _, true) => (p.border, p.bright),
            };
            self.canvas.round(b, 6.0, bg);
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                face.medium(),
                bx + 10.0,
                by,
                26.0,
                fg,
                b,
            );
            if ready {
                self.hit(b, Hit::Settings(action));
            }
            bx -= 8.0;
        }
        self.text.draw_in_row(
            &mut self.canvas,
            "Enter adds · Esc cancels",
            Face::ui(11.0),
            inner.x,
            by,
            26.0,
            p.muted,
            Rect::new(inner.x, by, (bx - inner.x).max(0.0), 26.0),
        );
    }

    // Plugins ---------------------------------------------------------------

    fn plugins_page(&mut self, v: &SettingsView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        c.y += 12.0;
        // Narrow, the list goes over the page rather than beside it.
        let stacked = c.w < 640.0;
        let list_w = if stacked {
            c.w
        } else {
            240.0_f32.min(c.w * 0.38)
        };
        let top = c.y;
        let picked = v.page.as_ref().map(|pg| pg.name.as_str());
        let mut y = top;
        for row in &v.plugins {
            let r = Rect::new(c.x, y, list_w, 42.0);
            let on = picked == Some(row.name.as_str());
            if on {
                self.s_round(r, 6.0, clip, p.chip);
            } else if self.s_hovered(r, clip) {
                self.s_round(r, 6.0, clip, p.hover);
            }
            self.plugin_mark(&row.title, r.x + 10.0, r.y + 10.0, 22.0, row.on, clip);
            let name_clip = Rect::new(r.x, r.y, r.w - 64.0, r.h).intersect(clip);
            let fg = if row.on { p.bright } else { p.muted };
            self.s_row_text(
                &row.title,
                Face::ui(12.5),
                r.x + 42.0,
                r.y + 4.0,
                18.0,
                fg,
                name_clip,
            );
            self.s_row_text(
                &row.name,
                Face::ui(11.0),
                r.x + 42.0,
                r.y + 21.0,
                16.0,
                p.muted,
                name_clip,
            );
            let (state, colour) = if row.failed {
                ("failed", p.error)
            } else if row.on {
                ("● on", p.success)
            } else {
                ("off", p.muted)
            };
            let f = Face::ui(11.0);
            let mem = if row.data_only || row.memory == 0 {
                "—".to_string()
            } else {
                bytes(row.memory)
            };
            let mw = self.text.width(&mem, f);
            self.s_row_text(
                &mem,
                f,
                r.right() - 10.0 - mw,
                r.y + 4.0,
                18.0,
                p.muted,
                clip,
            );
            let sw = self.text.width(state, f);
            self.s_row_text(
                state,
                f,
                r.right() - 10.0 - sw,
                r.y + 21.0,
                16.0,
                colour,
                clip,
            );
            self.s_hit(
                r,
                clip,
                Hit::Settings(SettingsAction::Plugin(row.name.clone())),
            );
            y += 44.0;
        }
        let list_bottom = y;
        let (dx, dy) = if stacked {
            self.s_fill(Rect::new(c.x, list_bottom + 8.0, c.w, 1.0), clip, p.line);
            (c.x, list_bottom + 24.0)
        } else {
            let dx = c.x + list_w + 24.0;
            self.s_fill(
                Rect::new(dx - 12.0, top, 1.0, (list_bottom - top).max(200.0)),
                clip,
                p.line,
            );
            (dx, top)
        };
        let mut d = Ctx {
            x: dx,
            w: c.x + c.w - dx,
            y: dy,
            clip,
            list: None,
            picked: None,
        };
        if let Some(page) = &v.page {
            self.plugin_detail(page, &mut d);
        }
        c.y = list_bottom.max(d.y);
    }

    /// A plugin's mark: its title's first letters on a tinted square.
    fn plugin_mark(&mut self, title: &str, x: f32, y: f32, size: f32, on: bool, clip: Rect) {
        let p = self.p;
        let letters: String = title
            .split_whitespace()
            .filter_map(|w| w.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase();
        let colour = if on { p.accent } else { p.muted };
        let r = Rect::new(x, y, size, size);
        self.s_round(r, 6.0, clip, mix(colour, p.island, 0.18));
        let face = Face::ui(size * 0.42).bold();
        let lw = self.text.width(&letters, face);
        self.s_row_text(&letters, face, x + (size - lw) / 2.0, y, size, colour, clip);
    }

    fn plugin_detail(&mut self, page: &PageView, c: &mut Ctx) {
        let p = self.p;
        let clip = c.clip;
        self.plugin_mark(&page.title, c.x, c.y, 34.0, page.enabled, clip);
        let right = c.x + c.w;
        // Its switch, then Reload and Output.
        let sw = Rect::new(right - 34.0, c.y + 7.0, 34.0, 20.0);
        self.switch(
            page.enabled,
            sw,
            clip,
            Hit::Settings(SettingsAction::Page(PageAction::Enable(
                page.name.clone(),
                !page.enabled,
            ))),
        );
        let mut bx = sw.x - 12.0;
        let face = Face::ui(12.0);
        for (label, action) in [
            ("Output", PageAction::Output),
            ("Reload", PageAction::Reload(page.name.clone())),
        ] {
            let bw = self.text.width(label, face) + 20.0;
            bx -= bw;
            let b = Rect::new(bx, c.y + 4.0, bw, 26.0);
            let bg = if self.s_hovered(b, clip) {
                p.border
            } else {
                p.chip
            };
            self.s_round(b, 6.0, clip, bg);
            self.s_row_text(label, face, bx + 10.0, b.y, b.h, p.bright, clip);
            self.s_hit(b, clip, Hit::Settings(SettingsAction::Page(action)));
            bx -= 8.0;
        }
        let tclip = Rect::new(c.x, c.y, bx - c.x - 8.0, 40.0).intersect(clip);
        self.s_row_text(
            &page.title,
            Face::ui(15.0).bold(),
            c.x + 46.0,
            c.y - 1.0,
            20.0,
            p.bright,
            tclip,
        );
        let origin = if page.installed {
            "installed"
        } else {
            "bundled"
        };
        let state = if page.failed {
            " · it failed to start"
        } else if page.waiting {
            " · waiting for your OK"
        } else if !page.enabled {
            " · off"
        } else {
            ""
        };
        let sub = format!("{} · API {} · {origin}{state}", page.name, page.api);
        self.s_row_text(
            &sub,
            Face::ui(11.5),
            c.x + 46.0,
            c.y + 19.0,
            16.0,
            p.muted,
            tclip,
        );
        c.y += 52.0;

        // What it costs now.
        let mut stats: Vec<(String, String)> = Vec::new();
        if !page.data_only {
            stats.push((bytes(page.memory as u64), "in merlin".into()));
        }
        for (prog, pid, mem) in &page.running {
            let name = prog.rsplit('/').next().unwrap_or(prog);
            stats.push((
                mem.map_or_else(|| "—".into(), bytes),
                format!("{name}, pid {pid}"),
            ));
        }
        if !page.data_only {
            stats.push((
                format!("{:.1} s", page.busy.as_secs_f32()),
                "CPU since it started".into(),
            ));
        }
        stats.push((
            format!("{} · {}", page.commands.len(), page.languages.len()),
            "commands · languages".into(),
        ));
        let mut sx = c.x;
        for (value, what) in &stats {
            let vw = self.s_row_text(value, Face::ui(15.0).bold(), sx, c.y, 20.0, p.bright, clip);
            let ww = self.s_row_text(what, Face::ui(11.0), sx, c.y + 21.0, 16.0, p.muted, clip);
            sx += vw.max(ww) + 28.0;
            if sx > right - 60.0 {
                break;
            }
        }
        c.y += 44.0;

        // What it may do, and what it adds.
        let half = (c.w - 24.0) / 2.0;
        let top = c.y;
        let mut l = Ctx {
            x: c.x,
            w: half,
            y: top,
            clip,
            list: None,
            picked: None,
        };
        self.s_heading("Allowed", None, &mut l);
        let mut perms: Vec<(bool, String)> = vec![
            (page.read, "Read the open files".into()),
            (page.write, "Change files".into()),
        ];
        if page.processes.is_empty() {
            perms.push((false, "Start programs".into()));
        } else {
            perms.push((true, format!("Start {}", page.processes.join(", "))));
        }
        if page.data_only {
            perms = vec![(false, "Nothing: it's only data".into())];
        }
        for (ok, what) in perms {
            let (mark, colour, fg) = if ok {
                ("✓", p.success, p.text)
            } else {
                ("—", p.faint, p.muted)
            };
            let face = Face::ui(12.5);
            let wrap = (l.w - 20.0).max(40.0);
            let h = self.text.wrapped_height(&what, face, wrap);
            self.s_row_text(mark, Face::ui(12.0), l.x, l.y, 24.0, colour, clip);
            self.text.draw(
                &mut self.canvas,
                &what,
                face,
                l.x + 20.0,
                l.y + (24.0 - 17.0) / 2.0,
                fg,
                clip,
                Some(wrap),
            );
            l.y += (h + 7.0).max(24.0);
        }
        let mut a = Ctx {
            x: c.x + half + 24.0,
            w: half,
            y: top,
            clip,
            list: None,
            picked: None,
        };
        self.s_heading("Adds", None, &mut a);
        // Its commands without its name before each, with their keys.
        let prefix = format!("{}: ", page.title);
        for (title, key) in page.commands.iter().take(5) {
            let title = title.strip_prefix(&prefix).unwrap_or(title);
            let face = Face::ui(12.5);
            let key_w = key
                .as_ref()
                .map_or(0.0, |k| self.text.width(k, Face::ui(11.5)) + 12.0);
            let shown = self.text.fit(title, face, (a.w - key_w).max(40.0));
            let tw = self.s_row_text(&shown, face, a.x, a.y, 24.0, p.text, clip);
            if let Some(k) = key {
                self.s_row_text(k, Face::ui(11.5), a.x + tw + 12.0, a.y, 24.0, p.muted, clip);
            }
            a.y += 24.0;
        }
        let mut adds: Vec<String> = Vec::new();
        if page.commands.len() > 5 {
            adds.push(format!("and {} more commands", page.commands.len() - 5));
        }
        if !page.languages.is_empty() {
            adds.push(format!("Languages: {}", page.languages.join(", ")));
        }
        if !page.looks.is_empty() {
            adds.push(format!("Looks: {}", page.looks.join(", ")));
        }
        if adds.is_empty() && page.commands.is_empty() {
            adds.push("Nothing to show yet".into());
        }
        for line in adds {
            let line = self.text.fit(&line, Face::ui(12.5), a.w);
            self.s_row_text(&line, Face::ui(12.5), a.x, a.y, 24.0, p.text, clip);
            a.y += 24.0;
        }
        c.y = l.y.max(a.y);

        // Its own settings, as rows.
        if !page.settings.is_empty() {
            self.s_heading("Its settings", None, c);
            for (key, value, changed) in &page.settings {
                let r = Rect::new(c.x, c.y, c.w, 40.0);
                let mut x = c.x;
                x += self.s_row_text(key, Face::ui(13.0).medium(), x, r.y, r.h, p.bright, clip)
                    + 8.0;
                if *changed {
                    self.tag("Modified", x, r.y + r.h / 2.0, clip);
                }
                let ctl_right = r.right();
                match value {
                    toml::Value::Boolean(on) => {
                        let s = Rect::new(ctl_right - 34.0, r.y + 10.0, 34.0, 20.0);
                        let action = PageAction::Setting(
                            page.name.clone(),
                            key.clone(),
                            toml::Value::Boolean(!on),
                        );
                        self.switch(*on, s, clip, Hit::Settings(SettingsAction::Page(action)));
                    }
                    other => {
                        let text = match other {
                            toml::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        let f = Face::mono(11.5);
                        let text = self.text.fit(&text, f, c.w * 0.45);
                        let tw = self.text.width(&text, f);
                        let b = Rect::new(ctl_right - tw - 20.0, r.y + 6.0, tw + 20.0, 28.0);
                        self.s_outline(b, 6.0, clip, p.border, p.raised);
                        self.s_row_text(&text, f, b.x + 10.0, b.y, b.h, p.text, clip);
                    }
                }
                self.s_fill(Rect::new(c.x, r.bottom() - 1.0, c.w, 1.0), clip, p.line);
                c.y += 40.0;
            }
            self.s_row_text(
                "Other values are set in settings.toml, under [plugins].",
                Face::ui(11.5),
                c.x,
                c.y + 4.0,
                20.0,
                p.muted,
                clip,
            );
            c.y += 28.0;
        }
        if page.installed {
            c.y += 12.0;
            let label = if page.uninstall_armed {
                "Uninstall: press again"
            } else {
                "Uninstall"
            };
            let bw = self.text.width(label, face) + 20.0;
            let b = Rect::new(c.x, c.y, bw, 26.0);
            let bg = if self.s_hovered(b, clip) {
                p.border
            } else {
                p.chip
            };
            self.s_round(b, 6.0, clip, bg);
            self.s_row_text(label, face, b.x + 10.0, b.y, b.h, p.error, clip);
            self.s_hit(
                b,
                clip,
                Hit::Settings(SettingsAction::Page(PageAction::Uninstall(
                    page.name.clone(),
                ))),
            );
            c.y += 34.0;
        }
    }

    // Search ----------------------------------------------------------------

    /// What a search found on every page, under each page's name.
    fn settings_found(&mut self, v: &SettingsView, c: &mut Ctx) {
        let p = self.p;
        if v.sections.is_empty() && v.keys.is_empty() {
            c.y += 20.0;
            let clip = c.clip;
            self.s_row_text(
                "Nothing matches.",
                Face::ui(13.0),
                c.x,
                c.y,
                24.0,
                p.muted,
                clip,
            );
            c.y += 24.0;
            return;
        }
        for s in &v.sections {
            let title = format!("{} › {}", s.category.title(), s.title);
            self.s_heading(&title, None, c);
            for row in &s.rows {
                self.setting_row(row, v.ui.open_choice == Some(row.key), c);
            }
        }
        if !v.keys.is_empty() {
            let more = Hit::Settings(SettingsAction::Category(Category::Keys));
            self.s_heading("Keyboard Shortcuts", Some(("All keys ›", more)), c);
            self.keys_header(c);
            for (i, row) in v.keys.iter().take(SEARCH_KEYS).enumerate() {
                self.key_row(row, i, v.ui.key_row == Some(i), c);
            }
            if v.keys.len() > SEARCH_KEYS {
                let clip = c.clip;
                let more = format!("and {} more", v.keys.len() - SEARCH_KEYS);
                self.s_row_text(&more, Face::ui(12.0), c.x + 6.0, c.y, 30.0, p.muted, clip);
                c.y += 30.0;
            }
        }
    }
}

/// Where the Keys and From columns start.
fn key_columns(c: &Ctx) -> (f32, f32) {
    let from = c.x + c.w - 76.0 - 120.0;
    let keys = (from - 250.0).max(c.x + c.w * 0.38);
    (keys, from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_read_as_words() {
        assert_eq!(option_label("after_delay"), "After delay");
        assert_eq!(option_label("line-thin"), "Line thin");
        assert_eq!(option_label("1.55"), "1.55");
    }

    #[test]
    fn sizes_read_in_a_few_figures() {
        assert_eq!(bytes(2 * 1024 * 1024 + 100 * 1024), "2.1 MB");
        assert_eq!(bytes(412 * 1024 * 1024), "412 MB");
        assert_eq!(bytes(20 * 1024), "20 KB");
    }
}
