//! The merge editor, as VS Code and JetBrains have it: a conflicted file's
//! Incoming and Current versions side by side over the Result, which is
//! the file itself and can be typed in. Each line of a conflict has its
//! box, to take some of a side; Accept takes all of one. A conflict not
//! chosen yet shows its base in the Result.

use std::ops::Range;
use std::path::PathBuf;

use super::{Editor, Effect, GitAction};
use crate::style::Style;

/// Which version of a conflict a line or a choice is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeSide {
    Incoming,
    Current,
}

/// A file being merged.
pub(crate) struct Merging {
    pub doc: u64,
    pub path: String,
    /// What each side is: a branch and its commit.
    pub incoming: String,
    pub current: String,
    /// The whole file as each side has it.
    pub incoming_lines: Vec<String>,
    pub current_lines: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// The Result's text the conflicts' lines are for, and its version.
    pub text: String,
    pub version: u64,
}

/// One conflict: its lines in each side, its base, and where it is in the
/// Result.
#[derive(Clone, Debug)]
pub(crate) struct Conflict {
    pub incoming: Range<usize>,
    pub current: Range<usize>,
    pub base: Vec<String>,
    pub result: Range<usize>,
    /// Each line's box, in each side.
    pub take_incoming: Vec<bool>,
    pub take_current: Vec<bool>,
    /// Typed in by hand, so the boxes no longer say what it is.
    pub edited: bool,
}

/// Below this many columns, the merge editor is the Result alone, each
/// conflict shown in place with both sides.
const SIDE_BY_SIDE: u16 = 125;

impl Conflict {
    fn chosen(&self) -> bool {
        self.edited || self.take_incoming.contains(&true) || self.take_current.contains(&true)
    }

    /// The Result's line its note goes under: its last.
    fn note_line(&self) -> usize {
        self.result.end.max(1) - 1
    }

    /// The line both sides open under in the Result alone: the one before
    /// it, and their rows: the links, then each side's title and lines.
    fn block_line(&self) -> usize {
        self.result.start.saturating_sub(1)
    }

    fn block_rows(&self) -> usize {
        3 + self.incoming.len() + self.current.len()
    }
}

/// A conflict's sides as git's markers give them.
#[derive(Default)]
struct Marked {
    current: Vec<String>,
    base: Option<Vec<String>>,
    incoming: Vec<String>,
}

/// Splits text with git's conflict markers into its parts: lines common
/// to both, and conflicts. Lines keep their ends. `None` when it has no
/// conflicts, or its markers don't close.
fn parse(text: &str) -> Option<Vec<Result<String, Marked>>> {
    enum In {
        Common,
        Current,
        Base,
        Incoming,
    }
    let mut parts: Vec<Result<String, Marked>> = Vec::new();
    let mut state = In::Common;
    let mut conflict = Marked::default();
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        match state {
            In::Common if bare.starts_with("<<<<<<<") => state = In::Current,
            In::Common => parts.push(Ok(line.to_string())),
            In::Current | In::Base if bare == "=======" => state = In::Incoming,
            In::Current if bare.starts_with("|||||||") => {
                conflict.base = Some(Vec::new());
                state = In::Base;
            }
            In::Current => conflict.current.push(line.to_string()),
            In::Base => conflict.base.get_or_insert_default().push(line.to_string()),
            In::Incoming if bare.starts_with(">>>>>>>") => {
                parts.push(Err(std::mem::take(&mut conflict)));
                state = In::Common;
            }
            In::Incoming => conflict.incoming.push(line.to_string()),
        }
    }
    let any = parts.iter().any(|p| p.is_err());
    (matches!(state, In::Common) && any).then_some(parts)
}

/// A line's text without its end.
fn bare(line: &str) -> &str {
    line.trim_end_matches(['\n', '\r'])
}

/// Lines with ends, each ending in one.
fn ended(lines: &[String]) -> impl Iterator<Item = String> + '_ {
    lines.iter().map(|l| {
        if l.ends_with('\n') {
            l.clone()
        } else {
            format!("{l}\n")
        }
    })
}

