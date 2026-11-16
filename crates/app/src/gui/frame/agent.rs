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
        for (n, b) in a.buttons.iter().chain(&a.quiet).enumerate() {
            let bw = self.small_width(&format!("{} {}", n + 1, b.label));
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
            // Each answer with its key, 1 to 9. Those that don't fit go
            // on the next line.
            let label = format!("{} {}", k + 1, b.label);
            let need = self.small_width(&label);
            if bx > x && bx + need > x + w {
                bx = x;
                y += SMALL_H + 6.0;
            }
            let bw = self.small_button(&label, bx, y, look, focused);
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

/// A mode's colour: one of merlin's hues by name, `#rrggbb` softened to
/// sit with the theme, or the accent. The hues are brighter than file
/// icons', as badges: build blue, plan purple and so on.
pub(super) fn mode_colour(p: &Palette, colour: &str) -> u32 {
    let light = theme::is_light(p);
    let named = |dark: u32, lightish: u32| if light { lightish } else { dark };
    match colour.trim().to_ascii_lowercase().as_str() {
        "blue" => named(0x5c9cf5, 0x2f6fd0),
        "purple" => named(0x9d7cd8, 0x7650b8),
        "green" => named(0x7fd88f, 0x2f8a4a),
        "orange" => named(0xf5a742, 0xb86e10),
        "red" => named(0xf0807a, 0xc0392b),
        "yellow" => named(0xe6c35c, 0x9a7a10),
        "cyan" => named(0x5cc8d8, 0x16808f),
        "grey" | "gray" => named(0x8e9099, 0x6a6d76),
        hex if hex.starts_with('#') && hex.len() == 7 => {
            match u32::from_str_radix(&hex[1..], 16) {
                // Softened toward the text, so a loud colour still reads.
                Ok(c) if light => mix(c, 0x000000, 0.8),
                Ok(c) => mix(c, p.bright, 0.85),
                Err(_) => p.accent,
            }
        }
        _ => p.accent,
    }
}

impl Frame<'_> {
    /// The row under an agent's text: the mode's badge, merlin's edit pill
    /// and the plugin's chips at the left; how full the context is, and
    /// Send or Stop, at the right.
    pub(super) fn compose_bar(
        &mut self,
        c: &merlin_core::ComposeBar,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let colour = c
            .mode
            .as_ref()
            .map_or(p.accent, |(_, colour)| mode_colour(&p, colour));
        let on_colour = if theme::is_light(&p) {
            0xffffff
        } else {
            0x0f1012
        };
        let mid = r.y + r.h / 2.0 - 3.0;
        let mut k = 0usize;
        let mut next = || {
            let t = target.map(|t| t + k);
            k += 1;
            t
        };
        let focused_at = |t: Option<usize>| sv.focused && t.is_some() && t == Some(sv.selected);
        // At the right first, so the pills know where to stop.
        let send = Rect::new(r.right() - 18.0 - 26.0, mid - 13.0, 26.0, 26.0);
        let mut right = send.x - 6.0;
        let ring = c.ring.as_ref().map(|_| {
            let rr = Rect::new(right - 22.0, mid - 11.0, 22.0, 22.0);
            right = rr.x - 4.0;
            rr
        });
        let mut x = r.x + 18.0;
        if let Some((name, _)) = &c.mode {
            let t = next();
            let label = name.to_uppercase();
            let face = Face::ui(10.5).bold();
            let w = self.text.width(&label, face) + 16.0;
            let b = Rect::new(x, mid - 10.0, w, 20.0);
            let fill = if self.hovered(b) {
                mix(colour, p.bright, 0.85)
            } else {
                colour
            };
            if focused_at(t) {
                self.canvas
                    .outline(b.inset(-2.0), 6.0, 1.5, p.accent, p.raised);
            }
            self.canvas.round(b, 4.0, fill);
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                face,
                x + 8.0,
                b.y,
                20.0,
                on_colour,
                clip,
            );
            if let Some(t) = t {
                self.hit(b, chat_pick(side, t));
            }
            x += w + 4.0;
        }
        // Short of room, merlin's edit pill shows just its icon (its
        // name stays in its menu).
        let face = Face::ui(11.5);
        let wanted: f32 = c
            .chips
            .iter()
            .filter(|c| !c.id.is_empty())
            .map(|c| self.text.width(&c.label, face) + 16.0)
            .sum();
        let squeeze = x + wanted > right - 4.0;
        for chip in &c.chips {
            if chip.id.is_empty() {
                continue;
            }
            let t = next();
            let short;
            let chip = if squeeze && chip.id == "agent:edit" {
                short = merlin_core::Chip {
                    label: chip.label.chars().take(1).collect::<String>() + " ▾",
                    ..chip.clone()
                };
                &short
            } else {
                chip
            };
            let tw = self.text.width(&chip.label, face);
            let room = right - x - 4.0;
            if room < 40.0 {
                continue;
            }
            let label = self.text.fit(&chip.label, face, (room - 14.0).max(0.0));
            let w = self.text.width(&label, face) + 14.0;
            let b = Rect::new(x, mid - 11.0, w.min(tw + 14.0), 22.0);
            if focused_at(t) {
                self.canvas
                    .outline(b.inset(-2.0), 7.0, 1.5, p.accent, p.raised);
            } else if self.hovered(b) {
                self.canvas.round(b, 6.0, p.hover);
            }
            let fg = if self.hovered(b) { p.bright } else { p.text };
            self.text
                .draw_in_row(&mut self.canvas, &label, face, x + 7.0, b.y, 22.0, fg, clip);
            if let Some(t) = t {
                self.hit(b, chat_pick(side, t));
            }
            x += b.w + 2.0;
        }
        if let (Some(rr), Some((full, _))) = (ring, &c.ring) {
            let t = next();
            if self.hovered(rr) || focused_at(t) {
                self.canvas.round(rr, 6.0, p.hover);
            }
            // A ring that fills clockwise from the top, amber when nearly
            // full.
            let (cx, cy, rad) = (rr.x + 11.0, rr.y + 11.0, 6.0);
            let fill = if *full >= 75 {
                mode_colour(&p, "orange")
            } else {
                colour
            };
            let steps = 48;
            let point = |n: usize| {
                let a =
                    n as f32 / steps as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                (cx + rad * a.cos(), cy + rad * a.sin())
            };
            for n in 0..steps {
                let done = (n as f32) < *full as f32 / 100.0 * steps as f32;
                let c2 = if done { fill } else { p.border };
                self.canvas.line(point(n), point(n + 1), 2.2, c2);
            }
            if let Some(t) = t {
                self.hit(rr, chat_pick(side, t));
            }
        }
        let t = next();
        let hover = self.hovered(send);
        if c.stop {
            self.canvas
                .round(send, 7.0, if hover { p.border } else { p.chip });
            self.canvas.round(
                Rect::new(send.x + 9.0, send.y + 9.0, 8.0, 8.0),
                1.5,
                p.bright,
            );
        } else {
            let fill = if hover {
                mix(colour, p.bright, 0.85)
            } else {
                colour
            };
            self.canvas.round(send, 7.0, fill);
            let (ax, ay) = (send.x + 13.0, send.y + 8.0);
            self.canvas.line((ax, ay), (ax, ay + 10.0), 1.8, on_colour);
            self.canvas
                .line((ax - 4.5, ay + 4.5), (ax, ay), 1.8, on_colour);
            self.canvas
                .line((ax + 4.5, ay + 4.5), (ax, ay), 1.8, on_colour);
        }
        if focused_at(t) {
            self.canvas
                .outline(send.inset(-2.0), 8.0, 1.5, p.accent, p.raised);
        }
        if let Some(t) = t {
            self.hit(send, chat_pick(side, t));
        }
    }
}

