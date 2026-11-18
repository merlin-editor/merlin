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
    key: (u64, u64, u64, usize),
    /// The document's lines, all of them.
    pub total: usize,
    /// The first line in `lines`.
    pub first: usize,
    /// The lines around the view: enough above and below it for any map
    /// a screen can show, never the whole file.
    pub lines: Rc<Vec<MinimapLine>>,
    /// The same kinds as the scrollbar's, by line, for the lines in
    /// `lines`.
    pub marks: Vec<(usize, Style)>,
}

impl MinimapView {
    /// Line `line`'s runs, if it's among those sent.
    pub fn line(&self, line: usize) -> Option<&MinimapLine> {
        self.lines.get(line.checked_sub(self.first)?)
    }
}

/// Lines the minimap gets above and below the view: more than a 4K
/// screen's height shows at two pixels a line, as the map can show from
/// as far up as its height above the view to as far down below it.
const MINIMAP_REACH: usize = 1280;
/// The lines sent start on a multiple of this, so scrolling builds them
/// again only now and then.
const MINIMAP_STEP: usize = 512;

impl PartialEq for MinimapView {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.marks == other.marks
    }
}

/// About the most places the scrollbar can mark apart.
const SCROLLBAR_PLACES: usize = 4096;

impl Editor {
    /// Lines in `lines` with something to mark, and what: changes,
    /// matches, selections and problems, in the order they're drawn. A run
    /// of lines marked alike gives every `step`th, so a run as long as the
    /// file costs what can be shown.
    fn line_marks(&self, lines: std::ops::Range<usize>, step: usize) -> Vec<(usize, Style)> {
        let doc = self.doc();
        let step = step.max(1);
        let mut marks = Vec::new();
        let run = |marks: &mut Vec<(usize, Style)>, r: std::ops::Range<usize>, style| {
            let r = r.start.max(lines.start)..r.end.min(lines.end);
            marks.extend(r.step_by(step).map(|l| (l, style)));
        };
        for (r, sign) in doc.git_marks() {
            let style = match sign {
                Sign::Added => Style::GitAdded,
                Sign::Changed => Style::GitChanged,
                Sign::Removed => Style::GitRemoved,
            };
            run(&mut marks, r.clone(), style);
        }
        // From the counts kept, not every match.
        if let Some(matcher) = self.query()
            && let Some(found) = doc.found(&matcher)
        {
            let found = found.marks(doc.buffer.rope());
            let from = found.partition_point(|&l| l < lines.start);
            let to = found.partition_point(|&l| l < lines.end);
            marks.extend(
                found[from..to]
                    .iter()
                    .map(|&line| (line, Style::ScrollMatch)),
            );
        }
        // The name's uses, where the server sends them.
        if let Some(uses) = self.current_highlights() {
            for &(a, _, write) in uses {
                let style = if write {
                    Style::WordWrite
                } else {
                    Style::WordRead
                };
                marks.push((doc.line_col(a).0, style));
            }
        }
        for caret in self.carets() {
            if let Some((a, b)) = caret.selection() {
                let (first, last) = (doc.line_col(a).0, doc.line_col(b).0);
                run(&mut marks, first..last + 1, Style::Selection);
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
        marks.retain(|(l, _)| lines.contains(l));
        marks
    }

    pub(crate) fn scrollbar_view(&self) -> ScrollbarView {
        let index = self.row_index();
        let total = index.total().max(1) as f32;
        // A mark close to the last of its kind can't be told apart from it:
        // the bar has a few thousand places at most.
        let count = self.doc().last_line() + 1;
        let gap = count / SCROLLBAR_PLACES;
        let mut last: Vec<(Style, usize)> = Vec::new();
        let marks = self
            .line_marks(0..count, gap)
            .into_iter()
            .filter(|&(line, style)| {
                if gap == 0 {
                    return true;
                }
                match last.iter_mut().find(|(s, _)| *s == style) {
                    Some((_, at)) if (*at..*at + gap).contains(&line) => false,
                    Some((_, at)) => {
                        *at = line;
                        true
                    }
                    None => {
                        last.push((style, line));
                        true
                    }
                }
            })
            .map(|(line, style)| (index.before(line) as f32 / total, style))
            .collect();
        ScrollbarView {
            marks,
            widest: (!self.wrapping()).then(|| self.widest_line()),
        }
    }

    /// The widest line's columns, with the most inlay hints a line has.
    fn widest_line(&self) -> usize {
        let doc = self.doc();
        let rope = doc.buffer.rope();
        let mut hints = 0;
        let mut line_hints = (usize::MAX, 0);
        for inlay in &doc.inlays {
            let line = rope.char_to_line(inlay.pos.min(rope.len_chars()));
            if line != line_hints.0 {
                line_hints = (line, 0);
            }
            line_hints.1 += inlay.cols;
            hints = hints.max(line_hints.1);
        }
        doc.widest() + hints
    }

    /// The minimap, when it's on: not for reviews and pages.
    pub(crate) fn minimap_view(&self) -> Option<MinimapView> {
        let doc = self.doc();
        if !self.minimap || doc.review.is_some() || doc.page.is_some() {
            return None;
        }
        let total = doc.last_line() + 1;
        let first = self.view().scroll.saturating_sub(MINIMAP_REACH) / MINIMAP_STEP * MINIMAP_STEP;
        let end = (first + 2 * MINIMAP_REACH + MINIMAP_STEP + self.text_height()).min(total);
        let key = (doc.id, doc.buffer.version(), doc.layers.colours, first);
        let mut cache = self.minimap_cache.borrow_mut();
        let lines = match cache.as_ref() {
            Some((k, lines)) if *k == key => lines.clone(),
            _ => {
                let lines = Rc::new(self.minimap_lines(first..end));
                *cache = Some((key, lines.clone()));
                lines
            }
        };
        let marks = self.line_marks(first..end, 1);
        Some(MinimapView {
            key,
            total,
            first,
            lines,
            marks,
        })
    }

    fn minimap_lines(&self, lines: std::ops::Range<usize>) -> Vec<MinimapLine> {
        let doc = self.doc();
        let rope = doc.buffer.rope();
        // The colour of each line's first characters, as many as the map
        // shows: where each line's are, and how many. Without colours,
        // as for a large file, it's all plain.
        let spans = doc.spans(lines.clone());
        let mut at = Vec::with_capacity(lines.len());
        let mut size = 0;
        for line in lines.clone().filter(|_| !spans.is_empty()) {
            let n = doc.buffer.line_len(line).min(MINIMAP_COLUMNS);
            at.push((doc.line_start(line), size, n));
            size += n;
        }
        let mut kinds = vec![Syntax::Plain; size];
        // Later spans over earlier ones.
        for (a, b, kind) in spans {
            let first = rope.char_to_line(a.min(rope.len_chars())).max(lines.start);
            for &(start, off, n) in at.iter().skip(first - lines.start) {
                if start >= b {
                    break;
                }
                let (from, to) = (a.saturating_sub(start).min(n), (b - start).min(n));
                kinds[off + from..off + to].fill(kind);
            }
        }
        let mut out = Vec::with_capacity(lines.len());
        let first = lines.start;
        // Read where they lie, not a line looked up at a time.
        let _ = crate::search::each_line(rope, lines, |line, _, text| {
            let kinds = match at.get(line - first) {
                Some(&(_, off, n)) => &kinds[off..off + n],
                None => &[][..],
            };
            let mut runs: MinimapLine = Vec::new();
            let mut col = 0usize;
            for (i, c) in text.chars().enumerate() {
                if col >= MINIMAP_COLUMNS {
                    break;
                }
                let w = crate::doc::advance(c, col);
                if !c.is_whitespace() {
                    let syntax = kinds.get(i).copied().unwrap_or_default();
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
            std::ops::ControlFlow::Continue(())
        });
        out
    }
}
