//! The side bars and the panel: their views, plugin panels as nodes,
//! and the terminal.

use super::*;

impl Frame<'_> {
    pub(crate) fn side(
        &mut self,
        sv: &SideView,
        side: Side,
        area: Rect,
        state: &mut State,
        term: Option<&TerminalView>,
    ) {
        let p = self.p;
        // A view in an editor tab has the tab for its title, and only a
        // gap under it.
        let header_h = if side == Side::Tab { 8.0 } else { HEADER_H };
        let header = Rect::new(area.x, area.y, area.w, header_h);
        // The rail picks what the left side bar shows; the panel under the
        // editor and the right side bar have their own tabs.
        let tabs = match side {
            Side::Bottom => sv.panels.len() > 1,
            Side::Right => true,
            _ => false,
        };
        // A plugin view's buttons, at the header's right.
        let tools_x = if side == Side::Right || (side != Side::Tab && sv.from.is_some()) {
            self.view_tools(sv, side, header)
        } else {
            header.right()
        };
        let tools_x = self.tabs_end.map_or(tools_x, |end| tools_x.min(end));
        if tabs {
            let clip = Rect::new(header.x, header.y, tools_x - header.x, header.h);
            self.panel_tabs(sv, side, header, clip);
        } else if side != Side::Tab {
            let title = title_case(&sv.title);
            let face = Face::ui(13.0).bold();
            self.text.draw_in_row(
                &mut self.canvas,
                &title,
                face,
                area.x + 14.0,
                area.y,
                HEADER_H,
                p.bright,
                header,
            );
            // Which plugin a view is from, quieter, after its title.
            if let Some(from) = &sv.from {
                let x = area.x + 14.0 + self.text.width(&title, face) + 6.0;
                let clip = Rect::new(header.x, header.y, tools_x - header.x, header.h);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &format!("· {from}"),
                    Face::ui(12.0),
                    x,
                    area.y,
                    HEADER_H,
                    p.muted,
                    clip,
                );
            }
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
            area.y + header_h,
            area.w,
            (area.h - header_h - 6.0).max(0.0),
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
            let h = self.node_height(node, sv, inner);
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
            self.node(node, sv, side, r, target, selected, body, state);
        }
    }

    /// The panels a side can show, as tabs along its header: each with
    /// its icon, its title, the plugin it's from in the panel, and its
    /// badge.
    fn panel_tabs(&mut self, sv: &SideView, side: Side, header: Rect, clip: Rect) {
        let p = self.p;
        let mut x = header.x + 8.0;
        let h = 26.0;
        let y = header.y + (header.h - h) / 2.0;
        for tab in &sv.panels {
            let face = Face::ui(12.5).medium();
            let icon_w = match (&tab.icon, side) {
                (Some(_), Side::Right) => 18.0,
                _ => 0.0,
            };
            let from = tab
                .from
                .as_ref()
                .filter(|_| side == Side::Bottom)
                .map(|f| format!("· {f}"));
            let from_face = Face::ui(11.5);
            let from_w = from
                .as_ref()
                .map_or(0.0, |f| self.text.width(f, from_face) + 6.0);
            let badge = (tab.badge > 0).then(|| tab.badge.min(99).to_string());
            let badge_face = Face::ui(10.0).bold();
            let badge_w = badge
                .as_ref()
                .map_or(0.0, |b| self.text.width(b, badge_face).max(7.0) + 14.0);
            let tw = self.text.width(&tab.title, face);
            let r = Rect::new(x, y, icon_w + tw + from_w + badge_w + 20.0, h);
            if r.x >= clip.right() {
                break;
            }
            if tab.on {
                self.canvas.round(r, 6.0, p.raised);
            } else if self.hovered(r) {
                self.canvas.round(r, 6.0, p.hover);
            }
            let colour = if tab.on { p.bright } else { p.muted };
            let mut tx = x + 10.0;
            if let (Some(icon), Side::Right) = (&tab.icon, side) {
                self.view_icon(icon, tx, y + 6.0, 14.0, colour, clip);
                tx += icon_w;
            }
            self.text
                .draw_in_row(&mut self.canvas, &tab.title, face, tx, y, h, colour, clip);
            tx += tw;
            if let Some(from) = &from {
                self.text.draw_in_row(
                    &mut self.canvas,
                    from,
                    from_face,
                    tx + 6.0,
                    y,
                    h,
                    p.muted,
                    clip,
                );
                tx += from_w;
            }
            if let Some(badge) = &badge {
                let bw = self.text.width(badge, badge_face).max(7.0) + 8.0;
                let b = Rect::new(tx + 6.0, y + (h - 15.0) / 2.0, bw, 15.0);
                self.canvas.round(b, 7.5, p.accent);
                self.text.draw_in_row(
                    &mut self.canvas,
                    badge,
                    badge_face,
                    b.x + 4.0,
                    b.y,
                    15.0,
                    p.ground,
                    clip,
                );
            }
            self.hit(
                r,
                Hit::Ui(UiEvent::PanelTitle {
                    side,
                    name: tab.name.clone(),
                }),
            );
            x += r.w + 3.0;
        }
    }

    /// A plugin view's toolbar and its ⋯ menu, from the header's right
    /// end. Returns where they start.
    fn view_tools(&mut self, sv: &SideView, side: Side, header: Rect) -> f32 {
        let p = self.p;
        let size = 24.0;
        let y = header.y + (header.h - size) / 2.0;
        let mut x = header.right() - 8.0;
        let more = sv.from.is_some().then(|| {
            (
                Some(merlin_core::ViewIcon::Glyph("⋯".into())),
                format!("view-menu:{}", sv.name),
                "Move or Hide".to_string(),
            )
        });
        // On the right, + shows a view that's hidden.
        let add = (sv.hidden > 0 && side == Side::Right).then(|| {
            (
                Some(merlin_core::ViewIcon::Glyph("+".into())),
                "views-hidden".to_string(),
                "Show a Hidden View".to_string(),
            )
        });
        let tools = sv
            .toolbar
            .iter()
            .map(|t| {
                let icon = t.icon.clone().or_else(|| {
                    t.title
                        .chars()
                        .next()
                        .map(|c| merlin_core::ViewIcon::Glyph(c.into()))
                });
                (icon, format!("run:{}", t.command), t.title.clone())
            })
            .rev();
        for (icon, press, tip) in more.into_iter().chain(add).chain(tools) {
            x -= size;
            let r = Rect::new(x, y, size, size);
            let hovered = self.hovered(r);
            if hovered {
                self.canvas.round(r, 6.0, p.hover);
            }
            let colour = if hovered { p.bright } else { p.muted };
            if let Some(icon) = &icon {
                self.view_icon(icon, x + 5.0, y + 5.0, 14.0, colour, header);
            }
            self.hit(r, Hit::Ui(UiEvent::Press(press)));
            self.tips.push((r, tip));
            x -= 2.0;
        }
        x
    }

    /// A plugin view's icon: its picture, or its glyph.
    pub(super) fn view_icon(
        &mut self,
        icon: &merlin_core::ViewIcon,
        x: f32,
        y: f32,
        size: f32,
        colour: u32,
        clip: Rect,
    ) -> bool {
        let icon = match icon {
            merlin_core::ViewIcon::Svg(file) => Icon::Svg {
                file: file.clone(),
                tint: Tint::Current,
            },
            merlin_core::ViewIcon::Glyph(text) => {
                let face = Face::ui(size);
                let w = self.text.width(text, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    face,
                    x + (size - w) / 2.0,
                    y,
                    size,
                    colour,
                    clip,
                );
                return true;
            }
        };
        self.draw_icon(&icon, x, y, size, colour, clip)
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

    pub(super) fn node_height(&mut self, node: &Node, sv: &SideView, inner: f32) -> f32 {
        match node {
            Node::Heading(_) => 30.0,
            Node::Text(t) | Node::Styled(t, _) => {
                self.text.wrapped_height(t, Face::ui(12.5), inner) + 8.0
            }
            Node::Markdown(t) => self.md_layout(t, inner).1 + 8.0,
            Node::Message(m) => self.message_layout(m, inner).height,
            Node::Chips { .. } => 32.0,
            Node::Edit(_) => 50.0,
            Node::Image(i) => image_size(i, inner).1 + 10.0,
            Node::Progress { detail, .. } => {
                if detail.is_empty() {
                    36.0
                } else {
                    52.0
                }
            }
            Node::Empty { text, .. } => {
                let w = (inner - 24.0).max(60.0);
                let lines = self.wrap_words(text, Face::ui(12.5), w).len() as f32;
                40.0 + 24.0 + lines * 18.0 + 16.0 + 30.0 + 32.0
            }
            // One line each, never wrapped: a long log costs no shaping.
            Node::Log(_) => LOG_ROW_H,
            Node::Button(_) | Node::Buttons(_) => 38.0,
            Node::Tools(_) => 32.0,
            Node::Entry(e) if e.id.starts_with("plugin:") => {
                if self.compact {
                    44.0
                } else {
                    50.0
                }
            }
            Node::Item { .. } | Node::Entry(_) => {
                if self.compact {
                    22.0
                } else {
                    ROW_H
                }
            }
            Node::Input {
                id,
                multiline: true,
                ..
            } => {
                let typed = sv
                    .inputs
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or_default();
                let text_w = (inner - 12.0).max(40.0);
                let face = Face::ui(13.0);
                let text_h = if typed.is_empty() {
                    self.text.height("M", face)
                } else {
                    self.text.wrapped_height(typed, face, text_w)
                };
                let box_h = (text_h + 12.0).clamp(30.0, 120.0);
                box_h + 10.0
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
        state: &mut State,
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
            Node::Log(l) => self.log_row(l, r, clip),
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
                let (ops, _) = self.md_layout(t, inner);
                let colours = node_colours(sv, node);
                self.md_draw(&ops, r.x + 14.0, r.y + 4.0, inner, colours, p.text, clip);
            }
            Node::Message(m) => {
                let colours = node_colours(sv, node);
                self.message(m, colours, sv, side, r, target, clip);
            }
            Node::Chips { chips, hint } => {
                let mut x = r.x + 14.0;
                let mut k = 0;
                for c in chips {
                    let index = (!c.id.is_empty()).then(|| target.map(|t| t + k)).flatten();
                    k += usize::from(!c.id.is_empty());
                    let focused = sv.focused && index.is_some() && index == Some(sv.selected);
                    let w = self.panel_chip(c, x, r.y + 5.0, focused, clip);
                    if let Some(index) = index {
                        self.hit(Rect::new(x, r.y + 5.0, w, 22.0), pick(side, index));
                    }
                    x += w + 6.0;
                }
                if !hint.is_empty() {
                    let f = Face::ui(11.0);
                    let hw = self.text.width(hint, f);
                    let hx = r.right() - 14.0 - hw;
                    if hx > x {
                        self.text.draw_in_row(
                            &mut self.canvas,
                            hint,
                            f,
                            hx,
                            r.y + 5.0,
                            22.0,
                            p.muted,
                            clip,
                        );
                    }
                }
            }
            Node::Edit(e) => self.edit_card(e, sv, side, r, target, clip),
            Node::Image(i) => {
                self.plugin_image(i, Rect::new(r.x + 14.0, r.y + 4.0, inner, r.h), clip)
            }
            Node::Progress {
                label,
                detail,
                done,
            } => {
                let f = Face::ui(12.5);
                let pct = done.map(|d| format!("{}%", d / 10));
                let mut x = r.x + 14.0;
                x += self.text.draw_in_row(
                    &mut self.canvas,
                    label,
                    f,
                    x,
                    r.y + 2.0,
                    20.0,
                    p.text,
                    clip,
                );
                if let Some(pct) = &pct {
                    let pw = self.text.width(pct, Face::ui(11.5));
                    self.text.draw_in_row(
                        &mut self.canvas,
                        pct,
                        Face::ui(11.5),
                        (r.right() - 14.0 - pw).max(x + 8.0),
                        r.y + 2.0,
                        20.0,
                        p.muted,
                        clip,
                    );
                }
                let bar = Rect::new(r.x + 14.0, r.y + 25.0, inner, 4.0);
                self.canvas.round(bar, 2.0, p.chip);
                // How far, or a short stretch while it can't say.
                let (from, to) = match done {
                    Some(d) => (0.0, *d as f32 / 1000.0),
                    None => (0.0, 0.3),
                };
                let fill = Rect::new(
                    bar.x + bar.w * from,
                    bar.y,
                    (bar.w * (to - from)).max(4.0),
                    bar.h,
                );
                self.canvas.round(fill, 2.0, p.accent);
                if !detail.is_empty() {
                    let d = self.text.fit(detail, Face::ui(11.5), inner);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &d,
                        Face::ui(11.5),
                        r.x + 14.0,
                        r.y + 32.0,
                        18.0,
                        p.muted,
                        clip,
                    );
                }
            }
            Node::Empty {
                title,
                text,
                buttons,
            } => {
                // In the middle: its title, a sentence, then its buttons.
                let mid = r.x + r.w / 2.0;
                let mut y = r.y + 40.0;
                let tf = Face::ui(14.0).bold();
                let tw = self.text.width(title, tf);
                self.text.draw_in_row(
                    &mut self.canvas,
                    title,
                    tf,
                    mid - tw / 2.0,
                    y,
                    24.0,
                    p.bright,
                    clip,
                );
                y += 24.0;
                let w = (inner - 24.0).max(60.0);
                let f = Face::ui(12.5);
                for line in self.wrap_words(text, f, w) {
                    let lw = self.text.width(&line, f);
                    self.text.draw_in_row(
                        &mut self.canvas,
                        &line,
                        f,
                        mid - lw / 2.0,
                        y,
                        18.0,
                        p.muted,
                        clip,
                    );
                    y += 18.0;
                }
                y += 16.0;
                let face = Face::ui(13.0).medium();
                let widths: Vec<f32> = buttons
                    .iter()
                    .map(|b| self.text.width(&b.label, face) + 28.0)
                    .collect();
                let total =
                    widths.iter().sum::<f32>() + 8.0 * widths.len().saturating_sub(1) as f32;
                let mut x = mid - total / 2.0;
                for (k, (b, bw)) in buttons.iter().zip(&widths).enumerate() {
                    let index = target.map(|t| t + k);
                    let focused = sv.focused && index == Some(sv.selected);
                    self.button(&b.label, x, y, focused, k == 0);
                    if let Some(index) = index {
                        self.hit(Rect::new(x, y, *bw, 30.0), pick(side, index));
                    }
                    x += bw + 8.0;
                }
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
                    self.hit(row, pick(side, index));
                }
            }
            Node::Entry(e) => {
                let row = Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h);
                row_bg(self, row);
                let hits = self.entry(e, row, clip);
                let check = hits.check;
                if let Some(index) = target {
                    self.hit(row, pick(side, index));
                    if let Some(b) = check {
                        self.hit(b, Hit::Ui(UiEvent::Check { side, index }));
                    }
                    if e.action.is_some() {
                        let a = Rect::new(row.right() - 50.0, row.y + 3.0, 20.0, 20.0);
                        self.hit(a, pick(side, index + 1));
                    }
                    if let Some(f) = hits.fold {
                        self.hit(f, Hit::Ui(UiEvent::Fold { side, index }));
                    }
                    let first_link = index + 1 + usize::from(e.action.is_some());
                    for (j, l) in hits.links.iter().enumerate() {
                        self.hit(*l, pick(side, first_link + j));
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
                        pick(side, index),
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
                        self.hit(Rect::new(x, r.y + 4.0, bw, 30.0), pick(side, index));
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
                        self.hit(br, pick(side, index));
                    }
                    x += br.w + 6.0;
                }
            }
            Node::Input {
                id,
                placeholder,
                secret,
                multiline,
            } => {
                let typed = sv
                    .inputs
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, t)| t.as_str())
                    .unwrap_or_default();
                let face = Face::ui(13.0);
                let (box_h, text_w) = if *multiline {
                    let tw = (r.w - 40.0).max(40.0);
                    let th = if typed.is_empty() {
                        self.text.height("M", face)
                    } else {
                        self.text.wrapped_height(typed, face, tw)
                    };
                    ((th + 12.0).clamp(30.0, 120.0), tw)
                } else {
                    (30.0, (r.w - 40.0).max(40.0))
                };
                let b = Rect::new(r.x + 10.0, r.y + 5.0, r.w - 20.0, box_h);
                let focused = selected && sv.focused;
                let border = if focused { p.accent } else { p.border };
                self.canvas.outline(b, 7.0, 1.0, border, p.raised);

                let cursor_byte = sv
                    .input_cursors
                    .iter()
                    .find(|(i, _)| i == id)
                    .map(|(_, c)| *c)
                    .unwrap_or(typed.len())
                    .min(typed.len());

                let shown = if *secret {
                    "•".repeat(typed.chars().count())
                } else {
                    typed.to_string()
                };

                let text_clip = b.inset(1.0).intersect(clip);

                if *multiline {
                    if shown.is_empty() {
                        self.text.draw_in_row(
                            &mut self.canvas,
                            placeholder.as_str(),
                            face,
                            b.x + 10.0,
                            b.y,
                            30.0,
                            p.muted,
                            text_clip,
                        );
                        if focused {
                            self.canvas
                                .fill(Rect::new(b.x + 10.0, b.y + 7.0, 1.5, 16.0), p.accent);
                        }
                    } else {
                        let (cx, cy, line_h) =
                            self.text
                                .cursor_pos(&shown, face, Some(text_w), cursor_byte);
                        let text_h = self.text.wrapped_height(&shown, face, text_w);
                        let visible_h = b.h - 12.0;
                        let scroll_y = state.input_scroll.entry(id.clone()).or_default();
                        if text_h > visible_h {
                            if cy < *scroll_y {
                                *scroll_y = cy;
                            } else if cy + line_h > *scroll_y + visible_h {
                                *scroll_y = cy + line_h - visible_h;
                            }
                            *scroll_y = scroll_y.clamp(0.0, text_h - visible_h);
                        } else {
                            *scroll_y = 0.0;
                        }
                        self.text.draw(
                            &mut self.canvas,
                            &shown,
                            face,
                            b.x + 10.0,
                            b.y + 6.0 - *scroll_y,
                            p.text,
                            text_clip,
                            Some(text_w),
                        );
                        if focused {
                            let cx_draw = b.x + 10.0 + cx;
                            let cy_draw = b.y + 6.0 + cy - *scroll_y;
                            let caret = Rect::new(cx_draw, cy_draw, 1.5, line_h.max(14.0))
                                .intersect(b.inset(2.0));
                            if caret.w > 0.0 && caret.h > 0.0 {
                                self.canvas.fill(caret, p.accent);
                            }
                        }
                    }
                } else {
                    let (label, colour) = if shown.is_empty() {
                        (placeholder.as_str(), p.muted)
                    } else {
                        (shown.as_str(), p.text)
                    };
                    let _tw = self.text.draw_in_row(
                        &mut self.canvas,
                        label,
                        face,
                        b.x + 10.0,
                        b.y,
                        b.h,
                        colour,
                        text_clip,
                    );
                    if focused {
                        let cx = if shown.is_empty() {
                            b.x + 10.0
                        } else {
                            let prefix = &shown[..cursor_byte.min(shown.len())];
                            b.x + 10.0 + self.text.width(prefix, face)
                        };
                        self.canvas
                            .fill(Rect::new(cx, b.y + 7.0, 1.5, b.h - 14.0), p.accent);
                    }
                }

                if let Some(index) = target {
                    self.hit(b, pick(side, index));
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
                        self.hit(Rect::new(x, b.y + 49.0, bw, 30.0), pick(side, index));
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
                    self.hit(b, pick(side, index));
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
        if e.id.starts_with("plugin:") {
            return self.plugin_entry(e, row, clip);
        }
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
                self.centred(&g, Face::mono(13.0), a, p.bright, clip);
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
            self.centred(&g, Face::mono(13.0), a, p.bright, clip);
        }
        hits
    }

    /// A two-line card for a plugin in the Plugins sidebar: a tinted monogram
    /// badge on the left, bold title and status pill on the first line, and
    /// full-width description on the second line.
    fn plugin_entry(&mut self, e: &Entry, row: Rect, clip: Rect) -> EntryHits {
        let p = self.p;
        let mid = row.y + row.h / 2.0;
        let tile_size = if self.compact { 28.0 } else { 32.0 };
        let tile = Rect::new(row.x + 8.0, mid - tile_size / 2.0, tile_size, tile_size);
        let (letters, hue) = match (&e.monogram, e.icon) {
            (Some(letters), Some((_, hue))) => (letters.clone(), hue),
            _ => merlin_core::plugin_mark(&e.label, None, None, false),
        };
        let accent = if e.badge_tone == Tone::Muted {
            p.muted
        } else {
            theme::hue(&p, hue)
        };
        self.canvas.round(tile, 7.0, mix(accent, p.island, 0.16));
        let mark_face = Face::ui((tile_size * 0.38).round()).bold();
        let mw = self.text.width(&letters, mark_face);
        self.text.draw_in_row(
            &mut self.canvas,
            &letters,
            mark_face,
            tile.x + (tile.w - mw) / 2.0,
            tile.y,
            tile.h,
            accent,
            clip,
        );

        let text_x = tile.right() + 10.0;
        let right = row.right() - 8.0;
        let top_y = row.y + if self.compact { 5.0 } else { 7.0 };
        let bot_y = row.y + if self.compact { 22.0 } else { 25.0 };

        let mut badge_left = right;
        if !e.badge.is_empty() {
            let b_face = Face::ui(10.5).bold();
            let bw = self.text.width(&e.badge, b_face);
            let pill = Rect::new(right - bw - 10.0, top_y + 1.0, bw + 10.0, 16.0);
            let (bg, fg) = match e.badge_tone {
                Tone::Badge | Tone::Normal => (p.chip, p.muted),
                Tone::Muted => (p.chip, p.muted),
                tone => {
                    let c = tone_colour(&p, tone);
                    (mix(c, p.island, 0.18), c)
                }
            };
            self.canvas.round(pill, 5.0, bg);
            self.text.draw_in_row(
                &mut self.canvas,
                &e.badge,
                b_face,
                pill.x + 5.0,
                pill.y,
                pill.h,
                fg,
                clip,
            );
            badge_left = pill.x - 6.0;
        }

        let title_face = Face::ui(13.0).bold();
        let title_room = (badge_left - text_x).max(0.0);
        let title = self.text.fit(&e.label, title_face, title_room);
        let title_fg = if e.badge_tone == Tone::Muted {
            p.muted
        } else {
            p.bright
        };
        self.text.draw_in_row(
            &mut self.canvas,
            &title,
            title_face,
            text_x,
            top_y,
            18.0,
            title_fg,
            clip,
        );

        if !e.detail.is_empty() {
            let desc_face = Face::ui(11.5);
            let desc_room = (right - text_x).max(0.0);
            let detail = self.text.fit(&e.detail, desc_face, desc_room);
            self.text.draw_in_row(
                &mut self.canvas,
                &detail,
                desc_face,
                text_x,
                bot_y,
                18.0,
                p.muted,
                clip,
            );
        }

        EntryHits::default()
    }
}