/// Where a line boundary of the old text is in the new, given the runs
/// that changed between them.
fn shift(line: usize, runs: &[(Range<usize>, Range<usize>)]) -> usize {
    let mut delta = 0isize;
    for (old, new) in runs {
        if old.end <= line {
            delta += new.len() as isize - old.len() as isize;
        }
    }
    (line as isize + delta).max(0) as usize
}

impl Merging {
    /// Builds it from the file's text with markers, and the base version
    /// git merged from, for the bases when the markers have none: each
    /// conflict's Current lines, found in the base. The Result's text has
    /// each conflict's base in its place.
    fn new(doc: u64, path: String, text: &str, base: &str) -> Option<(Self, String)> {
        let parts = parse(text)?;
        let current_text: String = parts
            .iter()
            .flat_map(|p| match p {
                Ok(line) => vec![line.clone()],
                Err(m) => ended(&m.current).collect(),
            })
            .collect();
        let runs = crate::diff::runs(&current_text, base);
        let base_lines: Vec<&str> = base.split_inclusive('\n').collect();
        let base_of = |range: Range<usize>| -> Vec<String> {
            let (mut a, mut b) = (shift(range.start, &runs), shift(range.end, &runs));
            for (old, new) in &runs {
                if old.start < range.end && old.end > range.start {
                    a = a.min(new.start);
                    b = b.max(new.end);
                }
            }
            let b = b.min(base_lines.len());
            base_lines[a.min(b)..b]
                .iter()
                .map(|l| l.to_string())
                .collect()
        };
        let (mut incoming_lines, mut current_lines, mut result) =
            (Vec::new(), Vec::new(), Vec::new());
        let mut conflicts = Vec::new();
        for part in parts {
            match part {
                Ok(line) => {
                    incoming_lines.push(line.clone());
                    current_lines.push(line.clone());
                    result.push(line);
                }
                Err(m) => {
                    let at = (incoming_lines.len(), current_lines.len(), result.len());
                    let base = m
                        .base
                        .unwrap_or_else(|| base_of(at.1..at.1 + m.current.len()));
                    let base: Vec<String> = ended(&base).collect();
                    conflicts.push(Conflict {
                        incoming: at.0..at.0 + m.incoming.len(),
                        current: at.1..at.1 + m.current.len(),
                        result: at.2..at.2 + base.len(),
                        take_incoming: vec![false; m.incoming.len()],
                        take_current: vec![false; m.current.len()],
                        base: base.clone(),
                        edited: false,
                    });
                    incoming_lines.extend(ended(&m.incoming));
                    current_lines.extend(ended(&m.current));
                    result.extend(base);
                }
            }
        }
        let result: String = result.concat();
        Some((
            Self {
                doc,
                path,
                incoming: String::new(),
                current: String::new(),
                incoming_lines,
                current_lines,
                conflicts,
                text: result.clone(),
                version: 0,
            },
            result,
        ))
    }

    pub(crate) fn left(&self) -> usize {
        self.conflicts.iter().filter(|c| !c.chosen()).count()
    }

    /// The lines a conflict's boxes make: Incoming's ticked lines, then
    /// Current's, or its base with none.
    fn made(&self, c: &Conflict) -> Vec<String> {
        let ticked = |lines: &[String], range: &Range<usize>, take: &[bool]| -> Vec<String> {
            lines[range.clone()]
                .iter()
                .zip(take)
                .filter(|(_, t)| **t)
                .map(|(l, _)| l.clone())
                .collect()
        };
        let mut out = ticked(&self.incoming_lines, &c.incoming, &c.take_incoming);
        out.extend(ticked(&self.current_lines, &c.current, &c.take_current));
        if out.is_empty() { c.base.clone() } else { out }
    }

