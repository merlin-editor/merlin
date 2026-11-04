//! The text of an editor group as plain data: each line on screen with its
//! colours, selections, matches, marks and guides, and where the cursors
//! are. The cell renderer paints it into a [`Screen`]; a window draws it in
//! pixels, with its own font, carets and scrolling.
//!
//! Positions stay in columns, as every code editor's do: a window places
//! column `c` at `c` times its font's advance.
//!
//! [`Screen`]: crate::screen::Screen

use merlin_syntax::Syntax;

use super::layout::Rect;
use super::wrap::{self, Row};
use super::{Editor, Focus, fold, guides, line_matches};
use crate::diff::{LineKind, Sign};
use crate::lang::Severity;
use crate::screen::{Hue, Style};

/// An editor group's text, as it's on screen now.
#[derive(Clone, Debug, PartialEq)]
pub struct TextView {
    /// Where the text is, in cells of the core's grid, scrollbar included.
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
    pub wrapping: bool,
    /// Whether keys go here, so the cursors and current line show.
    pub focused: bool,
    pub lines: Vec<LineView>,
    /// Lines just out of view kept above and below, for a window to scroll
    /// smoothly into: see [`OVERSCAN`].
    pub overscan: u16,
    /// Lines past the end of the file, drawn as `~`, by screen row.
    pub beyond: Vec<u16>,
    /// Every cursor, the primary one first.
    pub carets: Vec<CaretView>,
    /// Lines sticky scroll pins over the top, from the top row down.
    pub sticky: Vec<LineView>,
    /// The scrollbar, when the file is longer than the view.
    pub scrollbar: Option<ScrollbarView>,
}

/// The scrollbar, in parts of the track from its top: the thumb, and marks
/// for changes, matches and problems, as VS Code's overview ruler has them.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbarView {
    pub top: f32,
    pub len: f32,
    /// Where each mark is, and what it is: `GitAdded`, `GitChanged`,
    /// `GitRemoved`, `ScrollMatch`, `GutterError` or `GutterWarning`.
    pub marks: Vec<(f32, Style)>,
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
    pub guides: Vec<Option<bool>>,
    /// The line break is selected.
    pub eol_selected: bool,
}

/// A cursor: where it is in the text, and on screen in the text's own
/// rows and columns, if it's in view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaretView {
    pub line: usize,
    pub col: usize,
    pub cell: Option<(u16, u16)>,
}

/// Lines a window gets above and below the view, so scrolling can glide
/// as far as a page step without a gap. Not while wrapping, where rows and
/// lines differ.
pub const OVERSCAN: u16 = 8;

impl Editor {
    /// The text of every editor group, the active one last, as
    /// [`Editor::render`] draws them.
    pub fn text_views(&self) -> Vec<TextView> {
        let mut views = Vec::new();
        let panes = self.pane_rects(self.layout().editor);
        if panes.len() == self.groups.len() && panes.len() > 1 {
            for g in (0..self.groups.len()).filter(|&g| g != self.group) {
                self.as_group(g, || views.push(self.text_view(self.layout().text)));
            }
        }
        views.push(self.text_view(self.layout().text));
        views
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
        // A window drawing the text gets lines past both edges.
        let overscan = if self.own_text && !self.row_based() {
            OVERSCAN
        } else {
            0
        };
        let mut first = v.scroll;
        let mut above = 0i32;
        while above < overscan as i32 && first > 0 {
            first -= 1;
            match hidden.iter().find(|r| r.contains(&first)) {
                Some(fold) => first = fold.start.min(first),
                None => above += 1,
            }
        }
        // The lines on screen, which reach further down past folds.
        let last_shown = if self.row_based() {
            self.line_at_row(area.h as usize).0
        } else {
            v.scroll + area.h as usize + overscan as usize
        };
        let visible = first..last_shown.max(v.scroll + 1);
        let spans = doc.spans(visible);
        let focused = self.focus == Focus::Editor && self.drawing.get().is_none();
        let highlight_line = focused && selection.is_none();
        let guides = self.guides(super::render::visible_lines(
            first,
            last_shown - first,
            lines,
        ));
        // Fold arrows show for folded blocks, and for the others while the
        // mouse is over the gutter.
        let over_gutter = self.hover.is_some_and(|(hx, hy)| {
            area.contains(hx, hy) && (hx as usize) < x0 + gutter && self.drawing.get().is_none()
        });
        let mut depth = guides.depth;
        let wrapping = self.wrapping();
        let signs = self.has_signs();
        let mut view = TextView {
            area,
            gutter,
            columns: self.text_columns(),
            overscan,
            signs,
            scroll: v.scroll,
            hscroll: if wrapping { 0 } else { v.hscroll },
            wrapping,
            focused,
            lines: Vec::new(),
            beyond: Vec::new(),
            carets: Vec::new(),
            sticky: Vec::new(),
            scrollbar: None,
        };
        let bottom = area.h as i32 + overscan as i32;
        let mut row = -above;
        let mut line = first;
        while row < bottom {
            if line >= lines {
                if (0..area.h as i32).contains(&row) {
                    view.beyond.push(row as u16);
                }
                row += 1;
                continue;
            }
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
                wrap::wrap(&chars, self.text_columns())
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
            for mark in doc
                .marks
                .iter()
                .filter(|m| m.start <= end && m.end >= start)
            {
                worst = Some(worst.map_or(mark.severity, |w| w.min(mark.severity)));
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
            if let Some((query, fold)) = &query {
                for m in line_matches(&chars, query, *fold) {
                    if start + m == v.cursor {
                        styles[m..m + query.len()].fill(Style::MatchCurrent);
                    } else if selection.is_none() {
                        styles[m..m + query.len()].fill(Style::Match);
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
                guides: line_guides,
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
        view.scrollbar = self.scrollbar_view(area);
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
            guides: vec![None; n],
            eol_selected: false,
        }
    }

    fn scrollbar_view(&self, area: Rect) -> Option<ScrollbarView> {
        let doc = self.doc();
        let lines = doc.last_line() + 1;
        let h = area.h as usize;
        if lines <= h || h == 0 {
            return None;
        }
        let at = |line: usize| line as f32 / lines as f32;
        let mut marks = Vec::new();
        for (line, sign) in doc.git_signs.iter().enumerate() {
            let style = match sign {
                Some(Sign::Added) => Style::GitAdded,
                Some(Sign::Changed) => Style::GitChanged,
                Some(Sign::Removed) => Style::GitRemoved,
                None => continue,
            };
            marks.push((at(line), style));
        }
        if let Some((query, fold)) = self.query() {
            for m in self.all_matches(&query, fold) {
                marks.push((at(doc.line_col(m).0), Style::ScrollMatch));
            }
        }
        for mark in &doc.marks {
            let style = match mark.severity {
                Severity::Error => Style::GutterError,
                Severity::Warning => Style::GutterWarning,
                _ => continue,
            };
            marks.push((at(doc.line_col(mark.start).0), style));
        }
        Some(ScrollbarView {
            top: at(self.view().scroll),
            len: h as f32 / lines as f32,
            marks,
        })
    }
}
