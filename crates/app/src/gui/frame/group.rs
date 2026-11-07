//! An editor group's parts around the code: tabs, breadcrumbs, the bar
//! for a file changed while away, group headers, and test rows.

use super::*;

impl Frame<'_> {
    /// A strip of tabs in `row`, for group `group` when there are several.
    /// With more tabs than fit, it starts late enough for the active one to
    /// show. While a tab is dragged, a bar marks where it would go.
    /// A group's tabs as rounded pills, pinned ones first as only their
    /// icons, scrolled along when they don't all fit, with the split and
    /// more buttons at the right end.
    pub(super) fn tab_strip(
        &mut self,
        tabs: &[TabView],
        row: Rect,
        group: Option<usize>,
        focused: bool,
        bar: Option<&DiffBarView>,
        merge: Option<&MergeBarView>,
    ) {
        let p = self.p;
        // Empty strip takes drops too, at the end, and a double click.
        self.hit(row, Hit::TabStrip(group));
        // A changes tab's bar takes the strip's right end, before its
        // buttons.
        let bar_w = match (bar, merge) {
            (Some(b), _) => self.diff_bar(b, row, row.right() - 72.0),
            (_, Some(m)) => self.merge_bar(m, row, row.right() - 72.0),
            _ => 0.0,
        };
        // A tab in the merge editor says so after its name.
        let merging_w = self.text.width("Merging", Face::ui(11.0)) + 6.0;
        let face_of = |tab: &TabView| {
            if tab.preview {
                Face::ui(13.0).italic()
            } else {
                Face::ui(13.0)
            }
        };
        let widths: Vec<f32> = tabs
            .iter()
            .map(|t| {
                if t.pinned {
                    PINNED_W
                } else {
                    let nw = self.text.width(&t.name, face_of(t))
                        + if t.merging { merging_w } else { 0.0 };
                    TAB_PAD + 14.0 + 7.0 + nw + 5.0 + 18.0 + 6.0
                }
            })
            .collect();
        let (left, right) = (row.x + 6.0, row.right() - 72.0 - bar_w);
        let room = (right - left).max(0.0);
        let starts: Vec<f32> = widths
            .iter()
            .scan(0.0, |x, w| {
                let at = *x;
                *x += w + TAB_GAP;
                Some(at)
            })
            .collect();
        let total = widths.iter().map(|w| w + TAB_GAP).sum::<f32>() - TAB_GAP;
        // Scrolled to the active tab when it changes, else where the wheel
        // left it.
        let g = group.unwrap_or(0);
        if self.strips.len() <= g {
            self.strips.resize(g + 1, (0.0, usize::MAX));
        }
        let active = tabs.iter().position(|t| t.active).unwrap_or(0);
        let (mut offset, revealed) = self.strips[g];
        if revealed != active
            && let (Some(&x), Some(&w)) = (starts.get(active), widths.get(active))
        {
            if x < offset {
                offset = x;
            } else if x + w > offset + room {
                offset = x + w - room;
            }
        }
        offset = offset.clamp(0.0, (total - room).max(0.0));
        self.strips[g] = (offset, active);

        let first_prim = self.canvas.scene.prims.len();
        let clip = Rect::new(left, row.y, room, row.h);
        let pill_h = row.h - 2.0 * TAB_INSET;
        for (i, tab) in tabs.iter().enumerate() {
            let w = widths[i];
            let x = left + starts[i] - offset;
            if x + w < left || x > right {
                continue;
            }
            let r = Rect::new(x, row.y + TAB_INSET, w, pill_h);
            let shown = Rect::new(x.max(left), r.y, (x + w).min(right) - x.max(left), r.h);
            let hovered = self.hovered(shown) && self.tab_drag.is_none();
            // The active tab stands out from a hovered one by its edge,
            // in the focused group.
            if tab.active && focused {
                self.canvas.outline(r, TAB_RADIUS, 1.0, p.border, p.chip);
            } else if tab.active {
                self.canvas.round(r, TAB_RADIUS, p.raised);
            } else if hovered {
                self.canvas.round(r, TAB_RADIUS, p.hover);
            }
            if self.tab_drag.is_some() && self.hovered(shown) {
                self.canvas.round(
                    Rect::new(r.x - 2.5, r.y + 3.0, 2.0, r.h - 6.0),
                    1.0,
                    p.accent,
                );
            }
            let icon_x = if tab.pinned {
                x + (w - 14.0) / 2.0
            } else {
                x + TAB_PAD
            };
            // A tab named "theme.rs ↔ Saved" or "theme.rs (changes)" is
            // its file's: the icon goes by the first word.
            let file = tab.name.split(' ').next().unwrap_or(&tab.name);
            self.file_icon(
                file,
                Some(tab.icon),
                icon_x,
                r.y + (r.h - 14.0) / 2.0,
                14.0,
                clip,
            );
            let (tab_hit, close_hit) = match group {
                Some(group) => (
                    UiEvent::GroupTab {
                        group,
                        index: i,
                        clicks: 1,
                    },
                    UiEvent::CloseGroupTab { group, index: i },
                ),
                None => (
                    UiEvent::Tab {
                        index: i,
                        clicks: 1,
                    },
                    UiEvent::CloseTab(i),
                ),
            };
            self.marked_hit(shown, Hit::Ui(tab_hit), Mark::Tab);
            if tab.pinned {
                // Only its icon: the name is in its tooltip, and changes a
                // dot in its corner.
                self.tips.push((shown, tab.name.clone()));
                if tab.dirty {
                    self.canvas.round(
                        Rect::new(r.right() - 9.0, r.y + 4.0, 5.0, 5.0),
                        2.5,
                        p.muted,
                    );
                }
                continue;
            }
            let face = face_of(tab);
            let mut nw = self.text.width(&tab.name, face);
            let fg = match (tab.active, focused) {
                (true, true) => p.bright,
                (true, false) => p.text,
                _ => p.muted,
            };
            self.text.draw_in_row(
                &mut self.canvas,
                &tab.name,
                face,
                x + TAB_PAD + 21.0,
                r.y,
                r.h,
                fg,
                clip,
            );
            if tab.merging {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "Merging",
                    Face::ui(11.0),
                    x + TAB_PAD + 21.0 + nw + 6.0,
                    r.y,
                    r.h,
                    p.error,
                    clip,
                );
                nw += merging_w;
            }
            let close = Rect::new(
                x + TAB_PAD + 21.0 + nw + 5.0,
                r.y + (r.h - 18.0) / 2.0,
                18.0,
                18.0,
            );
            let mark = if tab.dirty && !self.hovered(close) {
                Some('●')
            } else if tab.active || hovered {
                Some('×')
            } else {
                None
            };
            if let Some(mark) = mark {
                if mark == '×' && self.hovered(close) {
                    self.canvas.round(close, 4.0, p.chip);
                }
                let face = Face::ui(if mark == '●' { 10.0 } else { 14.0 });
                self.centred(&mark.to_string(), face, close, p.muted, clip);
            }
            if close.x >= left && close.right() <= right {
                self.hit(close, Hit::Ui(close_hit));
            }
        }
        // Tabs scrolled part way out are cut at the strip's ends.
        let s = self.canvas.scale;
        clip_cols(
            &mut self.canvas.scene.prims[first_prim..],
            left * s,
            right * s,
        );
        // Split the editor, at the right end.
        let mid = row.y + (row.h - 28.0) / 2.0;
        self.icon_button(
            Rect::new(row.right() - 66.0, mid, 28.0, 28.0),
            "split",
            '\u{eb56}',
            false,
            Hit::Command("view.split"),
        );
        self.icon_button(
            Rect::new(row.right() - 34.0, mid, 28.0, 28.0),
            "more",
            '\u{ea7c}',
            false,
            Hit::Command("palette.open"),
        );
    }

    /// The changes tab's bar, ending at `right`: which change, arrows to
    /// the next and previous, side by side or inline, and Stage File and
    /// Revert File. Returns its width.
    /// The merge editor's bar: what's left, the arrows, Accept All
    /// Incoming and Complete Merge, ending at `right`. Its width.
    /// Over a file that changed on disk while merlin was closed: what
    /// happened, and Compare, Use the Disk's and Keep Mine.
    pub(super) fn away_bar(&mut self, id: u64, name: &str, r: Rect) {
        let p = self.p;
        let face = Face::ui(12.5);
        self.canvas.fill(r, mix(p.warning, p.island, 0.12));
        self.canvas.fill(
            Rect::new(r.x, r.bottom() - 1.0, r.w, 1.0),
            mix(p.warning, p.island, 0.22),
        );
        let (h, y) = (26.0, r.y + (r.h - 26.0) / 2.0);
        let mut x = r.right() - 12.0;
        for (label, what, primary) in [
            ("Keep Mine", "keep", true),
            ("Use the Disk's", "disk", false),
            ("Compare", "compare", false),
        ] {
            let face = if primary { face.medium() } else { face };
            let w = self.text.width(label, face) + 20.0;
            x -= w;
            let b = Rect::new(x, y, w, h);
            let hovered = self.hovered(b);
            let (bg, fg) = match (primary, hovered) {
                (true, false) => (p.accent, p.on_accent),
                (true, true) => (mix(p.accent, 0xffffff, 0.88), p.on_accent),
                (false, false) => (p.chip, p.bright),
                (false, true) => (p.border, p.bright),
            };
            self.canvas.round(b, 6.0, bg);
            self.text
                .draw_in_row(&mut self.canvas, label, face, b.x + 10.0, y, h, fg, b);
            self.hit(b, Hit::Ui(UiEvent::Press(format!("away:{what}:{id}"))));
            x -= 8.0;
        }
        let text_r = Rect::new(r.x + 12.0, r.y, (x - r.x - 12.0).max(0.0), r.h);
        self.text.draw_in_row(
            &mut self.canvas,
            "⚠",
            face,
            text_r.x,
            r.y,
            r.h,
            p.warning,
            text_r,
        );
        let bold = face.medium();
        let tx = text_r.x + 22.0;
        self.text
            .draw_in_row(&mut self.canvas, name, bold, tx, r.y, r.h, p.bright, text_r);
        let tx = tx + self.text.width(name, bold);
        self.text.draw_in_row(
            &mut self.canvas,
            " changed on disk since you left it. Your unsaved changes are kept.",
            face,
            tx,
            r.y,
            r.h,
            p.text,
            text_r,
        );
    }

    pub(super) fn crumb_row(&mut self, crumbs: &[String], row: Rect, group: Option<usize>) {
        let p = self.p;
        let face = Face::ui(12.0);
        let mut x = row.x + 16.0;
        let last = crumbs.len().saturating_sub(1);
        for (i, crumb) in crumbs.iter().enumerate() {
            let tw = self.text.width(crumb, face);
            let r = Rect::new(x - 3.0, row.y + 4.0, tw + 6.0, row.h - 8.0);
            let hovered = self.hovered(r);
            if hovered {
                self.canvas.round(r, 4.0, p.hover);
            }
            let colour = if i == last || hovered {
                p.text
            } else {
                p.muted
            };
            self.text
                .draw_in_row(&mut self.canvas, crumb, face, x, row.y, row.h, colour, row);
            let hit = match group {
                Some(group) => UiEvent::GroupCrumb { group, index: i },
                None => UiEvent::Crumb(i),
            };
            self.hit(r, Hit::Ui(hit));
            x += tw;
            if i < last {
                self.text.draw_in_row(
                    &mut self.canvas,
                    "›",
                    face,
                    x + 6.0,
                    row.y,
                    row.h,
                    p.muted,
                    row,
                );
                x += 20.0;
            }
            if x > row.right() - 20.0 {
                break;
            }
        }
    }

    /// Each group's tabs and breadcrumbs above its code, and the tab being
    /// dragged under the mouse.
    pub(super) fn group_heads(&mut self, view: &WindowView, a: &Areas) {
        // Hairlines between the groups, over the code so nothing hides
        // them, lit while dragged or under the mouse.
        let p = self.p;
        for &(r, sash) in &a.group_sashes {
            let line = match sash {
                GroupSash::Column(_) => Rect::new(r.x + SASH, r.y, DIVIDER, r.h),
                GroupSash::Row(_) => Rect::new(r.x, r.y + SASH, r.w, DIVIDER),
            };
            let lit = self.hovered(r);
            self.canvas.fill(line, if lit { p.accent } else { p.line });
        }
        for (g, (group, area)) in view.groups.iter().zip(&a.groups).enumerate() {
            let bar = view.diff_bar.as_ref().filter(|_| group.focused);
            let merge = view.merge_bar.as_ref().filter(|_| group.focused);
            self.tab_strip(&group.tabs, area.tabs, Some(g), group.focused, bar, merge);
            if let Some(row) = area.crumbs {
                self.crumb_row(&group.crumbs, row, Some(g));
            }
            if let (Some((id, name)), Some(r)) = (&group.away, area.away) {
                self.away_bar(*id, name, r);
            }
        }
        // Where a dragged tab or file would land: a side of a group, or
        // all of it.
        if let Some(zone) = self.drop_zone {
            self.canvas.frame(zone, 8.0, 1.0, self.p.accent, 36);
        }
        if let (Some((name, icon)), Some((px, py))) = (self.tab_drag.clone(), self.pointer) {
            let p = self.p;
            let face = Face::ui(13.0);
            let r = Rect::new(
                px + 12.0,
                py + 8.0,
                TAB_PAD + 21.0 + self.text.width(&name, face) + TAB_PAD,
                26.0,
            );
            self.canvas.shadow(r, TAB_RADIUS, theme::is_light(&p));
            self.canvas.outline(r, TAB_RADIUS, 1.0, p.accent, p.raised);
            self.file_icon(&name, icon, r.x + TAB_PAD, r.y + 6.0, 14.0, r);
            self.text.draw_in_row(
                &mut self.canvas,
                &name,
                face,
                r.x + TAB_PAD + 21.0,
                r.y,
                r.h,
                p.bright,
                r,
            );
        }
    }

    /// A text view's test rows: each test's ▶ Run Test and Debug and how
    /// it last went, in the row over its `fn`; a failure's message in a
    /// red box under the line that failed; and ✓ or ✕ in the gutter.
    pub fn test_rows(&mut self, text: &TextView, code: Rect, y0: f32, row_h: f32, cell_w: f32) {
        let p = self.p;
        let text_x = code.x + (text.gutter as f32 - text.hscroll as f32) * cell_w;
        let clip = Rect::new(
            code.x + text.gutter as f32 * cell_w,
            code.y,
            code.w - text.gutter as f32 * cell_w,
            code.h,
        );
        let link = Face::ui(11.5);
        for lens in &text.tests {
            let y = y0 + lens.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let mut x = text_x + lens.col as f32 * cell_w;
            for (label, id) in &lens.links {
                let w = self.text.width(label, link);
                let r = Rect::new(x - 3.0, y + 2.0, w + 6.0, row_h - 4.0);
                let hovered = self.hovered(r);
                if hovered {
                    self.canvas.round(r.intersect(clip), 4.0, p.hover);
                }
                let colour = if hovered { p.bright } else { p.muted };
                self.text
                    .draw_in_row(&mut self.canvas, label, link, x, y, row_h, colour, clip);
                self.marked_hit(
                    r.intersect(clip),
                    Hit::Ui(UiEvent::Press(id.clone())),
                    Mark::Link,
                );
                x += w + 14.0;
            }
            if let Some((status, failed)) = &lens.status {
                let colour = if *failed { p.error } else { p.success };
                self.text
                    .draw_in_row(&mut self.canvas, status, link, x, y, row_h, colour, clip);
            }
        }
        for failure in &text.failures {
            let y = y0 + failure.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let x = text_x + failure.col as f32 * cell_w;
            let face = Face::mono(11.5);
            let max = (clip.right() - x - 20.0).max(40.0);
            let shown = self.text.fit(&failure.text, face, max);
            let w = self.text.width(&shown, face);
            let r = Rect::new(x - 2.0, y + 2.0, w + 16.0, row_h - 4.0);
            self.canvas
                .round(r.intersect(clip), 4.0, mix(p.error, p.island, 0.18));
            self.text.draw_in_row(
                &mut self.canvas,
                &shown,
                face,
                x + 6.0,
                y,
                row_h,
                mix(p.error, p.bright, 0.35),
                clip,
            );
        }
        // ✓ or ✕ at the gutter's left, on each test's line.
        for line in text.lines.iter().filter(|l| l.test.is_some()) {
            let y = y0 + line.row as f32 * row_h;
            if y + row_h <= code.y || y >= code.bottom() {
                continue;
            }
            let passed = line.test == Some(true);
            let (glyph, colour) = if passed {
                ("✓", p.success)
            } else {
                ("✕", p.error)
            };
            let x = code.x + text.blame_cols as f32 * cell_w + 4.0;
            self.text.draw_in_row(
                &mut self.canvas,
                glyph,
                Face::ui(10.5).bold(),
                x,
                y,
                row_h,
                colour,
                code,
            );
        }
    }
}
