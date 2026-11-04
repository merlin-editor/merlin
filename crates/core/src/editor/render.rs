//! Drawing the editor into a [`Screen`].

use merlin_syntax::Syntax;

use super::layout::Rect;
use super::{
    COMPLETION_ROWS, Editor, Find, Focus, HOVER_ROWS, PALETTE_ROWS, POPUP_WIDTH, Palette, Popup,
    Side, Source,
};
use crate::diff::{LineKind, Sign};
use crate::doc::advance;
use crate::lang::Severity;
use crate::panel;
use crate::screen::{Hue, Screen, Style, Surface, str_width};

impl Editor {
    pub fn render(&self) -> Screen {
        let (w, h) = (self.width, self.height);
        let mut screen = Screen::new(w, h);
        if w == 0 || h == 0 {
            return screen;
        }
        let layout = self.layout();
        // Other editor groups first, each with its own tabs and view, and a
        // line between groups.
        let panes = self.pane_rects(layout.editor);
        if panes.len() == self.groups.len() && panes.len() > 1 && !self.own_text {
            for g in 0..self.groups.len() {
                self.as_group(g, || {
                    let layout = self.layout();
                    if g != self.group {
                        self.render_tabs(&mut screen, &layout);
                        self.render_crumbs(&mut screen, layout.crumbs);
                        self.render_text(&mut screen, layout.text);
                        if !self.own_text {
                            self.render_sticky(&mut screen, layout.text);
                        }
                    }
                });
            }
            // Lines right of and under groups, where another one is.
            let all = layout.editor;
            for pane in &panes {
                if pane.right() < all.right() {
                    for y in pane.y..pane.bottom() {
                        screen.put(pane.right(), y, '│', Style::Frame);
                    }
                }
            }
            for pane in panes.iter().filter(|p| p.bottom() < all.bottom()) {
                let y = pane.bottom();
                for x in pane.x..pane.right() {
                    screen.put(x, y, '─', Style::Frame);
                }
                // Where it meets the line between columns.
                if pane.right() < all.right() {
                    screen.put(pane.right(), y, '┤', Style::Frame);
                }
                if pane.x > all.x {
                    let x = pane.x - 1;
                    let joint = if screen.cell(x, y).ch == '┤' {
                        '┼'
                    } else {
                        '├'
                    };
                    screen.put(x, y, joint, Style::Frame);
                }
            }
        }
        self.render_tabs(&mut screen, &layout);
        self.render_crumbs(&mut screen, layout.crumbs);
        self.render_text(&mut screen, layout.text);
        if !self.own_text {
            self.render_sticky(&mut screen, layout.text);
        }
        // A window drawing the text draws find, popups and menus too.
        if let Some(find) = &self.find
            && !self.own_text
        {
            self.render_find(&mut screen, find, layout.text);
        }
        self.render_popup(&mut screen);
        // A window draws the rest of the frame itself.
        if self.windowed {
            if !self.own_text {
                self.render_menu(&mut screen);
            }
            return screen;
        }
        self.render_activity(&mut screen);
        for side in [Side::Left, Side::Right, Side::Bottom] {
            self.render_side(&mut screen, side);
        }
        self.render_status(&mut screen, layout.status);
        self.render_menu_bar(&mut screen);
        self.render_toast(&mut screen);
        self.render_menu(&mut screen);
        if let Some(palette) = &self.palette {
            self.render_palette(&mut screen, palette);
        }
        screen
    }