impl Frame<'_> {
    /// Who a reply is from: the agent's mark, its mode's badge, its name.
    pub(super) fn reply_head(
        &mut self,
        author: &str,
        mode: Option<&(String, String)>,
        r: Rect,
        clip: Rect,
    ) {
        let p = self.p;
        let colour = mode.map_or(p.accent, |(_, c)| mode_colour(&p, c));
        let (mark, name) = match author.split_once(' ') {
            Some((g, rest)) if !g.chars().any(char::is_alphanumeric) => (Some(g), rest),
            _ => (None, author),
        };
        let mut x = r.x + 14.0;
        let y = r.y + 6.0;
        if let Some((label, _)) = mode {
            let label = label.to_uppercase();
            let face = Face::ui(9.5).bold();
            let w = self.text.width(&label, face) + 12.0;
            self.canvas
                .round(Rect::new(x, y + 1.0, w, 16.0), 3.0, colour);
            let on = if theme::is_light(&p) {
                0xffffff
            } else {
                0x0f1012
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                face,
                x + 6.0,
                y + 1.0,
                16.0,
                on,
                clip,
            );
            x += w + 6.0;
        } else if let Some(g) = mark {
            x += self.text.draw_in_row(
                &mut self.canvas,
                g,
                Face::ui(11.0),
                x,
                y,
                18.0,
                colour,
                clip,
            ) + 5.0;
        }
        self.text.draw_in_row(
            &mut self.canvas,
            name,
            Face::ui(11.0).bold(),
            x,
            y,
            18.0,
            p.muted,
            clip,
        );
    }

    /// A run of steps folded to one line: ▸ and what they did, or a
    /// spinner and what the running one is doing; the time at the right.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fold_row(
        &mut self,
        label: &str,
        detail: &str,
        open: bool,
        running: bool,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let row = Rect::new(r.x + 8.0, r.y + 1.0, r.w - 16.0, r.h - 2.0);
        let focused = sv.focused && target.is_some() && target == Some(sv.selected);
        if focused {
            self.canvas.round(row, 7.0, p.focus_row);
        } else if self.hovered(row) {
            self.canvas.round(row, 7.0, p.hover);
        }
        let mut x = row.x + 8.0;
        if running {
            self.step_mark(StepState::Running, x - 1.0, row.y, row.h, clip);
        } else {
            self.canvas
                .chevron(x + 4.0, row.y + row.h / 2.0, 7.0, open, p.muted);
        }
        x += 16.0;
        let (did, time) = detail.split_once('|').unwrap_or((detail, ""));
        let face = Face::ui(12.0);
        let tw = if time.is_empty() {
            0.0
        } else {
            self.text.width(time, Face::ui(11.0))
        };
        let room = row.right() - 10.0 - tw - x;
        let label = self.text.fit(label, face, room);
        x += self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            row.y,
            row.h,
            p.text,
            clip,
        );
        if !did.is_empty() {
            let rest = self
                .text
                .fit(&format!(" · {did}"), face, row.right() - 14.0 - tw - x);
            self.text.draw_in_row(
                &mut self.canvas,
                &rest,
                face,
                x,
                row.y,
                row.h,
                p.muted,
                clip,
            );
        }
        if !time.is_empty() {
            self.text.draw_in_row(
                &mut self.canvas,
                time,
                Face::ui(11.0),
                row.right() - 8.0 - tw,
                row.y,
                row.h,
                p.faint,
                clip,
            );
        }
        if let Some(t) = target {
            self.hit(row, chat_pick(side, t));
        }
    }

    /// Under a finished reply: model, time, tokens and cost, and Copy.
    pub(super) fn turn_foot(
        &mut self,
        text: &str,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let face = Face::ui(11.0);
        self.text.draw_in_row(
            &mut self.canvas,
            text,
            face,
            r.x + 14.0,
            r.y,
            r.h,
            p.faint,
            clip,
        );
        let copy = Rect::new(r.right() - 14.0 - 22.0, r.y + 1.0, 22.0, 20.0);
        let focused = sv.focused && target.is_some() && target == Some(sv.selected);
        if focused || self.hovered(copy) {
            self.canvas.round(copy, 5.0, p.hover);
        }
        // Two overlapping squares: copy.
        let c = if self.hovered(copy) {
            p.bright
        } else {
            p.muted
        };
        self.canvas.outline(
            Rect::new(copy.x + 8.0, copy.y + 4.0, 8.0, 9.0),
            1.5,
            1.2,
            c,
            p.island,
        );
        self.canvas.outline(
            Rect::new(copy.x + 5.0, copy.y + 7.0, 8.0, 9.0),
            1.5,
            1.2,
            c,
            p.island,
        );
        if let Some(t) = target {
            self.hit(copy, chat_pick(side, t));
        }
    }
}

