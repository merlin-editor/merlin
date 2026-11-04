//! A minimap in braille, beside the scrollbar as VS Code has it. Each cell
//! holds 2×4 dots, one for every four columns of a line, so ten cells show
//! the shape of 80 columns of code, coloured like the code.

use merlin_syntax::Syntax;

use super::Editor;
use super::layout::Rect;
use crate::diff::Sign;
use crate::doc::advance;
use crate::lang::Severity;
use crate::screen::{Hue, Screen, Style};

/// Columns the minimap takes: a gap, then the braille cells.
pub(crate) const MINIMAP_W: u16 = 11;
/// Text columns per dot.
const DOT_COLUMNS: usize = 4;
/// Lines per row of cells.
const ROW_LINES: usize = 4;
/// The narrowest text area, gutter included, that keeps a minimap.
const MIN_TEXT: u16 = 80;
/// A braille dot's bit by its column and row in the cell.
const DOTS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// What a dot stands for: the code's colour if it isn't blank, and
/// whether a search match or a problem is under it.
#[derive(Clone, Copy, Default)]
struct Dot {
    syntax: Option<Syntax>,
    found: bool,
    severity: Option<Severity>,
}

impl Editor {
    /// Columns the minimap takes from the text area: none when it's off,
    /// the window is narrow, or a review is showing.
    pub(crate) fn minimap_width(&self) -> u16 {
        let on = self.minimap && self.doc().review.is_none() && self.layout().text.w >= MIN_TEXT;
        if on { MINIMAP_W } else { 0 }
    }

    /// The minimap's columns, left of the scrollbar.
    fn minimap_area(&self) -> Option<Rect> {
        let w = self.minimap_width();
        let text = self.layout().text;
        (w > 0 && text.h > 0).then(|| Rect {
            x: text.right() - 1 - w,
            w,
            ..text
        })
    }

    /// The line the minimap's top row starts with. A file taller than the
    /// minimap scrolls it in step with the text, as VS Code's does.
    fn minimap_first(&self, h: usize) -> usize {
        let doc = self.doc();
        let v = self.view();
        let lines = doc.last_line() + 1;
        let span = h * ROW_LINES;
        if lines <= span {
            return 0;
        }
        let scrollable = lines.saturating_sub(h).max(1);
        (lines - span) * v.scroll.min(scrollable) / scrollable
    }

