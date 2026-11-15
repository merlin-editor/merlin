//! What an agent's chat adds to the side bar's drawing, after
//! `docs/mockups/Agents.html`: its plan, a quiet line per step, the
//! permission card, the files a turn changed, and a command to copy.

use super::*;
use merlin_core::{AskCard, Button, ChangesCard, StepLine, StepState};

const SMALL_H: f32 = 24.0;

/// How a small button looks.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Look {
    Primary,
    Plain,
    Quiet,
}

impl Frame<'_> {
    /// A plan's or a step's mark: a tick, a spinner, a cross or a ring.
    pub(super) fn step_mark(&mut self, mark: StepState, x: f32, y: f32, h: f32, clip: Rect) {
        let p = self.p;
        let mid = y + h / 2.0;
        match mark {
            StepState::Done => {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "✓",
                    Face::ui(12.0),
                    x,
                    y,
                    h,
                    p.success,
                    clip,
                );
            }
            StepState::Failed => {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "✗",
                    Face::ui(12.0),
                    x,
                    y,
                    h,
                    p.error,
                    clip,
                );
            }
            StepState::Running => {
                let ring = Rect::new(x + 1.0, mid - 5.0, 10.0, 10.0);
                self.canvas
                    .outline(ring, 5.0, 1.6, mix(p.accent, p.island, 0.35), p.island);
                self.canvas
                    .round(Rect::new(x + 4.5, mid - 5.0, 3.0, 3.0), 1.5, p.accent);
            }
            StepState::Pending => {
                let ring = Rect::new(x + 1.0, mid - 5.0, 10.0, 10.0);
                self.canvas.outline(ring, 5.0, 1.4, p.faint, p.island);
            }
        }
    }

    pub(super) fn steps_height(steps: &[(String, StepState)]) -> f32 {
        steps.len() as f32 * 20.0 + 8.0
    }

    /// An agent's plan, each step with its mark.
    pub(super) fn steps(&mut self, steps: &[(String, StepState)], r: Rect, clip: Rect) {
        let p = self.p;
        let mut y = r.y + 2.0;
        for (text, mark) in steps {
            self.step_mark(*mark, r.x + 14.0, y, 20.0, clip);
            let colour = match mark {
                StepState::Done => p.text,
                StepState::Running => p.bright,
                _ => p.muted,
            };
            let t = self.text.fit(text, Face::ui(12.0), r.w - 50.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &t,
                Face::ui(12.0),
                r.x + 30.0,
                y,
                20.0,
                colour,
                clip,
            );
            y += 20.0;
        }
    }

    /// A small button: 24 tall, its parts in backticks drawn as code.
    /// Returns its width.
    pub(super) fn small_button(
        &mut self,
        label: &str,
        x: f32,
        y: f32,
        look: Look,
        focused: bool,
    ) -> f32 {
        let p = self.p;
        let parts = code_parts(label);
        let (ui, mono) = (Face::ui(12.0), Face::mono(11.0));
        let tw: f32 = parts
            .iter()
            .map(|(t, code)| {
                if *code {
                    self.text.width(t, mono) + 10.0
                } else {
                    self.text.width(t, ui)
                }
            })
            .sum();
        let r = Rect::new(x, y, tw + 18.0, SMALL_H);
        let hovered = self.hovered(r);
        let (bg, fg) = match look {
            Look::Primary => self.button_colours(true, hovered),
            Look::Plain => self.button_colours(false, hovered),
            Look::Quiet => (if hovered { p.hover } else { 0 }, p.bright),
        };
        if focused {
            self.canvas
                .outline(r.inset(-2.0), 8.0, 1.5, p.accent, p.island);
        }
        if bg != 0 {
            self.canvas.round(r, 6.0, bg);
        }
        let face = if look == Look::Primary {
            ui.medium()
        } else {
            ui
        };
        let mut tx = x + 9.0;
        for (t, code) in parts {
            if code {
                let w = self.text.width(&t, mono);
                let chip = Rect::new(tx + 2.0, y + 4.0, w + 8.0, SMALL_H - 8.0);
                self.canvas
                    .round(chip, 4.0, mix(p.ground, bg.max(p.chip), 0.5));
                self.text
                    .draw_in_row(&mut self.canvas, &t, mono, tx + 6.0, y, SMALL_H, p.text, r);
                tx += w + 10.0;
            } else {
                tx += self
                    .text
                    .draw_in_row(&mut self.canvas, &t, face, tx, y, SMALL_H, fg, r);
            }
        }
        r.w
    }

    /// One step an agent took: its mark, what it did, the file or command
    /// in code, lines added and removed, and links and buttons at the
    /// right end.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn step_line(
        &mut self,
        s: &StepLine,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let (y, h) = (r.y, r.h);
        self.step_mark(s.mark, r.x + 14.0, y, h, clip);
        // The right end first, so the name can stop short of it.
        let mut right = r.right() - 14.0;
        let mut k = s.links.len() + s.buttons.len();
        for b in s.buttons.iter().rev() {
            k -= 1;
            let w = self.text.width(&b.label, Face::ui(12.0)) + 18.0;
            right -= w;
            let index = target.map(|t| t + k);
            let focused = sv.focused && index == Some(sv.selected);
            self.small_button(
                &b.label,
                right,
                y + (h - SMALL_H) / 2.0,
                Look::Quiet,
                focused,
            );
            if let Some(index) = index {
                self.hit(Rect::new(right, y, w, h), chat_pick(side, index));
            }
            right -= 4.0;
        }
        for b in s.links.iter().rev() {
            k -= 1;
            let f = Face::ui(11.5);
            let w = self.text.width(&b.label, f);
            right -= w;
            let index = target.map(|t| t + k);
            let hit = Rect::new(right, y, w, h);
            let colour = if self.hovered(hit) {
                mix(p.accent, 0xffffff, 0.8)
            } else {
                p.accent
            };
            self.text
                .draw_in_row(&mut self.canvas, &b.label, f, right, y, h, colour, clip);
            if sv.focused && index == Some(sv.selected) {
                self.canvas
                    .fill(Rect::new(right, y + h - 4.0, w, 1.0), p.accent);
            }
            if let Some(index) = index {
                self.hit(hit, chat_pick(side, index));
            }
            right -= 10.0;
        }
        let text_clip = Rect::new(r.x, r.y, (right - r.x).max(0.0), r.h).intersect(clip);
        let mut x = r.x + 30.0;
        if !s.verb.is_empty() {
            x += self.text.draw_in_row(
                &mut self.canvas,
                &s.verb,
                Face::ui(12.0),
                x,
                y,
                h,
                p.text,
                text_clip,
            ) + 6.0;
        }
        let mono = Face::mono(11.0);
        let name = self.text.fit(&s.name, mono, (right - x - 60.0).max(40.0));
        x += self
            .text
            .draw_in_row(&mut self.canvas, &name, mono, x, y, h, p.bright, text_clip);
        if s.added + s.removed > 0 {
            for (text, colour, show) in [
                (format!("+{}", s.added), p.success, s.added > 0),
                (format!("−{}", s.removed), p.error, s.removed > 0),
            ] {
                if !show {
                    continue;
                }
                x += 6.0;
                x += self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    mono,
                    x,
                    y,
                    h,
                    colour,
                    text_clip,
                );
            }
        }
    }

    pub(super) fn ask_height(&mut self, a: &AskCard, inner: f32) -> f32 {
        // "merlin asks", then the question.
        let mut h = 10.0 + 18.0 + self.ask_title_height(a, inner - 48.0) + 8.0;
        if a.code.is_some() {
            h += 28.0 + 8.0;
        }
        if !a.diff.is_empty() {
            h += a.diff.len() as f32 * 17.0 + 12.0 + 8.0;
        }
        if !a.note.is_empty() {
            h += 16.0 + 8.0;
        }
        h += self.ask_rows(a, inner - 48.0) as f32 * (SMALL_H + 6.0) - 6.0;
        if !a.hint.is_empty() {
            h += 8.0 + 14.0;
        }
        h + 10.0 + 12.0
    }

    /// The width of a small button.
    fn small_width(&mut self, label: &str) -> f32 {
        code_parts(label)
            .iter()
            .map(|(t, code)| {
                if *code {
                    self.text.width(t, Face::mono(11.0)) + 10.0
                } else {
                    self.text.width(t, Face::ui(12.0))
                }
            })
            .sum::<f32>()
            + 18.0
    }

    /// The rows a card's answers take at width `w`.
    fn ask_rows(&mut self, a: &AskCard, w: f32) -> usize {
        let mut rows = 1;
        let mut x = 0.0;
        for b in a.buttons.iter().chain(&a.quiet) {
            let bw = self.small_width(&b.label);
            if x > 0.0 && x + bw > w {
                rows += 1;
                x = 0.0;
            }
            x += bw + 6.0;
        }
        rows
    }

    fn ask_title_height(&mut self, a: &AskCard, w: f32) -> f32 {
        let text: String = a.title.iter().map(|(t, _)| t.as_str()).collect();
        self.text
            .wrapped_height(&text, Face::ui(12.5).bold(), w)
            .max(18.0)
    }

    /// A permission card: in blue, or amber for something outside the
    /// workspace. The question, the command or path in a box, or the
    /// change as diff lines; a note; the answers, the main one first; and
    /// a hint at the keys.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ask_card(
        &mut self,
        a: &AskCard,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let tint = if a.warn { p.warning } else { p.accent };
        let card = Rect::new(r.x + 12.0, r.y + 6.0, r.w - 24.0, r.h - 12.0);
        self.canvas.outline(
            card,
            8.0,
            1.0,
            mix(tint, p.island, 0.38),
            mix(tint, p.island, 0.08),
        );
        let inside = card.inset(1.0).intersect(clip);
        let x = card.x + 12.0;
        let w = card.w - 24.0;
        let mut y = card.y + 10.0;
        // merlin's mark, which a plugin's pieces can't draw: a lock and
        // "merlin asks".
        let mark = mix(tint, p.bright, 0.35);
        let lock_bg = mix(tint, p.island, 0.08);
        self.canvas.outline(
            Rect::new(x + 1.5, y + 1.0, 6.0, 8.0),
            3.0,
            1.3,
            mark,
            lock_bg,
        );
        self.canvas
            .round(Rect::new(x, y + 5.0, 9.0, 6.5), 1.5, mark);
        self.text.draw_in_row(
            &mut self.canvas,
            "MERLIN ASKS",
            Face::ui(10.5).bold(),
            x + 15.0,
            y - 2.0,
            16.0,
            mark,
            inside,
        );
        y += 18.0;
        // The question, its code parts in mono.
        let title_h = self.ask_title_height(a, w);
        let mut tx = x;
        for (text, code) in &a.title {
            if *code {
                let mono = Face::mono(11.0);
                let tw = self.text.width(text, mono);
                let chip = Rect::new(tx + 2.0, y + 1.0, tw + 8.0, 17.0);
                self.canvas.round(chip, 4.0, p.chip);
                self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    mono,
                    tx + 6.0,
                    y,
                    19.0,
                    p.bright,
                    inside,
                );
                tx += tw + 12.0;
            } else {
                tx += self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    Face::ui(12.5).bold(),
                    tx,
                    y,
                    19.0,
                    p.bright,
                    inside,
                );
            }
        }
        y += title_h + 8.0;
        if let Some(code) = &a.code {
            let b = Rect::new(x, y, w, 28.0);
            self.canvas.outline(b, 6.0, 1.0, p.line, p.ground);
            let mono = Face::mono(11.5);
            let t = self.text.fit(code, mono, w - 20.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &t,
                mono,
                b.x + 10.0,
                b.y,
                b.h,
                p.text,
                inside,
            );
            y += 28.0 + 8.0;
        }
        if !a.diff.is_empty() {
            let b = Rect::new(x, y, w, a.diff.len() as f32 * 17.0 + 12.0);
            self.canvas.outline(b, 6.0, 1.0, p.line, p.ground);
            let mut ly = b.y + 6.0;
            let mono = Face::mono(11.0);
            for line in &a.diff {
                let (bg, fg) = if line.starts_with('-') {
                    (mix(p.error, p.ground, 0.16), p.error)
                } else {
                    (mix(p.success, p.ground, 0.14), p.success)
                };
                self.canvas
                    .fill(Rect::new(b.x + 6.0, ly, b.w - 12.0, 17.0), bg);
                let t = self.text.fit(line, mono, b.w - 24.0);
                self.text
                    .draw_in_row(&mut self.canvas, &t, mono, b.x + 10.0, ly, 17.0, fg, inside);
                ly += 17.0;
            }
            y += b.h + 8.0;
        }
        if !a.note.is_empty() {
            self.text.draw_in_row(
                &mut self.canvas,
                &a.note,
                Face::ui(11.5),
                x,
                y,
                16.0,
                p.muted,
                inside,
            );
            y += 16.0 + 8.0;
        }
        let mut bx = x;
        let quiet = a.quiet.iter().map(|b| (b, Look::Quiet));
        let buttons = a
            .buttons
            .iter()
            .enumerate()
            .map(|(n, b)| (b, if n == 0 { Look::Primary } else { Look::Plain }))
            .chain(quiet)
            .collect::<Vec<(&Button, Look)>>();
        for (k, (b, look)) in buttons.into_iter().enumerate() {
            let index = target.map(|t| t + k);
            let focused = sv.focused && index == Some(sv.selected);
            // Answers that don't fit go on the next line.
            let need = self.small_width(&b.label);
            if bx > x && bx + need > x + w {
                bx = x;
                y += SMALL_H + 6.0;
            }
            let bw = self.small_button(&b.label, bx, y, look, focused);
            if let Some(index) = index {
                self.hit(Rect::new(bx, y, bw, SMALL_H), chat_pick(side, index));
            }
            bx += bw + 6.0;
        }
        y += SMALL_H;
        if !a.hint.is_empty() {
            y += 8.0;
            self.text.draw_in_row(
                &mut self.canvas,
                &a.hint,
                Face::ui(11.0),
                x,
                y,
                14.0,
                p.muted,
                inside,
            );
        }
    }

    pub(super) fn changes_height(c: &ChangesCard) -> f32 {
        12.0 + 28.0 + c.files.len() as f32 * 26.0 + 10.0 + 12.0
    }

    /// The files a turn changed: "Changed 3 files" with Revert All, then
    /// each file with its tile, name, lines added and removed, Review and
    /// Revert.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn changes_card(
        &mut self,
        c: &ChangesCard,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = Rect::new(r.x + 12.0, r.y + 6.0, r.w - 24.0, r.h - 12.0);
        self.canvas.outline(card, 8.0, 1.0, p.border, p.island);
        let inside = card.inset(1.0).intersect(clip);
        let x = card.x + 10.0;
        let mut y = card.y + 6.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &c.title,
            Face::ui(12.5).bold(),
            x,
            y,
            28.0,
            p.bright,
            inside,
        );
        let rw = self.text.width(&c.revert_all.label, Face::ui(12.0)) + 18.0;
        let rx = card.right() - 8.0 - rw;
        let focused = sv.focused && target == Some(sv.selected);
        self.small_button(&c.revert_all.label, rx, y + 2.0, Look::Quiet, focused);
        if let Some(t) = target {
            self.hit(Rect::new(rx, y, rw, 28.0), chat_pick(side, t));
        }
        y += 28.0;
        for (n, f) in c.files.iter().enumerate() {
            let mut fx = x;
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            self.file_icon(&name, None, fx, y + 6.0, 14.0, inside);
            fx += 20.0;
            // Review and Revert at the right.
            let revert_w = self.text.width(&f.revert.label, Face::ui(12.0)) + 18.0;
            let review_w = self.text.width(&f.review.label, Face::ui(12.0)) + 18.0;
            let revert_x = card.right() - 8.0 - revert_w;
            let review_x = revert_x - 4.0 - review_w;
            let base = target.map(|t| t + 1 + 2 * n);
            let by = y + 1.0;
            let fr = base.is_some_and(|b| sv.focused && sv.selected == b);
            self.small_button(&f.review.label, review_x, by, Look::Plain, fr);
            let fv = base.is_some_and(|b| sv.focused && sv.selected == b + 1);
            self.small_button(&f.revert.label, revert_x, by, Look::Quiet, fv);
            if let Some(b) = base {
                self.hit(
                    Rect::new(review_x, by, review_w, SMALL_H),
                    chat_pick(side, b),
                );
                self.hit(
                    Rect::new(revert_x, by, revert_w, SMALL_H),
                    chat_pick(side, b + 1),
                );
            }
            let text_clip =
                Rect::new(card.x, y, (review_x - card.x - 6.0).max(0.0), 26.0).intersect(clip);
            fx += self.text.draw_in_row(
                &mut self.canvas,
                &name,
                Face::ui(12.0),
                fx,
                y,
                26.0,
                p.text,
                text_clip,
            );
            for (text, colour, show) in [
                (
                    format!("+{}", f.added),
                    p.success,
                    f.added > 0 || f.removed == 0,
                ),
                (format!("−{}", f.removed), p.error, f.removed > 0),
            ] {
                if show {
                    fx += 6.0;
                    fx += self.text.draw_in_row(
                        &mut self.canvas,
                        &text,
                        Face::mono(11.0),
                        fx,
                        y,
                        26.0,
                        colour,
                        text_clip,
                    );
                }
            }
            y += 26.0;
        }
    }

    /// A command in a box with Copy at its right end.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn command_box(
        &mut self,
        text: &str,
        copy: &Button,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let b = Rect::new(r.x + 14.0, r.y + 3.0, r.w - 28.0, 30.0);
        self.canvas.outline(b, 6.0, 1.0, p.line, p.ground);
        let cw = self.text.width(&copy.label, Face::ui(12.0)) + 18.0;
        let cx = b.right() - 3.0 - cw;
        let mono = Face::mono(11.5);
        let t = self.text.fit(text, mono, cx - b.x - 16.0);
        let inside = b.inset(1.0).intersect(clip);
        self.text.draw_in_row(
            &mut self.canvas,
            &t,
            mono,
            b.x + 10.0,
            b.y,
            b.h,
            p.text,
            inside,
        );
        let focused = sv.focused && target == Some(sv.selected);
        self.small_button(&copy.label, cx, b.y + 3.0, Look::Plain, focused);
        if let Some(t) = target {
            self.hit(Rect::new(cx, b.y, cw, b.h), chat_pick(side, t));
        }
    }
}

