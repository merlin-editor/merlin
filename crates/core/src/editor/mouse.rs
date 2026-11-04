//! Mouse input, by the part of the window it lands in.

use super::{Editor, Effect, Focus, Mouse, MouseKind, PALETTE_ROWS, Side, WHEEL_LINES};

/// Columns the wheel scrolls sideways per step.
const WHEEL_COLUMNS: usize = 6;
use super::fold;
use crate::doc::Doc;
use crate::key::{Key, KeyCode};

impl Editor {
    pub(crate) fn mouse(&mut self, m: Mouse, fx: &mut Vec<Effect>) {
        // Moving only changes what's lit up, which `update` records.
        if m.kind == MouseKind::Move {
            return self.pointer_moved(m.x, m.y);
        }
        if m.kind == MouseKind::Down {
            self.popup = None;
            self.probe = None;
            self.hover_anchor = None;
            self.close_context();
            self.clicks = m.clicks.max(1);
        }
        if self.palette.is_some() {
            return self.palette_mouse(m, fx);
        }
        if m.kind == MouseKind::Down
            && (self.menu.is_some() || (m.y == 0 && self.layout().menu.h > 0))
        {
            self.menu_click(m.x, m.y, fx);
            return;
        }
        // Another editor group: a click focuses it, and the wheel scrolls
        // it without taking focus.
        let editor = self.layout().editor;
        let panes = self.pane_rects(editor);
        if panes.len() == self.groups.len()
            && let Some(g) = panes.iter().position(|p| p.contains(m.x, m.y))
            && g != self.group
        {
            match m.kind {
                MouseKind::Down => self.focus_group(g),
                MouseKind::ScrollUp
                | MouseKind::ScrollDown
                | MouseKind::ScrollLeft
                | MouseKind::ScrollRight => {
                    let back = self.group;
                    self.focus_group(g);
                    self.mouse(m, fx);
                    let focus = self.focus;
                    self.focus_group(back);
                    self.focus = focus;
                    return;
                }
                _ => {}
            }
        }
        let layout = self.layout();
        match m.kind {
            MouseKind::Up => {
                if let Some(id) = self.tab_drag.take() {
                    self.drop_tab(id, m.x, m.y);
                }
                self.dragging = false;
                self.box_from = None;
                self.drag_unit = None;
                self.scrollbar_drag = false;
                self.minimap_drag = false;
                return;
            }
            MouseKind::Drag => {
                if self.scrollbar_drag {
                    self.scroll_to_row(m.y);
                } else if self.minimap_drag {
                    self.minimap_scroll(m.y, true);
                } else if self.dragging {
                    let text = layout.text;
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
        if m.kind == MouseKind::Down && layout.status.contains(m.x, m.y) {
            return self.status_click(m.x, fx);
        }
        if layout.activity.is_some_and(|a| a.contains(m.x, m.y)) {
            if m.kind == MouseKind::Down {
                self.activity_click(m.y, fx);
            }
            return;
        }
        for (side, area) in [
            (Side::Left, layout.left),
            (Side::Right, layout.right),
            (Side::Bottom, layout.bottom),
        ] {
            if area.is_some_and(|a| a.contains(m.x, m.y)) {
                return self.panel_mouse(side, m, fx);
            }
        }
        let wheel = |doc: &mut Doc, up: bool| {
            doc.scroll = if up {
                doc.scroll.saturating_sub(WHEEL_LINES)
            } else {
                (doc.scroll + WHEEL_LINES).min(doc.last_line())
            };
        };
        let text = layout.text;
        match m.kind {
            MouseKind::ScrollUp => wheel(self.doc_mut(), true),
            MouseKind::ScrollDown => wheel(self.doc_mut(), false),
            MouseKind::ScrollLeft | MouseKind::ScrollRight if self.wrapping() => {}
            MouseKind::ScrollLeft => {
                let doc = self.doc_mut();
                doc.hscroll = doc.hscroll.saturating_sub(WHEEL_COLUMNS);
            }
            MouseKind::ScrollRight => {
                let doc = self.doc_mut();
                // No further than the longest line on screen allows.
                let widest = (doc.scroll..(doc.scroll + 200).min(doc.last_line() + 1))
                    .map(|l| doc.display_col(l, doc.buffer.line_len(l)))
                    .max()
                    .unwrap_or(0);
                doc.hscroll = (doc.hscroll + WHEEL_COLUMNS).min(widest.saturating_sub(10));
            }
            MouseKind::Down
                if layout.tabs.contains(m.x, m.y) && self.diff_button() == Some(m.x) =>
            {
                if let Some(path) = self.doc().buffer.path.clone() {
                    fx.push(Effect::OpenDiff(path));
                }
            }
            MouseKind::Down if layout.tabs.contains(m.x, m.y) => {
                let hit = self
                    .tab_spans()
                    .into_iter()
                    .find(|(s, e, _)| (*s..*e).contains(&m.x));
                if let Some((_, end, i)) = hit {
                    // The × just before the tab's end closes it.
                    let closing = m.x + 2 == end;
                    self.current = i;
                    self.focus = Focus::Editor;
                    if closing {
                        self.close(false);
                    } else if m.clicks >= 2 {
                        self.keep_open();
                    } else {
                        self.tab_drag = Some(self.docs[i].id);
                    }
                }
            }
            MouseKind::Down if layout.crumbs.contains(m.x, m.y) => {
                self.crumb_click(layout.crumbs, m.x);
            }
            MouseKind::Down
                if text.contains(m.x, m.y)
                    && m.x + 1 < text.right()
                    && self.sticky_at(text, m.y).is_some() =>
            {
                if let Some(line) = self.sticky_at(text, m.y) {
                    self.go_to_block(line);
                }
            }
            MouseKind::Down if self.on_minimap(m.x, m.y) => {
                self.minimap_drag = true;
                self.minimap_scroll(m.y, false);
            }
            MouseKind::Down if text.contains(m.x, m.y) && m.x + 1 == text.right() => {
                self.scrollbar_drag = true;
                self.scroll_to_row(m.y);
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

    /// Drops a dragged tab: among a group's tabs, before the one under the
    /// mouse, or anywhere else in a group, after its tabs.
    fn drop_tab(&mut self, id: u64, x: u16, y: u16) {
        let panes = self.pane_rects(self.layout().editor);
        if panes.len() != self.groups.len() {
            return;
        }
        let Some(g) = panes.iter().position(|p| p.contains(x, y)) else {
            return;
        };
        let on_tabs = y == panes[g].y;
        if g == self.group && !on_tabs {
            return;
        }
        let at = on_tabs.then(|| {
            self.as_group(g, || {
                let spans = self.tab_spans();
                let tabs = self.pane_tabs();
                spans
                    .iter()
                    .find(|(s, e, _)| (*s..*e).contains(&x))
                    .and_then(|(_, _, i)| tabs.iter().position(|t| t == i))
                    .unwrap_or(tabs.len())
            })
        });
        self.move_tab(id, g, at);
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

    /// Scrolls so a click on the scrollbar's row shows that part of the file
    /// in the middle.
    fn scroll_to_row(&mut self, y: u16) {
        let text = self.layout().text;
        let height = text.h.max(1) as usize;
        let row = y.saturating_sub(text.y).min(text.h.saturating_sub(1)) as usize;
        let doc = self.doc_mut();
        let lines = doc.last_line() + 1;
        let line = row * lines / height;
        doc.scroll = line.saturating_sub(height / 2).min(lines.saturating_sub(1));
    }

    fn panel_mouse(&mut self, side: Side, m: Mouse, fx: &mut Vec<Effect>) {
        match m.kind {
            MouseKind::ScrollUp => self.scroll_panel(side, true, WHEEL_LINES),
            MouseKind::ScrollDown => self.scroll_panel(side, false, WHEEL_LINES),
            MouseKind::Down => {
                let Some(view) = self.panel_view(side) else {
                    self.focus = side.focus();
                    return;
                };
                if m.y == view.title_row {
                    if let Some((_, _, name)) = view
                        .titles
                        .into_iter()
                        .find(|(s, e, _)| (*s..*e).contains(&m.x))
                    {
                        match side {
                            Side::Left => self.show_view(&name, fx),
                            Side::Bottom => self.show_bottom(&name),
                            Side::Right => {
                                self.bar_mut(side).active = Some(name);
                                self.focus_panel(side);
                            }
                        }
                    }
                    return;
                }
                let col = m.x.saturating_sub(view.x);
                let hit = view
                    .rows
                    .iter()
                    .filter(|(y, _)| *y == m.y)
                    .flat_map(|(_, row)| &row.hits)
                    .find(|(s, e, _)| (*s..*e).contains(&col))
                    .map(|(_, _, t)| *t);
                match hit {
                    Some(target) => self.click_target(side, target, fx),
                    None => self.focus = side.focus(),
                }
            }
            _ => {}
        }
    }

    fn palette_mouse(&mut self, m: Mouse, fx: &mut Vec<Effect>) {
        let count = self.palette_items().len();
        let (x, width, first) = self.palette_geometry();
        let top = self.palette_top();
        let Some(palette) = &mut self.palette else {
            return;
        };
        match m.kind {
            MouseKind::ScrollUp => palette.selected = palette.selected.saturating_sub(1),
            MouseKind::ScrollDown => {
                palette.selected = (palette.selected + 1).min(count.saturating_sub(1))
            }
            MouseKind::Down => {
                let row = m.y.saturating_sub(top) as usize;
                let inside = (x..x + width).contains(&m.x) && m.y >= top;
                if inside && (1..=PALETTE_ROWS).contains(&row) && first + row - 1 < count {
                    palette.selected = first + row - 1;
                    self.palette_key(Key::plain(KeyCode::Enter), None, fx);
                } else if !(inside && row == 0) {
                    self.close_palette();
                }
            }
            _ => {}
        }
    }
}