    fn render_tabs(&self, s: &mut Screen, layout: &super::layout::Layout) {
        let area = layout.tabs;
        if area.h == 0 {
            return;
        }
        s.shade(area.x, area.y, area.w, 1, Surface::Bar);
        let hovered = self.hovered_tab();
        for (start, end, i) in self.tab_spans() {
            let room = end.min(area.right()).saturating_sub(start);
            let active = i == self.pane_doc();
            let preview = self.preview == Some(self.docs[i].id);
            let style = match (active, preview) {
                (true, false) => Style::TabActive,
                (false, false) => Style::Tab,
                (true, true) => Style::TabPreviewActive,
                (false, true) => Style::TabPreview,
            };
            s.print(start, area.y, &self.tab_label(i), style, room);
            let (_, hue) = self.tab_icon(i);
            if room > 2 {
                s.tint(start + 1, area.y, hue);
            }
            if let Some(hue) = self.tab_hue(i) {
                let name = str_width(&self.docs[i].name()) as u16;
                for x in start + 3..(start + 3 + name).min(start + room) {
                    s.tint(x, area.y, hue);
                }
            }
            if hovered == Some(i) && !active {
                s.hover(start, area.y, room);
            }
        }
        // The current file's changes, as VS Code offers them.
        if let Some(x) = self.diff_button() {
            s.put(
                x,
                area.y,
                crate::icons::icon(crate::icons::Icon::Diff, self.nerd),
                Style::Tab,
            );
            if self.hover == Some((x, area.y)) {
                s.hover(x, area.y, 1);
            }
        }
    }

    /// Where the button that opens the current file's changes goes, if it
    /// has any.
    pub(crate) fn diff_button(&self) -> Option<u16> {
        let doc = self.doc();
        let area = self.layout().tabs;
        let changed = doc.review.is_none()
            && doc.buffer.path.is_some()
            && doc.git_signs.iter().any(|s| s.is_some());
        (changed && area.h > 0 && area.w > 10).then(|| area.right() - 3)
    }

    /// A tab's name colour, as VS Code colours it for problems and git.
    fn tab_hue(&self, i: usize) -> Option<Hue> {
        let doc = &self.docs[i];
        match doc.marks.iter().map(|m| m.severity).min() {
            Some(Severity::Error) => return Some(Hue::Problem),
            Some(Severity::Warning) => return Some(Hue::Warned),
            _ => {}
        }
        let path = doc.buffer.path.as_ref()?.display().to_string();
        let (_, status) = self.git.files.iter().find(|(p, _)| *p == path)?;
        Some(match status {
            'A' | '?' => Hue::Untracked,
            'D' => Hue::Deleted,
            _ => Hue::Modified,
        })
    }

