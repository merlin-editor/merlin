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
                }
            }
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
        match m.kind {
            MouseKind::Down if text.contains(m.x, m.y) && self.sticky_at(text, m.y).is_some() => {
                if let Some(line) = self.sticky_at(text, m.y) {
                    self.go_to_block(line);
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
            // language server.
            MouseKind::Down if text.contains(m.x, m.y) && m.ctrl => {
                self.focus = Focus::Editor;
                self.clear_carets();
                let pos = self.pos_at_screen(m.x, m.y);
                self.set_cursor(pos);
                let has = self
                    .plugins
                    .iter()
                    .any(|p| p.name == "lsp" && p.commands.iter().any(|c| c.id == "definition"));
                if has {
                    fx.push(Effect::PluginCommand {
                        plugin: "lsp".into(),
                        command: "definition".into(),
                    });
                }
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