    /// Follows an edit of the Result: conflicts move with the lines around
    /// them, and one typed in is chosen by hand. One undone back to its
    /// base, or to what its boxes make, is that again.
    fn follow(&mut self, new: &str) {
        let runs = crate::diff::runs(&self.text, new);
        let lines: Vec<&str> = new.split_inclusive('\n').collect();
        for i in 0..self.conflicts.len() {
            let c = &self.conflicts[i];
            let (start, end) = (c.result.start, c.result.end);
            let mut a = shift(start, &runs);
            let mut b = shift(end, &runs).max(a);
            let mut touched = false;
            for (old, n) in &runs {
                let inside = old.start < end && old.end > start;
                let at_edge = start == end && old.start <= start && old.end >= start;
                if inside || at_edge {
                    a = a.min(n.start);
                    b = b.max(n.end);
                    touched = true;
                }
            }
            if !touched {
                self.conflicts[i].result = a..b;
                continue;
            }
            let now: Vec<String> = lines
                .get(a..b.min(lines.len()))
                .unwrap_or_default()
                .iter()
                .map(|l| l.to_string())
                .collect();
            let same = |x: &[String]| x.iter().map(|l| bare(l)).eq(now.iter().map(|l| bare(l)));
            let c = &mut self.conflicts[i];
            c.result = a..b;
            if same(&c.base) {
                c.take_incoming.fill(false);
                c.take_current.fill(false);
                c.edited = false;
            } else {
                let made = self.made(&self.conflicts[i]);
                self.conflicts[i].edited = !same(&made);
            }
        }
        self.text = new.to_string();
    }
}

/// The merge editor's bar, at the right of its tabs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeBarView {
    pub left: usize,
    pub conflicts: usize,
    /// The Result alone: a shorter bar.
    pub inline: bool,
    /// Not merging yet: only Open in Merge Editor.
    pub offer: bool,
}

/// A line of Incoming or Current, as its pane shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeRow {
    Line {
        /// From 1.
        number: usize,
        text: String,
        colours: Vec<(usize, usize, merlin_syntax::Syntax)>,
        /// In a conflict: which, the line's place in it, and whether its
        /// box is ticked.
        conflict: Option<(usize, usize, bool)>,
    },
    /// Under a conflict: Accept for its side.
    Accept(usize),
    /// In the Result alone, over a side's lines: which, and what it is.
    Title(MergeSide, String),
}

/// Incoming or Current, from the row the Result's top is at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergePaneView {
    pub side: MergeSide,
    /// The branch and commit.
    pub detail: String,
    pub rows: Vec<MergeRow>,
}

/// Under a conflict in the Result: what it is now, and its links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeNote {
    /// The screen row, from the top of the text.
    pub row: i32,
    pub conflict: usize,
    pub text: String,
    /// Each link's label and its id for [`super::UiEvent::Press`].
    pub links: Vec<(String, String)>,
    /// In the Result alone, a conflict not chosen yet: both sides, in the
    /// rows under its links.
    pub sides: Vec<MergeRow>,
}

/// What the merge editor shows besides the Result's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeView {
    /// Incoming and Current; none in the Result alone.
    pub panes: Vec<MergePaneView>,
    pub inline: bool,
    /// The Result's file name.
    pub name: String,
    pub notes: Vec<MergeNote>,
}

impl Editor {
    /// The file being merged in the document drawn.
    pub(crate) fn merging(&self) -> Option<&Merging> {
        let id = self.doc().id;
        self.merges.iter().find(|m| m.doc == id)
    }

    fn merging_mut(&mut self) -> Option<&mut Merging> {
        let id = self.doc().id;
        self.merges.iter_mut().find(|m| m.doc == id)
    }

    /// A conflicted file, from Source Control or a notice's Resolve: it
    /// opens, and git is asked for its sides.
    pub(crate) fn open_merge(&mut self, path: &str, fx: &mut Vec<Effect>) {
        self.open(PathBuf::from(path), fx);
        fx.push(Effect::MergeSides(path.to_string()));
    }

    /// Git's sides of a conflicted file: the base version it merged from,
    /// and what each side is. The file opens in the merge editor, once
    /// it's open.
    pub(crate) fn merge_sides(&mut self, path: String, base: String, sides: (String, String)) {
        self.pending_merge = Some((path, base, sides));
        self.start_merge();
    }