    fn render_text(&self, s: &mut Screen, area: Rect) {
        if let Some(rows) = self.split_rows()
            && !self.own_text
        {
            self.render_split(s, area, &rows);
            self.render_scrollbar(s, area);
            return;
        }
        // A window may draw the text itself, from `text_views`; the cells
        // keep only the minimap.
        if self.own_text {
            self.render_minimap(s);
            return;
        }
        let view = self.text_view(area);
        let gutter = view.gutter;
        let x0 = area.x as usize;
        // The last column is the scrollbar, after the minimap if it's on.
        let right = x0 + gutter + self.text_columns();
        let focused = view.focused;
        let hscroll = view.hscroll;
        for &row in &view.beyond {
            s.put(area.x, area.y + row, '~', Style::Muted);
        }
        for l in &view.lines {
            let (y, shown, rows) = (area.y + l.row as u16, l.shown, &l.rows);
            if l.base != Style::Normal {
                for r in 0..shown {
                    s.fill(
                        area.x + gutter as u16,
                        y + r,
                        (right - x0).saturating_sub(gutter) as u16,
                        l.base,
                    );
                }
            }
            let signs = view.signs as usize;
            if let Some((sign, sign_style)) = l.sign {
                s.put(area.x, y, sign, sign_style);
                // A changed line's bar runs down its wrapped rows.
                if sign == '▎' {
                    for r in 1..shown {
                        s.put(area.x, y + r, sign, sign_style);
                    }
                }
            }
            s.print(
                area.x + signs as u16,
                y,
                &l.number,
                l.number_style,
                (gutter - signs) as u16,
            );
            if let Some(folded) = l.fold_arrow {
                use crate::icons::{Icon, icon};
                let arrow = if folded { Icon::Closed } else { Icon::Open };
                s.put(
                    area.x + gutter as u16 - 1,
                    y,
                    icon(arrow, self.nerd),
                    Style::Gutter,
                );
            }
            let (styles, syntax, marks) = (&l.styles, &l.syntax, &l.marks);
            let left_edge = x0 + gutter;
            let mut d = 0;
            let mut r = 0;
            for (i, &c) in l.chars.iter().enumerate() {
                while r + 1 < rows.len() && i >= rows[r + 1].start {
                    r += 1;
                }
                if r >= shown as usize {
                    break;
                }
                let y = y + r as u16;
                let w = advance(c, d);
                let left = (rows[r].indent + d - rows[r].col) as isize - hscroll as isize;
                d += w;
                if let Some(active) = l.guides[i]
                    && left >= 0
                {
                    let x = left_edge as isize + left;
                    if (x as usize) < right {
                        let hue = if active { Hue::GuideActive } else { Hue::Guide };
                        for cx in x as usize..(x as usize + w).min(right) {
                            s.put(cx as u16, y, ' ', styles[i]);
                        }
                        s.put(x as u16, y, '│', styles[i]);
                        s.tint(x as u16, y, hue);
                        continue;
                    }
                }
                if left + (w as isize) <= 0 {
                    continue;
                }
                let x = left_edge as isize + left;
                if x as usize >= right {
                    // A wrapped row's space may hang past the edge.
                    if view.wrapping {
                        continue;
                    }
                    break;
                }
                // Tabs, and characters cut by an edge, draw as spaces.
                if c == '\t' || left < 0 || x as usize + w > right {
                    for cx in x.max(left_edge as isize) as usize..(x as usize + w).min(right) {
                        s.put(cx as u16, y, ' ', styles[i]);
                        s.decorate(cx as u16, y, syntax[i], marks[i]);
                    }
                } else {
                    s.put(x as u16, y, c, styles[i]);
                    s.decorate(x as u16, y, syntax[i], marks[i]);
                    if let Some(hue) = l.brackets[i] {
                        s.tint(x as u16, y, hue);
                    }
                }
            }
            // A selected line break shows as one selected cell.
            let last = rows.len() - 1;
            // The loop stops at the view's last row, so only a line drawn
            // to its end has its width in `d`.
            let end_x = if last < shown as usize {
                (rows[last].indent + d - rows[last].col) as isize - hscroll as isize
            } else {
                -1
            };
            // A folded block shows as an ellipsis after its first line.
            if l.folded && end_x >= 0 && left_edge + (end_x as usize) + 2 < right {
                s.print(
                    (left_edge + end_x as usize + 1) as u16,
                    y + last as u16,
                    "⋯",
                    Style::Muted,
                    1,
                );
            }
            if l.eol_selected && end_x >= 0 && left_edge + (end_x as usize) < right {
                s.put(
                    (left_edge + end_x as usize) as u16,
                    y + last as u16,
                    ' ',
                    Style::Selection,
                );
            }
        }
        self.render_minimap(s);
        self.render_scrollbar(s, area);
        if focused {
            s.cursor = self.cursor_cell();
            // The terminal has one cursor; the others are drawn.
            for caret in &self.carets()[1..] {
                if let Some((x, y)) = self.cell_of(caret.cursor) {
                    let ch = s.cell(x, y).ch;
                    s.put(x, y, if ch == '\0' { ' ' } else { ch }, Style::Caret);
                }
            }
        }
    }

