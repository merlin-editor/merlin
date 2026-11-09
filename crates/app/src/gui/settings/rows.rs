//! A setting's row: its label and tags, its control, a switch, and the
//! list a choice opens.

use super::*;
use crate::prefs::{Editing, FieldKind, SettingsUiView};

const FIELD_W: f32 = 240.0;
const ADD_FIELD_W: f32 = 120.0;
const LIST_MAX_W: f32 = 360.0;
const CHIP_PAD: f32 = 30.0;
const CHIP_GAP: f32 = 6.0;
const ADD: &str = "+ Add";

impl Frame<'_> {
    /// A small tinted label beside a setting's name.
    pub(super) fn tag(&mut self, label: &str, x: f32, mid: f32, clip: Rect) -> f32 {
        let p = self.p;
        let face = Face::ui(10.5).medium();
        let tw = self.text.width(label, face);
        let r = Rect::new(x, mid - 8.0, tw + 12.0, 16.0);
        self.s_round(r, 4.0, clip, mix(p.accent, p.island, 0.16));
        self.s_row_text(label, face, x + 6.0, r.y, r.h, p.accent, clip);
        r.w
    }

    /// A setting: its name and what it does on the left, its control on
    /// the right. `ui` says whether its list is open or its text typed.
    pub(super) fn setting_row(&mut self, row: &Row, ui: &SettingsUiView, c: &mut Ctx) {
        let p = self.p;
        let open = ui.open_choice.as_deref() == Some(row.key.as_str());
        let editing = ui.editing.as_ref().filter(|e| e.key == row.key);
        let (cw, seg) = self.control_width(&row.control, editing);
        let reset = row.changed && row.yours.is_none() && !row.locked;
        let reset_w = if reset { 28.0 } else { 0.0 };
        let text_w = (c.w - cw - reset_w - 24.0).max(80.0);
        let about = Face::ui(12.5);
        let about_h = if row.about.is_empty() {
            0.0
        } else {
            self.text.wrapped_height(&row.about, about, text_w)
        };
        let note_h = row
            .note
            .as_ref()
            .map_or(0.0, |n| self.text.wrapped_height(n, about, text_w) + 2.0);
        let yours_h = if row.yours.is_some() { 22.0 } else { 0.0 };
        let h = ROW_PAD * 2.0 + 20.0 + 3.0 + about_h + note_h + yours_h;
        if c.shows(h) {
            let clip = c.clip;
            let label = Face::ui(13.5).medium();
            let mut x = c.x;
            let fg = if row.locked { p.muted } else { p.bright };
            x += self.s_row_text(&row.label, label, x, c.y + ROW_PAD, 20.0, fg, clip) + 8.0;
            let key = Rect::new(x, c.y, c.x + text_w - x, h).intersect(clip);
            x += self.s_row_text(
                row.key_shown(),
                Face::mono(11.0),
                x + 2.0,
                c.y + ROW_PAD,
                20.0,
                p.faint,
                key,
            ) + 10.0;
            let mid = c.y + ROW_PAD + 10.0;
            if row.user_only {
                self.quiet_tag("User only", x, mid, clip);
            } else if row.in_workspace {
                self.tag("Workspace", x, mid, clip);
            } else if row.changed {
                self.tag("Modified", x, mid, clip);
            }
            let mut ty = c.y + ROW_PAD + 23.0;
            if !row.about.is_empty() {
                self.text.draw(
                    &mut self.canvas,
                    &row.about,
                    about,
                    c.x,
                    ty,
                    p.muted,
                    clip,
                    Some(text_w),
                );
                ty += about_h;
            }
            if let Some(note) = &row.note {
                self.text.draw(
                    &mut self.canvas,
                    note,
                    about,
                    c.x,
                    ty + 2.0,
                    mix(p.accent, p.text, 0.45),
                    clip,
                    Some(text_w),
                );
                ty += note_h;
            }
            if let Some(yours) = &row.yours {
                // Yours: cargo check  ↺ Use yours
                let f = Face::ui(12.0);
                let lx = c.x + self.s_row_text("Yours: ", f, c.x, ty + 2.0, 18.0, p.muted, clip);
                let lx = lx
                    + self.s_row_text(yours, f.medium(), lx, ty + 2.0, 18.0, p.text, clip)
                    + 10.0;
                let link = "↺ Use yours";
                let lw = self.text.width(link, f);
                let b = Rect::new(lx, ty + 2.0, lw, 18.0);
                let fg = if self.s_hovered(b, clip) {
                    mix(p.accent, 0xffffff, 0.8)
                } else {
                    p.accent
                };
                self.s_row_text(link, f, lx, b.y, b.h, fg, clip);
                self.s_hit(
                    b,
                    clip,
                    Hit::Settings(SettingsAction::Reset(row.key.clone())),
                );
            }
            let right = c.x + c.w;
            let ctl = Rect::new(right - cw, c.y + (h - 28.0) / 2.0, cw, 28.0);
            if reset {
                let b = Rect::new(ctl.x - 28.0, ctl.y + 2.0, 24.0, 24.0);
                let fg = if self.s_hovered(b, clip) {
                    mix(p.accent, 0xffffff, 0.8)
                } else {
                    p.accent
                };
                self.s_row_text("↺", Face::ui(14.0), b.x + 5.0, b.y, b.h, fg, clip);
                self.s_hit(
                    b,
                    clip,
                    Hit::Settings(SettingsAction::Reset(row.key.clone())),
                );
            }
            // A locked row's control shows its value, and does nothing.
            let at = self.hits.len();
            self.control(row, ctl, seg, editing, clip);
            if row.locked {
                self.hits.truncate(at);
            } else if open {
                c.list = Some((ctl, ListOf::Row(row.clone())));
            }
            self.s_fill(Rect::new(c.x, c.y + h - 1.0, c.w, 1.0), clip, p.line);
        }
        c.y += h;
    }

    /// A tag for what's so whichever tab is open: greyer than the others.
    fn quiet_tag(&mut self, label: &str, x: f32, mid: f32, clip: Rect) -> f32 {
        let p = self.p;
        let face = Face::ui(10.5).medium();
        let tw = self.text.width(label, face);
        let r = Rect::new(x, mid - 8.0, tw + 12.0, 16.0);
        self.s_outline(r, 4.0, clip, p.border, p.island);
        self.s_row_text(label, face, x + 6.0, r.y, r.h, p.muted, clip);
        r.w
    }

    /// How wide a control is, and whether a choice is shown as a
    /// segmented row rather than a list.
    pub(super) fn control_width(
        &mut self,
        control: &Control,
        editing: Option<&Editing>,
    ) -> (f32, bool) {
        match control {
            Control::Toggle(_) => (34.0, false),
            Control::Number(_) => (104.0, false),
            Control::Field(..) => (FIELD_W, false),
            Control::List(items) => {
                let face = Face::ui(12.0);
                let chips: f32 = items
                    .iter()
                    .map(|i| self.text.width(i, face) + CHIP_PAD + CHIP_GAP)
                    .sum();
                let add = if editing.is_some() {
                    ADD_FIELD_W
                } else {
                    self.text.width(ADD, face) + 12.0
                };
                ((chips + add).min(LIST_MAX_W), false)
            }
            Control::Choice(options, _) => {
                let face = Face::ui(12.0);
                let seg: f32 = options
                    .iter()
                    .map(|(_, l)| self.text.width(l, face) + 20.0)
                    .sum::<f32>()
                    + 6.0;
                // A few short ones side by side; more open a list.
                if options.len() <= 4 && seg <= 300.0 {
                    (seg, true)
                } else {
                    let widest = options
                        .iter()
                        .map(|(_, l)| self.text.width(l, Face::ui(12.5)))
                        .fold(0.0, f32::max);
                    ((widest + 44.0).max(150.0), false)
                }
            }
        }
    }

    pub(super) fn control(
        &mut self,
        row: &Row,
        r: Rect,
        seg: bool,
        editing: Option<&Editing>,
        clip: Rect,
    ) {
        let p = self.p;
        let key = || row.key.clone();
        match &row.control {
            Control::Toggle(on) => {
                let s = Rect::new(r.x, r.y + 4.0, 34.0, 20.0);
                self.switch(*on, s, clip, Hit::Settings(SettingsAction::Toggle(key())));
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
                    self.s_hit(b, clip, Hit::Settings(SettingsAction::Step(key(), by)));
                }
            }
            Control::Field(value, kind) => {
                let typing = editing.filter(|e| !e.add).map(|e| e.text.as_str());
                let hit = Hit::Settings(SettingsAction::Edit(key(), false));
                let mono = matches!(kind, FieldKind::Path);
                if *kind == FieldKind::Secret {
                    // Dots, never the secret: one per letter typed.
                    let dots = typing.map(|t| "•".repeat(t.chars().count()));
                    let value = if value.is_empty() {
                        ""
                    } else {
                        "••••••••"
                    };
                    self.text_field(value, dots.as_deref(), false, row.locked, r, clip, hit);
                } else {
                    self.text_field(value, typing, mono, row.locked, r, clip, hit);
                }
            }
            Control::List(items) => {
                let face = Face::ui(12.0);
                let mut x = r.x;
                for (i, item) in items.iter().enumerate() {
                    let w = self.text.width(item, face) + CHIP_PAD;
                    if x + w > r.right() - 40.0 && i > 0 {
                        break;
                    }
                    let chip = Rect::new(x, r.y + 2.0, w, r.h - 4.0);
                    self.s_round(chip, 5.0, clip, p.chip);
                    self.s_row_text(item, face, x + 8.0, chip.y, chip.h, p.chip_text, clip);
                    let b = Rect::new(chip.right() - 20.0, chip.y, 18.0, chip.h);
                    let fg = if self.s_hovered(b, clip) {
                        p.bright
                    } else {
                        p.muted
                    };
                    self.s_row_text("×", face, b.x + 4.0, b.y, b.h, fg, clip);
                    self.s_hit(b, clip, Hit::Settings(SettingsAction::Unlist(key(), i)));
                    x += w + CHIP_GAP;
                }
                match editing.filter(|e| e.add) {
                    Some(e) => {
                        let f = Rect::new(r.right() - ADD_FIELD_W, r.y, ADD_FIELD_W, r.h);
                        let hit = Hit::Settings(SettingsAction::Edit(key(), true));
                        self.text_field("", Some(&e.text), false, false, f, clip, hit);
                    }
                    None => {
                        let aw = self.text.width(ADD, face) + 12.0;
                        let b = Rect::new(r.right() - aw, r.y, aw, r.h);
                        let fg = if self.s_hovered(b, clip) {
                            mix(p.accent, 0xffffff, 0.8)
                        } else {
                            p.accent
                        };
                        self.s_row_text(ADD, face, b.x + 6.0, b.y, b.h, fg, clip);
                        self.s_hit(b, clip, Hit::Settings(SettingsAction::Edit(key(), true)));
                    }
                }
            }
            Control::Choice(options, now) if seg => {
                self.s_outline(r, 7.0, clip, p.border, p.raised);
                let face = Face::ui(12.0);
                let mut x = r.x + 3.0;
                for (value, label) in options {
                    let w = self.text.width(label, face) + 20.0;
                    let b = Rect::new(x, r.y + 2.0, w, r.h - 4.0);
                    let on = value == now;
                    if on {
                        self.s_round(b, 5.0, clip, mix(p.accent, p.raised, 0.3));
                    } else if self.s_hovered(b, clip) && !row.locked {
                        self.s_round(b, 5.0, clip, p.hover);
                    }
                    let fg = if on { p.bright } else { p.chip_text };
                    self.s_row_text(label, face, x + 10.0, b.y, b.h, fg, clip);
                    self.s_hit(
                        b,
                        clip,
                        Hit::Settings(SettingsAction::Choose(key(), value.clone())),
                    );
                    x += w;
                }
            }
            Control::Choice(options, now) => {
                let bg = if self.s_hovered(r, clip) && !row.locked {
                    p.hover
                } else {
                    p.raised
                };
                self.s_outline(r, 6.0, clip, p.border, bg);
                let face = Face::ui(12.5);
                let label = options
                    .iter()
                    .find(|(v, _)| v == now)
                    .map_or_else(|| option_label(now), |(_, l)| l.clone());
                self.s_row_text(&label, face, r.x + 10.0, r.y, r.h, p.bright, clip);
                self.s_row_text("▾", face, r.right() - 20.0, r.y, r.h, p.muted, clip);
                self.s_hit(r, clip, Hit::Settings(SettingsAction::OpenChoice(key())));
            }
        }
    }

    /// A box of text: its value, or what's being typed with a caret.
    #[allow(clippy::too_many_arguments)]
    fn text_field(
        &mut self,
        value: &str,
        typing: Option<&str>,
        mono: bool,
        locked: bool,
        r: Rect,
        clip: Rect,
        hit: Hit,
    ) {
        let p = self.p;
        let face = if mono {
            Face::mono(11.5)
        } else {
            Face::ui(12.5)
        };
        let border = if typing.is_some() {
            p.accent
        } else if self.s_hovered(r, clip) && !locked {
            mix(p.border, p.muted, 0.6)
        } else {
            p.border
        };
        let inside = if locked { p.island } else { p.raised };
        self.s_outline(r, 6.0, clip, border, inside);
        let inner = Rect::new(r.x + 10.0, r.y, r.w - 20.0, r.h).intersect(clip);
        match typing {
            Some(text) => {
                // The end of a long text shows, where the caret is.
                let mut shown = text;
                while self.text.width(shown, face) > inner.w - 4.0 && !shown.is_empty() {
                    let cut = shown.char_indices().nth(1).map_or(shown.len(), |(i, _)| i);
                    shown = &shown[cut..];
                }
                let tw = self.s_row_text(shown, face, inner.x, r.y, r.h, p.bright, inner);
                self.s_fill(
                    Rect::new(inner.x + tw + 1.0, r.y + 7.0, 1.5, r.h - 14.0),
                    clip,
                    p.accent,
                );
            }
            None if value.is_empty() => {
                self.s_row_text("Not set", Face::ui(12.5), inner.x, r.y, r.h, p.faint, inner);
            }
            None => {
                let fg = if locked { p.muted } else { p.text };
                let shown = self.text.fit(value, face, inner.w);
                self.s_row_text(&shown, face, inner.x, r.y, r.h, fg, inner);
            }
        }
        self.s_hit(r, clip, hit);
    }

    pub(super) fn switch(&mut self, on: bool, s: Rect, clip: Rect, hit: Hit) {
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
    pub(super) fn choice_list(&mut self, under: Rect, of: &ListOf, page: Rect) {
        let p = self.p;
        let items: Vec<(String, String, Hit, bool)> = match of {
            ListOf::Row(row) => match &row.control {
                Control::Choice(options, now) => options
                    .iter()
                    .map(|(value, label)| {
                        (
                            label.clone(),
                            String::new(),
                            Hit::Settings(SettingsAction::Choose(row.key.clone(), value.clone())),
                            value == now,
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
            ListOf::PluginMenu(_) | ListOf::Confirm(_) => return,
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
}
