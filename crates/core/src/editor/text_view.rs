//! The text of an editor group as plain data: each line on screen with its
//! colours, selections, matches, marks and guides, and where the cursors
//! are. The cell renderer paints it into a [`Screen`]; a window draws it in
//! pixels, with its own font, carets and scrolling.
//!
//! Positions stay in columns, as every code editor's do: a window places
//! column `c` at `c` times its font's advance.
//!
//! [`Screen`]: crate::style::Screen

use merlin_syntax::Syntax;

use super::layout::Rect;
use super::overview::{MinimapView, ScrollbarView};
use super::wrap::{self, Row};
use super::{Editor, Focus, fold, guides};
use crate::diff::{LineKind, Sign};
use crate::lang::{InlayKind, Severity};
use crate::style::{Hue, Style};

/// An editor group's text, as it's on screen now.
#[derive(Clone, Debug, PartialEq)]
pub struct TextView {
    /// The editor group it's in.
    pub group: usize,
    /// Where the text is, in cells of its group's grid.
    pub area: Rect,
    /// Cells left of the text: signs, numbers and the fold arrow.
    pub gutter: usize,
    /// Columns of text, between the gutter and the minimap or scrollbar.
    pub columns: usize,
    /// Whether the gutter's first cell holds git or review signs.
    pub signs: bool,
    /// The first line at the top, and how far the view is scrolled sideways,
    /// in columns.
    pub scroll: usize,
    pub hscroll: usize,
    /// How much of the top row is scrolled off, in rows: a window draws
    /// every row that much higher.
    pub top_frac: f32,
    /// The top, and every row there is, in rows from the top of the file.
    pub top_row: f64,
    pub total_rows: usize,
    pub wrapping: bool,
    /// Whether keys go here, so the cursors and current line show.
    pub focused: bool,
    pub lines: Vec<LineView>,
    /// Lines just out of view kept above and below, for a window to scroll
    /// smoothly into: see [`OVERSCAN`].
    pub overscan: u16,
    /// Every cursor, the primary one first.
    pub carets: Vec<CaretView>,
    /// Text an input method is composing at the primary cursor, and where
    /// its own cursor is in it.
    pub preedit: Option<(String, Option<usize>)>,
    /// Lines sticky scroll pins over the top, from the top row down.
    pub sticky: Vec<LineView>,
    /// The scrollbar's ruler, and the minimap when it's on.
    pub scrollbar: ScrollbarView,
    pub minimap: Option<MinimapView>,
}

/// A line of the file on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct LineView {
    /// The line in the buffer.
    pub line: usize,
    /// The screen row, counted from the top of the text, of its first row:
    /// below zero or past the bottom for the lines just out of view.
    pub row: i32,
    /// How it wraps; one row when it doesn't.
    pub rows: Vec<Row>,
    /// Rows it has on screen: fewer than `rows` at the bottom.
    pub shown: u16,
    /// Its number as the gutter shows it, padded to the gutter's width.
    pub number: String,
    pub number_style: Style,
    /// A git or review sign, in the gutter's first cell.
    pub sign: Option<(char, Style)>,
    /// The line's own background: added or removed in a review, or the
    /// current line.
    pub base: Style,
    /// A fold arrow in the gutter: `Some(true)` when folded.
    pub fold_arrow: Option<bool>,
    /// The block it starts is folded: an ellipsis follows it.
    pub folded: bool,
    pub chars: Vec<char>,
    /// For each character: its colour, a diagnostic under it, its style
    /// (selected, a match), a bracket colour, and an indent guide drawn in
    /// its place (`Some(active)`).
    pub syntax: Vec<Syntax>,
    pub marks: Vec<Option<Severity>>,
    pub styles: Vec<Style>,
    pub brackets: Vec<Option<Hue>>,
    /// Colours written on the line: first character, length, and the
    /// colour as `0xRRGGBBAA`.
    pub swatches: Vec<(usize, usize, u32)>,
    pub guides: Vec<Option<bool>>,
    /// Inlay hints, each before the character at its column.
    pub inlays: Vec<InlayView>,
    /// The worst problem starting on the line, shown after its end, as
    /// Error Lens does: its severity and first line.
    pub note: Option<(Severity, String)>,
    /// The line break is selected.
    pub eol_selected: bool,
}

