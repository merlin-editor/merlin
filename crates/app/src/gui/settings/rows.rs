//! A setting's row: its label and tags, its control, a switch, and the
//! list a choice opens.

use super::*;

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
    /// the right.
    pub(super) fn setting_row(&mut self, row: &Row, open: bool, c: &mut Ctx) {
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
    pub(super) fn control_width(&mut self, control: &Control) -> (f32, bool) {
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

    pub(super) fn control(&mut self, row: &Row, r: Rect, seg: bool, clip: Rect) {
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
}