    /// Starts the merge editor for the file asked for, if it's open now.
    pub(crate) fn start_merge(&mut self) {
        let Some((path, ..)) = &self.pending_merge else {
            return;
        };
        let want = PathBuf::from(path);
        let Some(i) = self
            .docs
            .iter()
            .position(|d| d.review.is_none() && d.buffer.path.as_ref() == Some(&want))
        else {
            return;
        };
        let Some((path, base, (incoming, current))) = self.pending_merge.take() else {
            return;
        };
        let doc = &self.docs[i];
        let id = doc.id;
        if self.merges.iter().any(|m| m.doc == id) {
            return;
        }
        let text = doc.buffer.text();
        let Some((mut m, result)) = Merging::new(id, path, &text, &base) else {
            return self.notify(format!("{} has no conflicts left", doc.name()));
        };
        m.incoming = incoming;
        m.current = current;
        let doc = &mut self.docs[i];
        super::replace_text(doc, &text, &result);
        doc.cursor = m
            .conflicts
            .first()
            .map_or(0, |c| doc.line_start(c.result.start.min(doc.last_line())));
        doc.anchor = None;
        m.version = doc.buffer.version();
        self.merges.push(m);
        self.current = i;
        self.focus = super::Focus::Editor;
        self.reveal_conflict(0);
    }

    /// After an edit: each merge follows its Result.
    pub(crate) fn follow_merges(&mut self) {
        let mut gone = Vec::new();
        for (k, m) in self.merges.iter_mut().enumerate() {
            let Some(doc) = self.docs.iter().find(|d| d.id == m.doc) else {
                gone.push(k);
                continue;
            };
            if doc.buffer.version() != m.version {
                m.follow(&doc.buffer.text());
                m.version = doc.buffer.version();
            }
        }
        for k in gone.into_iter().rev() {
            self.merges.remove(k);
        }
    }

    /// Whether the merge editor is the Result alone: in a narrow group,
    /// unless Alt+I says otherwise.
    pub(crate) fn merge_inline(&self) -> bool {
        self.merge_inline
            .unwrap_or(self.layout().text.w < SIDE_BY_SIDE)
    }

    /// Rows the merge editor opens after `line`: a conflict's note, or in
    /// the Result alone, both sides of one not chosen yet.
    pub(crate) fn merge_gap(&self, line: usize) -> usize {
        let Some(m) = self.merging() else { return 0 };
        let inline = self.merge_inline();
        m.conflicts
            .iter()
            .map(|c| match inline && !c.chosen() {
                true if c.block_line() == line => c.block_rows(),
                false if c.note_line() == line => 1,
                _ => 0,
            })
            .sum()
    }

    /// The notes and blocks after `line`, from screen row `row`, and the
    /// row after them.
    pub(crate) fn merge_notes(&self, line: usize, mut row: i32) -> (Vec<MergeNote>, i32) {
        let mut out = Vec::new();
        let Some(m) = self.merging() else {
            return (out, row);
        };
        let inline = self.merge_inline();
        for (i, c) in m.conflicts.iter().enumerate() {
            if inline && !c.chosen() {
                if c.block_line() == line {
                    out.extend(self.merge_block(i, row));
                    row += c.block_rows() as i32;
                }
            } else if c.note_line() == line {
                out.extend(self.merge_note(i, row));
                row += 1;
            }
        }
        (out, row)
    }

    /// A file's extension, for its colours.
    fn merge_ext(m: &Merging) -> String {
        std::path::Path::new(&m.path)
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }

    /// Each of `lines` with its colours, as `ext`.
    fn coloured(
        &self,
        ext: &str,
        lines: &[&str],
    ) -> Vec<Vec<(usize, usize, merlin_syntax::Syntax)>> {
        let spans = self.colours.get(ext, &lines.join("\n")).unwrap_or_default();
        let mut offset = 0;
        lines
            .iter()
            .map(|text| {
                let len = text.chars().count();
                let colours = spans
                    .iter()
                    .filter(|&&(a, b, _)| b > offset && a < offset + len)
                    .map(|&(a, b, s)| (a.saturating_sub(offset), (b - offset).min(len), s))
                    .collect();
                offset += len + 1;
                colours
            })
            .collect()
    }