impl Frame<'_> {
    /// A card above the box: a rounded box at the box's width.
    fn dock_card(&mut self, r: Rect, dashed: bool) -> Rect {
        let p = self.p;
        let card = Rect::new(r.x + 10.0, r.y + 3.0, r.w - 20.0, r.h - 6.0);
        if dashed {
            // A message waiting: a quieter, dotted edge.
            self.canvas.round(card, 8.0, p.island);
            let mut x = card.x + 6.0;
            while x < card.right() - 6.0 {
                self.canvas.fill(Rect::new(x, card.y, 4.0, 1.0), p.border);
                self.canvas
                    .fill(Rect::new(x, card.bottom() - 1.0, 4.0, 1.0), p.border);
                x += 7.0;
            }
            let mut y = card.y + 6.0;
            while y < card.bottom() - 6.0 {
                self.canvas.fill(Rect::new(card.x, y, 1.0, 4.0), p.border);
                self.canvas
                    .fill(Rect::new(card.right() - 1.0, y, 1.0, 4.0), p.border);
                y += 7.0;
            }
        } else {
            self.canvas.outline(card, 8.0, 1.0, p.border, p.island);
        }
        card
    }

    /// The plan above the box: "☰ Plan 3 of 4" and the step it's on, or
    /// open, every step.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn plan_dock(
        &mut self,
        steps: &[(String, StepState)],
        open: bool,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = self.dock_card(r, false);
        let inside = card.inset(1.0).intersect(clip);
        let focused = sv.focused && target.is_some() && target == Some(sv.selected);
        if focused || self.hovered(card) {
            self.canvas.round(card.inset(1.0), 7.0, p.hover);
        }
        let done = steps.iter().filter(|s| s.1 == StepState::Done).count();
        let x = card.x + 10.0;
        let mut y = card.y + 4.0;
        let w = self.text.draw_in_row(
            &mut self.canvas,
            "☰ Plan",
            Face::ui(12.0).medium(),
            x,
            y,
            20.0,
            p.bright,
            inside,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &format!("{done} of {}", steps.len()),
            Face::ui(12.0),
            x + w + 6.0,
            y,
            20.0,
            p.muted,
            inside,
        );
        self.canvas
            .chevron(card.right() - 14.0, y + 10.0, 7.0, open, p.muted);
        y += 20.0;
        let shown: Vec<&(String, StepState)> = if open {
            steps.iter().collect()
        } else {
            steps
                .iter()
                .find(|s| s.1 != StepState::Done)
                .into_iter()
                .collect()
        };
        for (text, state) in shown {
            self.step_mark(*state, x - 1.0, y, 20.0, inside);
            let t = self.text.fit(text, Face::ui(12.0), card.w - 40.0);
            let colour = match state {
                StepState::Done => p.faint,
                StepState::Running => p.text,
                _ => p.muted,
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &t,
                Face::ui(12.0),
                x + 16.0,
                y,
                20.0,
                colour,
                inside,
            );
            y += 20.0;
        }
        if let Some(t) = target {
            self.hit(card, chat_pick(side, t));
        }
    }

    /// A message waiting its turn: "NEXT", its text, ✎, ✕ and Send Now.
    pub(super) fn queued_row(
        &mut self,
        text: &str,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = self.dock_card(r, true);
        let inside = card.inset(1.0).intersect(clip);
        let x = card.x + 10.0;
        let lw = self.text.draw_in_row(
            &mut self.canvas,
            "NEXT",
            Face::ui(10.5).bold(),
            x,
            card.y,
            card.h,
            p.faint,
            inside,
        );
        // At the right: ✎, ✕, Send Now.
        let now_w = self.text.width("Send Now", Face::ui(12.0)) + 18.0;
        let now_x = card.right() - 6.0 - now_w;
        let drop_x = now_x - 24.0;
        let edit_x = drop_x - 24.0;
        let room = edit_x - (x + lw + 8.0) - 4.0;
        let t = self.text.fit(text, Face::ui(12.0), room);
        self.text.draw_in_row(
            &mut self.canvas,
            &t,
            Face::ui(12.0),
            x + lw + 8.0,
            card.y,
            card.h,
            p.text,
            inside,
        );
        let by = card.y + (card.h - SMALL_H) / 2.0;
        let at = |k: usize| target.map(|t| t + k);
        let focus = |k: usize| sv.focused && at(k).is_some() && at(k) == Some(sv.selected);
        for (k, (glyph, gx)) in [("✎", edit_x), ("✕", drop_x)].into_iter().enumerate() {
            let b = Rect::new(gx, by, 22.0, SMALL_H);
            if focus(k) || self.hovered(b) {
                self.canvas.round(b, 5.0, p.hover);
            }
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                Face::ui(12.0),
                gx + 6.0,
                by,
                SMALL_H,
                p.muted,
                inside,
            );
            if let Some(t) = at(k) {
                self.hit(b, chat_pick(side, t));
            }
        }
        self.small_button("Send Now", now_x, by, Look::Plain, focus(2));
        if let Some(t) = at(2) {
            self.hit(Rect::new(now_x, by, now_w, SMALL_H), chat_pick(side, t));
        }
    }

    /// The files changed, waiting to be kept: "2 files changed +18 −7",
    /// Review, Undo All and Keep All; open, each file's Undo and Keep.
    pub(super) fn changes_bar(
        &mut self,
        c: &merlin_core::ChangesBar,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = self.dock_card(r, false);
        let inside = card.inset(1.0).intersect(clip);
        let at = |k: usize| target.map(|t| t + k);
        let focus = |k: usize| sv.focused && at(k).is_some() && at(k) == Some(sv.selected);
        let head = Rect::new(card.x, card.y, card.w, 34.0);
        let by = head.y + (head.h - SMALL_H) / 2.0;
        // At the right: Review, Undo All, Keep All.
        let keep_w = self.text.width("Keep All", Face::ui(12.0)) + 18.0;
        let undo_w = self.text.width("Undo All", Face::ui(12.0)) + 18.0;
        let review_w = self.text.width("Review", Face::ui(12.0)) + 18.0;
        let keep_x = head.right() - 6.0 - keep_w;
        let undo_x = keep_x - 4.0 - undo_w;
        let n = c.files.len();
        let (a, rm) = c.files.iter().fold((0, 0), |(a, r), f| (a + f.1, r + f.2));
        let said = format!("{n} {} changed", if n == 1 { "file" } else { "files" });
        let said_w = self.text.width(&said, Face::ui(12.0))
            + self.text.width(&format!("+{a} −{rm}"), Face::ui(12.0))
            + 26.0;
        // Short of room, Review steps aside: the head still opens the list.
        let show_review = head.x + said_w < undo_x - 2.0 - review_w;
        let review_x = if show_review {
            undo_x - 2.0 - review_w
        } else {
            undo_x
        };
        // The head opens the list.
        let toggle = Rect::new(head.x, head.y, review_x - head.x, head.h);
        if focus(0) || self.hovered(toggle) {
            self.canvas.round(toggle.inset(2.0), 6.0, p.hover);
        }
        let mut x = head.x + 10.0;
        x += self.text.draw_in_row(
            &mut self.canvas,
            &said,
            Face::ui(12.0),
            x,
            head.y,
            head.h,
            p.text,
            inside,
        ) + 7.0;
        x += self.text.draw_in_row(
            &mut self.canvas,
            &format!("+{a}"),
            Face::ui(12.0),
            x,
            head.y,
            head.h,
            p.success,
            inside,
        ) + 5.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &format!("−{rm}"),
            Face::ui(12.0),
            x,
            head.y,
            head.h,
            p.error,
            inside,
        );
        if let Some(t) = at(0) {
            self.hit(toggle, chat_pick(side, t));
        }
        for (k, (label, bx, w, look)) in [
            ("Review", review_x, review_w, Look::Quiet),
            ("Undo All", undo_x, undo_w, Look::Plain),
            ("Keep All", keep_x, keep_w, Look::Primary),
        ]
        .into_iter()
        .enumerate()
        {
            if k == 0 && !show_review {
                continue;
            }
            self.small_button(label, bx, by, look, focus(k + 1));
            if let Some(t) = at(k + 1) {
                self.hit(Rect::new(bx, by, w, SMALL_H), chat_pick(side, t));
            }
        }
        if !c.open {
            return;
        }
        let mut y = head.bottom();
        for (n, (path, added, removed)) in c.files.iter().enumerate() {
            self.canvas
                .fill(Rect::new(card.x + 1.0, y, card.w - 2.0, 1.0), p.line);
            let row = Rect::new(card.x, y, card.w, 26.0);
            let base = 4 + 3 * n;
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            let mut fx = row.x + 10.0;
            self.file_icon(&name, None, fx, y + 6.0, 14.0, inside);
            fx += 20.0;
            let keep_w = self.text.width("Keep", Face::ui(11.5)) + 14.0;
            let undo_w = self.text.width("Undo", Face::ui(11.5)) + 14.0;
            let keep_x = row.right() - 6.0 - keep_w;
            let undo_x = keep_x - 2.0 - undo_w;
            let name_r = Rect::new(fx - 22.0, y, undo_x - fx + 18.0, 26.0);
            if focus(base) || self.hovered(name_r) {
                self.canvas.round(name_r.inset(2.0), 5.0, p.hover);
            }
            fx += self.text.draw_in_row(
                &mut self.canvas,
                &name,
                Face::ui(12.0),
                fx,
                y,
                26.0,
                p.text,
                inside,
            ) + 6.0;
            let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
            let dir = dir.rsplit('/').next().unwrap_or(dir);
            if !dir.is_empty() {
                fx += self.text.draw_in_row(
                    &mut self.canvas,
                    dir,
                    Face::ui(11.0),
                    fx,
                    y,
                    26.0,
                    p.faint,
                    inside,
                ) + 6.0;
            }
            fx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("+{added}"),
                Face::ui(11.5),
                fx,
                y,
                26.0,
                p.success,
                inside,
            ) + 4.0;
            self.text.draw_in_row(
                &mut self.canvas,
                &format!("−{removed}"),
                Face::ui(11.5),
                fx,
                y,
                26.0,
                p.error,
                inside,
            );
            if let Some(t) = at(base) {
                self.hit(name_r, chat_pick(side, t));
            }
            for (k, (label, bx, w)) in [("Undo", undo_x, undo_w), ("Keep", keep_x, keep_w)]
                .into_iter()
                .enumerate()
            {
                let b = Rect::new(bx, y + 3.0, w, 20.0);
                let hot = self.hovered(b);
                if focus(base + 1 + k) || hot {
                    self.canvas.round(b, 4.0, p.hover);
                }
                self.text.draw_in_row(
                    &mut self.canvas,
                    label,
                    Face::ui(11.5),
                    bx + 7.0,
                    b.y,
                    20.0,
                    if hot { p.bright } else { p.muted },
                    inside,
                );
                if let Some(t) = at(base + 1 + k) {
                    self.hit(b, chat_pick(side, t));
                }
            }
            y += 26.0;
        }
    }
}

