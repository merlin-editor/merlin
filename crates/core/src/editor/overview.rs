//! The file at a glance beside the text: the scrollbar's overview ruler,
//! with marks for changes, matches, selections and problems, and the
//! minimap, each line's code in tiny blocks of its colours, as VS Code
//! has them. The window draws both and scrolls with them.

use std::rc::Rc;

use merlin_syntax::Syntax;

use super::Editor;
use crate::diff::Sign;
use crate::lang::Severity;
use crate::style::Style;

/// The scrollbar's ruler, by rows of the file: where each mark is, as a
/// part of the whole, and what it is. The thumb is the view's top row and
/// height out of every row, which the text view has.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollbarView {
    /// `GitAdded`, `GitChanged`, `GitRemoved`, `ScrollMatch`, `Selection`,
    /// `GutterError` or `GutterWarning`.
    pub marks: Vec<(f32, Style)>,
    /// The widest line, in columns, for the scrollbar along the bottom;
    /// `None` while lines wrap.
    pub widest: Option<usize>,
}

/// Most columns of a line the minimap shows.
pub const MINIMAP_COLUMNS: usize = 120;

/// One line in the minimap: runs of characters other than spaces, each a
/// column, a length and a colour.
pub type MinimapLine = Vec<(u16, u16, Syntax)>;

/// The minimap: every line of the file in runs of colour, shared between
/// frames until the text or its colours change, and marks by line.
#[derive(Clone, Debug)]
pub struct MinimapView {
    /// The document, its version and its colours' generation: two views
    /// with the same are the same lines.
    key: (u64, u64, u64),
    pub lines: Rc<Vec<MinimapLine>>,
    /// The same kinds as the scrollbar's, by line.
    pub marks: Vec<(usize, Style)>,
}

impl PartialEq for MinimapView {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.marks == other.marks
    }
}

impl Editor {
    /// Lines with something to mark, and what: changes, matches,
    /// selections and problems, in the order they're drawn.
    fn line_marks(&self) -> Vec<(usize, Style)> {
        let doc = self.doc();
        let mut marks = Vec::new();
        for (line, sign) in doc.git_signs.iter().enumerate() {
            let style = match sign {
                Some(Sign::Added) => Style::GitAdded,
                Some(Sign::Changed) => Style::GitChanged,
                Some(Sign::Removed) => Style::GitRemoved,
                None => continue,
            };
            marks.push((line, style));
        }
        if let Some((query, fold)) = self.query() {
            for m in self.all_matches(&query, fold) {
                marks.push((doc.line_col(m).0, Style::ScrollMatch));
            }
        }
        for caret in self.carets() {
            if let Some((a, b)) = caret.selection() {
                let (first, last) = (doc.line_col(a).0, doc.line_col(b).0);
                marks.extend((first..=last).map(|l| (l, Style::Selection)));
            }
        }
        for mark in &doc.marks {
            let style = match mark.severity {
                Severity::Error => Style::GutterError,
                Severity::Warning => Style::GutterWarning,
                _ => continue,
            };
            marks.push((doc.line_col(mark.start).0, style));
        }
        marks
    }

    pub(crate) fn scrollbar_view(&self) -> ScrollbarView {
        let index = self.row_index();
        let total = (*index.last().unwrap_or(&1)).max(1) as f32;
        let marks = self
            .line_marks()
            .into_iter()
            .map(|(line, style)| (index[line.min(index.len() - 1)] as f32 / total, style))
            .collect();
        ScrollbarView {
            marks,
            widest: (!self.wrapping()).then(|| self.widest_line()),
        }
    }

    /// The widest line's columns, kept until the text changes.
    fn widest_line(&self) -> usize {
        let doc = self.doc();
        let key = (doc.id, doc.buffer.version());
        if let Some((k, widest)) = self.widest_cache.get()
            && k == key
        {
            return widest;
        }
        let widest = (0..=doc.last_line())
            .map(|l| doc.display_col(l, doc.buffer.line_len(l)))
            .max()
            .unwrap_or(0);
        self.widest_cache.set(Some((key, widest)));
        widest
    }

    /// The minimap, when it's on: not for reviews and pages.
    pub(crate) fn minimap_view(&self) -> Option<MinimapView> {
        let doc = self.doc();
        if !self.minimap || doc.review.is_some() || doc.page.is_some() {
            return None;
        }
        let key = (doc.id, doc.buffer.version(), doc.layers.colours);
        let mut cache = self.minimap_cache.borrow_mut();
        let lines = match cache.as_ref() {
            Some((k, lines)) if *k == key => lines.clone(),
            _ => {
                let lines = Rc::new(self.minimap_lines());
                *cache = Some((key, lines.clone()));
                lines
            }
        };
        Some(MinimapView {
            key,
            lines,
            marks: self.line_marks(),
        })
    }

    fn minimap_lines(&self) -> Vec<MinimapLine> {
        let doc = self.doc();
        let count = doc.last_line() + 1;
        // Each character's colour, later spans over earlier ones.
        let mut kinds = vec![Syntax::Plain; doc.buffer.len()];
        for (a, b, kind) in doc.spans(0..count) {
            let b = b.min(kinds.len());
            if a < b {
                kinds[a..b].fill(kind);
            }
        }
        let mut out = Vec::with_capacity(count);
        for line in 0..count {
            let start = doc.line_start(line);
            let mut runs: MinimapLine = Vec::new();
            let mut col = 0usize;
            for (i, c) in doc.buffer.rope().line(line).chars().enumerate() {
                if col >= MINIMAP_COLUMNS || c == '\n' || c == '\r' {
                    break;
                }
                let w = crate::doc::advance(c, col);
                if !c.is_whitespace() {
                    let syntax = kinds.get(start + i).copied().unwrap_or_default();
                    match runs.last_mut() {
                        Some((c0, len, s)) if *s == syntax && (*c0 + *len) as usize == col => {
                            *len += w as u16
                        }
                        _ => runs.push((col as u16, w as u16, syntax)),
                    }
                }
                col += w;
            }
            out.push(runs);
        }
        out
    }
}