/// A label's parts, those in backticks marked as code.
fn code_parts(label: &str) -> Vec<(String, bool)> {
    label
        .split('`')
        .enumerate()
        .filter(|(_, t)| !t.is_empty())
        .map(|(n, t)| (t.to_string(), n % 2 == 1))
        .collect()
}

fn chat_pick(side: Side, index: usize) -> Hit {
    Hit::Ui(UiEvent::Target {
        side,
        index,
        clicks: 1,
    })
}

impl Frame<'_> {
    /// A menu in a panel, such as an agent's slash commands over its text
    /// box: its title, then a row per item, its detail at the right.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn menu_card(
        &mut self,
        title: &str,
        items: &[(String, String, String)],
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = Rect::new(r.x + 10.0, r.y + 6.0, r.w - 20.0, r.h - 6.0);
        self.canvas.outline(card, 8.0, 1.0, p.border, p.raised);
        let inside = card.inset(1.0).intersect(clip);
        let x = card.x + 10.0;
        let mut y = card.y + 4.0;
        self.text.draw_in_row(
            &mut self.canvas,
            title,
            Face::ui(11.0),
            x,
            y,
            26.0,
            p.muted,
            inside,
        );
        y += 26.0;
        // The row the keys are on; else the one under the mouse; else the
        // first, which Enter takes.
        let rows_y = y;
        let row_at =
            |n: usize| Rect::new(card.x + 4.0, rows_y + n as f32 * 26.0, card.w - 8.0, 26.0);
        let on = target
            .filter(|t| sv.focused && (*t..*t + items.len()).contains(&sv.selected))
            .map(|t| sv.selected - t)
            .or_else(|| (0..items.len()).find(|&n| self.hovered(row_at(n))))
            .unwrap_or(0);
        for (n, (_, label, detail)) in items.iter().enumerate() {
            let row = row_at(n);
            let on = n == on;
            if on {
                self.canvas.round(row, 5.0, p.hover);
            }
            let face = Face::ui(12.5).medium();
            let lw =
                self.text
                    .draw_in_row(&mut self.canvas, label, face, x, y, 26.0, p.bright, inside);
            let f = Face::ui(11.5);
            // At the right end, shortened with … when it doesn't fit.
            let room = (row.right() - 8.0 - (x + lw + 12.0)).max(0.0);
            let detail = self.text.fit(detail, f, room);
            let dw = self.text.width(&detail, f);
            let dx = row.right() - 8.0 - dw;
            let detail_clip =
                Rect::new(dx, y, (row.right() - 6.0 - dx).max(0.0), 26.0).intersect(inside);
            self.text.draw_in_row(
                &mut self.canvas,
                &detail,
                f,
                dx,
                y,
                26.0,
                p.muted,
                detail_clip,
            );
            if let Some(t) = target {
                self.hit(row, chat_pick(side, t + n));
            }
            y += 26.0;
        }
    }
}