    /// A review side by side: the old text on the left, the new on the
    /// right, as VS Code shows diffs in a wide window.
    fn render_split(&self, s: &mut Screen, area: Rect, rows: &[(Option<usize>, Option<usize>)]) {
        let doc = self.doc();
        let v = self.view();
        let Some(review) = &doc.review else { return };
        let first = split_first(rows, v.scroll);
        let shown: Vec<_> = rows.iter().skip(first).take(area.h as usize).collect();
        // Syntax colours for the buffer lines on screen.
        let lines: Vec<usize> = shown.iter().flat_map(|(l, r)| [*l, *r]).flatten().collect();
        let range =
            lines.iter().min().copied().unwrap_or(0)..lines.iter().max().map_or(0, |m| m + 1);
        let spans = doc.spans(range);
        let digits = review
            .lines
            .iter()
            .map(|(_, n)| n + 1)
            .chain(review.old_numbers.iter().map(|n| n + 1))
            .max()
            .unwrap_or(1)
            .to_string()
            .len()
            .max(3);
        // Two halves, a divider, and the scrollbar.
        let half = area.w.saturating_sub(2) / 2;
        let cur = doc.line_col(v.cursor).0;
        for (row, (left, right)) in shown.iter().enumerate() {
            let y = area.y + row as u16;
            for (x, line, old) in [(area.x, *left, true), (area.x + half + 1, *right, false)] {
                let Some(line) = line else { continue };
                let (kind, new_number) = review.lines[line];
                let number = if old {
                    review.old_numbers[line]
                } else {
                    new_number
                };
                let (sign, base) = match kind {
                    LineKind::Added => ('+', Style::DiffAdded),
                    LineKind::Removed => ('-', Style::DiffRemoved),
                    LineKind::Gap => (' ', Style::Muted),
                    LineKind::Same if line == cur => (' ', Style::CurrentLine),
                    LineKind::Same => (' ', Style::Normal),
                };
                let gutter = digits + 2;
                if base != Style::Normal {
                    s.fill(
                        x + gutter as u16,
                        y,
                        half.saturating_sub(gutter as u16),
                        base,
                    );
                }
                s.put(x, y, sign, base);
                let number = if kind == LineKind::Gap {
                    String::new()
                } else {
                    (number + 1).to_string()
                };
                let style = if line == cur {
                    Style::GutterActive
                } else {
                    Style::Gutter
                };
                s.print(
                    x + 1,
                    y,
                    &format!("{number:>digits$} "),
                    style,
                    gutter as u16 - 1,
                );
                let start = doc.line_start(line);
                let chars = doc.line_chars(line);
                let mut d = 0;
                let right_edge = (x + half) as usize;
                for (i, &c) in chars.iter().enumerate() {
                    let w = advance(c, d);
                    let left = d as isize - v.hscroll as isize;
                    d += w;
                    if left < 0 {
                        continue;
                    }
                    let cx = x as usize + gutter + left as usize;
                    if cx + w > right_edge {
                        break;
                    }
                    let syntax = spans
                        .iter()
                        .rev()
                        .find(|(a, b, _)| (*a..*b).contains(&(start + i)))
                        .map_or(Syntax::Plain, |(_, _, k)| *k);
                    let c = if c == '\t' { ' ' } else { c };
                    s.put(cx as u16, y, c, base);
                    s.decorate(cx as u16, y, syntax, None);
                }
            }
            s.put(area.x + half, y, '│', Style::Frame);
        }
    }

    /// The scrollbar, with marks for errors, warnings, changed lines and
    /// search matches, as
    /// VS Code's overview ruler has them.
    fn render_scrollbar(&self, s: &mut Screen, area: Rect) {
        if area.w < 2 || area.h == 0 {
            return;
        }
        let x = area.right() - 1;
        let doc = self.doc();
        let v = self.view();
        let lines = doc.last_line() + 1;
        let h = area.h as usize;
        if lines <= h {
            return;
        }
        let row_of = |line: usize| (line * h / lines).min(h - 1) as u16;
        let thumb = (h * h / lines).max(1);
        let top = row_of(v.scroll) as usize;
        for row in 0..h {
            let style = if (top..top + thumb).contains(&row) {
                Style::ScrollThumb
            } else {
                Style::ScrollTrack
            };
            s.put(x, area.y + row as u16, ' ', style);
        }
        for (line, sign) in doc.git_signs.iter().enumerate() {
            let style = match sign {
                Some(Sign::Added) => Style::GitAdded,
                Some(Sign::Changed) => Style::GitChanged,
                Some(Sign::Removed) => Style::GitRemoved,
                None => continue,
            };
            s.put(x, area.y + row_of(line), '▐', style);
        }
        if let Some((query, fold)) = self.query() {
            for m in self.all_matches(&query, fold) {
                s.put(
                    x,
                    area.y + row_of(doc.line_col(m).0),
                    '▌',
                    Style::ScrollMatch,
                );
            }
        }
        for mark in &doc.marks {
            let style = match mark.severity {
                Severity::Error => Style::GutterError,
                Severity::Warning => Style::GutterWarning,
                _ => continue,
            };
            s.put(x, area.y + row_of(doc.line_col(mark.start).0), '■', style);
        }
    }