    pub(crate) fn render_minimap(&self, s: &mut Screen) {
        let Some(area) = self.minimap_area() else {
            return;
        };
        let doc = self.doc();
        let v = self.view();
        let lines = doc.last_line() + 1;
        let h = area.h as usize;
        let first = self.minimap_first(h);
        let last = (first + h * ROW_LINES).min(lines);
        let spans = doc.spans(first..last);
        // Search matches and problems, by char range, so their cells stand
        // out as VS Code's minimap shows them.
        let lo = doc.line_start(first);
        let hi = doc.line_start(last.min(doc.last_line()))
            + doc.buffer.line_len(last.min(doc.last_line()));
        let found: Vec<(usize, usize)> = self
            .query()
            .map(|(query, fold)| {
                self.all_matches(&query, fold)
                    .into_iter()
                    .filter(|m| (lo..=hi).contains(m))
                    .map(|m| (m, m + query.len()))
                    .collect()
            })
            .unwrap_or_default();
        let marks: Vec<(usize, usize, Severity)> = doc
            .marks
            .iter()
            .filter(|m| m.severity <= Severity::Warning && m.end >= lo && m.start <= hi)
            .map(|m| (m.start, m.end.max(m.start + 1), m.severity))
            .collect();
        let cells = (area.w - 1) as usize;
        // Each line's dots: the colour of its first character in each
        // four columns, if any isn't blank, and what's found there.
        let dots: Vec<Vec<Dot>> = (first..last)
            .map(|line| {
                let start = doc.line_start(line);
                let mut dots = vec![Dot::default(); cells * 2];
                let mut d = 0;
                for (i, c) in doc.line_chars(line).into_iter().enumerate() {
                    let col = d;
                    d += advance(c, d);
                    let dot = col / DOT_COLUMNS;
                    let Some(slot) = dots.get_mut(dot) else {
                        break;
                    };
                    let pos = start + i;
                    slot.found |= found.iter().any(|&(a, b)| (a..b).contains(&pos));
                    for &(a, b, severity) in &marks {
                        if (a..b).contains(&pos) {
                            slot.severity =
                                Some(slot.severity.map_or(severity, |s| s.min(severity)));
                        }
                    }
                    if c.is_whitespace() || slot.syntax.is_some() {
                        continue;
                    }
                    slot.syntax = Some(
                        spans
                            .iter()
                            .rev()
                            .find(|(a, b, _)| (*a..*b).contains(&pos))
                            .map_or(Syntax::Plain, |&(_, _, k)| k),
                    );
                }
                dots
            })
            .collect();
        let shown = v.scroll..v.scroll + h;
        for row in 0..h {
            let y = area.y + row as u16;
            let top = first + row * ROW_LINES;
            let band = if (top..top + ROW_LINES).any(|l| shown.contains(&l)) {
                Style::MinimapView
            } else {
                Style::Minimap
            };
            s.fill(area.x + 1, y, area.w - 1, band);
            // Changed lines, in the gap left of the dots.
            let sign = (top..top + ROW_LINES)
                .filter_map(|l| doc.git_signs.get(l).copied().flatten())
                .max_by_key(|s| match s {
                    Sign::Changed => 2,
                    Sign::Added => 1,
                    Sign::Removed => 0,
                });
            if let Some(sign) = sign {
                let style = match sign {
                    Sign::Added => Style::GitAdded,
                    Sign::Changed => Style::GitChanged,
                    Sign::Removed => Style::GitRemoved,
                };
                s.put(area.x, y, '▕', style);
            }
            for cell in 0..cells {
                let mut bits = 0;
                let mut colour = None;
                let mut found = false;
                let mut severity: Option<Severity> = None;
                for (dx, column) in DOTS.iter().enumerate() {
                    for (dy, bit) in column.iter().enumerate() {
                        let Some(line) = dots.get(row * ROW_LINES + dy) else {
                            continue;
                        };
                        let dot = line[cell * 2 + dx];
                        found |= dot.found;
                        if let Some(s) = dot.severity {
                            severity = Some(severity.map_or(s, |w| w.min(s)));
                        }
                        if let Some(syntax) = dot.syntax {
                            bits |= bit;
                            colour.get_or_insert(syntax);
                        }
                    }
                }
                let x = area.x + 1 + cell as u16;
                let style = if found { Style::MinimapMatch } else { band };
                if let Some(syntax) = colour {
                    s.put(x, y, char::from_u32(0x2800 + bits).unwrap_or(' '), style);
                    s.decorate(x, y, syntax, None);
                } else if found {
                    s.put(x, y, ' ', style);
                }
                match severity {
                    Some(Severity::Error) => s.tint(x, y, Hue::Problem),
                    Some(Severity::Warning) => s.tint(x, y, Hue::Warned),
                    _ => {}
                }
            }
        }
    }

    /// Whether a screen cell is on the minimap.
    pub(crate) fn on_minimap(&self, x: u16, y: u16) -> bool {
        self.minimap_area().is_some_and(|a| a.contains(x, y))
    }

    /// Scrolls so the lines at a minimap row are in the middle of the view.
    /// While dragging, a file taller than the minimap follows the mouse in
    /// proportion, as the scrollbar does, since the minimap itself moves.
    pub(crate) fn minimap_scroll(&mut self, y: u16, dragging: bool) {
        let Some(area) = self.minimap_area() else {
            return;
        };
        let h = area.h as usize;
        let row = y.saturating_sub(area.y).min(area.h - 1) as usize;
        let lines = self.doc().last_line() + 1;
        let line = if dragging && lines > h * ROW_LINES {
            row * lines / h
        } else {
            self.minimap_first(h) + row * ROW_LINES + ROW_LINES / 2
        };
        let doc = self.doc_mut();
        doc.scroll = line
            .min(lines - 1)
            .saturating_sub(h / 2)
            .min(lines.saturating_sub(1));
    }
}
