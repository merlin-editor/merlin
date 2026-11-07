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

mod appearance;
mod keys;
mod plugins;
mod rows;

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

    // Appearance ----------------------------------------------------------

    // Keyboard Shortcuts ----------------------------------------------------

    // Plugins ---------------------------------------------------------------

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