    fn render_side(&self, s: &mut Screen, side: Side) {
        let Some(area) = self.layout().side(side) else {
            return;
        };
        s.shade(area.x, area.y, area.w, area.h, Surface::Bar);
        let Some(view) = self.panel_view(side) else {
            let x = area.x + 2;
            let inner = area.w.saturating_sub(4);
            let mut y = area.y + 1;
            let mut line = |text: &str, style: Style| {
                if y < area.bottom() {
                    s.print(x, y, text, style, inner);
                }
                y += 1;
            };
            let plugins: Vec<_> = self.plugins.iter().filter(|p| p.side == side).collect();
            if plugins.iter().all(|p| p.error.is_none()) {
                line("No plugin panels", Style::Muted);
            }
            for plugin in plugins {
                if let Some(error) = &plugin.error {
                    line(&format!("{} failed", plugin.name), Style::Error);
                    for part in panel::wrap(error, inner as usize) {
                        line(&part, Style::Muted);
                    }
                }
            }
            return;
        };
        let focused = self.focus == side.focus();
        for (start, end, name) in &view.titles {
            let active = *name == view.active;
            let style = match side {
                Side::Bottom if active => Style::PanelTitleActive,
                Side::Bottom => Style::PanelTitle,
                _ if active && focused => Style::SideTitleFocused,
                _ if active => Style::SideTitle,
                _ => Style::Muted,
            };
            let (title, badge) = self.panel_title(side, name);
            let used = s.print(*start, view.title_row, &title, style, end - start);
            if let Some(badge) = badge {
                let x = start + used + 1;
                s.print(
                    x,
                    view.title_row,
                    &format!(" {badge} "),
                    Style::Badge,
                    end.saturating_sub(x),
                );
            }
        }
        let hover_row = self
            .hover
            .filter(|&(x, y)| area.contains(x, y))
            .map(|(_, y)| y);
        for (y, row) in &view.rows {
            if let Some(fill) = row.fill {
                if fill == Style::Selected {
                    s.fill(area.x, *y, area.w, fill);
                } else {
                    s.fill(view.x, *y, view.inner, fill);
                }
            }
            let mut cx = view.x;
            for (text, style, syntax) in &row.segments {
                let start = cx;
                cx += s.print(
                    cx,
                    *y,
                    text,
                    *style,
                    (view.x + view.inner).saturating_sub(cx),
                );
                if *syntax != Syntax::Plain {
                    for x in start..cx {
                        s.decorate(x, *y, *syntax, None);
                    }
                }
            }
            for &(col, hue) in &row.tints {
                s.tint(view.x + col, *y, hue);
            }
            if hover_row == Some(*y) && !row.hits.is_empty() && row.fill.is_none() {
                s.hover(area.x, *y, area.w);
            }
            if let Some(col) = row.cursor {
                s.cursor = Some((
                    (view.x + col).min(view.x + view.inner.saturating_sub(1)),
                    *y,
                ));
            }
        }
    }

