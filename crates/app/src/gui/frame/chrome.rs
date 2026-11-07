//! The window around the islands: the title bar, the activity rail, the
//! status bar, the debug toolbar and notices.

use super::*;

impl Frame<'_> {
    pub(super) fn title_bar(&mut self, view: &WindowView, w: f32) {
        let p = self.p;
        let bar = Rect::new(0.0, 0.0, w, TITLE_H);
        let ui = Face::ui(13.0);
        // The menus, behind one button.
        self.icon_button(
            Rect::new(10.0, 6.0, 30.0, 28.0),
            "menu",
            '\u{f0c9}',
            false,
            Hit::Key(Key::plain(KeyCode::F(10))),
        );
        // The project, with a mark in the accent.
        let mut x = 46.0;
        let mark = Rect::new(x + 6.0, 11.0, 18.0, 18.0);
        let name_w = self.text.width(&view.project, ui.bold());
        let project = Rect::new(x, 6.0, name_w + 40.0, 28.0);
        if self.hovered(project) {
            self.canvas.round(project, 6.0, p.hover);
        }
        self.canvas.round(mark, 5.0, p.accent);
        let m = "m";
        self.centred(m, Face::ui(11.0).bold(), mark, p.ground, mark);
        self.text.draw_in_row(
            &mut self.canvas,
            &view.project,
            ui.bold(),
            x + 32.0,
            6.0,
            28.0,
            p.bright,
            bar,
        );
        self.hit(project, Hit::Command("file.open"));
        x += project.w + 4.0;
        if let Some(branch) = &view.branch {
            let label = format!("\u{e725} {branch}");
            let bw = self.text.width(&label, ui);
            let r = Rect::new(x, 6.0, bw + 16.0, 28.0);
            if self.hovered(r) {
                self.canvas.round(r, 6.0, p.hover);
            }
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                ui,
                x + 8.0,
                6.0,
                28.0,
                p.muted,
                bar,
            );
            self.hit(r, Hit::Ui(UiEvent::Rail(2)));
        }
        // The one search box.
        let sw = 440.0_f32.min(w * 0.42);
        let search = Rect::new((w - sw) / 2.0, 6.0, sw, 28.0);
        let hovered = self.hovered(search);
        self.canvas
            .round(search, 7.0, if hovered { p.widget } else { p.raised });
        self.text.draw_in_row(
            &mut self.canvas,
            "\u{ea6d}",
            Face::mono(13.0),
            search.x + 10.0,
            search.y,
            28.0,
            p.muted,
            search,
        );
        let hint = self.text.fit(
            "Search files, commands, symbols and text",
            ui,
            (sw - 100.0).max(0.0),
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &hint,
            ui,
            search.x + 30.0,
            search.y,
            28.0,
            p.muted,
            search,
        );
        self.chip("Ctrl Shift P", search.right() - 8.0, search.y + 6.0, true);
        self.hit(search, Hit::Command("palette.open"));
        // Layout toggles: side bar, panel, secondary side bar.
        let mut bx = w - 10.0 - 3.0 * 32.0;
        for (i, (id, glyph, command)) in [
            ("layout-left", '\u{ebf3}', "view.sidebar"),
            ("layout-panel", '\u{ebf2}', "view.panel"),
            ("layout-right", '\u{ebf4}', "view.secondary"),
        ]
        .into_iter()
        .enumerate()
        {
            self.icon_button(
                Rect::new(bx, 6.0, 30.0, 28.0),
                id,
                glyph,
                view.shown[i],
                Hit::Command(command),
            );
            bx += 32.0;
        }
    }

    pub(super) fn rail(&mut self, view: &WindowView, h: f32) {
        let p = self.p;
        let mut y = TITLE_H + 2.0;
        for (i, item) in view.rail.iter().enumerate() {
            let r = Rect::new(4.0, y, 36.0, 36.0);
            let hovered = self.hovered(r);
            if item.active {
                self.canvas.round(r, 8.0, p.raised);
            } else if hovered {
                self.canvas.round(r, 8.0, p.hover);
            }
            let face = Face::mono(18.0);
            let g = item.glyph.to_string();
            let gw = self.text.width(&g, face);
            let colour = if item.active || hovered {
                p.bright
            } else {
                p.muted
            };
            let themed = self.icons.ui(item.id).cloned();
            let drawn = themed
                .is_some_and(|icon| self.draw_icon(&icon, r.x + 8.0, y + 8.0, 20.0, colour, r));
            if !drawn {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &g,
                    face,
                    r.x + (36.0 - gw) / 2.0,
                    y,
                    36.0,
                    colour,
                    r,
                );
            }
            if item.badge > 0 {
                let label = if item.badge > 99 {
                    "99+".to_string()
                } else {
                    item.badge.to_string()
                };
                let face = Face::ui(10.0).bold();
                let bw = self.text.width(&label, face).max(7.0) + 8.0;
                let b = Rect::new(r.right() - bw + 1.0, y + 3.0, bw, 15.0);
                self.canvas.round(b, 7.5, p.accent);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    b.x + 4.0,
                    b.y,
                    15.0,
                    p.ground,
                    b,
                );
            }
            self.hit(r, Hit::Ui(UiEvent::Rail(i)));
            self.tips.push((r, item.label.clone()));
            y += 40.0;
            if y + 40.0 > h - STATUS_H {
                break;
            }
        }
    }

    pub(super) fn status(&mut self, view: &WindowView, w: f32, h: f32) {
        let p = self.p;
        let bar = Rect::new(0.0, h - STATUS_H, w, STATUS_H);
        // While debugging, the bar takes the debug colour, as VS Code's.
        let debugging = view.debug.is_some();
        if debugging {
            self.canvas.fill(bar, theme::mix(p.warning, p.ground, 0.14));
        }
        let face = Face::ui(12.0);
        let mut x = RAIL_W + 12.0;
        // Vim's mode in its colour, then the keys typed so far.
        if let Some(vim) = &view.vim {
            use merlin_core::VimMode;
            let command = view.vim_line.is_some();
            let fill = match vim.mode {
                _ if command => p.warning,
                VimMode::Normal => p.accent,
                VimMode::Insert => p.success,
                VimMode::Visual | VimMode::VisualLine | VimMode::VisualBlock => p.keyword,
                VimMode::Replace => p.error,
            };
            let chip_face = Face::ui(10.5).bold();
            let label = if command { "COMMAND" } else { vim.mode.label() };
            let tw = self.text.width(label, chip_face);
            let r = Rect::new(x - 4.0, bar.y + 4.0, tw + 14.0, STATUS_H - 8.0);
            self.canvas.round(r, 4.0, fill);
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                chip_face,
                r.x + 7.0,
                r.y,
                r.h,
                p.ground,
                r,
            );
            x = r.right() + 10.0;
            if !vim.pending.is_empty() {
                let mono = Face::mono(12.0);
                let w = self.text.width(&vim.pending, mono);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &vim.pending,
                    mono,
                    x,
                    bar.y,
                    STATUS_H,
                    p.text,
                    bar,
                );
                x += w + 14.0;
            }
            if let Some(note) = &vim.note {
                // Cut short to leave the bar's other items their room.
                let items: f32 = view
                    .status
                    .iter()
                    .map(|i| self.text.width(&i.text, face) + 18.0)
                    .sum();
                let note = &self.text.fit(note, face, (w - 14.0 - items - x).max(60.0));
                let w = self.text.width(note, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    note,
                    face,
                    x,
                    bar.y,
                    STATUS_H,
                    p.text,
                    bar,
                );
                x += w + 14.0;
            }
        }
        // A macro recording, with its keys so far, or playing, with how far
        // it has got.
        if let Some(rec) = &view.recording {
            let (label, colour) = match rec.progress {
                Some((done, of)) => (format!("Playing {} · {done} of {of}", rec.name), p.text),
                None if rec.name.is_empty() => ("Recording".to_string(), p.error),
                None => (format!("Recording {}", rec.name), p.error),
            };
            if rec.progress.is_none() {
                let d = 7.0;
                self.canvas.round(
                    Rect::new(x, bar.y + (STATUS_H - d) / 2.0, d, d),
                    d / 2.0,
                    p.error,
                );
                x += d + 6.0;
            }
            let w = self.text.width(&label, face);
            self.text.draw_in_row(
                &mut self.canvas,
                &label,
                face,
                x,
                bar.y,
                STATUS_H,
                colour,
                bar,
            );
            x += w + 10.0;
            if let Some((done, of)) = rec.progress {
                let track = Rect::new(x, bar.y + (STATUS_H - 4.0) / 2.0, 120.0, 4.0);
                self.canvas.round(track, 2.0, p.chip);
                let part = (done as f32 / of.max(1) as f32).clamp(0.0, 1.0);
                if part > 0.0 {
                    let fill = Rect::new(track.x, track.y, track.w * part, track.h);
                    self.canvas.round(fill, 2.0, p.accent);
                }
                x = track.right() + 10.0;
            }
            if !rec.keys.is_empty() {
                let mono = Face::mono(12.0);
                // The last keys, if there are more than fit.
                let shown: String = {
                    let chars: Vec<char> = rec.keys.chars().collect();
                    let from = chars.len().saturating_sub(40);
                    chars[from..].iter().collect()
                };
                let w = self.text.width(&shown, mono);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &shown,
                    mono,
                    x,
                    bar.y,
                    STATUS_H,
                    p.bright,
                    bar,
                );
                x += w + 10.0;
            }
            let w = self.text.width(&rec.hint, face);
            self.text.draw_in_row(
                &mut self.canvas,
                &rec.hint,
                face,
                x,
                bar.y,
                STATUS_H,
                p.muted,
                bar,
            );
            x += w + 14.0;
        }
        let mut left_end = x;
        let mut right_items = Vec::new();
        for (i, item) in view.status.iter().enumerate() {
            if item.right {
                right_items.push((i, item));
                continue;
            }
            let tw = self.text.width(&item.text, face);
            let r = Rect::new(x - 6.0, bar.y + 3.0, tw + 12.0, STATUS_H - 6.0);
            if item.clickable && self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let colour = if item.error {
                p.error
            } else if item.accent {
                p.accent
            } else if debugging {
                p.warning
            } else {
                p.muted
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.text,
                face,
                x,
                bar.y,
                STATUS_H,
                colour,
                bar,
            );
            if item.clickable {
                self.hit(r, Hit::Ui(UiEvent::Status(i)));
            }
            x += tw + 18.0;
            left_end = x;
        }
        let mut rx = w - 14.0;
        for (i, item) in right_items.into_iter().rev() {
            let tw = self.text.width(&item.text, face);
            rx -= tw;
            let r = Rect::new(rx - 6.0, bar.y + 3.0, tw + 12.0, STATUS_H - 6.0);
            if item.clickable && self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let last = i + 1 == view.status.len();
            let colour = if last { p.chip_text } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.text,
                face,
                rx,
                bar.y,
                STATUS_H,
                colour,
                bar,
            );
            if item.clickable {
                self.hit(r, Hit::Ui(UiEvent::Status(i)));
            }
            rx -= 18.0;
        }
        if let Some((message, error)) = &view.problem {
            let room = rx - left_end - 12.0;
            if room > 60.0 {
                let text = self.text.fit(message, face, room);
                let colour = if *error { p.error } else { p.warning };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    face,
                    left_end + 6.0,
                    bar.y,
                    STATUS_H,
                    colour,
                    bar,
                );
            }
        }
    }

    /// The debug toolbar, floating at the top of the code: Continue or
    /// Pause, the steps, Restart and Stop; only Stop while building.
    pub(super) fn debug_bar(&mut self, bar: &merlin_core::DebugBarView, editor: Rect) {
        use merlin_core::DebugButton as B;
        let p = self.p;
        let buttons: Vec<(B, &str, &str, u32)> = if bar.building {
            vec![(B::Stop, "■", "Stop (Shift+F5)", p.error)]
        } else {
            let first = if bar.paused {
                (B::Continue, "▶", "Continue (F5)", p.success)
            } else {
                (B::Pause, "❚❚", "Pause (F6)", p.warning)
            };
            vec![
                first,
                (B::StepOver, "↷", "Step Over (F10)", p.accent),
                (B::StepInto, "↓", "Step Into (F11)", p.accent),
                (B::StepOut, "↑", "Step Out (Shift+F11)", p.accent),
                (B::Restart, "↻", "Restart (Ctrl+Shift+F5)", p.success),
                (B::Stop, "■", "Stop (Shift+F5)", p.error),
            ]
        };
        let (bw, bh, pad) = (30.0, 26.0, 4.0);
        let w = buttons.len() as f32 * (bw + 2.0) - 2.0 + pad * 2.0;
        let r = Rect::new(
            (editor.x + (editor.w - w) / 2.0).round(),
            editor.y + 6.0,
            w,
            bh + pad * 2.0,
        );
        self.canvas.shadow(r, 9.0, theme::is_light(&p));
        self.canvas.outline(r, 9.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let face = Face::ui(14.0).bold();
        let mut x = r.x + pad;
        for (button, glyph, tip, colour) in buttons {
            let b = Rect::new(x, r.y + pad, bw, bh);
            // Steps wait while it runs.
            let waiting = !bar.paused
                && !bar.building
                && matches!(button, B::StepOver | B::StepInto | B::StepOut);
            if !waiting {
                if self.hovered(b) {
                    self.canvas.round(b, 6.0, p.hover);
                }
                self.hit(b, Hit::Ui(UiEvent::Debug(button)));
            }
            self.tips.push((b, tip.to_string()));
            let colour = if waiting {
                theme::mix(colour, p.widget, 0.65)
            } else {
                colour
            };
            // A step is drawn as its arrow just over a dot, the line it
            // lands on, as VS Code draws them.
            if matches!(button, B::StepOver | B::StepInto | B::StepOut) {
                self.step_icon(button, b.x + bw / 2.0, b.y + bh / 2.0, colour);
            } else {
                let gw = self.text.width(glyph, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    glyph,
                    face,
                    b.x + (bw - gw) / 2.0,
                    b.y,
                    bh,
                    colour,
                    b,
                );
            }
            x += bw + 2.0;
        }
    }

    /// A step button's icon, in a 16 px square centred at (`cx`, `cy`):
    /// an arc over the line for Step Over, an arrow down into it or up out
    /// of it, and the dot under each.
    pub(super) fn step_icon(
        &mut self,
        button: merlin_core::DebugButton,
        cx: f32,
        cy: f32,
        colour: u32,
    ) {
        use merlin_core::DebugButton as B;
        let w = 1.7;
        let c = &mut self.canvas;
        match button {
            B::StepOver => {
                // An arc over the top, from left to right, then its head.
                let (ox, oy, r) = (cx, cy - 0.5, 5.5);
                let steps = 10;
                let at = |i: usize| {
                    let t = std::f32::consts::PI * (1.0 + i as f32 / steps as f32);
                    (ox + r * t.cos(), oy + r * t.sin())
                };
                for i in 0..steps {
                    c.line(at(i), at(i + 1), w, colour);
                }
                let end = (ox + r, oy);
                c.line(end, (end.0 - 3.6, end.1 - 1.2), w, colour);
                c.line(end, (end.0 + 0.9, end.1 - 3.6), w, colour);
            }
            B::StepInto => {
                let (top, tip) = (cy - 7.5, cy + 1.0);
                c.line((cx, top), (cx, tip), w, colour);
                c.line((cx - 3.5, tip - 3.5), (cx, tip), w, colour);
                c.line((cx + 3.5, tip - 3.5), (cx, tip), w, colour);
            }
            _ => {
                let (bottom, tip) = (cy + 1.0, cy - 7.5);
                c.line((cx, bottom), (cx, tip), w, colour);
                c.line((cx - 3.5, tip + 3.5), (cx, tip), w, colour);
                c.line((cx + 3.5, tip + 3.5), (cx, tip), w, colour);
            }
        }
        let r = 2.1;
        let dy = cy + 5.2;
        c.round(Rect::new(cx - r, dy - r, r * 2.0, r * 2.0), r, colour);
    }

    pub(super) fn toast(&mut self, toast: &ToastView, editor: Rect) {
        let p = self.p;
        let face = Face::ui(13.0);
        let w = 420.0_f32.min(editor.w - 32.0);
        let text_w = w - 56.0;
        let th = self.text.wrapped_height(&toast.text, face, text_w);
        let h = th + 28.0 + if toast.buttons.is_empty() { 0.0 } else { 40.0 };
        let r = Rect::new(editor.right() - w - 16.0, editor.bottom() - h - 16.0, w, h);
        self.canvas.shadow(r, 10.0, theme::is_light(&p));
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        // Later hits win, so the box goes under its buttons.
        self.hit(r, Hit::Block);
        let colour = if toast.error {
            p.error
        } else if toast.done {
            p.success
        } else if toast.warning {
            p.warning
        } else {
            p.accent
        };
        let dot = Rect::new(r.x + 16.0, r.y + 17.0, 10.0, 10.0);
        self.canvas.round(dot, 5.0, colour);
        self.text.draw(
            &mut self.canvas,
            &toast.text,
            face,
            r.x + 38.0,
            r.y + 12.0,
            p.text,
            r,
            Some(text_w),
        );
        let mut bx = r.right() - 14.0;
        for (i, label) in toast.buttons.iter().enumerate().rev() {
            let bf = Face::ui(12.5).medium();
            let bw = self.text.width(label, bf) + 24.0;
            bx -= bw;
            let b = Rect::new(bx, r.bottom() - 40.0, bw, 28.0);
            let primary = i + 1 == toast.buttons.len();
            let (bg, fg) = self.button_colours(primary, self.hovered(b));
            self.canvas.round(b, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, bf, b.x + 12.0, b.y, 28.0, fg, b);
            self.hit(b, Hit::Ui(UiEvent::ToastButton(i)));
            bx -= 8.0;
        }
    }
}
