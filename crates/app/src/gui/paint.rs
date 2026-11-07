//! Drawing the window each frame: the frame, then the code, the
//! terminal and the overlays, and the caret's blink and the scroll's glide.

use super::*;

impl Gui<'_> {
    pub(super) fn paint(&mut self) {
        let s = self.scale();
        let cell_h = self.grid.as_ref().map_or(1, |g| g.cell_h) as f32;
        let texts = self.app.editor.text_views();
        let caret_on = self.caret_level();
        let caret = code::Caret {
            style: self.app.window.cursor_style,
            on: caret_on,
            expand: self.app.window.cursor_blinking == Blinking::Expand,
        };
        self.follow_scrolls(&texts, cell_h);
        let bars: Vec<(usize, overview::Shown)> = (0..self.areas.groups.len())
            .map(|g| (g, self.bars_shown(g)))
            .collect();
        let fading = bars.iter().any(|(_, b)| b.alpha > 0.0 && b.alpha < 1.0);
        self.touched = false;
        let glide = self.glide.map_or(0.0, |(g, ..)| self.glide_offset(g));
        let (Some(shown), Some(text), Some(grid)) =
            (&mut self.window, &mut self.text, &mut self.grid)
        else {
            return;
        };
        let size = shown.window.inner_size();
        let (pw, ph) = (size.width as usize, size.height as usize);
        if pw == 0 || ph == 0 {
            return;
        }
        let mut view = self.app.editor.window_view();
        // Code to colour asks as it's laid out: ask, then lay it out again.
        if self.app.serve_colours() {
            view = self.app.editor.window_view();
        }
        let hovered = self.pointer.and_then(|p| {
            self.hits
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (r, _))| r.contains(p.0, p.1))
                .map(|(i, _)| i)
        });
        // The Settings tab, where a group shows it.
        let pages: Vec<usize> = view
            .groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.page.as_deref() == Some(merlin_core::SETTINGS_PAGE))
            .map(|(i, _)| i)
            .collect();
        let settings = (!pages.is_empty()).then(|| self.app.settings_view());
        self.settings_due = settings
            .as_ref()
            .filter(|s| s.ui.category == Category::Plugins)
            .map(|_| Instant::now() + prefs::MEMORY_EVERY);
        let drawn = Some((
            view.clone(),
            hovered,
            self.state.scroll,
            (size.width, size.height),
            self.app.look.clone(),
        ));
        let gliding = self.glide.is_some();
        // A dragged tab follows the mouse, so every move draws.
        let dragging = self.tab_drag.as_ref().is_some_and(|d| d.moved);
        let tip = self
            .tip
            .as_ref()
            .filter(|t| t.0 <= Instant::now())
            .map(|t| t.1.clone());
        let tip_changed = tip != self.tip_drawn;
        if self.drawn == drawn
            && self.bars_drawn == bars
            && !tip_changed
            && self.drawn_text == texts
            && caret_on == self.caret_drawn
            && self.term_drawn == self.app.terminals.version
            && !gliding
            && !dragging
            && self.settings_drawn == settings
        {
            return;
        }
        // Packs changed: their pictures may have too. Too many bitmaps:
        // start again.
        if self.drawn.as_ref().map(|d| &d.4) != Some(&self.app.look)
            || self.bitmaps.bytes() > Bitmaps::LIMIT
        {
            self.bitmaps.clear();
        }
        let (w, h) = (pw as f32 / s, ph as f32 / s);
        let areas = frame::areas(&view, w, h, self.app.look.layout.gap, &self.sizes);
        self.scene.clear();
        let mut f = Frame {
            canvas: Canvas {
                scene: &mut self.scene,
                bitmaps: &mut self.bitmaps,
                width: pw,
                height: ph,
                scale: s,
            },
            text,
            pointer: self.pointer,
            hits: Vec::new(),
            radius: self.app.look.layout.radius,
            p: self.app.look.palette,
            icons: &self.app.look.icons,
            pictures: &mut self.pictures,
            compact: self.app.look.layout.compact,
            sash_hits: Vec::new(),
            tips: Vec::new(),
            marks: Vec::new(),
            tip: tip.clone(),
            tab_drag: self
                .tab_drag
                .as_ref()
                .filter(|d| d.moved)
                .map(|d| (d.name.clone(), d.icon)),
            drop_zone: match self.drop {
                Some(Landing::Body { zone, .. }) => Some(zone),
                _ => None,
            },
            strips: std::mem::take(&mut self.strips),
            term_panes: Vec::new(),
            panel_max: self.sizes.bottom_before.is_some(),
        };
        f.base(&view, &areas, &mut self.state, w, h);
        let glide_of = |group: usize| match self.glide {
            Some((g, ..)) if g == group => glide,
            _ => 0.0,
        };
        draw_texts(
            &mut f,
            grid,
            &texts,
            &view,
            &areas,
            &self.app.look.palette,
            caret,
            s,
            glide_of,
        );
        // The terminals' screens, each fitted to its pane first.
        let colours = self.app.terminal_colours();
        let term = view.terminal.as_ref();
        let focused = term.filter(|t| t.focused).and_then(|t| t.active);
        for &(id, r) in &f.term_panes {
            let cols = (r.w * s / grid.cell_w as f32).floor() as usize;
            let lines = (r.h * s / grid.cell_h as f32).floor() as usize;
            let cell = (grid.cell_w as u16, grid.cell_h as u16);
            self.app.terminals.resize(id, cols, lines, cell);
            let ended = term
                .and_then(|t| t.sessions.iter().find(|s| s.id == id))
                .is_some_and(|s| s.exited.is_some());
            if let Some(mut screen) = self.app.terminals.screen(id, &colours) {
                if ended {
                    screen.cursor = None;
                }
                let at = ((r.x * s).round(), (r.y * s).round());
                grid.draw_terminal(&screen, &mut f.canvas, at, &colours, focused == Some(id));
            }
        }
        self.term_panes = f.term_panes.clone();
        self.term_drawn = self.app.terminals.version;
        let cell_w = grid.cell_w as f32 / s;
        draw_bars(
            &mut f,
            &areas,
            &texts,
            &bars,
            &self.app.look.palette,
            cell_w,
        );
        let mut room = None;
        if let Some(sv) = &settings {
            for &g in &pages {
                let Some(area) = areas.groups.get(g) else {
                    continue;
                };
                let r = Rect::new(
                    area.tabs.x,
                    area.tabs.bottom(),
                    area.tabs.w,
                    (area.text.bottom() - area.tabs.bottom()).max(0.0),
                );
                let focused = view.groups[g].focused
                    && view.focus == merlin_core::Focus::Editor
                    && view.palette.is_none();
                room = Some(f.settings_page(sv, r, focused));
            }
        }
        let cell = (grid.cell_w as f32 / s, grid.cell_h as f32 / s);
        f.floating(&view, &areas, cell, self.context_at, w, h);
        f.overlays(&view, &areas, w, h);
        self.hits = std::mem::take(&mut f.hits);
        self.strips = std::mem::take(&mut f.strips);
        self.tips = std::mem::take(&mut f.tips);
        self.marks = std::mem::take(&mut f.marks);
        self.pointer_link = view.pointer_link;
        self.busy = view.busy;
        self.tip_drawn = tip;
        if let Some(room) = room {
            let ui = &mut self.app.settings_ui;
            ui.room = room;
            ui.scroll = ui.scroll.min(room);
        }
        self.settings_drawn = settings;
        let shown_ok = present(shown, &self.scene, &self.bitmaps, pw, ph);
        self.areas = areas;
        if !shown_ok {
            // Not shown: draw again when asked.
            self.drawn = None;
            return;
        }
        // The hover index belongs to the hits just made.
        let hovered = self.pointer.and_then(|p| {
            self.hits
                .iter()
                .enumerate()
                .rev()
                .find(|(_, (r, _))| r.contains(p.0, p.1))
                .map(|(i, _)| i)
        });
        self.drawn = drawn.map(|d| (d.0, hovered, d.2, d.3, d.4));
        // The input method's candidates open by the cursor.
        if let Some(v) = texts.last()
            && let Some((cx, cy)) = v.carets.first().and_then(|c| c.cell)
            && let Some(area) = self.areas.groups.get(v.group)
        {
            let (cw, ch) = (grid.cell_w as f32 / s, grid.cell_h as f32 / s);
            let x = area.text.x + (v.gutter as f32 + cx as f32) * cw;
            let y = area.text.y + (cy as f32 - v.top_frac + 1.0) * ch;
            let spot = (x.round() as i32, y.round() as i32);
            if self.ime_spot != Some(spot) {
                self.ime_spot = Some(spot);
                shown.window.set_ime_cursor_area(
                    winit::dpi::LogicalPosition::new(x, y - ch),
                    winit::dpi::LogicalSize::new(cw, ch),
                );
            }
        }
        self.drawn_text = texts;
        self.bars_drawn = bars;
        self.caret_drawn = caret_on;
        if fading {
            self.redraw();
        }
        // Until the glide ends, every frame moves it on.
        if self.glide.is_some() {
            if glide == 0.0 {
                self.glide = None;
            }
            self.redraw();
        }
    }

    /// A scroll since the last frame glides there from where it was, but a
    /// touchpad's follows the fingers already. Scrolling shows the thumbs.
    fn follow_scrolls(&mut self, texts: &[TextView], cell_h: f32) {
        let mut tops = Vec::new();
        let mut moved_groups = Vec::new();
        for v in texts {
            if tops.iter().any(|(g, _)| *g == v.group) {
                continue;
            }
            let was = self
                .last_top
                .iter()
                .find(|(g, _)| *g == v.group)
                .map(|t| t.1);
            // Scrolling shows the thumbs.
            if was.is_some_and(|was| was != v.top_row) {
                moved_groups.push(v.group);
            }
            if let Some(was) = was
                && was != v.top_row
                && !self.touched
                && self.app.window.smooth_scrolling
            {
                let rows = (v.top_row - was) as f32;
                let from = self.glide_offset(v.group) + rows * cell_h;
                self.glide =
                    (rows.abs() <= v.overscan as f32).then(|| (v.group, Instant::now(), from));
            }
            tops.push((v.group, v.top_row));
        }
        self.last_top = tops;
        for g in moved_groups {
            self.show_bars(g);
        }
    }

    /// Whether the caret blinks now: for a while after each input, then
    /// it's on for good once the user has been away a while.
    pub(super) fn blinking(&self) -> bool {
        self.focused
            && self.blink_from.elapsed() < BLINK_FOR
            && self.app.window.cursor_blinking != Blinking::Solid
    }

    /// Where the blink is, in blinks since it was last on from the start:
    /// from 0 up to 2, on for the first and off for the second.
    pub(super) fn blink_phase(&self) -> f32 {
        let blink = BLINK.as_secs_f32();
        (self.blink_from.elapsed().as_secs_f32() / blink) % 2.0
    }

    /// How far on the caret is now, from 0 to 1, as the settings have it
    /// blink.
    pub(super) fn caret_level(&self) -> f32 {
        if !self.blinking() {
            return 1.0;
        }
        let t = self.blink_phase();
        let level = match self.app.window.cursor_blinking {
            Blinking::Blink | Blinking::Solid => (t < 1.0) as u8 as f32,
            Blinking::Phase => 0.5 + 0.5 * (t * std::f32::consts::PI).cos(),
            // On, fading out, off, fading in.
            Blinking::Smooth | Blinking::Expand => {
                if t < 1.0 - FADE {
                    1.0
                } else if t < 1.0 {
                    (1.0 - t) / FADE
                } else if t < 2.0 - FADE {
                    0.0
                } else {
                    (t - (2.0 - FADE)) / FADE
                }
            }
        };
        // In steps, so frames that look the same aren't drawn twice.
        (level * 32.0).round() / 32.0
    }

    /// When the caret next changes, if it's still blinking: its next
    /// blink, or its next frame while it fades.
    pub(super) fn next_blink(&self) -> Option<Instant> {
        if !self.blinking() {
            return None;
        }
        let now = Instant::now();
        let t = self.blink_phase();
        let at = |phase: f32| now + BLINK.mul_f32(phase - t);
        Some(match self.app.window.cursor_blinking {
            Blinking::Blink | Blinking::Solid => at(if t < 1.0 { 1.0 } else { 2.0 }),
            Blinking::Phase => now + FADE_FRAME,
            Blinking::Smooth | Blinking::Expand => {
                let fading = (1.0 - FADE..1.0).contains(&t) || t >= 2.0 - FADE;
                if fading {
                    now + FADE_FRAME
                } else if t < 1.0 - FADE {
                    at(1.0 - FADE)
                } else {
                    at(2.0 - FADE)
                }
            }
        })
    }

    /// How far, in physical pixels, the lines are from where they're
    /// gliding to: eased out, so it starts quick and settles.
    pub(super) fn glide_offset(&self, group: usize) -> f32 {
        let Some((g, start, from)) = self.glide else {
            return 0.0;
        };
        if g != group {
            return 0.0;
        }
        let t = start.elapsed().as_secs_f32() / GLIDE.as_secs_f32();
        if t >= 1.0 {
            return 0.0;
        }
        from * (1.0 - t).powi(3)
    }
}