/// A click on a side bar's row `index`.
/// A node's code colours, found by where it is in its view.
fn node_colours<'s>(sv: &'s SideView, node: &Node) -> &'s [Spans] {
    sv.nodes
        .iter()
        .position(|n| std::ptr::eq(n, node))
        .and_then(|i| sv.colours.get(i))
        .map_or(&[], |c| c.as_slice())
}

fn pick(side: Side, index: usize) -> Hit {
    Hit::Ui(UiEvent::Target {
        side,
        index,
        clicks: 1,
    })
}

/// A line of the Output panel.
const LOG_ROW_H: f32 = 20.0;

impl Frame<'_> {
    /// A log line: its time, its level in its colour, then its text, in a
    /// fixed-width font and cut at the panel's edge.
    fn log_row(&mut self, l: &merlin_core::LogRow, r: Rect, clip: Rect) {
        let p = self.p;
        if l.tone == Tone::Error {
            self.canvas
                .tint(Rect::new(r.x + 6.0, r.y, r.w - 12.0, r.h), 4.0, p.error, 28);
        }
        let face = Face::mono(12.0);
        let clip = clip.intersect(Rect::new(r.x, r.y, r.w - 14.0, r.h));
        let mut x = r.x + 14.0;
        x += self
            .text
            .draw_in_row(&mut self.canvas, &l.time, face, x, r.y, r.h, p.faint, clip);
        x += 12.0;
        let colour = match l.level.as_str() {
            "info" => p.accent,
            "stderr" => p.info,
            _ => tone_colour(&p, l.tone),
        };
        self.text
            .draw_in_row(&mut self.canvas, &l.level, face, x, r.y, r.h, colour, clip);
        x += self.text.width("stderr", face) + 12.0;
        self.text
            .draw_in_row(&mut self.canvas, &l.text, face, x, r.y, r.h, p.text, clip);
    }
}

