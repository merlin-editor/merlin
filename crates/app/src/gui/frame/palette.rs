//! The palette, with its scopes, sections and matched letters.

use super::*;

impl Frame<'_> {
    pub(super) fn palette(&mut self, pv: &PaletteView, w: f32, h: f32) {
        let p = self.p;
        self.canvas
            .tint(Rect::new(0.0, 0.0, w, h), 0.0, p.ground, 140);
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::ClosePalette));
        let pw = 640.0_f32.min(w - 32.0);
        let shown = pv.items.len().clamp(1, PALETTE_ROWS);
        // The items scrolled to keep the selection in view, and the group
        // headings over them, as Run Task has.
        let first = pv.selected.saturating_sub(PALETTE_ROWS - 1);
        let heading = |k: usize| pv.extras.get(k).and_then(|e| e.heading.as_deref());
        let more = |k: usize| pv.extras.get(k).and_then(|e| e.more.as_ref());
        let visible = first..pv.items.len().min(first + PALETTE_ROWS);
        let headings = visible.clone().filter(|&k| heading(k).is_some()).count();
        let mores = visible.filter(|&k| more(k).is_some()).count();
        let list_h = shown as f32 * 34.0 + headings as f32 * 26.0 + mores as f32 * 24.0 + 12.0;
        // The search's scope chips, under the query.
        let chips_h = if pv.scopes.is_empty() { 0.0 } else { 38.0 };
        let ph = 48.0 + chips_h + list_h + 34.0;
        let r = Rect::new((w - pw) / 2.0, 56.0, pw, ph);
        self.canvas.shadow(r, 12.0, theme::is_light(&p));
        self.canvas.outline(r, 12.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);

        self.palette_query(pv, r);
        self.palette_scopes(pv, r);
        self.canvas
            .fill(Rect::new(r.x, r.y + 48.0 + chips_h, r.w, 1.0), p.chip);

        let list = Rect::new(r.x, r.y + 49.0 + chips_h, r.w, list_h);
        if pv.items.is_empty() {
            self.text.draw_in_row(
                &mut self.canvas,
                "Nothing matches",
                Face::ui(13.0),
                r.x + 20.0,
                list.y + 6.0,
                34.0,
                p.muted,
                list,
            );
        }
        let mut y = list.y + 6.0;
        for (k, (label, key)) in pv.items.iter().enumerate().skip(first).take(PALETTE_ROWS) {
            if let Some(heading) = heading(k) {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &heading.to_uppercase(),
                    Face::ui(11.0).medium(),
                    r.x + 18.0,
                    y,
                    26.0,
                    p.muted,
                    list,
                );
                y += 26.0;
            }
            let row = Rect::new(r.x + 6.0, y, r.w - 12.0, 34.0);
            y += 34.0;
            let on = k == pv.selected;
            if on {
                self.canvas.round(row, 7.0, p.selected);
            } else if self.hovered(row) {
                self.canvas.round(row, 7.0, p.hover);
            }
            let chevron = self.palette_item(pv, k, label, key, row);
            self.hit(row, Hit::Ui(UiEvent::PaletteRow(k)));
            if let Some(b) = chevron {
                self.hit(b, Hit::Ui(UiEvent::PaletteChoices(k)));
            }
            if let Some((text, scope)) = more(k) {
                let face = Face::ui(12.0);
                let w = self.text.width(text, face);
                let link = Rect::new(r.x + 22.0, y, w + 8.0, 24.0);
                let ink = if self.hovered(link) {
                    p.bright
                } else {
                    p.accent
                };
                self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    face,
                    link.x + 4.0,
                    y,
                    24.0,
                    ink,
                    list,
                );
                self.marked_hit(link, Hit::Ui(UiEvent::PaletteScope(*scope)), Mark::Link);
                y += 24.0;
            }
        }
        self.palette_footer(pv, r);
    }

    /// The palette's query line: its title, sign, the query or its
    /// placeholder with the caret, and the Esc chip.
    fn palette_query(&mut self, pv: &PaletteView, r: Rect) {
        let p = self.p;
        let input = Rect::new(r.x, r.y, r.w, 48.0);
        self.input_box(input);
        self.text.draw_in_row(
            &mut self.canvas,
            "\u{ea6d}",
            Face::mono(15.0),
            r.x + 16.0,
            r.y,
            48.0,
            p.muted,
            input,
        );
        let mut qx = r.x + 42.0;
        if let Some(title) = &pv.title {
            let face = Face::ui(14.0).medium();
            let title = match title.ends_with('›') {
                true => title.clone(),
                false => format!("{title}:"),
            };
            qx += self.text.draw_in_row(
                &mut self.canvas,
                &title,
                face,
                qx,
                r.y,
                48.0,
                p.muted,
                input,
            ) + 8.0;
        }
        let face = Face::ui(15.0);
        if let Some(sign) = pv.sign {
            qx += self.text.draw_in_row(
                &mut self.canvas,
                &sign.to_string(),
                Face::mono(15.0),
                qx,
                r.y,
                48.0,
                p.muted,
                input,
            ) + 2.0;
        }
        let qw = self.text.draw_in_row(
            &mut self.canvas,
            &pv.query,
            face,
            qx,
            r.y,
            48.0,
            p.bright,
            input,
        );
        if let Some(placeholder) = pv.placeholder.as_deref().filter(|_| pv.query.is_empty()) {
            self.text.draw_in_row(
                &mut self.canvas,
                placeholder,
                face,
                qx + 6.0,
                r.y,
                48.0,
                p.faint,
                input,
            );
        }
        self.canvas
            .fill(Rect::new(qx + qw + 1.0, r.y + 14.0, 1.5, 20.0), p.accent);
        self.chip("Esc", r.right() - 14.0, r.y + 16.0, true);
        if pv.scopes.is_empty() {
            let kind_w = self.text.width(pv.kind, Face::ui(11.0)) + 12.0;
            self.chip(pv.kind, r.right() - 14.0 - 40.0 - kind_w, r.y + 16.0, false);
        }
    }

    /// The search's scope chips, under the query.
    fn palette_scopes(&mut self, pv: &PaletteView, r: Rect) {
        let p = self.p;
        let mut cx = r.x + 14.0;
        for (i, (label, key, on)) in pv.scopes.iter().enumerate() {
            let face = Face::ui(12.5).medium();
            let small = Face::ui(12.0);
            let key_w = key
                .as_ref()
                .map_or(0.0, |k| self.text.width(k, small) + 7.0);
            let chip = Rect::new(
                cx,
                r.y + 50.0,
                self.text.width(label, face) + key_w + 22.0,
                26.0,
            );
            let (fill, ink, dim) = match on {
                true => (p.accent, p.on_accent, mix(p.on_accent, p.accent, 0.7)),
                false if self.hovered(chip) => (p.hover, p.text, p.muted),
                false => (p.chip, p.text, p.muted),
            };
            self.canvas.round(chip, 13.0, fill);
            let x = chip.x + 11.0;
            let x = x + self.text.draw_in_row(
                &mut self.canvas,
                label,
                face,
                x,
                chip.y,
                chip.h,
                ink,
                chip,
            );
            if let Some(key) = key {
                self.text.draw_in_row(
                    &mut self.canvas,
                    key,
                    small,
                    x + 7.0,
                    chip.y,
                    chip.h,
                    dim,
                    chip,
                );
            }
            self.hit(chip, Hit::Ui(UiEvent::PaletteScope(i)));
            cx = chip.right() + 6.0;
        }
    }

    /// A palette row's contents: its key, plugin chip and detail at the
    /// right, what it is, then its label with the query's letters marked.
    /// Gives the chevron's box when a click on it opens more.
    fn palette_item(
        &mut self,
        pv: &PaletteView,
        k: usize,
        label: &str,
        key: &Option<String>,
        row: Rect,
    ) -> Option<Rect> {
        let p = self.p;
        let extra = pv.extras.get(k);
        // At the right: the key, then the plugin's chip, then where
        // it's from.
        let mut right = row.right() - 10.0;
        if let Some(key) = key {
            right -= self.chip(key, right, row.y + 9.0, true) + 8.0;
        }
        if let Some(chip) = extra.and_then(|e| e.chip.as_deref()) {
            right -= self.plugin_chip(chip, right, row.y + 9.0) + 8.0;
        }
        let at_right = pv.scopes.is_empty() || extra.is_some_and(|e| e.code);
        if let Some(detail) = extra
            .map(|e| e.detail.as_str())
            .filter(|d| !d.is_empty() && at_right)
        {
            let face = Face::ui(12.0);
            let w = self.text.width(detail, face);
            self.text.draw_in_row(
                &mut self.canvas,
                detail,
                face,
                right - w,
                row.y,
                row.h,
                p.muted,
                row,
            );
            right -= w + 12.0;
        }
        // What it is, before its label.
        let mut lx = row.x + 12.0;
        let mid = row.y + row.h / 2.0;
        let chevron = self.palette_icon(extra, &mut lx, mid, row);
        let max = (right - lx).min(row.w - 140.0);
        let code = extra.is_some_and(|e| e.code);
        let end = if extra.is_some_and(|e| e.muted) {
            lx + self.text.draw_in_row(
                &mut self.canvas,
                label,
                Face::ui(13.5),
                lx,
                row.y,
                row.h,
                p.muted,
                row,
            )
        } else if code {
            let hit = extra.and_then(|e| e.hit);
            self.code_match(label, hit, lx, row, max)
        } else {
            self.matched(label, &pv.query, lx, row, max)
        };
        // The search's detail follows its label, as what it's in.
        if let Some(detail) = extra
            .map(|e| e.detail.as_str())
            .filter(|d| !d.is_empty() && !pv.scopes.is_empty() && !code)
        {
            let face = Face::ui(12.5);
            let room = right - end - 12.0;
            if room > 40.0 {
                let detail = self.text.fit(detail, face, room);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &detail,
                    face,
                    end + 10.0,
                    row.y,
                    row.h,
                    p.muted,
                    row,
                );
            }
        }
        chevron
    }

    /// What a palette row is, before its label: a command's chevron box, a
    /// symbol's letter or a file's icon. Moves `lx` past it, and gives the
    /// chevron's box when a click on it opens more.
    fn palette_icon(
        &mut self,
        extra: Option<&merlin_core::PaletteExtra>,
        lx: &mut f32,
        mid: f32,
        row: Rect,
    ) -> Option<Rect> {
        let p = self.p;
        let mut chevron = None;
        match extra.map(|e| &e.icon) {
            Some(PaletteIcon::Command) => {
                // Brighter where it offers more, which a click opens.
                let b = Rect::new(*lx, mid - 9.0, 18.0, 18.0);
                let more = extra.is_some_and(|e| e.choices);
                let (fill, ink) = match more {
                    true if self.hovered(b) => (p.accent, p.on_accent),
                    true => (mix(p.accent, p.widget, 0.2), p.accent),
                    false => (p.chip, p.muted),
                };
                self.canvas.round(b, 4.0, fill);
                self.text.draw_in_row(
                    &mut self.canvas,
                    "›",
                    Face::ui(12.0),
                    b.x + 6.0,
                    b.y,
                    b.h,
                    ink,
                    b,
                );
                if more {
                    chevron = Some(b);
                }
                *lx += 28.0;
            }
            Some(PaletteIcon::Symbol(letter, ty)) => {
                let ink = if *ty { p.type_ } else { p.function };
                let b = Rect::new(*lx, mid - 9.0, 18.0, 18.0);
                self.canvas.round(b, 4.0, mix(ink, p.widget, 0.16));
                let face = Face::ui(11.0).bold();
                let s = letter.to_string();
                let w = self.text.width(&s, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &s,
                    face,
                    b.x + (18.0 - w) / 2.0,
                    b.y,
                    b.h,
                    ink,
                    b,
                );
                *lx += 28.0;
            }
            Some(PaletteIcon::File(name)) => {
                let core = merlin_core::file_icon(name, true);
                self.file_icon(name, Some(core), *lx + 1.0, mid - 8.0, 16.0, row);
                *lx += 28.0;
            }
            _ => {}
        }
        chevron
    }

    /// How to use it, along the palette's foot.
    fn palette_footer(&mut self, pv: &PaletteView, r: Rect) {
        let p = self.p;
        let foot = Rect::new(r.x, r.bottom() - 34.0, r.w, 34.0);
        self.canvas
            .fill(Rect::new(r.x + 1.0, foot.y, r.w - 2.0, 1.0), p.chip);
        let mut fx = r.x + 16.0;
        let usual = [("↑ ↓", "to move"), ("Enter", "to run"), ("Esc", "to close")];
        let hints: Vec<(&str, &str)> = match pv.hints.is_empty() {
            true => usual.to_vec(),
            false => pv
                .hints
                .iter()
                .map(|(k, w)| (k.as_str(), w.as_str()))
                .collect(),
        };
        for (keys, what) in hints {
            let f = Face::ui(11.5);
            fx += self.text.draw_in_row(
                &mut self.canvas,
                keys,
                f,
                fx,
                foot.y,
                34.0,
                p.chip_text,
                foot,
            ) + 5.0;
            fx += self
                .text
                .draw_in_row(&mut self.canvas, what, f, fx, foot.y, 34.0, p.muted, foot)
                + 18.0;
        }
    }

    /// A label with the letters of `query` it matches in the accent.
    pub(super) fn matched(&mut self, label: &str, query: &str, x: f32, row: Rect, max: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(13.5);
        let strong = Face::ui(13.5).bold();
        let label = self.text.fit(label, face, max);
        // The query as it is, where the label has it, else its letters in
        // order.
        let lower: Vec<char> = label.chars().flat_map(char::to_lowercase).collect();
        let whole: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
        let from = (!whole.is_empty() && lower.len() == label.chars().count())
            .then(|| {
                lower
                    .windows(whole.len())
                    .position(|w| w == whole.as_slice())
            })
            .flatten();
        let skip = from.unwrap_or(0);
        let mut wanted = query
            .trim()
            .chars()
            .filter(|c| !c.is_whitespace() || from.is_some())
            .flat_map(char::to_lowercase)
            .peekable();
        let mut x = x;
        let mut run = String::new();
        let mut run_match = false;
        let flush = |frame: &mut Self, run: &mut String, matched: bool, x: &mut f32| {
            if run.is_empty() {
                return;
            }
            let (f, c) = if matched {
                (strong, p.accent)
            } else {
                (face, p.text)
            };
            *x += frame
                .text
                .draw_in_row(&mut frame.canvas, run, f, *x, row.y, row.h, c, row);
            run.clear();
        };
        for (i, c) in label.chars().enumerate() {
            let hit = i >= skip
                && wanted
                    .peek()
                    .is_some_and(|w| c.to_lowercase().next() == Some(*w));
            if hit {
                wanted.next();
            }
            if hit != run_match {
                flush(self, &mut run, run_match, &mut x);
                run_match = hit;
            }
            run.push(c);
        }
        flush(self, &mut run, run_match, &mut x);
        x
    }

    /// A line of code, with the characters from `hit.0` to `hit.1` tinted
    /// as a match. Returns where it ends.
    pub(super) fn code_match(
        &mut self,
        label: &str,
        hit: Option<(usize, usize)>,
        x: f32,
        row: Rect,
        max: f32,
    ) -> f32 {
        let p = self.p;
        let face = Face::mono(12.5);
        let label = self.text.fit(label, face, max);
        let chars: Vec<char> = label.chars().collect();
        let (a, b) = hit.map_or((chars.len(), chars.len()), |(a, b)| {
            (a.min(chars.len()), b.min(chars.len()))
        });
        let parts = [
            (chars[..a].iter().collect::<String>(), false),
            (chars[a..b].iter().collect::<String>(), true),
            (chars[b..].iter().collect::<String>(), false),
        ];
        let mut x = x;
        for (text, tint) in parts {
            if text.is_empty() {
                continue;
            }
            let w = self.text.width(&text, face);
            if tint {
                let bg = Rect::new(x - 1.0, row.y + 8.0, w + 2.0, row.h - 16.0);
                self.canvas.round(bg, 2.0, mix(p.warning, p.widget, 0.2));
            }
            let ink = if tint { p.warning } else { p.text };
            x += self
                .text
                .draw_in_row(&mut self.canvas, &text, face, x, row.y, row.h, ink, row);
        }
        x
    }
}