    /// Conflict `i` in the Result alone, at screen row `row`: its links,
    /// then Incoming's lines and Current's under their titles.
    fn merge_block(&self, i: usize, row: i32) -> Option<MergeNote> {
        let m = self.merging()?;
        let c = m.conflicts.get(i)?;
        let ext = Self::merge_ext(m);
        let link = |label: &str, what: &str| (label.to_string(), format!("merge:{what}:{i}"));
        let mut sides = Vec::new();
        for (side, label, lines, range) in [
            (
                MergeSide::Incoming,
                &m.incoming,
                &m.incoming_lines,
                &c.incoming,
            ),
            (MergeSide::Current, &m.current, &m.current_lines, &c.current),
        ] {
            sides.push(MergeRow::Title(side, label.clone()));
            let texts: Vec<&str> = lines[range.clone()].iter().map(|l| bare(l)).collect();
            for (k, (text, colours)) in texts.iter().zip(self.coloured(&ext, &texts)).enumerate() {
                sides.push(MergeRow::Line {
                    number: range.start + k + 1,
                    text: text.to_string(),
                    colours,
                    conflict: Some((i, k, false)),
                });
            }
        }
        Some(MergeNote {
            row,
            conflict: i,
            text: String::new(),
            links: vec![
                link("Accept Incoming", "incoming"),
                link("Accept Current", "current"),
                link("Accept Both", "both"),
                link("Side by side", "side"),
            ],
            sides,
        })
    }

    /// The Result's line's look in a conflict: from Incoming, from Current,
    /// or its base not chosen yet.
    pub(crate) fn merge_style(&self, line: usize) -> Option<Style> {
        let m = self.merging()?;
        let c = m.conflicts.iter().find(|c| c.result.contains(&line))?;
        if c.edited {
            return None;
        }
        if !c.chosen() {
            return Some(Style::MergeBase);
        }
        let from_incoming = c.take_incoming.iter().filter(|t| **t).count();
        Some(if line - c.result.start < from_incoming {
            Style::MergeIncoming
        } else {
            Style::MergeCurrent
        })
    }

    /// The note under conflict `i` in the Result, at screen row `row`.
    pub(crate) fn merge_note(&self, i: usize, row: i32) -> Option<MergeNote> {
        let c = self.merging()?.conflicts.get(i)?;
        let link = |label: &str, what: &str| (label.to_string(), format!("merge:{what}:{i}"));
        let (from_in, from_cur) = (
            c.take_incoming.contains(&true),
            c.take_current.contains(&true),
        );
        let (text, links) = if c.edited {
            ("Edited by hand", vec![link("Reset", "reset")])
        } else if !c.chosen() {
            (
                "Not chosen yet",
                vec![
                    link("Accept Incoming", "incoming"),
                    link("Accept Current", "current"),
                    link("Accept Both", "both"),
                ],
            )
        } else {
            let text = match (from_in, from_cur) {
                (true, true) => "Chose a line from each",
                (true, false) => "From Incoming",
                _ => "From Current",
            };
            (
                text,
                vec![link("Accept Both", "both"), link("Reset", "reset")],
            )
        };
        Some(MergeNote {
            row,
            conflict: i,
            text: text.into(),
            links,
            sides: Vec::new(),
        })
    }

