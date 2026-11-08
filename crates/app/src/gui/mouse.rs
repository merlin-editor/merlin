//! The mouse in the window: presses, drags, the wheel and touchpads, the
//! scrollbars, tabs dropped where they go, and the pointer's shape.

use super::*;

impl Gui<'_> {
    /// A press on a scrollbar or the minimap: on the thumb it's held where
    /// it was grabbed; elsewhere the view jumps there and it's held by its
    /// middle.
    pub(super) fn press_bar(&mut self, g: usize, what: Bar, at: (f32, f32)) {
        let (Some(area), Some(v)) = (self.areas.groups.get(g).copied(), self.view_of(g)) else {
            return;
        };
        let grab = match what {
            Bar::Down => {
                let Some(t) = overview::vthumb(area.bar, &v) else {
                    return;
                };
                if t.rect.contains(at.0, at.1) {
                    at.1 - t.rect.y
                } else {
                    t.rect.h / 2.0
                }
            }
            Bar::Along => {
                let track = overview::htrack(area.text, &v, self.cell_w());
                let Some(t) = overview::hthumb(track, &v) else {
                    return;
                };
                if t.rect.contains(at.0, at.1) {
                    at.0 - t.rect.x
                } else {
                    t.rect.w / 2.0
                }
            }
            Bar::Map => {
                let (Some(map), Some(rect)) = (&v.minimap, area.minimap) else {
                    return;
                };
                let geom = overview::Map::new(rect, &v, map);
                if geom.slider.contains(at.0, at.1) {
                    at.1 - geom.slider.y
                } else {
                    // A click elsewhere centres the view on that line.
                    let line = geom.line_at(rect, at.1);
                    let row = line.saturating_sub(v.area.h as usize / 2) as f64;
                    self.dispatch(Event::ScrollTo { group: g, row });
                    self.bar_drag = Some(BarDrag {
                        group: g,
                        what,
                        grab: geom.slider.h / 2.0,
                    });
                    return;
                }
            }
        };
        self.bar_drag = Some(BarDrag {
            group: g,
            what,
            grab,
        });
        self.drag_bar(at);
    }

    /// A held bar follows the mouse.
    pub(super) fn drag_bar(&mut self, at: (f32, f32)) {
        let Some(drag) = self.bar_drag else { return };
        let g = drag.group;
        let (Some(area), Some(v)) = (self.areas.groups.get(g).copied(), self.view_of(g)) else {
            return;
        };
        self.show_bars(g);
        match drag.what {
            Bar::Down => {
                if let Some(t) = overview::vthumb(area.bar, &v) {
                    let row = t.position(at.1 - drag.grab - area.bar.y);
                    self.dispatch(Event::ScrollTo { group: g, row });
                }
            }
            Bar::Along => {
                let track = overview::htrack(area.text, &v, self.cell_w());
                if let Some(t) = overview::hthumb(track, &v) {
                    let col = t.position(at.0 - drag.grab - track.x).round();
                    let cols = (col - v.hscroll as f64) as f32;
                    if cols != 0.0 {
                        self.dispatch(Event::Scroll {
                            group: g,
                            rows: 0.0,
                            cols,
                        });
                    }
                }
            }
            Bar::Map => {
                if let (Some(map), Some(rect)) = (&v.minimap, area.minimap) {
                    let geom = overview::Map::new(rect, &v, map);
                    let row = geom.position(at.1 - drag.grab - rect.y);
                    self.dispatch(Event::ScrollTo { group: g, row });
                }
            }
        }
    }

    /// Group `g`'s thumbs show for a while.
    pub(super) fn show_bars(&mut self, g: usize) {
        let until = Instant::now() + BARS_LINGER;
        match self.bars_until.iter_mut().find(|(group, _)| *group == g) {
            Some(entry) => entry.1 = until,
            None => self.bars_until.push((g, until)),
        }
    }

    /// The mouse near a group's code or bars shows its thumbs.
    pub(super) fn hover_bars(&mut self, at: (f32, f32)) {
        let near = self.areas.groups.iter().position(|a| {
            let right = a.bar.right();
            Rect::new(a.text.x, a.text.y, right - a.text.x, a.text.h).contains(at.0, at.1)
        });
        if let Some(g) = near {
            self.show_bars(g);
            self.redraw();
        }
    }

    /// How group `g`'s bars look now.
    pub(super) fn bars_shown(&self, g: usize) -> overview::Shown {
        let dragging = self.bar_drag.is_some_and(|d| d.group == g);
        let alpha = match self.bars_until.iter().find(|(group, _)| *group == g) {
            _ if dragging => 1.0,
            Some(&(_, until)) => {
                let now = Instant::now();
                if now < until {
                    1.0
                } else {
                    1.0 - (now - until).as_secs_f32() / BARS_FADE.as_secs_f32()
                }
            }
            None => 0.0,
        }
        .clamp(0.0, 1.0);
        let area = self.areas.groups.get(g).copied().unwrap_or_default();
        let on = |r: Rect| self.pointer.is_some_and(|(x, y)| r.contains(x, y));
        overview::Shown {
            alpha,
            thumb_hovered: on(area.bar),
            dragging,
            map_hovered: area.minimap.is_some_and(on),
        }
    }

    /// When a fading thumb next needs drawing.
    pub(super) fn bars_due(&self) -> Option<Instant> {
        let now = Instant::now();
        self.bars_until
            .iter()
            .map(|&(_, until)| until)
            .filter(|&until| until + BARS_FADE > now)
            .map(|until| {
                if until > now {
                    until
                } else {
                    now + Duration::from_millis(16)
                }
            })
            .min()
    }

    /// The group whose code is under a point.
    pub(super) fn group_at(&self, (x, y): (f32, f32)) -> Option<usize> {
        self.areas.groups.iter().position(|g| g.text.contains(x, y))
    }

    /// A point in group `g`'s code as the editor counts it: rows from the
    /// top of the view, the part scrolled off included, and columns from
    /// the group's left edge.
    pub(super) fn text_point(&self, g: usize, (x, y): (f32, f32)) -> (f32, f32) {
        let (Some(grid), Some(area)) = (&self.grid, self.areas.groups.get(g)) else {
            return (0.0, 0.0);
        };
        let s = self.scale();
        let frac = self
            .drawn_text
            .iter()
            .find(|v| v.group == g)
            .map_or(0.0, |v| v.top_frac);
        let a = area.text;
        (
            (y - a.y) * s / grid.cell_h as f32 + frac,
            (x - a.x) * s / grid.cell_w as f32,
        )
    }

    /// Tells the editor what the mouse did in group `g`'s code.
    pub(super) fn pointer_event(&mut self, g: usize, kind: MouseKind, at: (f32, f32), clicks: u8) {
        let (row, col) = self.text_point(g, at);
        self.dispatch(Event::Pointer(Pointer {
            group: g,
            kind,
            row,
            col,
            shift: self.modifiers.shift_key(),
            alt: self.modifiers.alt_key(),
            ctrl: self.modifiers.control_key(),
            clicks,
        }));
    }

    /// The cell of group `g`'s code a point is nearest the gap before, as
    /// a press or drag picks it.
    pub(super) fn gap_cell(&self, g: usize, at: (f32, f32)) -> (i32, i32) {
        let (row, col) = self.text_point(g, at);
        (row.floor() as i32, (col + 0.5).floor() as i32)
    }

    /// The topmost clickable part under the mouse.
    pub(super) fn hit_at(&self, (x, y): (f32, f32)) -> Option<(usize, &Hit)> {
        self.hits
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (r, _))| r.contains(x, y))
            .map(|(i, (_, hit))| (i, hit))
    }

    pub(super) fn press(&mut self, at: (f32, f32)) {
        let clicks = self.clicks.count(at);
        let Some((_, hit)) = self.hit_at(at) else {
            return;
        };
        match hit.clone() {
            Hit::Text => {
                if let Some(g) = self.group_at(at) {
                    self.pressed_group = Some(g);
                    self.dragged_to = Some(self.gap_cell(g, at));
                    self.pointer_event(g, MouseKind::Down, at, clicks);
                }
            }
            Hit::Scrollbar { group, horizontal } => {
                let what = if horizontal { Bar::Along } else { Bar::Down };
                self.press_bar(group, what, at);
            }
            Hit::Minimap(group) => self.press_bar(group, Bar::Map, at),
            Hit::Surface(id) => {
                self.surface_point(id, at, false);
                self.surface_drag = Some(id);
            }
            Hit::Terminal(id) => {
                self.dispatch(Event::Ui(UiEvent::Terminal(id)));
                if !self.app.terminals.wants_mouse(id)
                    && let Some(cell) = self.term_cell(id, at)
                {
                    self.app
                        .terminals
                        .select(id, cell, terminal::SelectKind::Start(clicks.min(3)));
                    self.term_drag = Some(id);
                    self.redraw();
                }
            }
            Hit::MaximizePanel => {
                if let Some(shown) = &self.window {
                    let h = shown.window.inner_size().height as f32 / self.scale();
                    self.sizes.maximize_panel(h);
                }
                self.fit_text();
                self.redraw();
            }
            // A double click puts the groups back to the same sizes.
            Hit::GroupSash(_) if clicks >= 2 => {
                let even = vec![(1.0, 1.0); self.areas.groups.len()];
                self.dispatch(Event::Ui(UiEvent::GroupSizes(even)));
            }
            Hit::GroupSash(sash) => {
                self.group_drag = Some((sash, at, self.areas.groups.clone()));
            }
            // A plugin view's icon or tab: a click on release, unless it's
            // dragged to another place.
            Hit::Ui(event @ (UiEvent::Rail(_) | UiEvent::PanelTitle { .. }))
                if self.hold_view(&event, at) => {}
            Hit::Ui(UiEvent::Tab { index, .. }) => {
                self.hold_tab(None, index, at);
                self.dispatch(Event::Ui(UiEvent::Tab { index, clicks }))
            }
            Hit::Ui(UiEvent::GroupTab { group, index, .. }) => {
                self.hold_tab(Some(group), index, at);
                self.dispatch(Event::Ui(UiEvent::GroupTab {
                    group,
                    index,
                    clicks,
                }))
            }
            // A file opens on release, so it can be dragged first.
            Hit::Ui(UiEvent::Target { side, index, .. })
                if self.hold_entry(side, index, clicks, at) => {}
            Hit::Ui(UiEvent::Target { side, index, .. }) => {
                self.dispatch(Event::Ui(UiEvent::Target {
                    side,
                    index,
                    clicks,
                }))
            }
            Hit::Ui(UiEvent::PeekRow { index, .. }) => {
                self.dispatch(Event::Ui(UiEvent::PeekRow { index, clicks }))
            }
            // A view's ⋯ opens its menu under the pointer.
            Hit::Ui(UiEvent::Press(id)) if id.starts_with("view-menu:") => {
                self.context_at = Some(at);
                self.dispatch(Event::Ui(UiEvent::Press(id)))
            }
            Hit::Ui(event) => self.dispatch(Event::Ui(event)),
            Hit::Command(command) => self.dispatch(Event::Command(command.into())),
            Hit::Key(key) => self.dispatch(Event::Key(key)),
            Hit::Settings(action) => self.settings_press(action, clicks, at),
            Hit::SettingsBody => {
                // A click away from a field being typed in leaves it.
                self.app.settings_ui.stop_editing();
                self.app.settings_ui.plugin_menu = None;
                self.app.settings_ui.open_choice = None;
                self.focus_page(at);
            }
            Hit::Sash(side) => self.sash = Some(side),
            // A double click on a strip's empty end opens a new file there.
            Hit::TabStrip(group) if clicks >= 2 => {
                let view = self.app.editor.window_view();
                let g = group.unwrap_or(0);
                if let Some(index) = view
                    .groups
                    .get(g)
                    .and_then(|g| g.tabs.iter().position(|t| t.active))
                {
                    self.dispatch(Event::Ui(UiEvent::GroupTab {
                        group: g,
                        index,
                        clicks: 1,
                    }));
                }
                self.dispatch(Event::Command("file.new".into()));
            }
            Hit::Side(_) | Hit::Block | Hit::TabStrip(_) => {}
        }
    }

    /// The cell of terminal `id` under `at`, kept inside its screen.
    pub(super) fn term_cell(&self, id: u64, at: (f32, f32)) -> Option<(usize, usize)> {
        let (_, r) = self.term_panes.iter().find(|(i, _)| *i == id)?;
        let grid = self.grid.as_ref()?;
        let s = self.scale();
        let col = ((at.0 - r.x) * s / grid.cell_w as f32).max(0.0) as usize;
        let line = ((at.1 - r.y) * s / grid.cell_h as f32).max(0.0) as usize;
        let cols = (r.w * s / grid.cell_w as f32).floor().max(1.0) as usize;
        let lines = (r.h * s / grid.cell_h as f32).floor().max(1.0) as usize;
        Some((col.min(cols - 1), line.min(lines - 1)))
    }

    /// Remembers the tab pressed on, so moving the mouse can drag it.
    pub(super) fn hold_tab(&mut self, group: Option<usize>, index: usize, at: (f32, f32)) {
        let view = self.app.editor.window_view();
        let tabs = match group {
            Some(g) => view.groups.get(g).map(|g| &g.tabs),
            None => Some(&view.tabs),
        };
        if let Some(tab) = tabs.and_then(|t| t.get(index)) {
            self.tab_drag = Some(TabDrag {
                name: tab.name.clone(),
                icon: Some(tab.icon),
                from: at,
                moved: false,
                entry: None,
                view: None,
            });
        }
    }

    /// A file in a side's view pressed on, its `index`th element: held, to
    /// drag into the editor, if it's a file. Whether it was.
    pub(super) fn hold_entry(
        &mut self,
        side: Side,
        index: usize,
        clicks: u8,
        at: (f32, f32),
    ) -> bool {
        let view = self.app.editor.window_view();
        let sv = match side {
            Side::Left => view.left.as_ref(),
            Side::Right => view.right.as_ref(),
            Side::Bottom => view.bottom.as_ref(),
            Side::Tab => view.pages.iter().map(|(_, v)| v).find(|v| v.focused),
        };
        let Some(sv) = sv else { return false };
        let file = sv
            .nodes
            .iter()
            .zip(&sv.targets)
            .find_map(|(node, t)| match node {
                Node::Entry(e) if *t == Some(index) && e.id.starts_with("f:") => Some(e),
                _ => None,
            });
        let Some(file) = file else { return false };
        self.tab_drag = Some(TabDrag {
            name: file.label.clone(),
            icon: file.icon,
            from: at,
            moved: false,
            entry: Some((side, index, clicks)),
            view: None,
        });
        true
    }

    /// A plugin view's rail icon or tab pressed on: held, so it can be
    /// dragged to another place. Whether it's a view's.
    pub(super) fn hold_view(&mut self, click: &UiEvent, at: (f32, f32)) -> bool {
        let editor = &self.app.editor;
        let view = editor.window_view();
        let (name, title) = match click {
            UiEvent::Rail(i) => match view.rail.get(*i) {
                Some(item) if item.view.is_some() => (item.view.clone(), item.label.clone()),
                _ => return false,
            },
            UiEvent::PanelTitle { side, name } if editor.is_plugin_view(name) => {
                let sv = match side {
                    Side::Right => view.right.as_ref(),
                    _ => view.bottom.as_ref(),
                };
                let title = sv
                    .and_then(|sv| sv.panels.iter().find(|t| t.name == *name))
                    .map_or(name.clone(), |t| t.title.clone());
                (Some(name.clone()), title)
            }
            _ => return false,
        };
        let Some(name) = name else { return false };
        self.tab_drag = Some(TabDrag {
            name: title,
            icon: None,
            from: at,
            moved: false,
            entry: None,
            view: Some((name, click.clone())),
        });
        true
    }

    /// Where a plugin view dragged to `at` would go: a side bar or the
    /// panel it's over, the edge where a hidden one would open, or the
    /// editor, as a tab.
    fn view_landing(&self, at: (f32, f32)) -> Option<Landing> {
        use merlin_core::Place;
        let a = &self.areas;
        let e = a.editor;
        let over = |r: &Option<Rect>| r.filter(|r| r.contains(at.0, at.1));
        let (place, zone) = if let Some(r) = over(&a.right) {
            (Place::Right, r)
        } else if let Some(r) = over(&a.bottom) {
            (Place::Bottom, r)
        } else if let Some(r) = over(&a.left) {
            (Place::Left, r)
        } else if at.0 < e.x {
            let zone = a
                .left
                .unwrap_or(Rect::new(e.x, e.y, 260.0_f32.min(e.w / 3.0), e.h));
            (Place::Left, zone)
        } else if e.contains(at.0, at.1) {
            // Near an edge whose side is hidden: that side.
            let edge = 80.0;
            let w = 300.0_f32.min(e.w / 3.0);
            let h = 220.0_f32.min(e.h / 3.0);
            if a.right.is_none() && at.0 > e.right() - edge {
                (Place::Right, Rect::new(e.right() - w, e.y, w, e.h))
            } else if a.bottom.is_none() && at.1 > e.bottom() - edge {
                (Place::Bottom, Rect::new(e.x, e.bottom() - h, e.w, h))
            } else if a.left.is_none() && at.0 < e.x + edge {
                (Place::Left, Rect::new(e.x, e.y, w, e.h))
            } else {
                (Place::Tab, e)
            }
        } else {
            return None;
        };
        Some(Landing::View { place, zone })
    }

    /// Where a tab or file dragged to `at` would land: on a tab or the
    /// strip, or on a group's code, on the side it's nearest if it's near
    /// one. A tab dropped on its own group's middle, or its group's only
    /// tab on any of it, goes nowhere new.
    pub(super) fn drop_at(&self, at: (f32, f32)) -> Option<Landing> {
        let view = self.app.editor.window_view();
        let focused = view.groups.iter().position(|g| g.focused).unwrap_or(0);
        let strip = match self.hit_at(at).map(|(_, h)| h.clone()) {
            Some(Hit::Ui(UiEvent::Tab { index, .. } | UiEvent::CloseTab(index))) => {
                Some((focused, Some(index)))
            }
            Some(Hit::Ui(
                UiEvent::GroupTab { group, index, .. } | UiEvent::CloseGroupTab { group, index },
            )) => Some((group, Some(index))),
            Some(Hit::TabStrip(group)) => Some((group.unwrap_or(focused), None)),
            _ => None,
        };
        if let Some((group, at)) = strip {
            return Some(Landing::Strip { group, at });
        }
        let (x, y) = at;
        let (group, area) = self.areas.groups.iter().enumerate().find(|(_, a)| {
            let body = a.body();
            body.contains(x, y)
        })?;
        let body = area.body();
        let (fx, fy) = ((x - body.x) / body.w, (y - body.y) / body.h);
        let near = [
            (fx, Edge::Left),
            (1.0 - fx, Edge::Right),
            (fy, Edge::Up),
            (1.0 - fy, Edge::Down),
        ]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
        let edge = (near.0 < 0.3).then_some(near.1);
        let dragging_tab = self.tab_drag.as_ref().is_some_and(|d| d.entry.is_none());
        if dragging_tab && group == focused {
            let alone = view.groups.get(group).is_some_and(|g| g.tabs.len() <= 1);
            if edge.is_none() || alone {
                return None;
            }
        }
        let (hw, hh) = (body.w / 2.0, body.h / 2.0);
        let zone = match edge {
            None => body,
            Some(Edge::Left) => Rect::new(body.x, body.y, hw, body.h),
            Some(Edge::Right) => Rect::new(body.x + hw, body.y, hw, body.h),
            Some(Edge::Up) => Rect::new(body.x, body.y, body.w, hh),
            Some(Edge::Down) => Rect::new(body.x, body.y + hh, body.w, hh),
        };
        Some(Landing::Body { group, edge, zone })
    }

    /// A dragged tab or file let go at `at`, where [`Self::drop_at`]
    /// says.
    pub(super) fn drop_tab(&mut self, at: (f32, f32), entry: Option<(Side, usize, u8)>) {
        let Some(target) = self.drop_at(at) else {
            return;
        };
        let event = match (target, entry) {
            (Landing::Strip { group, at }, None) => UiEvent::DropTab { group, at },
            (
                Landing::Body {
                    group, edge: None, ..
                },
                None,
            ) => UiEvent::DropTab { group, at: None },
            (
                Landing::Body {
                    group,
                    edge: Some(edge),
                    ..
                },
                None,
            ) => UiEvent::SplitTab { group, edge },
            (Landing::Strip { group, .. }, Some((side, index, _))) => UiEvent::DropEntry {
                side,
                index,
                group,
                edge: None,
            },
            (Landing::Body { group, edge, .. }, Some((side, index, _))) => UiEvent::DropEntry {
                side,
                index,
                group,
                edge,
            },
            // Only a view's drag lands as a view.
            (Landing::View { .. }, _) => return,
        };
        self.dispatch(Event::Ui(event));
    }

    /// Shapes the pointer for what it's over: the hand on links and on a
    /// name with Ctrl down, the text bar on code and boxes to type in, the
    /// open hand on a tab after a rest, the edges' arrows, and the busy
    /// arrow while something is on its way. Everything else is the arrow.
    pub(super) fn update_pointer(&mut self) {
        // A drag keeps the shape it has.
        if self.tab_drag.as_ref().is_some_and(|d| d.moved)
            || self.sash.is_some()
            || self.group_drag.is_some()
        {
            return;
        }
        let Some(at) = self.pointer else {
            return;
        };
        let over = self.hit_at(at).map(|(i, h)| (i, h.clone()));
        let mark = over.as_ref().and_then(|(i, _)| {
            self.marks
                .iter()
                .find(|(m, _)| m == i)
                .map(|(_, mark)| *mark)
        });
        // The open hand shows once the pointer has rested on a tab.
        match (mark, over.as_ref()) {
            (Some(frame::Mark::Tab), Some((i, _))) => {
                if self.grab.is_none_or(|(g, _)| g != *i) {
                    self.grab = Some((*i, Instant::now() + GRAB_DELAY));
                }
            }
            _ => self.grab = None,
        }
        let icon = match (over.map(|(_, h)| h), mark) {
            (Some(Hit::Text), _) if self.pointer_link => CursorIcon::Pointer,
            (Some(Hit::Text | Hit::Terminal(_)), _) => CursorIcon::Text,
            (Some(Hit::Sash(Side::Bottom)), _) => CursorIcon::RowResize,
            (Some(Hit::Sash(_)), _) => CursorIcon::ColResize,
            (Some(Hit::GroupSash(frame::GroupSash::Column(_))), _) => CursorIcon::ColResize,
            (Some(Hit::GroupSash(frame::GroupSash::Row(_))), _) => CursorIcon::RowResize,
            (_, Some(frame::Mark::Link)) => CursorIcon::Pointer,
            (_, Some(frame::Mark::Input)) => CursorIcon::Text,
            (_, Some(frame::Mark::Tab))
                if self.grab.is_some_and(|(_, when)| when <= Instant::now()) =>
            {
                CursorIcon::Grab
            }
            _ if self.busy => CursorIcon::Progress,
            _ => CursorIcon::Default,
        };
        self.set_pointer(icon);
    }

    /// The pointer's shape for what it's over: an I-beam over the code, a
    /// grabbing hand while dragging a tab.
    pub(super) fn set_pointer(&mut self, icon: CursorIcon) {
        if self.cursor_icon != icon
            && let Some(shown) = &self.window
        {
            shown.window.set_cursor(icon);
            self.cursor_icon = icon;
        }
    }

    pub(super) fn wheel(&mut self, lines: f64, sideways: bool) {
        let Some(at) = self.pointer else {
            return;
        };
        let over = self.hit_at(at).map(|(_, h)| h.clone());
        if matches!(over, Some(Hit::Settings(_) | Hit::SettingsBody)) {
            self.app.settings_scroll(-(lines as f32) * 60.0);
            self.redraw();
            return;
        }
        // Over a document, its pages scroll.
        if let Some(Hit::Surface(id)) = over {
            let by = -(lines as f32) * 60.0 * self.sensitivity();
            let across = sideways || self.modifiers.shift_key();
            self.dispatch(Event::Ui(UiEvent::SurfaceScroll { id, by, across }));
            return;
        }
        // Over a terminal, it scrolls back through the history.
        if let Some(Hit::Terminal(id)) = over {
            let lines = (lines * 3.0).round() as i32;
            if lines != 0 {
                self.app.terminals.scroll(id, lines);
                self.redraw();
            }
            return;
        }
        // Over a tab strip, it scrolls the tabs along.
        let strip = match over {
            Some(Hit::TabStrip(group)) => Some(group.unwrap_or(0)),
            Some(Hit::Ui(
                UiEvent::GroupTab { group, .. } | UiEvent::CloseGroupTab { group, .. },
            )) => Some(group),
            _ => None,
        };
        if let Some(g) = strip {
            if let Some(strip) = self.strips.get_mut(g) {
                strip.0 -= lines as f32 * 40.0;
            }
            self.redraw();
            return;
        }
        let side = match over {
            Some(Hit::Side(side)) => Some(side),
            Some(Hit::Ui(UiEvent::Target { side, .. })) => Some(side),
            _ => None,
        };
        // A side's content scrolls in pixels, three rows a step.
        if let Some(side) = side
            && self.app.editor.window_view().palette.is_none()
        {
            let k = super::frame::side_index(side);
            self.state.scroll[k] -= lines as f32 * 78.0;
            self.redraw();
            return;
        }
        // Three rows a step, or six columns sideways, as VS Code scrolls,
        // times the settings' sensitivity, and again with Alt.
        let group = self.group_at(at).unwrap_or(self.focused_group());
        let steps = lines as f32 * self.sensitivity();
        let (rows, cols) = if sideways || self.modifiers.shift_key() {
            (0.0, -steps * 6.0)
        } else {
            (-steps * 3.0, 0.0)
        };
        self.dispatch(Event::Scroll { group, rows, cols });
    }

    /// A press or drag at `at` on a document's pages: where on which page.
    pub(super) fn surface_point(&mut self, id: u32, at: (f32, f32), drag: bool) {
        let Some(seen) = self.surfaces.iter().find(|s| s.id == id) else {
            return;
        };
        if let Some((page, x, y)) = seen.point(at.0, at.1) {
            self.dispatch(Event::Ui(UiEvent::SurfacePoint {
                id,
                page,
                x,
                y,
                drag,
            }));
        }
    }

    /// A touchpad moved the text by (`dx`, `dy`) physical pixels: the text
    /// follows it exactly, parts of a row and column too.
    pub(super) fn touch_scroll(&mut self, dx: f64, dy: f64) {
        let Some(at) = self.pointer else { return };
        let over_text = matches!(self.hit_at(at), None | Some((_, Hit::Text)));
        let (cw, ch) = self
            .grid
            .as_ref()
            .map_or((10.0, 20.0), |g| (g.cell_w as f64, g.cell_h as f64));
        let group = self.group_at(at).filter(|_| over_text);
        // Off the text: as many wheel steps as it adds up to.
        let Some(group) = group else {
            self.scroll += if dy.abs() >= dx.abs() { dy } else { dx };
            let step = ch * 3.0;
            let steps = (self.scroll / step).trunc();
            if steps != 0.0 {
                self.scroll -= steps * step;
                self.wheel(steps, dy.abs() < dx.abs());
            }
            return;
        };
        self.touched = true;
        let k = self.sensitivity();
        self.dispatch(Event::Scroll {
            group,
            rows: (-dy / ch) as f32 * k,
            cols: (-dx / cw) as f32 * k,
        });
    }

    /// How far the wheel and touchpad scroll the code, times their own.
    pub(super) fn sensitivity(&self) -> f32 {
        let w = &self.app.window;
        if self.modifiers.alt_key() {
            w.wheel_sensitivity * w.fast_sensitivity
        } else {
            w.wheel_sensitivity
        }
    }

    pub(super) fn settings_press(&mut self, action: SettingsAction, clicks: u8, at: (f32, f32)) {
        self.focus_page(at);
        let action = match action {
            SettingsAction::KeyRow(i, _) => SettingsAction::KeyRow(i, clicks),
            other => other,
        };
        if matches!(action, SettingsAction::Theme(_)) {
            self.hover_card = None;
        }
        for event in self.app.settings_action(action) {
            self.app.dispatch(event);
        }
        self.fit_text();
        self.redraw();
    }

    /// The pointer on a theme card previews it on the window; off it,
    /// the theme comes back.
    pub(super) fn hover_settings(&mut self, at: (f32, f32)) {
        let card = match self.hit_at(at) {
            Some((_, Hit::Settings(SettingsAction::Theme(name)))) => Some(name.clone()),
            _ => None,
        };
        if card == self.hover_card || (card.is_none() && self.app.settings_ui.card.is_some()) {
            return;
        }
        self.hover_card = card.clone();
        for event in self.app.preview_card(card) {
            self.app.dispatch(event);
        }
        self.redraw();
    }

    pub(super) fn moved(&mut self, position: PhysicalPosition<f64>) {
        let s = self.scale() as f64;
        let at = ((position.x / s) as f32, (position.y / s) as f32);
        let before = self.pointer.and_then(|p| self.hit_at(p).map(|(i, _)| i));
        self.pointer = Some(at);
        if self.bar_drag.is_some() {
            self.drag_bar(at);
            return;
        }
        self.hover_bars(at);
        self.hover_settings(at);
        if let Some((sash, from, areas)) = &self.group_drag {
            let sizes = group_sizes(*sash, &areas.clone(), (at.0 - from.0, at.1 - from.1));
            self.dispatch(Event::Ui(UiEvent::GroupSizes(sizes)));
            return;
        }
        if let Some(side) = self.sash {
            let s = self.scale();
            if let Some(shown) = &self.window {
                let size = shown.window.inner_size();
                let (w, h) = (size.width as f32 / s, size.height as f32 / s);
                self.sizes.drag(side, at, w, h);
            }
            self.fit_text();
            self.redraw();
            return;
        }
        if let Some(drag) = &mut self.tab_drag {
            let (dx, dy) = (at.0 - drag.from.0, at.1 - drag.from.1);
            if !drag.moved && dx.hypot(dy) > DRAG_START {
                drag.moved = true;
            }
            if drag.moved {
                // What's dragged needs no tip.
                self.tip = None;
                self.drop = if drag.view.is_some() {
                    self.view_landing(at)
                } else {
                    self.drop_at(at)
                };
                // Where it can't go, letting go puts it back.
                self.set_pointer(match self.drop {
                    Some(_) => CursorIcon::Grabbing,
                    None => CursorIcon::NotAllowed,
                });
                self.redraw();
            }
            return;
        }
        if let Some(id) = self.surface_drag {
            self.surface_point(id, at, true);
            return;
        }
        if let Some(id) = self.term_drag {
            if let Some(cell) = self.term_cell(id, at) {
                self.app
                    .terminals
                    .select(id, cell, terminal::SelectKind::Extend);
                self.redraw();
            }
            return;
        }
        if let Some(g) = self.pressed_group {
            let cell = self.gap_cell(g, at);
            if self.dragged_to != Some(cell) {
                self.dragged_to = Some(cell);
                self.pointer_event(g, MouseKind::Drag, at, 1);
            }
            return;
        }
        let over = self.hit_at(at).map(|(i, h)| (i, h.clone()));
        // Resting on an icon brings up what it is.
        let tip = self
            .tips
            .iter()
            .find(|(r, _)| r.contains(at.0, at.1))
            .cloned();
        match (&self.tip, tip) {
            (Some((_, (r, _))), Some((now, _))) if *r == now => {}
            (_, Some(tip)) => self.tip = Some((Instant::now() + TIP_DELAY, tip)),
            (Some(_), None) => {
                self.tip = None;
                self.redraw();
            }
            (None, None) => {}
        }
        self.update_pointer();
        // Over a menu's title, its menu opens, as menus do.
        if let Some((_, Hit::Ui(UiEvent::MenuTitle(i)))) = over {
            self.dispatch(Event::Ui(UiEvent::MenuTitle(i)));
        }
        if let (Some((_, Hit::Text)), Some(g)) = (&over, self.group_at(at)) {
            // The editor lights up what's under the mouse in the code, and
            // a rest on a word brings up its hover.
            let (row, col) = self.text_point(g, at);
            let cell = (g, row.floor() as i32, col.floor() as i32);
            if self.over_cell != Some(cell) {
                self.over_cell = Some(cell);
                self.pointer_event(g, MouseKind::Move, at, 1);
                self.rest = Some((Instant::now() + HOVER_DELAY, at));
            }
            return;
        }
        self.over_cell = None;
        self.rest = None;
        if before != over.map(|(i, _)| i) {
            self.redraw();
        }
    }
}