/// An inlay hint on a line: the character it's before, the columns it
/// takes, and what it says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayView {
    pub col: usize,
    pub cols: usize,
    pub label: String,
    pub kind: InlayKind,
}

/// A cursor: where it is in the text, and on screen in the text's own
/// rows and columns, if it's in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaretView {
    pub line: usize,
    pub col: usize,
    pub cell: Option<(u16, u16)>,
}

/// A problem's message on one line: its first, and the one after when it
/// says more, as rustc's "mismatched types" then "expected `u32`, found
/// `usize`" do, but not a `help:`, a `note:` or the lint that's on.
fn note_text(message: &str) -> String {
    let mut lines = message.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().unwrap_or_default().to_string();
    match lines.next() {
        Some(more) if !["help:", "note:", "`#["].iter().any(|p| more.starts_with(p)) => {
            format!("{first}: {more}")
        }
        _ => first,
    }
}

/// Rows a window gets above and below the view, so scrolling can glide
/// as far as a page step without a gap.
pub const OVERSCAN: u16 = 8;

impl Editor {
    /// The text of every editor group, the active one last, as
    /// [`Editor::render`] draws them.
    pub fn text_views(&self) -> Vec<TextView> {
        let mut views = Vec::new();
        for g in (0..self.groups.len()).filter(|&g| g != self.group) {
            self.as_group(g, || views.extend(self.group_views(self.layout().text)));
        }
        views.extend(self.group_views(self.layout().text));
        views
    }

    /// A group's text: one view, or two halves for a review side by side.
    fn group_views(&self, area: Rect) -> Vec<TextView> {
        match self.split_rows() {
            Some(rows) => self.split_views(area, &rows),
            None => vec![self.text_view(area)],
        }
    }