    /// The panes and notes, for a Result `rows` high.
    pub(crate) fn merge_view(&self, rows: usize, notes: Vec<MergeNote>) -> Option<MergeView> {
        let m = self.merging()?;
        let top = self.view().scroll;
        let ext = Self::merge_ext(m);
        let pane = |side: MergeSide| {
            let (lines, detail) = match side {
                MergeSide::Incoming => (&m.incoming_lines, &m.incoming),
                MergeSide::Current => (&m.current_lines, &m.current),
            };
            let range = |c: &Conflict| match side {
                MergeSide::Incoming => c.incoming.clone(),
                MergeSide::Current => c.current.clone(),
            };
            let take = |c: &Conflict| match side {
                MergeSide::Incoming => c.take_incoming.clone(),
                MergeSide::Current => c.take_current.clone(),
            };
            // The line level with the Result's top: past each conflict
            // above it, the difference in their lengths.
            let mut first = top as isize;
            for c in &m.conflicts {
                if top >= c.result.end {
                    first += range(c).len() as isize - c.result.len() as isize;
                } else if top >= c.result.start {
                    first = range(c).start as isize;
                    break;
                } else {
                    break;
                }
            }
            let first = (first.max(0) as usize).min(lines.len().saturating_sub(1));
            let shown: Vec<&str> = lines
                .iter()
                .skip(first)
                .take(rows + 1)
                .map(|l| bare(l))
                .collect();
            let mut out = Vec::new();
            let colours = self.coloured(&ext, &shown);
            for (k, (text, colours)) in shown.iter().zip(colours).enumerate() {
                let n = first + k;
                let at = m
                    .conflicts
                    .iter()
                    .enumerate()
                    .find(|(_, c)| range(c).contains(&n));
                out.push(MergeRow::Line {
                    number: n + 1,
                    text: text.to_string(),
                    colours,
                    conflict: at.map(|(i, c)| {
                        let place = n - range(c).start;
                        (i, place, take(c).get(place).copied().unwrap_or(false))
                    }),
                });
                // Accept goes under the conflict's last line.
                if let Some((i, _)) = at.filter(|(_, c)| range(c).end == n + 1) {
                    out.push(MergeRow::Accept(i));
                }
            }
            MergePaneView {
                side,
                detail: detail.clone(),
                rows: out,
            }
        };
        let name = std::path::Path::new(&m.path)
            .file_name()
            .map_or(m.path.clone(), |n| n.to_string_lossy().to_string());
        let inline = self.merge_inline();
        Some(MergeView {
            panes: if inline {
                Vec::new()
            } else {
                vec![pane(MergeSide::Incoming), pane(MergeSide::Current)]
            },
            inline,
            name,
            notes,
        })
    }

    /// The merge editor's bar; for a file with conflicts opened another
    /// way, Open in Merge Editor.
    pub(crate) fn merge_bar(&self) -> Option<MergeBarView> {
        let Some(m) = self.merging() else {
            let doc = self.doc();
            let path = doc.buffer.path.as_ref()?.to_string_lossy();
            // Git's status is a step behind a file just completed, so the
            // markers say.
            let conflicted = doc.review.is_none()
                && self.git.files.iter().any(|(p, s)| *s == '!' && *p == path)
                && doc
                    .buffer
                    .rope()
                    .lines()
                    .any(|l| l.chars().take(8).eq("<<<<<<< ".chars()));
            return conflicted.then_some(MergeBarView {
                left: 0,
                conflicts: 0,
                inline: false,
                offer: true,
            });
        };
        Some(MergeBarView {
            left: m.left(),
            conflicts: m.conflicts.len(),
            inline: self.merge_inline(),
            offer: false,
        })
    }

    /// Puts conflict `i`'s lines in the Result as its boxes make them, as
    /// an edit Ctrl+Z undoes.
    fn apply_conflict(&mut self, i: usize) {
        let Some(m) = self.merging() else { return };
        let Some(c) = m.conflicts.get(i) else { return };
        let made = m.made(c);
        let range = c.result.clone();
        let old = m.text.clone();
        let mut lines: Vec<String> = old.split_inclusive('\n').map(str::to_string).collect();
        let end = range.end.min(lines.len());
        let start = range.start.min(end);
        let added = made.len();
        lines.splice(start..end, made);
        let new: String = lines.concat();
        let doc = self.doc_mut();
        super::replace_text(doc, &old, &new);
        let version = doc.buffer.version();
        let Some(m) = self.merging_mut() else { return };
        let delta = added as isize - (end - start) as isize;
        for (k, c) in m.conflicts.iter_mut().enumerate() {
            if k == i {
                c.result = start..start + added;
                c.edited = false;
            } else if c.result.start >= end {
                let s = (c.result.start as isize + delta) as usize;
                let e = (c.result.end as isize + delta) as usize;
                c.result = s..e;
            }
        }
        m.text = new;
        m.version = version;
    }

