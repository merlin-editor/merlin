//! Where you are in a file, as VS Code shows it: breadcrumbs under the tabs
//! (the file's folders, the file, and the symbols around the cursor), and
//! sticky scroll, which pins the first lines of the blocks in view to the
//! top of the text.

use merlin_syntax::Syntax;

use super::Editor;
use super::fold::block_end;
use super::guides::indent;
use super::layout::Rect;
use crate::screen::{Screen, Style, Surface, str_width};
use crate::symbols::{self, Symbol};

/// Most lines sticky scroll pins.
const MAX_STICKY: usize = 5;

impl Editor {
    pub(crate) fn shows_crumbs(&self) -> bool {
        self.breadcrumbs && self.doc().review.is_none() && self.doc().buffer.path.is_some()
    }

    /// The symbols around the cursor, outermost first.
    pub(crate) fn symbol_path(&self) -> Vec<Symbol> {
        let doc = self.doc();
        let line = doc.line_col(self.view().cursor).0;
        symbols::path_to(&symbols::symbols(doc), doc, line)
    }

    /// The breadcrumbs: each part's text, and the line it goes to, if any.
    fn crumbs(&self) -> Vec<(String, Option<usize>)> {
        let doc = self.doc();
        let Some(path) = &doc.buffer.path else {
            return Vec::new();
        };
        let mut parts: Vec<(String, Option<usize>)> = path
            .iter()
            .map(|p| (p.to_string_lossy().to_string(), None))
            .collect();
        if let Some((name, _)) = parts.last_mut() {
            let (glyph, _) = crate::icons::file_icon(name, self.nerd);
            *name = format!("{glyph} {name}");
        }
        for s in self.symbol_path() {
            // A plugin's symbols show by name, as VS Code shows them.
            let name = if s.kind == "impl" || s.kind == "#" || s.end_line.is_some() {
                s.name
            } else {
                format!("{} {}", s.kind, s.name)
            };
            parts.push((name, Some(s.line)));
        }
        parts
    }

    /// Each breadcrumb's columns on screen and the line it goes to.
    pub(crate) fn crumb_spans(&self, area: Rect) -> Vec<(u16, u16, Option<usize>)> {
        let mut x = area.x + 1;
        let mut spans = Vec::new();
        for (i, (text, line)) in self.crumbs().into_iter().enumerate() {
            if i > 0 {
                x += 3;
            }
            let w = str_width(&text) as u16;
            spans.push((x, x + w, line));
            x += w;
        }
        spans
    }

    pub(crate) fn render_crumbs(&self, s: &mut Screen, area: Rect) {
        if area.h == 0 {
            return;
        }
        let crumbs = self.crumbs();
        let spans = self.crumb_spans(area);
        let last = crumbs.len().saturating_sub(1);
        for (i, ((text, _), (x, end, _))) in crumbs.iter().zip(&spans).enumerate() {
            if *x >= area.right() {
                break;
            }
            if i > 0 {
                s.print(x - 3, area.y, " › ", Style::Muted, area.right() - (x - 3));
            }
            let style = if i == last {
                Style::Normal
            } else {
                Style::Muted
            };
            s.print(*x, area.y, text, style, area.right() - x);
            if self
                .hover
                .is_some_and(|(hx, hy)| hy == area.y && (*x..*end).contains(&hx))
            {
                s.hover(*x, area.y, end.min(&area.right()) - x);
            }
        }
    }

    /// Goes to what a click on the breadcrumbs at `x` names.
    pub(crate) fn crumb_click(&mut self, area: Rect, x: u16) {
        let hit = self
            .crumb_spans(area)
            .into_iter()
            .find(|(a, b, _)| (*a..*b).contains(&x))
            .and_then(|(_, _, line)| line);
        if let Some(line) = hit {
            self.go_to_block(line);
        }
    }