/// The code of each text view, then what's drawn over and between its
/// lines: changes tabs' boxes, peeks, compared files' titles, the merge
/// editor, tests, the blame card and a change opened in place.
#[allow(clippy::too_many_arguments)]
fn draw_texts(
    f: &mut Frame,
    grid: &mut Fonts,
    texts: &[TextView],
    view: &WindowView,
    areas: &Areas,
    palette: &crate::theme::Palette,
    caret: code::Caret,
    s: f32,
    glide_of: impl Fn(usize) -> f32,
) {
    for text in texts {
        let Some(area) = areas.groups.get(text.group) else {
            continue;
        };
        let t = area.text;
        let glide = glide_of(text.group);
        grid.draw_code(
            text,
            &mut f.canvas,
            f.text,
            [t.x * s, t.y * s, t.w * s, t.h * s],
            palette,
            caret,
            glide,
        );
    }
    // The peek views, in the rows they open between the lines.
    let row_h = grid.cell_h as f32 / s;
    // A changes tab's boxes, in the gap between its halves.
    for text in texts {
        let Some(area) = areas.groups.get(text.group) else {
            continue;
        };
        if text.diff.is_none() {
            continue;
        }
        let glide = glide_of(text.group);
        let y0 = area.text.y + (text.area.y as f32 - text.top_frac) * row_h + glide / s;
        f.diff_marks(text, area.text, grid.cell_w as f32 / s, row_h, y0);
    }
    for text in texts {
        let (Some(peek), Some(area)) = (&text.peek, areas.groups.get(text.group)) else {
            continue;
        };
        let glide = glide_of(text.group);
        let rows = text.area.y as f32 + peek.row as f32 - text.top_frac;
        let y = area.text.y + rows * row_h + glide / s;
        f.peek(peek, area.text, y, row_h);
    }
    // Two compared files' titles over their halves.
    let cell_w = grid.cell_w as f32 / s;
    for (g, group) in view.groups.iter().enumerate() {
        let (Some(titles), Some(area)) = (&group.compare, areas.groups.get(g)) else {
            continue;
        };
        let halves: Vec<Rect> = texts
            .iter()
            .filter(|t| t.group == g)
            .map(|t| {
                Rect::new(
                    area.text.x + t.area.x as f32 * cell_w,
                    area.text.y,
                    t.area.w as f32 * cell_w,
                    area.text.h,
                )
            })
            .collect();
        f.compare_titles(titles, &halves);
    }
    // The merge editor's panes and the Result's notes.
    for text in texts {
        let (Some(merge), Some(area)) = (&text.merge, areas.groups.get(text.group)) else {
            continue;
        };
        let glide = glide_of(text.group);
        let y0 = area.text.y + (text.area.y as f32 - text.top_frac) * row_h + glide / s;
        let gutter = text.gutter as f32 * grid.cell_w as f32 / s;
        f.merge_editor(merge, area, y0, row_h, gutter);
    }
    // Tests: ▶ Run Test over each, failures under their lines, and
    // how each went in the gutter.
    for text in texts {
        let Some(area) = areas.groups.get(text.group) else {
            continue;
        };
        if text.tests.is_empty() && text.failures.is_empty() {
            continue;
        }
        let glide = glide_of(text.group);
        let cell_w = grid.cell_w as f32 / s;
        let y0 = area.text.y + (text.area.y as f32 - text.top_frac) * row_h + glide / s;
        let x0 = area.text.x + text.area.x as f32 * cell_w;
        let code = Rect::new(x0, area.text.y, text.area.w as f32 * cell_w, area.text.h);
        f.test_rows(text, code, y0, row_h, cell_w);
    }
    // A commit's card over the blame column.
    for text in texts {
        let (Some(card), Some(area)) = (&text.blame_card, areas.groups.get(text.group)) else {
            continue;
        };
        let rows = text.area.y as f32 + card.row as f32 - text.top_frac;
        f.blame_card(card, area.text, area.text.y + rows * row_h);
    }
    // A change opened in place, likewise.
    for text in texts {
        let (Some(change), Some(area)) = (&text.change, areas.groups.get(text.group)) else {
            continue;
        };
        let glide = glide_of(text.group);
        let rows = text.area.y as f32 + change.row as f32 - text.top_frac;
        let y = area.text.y + rows * row_h + glide / s;
        f.change_peek(change, area.text, y, row_h, change.rows);
    }
}