    /// A pane line's box: ticks or unticks it.
    pub(crate) fn toggle_merge_line(&mut self, i: usize, side: MergeSide, line: usize) {
        let Some(m) = self.merging_mut() else { return };
        let Some(c) = m.conflicts.get_mut(i) else {
            return;
        };
        let take = match side {
            MergeSide::Incoming => &mut c.take_incoming,
            MergeSide::Current => &mut c.take_current,
        };
        if let Some(t) = take.get_mut(line) {
            *t = !*t;
        }
        self.apply_conflict(i);
    }

    /// Accept: all of one side, both (Incoming's lines first), or neither,
    /// back to the base.
    pub(crate) fn accept_conflict(&mut self, i: usize, what: &str) {
        let Some(m) = self.merging_mut() else { return };
        let Some(c) = m.conflicts.get_mut(i) else {
            return;
        };
        let (inc, cur) = match what {
            "incoming" => (true, false),
            "current" => (false, true),
            "both" => (true, true),
            _ => (false, false),
        };
        c.take_incoming.fill(inc);
        c.take_current.fill(cur);
        self.apply_conflict(i);
    }

    /// A merge editor link or box, by its id.
    pub(crate) fn merge_press(&mut self, id: &str) {
        let parts: Vec<&str> = id.split(':').collect();
        match parts.as_slice() {
            ["merge", "side", _] => self.merge_inline = Some(false),
            ["merge", "box", side, i, line] => {
                let side = if *side == "i" {
                    MergeSide::Incoming
                } else {
                    MergeSide::Current
                };
                if let (Ok(i), Ok(line)) = (i.parse(), line.parse()) {
                    self.toggle_merge_line(i, side, line);
                }
            }
            ["merge", what, i] => {
                if let Ok(i) = i.parse() {
                    self.accept_conflict(i, what);
                }
            }
            _ => {}
        }
    }