impl Frame<'_> {
    /// The Output panel's choices at the right of its tabs: the channel,
    /// with its plugin's tile, and how much to show. Returns where the
    /// controls are, for the open list to hang from.
    /// How wide the Output bar's choices are, from the panel's right.
    pub(super) fn output_bar_width(&mut self, bar: &merlin_core::OutputBarView) -> f32 {
        let face = Face::ui(12.0);
        let mut w = 10.0 + 26.0 + 6.0;
        for (label, mark) in [(&bar.level, false), (&bar.channel, bar.mark.is_some())] {
            let tile = if mark { 20.0 } else { 0.0 };
            w += self.text.width(label, face) + tile + 34.0 + 6.0;
        }
        w
    }

    pub(super) fn output_bar(&mut self, bar: &merlin_core::OutputBarView, panel: Rect) -> Rect {
        let p = self.p;
        let face = Face::ui(12.0);
        let mut x = panel.right() - 10.0;
        let y = panel.y + 7.0;
        let mut first = None;
        // Right to left: Clear, the level, the channel.
        let clear = Rect::new(x - 26.0, y - 1.0, 26.0, 26.0);
        if self.hovered(clear) {
            self.canvas.round(clear, 6.0, p.hover);
        }
        self.text.draw_in_row(
            &mut self.canvas,
            "⊘",
            face,
            clear.x + 8.0,
            clear.y,
            clear.h,
            p.muted,
            panel,
        );
        self.hit(clear, Hit::Command("output.clear"));
        self.tips.push((clear, "Clear the Channel".into()));
        x = clear.x - 6.0;
        for (list, label, mark) in [
            (merlin_core::OutputList::Levels, &bar.level, None),
            (
                merlin_core::OutputList::Channels,
                &bar.channel,
                bar.mark.as_ref(),
            ),
        ] {
            let tile = if mark.is_some() { 20.0 } else { 0.0 };
            let w = self.text.width(label, face) + tile + 34.0;
            let r = Rect::new(x - w, y, w, 24.0);
            x = r.x - 6.0;
            let open = bar.open == Some(list);
            let bg = if self.hovered(r) { p.hover } else { p.raised };
            self.canvas
                .outline(r, 6.0, 1.0, if open { p.accent } else { p.border }, bg);
            let mut tx = r.x + 9.0;
            if let Some(mark) = mark {
                self.plugin_mark(mark, tx, r.y + 4.0, 16.0, true, panel);
                tx += tile;
            }
            self.text
                .draw_in_row(&mut self.canvas, label, face, tx, r.y, r.h, p.text, panel);
            self.text.draw_in_row(
                &mut self.canvas,
                "▾",
                face,
                r.right() - 17.0,
                r.y,
                r.h,
                p.muted,
                panel,
            );
            self.hit(
                r,
                Hit::Ui(UiEvent::Output(merlin_core::OutputClick::Open(list))),
            );
            first.get_or_insert(r);
            if open {
                first = Some(r);
            }
        }
        first.unwrap_or(panel)
    }

    /// The Output bar's open list, floating under its control: channels
    /// with their tiles, line counts and a red dot for errors not seen yet,
    /// or the levels.
    pub(super) fn output_list(
        &mut self,
        bar: &merlin_core::OutputBarView,
        under: Rect,
        w: f32,
        h: f32,
    ) {
        if bar.rows.is_empty() {
            return;
        }
        let p = self.p;
        // A click anywhere else closes it.
        self.hit(
            Rect::new(0.0, 0.0, w, h),
            Hit::Ui(UiEvent::Output(merlin_core::OutputClick::Close)),
        );
        let face = Face::ui(12.5);
        let row_h = 28.0;
        let heads = bar.rows.iter().filter(|r| r.heading.is_some()).count() as f32;
        let list_w = bar
            .rows
            .iter()
            .map(|r| self.text.width(&r.label, face) + 92.0)
            .fold(under.w.max(200.0), f32::max);
        let list_h = bar.rows.len() as f32 * row_h + heads * 26.0 + 9.0 + heads * 9.0;
        let x = (under.right() - list_w).max(8.0);
        let mut y = under.bottom() + 4.0;
        if y + list_h > h - 8.0 {
            y = (under.y - 4.0 - list_h).max(8.0);
        }
        let r = Rect::new(x, y, list_w, list_h);
        self.canvas.shadow(r, 8.0, false);
        self.canvas.outline(r, 8.0, 1.0, p.border, p.widget);
        self.hit(r, Hit::Block);
        let mut iy = r.y + 4.0;
        for (i, row) in bar.rows.iter().enumerate() {
            if let Some(heading) = &row.heading {
                self.canvas
                    .fill(Rect::new(r.x + 6.0, iy + 4.0, r.w - 12.0, 1.0), p.line);
                iy += 9.0;
                self.text.draw_in_row(
                    &mut self.canvas,
                    &heading.to_uppercase(),
                    Face::ui(10.5).bold(),
                    r.x + 14.0,
                    iy,
                    26.0,
                    p.muted,
                    r,
                );
                iy += 26.0;
            }
            let rr = Rect::new(r.x + 4.0, iy, r.w - 8.0, row_h);
            if row.on {
                self.canvas.round(rr, 5.0, p.selected);
            } else if self.hovered(rr) {
                self.canvas.round(rr, 5.0, p.focus_row);
            }
            let mut tx = rr.x + 10.0;
            if let Some(mark) = &row.mark {
                self.plugin_mark(mark, tx, rr.y + 6.0, 16.0, true, r);
                tx += 24.0;
            }
            let fg = if row.on { p.bright } else { p.text };
            self.text
                .draw_in_row(&mut self.canvas, &row.label, face, tx, rr.y, rr.h, fg, r);
            let mut rx = rr.right() - 10.0;
            if let Some(n) = row.count {
                let n = n.to_string();
                let nf = Face::ui(11.0);
                rx -= self.text.width(&n, nf);
                self.text
                    .draw_in_row(&mut self.canvas, &n, nf, rx, rr.y, rr.h, p.muted, r);
                rx -= 10.0;
            }
            if row.unseen_error {
                self.canvas
                    .round(Rect::new(rx - 7.0, rr.y + 10.5, 7.0, 7.0), 3.5, p.error);
            }
            self.hit(
                rr,
                Hit::Ui(UiEvent::Output(merlin_core::OutputClick::Row(i))),
            );
            iy += row_h;
        }
    }
}

