//! What a chat view draws: messages, chips, and changes waiting for an
//! answer.

use super::markdown::MdOp;
use super::*;
use merlin_core::{Chip, EditCard, Message};

/// A message laid out: its Markdown, and how tall it is.
pub(super) struct MessageLayout {
    pub ops: Vec<(f32, MdOp)>,
    pub md_h: f32,
    pub height: f32,
}

const AUTHOR_H: f32 = 22.0;
const CHIPS_H: f32 = 28.0;
const PAD: f32 = 9.0;

impl Frame<'_> {
    /// A message at width `inner`: yours boxed, others' as they are.
    pub(super) fn message_layout(&mut self, m: &Message, inner: f32) -> MessageLayout {
        let w = if m.mine { inner - 2.0 * PAD } else { inner };
        let (ops, md_h) = self.md_layout(&m.markdown, w);
        let chips = if m.chips.is_empty() { 0.0 } else { CHIPS_H };
        let body = md_h + chips + if m.mine { 2.0 * PAD } else { 0.0 };
        MessageLayout {
            ops,
            md_h,
            height: AUTHOR_H + body + 12.0,
        }
    }

    /// A message: who from, its Markdown, its chips, and a caret while it
    /// streams in.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn message(
        &mut self,
        m: &Message,
        colours: &[Spans],
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let inner = r.w - 28.0;
        let layout = self.message_layout(m, inner);
        let x = r.x + 14.0;
        let mut y = r.y + 4.0;
        let who = Face::ui(11.0).bold();
        if m.mine {
            self.text
                .draw_in_row(&mut self.canvas, "You", who, x, y, 18.0, p.muted, clip);
        } else {
            let dot = Rect::new(x, y + 6.0, 7.0, 7.0);
            self.canvas.round(dot, 3.5, p.accent);
            self.text.draw_in_row(
                &mut self.canvas,
                &m.author,
                who,
                x + 12.0,
                y,
                18.0,
                p.muted,
                clip,
            );
        }
        y += AUTHOR_H - 4.0;
        let (body_x, body_w) = if m.mine {
            let chips = if m.chips.is_empty() { 0.0 } else { CHIPS_H };
            let b = Rect::new(x, y, inner, layout.md_h + chips + 2.0 * PAD);
            self.canvas.round(b, 8.0, p.raised);
            y += PAD;
            (x + PAD, inner - 2.0 * PAD)
        } else {
            (x, inner)
        };
        let colour = if m.mine { p.bright } else { p.text };
        let end = self.md_draw(&layout.ops, body_x, y, body_w, colours, colour, clip);
        if m.streaming {
            let (cx, cy) = if layout.ops.is_empty() {
                (body_x, y)
            } else {
                end
            };
            self.canvas
                .fill(Rect::new(cx + 2.0, cy + 3.0, 7.0, 13.0), p.accent);
        }
        y += layout.md_h;
        let mut cx = body_x;
        let mut k = 0;
        for c in &m.chips {
            let index = (!c.id.is_empty()).then(|| target.map(|t| t + k)).flatten();
            k += usize::from(!c.id.is_empty());
            let focused = sv.focused && index.is_some() && index == Some(sv.selected);
            let w = self.panel_chip(c, cx, y + 3.0, focused, clip);
            if let Some(index) = index {
                self.hit(Rect::new(cx, y + 3.0, w, 22.0), pick_target(side, index));
            }
            cx += w + 6.0;
        }
    }

    /// A chip: its label on a small tile, outlined while focused. Returns
    /// its width.
    pub(super) fn panel_chip(
        &mut self,
        c: &Chip,
        x: f32,
        y: f32,
        focused: bool,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let f = if c.label.chars().any(|ch| ch.is_ascii_digit()) && c.label.contains('.') {
            Face::mono(11.0)
        } else {
            Face::ui(11.5)
        };
        let tw = self.text.width(&c.label, f);
        let r = Rect::new(x, y, tw + 14.0, 22.0);
        let clickable = !c.id.is_empty();
        let bg = if clickable && self.hovered(r) {
            p.border
        } else {
            p.chip
        };
        if focused {
            self.canvas
                .outline(r.inset(-2.0), 7.0, 1.5, p.accent, p.island);
        }
        self.canvas.round(r, 5.0, bg);
        let fg = match c.tone {
            Tone::Normal => p.text,
            tone => tone_colour(&p, tone),
        };
        self.text
            .draw_in_row(&mut self.canvas, &c.label, f, x + 7.0, y, 22.0, fg, clip);
        r.w
    }

    /// A change waiting for an answer: the file, its lines added and
    /// removed, and its buttons at the right, the last the main one.
    pub(super) fn edit_card(
        &mut self,
        e: &EditCard,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        clip: Rect,
    ) {
        let p = self.p;
        let card = Rect::new(r.x + 14.0, r.y + 3.0, r.w - 28.0, 44.0);
        self.canvas.outline(card, 8.0, 1.0, p.border, p.island);
        let inside = card.inset(1.0).intersect(clip);
        let face = Face::ui(13.0).medium();
        let widths: Vec<f32> = e
            .buttons
            .iter()
            .map(|b| self.text.width(&b.label, face) + 28.0)
            .collect();
        let total = widths.iter().sum::<f32>() + 6.0 * widths.len().saturating_sub(1) as f32;
        let mut bx = card.right() - 8.0 - total;
        let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
        let mut x = card.x + 10.0;
        self.file_icon(&name, None, x, card.y + 15.0, 14.0, inside);
        x += 20.0;
        let text_clip =
            Rect::new(card.x, card.y, (bx - card.x - 6.0).max(0.0), card.h).intersect(clip);
        x += self.text.draw_in_row(
            &mut self.canvas,
            &name,
            Face::ui(12.5),
            x,
            card.y,
            card.h,
            p.bright,
            text_clip,
        );
        for (text, colour) in [
            (format!("+{}", e.added), p.success),
            (format!("−{}", e.removed), p.error),
        ] {
            x += 8.0;
            x += self.text.draw_in_row(
                &mut self.canvas,
                &text,
                Face::mono(11.5),
                x,
                card.y,
                card.h,
                colour,
                text_clip,
            );
        }
        let last = e.buttons.len().saturating_sub(1);
        for (k, (b, bw)) in e.buttons.iter().zip(&widths).enumerate() {
            let index = target.map(|t| t + k);
            let focused = sv.focused && index == Some(sv.selected);
            self.button(&b.label, bx, card.y + 7.0, focused, k == last);
            if let Some(index) = index {
                self.hit(
                    Rect::new(bx, card.y + 7.0, *bw, 30.0),
                    pick_target(side, index),
                );
            }
            bx += bw + 6.0;
        }
    }
}

fn pick_target(side: Side, index: usize) -> Hit {
    Hit::Ui(UiEvent::Target {
        side,
        index,
        clicks: 1,
    })
}