impl Frame<'_> {
    /// Over your message, while it's hovered or one of them has the keys:
    /// Edit, Copy and Rewind.
    pub(super) fn you_said_tools(
        &mut self,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let focus = |k: usize| sv.focused && target.is_some_and(|t| sv.selected == t + k);
        if !self.hovered(r) && !(0..3).any(focus) {
            return;
        }
        let face = Face::ui(11.0);
        let labels = ["✎ Edit", "⧉ Copy", "↶ Rewind"];
        let widths: Vec<f32> = labels
            .iter()
            .map(|l| self.text.width(l, face) + 12.0)
            .collect();
        let w: f32 = widths.iter().sum::<f32>() + 4.0;
        let bar = Rect::new(r.right() - 14.0 - w, r.y, w, 24.0);
        self.canvas.shadow(bar, 6.0, theme::is_light(&p));
        self.canvas.outline(bar, 6.0, 1.0, p.border, p.raised);
        let mut x = bar.x + 2.0;
        for (k, (label, bw)) in labels.iter().zip(&widths).enumerate() {
            let b = Rect::new(x, bar.y + 2.0, *bw, 20.0);
            let hot = self.hovered(b) || focus(k);
            if hot {
                self.canvas.round(b, 4.0, p.hover);
            }
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                face,
                x + 6.0,
                b.y,
                20.0,
                if hot { p.bright } else { p.text },
                clip,
            );
            if let Some(t) = target {
                self.hit(b, chat_pick(side, t + k));
            }
            x += bw;
        }
    }
}
