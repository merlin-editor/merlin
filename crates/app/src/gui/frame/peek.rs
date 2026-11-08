//! The views that open between lines: a change from its gutter bar, and
//! the references peek.

use super::*;

impl Frame<'_> {
    /// A change opened in place from its gutter bar, in the `rows` after
    /// screen position `y`: which change, Stage, Revert, the arrows and
    /// close, then what the lines were and are.
    pub fn change_peek(&mut self, change: &ChangeView, text: Rect, y: f32, ch: f32, rows: usize) {
        let p = self.p;
        let r = Rect::new(
            text.x + 36.0,
            y + 2.0,
            (text.w - 36.0 - 16.0).max(200.0),
            rows as f32 * ch - 4.0,
        );
        let clip = Rect::new(text.x, text.y, text.w, text.h);
        if r.bottom() < clip.y || r.y > clip.bottom() {
            return;
        }
        let (inner, clip, head) = self.peek_card(r, p.border, clip);
        let face = Face::ui(12.0);
        let title = format!("Change {} of {}", change.index, change.count);
        let tw = self.text.draw_in_row(
            &mut self.canvas,
            &title,
            face,
            head.x + 10.0,
            head.y,
            30.0,
            p.text,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            "· since the last commit",
            face,
            head.x + 10.0 + tw + 8.0,
            head.y,
            30.0,
            p.muted,
            clip,
        );
        // Close, the arrows, then Revert and Stage, from the right.
        let mut x = head.right() - 8.0;
        for (glyph, event, tip) in [
            ("✕", UiEvent::ChangeClose, "Close (Esc)"),
            ("↓", UiEvent::ChangeStep(true), "Next Change (Alt+F5)"),
            ("↑", UiEvent::ChangeStep(false), "Previous Change (Alt+F3)"),
        ] {
            x -= 22.0;
            let b = Rect::new(x, head.y + 4.0, 22.0, 22.0);
            let hovered = self.hovered(b);
            if hovered {
                self.canvas.round(b, 5.0, p.hover);
            }
            self.centred(
                glyph,
                face,
                b,
                if hovered { p.bright } else { p.muted },
                clip,
            );
            self.tips.push((b, tip.into()));
            self.hit(b, Hit::Ui(event));
        }
        x -= 6.0;
        let stage = if change.staged { "Unstage" } else { "Stage" };
        for (label, event) in [
            ("Revert", UiEvent::ChangeRevert),
            (stage, UiEvent::ChangeStage),
        ] {
            let w = self.text.width(label, face) + 18.0;
            x -= w;
            let b = Rect::new(x, head.y + 4.0, w, 22.0);
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
                face,
                b.x + 9.0,
                b.y,
                22.0,
                p.bright,
                clip,
            );
            self.hit(b, Hit::Ui(event));
            x -= 6.0;
        }
        // The lines: what they were in red, what they are in green.
        let mono = Face::mono(13.0);
        let mut ly = head.bottom() + 1.0;
        for (sign, line) in &change.lines {
            if ly + ch > inner.bottom() + 0.5 {
                break;
            }
            let (band, colour) = if *sign == '-' {
                (mix(p.error, p.island, 0.16), p.error)
            } else {
                (mix(p.success, p.island, 0.14), p.success)
            };
            let row = Rect::new(inner.x, ly, inner.w, ch);
            self.canvas.fill(row, band);
            let sw = self.text.width(&sign.to_string(), mono);
            self.text.draw_in_row(
                &mut self.canvas,
                &sign.to_string(),
                mono,
                inner.x + 24.0 - sw / 2.0,
                ly,
                ch,
                colour,
                clip,
            );
            let room = inner.right() - (inner.x + 44.0) - 8.0;
            let fitted = self.text.fit(line, mono, room);
            self.text.draw_in_row(
                &mut self.canvas,
                &fitted,
                mono,
                inner.x + 44.0,
                ly,
                ch,
                p.text,
                clip,
            );
            ly += ch;
        }
        if change.more > 0 && ly + ch <= inner.bottom() + 0.5 {
            let more = format!("{} more lines", change.more);
            self.text.draw_in_row(
                &mut self.canvas,
                &more,
                face,
                inner.x + 44.0,
                ly,
                ch,
                p.muted,
                clip,
            );
        }
    }

    /// The references peek view, in the rows `text` opens for it from `y`:
    /// a header, the chosen reference's lines on the left, and every
    /// reference by file on the right.
    pub fn peek(&mut self, peek: &PeekView, text: Rect, y: f32, ch: f32) {
        let p = self.p;
        let r = Rect::new(
            text.x + 40.0,
            y + 4.0,
            (text.w - 40.0 - 16.0).max(200.0),
            peek.rows as f32 * ch - 8.0,
        );
        let clip = Rect::new(text.x, text.y, text.w, text.h);
        if r.bottom() < clip.y || r.y > clip.bottom() {
            return;
        }
        let border = if peek.focused { p.accent } else { p.border };
        // The header: the name, how many, and its buttons.
        let (inner, clip, head) = self.peek_card(r, border, clip);
        self.peek_header(peek, head, clip);
        // The list, on the right.
        let list_w = (inner.w * 0.32).clamp(220.0, 330.0);
        let body = Rect::new(
            inner.x,
            head.bottom() + 1.0,
            inner.w,
            inner.bottom() - head.bottom() - 1.0,
        );
        let list = Rect::new(body.right() - list_w, body.y, list_w, body.h);
        let code = Rect::new(body.x, body.y, body.w - list_w - 1.0, body.h);
        self.canvas.fill(code, p.island);
        self.canvas
            .fill(Rect::new(list.x - 1.0, list.y, 1.0, list.h), p.border);
        self.peek_preview(peek, code, clip, ch);
        self.peek_list(peek, list, clip);
    }

    /// The references peek's header: the name, how many references in how
    /// many files, and its buttons.
    fn peek_header(&mut self, peek: &PeekView, head: Rect, clip: Rect) {
        let p = self.p;
        let bold = Face::ui(12.5).bold();
        let nw = self.text.draw_in_row(
            &mut self.canvas,
            &peek.name,
            bold,
            head.x + 12.0,
            head.y,
            30.0,
            p.bright,
            clip,
        );
        self.text.draw_in_row(
            &mut self.canvas,
            &peek.summary,
            Face::ui(12.5),
            head.x + 12.0 + nw + 10.0,
            head.y,
            30.0,
            p.muted,
            clip,
        );
        let close = Rect::new(head.right() - 30.0, head.y + 3.0, 24.0, 24.0);
        let search = Rect::new(close.x - 26.0, close.y, 24.0, 24.0);
        for (b, event) in [(search, UiEvent::PeekToSearch), (close, UiEvent::PeekClose)] {
            let hovered = self.hovered(b);
            if hovered {
                self.canvas.round(b, 5.0, p.hover);
            }
            let c = if hovered { p.bright } else { p.muted };
            let (cx, cy) = (b.x + 12.0, b.y + 12.0);
            if event == UiEvent::PeekClose {
                self.canvas
                    .line((cx - 3.5, cy - 3.5), (cx + 3.5, cy + 3.5), 1.5, c);
                self.canvas
                    .line((cx + 3.5, cy - 3.5), (cx - 3.5, cy + 3.5), 1.5, c);
            } else {
                self.canvas
                    .frame(Rect::new(cx - 5.0, cy - 5.0, 10.0, 10.0), 1.5, 1.3, c, 0);
                self.canvas
                    .line((cx - 2.0, cy - 5.0), (cx - 2.0, cy + 5.0), 1.3, c);
            }
            self.hit(b, Hit::Ui(event));
        }
    }

    /// The chosen reference's lines, in the peek's left part, without
    /// their shared indent and slid left so the reference shows.
    fn peek_preview(&mut self, peek: &PeekView, code: Rect, clip: Rect, ch: f32) {
        let p = self.p;
        let code_clip = Rect::new(
            code.x,
            clip.y.max(code.y),
            code.w,
            clip.bottom().min(code.bottom()) - clip.y.max(code.y),
        );
        let mono = Face::mono(13.0);
        let cw = self.text.width("0", mono);
        let x = code.x + 60.0;
        let end = code_clip.right() - 12.0;
        // The lines' shared indent is left out, and long lines slide left
        // so the chosen reference shows.
        let indent = peek
            .preview
            .iter()
            .filter(|l| !l.text.trim().is_empty())
            .map(|l| l.text.chars().take_while(|c| c.is_whitespace()).count())
            .min()
            .unwrap_or(0);
        let room = ((end - x) / cw).max(1.0) as usize;
        let skip = peek
            .preview
            .iter()
            .find(|l| l.current)
            .and_then(|l| l.marks.first())
            .filter(|&&(_, b)| b.saturating_sub(indent) > room)
            .map_or(indent, |&(a, _)| a.saturating_sub(room / 3).max(indent));
        let mut ly = code.y + 6.0;
        for line in &peek.preview {
            if ly + ch > code.bottom() {
                break;
            }
            let number = line.number.to_string();
            let nw = self.text.width(&number, mono);
            self.text.draw_in_row(
                &mut self.canvas,
                &number,
                mono,
                code.x + 40.0 - nw,
                ly,
                ch,
                p.muted,
                code_clip,
            );
            let chars: Vec<char> = line.text.chars().skip(skip).collect();
            let shift = |(a, b): (usize, usize)| (a.saturating_sub(skip), b.saturating_sub(skip));
            for &mark in &line.marks {
                let (a, b) = shift(mark);
                if b == 0 {
                    continue;
                }
                let before: String = chars[..a.min(chars.len())].iter().collect();
                let word: String = chars[a.min(chars.len())..b.min(chars.len())]
                    .iter()
                    .collect();
                let mx = x + self.text.width(&before, mono);
                let mw = self.text.width(&word, mono).max(cw).min(end - mx);
                if mw > 0.0 {
                    self.canvas
                        .round(Rect::new(mx, ly + 2.0, mw, ch - 4.0), 2.0, p.find_match);
                }
            }
            let text: String = chars.iter().collect();
            let colours: Spans = line
                .colours
                .iter()
                .map(|&(a, b, k)| {
                    let (a, b) = shift((a, b));
                    (a, b, k)
                })
                .filter(|&(a, b, _)| b > a)
                .collect();
            self.code_line_in(&text, &colours, x, ly, ch, code_clip, mono);
            ly += ch;
        }
    }

    /// Files and their references, in the peek's right part.
    fn peek_list(&mut self, peek: &PeekView, list: Rect, clip: Rect) {
        let p = self.p;
        let list_clip = Rect::new(
            list.x,
            clip.y.max(list.y),
            list.w,
            clip.bottom().min(list.bottom()) - clip.y.max(list.y),
        );
        let mut iy = list.y + 4.0;
        for (i, item) in peek.items.iter().enumerate() {
            let h = if matches!(item, PeekItem::File { .. }) {
                26.0
            } else {
                24.0
            };
            if iy + h > list.bottom() {
                break;
            }
            let row = Rect::new(list.x, iy, list.w, h);
            if peek.selected == Some(i) {
                self.canvas.fill(
                    row,
                    if peek.focused {
                        p.focus_row
                    } else {
                        p.selected
                    },
                );
            } else if self.hovered(row) {
                self.canvas.fill(row, p.hover);
            }
            match item {
                PeekItem::File {
                    name,
                    icon,
                    folder,
                    count,
                    open,
                } => {
                    let mid = iy + h / 2.0;
                    self.canvas.chevron(row.x + 15.0, mid, 7.0, *open, p.muted);
                    let mut x = row.x + 24.0;
                    x += self.file_icon(name, Some(*icon), x, mid - 8.0, 16.0, list_clip);
                    let face = Face::ui(12.5);
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        name,
                        face,
                        x,
                        iy,
                        h,
                        p.text,
                        list_clip,
                    );
                    let badge = count.to_string();
                    let bf = Face::ui(11.0);
                    let bw = self.text.width(&badge, bf) + 12.0;
                    let pill = Rect::new(row.right() - 10.0 - bw, mid - 8.0, bw, 16.0);
                    let room = (pill.x - x - 14.0).max(0.0);
                    let folder = self.text.fit(folder, Face::ui(11.5), room);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &folder,
                        Face::ui(11.5),
                        x + 6.0,
                        iy,
                        h,
                        p.muted,
                        list_clip,
                    );
                    self.canvas.round(pill, 8.0, p.chip);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &badge,
                        bf,
                        pill.x + 6.0,
                        pill.y,
                        16.0,
                        p.muted,
                        list_clip,
                    );
                }
                PeekItem::Place { line, text, mark } => {
                    let face = Face::mono(12.0);
                    let colour = if peek.selected == Some(i) {
                        p.bright
                    } else {
                        p.text
                    };
                    let number = format!("{line}  ");
                    let x = row.x + 26.0;
                    let nw = self.text.draw_in_row(
                        &mut self.canvas,
                        &number,
                        face,
                        x,
                        iy,
                        h,
                        p.muted,
                        list_clip,
                    );
                    let end = row.right() - 10.0;
                    let chars: Vec<char> = text.chars().collect();
                    let (a, b) = (mark.0.min(chars.len()), mark.1.min(chars.len()));
                    let before: String = chars[..a].iter().collect();
                    let word: String = chars[a..b].iter().collect();
                    let mx = x + nw + self.text.width(&before, face);
                    let mw = self.text.width(&word, face).min(end - mx);
                    if mw > 0.0 {
                        self.canvas
                            .round(Rect::new(mx, iy + 4.0, mw, h - 8.0), 2.0, p.find_match);
                    }
                    let shown = self.text.fit(text, face, (end - x - nw).max(0.0));
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &shown,
                        face,
                        x + nw,
                        iy,
                        h,
                        colour,
                        list_clip,
                    );
                }
            }
            self.hit(
                row,
                Hit::Ui(UiEvent::PeekRow {
                    index: i,
                    clicks: 1,
                }),
            );
            iy += h;
        }
    }

    /// A peek's card over `r`, outlined in `border`, which the text behind
    /// takes no clicks through: its inside, `clip` cut to that, and its
    /// 30-pixel header.
    fn peek_card(&mut self, r: Rect, border: u32, clip: Rect) -> (Rect, Rect, Rect) {
        let p = self.p;
        self.hit(r, Hit::Block);
        self.canvas
            .outline(r, 8.0, 1.0, border, mix(p.ground, p.island, 0.5));
        let inner = Rect::new(r.x + 1.0, r.y + 1.0, r.w - 2.0, r.h - 2.0);
        let clip = Rect::new(
            inner.x,
            inner.y.max(clip.y),
            inner.w,
            (inner.bottom().min(clip.bottom()) - inner.y.max(clip.y)).max(0.0),
        );
        let head = Rect::new(inner.x, inner.y, inner.w, 30.0);
        self.canvas.round(head, 7.0, p.widget);
        self.canvas.fill(
            Rect::new(head.x, head.bottom() - 7.0, head.w, 7.0),
            p.widget,
        );
        self.canvas
            .fill(Rect::new(head.x, head.bottom(), head.w, 1.0), p.border);
        (inner, clip, head)
    }
}