/// How large a plugin's picture is drawn at width `inner`: as wide as it
/// fits, never larger than it is, keeping its shape. A box for its words
/// until it's there.
fn image_size(node: &merlin_core::ImageNode, inner: f32) -> (f32, f32) {
    match &node.image {
        Some(i) => {
            let w = inner.min(i.width as f32).max(1.0);
            (w, (w * i.height as f32 / i.width as f32).max(1.0))
        }
        None => (inner, 64.0),
    }
}

impl Frame<'_> {
    /// A plugin's picture with its top left at `at`, or, until the plugin
    /// hands it over, a hatched box with what it shows in words.
    fn plugin_image(&mut self, node: &merlin_core::ImageNode, at: Rect, clip: Rect) {
        let p = self.p;
        let (w, h) = image_size(node, at.w);
        let r = Rect::new(at.x, at.y, w, h);
        let Some(image) = &node.image else {
            self.canvas.round(r, 6.0, p.chip);
            self.hatch(r.inset(1.0).intersect(clip), p.border);
            let f = Face::ui(11.5);
            let tw = self.text.width(&node.alt, f).min(r.w - 24.0);
            let pill = Rect::new(
                r.x + (r.w - tw) / 2.0 - 8.0,
                r.y + r.h / 2.0 - 11.0,
                tw + 16.0,
                22.0,
            );
            self.canvas.round(pill, 5.0, p.island);
            let alt = self.text.fit(&node.alt, f, tw);
            self.text.draw_in_row(
                &mut self.canvas,
                &alt,
                f,
                pill.x + 8.0,
                pill.y,
                22.0,
                p.muted,
                clip,
            );
            return;
        };
        let s = self.canvas.scale;
        let (pw, ph) = ((w * s).round() as u32, (h * s).round() as u32);
        let picture = self
            .pictures
            .plugin(image, pw, ph, 6.0 * s, self.canvas.bitmaps);
        self.canvas.picture(picture, r.x, r.y, None, clip);
    }

    /// Stripes across `r`, as a place still waiting for its picture.
    pub(super) fn hatch(&mut self, r: Rect, colour: u32) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        // Lines where x + y is constant, cut to the box.
        let mut c = r.x + r.y + 10.0;
        while c < r.right() + r.bottom() {
            let x0 = r.x.max(c - r.bottom());
            let x1 = r.right().min(c - r.y);
            if x0 < x1 {
                self.canvas.line((x0, c - x0), (x1, c - x1), 1.0, colour);
            }
            c += 10.0;
        }
    }
}