/// Beside each group's code, its minimap and scrollbars, and a shadow
/// under the tabs once it's scrolled; the review side by side has them on
/// its new side.
fn draw_bars(
    f: &mut Frame,
    areas: &Areas,
    texts: &[TextView],
    bars: &[(usize, overview::Shown)],
    p: &crate::theme::Palette,
    cell_w: f32,
) {
    for (g, area) in areas.groups.iter().enumerate() {
        let Some(v) = texts.iter().rfind(|v| v.group == g) else {
            continue;
        };
        let shown = bars.get(g).map_or_else(Default::default, |b| b.1);
        if let Some(rect) = area.minimap {
            overview::draw_minimap(&mut f.canvas, p, rect, v, shown);
            f.hits.push((rect, Hit::Minimap(g)));
        }
        overview::draw_vbar(&mut f.canvas, p, area.bar, v, shown);
        f.hits.push((
            area.bar,
            Hit::Scrollbar {
                group: g,
                horizontal: false,
            },
        ));
        let track = overview::htrack(area.text, v, cell_w);
        if overview::hthumb(track, v).is_some() {
            overview::draw_hbar(&mut f.canvas, p, track, v, shown);
            f.hits.push((
                track,
                Hit::Scrollbar {
                    group: g,
                    horizontal: true,
                },
            ));
        }
        overview::draw_shadow(&mut f.canvas, area.text, v);
    }
}

/// The scene, on the GPU or in the window's buffer: whether it showed.
fn present(shown: &mut Shown, scene: &Scene, bitmaps: &Bitmaps, pw: usize, ph: usize) -> bool {
    match &mut shown.backend {
        Backend::Gpu(gpu) => gpu.draw(scene, bitmaps, &shown.window),
        Backend::Cpu(surface) => match surface.buffer_mut() {
            Ok(mut buffer) if buffer.len() == pw * ph => {
                cpu::draw(scene, bitmaps, &mut buffer, pw, ph);
                shown.window.pre_present_notify();
                buffer.present().is_ok()
            }
            _ => false,
        },
    }
}
