//! The bars over the text: find and replace, Vim's command line, and
//! which-key.

use super::*;

impl Frame<'_> {
    pub(super) fn find(&mut self, find: &FindView, a: &Areas) {
        let p = self.p;
        let rows = 1.0 + find.replace.is_some() as u8 as f32;
        let bw = 440.0_f32.min(a.text.w - 24.0);
        if bw < 200.0 {
            return;
        }
        let r = Rect::new(
            a.text.right() - bw - 22.0,
            a.text.y + 4.0,
            bw,
            rows * 34.0 + 8.0,
        );
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let ui = Face::ui(13.0);
        // The arrow that shows and hides the replacement.
        let toggle = Rect::new(r.x + 4.0, r.y + 4.0, 20.0, rows * 34.0);
        if self.hovered(toggle) {
            self.canvas.round(toggle, 4.0, p.hover);
        }
        self.canvas.chevron(
            toggle.x + 10.0,
            r.y + 4.0 + 17.0,
            8.0,
            find.replace.is_some(),
            p.muted,
        );
        self.hit(toggle, Hit::Ui(UiEvent::Find(FindAction::ToggleReplace)));
        let field_x = toggle.right() + 4.0;
        let buttons_w = 124.0;
        let field_w = r.w - (field_x - r.x) - buttons_w - 8.0;
        let count = match (find.current, find.total) {
            (_, 0) if find.query.is_empty() => String::new(),
            (_, 0) => "No results".to_string(),
            (Some(i), n) => format!("{i} of {n}"),
            (None, n) => format!("{n} found"),
        };
        let fields: Vec<(&str, bool, &str)> = std::iter::once((find.query.as_str(), false, "Find"))
            .chain(find.replace.as_deref().map(|t| (t, true, "Replace")))
            .collect();
        for (k, (value, is_replace, hint)) in fields.into_iter().enumerate() {
            let y = r.y + 4.0 + k as f32 * 34.0;
            let field = Rect::new(field_x, y + 4.0, field_w, 26.0);
            let active = find.focused && find.in_replace == is_replace;
            let edge = if find.invalid && !is_replace {
                p.error
            } else if active {
                p.accent
            } else {
                p.border
            };
            self.canvas.outline(field, 5.0, 1.0, edge, p.raised);
            self.marked_hit(
                field,
                Hit::Ui(UiEvent::Find(FindAction::Field(is_replace))),
                Mark::Input,
            );
            // Match Case, Whole Word and Regex, at the query's right end.
            let toggles_w = if is_replace {
                0.0
            } else {
                self.find_toggles(find, field)
            };
            let info_w = if is_replace || count.is_empty() {
                0.0
            } else {
                self.text.width(&count, Face::ui(12.0)) + 12.0
            };
            let room = field.w - 16.0 - info_w - toggles_w;
            let shown = self.text.fit(value, ui, room);
            let (text, colour) = if value.is_empty() {
                (hint.to_string(), p.muted)
            } else {
                (shown, p.bright)
            };
            let tw = self.text.draw_in_row(
                &mut self.canvas,
                &text,
                ui,
                field.x + 8.0,
                field.y,
                field.h,
                colour,
                field,
            );
            if active {
                let cx = field.x + 8.0 + if value.is_empty() { 0.0 } else { tw } + 1.0;
                self.canvas
                    .fill(Rect::new(cx, field.y + 5.0, 1.5, field.h - 10.0), p.accent);
            }
            if info_w > 0.0 {
                let colour = if find.total == 0 { p.error } else { p.muted };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &count,
                    Face::ui(12.0),
                    field.right() - info_w - toggles_w,
                    field.y,
                    field.h,
                    colour,
                    field,
                );
            }
            // Buttons right of the field.
            let mut bx = field.right() + 6.0;
            let buttons: &[(&str, FindAction)] = if is_replace {
                &[
                    ("Replace", FindAction::ReplaceOne),
                    ("All", FindAction::ReplaceAll),
                ]
            } else {
                &[
                    ("↑", FindAction::Previous),
                    ("↓", FindAction::Next),
                    ("", FindAction::InSelection),
                    ("✕", FindAction::Close),
                ]
            };
            for &(label, action) in buttons {
                let lw = self.text.width(label, ui) + 14.0;
                let b = Rect::new(bx, field.y, lw.max(26.0), field.h);
                bx = b.right() + 2.0;
                self.hit(b, Hit::Ui(UiEvent::Find(action)));
                if action == FindAction::InSelection {
                    self.in_selection_button(b, find.in_selection);
                    continue;
                }
                if self.hovered(b) {
                    self.canvas.round(b, 5.0, p.hover);
                }
                let lx = b.x + (b.w - self.text.width(label, ui)) / 2.0;
                self.text
                    .draw_in_row(&mut self.canvas, label, ui, lx, b.y, b.h, p.text, b);
            }
        }
    }

    /// The query's toggles, drawn at the right of `field`; returns the
    /// width they take.
    pub(super) fn find_toggles(&mut self, find: &FindView, field: Rect) -> f32 {
        let p = self.p;
        let toggles = [
            (
                "Aa",
                find.match_case,
                FindAction::MatchCase,
                "Match Case (Alt+C)",
            ),
            (
                "ab",
                find.whole_word,
                FindAction::WholeWord,
                "Match Whole Word (Alt+W)",
            ),
            (
                ".*",
                find.regex,
                FindAction::Regex,
                "Use Regular Expression (Alt+R)",
            ),
        ];
        let size = field.h - 6.0;
        let mut x = field.right() - 3.0 - toggles.len() as f32 * (size + 2.0);
        let start = x;
        for (label, on, action, tip) in toggles {
            let b = Rect::new(x, field.y + 3.0, size, size);
            x += size + 2.0;
            if on {
                self.canvas.outline(b, 4.0, 1.0, p.accent, p.chip);
            } else if self.hovered(b) {
                self.canvas.round(b, 4.0, p.hover);
            }
            let face = Face::ui(12.0);
            let lw = self.text.width(label, face);
            let colour = if on { p.bright } else { p.muted };
            let lx = b.x + (b.w - lw) / 2.0;
            self.text
                .draw_in_row(&mut self.canvas, label, face, lx, b.y, b.h, colour, b);
            // Whole Word's letters sit on a line, as VS Code draws it.
            if action == FindAction::WholeWord {
                let y = (b.y + b.h * 0.78).round();
                self.canvas
                    .fill(Rect::new(lx - 1.0, y, lw + 2.0, 1.0), colour);
            }
            self.hit(b, Hit::Ui(UiEvent::Find(action)));
            self.tips.push((b, tip.to_string()));
        }
        field.right() - start
    }

    /// Find in Selection's button: lines in a box, lit when on.
    pub(super) fn in_selection_button(&mut self, b: Rect, on: bool) {
        let p = self.p;
        if on {
            self.canvas.outline(b, 5.0, 1.0, p.accent, p.chip);
        } else if self.hovered(b) {
            self.canvas.round(b, 5.0, p.hover);
        }
        let colour = if on { p.bright } else { p.text };
        let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
        for (k, w) in [(-1.0, 10.0), (0.0, 7.0), (1.0, 10.0)] {
            let y = (cy + k * 3.5).round();
            self.canvas
                .fill(Rect::new((cx - 5.0).round(), y, w, 1.5), colour);
        }
        self.tips.push((b, "Find in Selection (Alt+L)".to_string()));
    }

    /// Vim's command line along the bottom of the text, with what it
    /// suggests in a card over it.
    pub(super) fn vim_line(&mut self, line: &merlin_core::VimLineView, a: &Areas) {
        let p = self.p;
        let h = 34.0;
        // The group's whole width, over the minimap and scrollbar too.
        let right = a
            .groups
            .iter()
            .find(|g| g.text == a.text)
            .map_or(a.text.right(), |g| g.bar.right());
        let r = Rect::new(a.text.x, a.text.bottom() - h, right - a.text.x, h);
        // Square at the top, and round below as the island's corners are.
        let fill = theme::mix(p.ground, p.island, 0.5);
        self.canvas.round(r, 10.0, fill);
        self.canvas.fill(Rect::new(r.x, r.y, r.w, h / 2.0), fill);
        self.canvas.fill(Rect::new(r.x, r.y, r.w, 1.0), p.border);
        self.hit(r, Hit::Block);
        self.input_box(r);
        let mono = Face::mono(13.0);
        let ui = Face::ui(11.5);
        let x = r.x + 12.0;
        let pw = self.text.width(&line.prefix, mono);
        self.text
            .draw_in_row(&mut self.canvas, &line.prefix, mono, x, r.y, h, p.muted, r);
        let tx = x + pw + 2.0;
        // The hint first, so the text knows where it must stop.
        let hw = self.text.width(&line.hint, ui);
        let hx = (r.right() - 12.0 - hw).max(tx + 120.0);
        let hint_colour = if line.error { p.error } else { p.muted };
        self.text
            .draw_in_row(&mut self.canvas, &line.hint, ui, hx, r.y, h, hint_colour, r);
        let clip = Rect::new(tx, r.y, (hx - 16.0 - tx).max(0.0), h);
        self.text.draw_in_row(
            &mut self.canvas,
            &line.text,
            mono,
            tx,
            r.y,
            h,
            p.bright,
            clip,
        );
        let before: String = line.text.chars().take(line.cursor).collect();
        let cx = tx + self.text.width(&before, mono);
        if cx < clip.right() {
            self.canvas
                .fill(Rect::new(cx, r.y + 9.0, 1.5, h - 18.0), p.text);
        }
        if line.items.is_empty() {
            return;
        }
        // The suggestions, over the line at its left.
        let row = 26.0;
        let w = 440.0_f32.min(r.w - 24.0);
        let ch = line.items.len() as f32 * row + 10.0;
        let card = Rect::new(r.x + 12.0, r.y - ch - 6.0, w, ch);
        self.canvas.shadow(card, 8.0, theme::is_light(&p));
        self.canvas.outline(card, 8.0, 1.0, p.border, p.widget);
        let body = Face::ui(12.5);
        for (i, item) in line.items.iter().enumerate() {
            let rr = Rect::new(
                card.x + 5.0,
                card.y + 5.0 + i as f32 * row,
                card.w - 10.0,
                row,
            );
            if i == line.selected {
                self.canvas.round(rr, 5.0, p.selected);
            }
            let face = if item.merlin { body } else { mono };
            let nw = self.text.width(&item.name, face);
            self.text.draw_in_row(
                &mut self.canvas,
                &item.name,
                face,
                rr.x + 10.0,
                rr.y,
                row,
                p.bright,
                rr,
            );
            let ax = rr.x + 10.0 + nw.max(if item.merlin { 0.0 } else { 44.0 }) + 12.0;
            let chip = if item.merlin {
                self.plugin_chip("merlin", rr.right() - 8.0, rr.y + 5.0) + 8.0
            } else {
                0.0
            };
            let about = Rect::new(ax, rr.y, (rr.right() - 12.0 - chip - ax).max(0.0), row);
            self.text.draw_in_row(
                &mut self.canvas,
                &item.about,
                body,
                ax,
                rr.y,
                row,
                p.muted,
                about,
            );
        }
    }

    /// The find box at the top right of the code, as VS Code has it: the
    /// query with its count and arrows, and the replacement below it.
    /// Which-key: what the next keys do, in a card along the bottom of
    /// the text.
    pub(super) fn which_key(&mut self, card: &merlin_core::WhichKeyView, a: &Areas) {
        let p = self.p;
        let row = 24.0;
        let cols = card.columns.max(1);
        let rows = card.items.len().div_ceil(cols).max(1);
        let right = a
            .groups
            .iter()
            .find(|g| g.text == a.text)
            .map_or(a.text.right(), |g| g.bar.right());
        let w = (right - a.text.x - 24.0).max(200.0);
        let h = 16.0 + 30.0 + rows as f32 * row + 8.0;
        let r = Rect::new(a.text.x + 12.0, a.text.bottom() - h - 12.0, w, h);
        self.canvas.shadow(r, 10.0, theme::is_light(&p));
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let ui = Face::ui(12.5);
        let mono = Face::mono(12.0);
        let mut x = r.x + 16.0;
        let top = r.y + 12.0;
        // The keys so far, in a chip, then "then…" and the hint.
        let cw = self.text.width(&card.chord, mono);
        let chip = Rect::new(x, top + 3.0, cw + 14.0, 20.0);
        self.canvas.round(chip, 4.0, p.chip);
        self.text.draw_in_row(
            &mut self.canvas,
            &card.chord,
            mono,
            x + 7.0,
            chip.y,
            chip.h,
            p.bright,
            chip,
        );
        x = chip.right() + 10.0;
        self.text
            .draw_in_row(&mut self.canvas, "then…", ui, x, top, 26.0, p.muted, r);
        let small = Face::ui(11.5);
        let hw = self.text.width(&card.hint, small);
        self.text.draw_in_row(
            &mut self.canvas,
            &card.hint,
            small,
            r.right() - 16.0 - hw,
            top,
            26.0,
            p.muted,
            r,
        );
        let y0 = top + 34.0;
        let col_w = (r.w - 32.0 - (cols - 1) as f32 * 24.0) / cols as f32;
        let key_w = card
            .items
            .iter()
            .map(|i| self.text.width(&i.key, mono.bold()))
            .fold(18.0_f32, f32::max);
        for (i, item) in card.items.iter().enumerate() {
            let (c, rr) = (i / rows, i % rows);
            let cell = Rect::new(
                r.x + 16.0 + c as f32 * (col_w + 24.0),
                y0 + rr as f32 * row,
                col_w,
                row,
            );
            self.text.draw_in_row(
                &mut self.canvas,
                &item.key,
                mono.bold(),
                cell.x,
                cell.y,
                row,
                p.function,
                cell,
            );
            let lx = cell.x + key_w + 10.0;
            let label = if item.group {
                format!("+ {}", item.label)
            } else {
                item.label.clone()
            };
            let colour = if item.group { p.accent } else { p.text };
            let lw = self.text.width(&label, ui);
            let mut end = cell.right();
            if let Some(plugin) = &item.plugin {
                end -= self.plugin_chip(plugin, cell.right(), cell.y + 4.0) + 6.0;
            }
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                ui,
                lx,
                cell.y,
                row,
                colour,
                Rect::new(lx, cell.y, (end - lx).max(0.0), row),
            );
            if let Some(preview) = &item.preview {
                let px = lx + lw.max(if label.is_empty() { 0.0 } else { 140.0 }) + 14.0;
                self.text.draw_in_row(
                    &mut self.canvas,
                    preview,
                    mono,
                    px,
                    cell.y,
                    row,
                    p.muted,
                    Rect::new(px, cell.y, (end - px).max(0.0), row),
                );
            }
        }
    }
}
