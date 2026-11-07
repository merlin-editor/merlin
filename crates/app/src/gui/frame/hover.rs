//! The hover card: problems with their fixes, then the language server's
//! text, as Markdown with coloured code.

use super::*;

impl Frame<'_> {
    /// The hover card: the problems at the spot first, each with its
    /// severity's badge, its message and notes, what reported it and its
    /// code; then, under a divider, the signature in code and the first of
    /// the docs, and Go to definition.
    pub(super) fn hover_card(&mut self, card: &HoverCard, anchor: (f32, f32), a: &Areas, ch: f32) {
        let HoverCard {
            problems,
            lines,
            colours,
            definition,
            fix,
            more_fixes,
        } = *card;
        let p = self.p;
        let parts = hover_parts(lines, colours);
        if problems.is_empty() && parts.is_empty() {
            return;
        }
        // As wide as the widest line, up to a limit past which prose wraps.
        let mut widest: f32 = 0.0;
        for pr in problems {
            for (k, line) in pr.message.lines().enumerate() {
                let face = if k == 0 { MESSAGE } else { PROSE };
                widest = widest.max(self.rich_width(line.trim(), face) + 50.0);
            }
        }
        for part in &parts {
            widest = widest.max(match part {
                Part::Code(line, _) => self.text.width(line, CODE) + 24.0,
                Part::Prose(text) => self.rich_width(text, PROSE) + 24.0,
                Part::Gap => 0.0,
            });
        }
        // Room for the fix, More fixes, Copy and Problems in a row.
        if let Some(title) = fix {
            let tw = self.rich_width(title, (Face::ui(12.0), Face::mono(11.5)));
            widest = widest.max(38.0 + tw + 36.0 + 6.0 + 120.0 + 130.0);
        }
        let min = if problems.is_empty() { 200.0 } else { 340.0 };
        let w = widest.clamp(min, 560.0);
        let layout = self.hover_layout(problems, &parts, definition, w);
        let h = layout.last().map_or(0.0, |(y, h, _)| y + h) + 10.0;
        let r = self.card_place(anchor, w, h, a, ch);
        self.canvas.shadow(r, 8.0, theme::is_light(&p));
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        // The gap between the line and the card holds it open too, so the
        // mouse can cross into it.
        let gap = if r.y > anchor.1 {
            (anchor.1 + ch, r.y)
        } else {
            (r.bottom(), anchor.1)
        };
        self.hit(
            Rect::new(r.x, gap.0, r.w, (gap.1 - gap.0).max(0.0)),
            Hit::Block,
        );
        let clip = r;
        let text_x = r.x + 38.0;
        for (y, h, op) in layout {
            let y = r.y + y;
            match op {
                Op::Badge(severity) => self.badge_icon(severity, r.x + 12.0, y + 2.0),
                Op::Message(line) => {
                    self.draw_rich(&line, text_x, y, h, MESSAGE, p.bright, p.type_, clip)
                }
                Op::Note { line, help } => {
                    let mut x = text_x;
                    if help {
                        let face = PROSE.0;
                        self.text.draw_in_row(
                            &mut self.canvas,
                            "help:",
                            face,
                            x,
                            y,
                            h,
                            p.text,
                            clip,
                        );
                        x += self.text.width("help: ", face);
                    }
                    self.draw_rich(&line, x, y, h, PROSE, p.muted, p.type_, clip);
                }
                Op::Source(i) => {
                    let problem = &problems[i];
                    let mut x = text_x;
                    if !problem.source.is_empty() {
                        x += self.chip(&problem.source, x, y + (h - 16.0) / 2.0, false) + 6.0;
                    }
                    let face = Face::ui(11.5);
                    if !problem.code.is_empty() {
                        let cw = self.text.width(&problem.code, face);
                        let link = !problem.link.is_empty();
                        let code = Rect::new(x, y, cw, h);
                        let colour = match (link, self.hovered(code)) {
                            (true, true) => p.bright,
                            (true, false) => p.accent,
                            (false, _) => p.muted,
                        };
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &problem.code,
                            face,
                            x,
                            y,
                            h,
                            colour,
                            clip,
                        );
                        if link {
                            self.marked_hit(
                                code,
                                Hit::Ui(UiEvent::Hover(HoverButton::Link(i))),
                                Mark::Link,
                            );
                        }
                        x += cw + 6.0;
                    }
                    if !problem.also.is_empty() {
                        let also = format!("·  also from {}", problem.also.join(", "));
                        let shown = self.text.fit(&also, face, r.right() - 12.0 - x);
                        self.text.draw_in_row(
                            &mut self.canvas,
                            &shown,
                            face,
                            x,
                            y,
                            h,
                            p.muted,
                            clip,
                        );
                    }
                }
                Op::Buttons => {
                    // The likeliest fix and the rest on the left, quiet
                    // links on the right.
                    let links = Face::ui(12.0);
                    let links_w =
                        self.text.width("Problems", links) + self.text.width("Copy", links) + 24.0;
                    let label = if fix.is_some() { "More fixes" } else { "Fixes" };
                    let more_w = if more_fixes {
                        self.more_width(label) + 6.0
                    } else {
                        0.0
                    };
                    let mut fx = text_x;
                    if let Some(title) = fix {
                        let room = r.right() - 8.0 - links_w - 8.0 - more_w - fx;
                        fx += self.fix_button(title, fx, y, h, room) + 6.0;
                    }
                    if more_fixes {
                        self.more_button(label, fx, y, h);
                    }
                    let mut x = r.right() - 8.0;
                    for (label, button) in [
                        ("Problems", HoverButton::Problems),
                        ("Copy", HoverButton::Copy),
                    ] {
                        let face = Face::ui(12.0);
                        let bw = self.text.width(label, face) + 12.0;
                        x -= bw;
                        let b = Rect::new(x, y, bw, h);
                        let hovered = self.hovered(b);
                        if hovered {
                            self.canvas.round(b, 5.0, p.hover);
                        }
                        let colour = if hovered { p.bright } else { p.muted };
                        self.text.draw_in_row(
                            &mut self.canvas,
                            label,
                            face,
                            x + 6.0,
                            y,
                            h,
                            colour,
                            clip,
                        );
                        self.hit(b, Hit::Ui(UiEvent::Hover(button)));
                    }
                }
                Op::Divider => self
                    .canvas
                    .fill(Rect::new(r.x + 1.0, y, r.w - 2.0, 1.0), p.border),
                Op::Code(line, spans) => self.code_line(&line, &spans, r.x + 12.0, y, h, clip),
                Op::Prose(line) => {
                    self.draw_rich(&line, r.x + 12.0, y, h, PROSE, p.muted, p.text, clip)
                }
                Op::Definition => {
                    let face = Face::ui(11.5);
                    let label = "Go to definition";
                    let lw = self.text.width(label, face);
                    let link = Rect::new(r.x + 12.0, y, lw + 8.0 + 30.0, h);
                    let colour = if self.hovered(link) {
                        p.bright
                    } else {
                        p.accent
                    };
                    self.text.draw_in_row(
                        &mut self.canvas,
                        label,
                        face,
                        r.x + 12.0,
                        y,
                        h,
                        colour,
                        clip,
                    );
                    self.chip("F12", r.x + 12.0 + lw + 8.0, y + (h - 16.0) / 2.0, false);
                    self.marked_hit(
                        link,
                        Hit::Ui(UiEvent::Hover(HoverButton::Definition)),
                        Mark::Link,
                    );
                }
            }
        }
    }

    /// Lays the hover card out at width `w`: each thing to draw, with its
    /// top and height from the card's top.
    pub(super) fn hover_layout(
        &mut self,
        problems: &[ProblemView],
        parts: &[Part],
        definition: bool,
        w: f32,
    ) -> Vec<(f32, f32, Op)> {
        let mut ops = Vec::new();
        let mut y = 10.0;
        let text_w = w - 38.0 - 12.0;
        for (i, problem) in problems.iter().enumerate() {
            ops.push((y, 20.0, Op::Badge(problem.severity)));
            let mut lines = problem.message.lines().filter(|l| !l.trim().is_empty());
            let first = lines.next().unwrap_or_default();
            for line in self.wrap_rich(first, MESSAGE, text_w).into_iter().take(3) {
                ops.push((y, 20.0, Op::Message(line)));
                y += 20.0;
            }
            for note in lines.take(6) {
                let note = note.trim();
                let (help, rest) = match note.strip_prefix("help:") {
                    Some(rest) => (true, rest.trim_start()),
                    None => (false, note),
                };
                let indent = if help {
                    self.text.width("help: ", PROSE.0)
                } else {
                    0.0
                };
                for (k, line) in self
                    .wrap_rich(rest, PROSE, text_w - indent)
                    .into_iter()
                    .take(3)
                    .enumerate()
                {
                    ops.push((
                        y,
                        18.0,
                        Op::Note {
                            line,
                            help: help && k == 0,
                        },
                    ));
                    y += 18.0;
                }
            }
            if !problem.source.is_empty() || !problem.code.is_empty() {
                y += 4.0;
                ops.push((y, 20.0, Op::Source(i)));
                y += 20.0;
            }
            y += 8.0;
        }
        if !problems.is_empty() {
            ops.push((y - 4.0, 24.0, Op::Buttons));
            y += 20.0 + 8.0;
        }
        if !parts.is_empty() || definition {
            if !problems.is_empty() {
                ops.push((y, 1.0, Op::Divider));
                y += 9.0;
            }
            for part in parts {
                match part {
                    Part::Code(line, spans) => {
                        ops.push((y, 19.0, Op::Code(line.clone(), spans.clone())));
                        y += 19.0;
                    }
                    Part::Prose(text) => {
                        for line in self.wrap_rich(text, PROSE, w - 24.0).into_iter().take(4) {
                            ops.push((y, 18.0, Op::Prose(line)));
                            y += 18.0;
                        }
                    }
                    Part::Gap => y += 6.0,
                }
            }
            if definition {
                y += 6.0;
                ops.push((y, 20.0, Op::Definition));
            }
        }
        ops
    }

    /// The hover card's likeliest fix: a bulb and its title, as the
    /// primary button. Returns its width.
    pub(super) fn fix_button(&mut self, title: &str, x: f32, y: f32, h: f32, room: f32) -> f32 {
        let p = self.p;
        let face = (Face::ui(12.0), Face::mono(11.5));
        let tw = self.rich_width(title, face).min((room - 36.0).max(60.0));
        let b = Rect::new(x, y, tw + 36.0, h);
        let hovered = self.hovered(b);
        let fill = if hovered {
            mix(p.accent, p.widget, 0.32)
        } else {
            mix(p.accent, p.widget, 0.2)
        };
        self.canvas
            .outline(b, 5.0, 1.0, mix(p.accent, p.widget, 0.45), fill);
        self.bulb_icon(x + 8.0, y + (h - 12.0) / 2.0, 12.0, p.warning);
        let line = self
            .wrap_rich(title, face, f32::MAX)
            .into_iter()
            .next()
            .unwrap_or_default();
        // `draw_rich` stops 12 short of the clip, so this ends 10 inside.
        let clip = Rect::new(x, y, b.w + 2.0, h);
        self.draw_rich(&line, x + 26.0, y, h, face, p.bright, p.bright, clip);
        self.hit(b, Hit::Ui(UiEvent::Hover(HoverButton::Fix)));
        b.w
    }

    pub(super) fn more_width(&mut self, label: &str) -> f32 {
        let key_w = self.text.width("Ctrl .", Face::ui(11.0)) + 12.0;
        self.text.width(label, Face::ui(12.0)) + key_w + 24.0
    }

    /// More fixes, with its key.
    pub(super) fn more_button(&mut self, label: &str, x: f32, y: f32, h: f32) {
        let p = self.p;
        let face = Face::ui(12.0);
        let tw = self.text.width(label, face);
        let b = Rect::new(x, y, self.more_width(label), h);
        let hovered = self.hovered(b);
        self.canvas
            .round(b, 5.0, if hovered { p.border } else { p.chip });
        self.text
            .draw_in_row(&mut self.canvas, label, face, x + 9.0, y, h, p.text, b);
        self.chip("Ctrl .", x + tw + 15.0, y + (h - 16.0) / 2.0, false);
        self.hit(b, Hit::Ui(UiEvent::Hover(HoverButton::MoreFixes)));
    }

    /// A lightbulb in a `size` square at (`x`, `y`): its glass and base.
    pub(super) fn bulb_icon(&mut self, x: f32, y: f32, size: f32, colour: u32) {
        let (cx, cy, r) = (x + size / 2.0, y + size * 0.4, size * 0.3);
        self.canvas
            .round(Rect::new(cx - r, cy - r, r * 2.0, r * 2.0), r, colour);
        let w = r * 0.6;
        self.canvas.round(
            Rect::new(cx - w, cy + r * 0.6, w * 2.0, r * 1.0),
            1.0,
            colour,
        );
    }

    /// A line of code in a card, in the colours its language's plugin
    /// gave, by character.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_line(
        &mut self,
        line: &str,
        spans: &Spans,
        x: f32,
        y: f32,
        h: f32,
        clip: Rect,
    ) {
        self.code_line_in(line, spans, x, y, h, clip, CODE);
    }

    /// A line of code in `face`, coloured by `spans`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_line_in(
        &mut self,
        line: &str,
        spans: &Spans,
        mut x: f32,
        y: f32,
        h: f32,
        clip: Rect,
        face: Face,
    ) {
        let p = self.p;
        let chars: Vec<char> = line.chars().collect();
        let mut runs: Vec<(String, u32)> = Vec::new();
        let mut at = 0;
        for &(start, end, syntax) in spans {
            let (start, end) = (start.min(chars.len()), end.min(chars.len()));
            if start < at || start >= end {
                continue;
            }
            if start > at {
                runs.push((chars[at..start].iter().collect(), p.text));
            }
            let colour = theme::code(&p, syntax).unwrap_or(p.text);
            runs.push((chars[start..end].iter().collect(), colour));
            at = end;
        }
        if at < chars.len() {
            runs.push((chars[at..].iter().collect(), p.text));
        }
        for (text, colour) in runs {
            let max = clip.right() - 12.0 - x;
            if max <= 0.0 {
                break;
            }
            let shown = self.text.fit(&text, face, max);
            self.text
                .draw_in_row(&mut self.canvas, &shown, face, x, y, h, colour, clip);
            x += self.text.width(&shown, face);
        }
    }

    /// A severity's badge: its mark in a round chip of its colour.
    pub(super) fn badge_icon(&mut self, severity: Severity, x: f32, y: f32) {
        let p = self.p;
        let colour = theme::severity(&p, severity);
        let r = Rect::new(x, y, 16.0, 16.0);
        self.canvas.round(r, 8.0, mix(colour, p.widget, 0.2));
        let mark = match severity {
            Severity::Error => "×",
            Severity::Warning => "!",
            _ => "i",
        };
        let face = Face::ui(if severity == Severity::Error {
            13.0
        } else {
            11.0
        })
        .bold();
        let mw = self.text.width(mark, face);
        self.text.draw_in_row(
            &mut self.canvas,
            mark,
            face,
            x + (16.0 - mw) / 2.0,
            y,
            16.0,
            colour,
            r,
        );
    }

    /// The width of a line whose `backticked` parts are code.
    pub(super) fn rich_width(&mut self, line: &str, (face, code): (Face, Face)) -> f32 {
        code_spans(line)
            .iter()
            .map(|(text, is_code)| self.text.width(text, if *is_code { code } else { face }))
            .sum()
    }

    /// A line with backticked code wrapped at `w`, as runs of text and code.
    pub(super) fn wrap_rich(
        &mut self,
        line: &str,
        (face, code): (Face, Face),
        w: f32,
    ) -> Vec<Vec<(String, bool)>> {
        let mut out: Vec<Vec<(String, bool)>> = vec![Vec::new()];
        let mut width = 0.0;
        for (span, is_code) in code_spans(line) {
            for word in span.split_inclusive(' ') {
                let ww = self.text.width(word, if is_code { code } else { face });
                let line = out.last_mut().unwrap();
                if width + ww > w && !line.is_empty() {
                    out.push(Vec::new());
                    width = 0.0;
                }
                let line = out.last_mut().unwrap();
                match line.last_mut() {
                    Some((text, c)) if *c == is_code => text.push_str(word),
                    _ => line.push((word.to_string(), is_code)),
                }
                width += ww;
            }
        }
        out.retain(|l| !l.is_empty());
        out
    }

    /// Runs of text and code side by side, cut off at the card's edge.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_rich(
        &mut self,
        line: &[(String, bool)],
        mut x: f32,
        y: f32,
        h: f32,
        (face, code): (Face, Face),
        colour: u32,
        code_colour: u32,
        clip: Rect,
    ) {
        for (text, is_code) in line {
            let (f, c) = if *is_code {
                (code, code_colour)
            } else {
                (face, colour)
            };
            let max = clip.right() - 12.0 - x;
            if max <= 0.0 {
                break;
            }
            let shown = self.text.fit(text, f, max);
            self.text
                .draw_in_row(&mut self.canvas, &shown, f, x, y, h, c, clip);
            x += self.text.width(&shown, f);
        }
    }
}
