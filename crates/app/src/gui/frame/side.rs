//! The side bars and the panel: their views, plugin panels as nodes,
//! and the terminal.

use super::*;

impl Frame<'_> {
    pub(super) fn side(
        &mut self,
        sv: &SideView,
        side: Side,
        area: Rect,
        state: &mut State,
        term: Option<&TerminalView>,
    ) {
        let p = self.p;
        let header = Rect::new(area.x, area.y, area.w, HEADER_H);
        // The rail picks what the side bars show; the panel under the editor
        // has its own tabs: Problems and Output.
        if side == Side::Bottom && sv.panels.len() > 1 {
            // Panels that share the side, as tabs: Problems and Output.
            let mut x = area.x + 10.0;
            for (name, title, on) in &sv.panels {
                let face = Face::ui(12.5).medium();
                let tw = self.text.width(title, face);
                let r = Rect::new(x, area.y + 7.0, tw + 20.0, 24.0);
                if *on {
                    self.canvas.round(r, 6.0, p.raised);
                } else if self.hovered(r) {
                    self.canvas.round(r, 6.0, p.hover);
                }
                let colour = if *on { p.bright } else { p.muted };
                self.text.draw_in_row(
                    &mut self.canvas,
                    title,
                    face,
                    x + 10.0,
                    r.y,
                    24.0,
                    colour,
                    header,
                );
                self.hit(
                    r,
                    Hit::Ui(UiEvent::PanelTitle {
                        side,
                        name: name.clone(),
                    }),
                );
                x += r.w + 4.0;
            }
        } else {
            let title = title_case(&sv.title);
            self.text.draw_in_row(
                &mut self.canvas,
                &title,
                Face::ui(13.0).bold(),
                area.x + 14.0,
                area.y,
                HEADER_H,
                p.bright,
                header,
            );
            // The Tests view's passed and failed, at the right.
            if let Some((passed, failed)) = sv.counts.filter(|(a, b)| a + b > 0) {
                let face = Face::ui(12.0).medium();
                let mut x = header.right() - 14.0;
                for (text, colour) in [
                    (format!("{failed} ✕"), p.error),
                    (format!("{passed} ✓"), p.success),
                ] {
                    let w = self.text.width(&text, face);
                    x -= w;
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &text,
                        face,
                        x,
                        area.y,
                        HEADER_H,
                        colour,
                        header,
                    );
                    x -= 8.0;
                }
            }
        }
        let body = Rect::new(
            area.x,
            area.y + HEADER_H,
            area.w,
            (area.h - HEADER_H - 6.0).max(0.0),
        );
        if let Some(term) = term {
            return self.terminal_panel(term, header, body);
        }
        self.hit(body, Hit::Side(side));

        // Lay the nodes out first, to keep the selection in view.
        let inner = area.w - 28.0;
        let mut rows = Vec::with_capacity(sv.nodes.len());
        let mut y = 0.0;
        for node in &sv.nodes {
            let h = self.node_height(node, inner);
            rows.push((y, h));
            y += h;
        }
        let total = y;
        let k = side_index(side);
        if sv.focused && state.selected[k] != Some(sv.selected) {
            if let Some(i) = sv.targets.iter().position(|t| *t == Some(sv.selected)) {
                let (ry, rh) = rows[i];
                let scroll = &mut state.scroll[k];
                if ry < *scroll {
                    *scroll = ry;
                } else if ry + rh > *scroll + body.h {
                    *scroll = ry + rh - body.h;
                }
            }
            state.selected[k] = Some(sv.selected);
        }
        let scroll = state.scroll[k].clamp(0.0, (total - body.h).max(0.0));
        state.scroll[k] = scroll;

        for (i, node) in sv.nodes.iter().enumerate() {
            let (ry, rh) = rows[i];
            let top = body.y + ry - scroll;
            if top + rh < body.y || top > body.bottom() {
                continue;
            }
            let r = Rect::new(area.x, top, area.w, rh);
            let target = sv.targets[i];
            let selected = target.is_some() && target == Some(sv.selected);
            self.node(node, sv, side, r, target, selected, body);
        }
    }

    /// The terminal panel: its buttons in the header, the shells showing
    /// side by side, and the list of shells when there are several. The
    /// shells' cells are drawn after, in each of [`Frame::term_panes`].
    pub(super) fn terminal_panel(&mut self, tv: &TerminalView, header: Rect, body: Rect) {
        let p = self.p;
        // The header's buttons, right to left.
        let mut x = header.right() - 8.0;
        let close = (u8::MAX, "Close Panel (Ctrl+J)", Hit::Command("view.panel"));
        let buttons = [
            close,
            if self.panel_max {
                (5, "Restore Panel Size", Hit::MaximizePanel)
            } else {
                (4, "Maximise Panel Size", Hit::MaximizePanel)
            },
            (3, "Kill the Terminal", Hit::Command("terminal.kill")),
            (
                2,
                "Split Terminal (Ctrl+Shift+5)",
                Hit::Command("terminal.split"),
            ),
            (
                1,
                "New Terminal (Ctrl+Shift+`)",
                Hit::Command("terminal.new"),
            ),
        ];
        for (icon, tip, hit) in buttons {
            let r = Rect::new(x - 26.0, header.y + 6.0, 26.0, 26.0);
            x = r.x - 2.0;
            let hovered = self.hovered(r);
            if hovered {
                self.canvas.round(r, 6.0, p.hover);
            }
            let c = if hovered { p.bright } else { p.muted };
            self.toolbar_icon(icon, r, c);
            self.hit(r, hit);
            self.tips.push((r, tip.to_string()));
        }
        // The list of shells, when there's more than one.
        let list_w = if tv.sessions.len() > 1 { 168.0 } else { 0.0 };
        if list_w > 0.0 {
            let lx = body.right() - list_w;
            self.canvas.fill(Rect::new(lx, body.y, 1.0, body.h), p.line);
            for (k, session) in tv.sessions.iter().enumerate() {
                let r = Rect::new(
                    lx + 6.0,
                    body.y + 2.0 + k as f32 * 28.0,
                    list_w - 12.0,
                    26.0,
                );
                if r.bottom() > body.bottom() {
                    break;
                }
                let on = tv.active == Some(session.id);
                if on {
                    self.canvas.round(r, 6.0, p.chip);
                } else if self.hovered(r) {
                    self.canvas.round(r, 6.0, p.hover);
                }
                let dot = if session.exited.is_some() {
                    p.faint
                } else {
                    p.success
                };
                let cy = r.y + r.h / 2.0;
                self.canvas
                    .round(Rect::new(r.x + 9.0, cy - 3.5, 7.0, 7.0), 3.5, dot);
                let face = Face::ui(12.5);
                let title = self.text.fit(&session.title, face, r.w - 46.0);
                let colour = if on { p.bright } else { p.text };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &title,
                    face,
                    r.x + 24.0,
                    r.y,
                    r.h,
                    colour,
                    r,
                );
                let n = (k + 1).to_string();
                let nw = self.text.width(&n, Face::ui(11.0));
                self.text.draw_in_row(
                    &mut self.canvas,
                    &n,
                    Face::ui(11.0),
                    r.right() - 8.0 - nw,
                    r.y,
                    r.h,
                    p.muted,
                    r,
                );
                self.hit(r, Hit::Ui(UiEvent::Terminal(session.id)));
            }
        }
        // The panes, side by side with a hairline between.
        let panes = Rect::new(body.x, body.y, body.w - list_w, body.h);
        let n = tv.panes.len().max(1) as f32;
        let w = panes.w / n;
        for (k, &id) in tv.panes.iter().enumerate() {
            let r = Rect::new(panes.x + k as f32 * w, panes.y, w, panes.h);
            if k > 0 {
                self.canvas.fill(Rect::new(r.x, r.y, 1.0, r.h), p.line);
            }
            // A focused split pane gets a quiet edge along its top.
            if tv.panes.len() > 1 && tv.focused && tv.active == Some(id) {
                self.canvas
                    .fill(Rect::new(r.x + 8.0, r.y, r.w - 16.0, 1.5), p.accent);
            }
            let inner = Rect::new(r.x + 12.0, r.y + 6.0, r.w - 20.0, r.h - 8.0);
            self.hit(r, Hit::Terminal(id));
            if let Some((code, task)) = tv
                .sessions
                .iter()
                .find(|s| s.id == id)
                .and_then(|s| Some((s.exited?, s.task)))
            {
                let note = match (task, code) {
                    (true, 0) => "The task is done. Run it again from Run Task.".to_string(),
                    (true, code) => format!("The task failed with code {code}."),
                    (false, code) => {
                        format!("The shell ended with code {code}. The bin closes it.")
                    }
                };
                let face = Face::ui(12.0);
                let y = r.bottom() - 26.0;
                self.text
                    .draw_in_row(&mut self.canvas, &note, face, inner.x, y, 22.0, p.muted, r);
                self.term_panes.push((
                    id,
                    Rect::new(inner.x, inner.y, inner.w, (inner.h - 28.0).max(0.0)),
                ));
            } else {
                self.term_panes.push((id, inner));
            }
        }
    }

    pub(super) fn node_height(&mut self, node: &Node, inner: f32) -> f32 {
        match node {
            Node::Heading(_) => 30.0,
            Node::Text(t) | Node::Styled(t, _) => {
                self.text.wrapped_height(t, Face::ui(12.5), inner) + 8.0
            }
            Node::Markdown(t) => self.text.wrapped_height(&plain(t), Face::ui(12.5), inner) + 8.0,
            Node::Button(_) | Node::Buttons(_) => 38.0,
            Node::Tools(_) => 32.0,
            Node::Item { .. } | Node::Entry(_) => {
                if self.compact {
                    22.0
                } else {
                    ROW_H
                }
            }
            Node::Input { .. } => 40.0,
            Node::Divider => 13.0,
            Node::Footer => 0.0,
            Node::Header { .. } => 38.0,
            Node::Banner { .. } => 96.0,
            Node::Wide { .. } => 38.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn node(
        &mut self,
        node: &Node,
        sv: &SideView,
        side: Side,
        r: Rect,
        target: Option<usize>,
        selected: bool,
        clip: Rect,
    ) {
        let p = self.p;
        let inner = r.w - 28.0;
        let row_bg = |frame: &mut Self, row: Rect| {
            let bg = if selected && sv.focused {
                p.focus_row
            } else if selected {
                p.selected
            } else if frame.hovered(row) {
                p.hover
            } else {
                return;
            };
            frame.canvas.round(row, 6.0, bg);
        };
        match node {
            Node::Heading(t) => {
                let face = Face::ui(11.0).bold();
                self.text.draw_in_row(
                    &mut self.canvas,
                    &t.to_uppercase(),
                    face,
                    r.x + 14.0,
                    r.y + 4.0,
                    26.0,
                    p.muted,
                    clip,
                );
            }
            Node::Text(t) => {
                self.text.draw(
                    &mut self.canvas,
                    t,
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    p.text,
                    clip,
                    Some(inner),
                );
            }
            Node::Styled(t, tone) => {
                self.text.draw(
                    &mut self.canvas,
                    t,
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    tone_colour(&p, *tone),
                    clip,
                    Some(inner),
                );
            }
            Node::Markdown(t) => {
                self.text.draw(
                    &mut self.canvas,
                    &plain(t),
                    Face::ui(12.5),
                    r.x + 14.0,
                    r.y + 4.0,
                    p.text,
                    clip,
                    Some(inner),
                );
            }
            Node::Item { label, tone, .. } => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                let label = self.text.fit(label, Face::ui(13.0), inner);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    Face::ui(13.0),
                    r.x + 14.0,
                    r.y,
                    r.h,
                    tone_colour(&p, *tone),
                    clip,
                );
                if let Some(index) = target {
                    self.hit(
                        row,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                }
            }
            Node::Entry(e) => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                let hits = self.entry(e, row, clip);
                let check = hits.check;
                if let Some(index) = target {
                    self.hit(
                        row,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                    if let Some(b) = check {
                        self.hit(b, Hit::Ui(UiEvent::Check { side, index }));
                    }
                    if e.action.is_some() {
                        let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
                        self.hit(
                            a,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index: index + 1,
                                clicks: 1,
                            }),
                        );
                    }
                    if let Some(f) = hits.fold {
                        self.hit(f, Hit::Ui(UiEvent::Fold { side, index }));
                    }
                    let first_link = index + 1 + usize::from(e.action.is_some());
                    for (j, l) in hits.links.iter().enumerate() {
                        self.hit(
                            *l,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index: first_link + j,
                                clicks: 1,
                            }),
                        );
                    }
                }
            }
            Node::Button(b) => {
                let bw = self.button(
                    &b.label,
                    r.x + 14.0,
                    r.y + 4.0,
                    selected && sv.focused,
                    true,
                );
                if let Some(index) = target {
                    self.hit(
                        Rect::new(r.x + 14.0, r.y + 4.0, bw, 30.0),
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                }
            }
            Node::Buttons(bs) => {
                let mut x = r.x + 14.0;
                for (k, b) in bs.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let bw = self.button(&b.label, x, r.y + 4.0, focused, k == 0);
                    if let Some(index) = index {
                        self.hit(
                            Rect::new(x, r.y + 4.0, bw, 30.0),
                            Hit::Ui(UiEvent::Target {
                                side,
                                index,
                                clicks: 1,
                            }),
                        );
                    }
                    x += bw + 8.0;
                }
            }
            Node::Tools(bs) => {
                let mut x = r.x + 12.0;
                let face = Face::ui(12.0);
                for (k, b) in bs.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let tw = self.text.width(&b.label, face);
                    let br = Rect::new(x, r.y + 4.0, tw + 16.0, 24.0);
                    if x + br.w > r.right() - 8.0 {
                        break;
                    }
                    let bg = if self.hovered(br) { p.border } else { p.chip };
                    if focused {
                        self.canvas
                            .outline(br.inset(-2.0), 7.0, 1.5, p.accent, p.island);
                    }
                    self.canvas.round(br, 5.0, bg);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &b.label,
                        face,
                        x + 8.0,
                        br.y,
                        br.h,
                        p.text,
                        br,
                    );
                    if let Some(index) = index {
                        self.hit(
                            br,
                            Hit::Ui(UiEvent::Target {
                                side,
                                index,
                                clicks: 1,
                            }),
                        );
                    }
                    x += br.w + 6.0;
                }
            }
            Node::Input {
                id,
                placeholder,
                secret,
            } => {
                let b = Rect::new(r.x + 10.0, r.y + 5.0, r.w - 20.0, 30.0);
                let focused = selected && sv.focused;
                let border = if focused { p.accent } else { p.border };
                self.canvas.outline(b, 7.0, 1.0, border, p.raised);
                let typed = sv
                    .inputs
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or_default();
                let shown = if *secret {
                    "•".repeat(typed.chars().count())
                } else {
                    typed.to_string()
                };
                let face = Face::ui(13.0);
                let (label, colour) = if shown.is_empty() {
                    (placeholder.as_str(), p.muted)
                } else {
                    (shown.as_str(), p.text)
                };
                let tw = self.text.draw_in_row(
                    &mut self.canvas,
                    label,
                    face,
                    b.x + 10.0,
                    b.y,
                    b.h,
                    colour,
                    b.inset(1.0),
                );
                if focused {
                    let cx = if shown.is_empty() {
                        b.x + 10.0
                    } else {
                        b.x + 10.0 + tw
                    };
                    self.canvas
                        .fill(Rect::new(cx, b.y + 7.0, 1.5, b.h - 14.0), p.accent);
                }
                if let Some(index) = target {
                    self.hit(
                        b,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                }
                self.input_box(b);
            }
            Node::Divider => {
                self.canvas
                    .fill(Rect::new(r.x + 14.0, r.y + 6.0, r.w - 28.0, 1.0), p.line);
            }
            Node::Footer => {}
            Node::Header {
                title,
                detail,
                buttons,
            } => self.header(title, detail, buttons, r, clip),
            Node::Banner {
                title,
                detail,
                tone,
                buttons,
            } => {
                // A box tinted by its tone: the title, what's left, then
                // its buttons.
                let colour = tone_colour(&p, *tone);
                let b = Rect::new(r.x + 10.0, r.y + 2.0, r.w - 20.0, r.h - 10.0);
                self.canvas.outline(
                    b,
                    8.0,
                    1.0,
                    mix(colour, p.island, 0.22),
                    mix(colour, p.island, 0.07),
                );
                let inside = b.inset(1.0).intersect(clip);
                let mut x = b.x + 10.0;
                for (text, strong) in title {
                    let face = if *strong {
                        Face::ui(12.5).medium()
                    } else {
                        Face::ui(12.5)
                    };
                    let colour = if *strong { p.bright } else { p.text };
                    x += self.text.draw_in_row(
                        &mut self.canvas,
                        text,
                        face,
                        x,
                        b.y + 7.0,
                        18.0,
                        colour,
                        inside,
                    );
                }
                self.text.draw_in_row(
                    &mut self.canvas,
                    detail,
                    Face::ui(12.0),
                    b.x + 10.0,
                    b.y + 25.0,
                    18.0,
                    p.muted,
                    inside,
                );
                let mut x = b.x + 10.0;
                for (k, button) in buttons.iter().enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    let bw = self.button(&button.label, x, b.y + 49.0, focused, false);
                    if let Some(index) = index {
                        self.hit(
                            Rect::new(x, b.y + 49.0, bw, 30.0),
                            Hit::Ui(UiEvent::Target {
                                side,
                                index,
                                clicks: 1,
                            }),
                        );
                    }
                    x += bw + 8.0;
                }
            }
            Node::Wide { button, ready } => {
                // The main button across the panel, faded until it's ready.
                let b = Rect::new(r.x + 14.0, r.y + 4.0, r.w - 28.0, 30.0);
                let focused = selected && sv.focused;
                if focused {
                    self.canvas.outline(
                        Rect::new(b.x - 2.0, b.y - 2.0, b.w + 4.0, 34.0),
                        9.0,
                        1.5,
                        p.accent,
                        p.island,
                    );
                }
                let bg = if !*ready {
                    mix(p.accent, p.island, 0.45)
                } else if self.hovered(b) {
                    mix(p.accent, 0xffffff, 0.88)
                } else {
                    p.accent
                };
                self.canvas.round(b, 7.0, bg);
                let face = Face::ui(13.0).medium();
                let label = self.text.fit(&button.label, face, b.w - 16.0);
                let tw = self.text.width(&label, face);
                let fg = if *ready {
                    p.ground
                } else {
                    mix(p.ground, p.island, 0.6)
                };
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    b.x + (b.w - tw) / 2.0,
                    b.y,
                    30.0,
                    fg,
                    b,
                );
                if let Some(index) = target {
                    self.hit(
                        b,
                        Hit::Ui(UiEvent::Target {
                            side,
                            index,
                            clicks: 1,
                        }),
                    );
                }
            }
        }
    }

    /// A title, toned parts as code, with buttons and their keys at the
    /// right end, the last the main one.
    pub(super) fn header(
        &mut self,
        title: &[(String, Tone)],
        detail: &str,
        buttons: &[(merlin_core::panel::Button, String)],
        r: Rect,
        clip: Rect,
    ) {
        let p = self.p;
        let (y, h) = (r.y + 4.0, 26.0);
        let mut right = r.right() - 14.0;
        for (k, (button, key)) in buttons.iter().enumerate().rev() {
            let w = self.key_button(&button.label, key, right, y, k + 1 == buttons.len());
            self.hit(
                Rect::new(right - w, y, w, h),
                Hit::Ui(UiEvent::Press(button.id.clone())),
            );
            right -= w + 8.0;
        }
        let mut x = r.x + 14.0;
        for (text, tone) in title {
            let (face, colour) = match tone {
                Tone::Normal => (Face::ui(13.0), p.text),
                tone => (Face::mono(12.5), tone_colour(&p, *tone)),
            };
            x += self
                .text
                .draw_in_row(&mut self.canvas, text, face, x, y, h, colour, clip);
        }
        if !detail.is_empty() {
            let face = Face::ui(13.0);
            let room = (right - x - 24.0).max(0.0);
            let shown = self.text.fit(&format!("  ·  {detail}"), face, room);
            self.text
                .draw_in_row(&mut self.canvas, &shown, face, x, y, h, p.muted, clip);
        }
    }

    /// A line with a name changed: its number, the old name struck through
    /// and tinted, then the new.
    pub(super) fn swap(
        &mut self,
        s: &Swap,
        muted: bool,
        x: f32,
        row: Rect,
        room: f32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let mono = Face::mono(12.5);
        let number = s.line.to_string();
        let nw = self.text.width(&number, mono);
        let num_x = x + 26.0 - nw;
        self.text.draw_in_row(
            &mut self.canvas,
            &number,
            mono,
            num_x,
            row.y,
            row.h,
            p.muted,
            clip,
        );
        let start = x + 34.0;
        let end = start + (room - 34.0).max(0.0);
        let clip = Rect::new(clip.x, clip.y, (end - clip.x).max(0.0), clip.h);
        let text = if muted { p.muted } else { p.text };
        let mid = row.y + row.h / 2.0;
        let mut at = start;
        at += self.text.draw_in_row(
            &mut self.canvas,
            &s.before,
            mono,
            at,
            row.y,
            row.h,
            text,
            clip,
        );
        let ow = self.text.width(&s.old, mono);
        if !muted {
            let tint = Rect::new(at, mid - 8.0, ow, 16.0);
            self.canvas.tint(tint, 2.0, p.error, 77);
        }
        self.text
            .draw_in_row(&mut self.canvas, &s.old, mono, at, row.y, row.h, text, clip);
        if at < end {
            let strike = Rect::new(at, mid, ow.min(end - at), 1.0);
            self.canvas.fill(strike, text);
        }
        at += ow;
        if !s.new.is_empty() {
            let w = self.text.width(&s.new, mono);
            self.canvas
                .tint(Rect::new(at, mid - 8.0, w, 16.0), 2.0, p.success, 71);
            at +=
                self.text
                    .draw_in_row(&mut self.canvas, &s.new, mono, at, row.y, row.h, text, clip);
        }
        // What's past the room is cut with an ellipsis.
        let after = self.text.fit(&s.after, mono, (end - at).max(0.0));
        at += self
            .text
            .draw_in_row(&mut self.canvas, &after, mono, at, row.y, row.h, text, clip);
        at - x
    }

    /// A tree row: indent, arrow, checkbox, icon, name, detail, counts,
    /// and a badge and buttons at the end. Returns where its checkbox,
    /// arrow and buttons are, to click.
    pub(super) fn entry(&mut self, e: &Entry, row: Rect, clip: Rect) -> EntryHits {
        let p = self.p;
        let mut x = row.x + 8.0 + e.depth as f32 * 14.0;
        let mid = row.y + row.h / 2.0;
        let mut hits = EntryHits::default();
        // A section's heading: its title in capitals and its count, with
        // its button on hover.
        if e.heading {
            let face = Face::ui(11.0).bold();
            let colour = match e.tone {
                Tone::Normal => p.muted,
                tone => tone_colour(&p, tone),
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &e.label.to_uppercase(),
                face,
                x,
                row.y,
                row.h,
                colour,
                clip,
            );
            let bw = self.text.width(&e.badge, face);
            let right = row.right() - 8.0;
            self.text.draw_in_row(
                &mut self.canvas,
                &e.badge,
                face,
                right - bw,
                row.y,
                row.h,
                p.muted,
                clip,
            );
            if let Some((glyph, _)) = &e.action
                && self.hovered(row)
            {
                let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
                self.canvas.round(a, 5.0, p.chip);
                let g = glyph.to_string();
                let gw = self.text.width(&g, Face::mono(13.0));
                self.text.draw_in_row(
                    &mut self.canvas,
                    &g,
                    Face::mono(13.0),
                    a.x + (20.0 - gw) / 2.0,
                    a.y,
                    20.0,
                    p.bright,
                    clip,
                );
            }
            return hits;
        }
        if let Some(open) = e.fold {
            self.canvas.chevron(x + 5.0, mid, 7.0, open, p.muted);
            hits.fold = Some(Rect::new(x - 4.0, row.y, 20.0, row.h));
            x += 16.0;
        }
        // Folders have an open or closed arrow; files a blank in its place.
        let folder = match e.twisty.filter(|_| e.fold.is_none()) {
            Some(t @ ('▾' | '▸' | '\u{eab4}' | '\u{eab6}')) => {
                let open = matches!(t, '▾' | '\u{eab4}');
                self.canvas.chevron(x + 5.0, mid, 7.0, open, p.muted);
                x += 16.0;
                Some(open)
            }
            Some(_) => {
                x += 16.0;
                None
            }
            None => None,
        };
        hits.check = e.check.map(|check| {
            self.checkbox(check, x, mid);
            x += 22.0;
            Rect::new(x - 26.0, row.y, 26.0, row.h)
        });
        match folder {
            // Folders get an icon only from an icon theme that has them.
            Some(open) => {
                if let Some(icon) = self.icons.folder(&e.label, open).cloned()
                    && self.draw_icon(&icon, x, mid - 8.0, 16.0, p.muted, clip)
                {
                    x += 22.0;
                }
            }
            None if e.icon.is_some() => {
                // A test's mark stays a mark, whatever its file's icon.
                let mark = matches!(e.icon, Some(('✓' | '✕' | '○' | '⟳' | '●', _)));
                let name = if mark { "" } else { e.label.as_str() };
                x += self.file_icon(name, e.icon, x, mid - 8.0, 16.0, clip);
            }
            None => {}
        }
        let face = if e.code {
            Face::mono(12.0)
        } else {
            Face::ui(13.0)
        };
        let detail_face = if e.code {
            Face::mono(11.5)
        } else {
            Face::ui(12.0)
        };
        let badge_w = if e.badge.is_empty() {
            0.0
        } else {
            self.text.width(&e.badge, Face::ui(11.0).bold()) + 14.0
        };
        let link_face = Face::ui(11.5);
        let hovered = self.hovered(row);
        let links: Vec<&(String, String)> = e
            .links
            .iter()
            .enumerate()
            .filter(|(j, _)| *j == 0 || hovered)
            .map(|(_, l)| l)
            .collect();
        let links_w: f32 = links
            .iter()
            .map(|(l, _)| self.text.width(l, link_face) + 12.0)
            .sum();
        let room = (row.right()
            - x
            - badge_w
            - links_w
            - if e.action.is_some() { 26.0 } else { 0.0 }
            - 6.0)
            .max(0.0);
        let colour = match e.tone {
            _ if e.strike => p.muted,
            Tone::Normal => p.text,
            tone => tone_colour(&p, tone),
        };
        let lw = match &e.swap {
            Some(s) => {
                let room = room - if e.detail.is_empty() { 0.0 } else { 70.0 };
                self.swap(s, e.tone == Tone::Muted, x, row, room, clip)
            }
            None => {
                let label = self.text.fit(&e.label, face, room);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    face,
                    x,
                    row.y,
                    row.h,
                    colour,
                    clip,
                )
            }
        };
        let mut used = lw;
        // A deleted file's name is struck through.
        if e.strike {
            self.canvas.fill(Rect::new(x, mid, lw, 1.0), p.muted);
        }
        // A chip after the name, as a mode change's.
        if let Some(chip) = &e.chip {
            let face = Face::mono(10.5);
            let cw = self.text.width(chip, face) + 10.0;
            if used + 8.0 + cw < room {
                let r = Rect::new(x + used + 8.0, mid - 8.0, cw, 16.0);
                self.canvas.round(r, 4.0, p.chip);
                self.text.draw_in_row(
                    &mut self.canvas,
                    chip,
                    face,
                    r.x + 5.0,
                    r.y,
                    16.0,
                    p.muted,
                    clip,
                );
                used += 8.0 + cw;
            }
        }
        // A note after the name in its tone, as a conflict's "2 left".
        if let Some((note, tone)) = &e.note {
            let face = Face::ui(11.5);
            let nw = self.text.width(note, face);
            if used + 8.0 + nw < room {
                self.text.draw_in_row(
                    &mut self.canvas,
                    note,
                    face,
                    x + used + 8.0,
                    row.y,
                    row.h,
                    tone_colour(&p, *tone),
                    clip,
                );
                used += 8.0 + nw;
            }
        }
        if !e.detail.is_empty() && used + 12.0 < room {
            let detail = self.text.fit(&e.detail, detail_face, room - used - 8.0);
            used += 8.0
                + self.text.draw_in_row(
                    &mut self.canvas,
                    &detail,
                    detail_face,
                    x + used + 8.0,
                    row.y,
                    row.h,
                    p.muted,
                    clip,
                );
        }
        // Lines added and removed, in green and red.
        if let Some((added, removed)) = e.counts {
            let count = Face::ui(11.5);
            let mut at = x + used + 8.0;
            for (n, sign, colour) in [(added, '+', p.success), (removed, '−', p.error)] {
                if n == 0 {
                    continue;
                }
                let text = format!("{sign}{n}");
                if at + self.text.width(&text, count) > x + room {
                    break;
                }
                at += self.text.draw_in_row(
                    &mut self.canvas,
                    &text,
                    count,
                    at,
                    row.y,
                    row.h,
                    colour,
                    clip,
                ) + 6.0;
            }
        }
        let mut right = row.right() - 8.0;
        // Buttons as words: the first always, in green, at the end, so it
        // stays put; the rest before it while the row is hovered.
        for (j, (label, _)) in links.iter().enumerate() {
            let w = self.text.width(label, link_face);
            let r = Rect::new(right - w - 6.0, row.y + 3.0, w + 12.0, row.h - 6.0);
            if self.hovered(r) {
                self.canvas.round(r, 5.0, p.chip);
            }
            let colour = if j == 0 { p.success } else { p.muted };
            self.text.draw_in_row(
                &mut self.canvas,
                label,
                link_face,
                right - w,
                row.y,
                row.h,
                colour,
                clip,
            );
            hits.links.push(r);
            right -= w + 12.0;
        }
        if !e.badge.is_empty() {
            let face = Face::ui(11.0).bold();
            let bw = self.text.width(&e.badge, face);
            let colour = tone_colour(&p, e.badge_tone);
            if e.badge_tone == Tone::Badge {
                let pill = Rect::new(right - bw - 10.0, mid - 8.0, bw + 10.0, 16.0);
                self.canvas.round(pill, 8.0, p.chip);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &e.badge,
                    face,
                    pill.x + 5.0,
                    pill.y,
                    16.0,
                    p.text,
                    clip,
                );
            } else {
                self.text.draw_in_row(
                    &mut self.canvas,
                    &e.badge,
                    face,
                    right - bw,
                    row.y,
                    row.h,
                    colour,
                    clip,
                );
            }
            right -= bw + 14.0;
        }
        if let Some((glyph, _)) = &e.action
            && self.hovered(row)
        {
            let a = Rect::new(right - 20.0, row.y + 3.0, 20.0, 20.0);
            self.canvas.round(a, 5.0, p.chip);
            let g = glyph.to_string();
            let gw = self.text.width(&g, Face::mono(13.0));
            self.text.draw_in_row(
                &mut self.canvas,
                &g,
                Face::mono(13.0),
                a.x + (20.0 - gw) / 2.0,
                a.y,
                20.0,
                p.bright,
                clip,
            );
        }
        hits
    }
}