    /// Find and replace, in a box at the top right of the text, as VS Code
    /// draws it.
    fn render_find(&self, s: &mut Screen, find: &Find, area: Rect) {
        let width = area.w.saturating_sub(3).min(52);
        if width < 12 || area.h < 2 {
            return;
        }
        let x = area.right() - 2 - width;
        let rows = 1 + find.replace.is_some() as u16;
        s.frame(
            x - 1,
            area.y,
            width + 2,
            rows + 2,
            Style::WidgetFrame,
            Style::Widget,
        );
        let row = area.y + 1;
        let count = self
            .query()
            .map(|(query, fold)| self.all_matches(&query, fold).len());
        let info = match count {
            None => String::new(),
            Some(0) => "No results".into(),
            Some(1) => "1 match".into(),
            Some(n) => format!("{n} matches"),
        };
        s.fill(x, row, width, Style::Widget);
        let used = s.print(x, row, " Find: ", Style::Widget, width);
        let iw = str_width(&info) as u16;
        let room = width.saturating_sub(used + iw + 2);
        let query_w = s.print(x + used, row, &find.query, Style::WidgetInput, room);
        s.print(x + width - iw - 1, row, &info, Style::WidgetMuted, iw);
        if self.focus == Focus::Find && !find.in_replace {
            s.cursor = Some((x + used + query_w, row));
        }
        let Some(replace) = &find.replace else {
            return;
        };
        let row = row + 1;
        s.fill(x, row, width, Style::Widget);
        let used = s.print(x, row, " Replace: ", Style::Widget, width);
        let hint = "Enter one · Alt+Enter all";
        let hw = str_width(hint) as u16;
        let room = width.saturating_sub(used + hw + 2);
        let replace_w = s.print(x + used, row, replace, Style::WidgetInput, room);
        if used + replace_w + hw + 2 <= width {
            s.print(x + width - hw - 1, row, hint, Style::WidgetMuted, hw);
        }
        if self.focus == Focus::Find && find.in_replace {
            s.cursor = Some((x + used + replace_w, row));
        }
    }

