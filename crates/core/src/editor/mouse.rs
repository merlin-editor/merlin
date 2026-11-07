//! The mouse over the text: a group's pointer events, in its own grid of
//! cells. Everything else in a window reaches the editor as a UI event.

use super::fold;
use super::{Editor, Effect, Focus, Mouse, MouseKind, Pointer};

impl Editor {
    /// The mouse over a group's text in a window. A press focuses the
    /// group; the rest is as a click in the text's own grid, at the nearest
    /// gap between characters for presses and drags.
    pub(crate) fn pointer(&mut self, p: Pointer, fx: &mut Vec<Effect>) {
        // Scripts say `usize::MAX` for the focused group.
        let g = if p.group == usize::MAX {
            self.group
        } else {
            p.group.min(self.groups.len().saturating_sub(1))
        };
        if matches!(p.kind, MouseKind::Down | MouseKind::Context) {
            self.rename_box = None;
            self.peek = None;
            self.focus_group(g);
        }
        let here = g == self.group;
        let gutter = self.gutter() as f32;
        let x = if p.col < gutter || matches!(p.kind, MouseKind::Move | MouseKind::Rest) {
            p.col.floor()
        } else {
            (p.col + 0.5).floor()
        };
        let (x, y) = (
            x.clamp(0.0, u16::MAX as f32) as u16,
            p.row.floor().clamp(0.0, u16::MAX as f32) as u16,
        );
        match p.kind {
            // Lit-up fold arrows and hovers are for the focused group.
            MouseKind::Move => {
                self.hover = here.then_some((x, y));
                if here {
                    self.pointer_moved(x, y);
                    self.link_moved(x, y, p.ctrl);
                }
            }
            // With Ctrl down, a rest shows the definition instead.
            MouseKind::Rest if here && self.link_span().is_some() => self.link_rest(fx),
            MouseKind::Rest if here => self.hover_at(x, y, fx),
            MouseKind::Rest => {}
            MouseKind::Context => self.open_context(x, y),
            MouseKind::Middle => {
                self.focus_group(g);
                self.focus = super::Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(x, y);
                self.set_cursor(pos);
                fx.push(Effect::ReadSelection);
            }
            // A press on the lightbulb lists the code actions.
            MouseKind::Down if here && self.on_bulb(x, y) => {
                self.popup = None;
                self.code_actions(fx);
            }
            // Elsewhere in that room, a press sets or clears a breakpoint,
            // as in VS Code's glyph margin.
            MouseKind::Down if here && self.break_line(x, y).is_some() => {
                if let (Some(line), Some(path)) =
                    (self.break_line(x, y), self.doc().buffer.path.clone())
                {
                    self.toggle_breakpoint(&path, line as u32, fx);
                }
            }
            kind => {
                // A drag goes on in the group it began in.
                if !here && kind != MouseKind::Down {
                    return;
                }
                self.hover = Some((x, y));
                self.mouse(
                    Mouse {
                        kind,
                        x,
                        y,
                        shift: p.shift,
                        alt: p.alt,
                        ctrl: p.ctrl,
                        clicks: p.clicks,
                    },
                    fx,
                );
            }
        }
    }

    /// The line at screen row `y`, if it has a git sign.
    fn sign_line(&self, y: u16) -> Option<usize> {
        let text = self.layout().text;
        let pos = self.pos_at_screen(text.x + self.gutter() as u16, y);
        let line = self.doc().line_col(pos).0;
        self.doc()
            .git_signs
            .get(line)
            .copied()
            .flatten()
            .map(|_| line)
    }

    /// Whether cell (`x`, `y`) is the lightbulb: its room in the gutter,
    /// after the sign, on its line.
    fn on_bulb(&self, x: u16, y: u16) -> bool {
        let text = self.layout().text;
        let Some(line) = self.bulb_line() else {
            return false;
        };
        let from = text.x + self.blame_cols() as u16 + self.has_signs() as u16;
        if !text.contains(x, y) || x < from || x >= from + self.bulb_room() as u16 {
            return false;
        }
        let pos = self.pos_at_screen(text.x + self.gutter() as u16, y);
        self.doc().line_col(pos).0 == line
    }

