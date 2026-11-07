//! The merge editor and side-by-side compare.

use super::*;

impl Frame<'_> {
    pub(super) fn merge_bar(&mut self, bar: &MergeBarView, row: Rect, right: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(12.0);
        let h = 24.0;
        let y = row.y + (row.h - h) / 2.0;
        let mut x = right - 6.0;
        // A file with markers opened as text: only the way in.
        let buttons: &[(&str, &'static str, bool)] = if bar.offer {
            &[("Open in Merge Editor", "merge.open_here", true)]
        } else if bar.inline {
            &[("Complete", "merge.complete", true)]
        } else {
            &[
                ("Complete Merge", "merge.complete", true),
                ("Accept All Incoming", "merge.accept_all_incoming", false),
            ]
        };
        for &(label, command, primary) in buttons {
            let face = if primary { face.medium() } else { face };
            let w = self.text.width(label, face) + 20.0;
            x -= w;
            let r = Rect::new(x, y, w, h);
            let hovered = self.hovered(r);
            let (bg, fg) = match (primary, hovered) {
                (true, false) => (p.accent, p.on_accent),
                (true, true) => (mix(p.accent, 0xffffff, 0.88), p.on_accent),
                (false, false) => (p.chip, p.bright),
                (false, true) => (p.border, p.bright),
            };
            self.canvas.round(r, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, h, fg, r);
            self.hit(r, Hit::Command(command));
            x -= 6.0;
        }
        if bar.offer {
            return right - x;
        }
        x -= 4.0;
        for (glyph, command) in [("↓", "merge.next"), ("↑", "merge.prev")] {
            x -= 20.0;
            let r = Rect::new(x, y, 20.0, h);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            self.centred(glyph, face, r, p.muted, r);
            self.hit(r, Hit::Command(command));
        }
        let (label, colour) = match (bar.left, bar.inline) {
            (0, _) => ("No conflicts left".to_string(), p.success),
            (n, true) => (format!("{n} left"), p.error),
            (1, false) => ("1 conflict left".to_string(), p.error),
            (n, false) => (format!("{n} conflicts left"), p.error),
        };
        let lw = self.text.width(&label, face);
        x -= lw + 10.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            y,
            h,
            colour,
            Rect::new(x, y, lw + 1.0, h),
        );
        right - x
    }

    /// The merge editor around the Result's code: Incoming and Current,
    /// each line of a conflict with its box and Accept under it, the
    /// panes' titles, and the Result's notes under its conflicts, from
    /// `y0`, the Result's first row.
    pub fn merge_editor(
        &mut self,
        merge: &MergeView,
        area: &GroupArea,
        y0: f32,
        row_h: f32,
        gutter: f32,
    ) {
        let p = self.p;
        let Some([incoming, current, result]) = area.merge else {
            return;
        };
        for (pane, r) in merge.panes.iter().zip([incoming, current]) {
            self.merge_pane(pane, r, row_h);
        }
        // The Result's frame and title, around the code drawn already.
        self.canvas.frame(result, 8.0, 1.0, p.line, 0);
        let head = Rect::new(
            result.x + 1.0,
            result.y + 1.0,
            result.w - 2.0,
            MERGE_HEAD_H - 1.0,
        );
        self.canvas.fill(head, p.widget);
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.line);
        let face = Face::ui(12.0);
        let mut x = head.x + 10.0;
        x += self.text.draw_in_row(
            &mut self.canvas,
            "Result",
            face.medium(),
            x,
            head.y,
            head.h,
            p.bright,
            head,
        ) + 8.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &if merge.inline {
                "with each conflict in place".to_string()
            } else {
                format!("{} · you can edit here", merge.name)
            },
            face,
            x,
            head.y,
            head.h,
            p.muted,
            head,
        );
        let hint = "Base shows through until you choose";
        let hw = self.text.width(hint, face);
        if head.w > 520.0 && !merge.inline {
            self.text.draw_in_row(
                &mut self.canvas,
                hint,
                face,
                head.right() - 10.0 - hw,
                head.y,
                head.h,
                p.muted,
                head,
            );
        }
        // Each conflict's note and links, in the row under it.
        let code = area.text;
        let link = Face::ui(11.5);
        let clip = Rect::new(code.x, code.y, code.w, code.h);
        let text_x = code.x + gutter;
        for note in &merge.notes {
            let y = y0 + note.row as f32 * row_h;
            let rows = 1 + note.sides.len();
            if y + rows as f32 * row_h <= code.y || y >= code.bottom() {
                continue;
            }
            self.canvas.fill(
                Rect::new(code.x, y, code.w, rows as f32 * row_h).intersect(clip),
                p.island,
            );
            let mut x = text_x;
            if !note.text.is_empty() {
                x += self.text.draw_in_row(
                    &mut self.canvas,
                    &note.text,
                    link,
                    x,
                    y,
                    row_h,
                    p.muted,
                    clip,
                ) + 6.0;
            }
            for (k, (label, id)) in note.links.iter().enumerate() {
                if k > 0 || !note.text.is_empty() {
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        "·",
                        link,
                        x,
                        y,
                        row_h,
                        p.muted,
                        clip,
                    ) + 6.0;
                }
                let w = self.text.width(label, link);
                let r = Rect::new(x - 2.0, y + 2.0, w + 4.0, row_h - 4.0);
                if self.hovered(r) {
                    self.canvas.round(r, 4.0, p.hover);
                }
                // Side by side changes the view, not the text: muted.
                let colour = if id.starts_with("merge:side") {
                    p.muted
                } else {
                    p.accent
                };
                self.text
                    .draw_in_row(&mut self.canvas, label, link, x, y, row_h, colour, clip);
                self.marked_hit(
                    r.intersect(clip),
                    Hit::Ui(UiEvent::Press(id.clone())),
                    Mark::Link,
                );
                x += w + 6.0;
            }
            // In the Result alone: both sides under their titles.
            for (k, row) in note.sides.iter().enumerate() {
                let ry = y + (k + 1) as f32 * row_h;
                if ry >= code.bottom() {
                    break;
                }
                let line = Rect::new(code.x, ry, code.w, row_h).intersect(clip);
                match row {
                    MergeRow::Title(side, label) => {
                        let (who, edge) = match side {
                            MergeSide::Incoming => ("Incoming", p.git_changed),
                            MergeSide::Current => ("Current", p.success),
                        };
                        self.canvas.fill(line, mix(edge, p.island, 0.10));
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &format!("{who} · {label}"),
                            Face::ui(11.0),
                            text_x,
                            ry,
                            row_h,
                            mix(edge, p.bright, 0.7),
                            clip,
                        );
                    }
                    MergeRow::Line {
                        number,
                        text,
                        colours,
                        ..
                    } => {
                        let incoming = note.sides[..k]
                            .iter()
                            .rfind(|r| matches!(r, MergeRow::Title(..)))
                            .is_some_and(|r| matches!(r, MergeRow::Title(MergeSide::Incoming, _)));
                        let edge = if incoming { p.git_changed } else { p.success };
                        let tint = if incoming { 0.16 } else { 0.15 };
                        self.canvas.fill(line, mix(edge, p.island, tint));
                        self.canvas.fill(
                            Rect::new(text_x - 6.0, ry, 3.0, row_h).intersect(clip),
                            edge,
                        );
                        if incoming {
                            let n = number.to_string();
                            let nw = self.text.width(&n, CODE);
                            self.text.draw_in_row(
                                &mut self.canvas,
                                &n,
                                CODE,
                                text_x - 14.0 - nw,
                                ry,
                                row_h,
                                p.faint,
                                clip,
                            );
                        }
                        self.code_line(text, colours, text_x, ry, row_h, clip);
                    }
                    MergeRow::Accept(_) => {}
                }
            }
        }
    }

    /// Two compared files' titles, over the halves of the code at `halves`.
    pub fn compare_titles(&mut self, titles: &[(String, String); 2], halves: &[Rect]) {
        let p = self.p;
        // Inline, one view: the right file's title over it all.
        let pairs: Vec<(&(String, String), Rect)> = match halves {
            [one] => vec![(&titles[1], *one)],
            _ => titles.iter().zip(halves.iter().copied()).collect(),
        };
        for ((name, note), r) in pairs {
            let head = Rect::new(r.x, r.y - MERGE_HEAD_H, r.w, MERGE_HEAD_H - 4.0);
            self.canvas.round(head, 6.0, p.widget);
            let face = Face::ui(12.0);
            let x = head.x + 10.0;
            let w = self.text.draw_in_row(
                &mut self.canvas,
                name,
                face.medium(),
                x,
                head.y,
                head.h,
                p.bright,
                head,
            );
            self.text.draw_in_row(
                &mut self.canvas,
                note,
                face,
                x + w + 8.0,
                head.y,
                head.h,
                p.muted,
                head,
            );
        }
    }

    /// Incoming or Current: its title, then its lines from the one level
    /// with the Result's top.
    pub(super) fn merge_pane(&mut self, pane: &merlin_core::MergePaneView, r: Rect, row_h: f32) {
        let p = self.p;
        self.hit(r, Hit::Block);
        self.canvas.outline(r, 8.0, 1.0, p.line, p.island);
        let head = Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, MERGE_HEAD_H - 1.0);
        self.canvas.fill(head, p.widget);
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.line);
        let face = Face::ui(12.0);
        let (who, tint, edge, accept, key) = match pane.side {
            MergeSide::Incoming => (
                "Incoming",
                mix(p.git_changed, p.island, 0.16),
                p.git_changed,
                "Accept Incoming",
                "i",
            ),
            MergeSide::Current => (
                "Current",
                mix(p.success, p.island, 0.15),
                p.success,
                "Accept Current",
                "c",
            ),
        };
        let x = head.x + 10.0;
        let w = self.text.draw_in_row(
            &mut self.canvas,
            who,
            face.medium(),
            x,
            head.y,
            head.h,
            p.bright,
            head,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &pane.detail,
            face,
            x + w + 8.0,
            head.y,
            head.h,
            p.muted,
            head,
        );
        let code = Rect::new(
            r.x + 1.0,
            head.bottom() + 1.0,
            r.w - 2.0,
            r.bottom() - head.bottom() - 2.0,
        );
        let mono = CODE;
        let digits = pane
            .rows
            .iter()
            .filter_map(|row| match row {
                MergeRow::Line { number, .. } => Some(number.to_string().len()),
                _ => None,
            })
            .max()
            .unwrap_or(1)
            .max(2);
        let cw = self.text.width("0", mono);
        let num_w = digits as f32 * cw + 14.0;
        let box_x = code.x + num_w;
        let text_x = box_x + 22.0;
        let mut y = code.y + 4.0;
        for row in &pane.rows {
            if y >= code.bottom() {
                break;
            }
            let line = Rect::new(code.x, y, code.w, row_h);
            match row {
                MergeRow::Line {
                    number,
                    text,
                    colours,
                    conflict,
                } => {
                    if conflict.is_some() {
                        self.canvas.fill(line.intersect(code), tint);
                        self.canvas
                            .fill(Rect::new(code.x, y, 3.0, row_h).intersect(code), edge);
                    }
                    let n = number.to_string();
                    let nw = self.text.width(&n, mono);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &n,
                        mono,
                        box_x - 10.0 - nw,
                        y,
                        row_h,
                        p.faint,
                        code,
                    );
                    if let Some((i, k, ticked)) = conflict {
                        let b = Rect::new(box_x, y + (row_h - 14.0) / 2.0, 14.0, 14.0);
                        let hovered = self.hovered(b);
                        if *ticked {
                            self.canvas.round(b, 3.0, p.accent);
                            self.centred("✓", Face::ui(11.0), b, p.on_accent, code);
                        } else {
                            let border = if hovered { p.accent } else { p.faint };
                            self.canvas.outline(b, 3.0, 1.0, border, p.island);
                        }
                        let id = format!("merge:box:{key}:{i}:{k}");
                        self.hit(line.intersect(code), Hit::Ui(UiEvent::Press(id)));
                    }
                    self.code_line(text, colours, text_x, y, row_h, code);
                }
                MergeRow::Title(..) => {}
                MergeRow::Accept(i) => {
                    let link = Face::ui(11.5);
                    let w = self.text.width(accept, link);
                    let r = Rect::new(text_x - 2.0, y + 2.0, w + 4.0, row_h - 4.0);
                    if self.hovered(r) {
                        self.canvas.round(r, 4.0, p.hover);
                    }
                    self.text.draw_in_row(
                        &mut self.canvas,
                        accept,
                        link,
                        text_x,
                        y,
                        row_h,
                        p.accent,
                        code,
                    );
                    let what = if key == "i" { "incoming" } else { "current" };
                    self.marked_hit(
                        r.intersect(code),
                        Hit::Ui(UiEvent::Press(format!("merge:{what}:{i}"))),
                        Mark::Link,
                    );
                }
            }
            y += row_h;
        }
    }

    pub(super) fn diff_bar(&mut self, bar: &DiffBarView, row: Rect, right: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(12.0);
        let h = 22.0;
        let y = row.y + (row.h - h) / 2.0;
        let mut x = right - 6.0;
        let button = |f: &mut Self, label: &str, command: &'static str, x: &mut f32| {
            let w = f.text.width(label, face) + 18.0;
            *x -= w;
            let r = Rect::new(*x, y, w, h);
            let hovered = f.hovered(r);
            f.canvas
                .round(r, 6.0, if hovered { p.border } else { p.chip });
            f.text
                .draw_in_row(&mut f.canvas, label, face, r.x + 9.0, y, h, p.bright, r);
            f.hit(r, Hit::Command(command));
            *x -= 6.0;
        };
        // A narrow strip keeps its room for the tabs; the file's buttons
        // are in Source Control too.
        if bar.compare {
            if bar.swap {
                button(self, "⇄ Swap", "compare.swap", &mut x);
            }
        } else if row.w >= 900.0 {
            button(self, "Revert File", "diff.revert_file", &mut x);
            button(self, "Stage File", "diff.stage_file", &mut x);
        }
        // Side by side or inline, the one shown lit.
        let segs = [("Inline", true), ("Side by side", false)];
        let widths: Vec<f32> = segs
            .iter()
            .map(|(l, _)| self.text.width(l, face) + 20.0)
            .collect();
        let total: f32 = widths.iter().sum();
        x -= total;
        let seg = Rect::new(x, y, total, h);
        self.canvas.frame(seg, 6.0, 1.0, p.border, 0);
        let mut sx = seg.right();
        for ((label, inline), w) in segs.iter().zip(&widths) {
            sx -= w;
            let r = Rect::new(sx, y, *w, h);
            let on = bar.inline == *inline;
            if on {
                self.canvas.round(r.inset(1.0), 5.0, p.raised);
            } else {
                self.hit(r, Hit::Command("diff.inline"));
            }
            let colour = if on { p.bright } else { p.muted };
            self.text
                .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, h, colour, r);
        }
        x -= 10.0;
        // The arrows, then which change it is.
        for (glyph, command) in [("↓", "diff.next"), ("↑", "diff.prev")] {
            x -= 20.0;
            let r = Rect::new(x, y, 20.0, h);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.hover);
            }
            self.centred(glyph, face, r, p.muted, r);
            self.hit(r, Hit::Command(command));
        }
        // A note such as "Moved from x.rs, no changes" says it already.
        let label = match bar.changes {
            0 if bar.note.is_some() => String::new(),
            0 => "No changes".to_string(),
            n => format!("Change {} of {n}", bar.change.max(1)),
        };
        let lw = self.text.width(&label, face);
        x -= lw + 6.0;
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x,
            y,
            h,
            p.text,
            Rect::new(x, y, lw + 1.0, h),
        );
        // What kind of change, when it isn't plain edits, while there's
        // room for the tabs.
        if let Some(note) = &bar.note {
            let nw = self.text.width(note, face);
            if right - x + nw + 16.0 < row.w - 260.0 {
                x -= nw + 14.0;
                self.text.draw_in_row(
                    &mut self.canvas,
                    note,
                    face,
                    x,
                    y,
                    h,
                    p.muted,
                    Rect::new(x, y, nw + 1.0, h),
                );
            }
        }
        right - x
    }

    /// A changes tab's boxes and « buttons in the gap between its halves,
    /// each changed row's box before the new side's numbers, and the rows
    /// with no line hatched. `text` is the group's text area, and `y0` the
    /// top of the view's first row.
    pub fn diff_marks(&mut self, view: &TextView, text: Rect, cell_w: f32, row_h: f32, y0: f32) {
        let Some(marks) = &view.diff else { return };
        let p = self.p;
        let col_x = |c: u16| text.x + c as f32 * cell_w;
        let row_y = |r: i32| y0 + r as f32 * row_h;
        let side = Rect::new(
            col_x(view.area.x),
            text.y,
            view.area.w as f32 * cell_w,
            text.h,
        );
        // Hatching where a side has no line.
        for &r in &marks.fillers {
            let row = Rect::new(side.x, row_y(r), side.w, row_h);
            if row.bottom() <= text.y || row.y >= text.bottom() {
                continue;
            }
            self.canvas.fill(row, p.island);
            let colour = mix(p.line, p.island, 0.6);
            let mut x0 = row.x - row.h;
            while x0 < row.right() {
                let (mut a, mut b) = ((x0, row.bottom()), (x0 + row.h, row.y));
                if a.0 < row.x {
                    a = (row.x, row.bottom() - (row.x - a.0));
                }
                if b.0 > row.right() {
                    b = (row.right(), row.y + (b.0 - row.right()));
                }
                self.canvas.line(a, b, 1.0, colour);
                x0 += 6.0;
            }
        }
        let gap = Rect::new(
            col_x(marks.gap.0),
            text.y,
            marks.gap.1 as f32 * cell_w,
            text.h,
        );
        for m in &marks.rows {
            let y = row_y(m.row);
            if y + row_h <= text.y || y >= text.bottom() {
                continue;
            }
            let mid = y + row_h / 2.0;
            if let Some((index, check, rows)) = m.change {
                // The change's band across the gap, its « and its box.
                let band = Rect::new(gap.x, y, gap.w, rows as f32 * row_h);
                let band = Rect::new(band.x, band.y.max(text.y), band.w, band.h);
                self.canvas.fill(band, mix(p.accent, p.island, 0.1));
                let revert = Rect::new(gap.x + gap.w / 2.0 - 24.0, mid - 9.0, 22.0, 18.0);
                let hovered = self.hovered(revert);
                self.canvas
                    .round(revert, 4.0, if hovered { p.border } else { p.chip });
                let face = Face::ui(12.0);
                self.centred("«", face, revert, p.bright, revert);
                self.tips
                    .push((revert, "Revert to the last commit (Alt+R)".into()));
                self.hit(revert, Hit::Ui(UiEvent::DiffRevert(index)));
                let bx = gap.x + gap.w / 2.0 + 4.0;
                self.checkbox(check, bx, mid);
                let b = Rect::new(bx - 3.0, y, 20.0, row_h);
                self.tips.push((b, "Stage this change (Alt+S)".into()));
                self.hit(b, Hit::Ui(UiEvent::DiffStage(index)));
            }
            if let Some((line, staged)) = m.line {
                let x = col_x(view.area.x) + cell_w * 0.5;
                // A staged row has a blue edge.
                if staged {
                    self.canvas
                        .fill(Rect::new(col_x(view.area.x), y, 3.0, row_h), p.accent);
                }
                let check = if staged { Check::On } else { Check::Off };
                self.checkbox(check, x, mid);
                self.hit(
                    Rect::new(x - 3.0, y, 20.0, row_h),
                    Hit::Ui(UiEvent::DiffLine(line)),
                );
            }
        }
    }
}
