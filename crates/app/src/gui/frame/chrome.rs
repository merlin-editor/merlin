//! The window around the islands: the title bar, the activity rail, the
//! status bar, the debug toolbar and notices.

use super::*;
use merlin_core::lang::StatusState;
use merlin_core::{LanguageCardHit, LanguageCardView, StatusMark};

/// The room a status bar item's mark takes before its text.
const MARK_W: f32 = 20.0;

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
            // A thin line before the plugins' views.
            if item.divider {
                self.canvas
                    .fill(Rect::new(12.0, y + 2.0, 20.0, 1.0), p.border);
                y += 7.0;
            }
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
            let drawn = match &item.icon {
                Some(icon) => self.view_icon(icon, r.x + 8.0, y + 8.0, 20.0, colour, r),
                None => {
                    let themed = self.icons.ui(item.id).cloned();
                    themed.is_some_and(|icon| {
                        self.draw_icon(&icon, r.x + 8.0, y + 8.0, 20.0, colour, r)
                    })
                }
            };
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
        let mut left_items = Vec::new();
        let mut right_items = Vec::new();
        for (i, item) in view.status.iter().enumerate() {
            // The language's { } or a spinner goes before its text.
            let tw = self.text.width(&item.text, face) + item.mark.map_or(0.0, |_| MARK_W);
            if item.right {
                right_items.push((i, item, tw));
            } else {
                left_items.push((i, item, tw));
            }
        }
        // Where there isn't room for both sides, the right side's middle
        // items go first: the indent, encoding, line endings and keymap,
        // keeping the cursor's place and the language.
        let left_w: f32 = left_items.iter().map(|(.., tw)| tw + 18.0).sum();
        let right_w = |r: &[(usize, _, f32)]| r.iter().map(|(.., tw)| tw + 18.0).sum::<f32>();
        while right_items.len() > 2 && x + left_w + right_w(&right_items) > w - 14.0 {
            right_items.remove(1);
        }
        let mut rx = w - 14.0;
        for &(i, item, tw) in right_items.iter().rev() {
            rx -= tw;
            let r = Rect::new(rx - 6.0, bar.y + 3.0, tw + 12.0, STATUS_H - 6.0);
            if item.open || (item.clickable && self.hovered(r)) {
                self.canvas.round(r, 5.0, p.hover);
            }
            let last = i + 1 == view.status.len();
            let colour = if last { p.chip_text } else { p.muted };
            let tx = match item.mark {
                Some(mark) => {
                    self.status_mark(mark, rx, bar.y);
                    rx + MARK_W
                }
                None => rx,
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.text,
                face,
                tx,
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
        // The left side stops short of the right: an item without room is
        // cut short, and those after it left out.
        let mut left_end = x;
        for (i, item, tw) in left_items {
            let room = rx - 12.0 - x;
            if room < 40.0 {
                break;
            }
            let text = if tw > room {
                self.text.fit(&item.text, face, room)
            } else {
                item.text.clone()
            };
            let tw = tw.min(room);
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
                &text,
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
        let h = th
            + 28.0
            + if toast.buttons.is_empty() && toast.link.is_none() {
                0.0
            } else {
                40.0
            };
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
        // The link, at the start of the row.
        if let Some(link) = &toast.link {
            let lf = Face::ui(12.0);
            let lw = self.text.width(link, lf);
            let l = Rect::new(r.x + 14.0, r.bottom() - 40.0, lw.min(bx - r.x - 22.0), 28.0);
            let colour = if self.hovered(l) {
                self.p.bright
            } else {
                self.p.accent
            };
            self.text
                .draw_in_row(&mut self.canvas, link, lf, l.x, l.y, l.h, colour, l);
            self.hit(l, Hit::Ui(UiEvent::ToastButton(toast.buttons.len())));
        }
    }

    /// A state's colour: green, the accent while busy, amber or red.
    fn state_colour(&self, state: StatusState) -> u32 {
        match state {
            StatusState::Ready => self.p.success,
            StatusState::Busy => self.p.accent,
            StatusState::Warning => self.p.warning,
            StatusState::Error => self.p.error,
        }
    }

    /// A status bar item's mark at `x`: the language's { } in its state's
    /// colour, or a spinner while something is busy.
    fn status_mark(&mut self, mark: StatusMark, x: f32, y: f32) {
        match mark {
            StatusMark::Braces(state) if state != StatusState::Busy => {
                let colour = self.state_colour(state);
                let face = Face::mono(11.5).bold();
                let bar = Rect::new(x, y, MARK_W, STATUS_H);
                self.text
                    .draw_in_row(&mut self.canvas, "{}", face, x, y, STATUS_H, colour, bar);
            }
            _ => self.spinner(x + 1.0, y + (STATUS_H - 10.0) / 2.0),
        }
    }

    /// A ring with its head in the accent, 10 across, from `x`, `y`.
    fn spinner(&mut self, x: f32, y: f32) {
        let p = self.p;
        let ring = Rect::new(x, y, 10.0, 10.0);
        self.canvas.outline(ring, 5.0, 1.5, p.faint, p.ground);
        self.canvas
            .round(Rect::new(x + 3.5, y, 3.0, 3.0), 1.5, p.accent);
    }

    /// The card the language item opens, over the status bar's right end:
    /// what the language's plugin reports, then the file's problems, format
    /// on save and tests.
    pub(super) fn language_card(&mut self, card: &LanguageCardView, w: f32, h: f32) {
        let p = self.p;
        let (cw, head_h, row_h, hint_h) = (400.0_f32.min(w - 20.0), 36.0, 32.0, 28.0);
        let rows_h: f32 = card
            .rows
            .iter()
            .map(|r| row_h + if r.hint.is_some() { hint_h } else { 0.0 })
            .sum();
        let ch = head_h + 1.0 + rows_h + 8.0;
        let r = Rect::new(w - cw - 10.0, h - STATUS_H - ch - 6.0, cw, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        // The language, the plugin it's from, and its settings.
        let (title_f, small) = (Face::ui(13.0).bold(), Face::ui(11.5));
        let mut x = r.x + 12.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &card.title,
            title_f,
            x,
            r.y,
            head_h,
            p.bright,
            r,
        );
        x += self.text.width(&card.title, title_f) + 8.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &card.from,
            small,
            x,
            r.y,
            head_h,
            p.faint,
            r,
        );
        if card.settings {
            let label = "Plugin settings";
            let lw = self.text.width(label, Face::ui(12.0));
            let l = Rect::new(r.right() - 12.0 - lw, r.y + 8.0, lw, head_h - 16.0);
            let colour = if self.hovered(l) { p.bright } else { p.accent };
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                Face::ui(12.0),
                l.x,
                r.y,
                head_h,
                colour,
                r,
            );
            self.hit(l, Hit::Ui(UiEvent::LanguageCard(LanguageCardHit::Settings)));
        }
        self.canvas
            .fill(Rect::new(r.x, r.y + head_h, r.w, 1.0), p.border);
        let mut y = r.y + head_h + 5.0;
        for (i, row) in card.rows.iter().enumerate() {
            self.language_row(row, i, r, y, row_h);
            y += row_h;
            if let Some(hint) = &row.hint {
                let mono = Face::mono(12.0);
                let copy = Rect::new(r.right() - 12.0 - 52.0, y + 2.0, 52.0, 22.0);
                let room = copy.x - r.x - 37.0 - 8.0;
                let text = self.text.fit(hint, mono, room);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    mono,
                    r.x + 29.0,
                    y,
                    hint_h - 2.0,
                    p.text,
                    r,
                );
                let (bg, fg) = self.button_colours(false, self.hovered(copy));
                self.canvas.round(copy, 5.0, bg);
                let f = Face::ui(12.0);
                let cx = copy.x + (copy.w - self.text.width("Copy", f)) / 2.0;
                self.text
                    .draw_in_row(&mut self.canvas, "Copy", f, cx, copy.y, copy.h, fg, copy);
                self.hit(
                    copy,
                    Hit::Ui(UiEvent::LanguageCard(LanguageCardHit::Copy(i))),
                );
                y += hint_h;
            }
        }
    }

    /// One of the card's rows: its dot, name and detail, then its progress
    /// and its button or link at the right.
    fn language_row(
        &mut self,
        row: &merlin_core::LanguageRowView,
        i: usize,
        card: Rect,
        y: f32,
        h: f32,
    ) {
        let p = self.p;
        let dot = row.state.map_or(p.faint, |s| self.state_colour(s));
        if row.state == Some(StatusState::Busy) {
            self.spinner(card.x + 11.0, y + (h - 10.0) / 2.0);
        } else {
            self.canvas.round(
                Rect::new(card.x + 12.0, y + (h - 8.0) / 2.0, 8.0, 8.0),
                4.0,
                dot,
            );
        }
        // At the right: the button or link, and the progress before it.
        let mut right = card.right() - 12.0;
        let hit = Hit::Ui(UiEvent::LanguageCard(LanguageCardHit::Row(i)));
        if let Some((label, primary)) = &row.button {
            let f = Face::ui(12.0);
            let bw = self.text.width(label, f) + 18.0;
            let b = Rect::new(right - bw, y + (h - 22.0) / 2.0, bw, 22.0);
            let (bg, fg) = self.button_colours(*primary, self.hovered(b));
            self.canvas.round(b, 5.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, f, b.x + 9.0, b.y, b.h, fg, b);
            self.hit(b, hit);
            right = b.x - 10.0;
        } else if let Some(link) = &row.link {
            let f = Face::ui(12.0);
            let lw = self.text.width(link, f);
            let l = Rect::new(right - lw, y + 6.0, lw, h - 12.0);
            let colour = if self.hovered(l) { p.bright } else { p.accent };
            self.text
                .draw_in_row(&mut self.canvas, link, f, l.x, y, h, colour, card);
            self.hit(l, hit);
            right = l.x - 10.0;
        }
        if let Some(done) = row.progress {
            let track = Rect::new(right - 90.0, y + (h - 3.0) / 2.0, 90.0, 3.0);
            self.canvas.round(track, 1.5, p.chip);
            let part = f32::from(done.min(100)) / 100.0;
            self.canvas.round(
                Rect::new(track.x, track.y, track.w * part, 3.0),
                1.5,
                p.accent,
            );
            right = track.x - 10.0;
        }
        let (name_f, detail_f) = (Face::ui(12.5).medium(), Face::ui(12.0));
        let x = card.x + 29.0;
        self.text
            .draw_in_row(&mut self.canvas, &row.name, name_f, x, y, h, p.bright, card);
        let dx = x + self.text.width(&row.name, name_f) + 9.0;
        let detail = self.text.fit(&row.detail, detail_f, (right - dx).max(0.0));
        self.text
            .draw_in_row(&mut self.canvas, &detail, detail_f, dx, y, h, p.muted, card);
    }
}