    /// The line whose breakpoint room cell (`x`, `y`) is in: the gutter's
    /// cells left of the numbers, after the sign.
    pub(crate) fn break_line(&self, x: u16, y: u16) -> Option<usize> {
        let text = self.layout().text;
        let from = text.x + self.blame_cols() as u16 + self.has_signs() as u16;
        let room = self.bulb_room() as u16;
        if room == 0
            || !text.contains(x, y)
            || x < from
            || x >= from + room
            || !self.can_break_here()
        {
            return None;
        }
        let pos = self.pos_at_screen(text.x + self.gutter() as u16, y);
        let line = self.doc().line_col(pos).0;
        // Past the last line is no line.
        let rows = self.doc().buffer.rope().len_lines();
        (line < rows).then_some(line)
    }

    /// Does `f` in group `g`, without focusing it.
    pub(crate) fn in_group(&mut self, g: usize, f: impl FnOnce(&mut Self)) {
        if g >= self.groups.len() {
            return;
        }
        let back = (self.group, self.focus);
        self.focus_group(g);
        f(self);
        self.focus_group(back.0);
        self.focus = back.1;
    }

    /// Scrolls group `g` by `rows` and `cols`, without focusing it.
    pub(crate) fn scroll_group(&mut self, g: usize, rows: f32, cols: f32) {
        self.in_group(g, |e| {
            if rows != 0.0 {
                e.scroll_by(rows as f64);
            }
            if cols != 0.0 && !e.wrapping() {
                e.sideways += cols;
                let whole = e.sideways.trunc();
                e.sideways -= whole;
                let widest = e.widest_on_screen();
                let doc = e.doc_mut();
                doc.hscroll = doc
                    .hscroll
                    .saturating_add_signed(whole as isize)
                    .min(widest.saturating_sub(10));
            }
        });
    }

    /// The display width of the longest line in and around the view.
    fn widest_on_screen(&self) -> usize {
        let doc = self.doc();
        (doc.scroll..(doc.scroll + 200).min(doc.last_line() + 1))
            .map(|l| doc.display_col(l, doc.buffer.line_len(l)))
            .max()
            .unwrap_or(0)
    }

