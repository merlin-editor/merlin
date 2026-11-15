//! The title bar's update pill and the card it opens, as in
//! `docs/mockups/AppUpdate.dc.html` and `AppUpdateStates.html`.

use super::*;
use merlin_core::{ButtonStyle, StepMark, UpdateHit, UpdateView};

/// The pill's height, and the gap between it and the layout buttons.
const PILL_H: f32 = 26.0;
const PILL_GAP: f32 = 10.0;
/// The card's padding, the gap between its parts, and its widths.
const PAD: f32 = 16.0;
const GAP: f32 = 12.0;
const CARD_W: f32 = 320.0;
const WIDE_W: f32 = 380.0;
/// The release notes scroll past this height.
const NOTES_MAX: f32 = 210.0;
const STEP_H: f32 = 21.0;
const TEXT_LINE: f32 = 18.0;
const BUTTON_H: f32 = 26.0;

impl Frame<'_> {
    /// Where the pill goes: ending a gap before the layout buttons, which
    /// start at `toggles`.
    fn pill_rect(&mut self, update: &UpdateView, toggles: f32) -> Rect {
        let label = self.text.width(&update.pill, Face::ui(12.0).medium());
        let arrow = if update.arrow { 7.0 + 9.0 } else { 0.0 };
        let w = 8.0 + 9.0 + 7.0 + label + arrow + 10.0;
        Rect::new(toggles - PILL_GAP - w, (TITLE_H - PILL_H) / 2.0, w, PILL_H)
    }

    /// The pill, before the layout buttons at `toggles`, with a line
    /// between them.
    pub(super) fn update_pill(&mut self, update: &UpdateView, toggles: f32) {
        let p = self.p;
        let r = self.pill_rect(update, toggles);
        let hovered = self.hovered(r) || update.card.is_some();
        let (ground, edge, ink) = if hovered {
            (
                mix(p.ground, p.accent, 0.82),
                mix(p.ground, p.accent, 0.55),
                mix(p.accent, 0xffffff, 0.6),
            )
        } else {
            (
                mix(p.ground, p.accent, 0.88),
                mix(p.ground, p.accent, 0.7),
                mix(p.accent, 0xffffff, 0.8),
            )
        };
        self.canvas.outline(r, PILL_H / 2.0, 1.0, edge, ground);
        let mut x = r.x + 8.0;
        let cy = r.y + PILL_H / 2.0;
        if update.busy {
            let ring = Rect::new(x, cy - 4.5, 9.0, 9.0);
            self.canvas.outline(ring, 4.5, 1.6, edge, ground);
            self.canvas
                .round(Rect::new(x + 3.0, cy - 4.5, 3.0, 3.0), 1.5, p.accent);
        } else {
            // The dot, with a glow around it.
            self.canvas.round(
                Rect::new(x - 2.0, cy - 5.5, 11.0, 11.0),
                5.5,
                mix(ground, p.accent, 0.7),
            );
            self.canvas
                .round(Rect::new(x, cy - 3.5, 7.0, 7.0), 3.5, p.accent);
        }
        x += 9.0 + 7.0;
        let face = Face::ui(12.0).medium();
        self.text
            .draw_in_row(&mut self.canvas, &update.pill, face, x, r.y, r.h, ink, r);
        if update.arrow {
            x += self.text.width(&update.pill, face) + 7.0;
            self.text
                .draw_in_row(&mut self.canvas, "↓", Face::ui(11.0), x, r.y, r.h, ink, r);
        }
        self.hit(r, Hit::Ui(UiEvent::Update(UpdateHit::Pill)));
        // The line between the pill and the layout buttons.
        self.canvas.fill(
            Rect::new(toggles - PILL_GAP / 2.0, r.y + 4.0, 1.0, r.h - 8.0),
            p.border,
        );
    }

    /// The card under the pill, its right edge under the pill's.
    pub(super) fn update_card(&mut self, update: &UpdateView, toggles: f32, w: f32, h: f32) {
        let Some(card) = &update.card else {
            return;
        };
        let p = self.p;
        let pill = self.pill_rect(update, toggles);
        let cw = if card.wide { WIDE_W } else { CARD_W }.min(w - 20.0);
        let inner = cw - 2.0 * PAD;
        // Lay it out first, for its height.
        let text_lines = if card.text.is_empty() {
            Vec::new()
        } else {
            self.wrap_words(&card.text, Face::ui(12.0), inner)
        };
        let notes = card.notes.as_ref().map(|(md, _)| self.md_layout(md, inner));
        let mut parts: Vec<f32> = vec![20.0];
        if card.progress.is_some() {
            parts.push(4.0);
        }
        if !card.steps.is_empty() {
            parts.push(card.steps.len() as f32 * STEP_H);
        }
        if let Some((_, nh)) = &notes {
            parts.push(nh.min(NOTES_MAX) + 20.0);
        }
        if card.page_link {
            parts.push(16.0);
        }
        if !text_lines.is_empty() {
            parts.push(text_lines.len() as f32 * TEXT_LINE);
        }
        if card.command.is_some() {
            parts.push(32.0);
        }
        if !card.buttons.is_empty() {
            parts.push(BUTTON_H + 4.0);
        }
        let ch = 2.0 * PAD + parts.iter().sum::<f32>() + GAP * (parts.len() as f32 - 1.0);
        let right = pill.right().min(w - 10.0);
        let r = Rect::new(
            (right - cw).max(10.0),
            TITLE_H + 6.0,
            cw,
            ch.min(h - TITLE_H - 16.0),
        );
        self.canvas.shadow(r, 10.0, theme::is_light(&p));
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);

        let x = r.x + PAD;
        let mut y = r.y + PAD;
        // The title, and the version or percentage in a chip at its right.
        let mono = Face::mono(11.0);
        let bw = self.text.width(&card.badge, mono) + 12.0;
        let badge = Rect::new(r.right() - PAD - bw, y + 1.0, bw, 18.0);
        self.canvas.round(badge, 4.0, mix(p.widget, p.accent, 0.84));
        self.text.draw_in_row(
            &mut self.canvas,
            &card.badge,
            mono,
            badge.x + 6.0,
            badge.y,
            badge.h,
            p.accent,
            r,
        );
        let title_f = Face::ui(13.5).bold();
        let title = self
            .text
            .fit(&card.title, title_f, (badge.x - x - 8.0).max(0.0));
        self.text
            .draw_in_row(&mut self.canvas, &title, title_f, x, y, 20.0, p.bright, r);
        y += 20.0 + GAP;

        if let Some(done) = card.progress {
            let track = Rect::new(x, y, inner, 4.0);
            self.canvas.round(track, 2.0, p.chip);
            self.canvas.round(
                Rect::new(x, y, inner * done.clamp(0.0, 1.0), 4.0),
                2.0,
                p.accent,
            );
            y += 4.0 + GAP;
        }

        if !card.steps.is_empty() {
            for step in &card.steps {
                self.update_step(step, x, y, inner, r);
                y += STEP_H;
            }
            y += GAP;
        }

        if let (Some((ops, nh)), Some((_, scroll))) = (&notes, &card.notes) {
            let view_h = nh.min(NOTES_MAX);
            let max = (nh - view_h).max(0.0);
            let scroll = (*scroll as f32).clamp(0.0, max);
            self.canvas
                .fill(Rect::new(r.x + PAD, y, inner, 1.0), p.border);
            let area = Rect::new(x, y + 10.0, inner, view_h);
            self.md_draw(ops, x, area.y - scroll, inner, &[], p.text, area);
            // Fade the last lines out while there's more below.
            if scroll < max {
                for i in 0..6 {
                    let t = (i + 1) as f32 / 6.0;
                    self.canvas.tint(
                        Rect::new(
                            r.x + 1.0,
                            area.bottom() - 18.0 + i as f32 * 3.0,
                            r.w - 2.0,
                            3.0,
                        ),
                        0.0,
                        p.widget,
                        (t * 230.0) as u8,
                    );
                }
            }
            self.canvas.fill(
                Rect::new(r.x + PAD, area.bottom() + 10.0, inner, 1.0),
                p.border,
            );
            self.hit(area, Hit::UpdateNotes(max as i32));
            y += view_h + 20.0 + GAP;
        }

        if card.page_link {
            let label = "Open on GitHub ↗";
            let f = Face::ui(12.0);
            let l = Rect::new(x, y, self.text.width(label, f), 16.0);
            let colour = if self.hovered(l) { p.bright } else { p.accent };
            self.text
                .draw_in_row(&mut self.canvas, label, f, x, y, 16.0, colour, r);
            self.hit(l, Hit::Ui(UiEvent::Update(UpdateHit::Page)));
            y += 16.0 + GAP;
        }

        if !text_lines.is_empty() {
            let colour = if card.error { p.error } else { p.muted };
            for line in &text_lines {
                self.text.draw_in_row(
                    &mut self.canvas,
                    line,
                    Face::ui(12.0),
                    x,
                    y,
                    TEXT_LINE,
                    colour,
                    r,
                );
                y += TEXT_LINE;
            }
            y += GAP;
        }

        if let Some(command) = &card.command {
            let row = Rect::new(x, y, inner, 32.0);
            self.canvas
                .outline(row, 6.0, 1.0, p.border, mix(p.widget, p.ground, 0.5));
            let f = Face::ui(11.5);
            let copy = Rect::new(row.right() - 6.0 - 48.0, row.y + 5.0, 48.0, 22.0);
            let (bg, fg) = self.button_colours(false, self.hovered(copy));
            self.canvas.round(copy, 5.0, bg);
            let cx = copy.x + (copy.w - self.text.width("Copy", f)) / 2.0;
            self.text
                .draw_in_row(&mut self.canvas, "Copy", f, cx, copy.y, copy.h, fg, copy);
            self.hit(copy, Hit::Ui(UiEvent::Update(UpdateHit::Copy)));
            let mono = Face::mono(11.5);
            let fitted = self
                .text
                .fit(command, mono, (copy.x - row.x - 18.0).max(0.0));
            self.text.draw_in_row(
                &mut self.canvas,
                &fitted,
                mono,
                row.x + 10.0,
                row.y,
                row.h,
                p.text,
                row,
            );
            y += 32.0 + GAP;
        }

        if !card.buttons.is_empty() {
            // Right-aligned, the primary last.
            let f = Face::ui(12.0);
            let by = y + 4.0;
            let mut bx = r.right() - PAD;
            for b in card.buttons.iter().rev() {
                let bw = self.text.width(&b.label, f) + 22.0;
                let br = Rect::new(bx - bw, by, bw, BUTTON_H);
                let hovered = self.hovered(br);
                let (bg, fg) = match b.style {
                    ButtonStyle::Primary => self.button_colours(true, hovered),
                    ButtonStyle::Plain => self.button_colours(false, hovered),
                    ButtonStyle::Ghost => (0, if hovered { p.text } else { p.muted }),
                };
                if b.style != ButtonStyle::Ghost {
                    self.canvas.round(br, 6.0, bg);
                }
                let face = if b.style == ButtonStyle::Primary {
                    f.bold()
                } else {
                    f
                };
                let tx = br.x + (bw - self.text.width(&b.label, face)) / 2.0;
                self.text
                    .draw_in_row(&mut self.canvas, &b.label, face, tx, br.y, br.h, fg, br);
                self.hit(br, Hit::Ui(UiEvent::Update(b.hit)));
                bx = br.x - 8.0;
            }
        }
    }

    /// A line of the card's steps: its mark, label, and detail at the right.
    fn update_step(
        &mut self,
        step: &merlin_core::UpdateStepView,
        x: f32,
        y: f32,
        w: f32,
        card: Rect,
    ) {
        let p = self.p;
        let m = Rect::new(x, y + (STEP_H - 14.0) / 2.0, 14.0, 14.0);
        let colour = match step.mark {
            StepMark::Done => {
                self.canvas.round(m, 7.0, mix(p.widget, p.accent, 0.75));
                self.centred("✓", Face::ui(11.0).bold(), m, p.accent, card);
                p.text
            }
            StepMark::Now => {
                self.canvas
                    .outline(m, 7.0, 1.5, mix(p.widget, p.accent, 0.6), p.widget);
                self.canvas
                    .round(Rect::new(m.x + 5.5, m.y, 3.0, 3.0), 1.5, p.accent);
                p.bright
            }
            StepMark::Todo => {
                self.canvas.outline(m, 7.0, 1.5, p.border, p.widget);
                p.muted
            }
        };
        let f = Face::ui(12.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &step.label,
            f,
            x + 23.0,
            y,
            STEP_H,
            colour,
            card,
        );
        if !step.detail.is_empty() {
            let mono = Face::mono(11.0);
            let dw = self.text.width(&step.detail, mono);
            self.text.draw_in_row(
                &mut self.canvas,
                &step.detail,
                mono,
                x + w - dw,
                y,
                STEP_H,
                p.muted,
                card,
            );
        }
    }
}