    /// A review side by side, as VS Code shows diffs in a wide window: the
    /// old text on the left, the new on the right, with the scrollbar.
    fn split_views(&self, area: Rect, rows: &[(Option<usize>, Option<usize>)]) -> Vec<TextView> {
        let doc = self.doc();
        let v = self.view();
        let Some(review) = &doc.review else {
            return Vec::new();
        };
        let first = split_first(rows, v.scroll);
        let shown: Vec<_> = rows.iter().skip(first).take(area.h as usize).collect();
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
        let gutter = digits + 2;
        let half = area.w.saturating_sub(2) / 2;
        let cur = doc.line_col(v.cursor).0;
        let focused = self.focus == Focus::Editor && !self.drawing_other();
        let halves = [
            (Rect { w: half, ..area }, true),
            (
                Rect {
                    x: area.x + half + 1,
                    w: area.w - half - 1,
                    ..area
                },
                false,
            ),
        ];
        halves
            .into_iter()
            .map(|(rect, old)| {
                let lines = shown
                    .iter()
                    .enumerate()
                    .filter_map(|(row, (l, r))| {
                        let line = if old { (*l)? } else { (*r)? };
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
                            LineKind::Same if line == cur && focused => (' ', Style::CurrentLine),
                            LineKind::Same => (' ', Style::Normal),
                        };
                        let mut l = self.plain_line(line, row as i32, gutter, true);
                        l.number = if kind == LineKind::Gap {
                            " ".repeat(digits + 1)
                        } else {
                            format!("{:>digits$} ", number + 1)
                        };
                        l.number_style = if line == cur {
                            Style::GutterActive
                        } else {
                            Style::Gutter
                        };
                        l.sign = Some((sign, base));
                        l.base = base;
                        l.styles = vec![base; l.chars.len()];
                        Some(l)
                    })
                    .collect();
                TextView {
                    group: self.pane(),
                    top_row: v.scroll as f64,
                    total_rows: rows.len(),
                    area: rect,
                    gutter,
                    columns: (rect.w as usize).saturating_sub(gutter),
                    overscan: 0,
                    signs: true,
                    scroll: v.scroll,
                    hscroll: v.hscroll,
                    top_frac: 0.0,
                    wrapping: false,
                    focused,
                    lines,
                    carets: Vec::new(),
                    preedit: None,
                    sticky: Vec::new(),
                    scrollbar: self.scrollbar_view(),
                    minimap: None,
                }
            })
            .collect()
    }

    /// The text in `area` of the group drawn now.
    pub(crate) fn text_view(&self, area: Rect) -> TextView {
        let doc = self.doc();
        let v = self.view();
        let lines = doc.last_line() + 1;
        let gutter = self.gutter();
        let x0 = area.x as usize;
        let (cur_line, _) = doc.line_col(v.cursor);
        let query = self.query();
        // Every cursor's selection, when there are several.
        let selections: Vec<(usize, usize)> =
            self.carets().iter().filter_map(|c| c.selection()).collect();
        let selection = selections.first().copied();
        let hidden = self.hidden_lines();
        // A window drawing the text gets rows past both edges.
        let overscan = OVERSCAN;
        // The top may be a row part-way down the first line: its rows above
        // that are above the view.
        let (top_row, top_frac) = self.top_sub();
        let mut first = v.scroll;
        let mut above = top_row as i32;
        while above < overscan as i32 + top_row as i32 && first > 0 {
            first -= 1;
            match hidden.iter().find(|r| r.contains(&first)) {
                Some(fold) => first = fold.start.min(first),
                None => above += self.line_rows(first).len().max(1) as i32,
            }
        }
        // The lines on screen, which reach further down past folds.
        let last_shown = if self.row_based() {
            self.line_at_row(area.h as usize + overscan as usize).0
        } else {
            v.scroll + area.h as usize + overscan as usize
        };
        let visible = first..last_shown.max(v.scroll + 1);
        let spans = doc.spans(visible);
        let focused = self.focus == Focus::Editor && !self.drawing_other();
        let highlight_line = focused && selection.is_none();
        let guides = self.guides(visible_lines(first, last_shown - first, lines));
        // Fold arrows show for folded blocks, and for the others while the
        // mouse is over the gutter.
        let over_gutter = self.hover.is_some_and(|(hx, hy)| {
            area.contains(hx, hy) && (hx as usize) < x0 + gutter && !self.drawing_other()
        });
        let mut depth = guides.depth;
        let wrapping = self.wrapping();
        let signs = self.has_signs();
        let mut view = TextView {
            group: self.pane(),
            top_row: self.top_row(),
            total_rows: *self.row_index().last().unwrap_or(&0),
            area,
            gutter,
            columns: self.text_columns(),
            overscan,
            signs,
            scroll: v.scroll,
            hscroll: if wrapping { 0 } else { v.hscroll },
            top_frac,
            wrapping,
            focused,
            lines: Vec::new(),
            carets: Vec::new(),
            preedit: if focused { self.preedit.clone() } else { None },
            sticky: Vec::new(),
            scrollbar: self.scrollbar_view(),
            minimap: self.minimap_view(),
        };
        let bottom = area.h as i32 + overscan as i32;
        let mut row = -above;
        let mut line = first;
        while row < bottom && line < lines {
            if let Some(fold) = hidden.iter().find(|r| r.contains(&line)) {
                line = fold.end;
                // Brackets in the folded lines still count.
                if depth.is_some() {
                    depth = self.bracket_depth(line.min(lines - 1));
                }
                continue;
            }
            let start = doc.line_start(line);
            let chars = doc.line_chars(line);
            let end = start + chars.len();
            // The screen rows the line takes, more than one if it wraps.
            let rows = if wrapping {
                wrap::wrap(&chars, &doc.line_inlays(line), self.text_columns())
            } else {
                vec![Row {
                    start: 0,
                    col: 0,
                    indent: 0,
                }]
            };
            let folded = hidden.iter().any(|r| r.start == line + 1);
            let shown = rows.len().min((bottom - row) as usize) as u16;

            let mut syntax = vec![Syntax::Plain; chars.len()];
            for &(a, b, kind) in &spans {
                for i in a.max(start)..b.min(end) {
                    syntax[i - start] = kind;
                }
            }
            // An empty mark underlines one character, so it can be seen.
            let mut marks: Vec<Option<Severity>> = vec![None; chars.len()];
            let mut worst: Option<Severity> = None;
            let mut note: Option<&crate::lang::Mark> = None;
            for mark in doc
                .marks
                .iter()
                .filter(|m| m.start <= end && m.end >= start)
            {
                worst = Some(worst.map_or(mark.severity, |w| w.min(mark.severity)));
                if (start..=end).contains(&mark.start)
                    && mark.severity < Severity::Hint
                    && note.is_none_or(|n| mark.severity < n.severity)
                {
                    note = Some(mark);
                }
                let b = mark.end.max(mark.start + 1).min(end);
                let a = mark.start.max(start).min(b.saturating_sub(1));
                for i in a.max(start)..b {
                    let slot = &mut marks[i - start];
                    *slot = Some(slot.map_or(mark.severity, |s| s.min(mark.severity)));
                }
            }

            let review = doc.review.as_ref().and_then(|r| r.lines.get(line)).copied();
            let git = doc.git_signs.get(line).copied().flatten();
            let current = highlight_line && line == cur_line && review.is_none();
            let plain = if current {
                Style::CurrentLine
            } else {
                Style::Normal
            };
            let (sign, sign_style, base) = match (review, git) {
                (Some((LineKind::Added, _)), _) => ('+', Style::DiffAdded, Style::DiffAdded),
                (Some((LineKind::Removed, _)), _) => ('-', Style::DiffRemoved, Style::DiffRemoved),
                (Some((LineKind::Gap, _)), _) => (' ', Style::Gutter, Style::Muted),
                (None, Some(Sign::Added)) => ('▎', Style::GitAdded, plain),
                (None, Some(Sign::Changed)) => ('▎', Style::GitChanged, plain),
                (None, Some(Sign::Removed)) => ('‾', Style::GitRemoved, plain),
                _ => (' ', Style::Gutter, plain),
            };
            let number = match review {
                Some((LineKind::Gap, _)) => String::new(),
                Some((_, n)) => (n + 1).to_string(),
                None => (line + 1).to_string(),
            };
            let number = format!("{number:>w$} ", w = gutter - 1 - signs as usize);
            let number_style = match worst {
                Some(Severity::Error) => Style::GutterError,
                Some(Severity::Warning) => Style::GutterWarning,
                _ if line == cur_line => Style::GutterActive,
                _ => Style::Gutter,
            };
            let fold_arrow = (review.is_none()
                && (folded || over_gutter)
                && fold::block_end(doc, line).is_some())
            .then_some(folded);

            let mut styles = vec![base; chars.len()];
            for &(a, b) in &selections {
                for (i, style) in styles.iter_mut().enumerate() {
                    if (a..b).contains(&(start + i)) {
                        *style = Style::Selection;
                    }
                }
            }
            if let Some(matcher) = &query {
                for (m, len) in matcher.line(&chars, start) {
                    if start + m == v.cursor {
                        styles[m..m + len].fill(Style::MatchCurrent);
                    } else if selection.is_none() {
                        styles[m..m + len].fill(Style::Match);
                    }
                }
            }
            if let Some(word) = &guides.word {
                for m in guides::word_matches(&chars, word) {
                    for style in &mut styles[m..m + word.len()] {
                        if matches!(*style, Style::Normal | Style::CurrentLine) {
                            *style = Style::WordMatch;
                        }
                    }
                }
            }
            // Bracket colours, counted over the whole line, drawn or not.
            let brackets: Vec<Option<Hue>> = chars
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let d = depth.as_mut()?;
                    if matches!(syntax[i], Syntax::String | Syntax::Comment) {
                        return None;
                    }
                    let hue = |k: usize| [Hue::Bracket1, Hue::Bracket2, Hue::Bracket3][k % 3];
                    match c {
                        '(' | '[' | '{' => {
                            *d += 1;
                            Some(hue(*d - 1))
                        }
                        ')' | ']' | '}' => {
                            *d = d.saturating_sub(1);
                            Some(hue(*d))
                        }
                        _ => None,
                    }
                })
                .collect();
            // Indent guides on the first cell of each level's whitespace,
            // in the line's first row.
            let mut col = 0;
            let first_row_end = rows.get(1).map_or(chars.len(), |r| r.start);
            let line_guides = chars
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    let here = col;
                    col += crate::doc::advance(c, col);
                    (i < first_row_end
                        && matches!(c, ' ' | '\t')
                        && guides.is_guide(line, first, here))
                    .then(|| guides.is_active(line, here))
                })
                .collect();
            let eol_selected = selections.iter().any(|&(a, b)| (a..b).contains(&end));
            let first = doc.inlays.partition_point(|i| i.pos < start);
            let inlays = doc.inlays[first..]
                .iter()
                .take_while(|i| i.pos <= end)
                .map(|i| InlayView {
                    col: i.pos - start,
                    cols: i.cols,
                    label: i.label.clone(),
                    kind: i.kind,
                })
                .collect();
            let note = note
                .filter(|_| self.inline_notes)
                .map(|m| (m.severity, note_text(&m.message)));
            let swatches = if self.colour_swatches && doc.review.is_none() {
                super::swatches::find(&chars)
            } else {
                Vec::new()
            };
            view.lines.push(LineView {
                line,
                row,
                rows,
                shown,
                number,
                number_style,
                sign: signs.then_some((sign, sign_style)),
                base,
                fold_arrow,
                folded,
                chars,
                syntax,
                marks,
                styles,
                brackets,
                swatches,
                guides: line_guides,
                inlays,
                note,
                eol_selected,
            });
            row += shown as i32;
            line += 1;
        }
        view.sticky = self
            .sticky_lines(area)
            .into_iter()
            .enumerate()
            .map(|(row, line)| self.plain_line(line, row as i32, gutter, signs))
            .collect();
        view.carets = self
            .carets()
            .iter()
            .map(|c| {
                let (line, col) = doc.line_col(c.cursor);
                let cell = self.cell_of(c.cursor).and_then(|(x, y)| {
                    Some((
                        x.checked_sub(area.x + gutter as u16)?,
                        y.checked_sub(area.y)?,
                    ))
                });
                CaretView { line, col, cell }
            })
            .collect();
        view
    }

    /// A line with its colours and number only, as sticky scroll pins it.
    fn plain_line(&self, line: usize, row: i32, gutter: usize, signs: bool) -> LineView {
        let doc = self.doc();
        let start = doc.line_start(line);
        let chars: Vec<char> = doc
            .line_chars(line)
            .into_iter()
            .take_while(|c| *c != '\n' && *c != '\r')
            .collect();
        let mut syntax = vec![Syntax::Plain; chars.len()];
        for (a, b, kind) in doc.spans(line..line + 1) {
            for i in a.max(start)..b.min(start + chars.len()) {
                syntax[i - start] = kind;
            }
        }
        let n = chars.len();
        LineView {
            line,
            row,
            rows: vec![Row {
                start: 0,
                col: 0,
                indent: 0,
            }],
            shown: 1,
            number: format!("{:>w$} ", line + 1, w = gutter - 1 - signs as usize),
            number_style: Style::Gutter,
            sign: None,
            base: Style::Normal,
            fold_arrow: None,
            folded: false,
            chars,
            syntax,
            marks: vec![None; n],
            styles: vec![Style::Normal; n],
            brackets: vec![None; n],
            swatches: Vec::new(),
            guides: vec![None; n],
            inlays: Vec::new(),
            note: None,
            eol_selected: false,
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
