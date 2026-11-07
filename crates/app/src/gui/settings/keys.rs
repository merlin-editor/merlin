//! The Keyboard Shortcuts page: each command's keys as chips, and the card
//! that records new ones.

use super::*;

impl Frame<'_> {
    pub(super) fn keys_page(&mut self, v: &SettingsView, c: &mut Ctx) {
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

    pub(super) fn keys_header(&mut self, c: &mut Ctx) {
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

    pub(super) fn key_row(&mut self, row: &KeyRow, i: usize, picked: bool, c: &mut Ctx) {
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
    pub(super) fn key_chips(&mut self, shown: &str, mut x: f32, mid: f32, clip: Rect) -> f32 {
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
    pub(super) fn capture_card(&mut self, v: &SettingsView, c: &Ctx, body: Rect) {
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
}

/// Where the Keys and From columns start.
pub(super) fn key_columns(c: &Ctx) -> (f32, f32) {
    let from = c.x + c.w - 76.0 - 120.0;
    let keys = (from - 250.0).max(c.x + c.w * 0.38);
    (keys, from)
}