    /// Puts the cursor at the start of the text on `line`, for a jump back.
    pub(crate) fn go_to_block(&mut self, line: usize) {
        let doc = self.doc();
        let id = doc.id;
        let cursor = doc.cursor;
        let lead = doc
            .line_chars(line)
            .iter()
            .take_while(|c| matches!(c, ' ' | '\t'))
            .count();
        let pos = doc.line_start(line) + lead;
        self.jumps.push((id, cursor));
        let doc = self.doc_mut();
        doc.cursor = pos;
        doc.anchor = None;
        doc.goal = None;
        self.focus = super::Focus::Editor;
    }

    /// The lines that start the blocks around `line`, outermost first.
    fn blocks_around(&self, line: usize) -> Vec<usize> {
        let doc = self.doc();
        let Some(mut target) = (line..=doc.last_line())
            .take(50)
            .find_map(|l| indent(doc, l))
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for l in (0..line).rev().take(5000) {
            if target == 0 {
                break;
            }
            let Some(i) = indent(doc, l) else { continue };
            if i < target && block_end(doc, l).is_some_and(|end| end >= line) {
                out.push(l);
                target = i;
            }
        }
        out.reverse();
        out
    }

    /// The lines sticky scroll pins over the top of the text: the blocks
    /// the first line under them is in. Never over the cursor.
    pub(crate) fn sticky_lines(&self, area: Rect) -> Vec<usize> {
        if !self.sticky_scroll || self.doc().review.is_some() || area.h < 8 {
            return Vec::new();
        }
        let v = self.view();
        let row_of = |rows: usize| {
            if self.row_based() {
                self.line_at_row(rows).0
            } else {
                v.scroll + rows
            }
        };
        let most = MAX_STICKY.min(area.h as usize / 3);
        let mut lines: Vec<usize> = Vec::new();
        // Pinning lines covers more, which may be inside other blocks:
        // settle on lines that start the blocks of the first line still
        // seen below them.
        for _ in 0..4 {
            let first = row_of(lines.len());
            let mut next: Vec<usize> = self
                .blocks_around(first)
                .into_iter()
                .filter(|&l| l < first)
                .collect();
            next.truncate(most);
            if next == lines {
                break;
            }
            lines = next;
        }
        // Not over the cursor's row.
        let (cur_line, r, _) = self.visual(v.cursor);
        if let Some(row) = self.screen_row(cur_line, r) {
            lines.truncate(row);
        }
        lines
    }

    pub(crate) fn render_sticky(&self, s: &mut Screen, area: Rect) {
        let lines = self.sticky_lines(area);
        if lines.is_empty() {
            return;
        }
        let doc = self.doc();
        let gutter = self.gutter();
        let width = (gutter + self.text_columns()) as u16;
        for (row, &line) in lines.iter().enumerate() {
            let y = area.y + row as u16;
            s.shade(area.x, y, width, 1, Surface::Sticky);
            let signs = self.has_signs() as usize;
            let number = format!("{:>w$} ", line + 1, w = gutter - 1 - signs);
            let x0 = area.x + signs as u16;
            s.print(x0, y, &number, Style::Gutter, (gutter - signs) as u16);
            let start = doc.line_start(line);
            let spans = doc.spans(line..line + 1);
            let mut x = area.x + gutter as u16;
            let mut d = 0;
            for (i, &c) in doc.line_chars(line).iter().enumerate() {
                if c == '\n' || c == '\r' {
                    break;
                }
                let w = crate::doc::advance(c, d) as u16;
                d += w as usize;
                if x + w > area.x + width {
                    break;
                }
                let syntax = spans
                    .iter()
                    .find(|(a, b, _)| (*a..*b).contains(&(start + i)))
                    .map_or(Syntax::Plain, |(_, _, k)| *k);
                for cx in x..x + w {
                    s.put(cx, y, if c == '\t' { ' ' } else { c }, Style::Normal);
                    s.decorate(cx, y, syntax, None);
                }
                x += w;
            }
            if self
                .hover
                .is_some_and(|(hx, hy)| hy == y && hx >= area.x && hx < area.x + width)
            {
                s.hover(area.x, y, width);
            }
        }
    }

    /// The line a click on a sticky row goes to.
    pub(crate) fn sticky_at(&self, area: Rect, y: u16) -> Option<usize> {
        let row = y.checked_sub(area.y)? as usize;
        self.sticky_lines(area).get(row).copied()
    }
}