    /// The language of the active file, as the status bar names it.
    pub(crate) fn language(&self) -> String {
        if let Some(lang) = self.plugin_language() {
            return lang.name.clone();
        }
        let doc = self.doc();
        let path = doc
            .review
            .as_ref()
            .map(|r| &r.path)
            .or(doc.buffer.path.as_ref());
        let name = match path.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
            Some("md") => "Markdown",
            Some("rs") => "Rust",
            Some("py" | "pyi") => "Python",
            Some("js" | "mjs" | "cjs" | "jsx") => "JavaScript",
            Some("json") => "JSON",
            Some("toml") => "TOML",
            Some("ts" | "tsx") => "TypeScript",
            Some("go") => "Go",
            Some("c" | "h") => "C",
            Some("cc" | "cpp" | "hpp") => "C++",
            Some("sh") => "Shell",
            Some("lua") => "Lua",
            Some("yaml" | "yml") => "YAML",
            Some("html") => "HTML",
            Some("css") => "CSS",
            _ => "Plain Text",
        };
        name.into()
    }

    /// Errors and warnings from every plugin, in every file.
    pub(crate) fn problem_counts(&self) -> (usize, usize) {
        let all = self.diagnostics.values().flatten();
        let count = |s: Severity| all.clone().filter(|d| d.severity == s).count();
        (count(Severity::Error), count(Severity::Warning))
    }

    fn render_popup(&self, s: &mut Screen) {
        let Some(popup) = &self.popup else {
            return;
        };
        // A window draws popups as cards of its own.
        if self.own_text {
            return;
        }
        let Some((cx, cy)) = self.cursor_cell().filter(|_| self.focus == Focus::Editor) else {
            return;
        };
        // Label, detail, and whether it's the selected row.
        let rows: Vec<(String, String, bool)> = match popup {
            Popup::Hover(lines) => lines
                .iter()
                .flat_map(|l| {
                    // Code keeps its indentation; only long lines wrap.
                    if str_width(l) <= POPUP_WIDTH - 2 {
                        vec![l.clone()]
                    } else {
                        panel::wrap(l, POPUP_WIDTH - 2)
                    }
                })
                .take(HOVER_ROWS)
                .map(|l| (l, String::new(), false))
                .collect(),
            Popup::Completion { selected, .. } => {
                let first = selected.saturating_sub(COMPLETION_ROWS - 1);
                self.completions()
                    .into_iter()
                    .enumerate()
                    .skip(first)
                    .take(COMPLETION_ROWS)
                    .map(|(i, c)| (c.label.clone(), c.detail.clone(), i == *selected))
                    .collect()
            }
        };
        let content = rows
            .iter()
            .map(|(l, d, _)| str_width(l) + if d.is_empty() { 0 } else { str_width(d) + 2 })
            .max()
            .unwrap_or(0);
        let width = (content + 2).min(POPUP_WIDTH).min(s.width as usize - 2) as u16;
        let height = rows.len() as u16;
        let text = self.layout().text;
        // Below the cursor if it fits, else above; a line of frame each side.
        let y0 = if cy + 2 + height < text.bottom() || cy < text.y + height + 2 {
            cy + 2
        } else {
            cy - height - 1
        };
        let x0 = cx.max(1).min(s.width - width - 1);
        s.frame(
            x0 - 1,
            y0 - 1,
            width + 2,
            height + 2,
            Style::WidgetFrame,
            Style::Popup,
        );
        for (i, (label, detail, selected)) in rows.iter().enumerate() {
            let y = y0 + i as u16;
            let style = if *selected {
                Style::Selected
            } else {
                Style::Popup
            };
            s.fill(x0, y, width, style);
            let used = s.print(x0 + 1, y, label, style, width.saturating_sub(2));
            if !detail.is_empty() {
                let muted = if *selected { style } else { Style::PopupMuted };
                let x = x0 + 1 + used + 2;
                s.print(x, y, detail, muted, (x0 + width).saturating_sub(x + 1));
            }
        }
    }

    /// The palette's input row, inside its frame.
    pub(crate) fn palette_top(&self) -> u16 {
        self.layout().menu.h + 1
    }

    fn render_palette(&self, s: &mut Screen, palette: &Palette) {
        let (x, width, first) = self.palette_geometry();
        let top = self.palette_top();
        let shown = self.palette_items().len().clamp(1, PALETTE_ROWS) as u16;
        s.frame(
            x - 1,
            top - 1,
            width + 2,
            shown + 3,
            Style::WidgetFrame,
            Style::Widget,
        );
        s.fill(x, top, width, Style::Widget);
        let prompt = match &palette.title {
            Some(title) => format!(" {title}: "),
            None => " > ".into(),
        };
        let used = s.print(x, top, &prompt, Style::Widget, width);
        let query_w = s.print(
            x + used,
            top,
            &palette.query,
            Style::WidgetInput,
            width - used,
        );
        s.cursor = Some((x + used + query_w, top));
        let items = self.palette_items();
        if items.is_empty() {
            s.fill(x, top + 1, width, Style::Widget);
            s.print(
                x + 1,
                top + 1,
                "Nothing matches",
                Style::WidgetMuted,
                width - 1,
            );
        }
        for (i, (_, title)) in items.iter().enumerate().skip(first).take(PALETTE_ROWS) {
            let y = top + (i - first + 1) as u16;
            let style = if i == palette.selected {
                Style::Selected
            } else {
                Style::Widget
            };
            s.fill(x, y, width, style);
            let hint = match palette.source {
                Source::Commands => items[i].0.as_str(),
                _ => "",
            };
            let hint = self
                .key_hint(hint)
                .map(|h| format!("{h} "))
                .unwrap_or_default();
            let hint_w = str_width(&hint) as u16;
            let used = s.print(x + 1, y, title, style, width - 1);
            if used + 2 + hint_w < width {
                let muted = if i == palette.selected {
                    style
                } else {
                    Style::WidgetMuted
                };
                s.print(x + width - hint_w, y, &hint, muted, hint_w);
            }
        }
    }
}

/// The lines a text area of `rows` rows shows from `scroll`.
pub(super) fn visible_lines(scroll: usize, rows: usize, lines: usize) -> std::ops::Range<usize> {
    scroll.min(lines)..(scroll + rows).min(lines)
}

/// The first side-by-side row to show, for a view scrolled to `scroll`.
pub(crate) fn split_first(rows: &[(Option<usize>, Option<usize>)], scroll: usize) -> usize {
    rows.iter()
        .position(|(l, r)| (*l).max(*r).is_some_and(|line| line >= scroll))
        .unwrap_or(0)
}