    /// The mouse in the focused group's text, in cells of its grid.
    pub(crate) fn mouse(&mut self, m: Mouse, fx: &mut Vec<Effect>) {
        if m.kind == MouseKind::Move {
            return self.pointer_moved(m.x, m.y);
        }
        if m.kind == MouseKind::Down {
            // A press anywhere puts away what floats over the text.
            self.popup = None;
            self.rename_box = None;
            self.probe = None;
            self.hover_anchor = None;
            self.close_context();
            self.menu = None;
            if self.palette.is_some() {
                self.close_palette();
            }
            self.clicks = m.clicks.max(1);
        }
        let text = self.layout().text;
        match m.kind {
            MouseKind::Up => {
                self.dragging = false;
                self.box_from = None;
                self.drag_unit = None;
                return;
            }
            MouseKind::Drag => {
                if self.dragging {
                    let edge = text.x + (self.gutter() + self.text_columns()) as u16;
                    let x = m.x.clamp(text.x, edge.saturating_sub(1));
                    let pos = self.pos_at_screen(x, m.y);
                    if self.box_from.is_some() {
                        let line = self.doc().line_col(pos).0;
                        return self.select_box((line, self.column_at_screen(x)));
                    }
                    if self.drag_unit.is_some() {
                        return self.drag_units(pos);
                    }
                    let doc = self.doc_mut();
                    let cursor = doc.cursor;
                    doc.anchor.get_or_insert(cursor);
                    doc.cursor = pos;
                }
                return;
            }
            _ => {}
        }
        // Two files compared side by side: the half clicked is the one
        // typing edits.
        if m.kind == MouseKind::Down && text.contains(m.x, m.y) && self.in_compare() {
            self.compare_on_left =
                self.split_rows().is_some() && m.x < text.x + text.w.saturating_sub(2) / 2;
        }
        match m.kind {
            MouseKind::Down if text.contains(m.x, m.y) && self.sticky_at(text, m.y).is_some() => {
                if let Some(line) = self.sticky_at(text, m.y) {
                    self.go_to_block(line);
                }
            }
            // A git bar in the gutter opens its change in place.
            MouseKind::Down
                if text.contains(m.x, m.y)
                    && m.x as usize == text.x as usize + self.blame_cols()
                    && self.has_signs()
                    && self.doc().review.is_none()
                    && self.sign_line(m.y).is_some() =>
            {
                if let Some(line) = self.sign_line(m.y) {
                    self.toggle_change_peek(line);
                }
            }
            // The arrow left of a block's first line folds or unfolds it.
            MouseKind::Down
                if text.contains(m.x, m.y)
                    && m.x as usize + 1 == text.x as usize + self.gutter()
                    && self.doc().review.is_none() =>
            {
                let pos = self.pos_at_screen(text.x + self.gutter() as u16, m.y);
                let line = self.doc().line_col(pos).0;
                if fold::block_end(self.doc(), line).is_some() {
                    self.toggle_fold(line);
                } else {
                    self.focus = Focus::Editor;
                    self.clear_carets();
                    let doc = self.doc_mut();
                    doc.cursor = pos;
                    doc.anchor = None;
                    doc.goal = None;
                }
            }
            // Shift+Alt+drag selects a box, a cursor a line.
            MouseKind::Down if text.contains(m.x, m.y) && m.alt && m.shift => {
                self.focus = Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(m.x, m.y);
                let line = self.doc().line_col(pos).0;
                self.box_from = Some((line, self.column_at_screen(m.x)));
                self.set_cursor(pos);
                self.dragging = true;
            }
            // Ctrl+click goes to the definition, as in every editor with a
            // language server, and Ctrl+Alt+click opens it to the side.
            MouseKind::Down if text.contains(m.x, m.y) && m.ctrl => {
                self.focus = Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(m.x, m.y);
                self.set_cursor(pos);
                self.link_click(m.x, m.y, m.alt, fx);
            }
            MouseKind::Down if text.contains(m.x, m.y) && m.alt => {
                self.focus = Focus::Editor;
                let pos = self.pos_at_screen(m.x, m.y);
                self.add_caret_at(pos);
            }
            MouseKind::Down if text.contains(m.x, m.y) && m.clicks >= 2 && !m.shift => {
                self.focus = Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(m.x, m.y);
                self.select_unit(pos, m.clicks);
                let (start, end) = self.unit_at(pos, m.clicks);
                self.drag_unit = Some((start, end, m.clicks));
                self.dragging = true;
            }
            MouseKind::Down if text.contains(m.x, m.y) => {
                self.focus = Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(m.x, m.y);
                let doc = self.doc_mut();
                if m.shift {
                    let cursor = doc.cursor;
                    doc.anchor.get_or_insert(cursor);
                } else {
                    doc.anchor = None;
                }
                doc.cursor = pos;
                doc.goal = None;
                self.dragging = true;
            }
            _ => {}
        }
    }

    /// Selects the word at `pos` on a double click, or its line on a triple
    /// click.
    fn select_unit(&mut self, pos: usize, clicks: u8) {
        let (start, end) = self.unit_at(pos, clicks);
        let doc = self.doc_mut();
        doc.anchor = Some(start);
        doc.cursor = end;
        doc.goal = None;
    }

    /// The text column under screen column `x`, past the line's end too,
    /// as a box selection needs.
    fn column_at_screen(&self, x: u16) -> usize {
        let text = self.layout().text;
        let hscroll = if self.wrapping() {
            0
        } else {
            self.doc().hscroll
        };
        (x.saturating_sub(text.x) as usize).saturating_sub(self.gutter()) + hscroll
    }
}
