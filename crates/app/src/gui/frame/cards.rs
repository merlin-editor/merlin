//! Cards over the text: hover, definition, code actions, completions,
//! signature help, rename and breakpoint boxes, menus, dialogs and tooltips.

use super::*;

impl Frame<'_> {
    /// Hover and completion as a card under the text they're about, or
    /// over it when there's no room below.
    pub(super) fn popup(&mut self, popup: &PopupView, a: &Areas, (cw, ch): (f32, f32)) {
        let anchor = (
            a.text.x + popup.anchor.0 as f32 * cw,
            a.text.y + popup.anchor.1 as f32 * ch,
        );
        match &popup.kind {
            PopupKind::Rename(view) => self.rename_box(view, anchor, a, (cw, ch)),
            PopupKind::Breakpoint(view) => self.break_box(view, anchor, a, ch),
            PopupKind::Exception { title, message } => {
                self.exception_card(title, message, anchor, a, ch)
            }
            PopupKind::Hover {
                problems,
                lines,
                definition,
                colours,
                fix,
                more_fixes,
            } => {
                let card = HoverCard {
                    problems,
                    lines,
                    colours,
                    definition: *definition,
                    fix: fix.as_deref(),
                    more_fixes: *more_fixes,
                };
                self.hover_card(&card, anchor, a, ch)
            }
            PopupKind::Actions { actions, selected } => {
                self.actions_card(actions, *selected, anchor, a, ch)
            }
            PopupKind::Completion { items, selected } => {
                self.completions(items, *selected, anchor, a, ch)
            }
            PopupKind::Definition {
                file,
                line,
                first,
                lines,
                colours,
            } => self.definition_card(file, (*line, *first), lines, colours, anchor, a, ch),
        }
    }

    /// The definition of the name under Ctrl, under it: its file and line,
    /// its first lines coloured with their numbers, and what a click does.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn definition_card(
        &mut self,
        file: &str,
        (line, first): (usize, usize),
        lines: &[String],
        colours: &[Spans],
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let ui = Face::ui(12.0);
        let line_h = 20.0;
        let last = first + lines.len().saturating_sub(1);
        let num_w = self.text.width(&last.to_string(), CODE) + 14.0;
        let widest = lines
            .iter()
            .map(|l| self.text.width(&l.replace('\t', "    "), CODE))
            .fold(0.0, f32::max);
        let w = (num_w + widest + 36.0).clamp(360.0, 640.0);
        let (head_h, foot_h) = (30.0, 28.0);
        let h = head_h + 12.0 + lines.len() as f32 * line_h + foot_h;
        let r = self.card_place((anchor.0 - 7.0, anchor.1), w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        // The file and line.
        let x = r.x + 12.0;
        let fw = self
            .text
            .draw_in_row(&mut self.canvas, file, ui, x, r.y, head_h, p.text, r);
        self.text.draw_in_row(
            &mut self.canvas,
            &format!(" · line {line}"),
            ui,
            x + fw,
            r.y,
            head_h,
            p.muted,
            r,
        );
        self.canvas
            .fill(Rect::new(r.x + 1.0, r.y + head_h, r.w - 2.0, 1.0), p.line);
        // The lines, numbered.
        let code = Rect::new(
            r.x + 1.0,
            r.y + head_h + 1.0,
            r.w - 2.0,
            h - head_h - foot_h - 2.0,
        );
        let mut y = r.y + head_h + 6.0;
        let none = Spans::new();
        for (k, text) in lines.iter().enumerate() {
            let n = (first + k).to_string();
            let nw = self.text.width(&n, CODE);
            self.text.draw_in_row(
                &mut self.canvas,
                &n,
                CODE,
                x + num_w - 14.0 - nw,
                y,
                line_h,
                p.faint,
                code,
            );
            let spans = colours.get(k).unwrap_or(&none);
            self.code_line(text, spans, x + num_w, y, line_h, code);
            y += line_h;
        }
        // What a click does.
        let fy = r.bottom() - foot_h;
        self.canvas
            .fill(Rect::new(r.x + 1.0, fy, r.w - 2.0, 1.0), p.line);
        let small = Face::ui(11.5);
        let mut fx = x;
        for (keys, what) in [
            ("Click", " to go there"),
            ("Ctrl Alt click", " to the side"),
        ] {
            fx += self.text.draw_in_row(
                &mut self.canvas,
                keys,
                small.medium(),
                fx,
                fy,
                foot_h,
                p.text,
                r,
            );
            fx += self
                .text
                .draw_in_row(&mut self.canvas, what, small, fx, fy, foot_h, p.muted, r)
                + 16.0;
        }
    }

    /// The rename box (F2) under the name, which is outlined: the new name
    /// in a field, and what Enter and Shift+Enter do.
    pub(super) fn rename_box(
        &mut self,
        view: &RenameView,
        anchor: (f32, f32),
        a: &Areas,
        (cw, ch): (f32, f32),
    ) {
        let p = self.p;
        let name = Rect::new(anchor.0 - 1.0, anchor.1, view.name as f32 * cw + 2.0, ch);
        self.canvas.frame(name, 2.0, 1.0, p.accent, 0);
        let mono = Face::mono(13.0);
        let note = Face::ui(11.5);
        let text_w = self.text.width(&view.text, mono);
        let note_w = self.text.width(&view.note, note);
        let w = (text_w + 40.0).max(note_w + 16.0).clamp(280.0, 560.0);
        let h = 6.0 + 26.0 + 6.0 + 16.0 + 6.0;
        let r = self.card_place((anchor.0 - 7.0, anchor.1), w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let field = Rect::new(r.x + 6.0, r.y + 6.0, r.w - 12.0, 26.0);
        self.canvas.outline(field, 5.0, 1.0, p.accent, p.raised);
        self.input_box(field);
        let x = field.x + 8.0;
        if view.selected && !view.text.is_empty() {
            let sel = Rect::new(x - 1.0, field.y + 5.0, text_w + 2.0, 16.0);
            self.canvas.round(sel, 2.0, p.selection);
        }
        self.text.draw_in_row(
            &mut self.canvas,
            &view.text,
            mono,
            x,
            field.y,
            field.h,
            p.bright,
            field.inset(1.0),
        );
        let before: String = view.text.chars().take(view.caret).collect();
        let cx = x + self.text.width(&before, mono);
        self.canvas
            .fill(Rect::new(cx, field.y + 5.5, 1.5, 15.0), p.accent);
        let shown = self.text.fit(&view.note, note, r.w - 16.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &shown,
            note,
            r.x + 8.0,
            field.bottom() + 4.0,
            16.0,
            p.muted,
            r,
        );
    }

    /// What went wrong, under the line a run stopped on with a panic: a
    /// red-edged card with its title, the message in the code's font, and
    /// Continue and Copy.
    pub(super) fn exception_card(
        &mut self,
        title: &str,
        message: &str,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let mono = Face::mono(12.5);
        let title_face = Face::ui(13.0).bold();
        let lines: Vec<&str> = message.lines().take(6).collect();
        let title_w = self.text.width(title, title_face) + 40.0;
        let widest = lines
            .iter()
            .map(|l| self.text.width(l, mono))
            .fold(title_w, f32::max);
        let w = (widest + 32.0).clamp(340.0, 640.0);
        let line_h = 18.0;
        let h = 10.0 + 22.0 + 4.0 + lines.len() as f32 * line_h + 10.0 + 30.0 + 10.0;
        let r = self.card_place((anchor.0 - 7.0, anchor.1), w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(
            r,
            8.0,
            1.0,
            theme::mix(p.error, p.widget, 0.5),
            theme::mix(p.error, p.widget, 0.1),
        );
        self.hit(r, Hit::Block);
        let x = r.x + 12.0;
        let mut y = r.y + 10.0;
        self.text.draw_in_row(
            &mut self.canvas,
            "⊗",
            Face::ui(14.0),
            x,
            y,
            22.0,
            p.error,
            r,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            title,
            title_face,
            x + 22.0,
            y,
            22.0,
            p.bright,
            r,
        );
        y += 26.0;
        for line in &lines {
            let shown = self.text.fit(line, mono, r.w - 24.0);
            self.text
                .draw_in_row(&mut self.canvas, &shown, mono, x, y, line_h, p.text, r);
            y += line_h;
        }
        y += 10.0;
        let bw = self.button("Continue  F5", x, y, false, true);
        self.hit(
            Rect::new(x, y, bw, 30.0),
            Hit::Ui(UiEvent::Debug(merlin_core::DebugButton::Continue)),
        );
        let cx = x + bw + 8.0;
        let cw = self.button("Copy", cx, y, false, false);
        self.hit(Rect::new(cx, y, cw, 30.0), Hit::Ui(UiEvent::CopyException));
    }

    /// The box that edits a breakpoint, under its line: the kind it sets
    /// in a chip, the text with its caret, and what Enter does.
    pub(super) fn break_box(
        &mut self,
        view: &merlin_core::BreakBoxView,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let mono = Face::mono(13.0);
        let note = Face::ui(11.5);
        let chip_face = Face::ui(12.0).medium();
        let chip_w = self.text.width(&view.field, chip_face) + 20.0;
        let text_w = self.text.width(&view.text, mono);
        let note_w = self.text.width(&view.hint, note);
        let w = (chip_w + text_w + 52.0)
            .max(note_w + 16.0)
            .clamp(360.0, 620.0);
        let h = 6.0 + 26.0 + 6.0 + 16.0 + 6.0;
        let r = self.card_place((anchor.0 - 7.0, anchor.1), w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let chip = Rect::new(r.x + 6.0, r.y + 6.0, chip_w, 26.0);
        self.canvas.outline(chip, 5.0, 1.0, p.border, p.raised);
        self.text.draw_in_row(
            &mut self.canvas,
            &view.field,
            chip_face,
            chip.x + 10.0,
            chip.y,
            chip.h,
            p.text,
            chip,
        );
        let field = Rect::new(
            chip.right() + 6.0,
            chip.y,
            r.right() - 6.0 - chip.right() - 6.0,
            26.0,
        );
        self.canvas.outline(field, 5.0, 1.0, p.accent, p.raised);
        self.input_box(field);
        let x = field.x + 8.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &view.text,
            mono,
            x,
            field.y,
            field.h,
            p.bright,
            field.inset(1.0),
        );
        let before: String = view.text.chars().take(view.caret).collect();
        let cx = x + self.text.width(&before, mono);
        self.canvas
            .fill(Rect::new(cx, field.y + 5.5, 1.5, 15.0), p.accent);
        let shown = self.text.fit(&view.hint, note, r.w - 16.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &shown,
            note,
            r.x + 8.0,
            field.bottom() + 4.0,
            16.0,
            p.muted,
            r,
        );
    }

    /// Where a card of `w` by `h` goes for text at `anchor`: under its
    /// line, or over it when there's no room below.
    pub(super) fn card_place(
        &self,
        (ax, ay): (f32, f32),
        w: f32,
        h: f32,
        a: &Areas,
        ch: f32,
    ) -> Rect {
        let below = ay + ch + 4.0;
        let y = if below + h > a.editor.bottom() - 4.0 && ay - h - 4.0 > a.editor.y {
            ay - h - 4.0
        } else {
            below
        };
        let x = ax.min(a.editor.right() - w - 8.0).max(a.editor.x + 4.0);
        Rect::new(x, y, w, h)
    }

    pub(super) fn completions(
        &mut self,
        items: &[(String, String)],
        selected: usize,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let mono = Face::mono(12.5);
        let ui = Face::ui(13.0);
        let first = selected.saturating_sub(7);
        let rows: Vec<&(String, String)> = items.iter().skip(first).take(8).collect();
        if rows.is_empty() {
            return;
        }
        let row_h = 21.0;
        let widest = rows
            .iter()
            .map(|(t, d)| {
                self.text.width(t, mono)
                    + if d.is_empty() {
                        0.0
                    } else {
                        self.text.width(d, ui) + 24.0
                    }
            })
            .fold(0.0, f32::max);
        let pw = (widest + 24.0).clamp(160.0, 560.0);
        let ph = rows.len() as f32 * row_h + 14.0;
        let r = self.card_place(anchor, pw, ph, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        for (k, (text, detail)) in rows.iter().enumerate() {
            let ry = r.y + 7.0 + k as f32 * row_h;
            let row = Rect::new(r.x + 4.0, ry, r.w - 8.0, row_h);
            if selected - first == k {
                self.canvas.round(row, 4.0, p.focus_row);
            }
            let shown = self.text.fit(text, mono, row.w - 16.0);
            self.text.draw_in_row(
                &mut self.canvas,
                &shown,
                mono,
                row.x + 8.0,
                ry,
                row_h,
                p.bright,
                row,
            );
            if !detail.is_empty() {
                let dw = self.text.width(detail, ui).min(row.w * 0.5);
                let shown = self.text.fit(detail, ui, dw);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &shown,
                    ui,
                    row.right() - 8.0 - dw,
                    ry,
                    row_h,
                    p.muted,
                    row,
                );
            }
        }
    }

    /// Code actions by the cursor, grouped as Quick Fix, Refactor, Source
    /// and More Actions, the one Enter takes lit up and the server's
    /// preferred fix marked with a bulb.
    pub(super) fn actions_card(
        &mut self,
        actions: &[CodeAction],
        selected: usize,
        anchor: (f32, f32),
        a: &Areas,
        ch: f32,
    ) {
        let p = self.p;
        let face = (Face::ui(12.5), Face::mono(12.0));
        const SHOWN: usize = 14;
        let first = selected.saturating_sub(SHOWN - 1);
        let shown: Vec<(usize, &CodeAction)> =
            actions.iter().enumerate().skip(first).take(SHOWN).collect();
        if shown.is_empty() {
            return;
        }
        let mut widest: f32 = 0.0;
        for (_, action) in &shown {
            widest = widest.max(self.rich_width(&action.title, face));
        }
        let w = (widest + 52.0).clamp(260.0, 520.0);
        // Rows: a group's title before its first action, a rule between.
        let (row_h, title_h, rule_h) = (28.0, 22.0, 9.0);
        let mut layout: Vec<(f32, Option<usize>, Option<ActionKind>)> = Vec::new();
        let mut y = 4.0;
        let mut group = None;
        for &(i, action) in &shown {
            if group != Some(action.kind) {
                if group.is_some() {
                    layout.push((y, None, None));
                    y += rule_h;
                }
                // Formatting is the editor's own, under the rule alone.
                if action.kind != ActionKind::Format {
                    layout.push((y, None, Some(action.kind)));
                    y += title_h;
                }
                group = Some(action.kind);
            }
            layout.push((y, Some(i), None));
            y += row_h;
        }
        let r = self.card_place(anchor, w, y + 4.0, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        for (top, row, title) in layout {
            let ry = r.y + top;
            match (row, title) {
                (None, Some(kind)) => {
                    let label = match kind {
                        ActionKind::QuickFix => "Quick Fix",
                        ActionKind::Refactor => "Refactor",
                        ActionKind::Source => "Source",
                        ActionKind::Other => "More Actions",
                        ActionKind::Format => "Format",
                    };
                    let f = Face::ui(11.0);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        label,
                        f,
                        r.x + 14.0,
                        ry,
                        title_h,
                        p.muted,
                        r,
                    );
                }
                (None, None) => self.canvas.fill(
                    Rect::new(r.x + 6.0, ry + rule_h / 2.0, r.w - 12.0, 1.0),
                    p.border,
                ),
                (Some(i), _) => {
                    let action = &actions[i];
                    let b = Rect::new(r.x + 4.0, ry, r.w - 8.0, row_h);
                    let hovered = self.hovered(b);
                    if i == selected {
                        self.canvas.round(b, 5.0, p.focus_row);
                    } else if hovered {
                        self.canvas.round(b, 5.0, p.hover);
                    }
                    if action.preferred {
                        self.bulb_icon(b.x + 8.0, ry + (row_h - 12.0) / 2.0, 12.0, p.warning);
                    }
                    let format = action.kind == ActionKind::Format;
                    let colour = match (i == selected, format) {
                        (true, _) => p.bright,
                        (false, true) => p.muted,
                        (false, false) => p.text,
                    };
                    if format {
                        let key = if action.id == merlin_core::editor::FORMAT_SELECTION {
                            "Ctrl K Ctrl F"
                        } else {
                            "Ctrl Shift I"
                        };
                        self.chip(key, b.right() - 8.0, ry + (row_h - 16.0) / 2.0, true);
                    }
                    let line = self
                        .wrap_rich(&action.title, face, f32::MAX)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    let clip = Rect::new(b.x, ry, b.w - 4.0, row_h);
                    self.draw_rich(&line, b.x + 28.0, ry, row_h, face, colour, colour, clip);
                    self.hit(b, Hit::Ui(UiEvent::Action(i)));
                }
            }
        }
    }

    /// Signature help: a card over the line of the call being typed, or
    /// under it when there's no room above, so it keeps clear of the
    /// completions below. The parameter the cursor is in stands out.
    pub(super) fn signature(&mut self, view: &SignatureView, a: &Areas, (cw, ch): (f32, f32)) {
        let p = self.p;
        let sig = &view.signature;
        let (ax, ay) = (
            a.text.x + view.anchor.0 as f32 * cw,
            a.text.y + view.anchor.1 as f32 * ch,
        );
        let mono = Face::mono(12.5);
        let ui = Face::ui(12.0);
        let chars: Vec<char> = sig.label.chars().collect();
        let (s, e) = sig.active.unwrap_or((0, 0));
        let (s, e) = (
            s.min(chars.len()),
            e.min(chars.len()).max(s.min(chars.len())),
        );
        let parts: [String; 3] = [
            chars[..s].iter().collect(),
            chars[s..e].iter().collect(),
            chars[e..].iter().collect(),
        ];
        let count = format!("{} of {}", sig.index + 1, sig.count.max(1));
        let label_w: f32 = parts
            .iter()
            .enumerate()
            .map(|(k, t)| self.text.width(t, if k == 1 { mono.bold() } else { mono }))
            .sum();
        let count_w = self.text.width(&count, ui);
        let pw = (label_w + count_w + 48.0).clamp(220.0, 620.0);
        let doc_w = pw - 24.0;
        let doc_h = if sig.doc.is_empty() {
            0.0
        } else {
            self.text
                .wrapped_height(&sig.doc, ui, doc_w)
                .min(3.0 * 17.0)
                + 6.0
        };
        let ph = 16.0 + 20.0 + doc_h;
        let above = ay - ph - 4.0;
        let y = if above > a.editor.y {
            above
        } else {
            ay + ch + 4.0
        };
        let x = ax.min(a.editor.right() - pw - 8.0).max(a.editor.x + 4.0);
        let r = Rect::new(x, y, pw, ph);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let row = Rect::new(r.x + 12.0, r.y + 8.0, r.w - 24.0 - count_w - 12.0, 20.0);
        let mut tx = row.x;
        for (k, part) in parts.iter().enumerate() {
            let (face, colour) = if k == 1 {
                (mono.bold(), p.bright)
            } else {
                (mono, p.text)
            };
            let w =
                self.text
                    .draw_in_row(&mut self.canvas, part, face, tx, row.y, row.h, colour, row);
            if k == 1 && !part.is_empty() {
                let under = Rect::new(tx, row.y + row.h - 2.0, w.min(row.right() - tx), 1.5);
                self.canvas.fill(under, p.accent);
            }
            tx += w;
        }
        self.text.draw_in_row(
            &mut self.canvas,
            &count,
            ui,
            r.right() - 12.0 - count_w,
            row.y,
            row.h,
            p.muted,
            r,
        );
        if !sig.doc.is_empty() {
            let clip = Rect::new(r.x + 12.0, row.bottom() + 4.0, doc_w, doc_h - 6.0);
            self.text.draw(
                &mut self.canvas,
                &sig.doc,
                ui,
                clip.x,
                clip.y,
                p.muted,
                clip,
                Some(doc_w),
            );
        }
    }

    /// A commit's card over the blame column, from `y` down: who and when,
    /// its hash, its message, its size, and Open Changes, Blame Before
    /// This and Copy Hash.
    pub fn blame_card(&mut self, card: &BlameCardView, text: Rect, y: f32) {
        let p = self.p;
        let face = Face::ui(12.5);
        let w = (text.w - 24.0).clamp(200.0, 470.0);
        let x = text.x + 12.0;
        let body_lines: Vec<String> = if card.body.is_empty() {
            Vec::new()
        } else {
            vec![card.body.replace('\n', " ")]
        };
        let body_h = if body_lines.is_empty() {
            0.0
        } else {
            self.text
                .wrapped_height(&body_lines[0], face, w - 28.0)
                .min(80.0)
                + 6.0
        };
        let h = 12.0 + 22.0 + 10.0 + 18.0 + body_h + 10.0 + 38.0;
        // It opens below the line, or above it near the bottom.
        let y = if y + h > text.bottom() && y - h > text.y {
            y - h - 22.0
        } else {
            y
        };
        let r = Rect::new(x, y, w, h);
        self.hit(r, Hit::Block);
        self.canvas.shadow(r, 10.0, false);
        self.canvas.outline(r, 10.0, 1.0, p.border, p.widget);
        let clip = r;
        // Who, when, and the hash.
        let top = r.y + 12.0;
        let av = Rect::new(r.x + 14.0, top, 22.0, 22.0);
        self.canvas.round(av, 11.0, mix(p.accent, p.widget, 0.3));
        let initial: String = card
            .author
            .chars()
            .take(1)
            .collect::<String>()
            .to_uppercase();
        self.centred(&initial, Face::ui(11.0).bold(), av, p.bright, clip);
        let aw = self.text.draw_in_row(
            &mut self.canvas,
            &card.author,
            face.bold(),
            av.right() + 8.0,
            top,
            22.0,
            p.bright,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &card.when,
            face,
            av.right() + 8.0 + aw + 8.0,
            top,
            22.0,
            p.muted,
            clip,
        );
        let mono = Face::mono(11.5);
        let hw = self.text.width(&card.hash, mono);
        self.text.draw_in_row(
            &mut self.canvas,
            &card.hash,
            mono,
            r.right() - 14.0 - hw,
            top,
            22.0,
            p.accent,
            clip,
        );
        // The summary, then the rest of the message.
        let mut ly = top + 22.0 + 10.0;
        let summary = self.text.fit(&card.summary, face.medium(), w - 28.0);
        self.text.draw_in_row(
            &mut self.canvas,
            &summary,
            face.medium(),
            r.x + 14.0,
            ly,
            18.0,
            p.bright,
            clip,
        );
        ly += 18.0;
        if let Some(body) = body_lines.first() {
            let inner = Rect::new(r.x + 14.0, ly + 4.0, w - 28.0, body_h - 6.0);
            self.text.draw(
                &mut self.canvas,
                body,
                face,
                inner.x,
                inner.y,
                p.muted,
                inner,
                Some(inner.w),
            );
            ly += body_h;
        }
        ly += 10.0;
        // A rule, the size, and the buttons.
        self.canvas
            .fill(Rect::new(r.x + 1.0, ly, w - 2.0, 1.0), p.border);
        let row = Rect::new(r.x, ly + 1.0, w, 37.0);
        if let Some((added, removed, files)) = card.stats {
            let small = Face::ui(11.5);
            let mut sx = r.x + 14.0;
            sx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("+{added}"),
                small,
                sx,
                row.y,
                row.h,
                p.success,
                clip,
            ) + 4.0;
            sx += self.text.draw_in_row(
                &mut self.canvas,
                &format!("−{removed}"),
                small,
                sx,
                row.y,
                row.h,
                p.error,
                clip,
            ) + 4.0;
            let s = if files == 1 { "" } else { "s" };
            self.text.draw_in_row(
                &mut self.canvas,
                &format!("in {files} file{s}"),
                small,
                sx,
                row.y,
                row.h,
                p.muted,
                clip,
            );
        }
        let mut bx = r.right() - 10.0;
        for (label, id) in [
            ("Copy Hash", "copy"),
            ("Blame Before This", "before"),
            ("Open Changes", "open"),
        ] {
            let bw = self.text.width(label, Face::ui(12.0)) + 18.0;
            bx -= bw;
            let b = Rect::new(bx, row.y + 7.5, bw, 22.0);
            let hovered = self.hovered(b);
            self.canvas.outline(
                b,
                6.0,
                1.0,
                p.border,
                if hovered { p.border } else { p.chip },
            );
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                Face::ui(12.0),
                b.x + 9.0,
                b.y,
                22.0,
                p.bright,
                clip,
            );
            self.hit(b, Hit::Ui(UiEvent::BlameButton(id.into())));
            bx -= 6.0;
        }
    }

    /// A card of commands with their keys at `at`, or up and left of it at
    /// the window's edges; a click on the `k`th item sends `event(k)`.
    pub(super) fn items_card(
        &mut self,
        items: &[ContextItem],
        selected: Option<usize>,
        (x, y): (f32, f32),
        w: f32,
        h: f32,
        event: fn(usize) -> UiEvent,
    ) {
        let p = self.p;
        let (label_f, key_f) = (Face::ui(13.0), Face::ui(12.0));
        let widest = items
            .iter()
            .map(|i| {
                self.text.width(&i.label, label_f)
                    + i.key
                        .as_ref()
                        .map_or(0.0, |k| self.text.width(k, key_f) + 32.0)
            })
            .fold(0.0, f32::max);
        let mw = (widest + 40.0).max(220.0);
        let height = |label: &str| if label.is_empty() { 9.0 } else { 28.0 };
        let mh: f32 = items.iter().map(|i| height(&i.label)).sum::<f32>() + 10.0;
        let x = if x + mw > w - 8.0 {
            (x - mw).max(8.0)
        } else {
            x
        };
        let y = if y + mh > h - 8.0 {
            (h - 8.0 - mh).max(8.0)
        } else {
            y
        };
        let r = Rect::new(x, y, mw, mh);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let mut ry = r.y + 5.0;
        for (k, item) in items.iter().enumerate() {
            let rh = height(&item.label);
            if item.label.is_empty() {
                self.canvas
                    .fill(Rect::new(r.x + 10.0, ry + 4.0, r.w - 20.0, 1.0), p.chip);
                ry += rh;
                continue;
            }
            let row = Rect::new(r.x + 5.0, ry, r.w - 10.0, rh);
            let on = selected == Some(k) || self.hovered(row);
            if on {
                self.canvas.round(row, 5.0, p.focus_row);
            }
            let colour = if on { p.bright } else { p.text };
            self.text.draw_in_row(
                &mut self.canvas,
                &item.label,
                label_f,
                row.x + 12.0,
                ry,
                rh,
                colour,
                row,
            );
            if let Some(key) = &item.key {
                let kw = self.text.width(key, key_f);
                self.text.draw_in_row(
                    &mut self.canvas,
                    key,
                    key_f,
                    row.right() - 12.0 - kw,
                    ry,
                    rh,
                    p.muted,
                    row,
                );
            }
            self.hit(row, Hit::Ui(event(k)));
            ry += rh;
        }
    }

    pub(super) fn context(&mut self, menu: &ContextView, at: (f32, f32), w: f32, h: f32) {
        // A click anywhere else closes it.
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::CloseContext));
        self.items_card(&menu.items, menu.selected, at, w, h, UiEvent::ContextItem);
    }

    /// The menus behind the title bar's button: their titles in one card,
    /// and the open one's items in another beside it.
    pub(super) fn menu(&mut self, menu: &MenuView, w: f32, h: f32) {
        let p = self.p;
        self.hit(Rect::new(0.0, 0.0, w, h), Hit::Ui(UiEvent::CloseMenu));
        let row_h = 28.0;
        let titles = Rect::new(
            10.0,
            TITLE_H - 4.0,
            180.0,
            menu.titles.len() as f32 * row_h + 10.0,
        );
        self.canvas.shadow(titles, 8.0, theme::is_light(&p));
        self.canvas.outline(titles, 8.0, 1.0, p.border, p.widget);
        self.hit(titles, Hit::Block);
        let face = Face::ui(13.0);
        let mut open_y = titles.y;
        for (i, title) in menu.titles.iter().enumerate() {
            let row = Rect::new(
                titles.x + 5.0,
                titles.y + 5.0 + i as f32 * row_h,
                titles.w - 10.0,
                row_h,
            );
            let open = i == menu.open;
            if open {
                self.canvas.round(row, 5.0, p.focus_row);
                open_y = row.y - 5.0;
            } else if self.hovered(row) {
                self.canvas.round(row, 5.0, p.hover);
            }
            let colour = if open { p.bright } else { p.text };
            self.text.draw_in_row(
                &mut self.canvas,
                title,
                face,
                row.x + 12.0,
                row.y,
                row_h,
                colour,
                row,
            );
            self.canvas
                .chevron(row.right() - 14.0, row.y + row_h / 2.0, 7.0, false, p.muted);
            self.hit(row, Hit::Ui(UiEvent::MenuTitle(i)));
        }
        let at = (titles.right() + 4.0, open_y);
        self.items_card(&menu.items, menu.selected, at, w, h, UiEvent::MenuItem);
    }

    /// A small label under `r`, or beside it on the rail.
    pub(super) fn tooltip(&mut self, r: Rect, tip: &str, w: f32) {
        let p = self.p;
        let face = Face::ui(12.0);
        let tw = self.text.width(tip, face) + 16.0;
        let (x, y) = if r.x < RAIL_W {
            (r.right() + 6.0, r.y + (r.h - 24.0) / 2.0)
        } else {
            (
                (r.x + r.w / 2.0 - tw / 2.0).clamp(6.0, w - tw - 6.0),
                r.bottom() + 6.0,
            )
        };
        let b = Rect::new(x, y, tw, 24.0);
        self.canvas.shadow(b, 5.0, theme::is_light(&p));
        self.canvas.outline(b, 5.0, 1.0, p.border, p.widget);
        self.text
            .draw_in_row(&mut self.canvas, tip, face, b.x + 8.0, b.y, b.h, p.text, b);
    }

    /// A question over the dimmed window: what it's about, why it
    /// matters, and the buttons, the first the main one, right-aligned as
    /// a desktop's dialogs have them. Nothing else takes a click.
    pub(super) fn dialog(&mut self, dv: &DialogView, w: f32, h: f32) {
        let p = self.p;
        let all = Rect::new(0.0, 0.0, w, h);
        self.canvas.tint(all, 0.0, p.ground, 150);
        self.hit(all, Hit::Block);
        let dw = 460.0_f32.min(w - 32.0);
        let inner = dw - 48.0;
        let title_face = Face::ui(15.0).medium();
        let detail_face = Face::ui(13.0);
        let title_h = self.text.wrapped_height(&dv.title, title_face, inner);
        let detail_h = self.text.wrapped_height(&dv.detail, detail_face, inner);
        let dh = 24.0 + title_h + 10.0 + detail_h + 28.0 + 30.0 + 22.0;
        let r = Rect::new((w - dw) / 2.0, ((h - dh) / 2.0 - 40.0).max(16.0), dw, dh);
        self.canvas.shadow(r, 12.0, theme::is_light(&p));
        self.canvas.outline(r, 12.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let (x, mut y) = (r.x + 24.0, r.y + 24.0);
        self.text.draw(
            &mut self.canvas,
            &dv.title,
            title_face,
            x,
            y,
            p.bright,
            r,
            Some(inner),
        );
        y += title_h + 10.0;
        self.text.draw(
            &mut self.canvas,
            &dv.detail,
            detail_face,
            x,
            y,
            p.muted,
            r,
            Some(inner),
        );
        // The buttons, right to left from the edge, so the main one ends
        // up on the left of the row as GNOME and VS Code put it.
        let face = Face::ui(13.0).medium();
        let widths: Vec<f32> = dv
            .buttons
            .iter()
            .map(|b| self.text.width(b, face) + 28.0)
            .collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0 * (widths.len().saturating_sub(1)) as f32;
        let mut bx = r.right() - 24.0 - total;
        let by = r.bottom() - 22.0 - 30.0;
        for (i, label) in dv.buttons.iter().enumerate() {
            let bw = self.button(label, bx, by, i == dv.selected, i == dv.primary);
            self.hit(
                Rect::new(bx, by, bw, 30.0),
                Hit::Ui(UiEvent::DialogButton(i)),
            );
            bx += bw + 8.0;
        }
    }
}