    /// The bar's and the palette's merge commands.
    pub(crate) fn merge_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        // A notice's Open lib.rs and Commit Merge.
        if let Some(path) = id.strip_prefix("merge.open:") {
            return self.open_merge(path, fx);
        }
        if id == "merge.commit" {
            return self.finish_merge(fx);
        }
        if id == "merge.open_here" {
            let path = self.doc().buffer.path.clone();
            if let Some(path) = path {
                self.open_merge(&path.to_string_lossy(), fx);
            }
            return;
        }
        if self.merging().is_none() {
            return self.notify("No file is being merged here".into());
        }
        match id {
            "merge.next" => self.step_conflict(true),
            "merge.prev" => self.step_conflict(false),
            "merge.accept_all_incoming" | "merge.accept_all_current" => {
                let what = if id.ends_with("incoming") {
                    "incoming"
                } else {
                    "current"
                };
                let left: Vec<usize> = self
                    .merging()
                    .map(|m| {
                        (0..m.conflicts.len())
                            .filter(|&i| !m.conflicts[i].chosen())
                            .collect()
                    })
                    .unwrap_or_default();
                for i in left {
                    self.accept_conflict(i, what);
                }
            }
            "merge.complete" => self.complete_merge(fx),
            _ => {}
        }
    }

    /// ↑ ↓: the cursor goes to the next or previous conflict not chosen
    /// yet, else the next or previous one.
    fn step_conflict(&mut self, forward: bool) {
        let Some(m) = self.merging() else { return };
        let line = self.doc().line_col(self.doc().cursor).0;
        let order: Vec<usize> = (0..m.conflicts.len()).collect();
        let pick = |only_left: bool| {
            let ok = |&&i: &&usize| !only_left || !m.conflicts[i].chosen();
            if forward {
                order
                    .iter()
                    .filter(ok)
                    .find(|&&i| m.conflicts[i].result.start > line)
                    .or_else(|| order.iter().find(ok))
                    .copied()
            } else {
                order
                    .iter()
                    .rev()
                    .filter(ok)
                    .find(|&&i| m.conflicts[i].result.start < line)
                    .or_else(|| order.iter().rev().find(ok))
                    .copied()
            }
        };
        let Some(i) = pick(true).or_else(|| pick(false)) else {
            return;
        };
        let start = m.conflicts[i].result.start;
        let target = start.min(self.doc().last_line());
        let pos = self.doc().line_start(target);
        self.set_cursor(pos);
        self.reveal_conflict(i);
    }

    /// Scrolls so conflict `i` shows, a little below the top.
    fn reveal_conflict(&mut self, i: usize) {
        let Some(start) = self
            .merging()
            .and_then(|m| m.conflicts.get(i))
            .map(|c| c.result.start)
        else {
            return;
        };
        let height = self.text_height().max(1);
        let doc = self.doc_mut();
        if start < doc.scroll || start + 3 > doc.scroll + height {
            doc.scroll = start.saturating_sub(3);
        }
    }

    /// Complete Merge: asks first while conflicts are left.
    pub(crate) fn complete_merge(&mut self, fx: &mut Vec<Effect>) {
        match self.merging() {
            Some(m) if m.left() > 0 => self.ask(super::dialog::Ask::MergeLeft(m.doc)),
            Some(_) => self.finish_merge_file(fx),
            None => {}
        }
    }

    /// Saves the Result and stages it, which tells git it's resolved, and
    /// the tab is the file again. The notice offers the next file with
    /// conflicts, or else to commit the merge.
    pub(crate) fn finish_merge_file(&mut self, fx: &mut Vec<Effect>) {
        let id = self.doc().id;
        let Some(k) = self.merges.iter().position(|m| m.doc == id) else {
            return;
        };
        let m = self.merges.remove(k);
        // Git sees the file on disk, so it's staged once it's saved.
        if self.doc().buffer.is_dirty() {
            self.saving.quiet.insert(id);
            self.merge_staging.push((id, m.path.clone()));
            self.save_current(fx);
        } else {
            fx.push(Effect::Git(GitAction::Stage(m.path.clone())));
        }
        let name = self.doc().name();
        let next = self
            .git
            .files
            .iter()
            .find(|(p, s)| *s == '!' && *p != m.path)
            .map(|(p, _)| p.clone());
        match next {
            Some(path) => {
                let next_name = path.rsplit('/').next().unwrap_or(&path).to_string();
                self.notify(format!(
                    "{name} is resolved and staged. {next_name} is next."
                ));
                self.add_action(&format!("Open {next_name}"), &format!("merge.open:{path}"));
            }
            None => {
                self.notify_done("No conflicts left.".into());
                let label = match self.git.merge.as_ref().map(|g| g.kind) {
                    Some(super::MergeKind::Rebase) => "Continue Rebase",
                    Some(super::MergeKind::CherryPick) => "Continue Cherry-Pick",
                    _ => "Commit Merge",
                };
                if self.git.merge.is_some() {
                    self.add_action(label, "merge.commit");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKED: &str = "a\n<<<<<<< HEAD\nours 1\nours 2\n=======\ntheirs\n>>>>>>> themes\nb\n";
    const BASE: &str = "a\nbase\nb\n";

    #[test]
    fn markers_split_into_sides_and_the_result_has_the_base() {
        let (m, result) = Merging::new(1, "f.rs".into(), MARKED, BASE).unwrap();
        assert_eq!(result, "a\nbase\nb\n");
        assert_eq!(m.current_lines.concat(), "a\nours 1\nours 2\nb\n");
        assert_eq!(m.incoming_lines.concat(), "a\ntheirs\nb\n");
        let c = &m.conflicts[0];
        assert_eq!((c.current.clone(), c.incoming.clone()), (1..3, 1..2));
        assert_eq!(c.result, 1..2);
        assert_eq!(m.left(), 1);
    }

    #[test]
    fn a_conflict_follows_edits_around_it_and_one_typed_in_is_chosen() {
        let (mut m, _) = Merging::new(1, "f.rs".into(), MARKED, BASE).unwrap();
        m.follow("new\na\nbase\nb\n");
        assert_eq!(m.conflicts[0].result, 2..3);
        assert_eq!(m.left(), 1);
        m.follow("new\na\nmine\nb\n");
        assert!(m.conflicts[0].edited);
        assert_eq!(m.left(), 0);
        m.follow("new\na\nbase\nb\n");
        assert_eq!(m.left(), 1, "undone back to its base");
    }
}
