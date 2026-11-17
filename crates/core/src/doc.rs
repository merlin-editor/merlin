//! An open file: its buffer plus where the user is in it.

use std::path::PathBuf;

use crate::buffer::{Buffer, Change};
use crate::diff::{self, LineKind, Sign};
use crate::indent::{self, Indent};
use crate::lang::{Inlay, Mark};
use crate::layers::Layers;
use crate::style::char_width;
use merlin_syntax::Syntax;

pub const TAB_WIDTH: usize = 4;

/// A change a plugin proposes, shown read-only for the user to accept or
/// reject. The buffer holds the diff's lines.
#[derive(Clone)]
pub struct Review {
    pub plugin: String,
    pub id: String,
    /// The file the change is to.
    pub path: PathBuf,
    /// What each buffer line is, and its line number in the file.
    pub lines: Vec<(LineKind, usize)>,
    /// Each buffer line's number in the old text, for side-by-side views.
    pub old_numbers: Vec<usize>,
    /// The document to go back to once it's answered.
    pub back_to: u64,
    /// Changed words in the buffer, as character ranges, to tint more than
    /// their lines.
    pub words: Vec<(usize, usize)>,
    /// For changes since the last commit: whether each changed buffer line
    /// is staged. `None` for unchanged lines.
    pub staged: Vec<Option<bool>>,
}

/// A cursor besides the document's own, with its selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub cursor: usize,
    pub anchor: Option<usize>,
}

impl Caret {
    /// A cursor with nothing selected.
    pub fn at(cursor: usize) -> Self {
        Self {
            cursor,
            anchor: None,
        }
    }

    pub fn start(&self) -> usize {
        self.anchor.map_or(self.cursor, |a| a.min(self.cursor))
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }
}

/// The top of a view, past the first line shown: which of the line's rows
/// is at the top, and the part of that row scrolled off. It's kept with the
/// line it's for, so whatever moves `scroll` leaves it behind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sub {
    pub line: usize,
    pub row: usize,
    pub frac: f32,
}

pub struct Doc {
    /// Stays the same for as long as the file is open.
    pub id: u64,
    pub buffer: Buffer,
    pub cursor: usize,
    /// The other end of the selection, if there is one.
    pub anchor: Option<usize>,
    /// Display column that vertical moves aim for.
    pub goal: Option<usize>,
    /// First line shown.
    pub scroll: usize,
    /// Where in it the view starts; see [`Sub`].
    pub scroll_sub: Sub,
    /// Display columns scrolled off the left.
    pub hscroll: usize,
    /// Diagnostics from plugins.
    pub marks: Vec<Mark>,
    /// Inlay hints from plugins, in order.
    pub inlays: Vec<Inlay>,
    pub review: Option<Review>,
    /// The file as of the last commit, for gutter signs. `None` outside git.
    pub git_base: Option<String>,
    /// A sign for each line, from comparing the text with `git_base`.
    pub git_signs: Vec<Option<Sign>>,
    /// The lines with a sign, in order, for what draws all of them, such
    /// as the scrollbar, every frame.
    pub git_marks: Vec<(usize, Sign)>,
    /// More cursors, for editing in several places at once.
    pub extra: Vec<Caret>,
    /// The text version `extra` was last placed for. Edits after it, such
    /// as an undo or a plugin's, move them along.
    pub extra_version: u64,
    /// Folded blocks, by where their first line starts.
    pub folds: Vec<usize>,
    /// Colours, symbols and folds that plugins sent.
    pub layers: Layers,
    /// A page the window draws instead of text, such as a plugin's: its
    /// name and title. Pages can't be edited.
    pub page: Option<(String, String)>,
    /// The indentation the file already uses, if it has any.
    pub indent: Option<Indent>,
    /// What `.editorconfig` files say about it.
    pub config: crate::editor::FileConfig,
    /// The widest line, measured once and kept up to date with edits.
    widest: std::cell::Cell<Option<Widest>>,
    /// The last long line's checkpoints, for finding its columns.
    checkpoints: std::cell::RefCell<Option<Checkpoints>>,
}

/// A long line's checkpoints: a character every few thousand, on a tab
/// stop, with its display column, worked out as far along as asked.
#[derive(Debug)]
struct Checkpoints {
    /// The text's version and the line they're for.
    version: u64,
    line: usize,
    /// Each one's character, byte and column in the line, in order; the
    /// first is its start.
    points: Vec<(usize, usize, usize)>,
    /// Worked out to the line's end.
    whole: bool,
}

/// The widest line's columns, without inlay hints.
#[derive(Clone, Copy, Debug)]
struct Widest {
    cols: usize,
    /// The line's characters, until an edit to it in a file too large to
    /// measure again; `cols` is then at most what it is.
    span: Option<(usize, usize)>,
    /// The buffer's version it's for.
    version: u64,
}

/// Lines this long keep checkpoints of their columns, so a column far
/// along one is found from near it.
const CHECKPOINT_LINE: usize = 16_384;
/// Characters at least between checkpoints.
const CHECKPOINT: usize = 4096;

/// Files up to this many bytes are measured again when their widest line
/// gets narrower; larger ones keep the old width until a wider line.
const REMEASURE_BYTES: usize = 4 << 20;

impl Doc {
    pub fn new(id: u64, text: &str, path: Option<PathBuf>) -> Self {
        let buffer = Buffer::new(text, path);
        let indent = indent::detect(buffer.rope());
        Self {
            id,
            buffer,
            cursor: 0,
            anchor: None,
            goal: None,
            scroll: 0,
            scroll_sub: Sub::default(),
            hscroll: 0,
            marks: Vec::new(),
            inlays: Vec::new(),
            review: None,
            git_base: None,
            git_signs: Vec::new(),
            git_marks: Vec::new(),
            extra: Vec::new(),
            extra_version: 0,
            folds: Vec::new(),
            layers: Layers::default(),
            page: None,
            indent,
            config: Default::default(),
            widest: Default::default(),
            checkpoints: Default::default(),
        }
    }

    /// A page for the window to draw, such as a plugin's.
    pub fn page(id: u64, name: &str, title: &str) -> Self {
        let mut doc = Self::new(id, "", None);
        doc.page = Some((name.to_string(), title.to_string()));
        doc
    }

    /// A read-only document showing the change from `old` to `new`.
    pub fn review(id: u64, review: (String, String, PathBuf, u64), old: &str, new: &str) -> Self {
        Self::review_in(id, review, old, new, 3)
    }

    /// A review with `context` unchanged lines around each change, and the
    /// rest left out.
    pub fn review_in(
        id: u64,
        review: (String, String, PathBuf, u64),
        old: &str,
        new: &str,
        context: usize,
    ) -> Self {
        let (plugin, change, path, back_to) = review;
        let mut lines = diff::unified(old, new, context);
        if lines.is_empty() {
            lines.push(diff::DiffLine {
                kind: LineKind::Gap,
                text: "No changes".into(),
                number: 0,
                old_number: 0,
            });
        }
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        let mut doc = Doc::new(id, &text.join("\n"), None);
        // Start at the first change.
        let first = lines
            .iter()
            .position(|l| matches!(l.kind, LineKind::Added | LineKind::Removed));
        doc.cursor = doc.line_start(first.unwrap_or(0));
        doc.review = Some(Review {
            plugin,
            id: change,
            path,
            lines: lines.iter().map(|l| (l.kind, l.number)).collect(),
            old_numbers: lines.iter().map(|l| l.old_number).collect(),
            back_to,
            words: Vec::new(),
            staged: Vec::new(),
        });
        doc
    }

    /// Sets the committed text to compare against, and updates the signs.
    pub fn set_git_base(&mut self, base: Option<String>) {
        self.git_base = base;
        self.update_signs();
    }

    fn update_signs(&mut self) {
        self.git_signs = match &self.git_base {
            Some(base) => diff::signs_rope(base, self.buffer.rope()),
            None => Vec::new(),
        };
        self.git_marks = self
            .git_signs
            .iter()
            .enumerate()
            .filter_map(|(line, sign)| sign.map(|s| (line, s)))
            .collect();
    }

    /// Brings marks and the syntax tree up to date with the text, and
    /// returns the changes, for other views of it.
    pub fn sync(&mut self) -> Vec<Change> {
        let changes = self.buffer.take_changes();
        if changes.is_empty() {
            return changes;
        }
        let carets_placed = self.extra_version == self.buffer.version();
        for change in &changes {
            for mark in &mut self.marks {
                mark.start = change.map(mark.start);
                mark.end = change.map(mark.end).max(mark.start);
            }
            for fold in &mut self.folds {
                *fold = change.map(*fold);
            }
            for inlay in &mut self.inlays {
                inlay.pos = change.map(inlay.pos);
            }
            self.layers.map(change);
            if !carets_placed {
                for caret in &mut self.extra {
                    caret.cursor = change.map(caret.cursor);
                    caret.anchor = caret.anchor.map(|a| change.map(a));
                }
            }
        }
        self.extra_version = self.buffer.version();
        self.update_widest(&changes);
        self.update_checkpoints(&changes);
        self.layers.tidy();
        // A fold whose first line was joined to another goes.
        self.folds
            .retain(|&f| f == 0 || self.buffer.rope().char(f - 1) == '\n');
        self.folds.sort_unstable();
        self.folds.dedup();
        if self.git_base.is_some() {
            self.update_signs();
        }
        changes
    }

    /// Syntax colours over `lines`, by character range, as plugins sent
    /// them: base colours, and semantic colours over them.
    pub fn spans(&self, lines: std::ops::Range<usize>) -> Vec<(usize, usize, Syntax)> {
        let count = self.last_line() + 1;
        let lines = lines.start.min(count)..lines.end.min(count);
        let at = |l: usize| {
            if l >= count {
                self.buffer.len()
            } else {
                self.line_start(l)
            }
        };
        self.layers.spans(&(at(lines.start)..at(lines.end)))
    }

    /// Whether anything colours this document.
    pub fn has_colours(&self) -> bool {
        self.layers.base.is_some()
    }

    /// The position of an LSP-style line and UTF-16 column, clamped to the
    /// text.
    pub fn pos_at_utf16(&self, line: u32, column: u32) -> usize {
        let line = (line as usize).min(self.last_line());
        let text: String = self.line_chars(line).into_iter().collect();
        self.line_start(line) + crate::lsp::utf16_to_char_col(&text, column as usize)
    }

    /// An empty file that was never named or edited, so opening a file can
    /// take its place.
    pub fn is_blank(&self) -> bool {
        self.buffer.path.is_none() && self.buffer.is_empty() && self.buffer.revision() == 0
    }

    pub fn name(&self) -> String {
        if let Some((_, title)) = &self.page {
            return title.clone();
        }
        if let Some(review) = &self.review {
            let name = review.path.file_name().map_or_else(
                || review.path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            // Changes since the last commit are looked at, not answered.
            return if review.plugin.is_empty() {
                format!("{name} (changes)")
            } else if review.plugin == crate::editor::COMPARE_REVIEW {
                let left = match review.id.as_str() {
                    "clipboard:" => "Clipboard".to_string(),
                    "saved:" => "Saved".to_string(),
                    other if other.starts_with("before:") => other["before:".len()..].to_string(),
                    other => std::path::Path::new(other)
                        .file_name()
                        .map_or(other.to_string(), |n| n.to_string_lossy().into_owned()),
                };
                format!("{name} ↔ {left}")
            } else if review.plugin == "merlin.commit" {
                format!("{name} @ {}", review.id)
            } else if review.plugin == crate::editor::RENAME_REVIEW {
                format!("{name} · Rename")
            } else {
                format!("Review {name}")
            };
        }
        match &self.buffer.path {
            Some(p) => p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            None => "[scratch]".into(),
        }
    }

    /// The selected range, if it isn't empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        self.buffer.rope().slice(start..end).to_string()
    }

    pub fn line_col(&self, pos: usize) -> (usize, usize) {
        let rope = self.buffer.rope();
        let line = rope.char_to_line(pos);
        (line, pos - rope.line_to_char(line))
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.buffer.rope().line_to_char(line)
    }

    /// A line's characters, without its line break.
    pub fn line_chars(&self, line: usize) -> Vec<char> {
        self.buffer
            .rope()
            .line(line)
            .chars()
            .take(self.buffer.line_len(line))
            .collect()
    }

    pub fn last_line(&self) -> usize {
        self.buffer.rope().len_lines() - 1
    }

    /// The widest line's columns, not counting inlay hints: measured once,
    /// then only the lines edits touch.
    pub fn widest(&self) -> usize {
        let version = self.buffer.version();
        match self.widest.get() {
            Some(w) if w.version == version => return w.cols,
            _ => {}
        }
        let (cols, line) = widest_in(self.buffer.rope());
        let start = self.line_start(line);
        // Edits still to sync would be counted twice.
        if !self.buffer.has_changes() {
            self.widest.set(Some(Widest {
                cols,
                span: Some((start, start + self.buffer.line_len(line))),
                version,
            }));
        }
        cols
    }

    /// Measures the lines `changes` touched, and drops the width if they
    /// made the widest line narrower in a file small enough to measure again.
    fn update_widest(&mut self, changes: &[Change]) {
        let Some(mut w) = self.widest.get() else {
            return;
        };
        // From another buffer.
        if w.version >> 32 != self.buffer.version() >> 32 {
            self.widest.set(None);
            return;
        }
        let mut lost = false;
        for change in changes {
            if let Some((a, b)) = w.span {
                if change.at <= b && change.at + change.removed >= a {
                    lost = true;
                    w.span = None;
                } else {
                    w.span = Some((change.map(a), change.map(b)));
                }
            }
        }
        // Where each change's text ended up, after the ones after it.
        let rope = self.buffer.rope();
        let mut best: Option<(usize, usize)> = None;
        for (i, change) in changes.iter().enumerate() {
            let (mut a, mut b) = (change.at, change.at + change.inserted);
            for later in &changes[i + 1..] {
                a = later.map(a);
                b = later.map(b).max(a);
            }
            let a = a.min(rope.len_chars());
            let b = b.min(rope.len_chars());
            for line in rope.char_to_line(a)..=rope.char_to_line(b) {
                let cols = self.text_cols(line);
                if best.is_none_or(|(c, _)| cols > c) {
                    best = Some((cols, line));
                }
            }
        }
        if let Some((cols, line)) = best
            && cols >= w.cols
        {
            let start = self.line_start(line);
            w.cols = cols;
            w.span = Some((start, start + self.buffer.line_len(line)));
            lost = false;
        }
        if lost && rope.len_bytes() <= REMEASURE_BYTES {
            self.widest.set(None);
            return;
        }
        w.version = self.buffer.version();
        self.widest.set(Some(w));
    }

    /// A line's columns, not counting inlay hints.
    fn text_cols(&self, line: usize) -> usize {
        widest_of(self.buffer.rope().line(line).chunks()).0
    }

    /// Where `col` on `line` is drawn, counting tabs, wide characters and
    /// the inlay hints before it. A hint at `col` itself comes after: the
    /// cursor there stays by the text before.
    pub fn display_col(&self, line: usize, col: usize) -> usize {
        let inlays = self.line_inlays(line);
        if inlays.is_empty() {
            let col = col.min(self.buffer.line_len(line));
            let from = self.checkpoint(line, |(i, _)| i <= col);
            let text = self.buffer.rope().line(line).slice(from.0..col);
            return walk(text.chunks(), from, |_, _| true).1;
        }
        let mut d = 0;
        let chars = self.buffer.rope().line(line).chars();
        for (i, c) in chars.take(col.min(self.buffer.line_len(line))).enumerate() {
            d += inlay_cols(&inlays, i);
            d += advance(c, d);
        }
        d
    }

    /// The character column drawn at display column `target`. A click on
    /// an inlay hint lands before the character it's before.
    pub fn col_at_display(&self, line: usize, target: usize) -> usize {
        let inlays = self.line_inlays(line);
        let len = self.buffer.line_len(line);
        if inlays.is_empty() {
            // The character that covers `target`: the last to start at or
            // before it, unless the line ends first.
            let from = self.checkpoint(line, |(_, d)| d <= target);
            let mut last = from.0;
            let text = self.buffer.rope().line(line).slice(from.0..len);
            let (i, d) = walk(text.chunks(), from, |i, d| {
                if d > target {
                    return false;
                }
                last = i;
                true
            });
            return if i == len && d <= target { len } else { last };
        }
        let mut d = 0;
        for (i, c) in self.buffer.rope().line(line).chars().take(len).enumerate() {
            d += inlay_cols(&inlays, i);
            if d > target {
                return i;
            }
            let w = advance(c, d);
            if d + w > target {
                return i;
            }
            d += w;
        }
        len
    }

    /// The last of a long line's checkpoints that `before` takes, as a
    /// character and its display column; the line's start for a short one.
    /// A line with inlay hints has none: they move its columns.
    fn checkpoint(&self, line: usize, before: impl Fn((usize, usize)) -> bool) -> (usize, usize) {
        let len = self.buffer.line_len(line);
        // Edits not yet synced would move them wrongly: from the start.
        if len < CHECKPOINT_LINE || self.buffer.has_changes() {
            return (0, 0);
        }
        let version = self.buffer.version();
        let mut cache = self.checkpoints.borrow_mut();
        if !cache
            .as_ref()
            .is_some_and(|c| c.version == version && c.line == line)
        {
            *cache = Some(Checkpoints {
                version,
                line,
                points: vec![(0, 0, 0)],
                whole: false,
            });
        }
        let cp = cache.as_mut().expect("just set");
        // Worked out further, as far as `before` takes them: at the first
        // character past every CHECKPOINT-th whose column is on a tab stop,
        // so drawing from one lines tabs up.
        let &(i0, _, d0) = cp.points.last().expect("the line's start");
        if !cp.whole && before((i0, d0)) {
            let text = self.buffer.rope().line(line);
            let mut last = i0;
            let mut stopped = false;
            walk(text.slice(i0..len).chunks(), (i0, d0), |i, d| {
                if i >= last + CHECKPOINT && d.is_multiple_of(TAB_WIDTH) {
                    cp.points.push((i, text.char_to_byte(i), d));
                    last = i;
                    if !before((i, d)) {
                        stopped = true;
                        return false;
                    }
                }
                true
            });
            cp.whole = !stopped;
        }
        let k = cp.points.partition_point(|&(i, _, d)| before((i, d)));
        let (i, _, d) = cp.points[k.saturating_sub(1)];
        (i, d)
    }

    /// Keeps a long line's checkpoints through `changes`: those before an
    /// edit in the line stay, and an edit above it moves the line.
    fn update_checkpoints(&mut self, changes: &[Change]) {
        let cache = self.checkpoints.get_mut();
        let Some(cp) = cache.as_mut() else {
            return;
        };
        for change in changes {
            let e = &change.edit;
            if e.start.0 > cp.line {
                continue;
            }
            if e.old_end.0 < cp.line {
                cp.line = cp.line - e.old_end.0 + e.new_end.0;
                continue;
            }
            if e.start.0 == cp.line && e.old_end.0 == cp.line && e.new_end.0 == cp.line {
                // Columns before the edit are as they were.
                cp.points.retain(|&(_, byte, _)| byte <= e.start.1);
                cp.whole = false;
                continue;
            }
            *cache = None;
            return;
        }
        cp.version = self.buffer.version();
    }

    /// Where to start drawing a long `line` to show display column
    /// `target`: the last character at or before it on a tab stop, so tabs
    /// after it line up, and its display column.
    pub fn draw_from(&self, line: usize, target: usize) -> (usize, usize) {
        let inlays = self.line_inlays(line);
        let len = self.buffer.line_len(line);
        let mut best = (0, 0);
        if inlays.is_empty() {
            let from = self.checkpoint(line, |(_, d)| d <= target);
            best = from;
            let text = self.buffer.rope().line(line).slice(from.0..len);
            walk(text.chunks(), from, |i, d| {
                if d.is_multiple_of(TAB_WIDTH) && d <= target {
                    best = (i, d);
                }
                d <= target
            });
            return best;
        }
        let mut d = 0;
        for (i, c) in self.buffer.rope().line(line).chars().take(len).enumerate() {
            if d > target {
                break;
            }
            if d.is_multiple_of(TAB_WIDTH) {
                best = (i, d);
            }
            d += inlay_cols(&inlays, i);
            d += advance(c, d);
        }
        best
    }

    /// The word of characters `is_word` takes that `pos` is in or just
    /// after, as a char range: found from `pos` out, whatever the line's
    /// length.
    pub fn word_around(
        &self,
        pos: usize,
        is_word: impl Fn(char) -> bool,
    ) -> Option<(usize, usize)> {
        let rope = self.buffer.rope();
        let at = |i: usize| i < rope.len_chars() && is_word(rope.char(i));
        let mut a = pos.min(rope.len_chars());
        if !at(a) && a > 0 && at(a - 1) {
            a -= 1;
        }
        if !at(a) {
            return None;
        }
        let mut before = rope.chars_at(a);
        while before.prev().is_some_and(&is_word) {
            a -= 1;
        }
        let b = a + rope.chars_at(a).take_while(|&c| is_word(c)).count();
        Some((a, b))
    }

    /// The inlay hints on a line, as the column of the character each is
    /// before and how many columns it takes.
    pub fn line_inlays(&self, line: usize) -> Vec<(usize, usize)> {
        if self.inlays.is_empty() {
            return Vec::new();
        }
        let start = self.line_start(line);
        let end = start + self.buffer.line_len(line);
        let first = self.inlays.partition_point(|i| i.pos < start);
        self.inlays[first..]
            .iter()
            .take_while(|i| i.pos <= end)
            .map(|i| (i.pos - start, i.cols))
            .collect()
    }

    /// The position of `line`, display column `target`, clamped to the text.
    pub fn pos_at(&self, line: usize, target: usize) -> usize {
        let line = line.min(self.last_line());
        self.line_start(line) + self.col_at_display(line, target)
    }

    /// The start of the word `pos` is in or just after.
    pub fn word_start_at(&self, pos: usize) -> usize {
        let rope = self.buffer.rope();
        let mut i = pos;
        while i > 0 && class_of(rope.char(i - 1)) == Class::Word {
            i -= 1;
        }
        i
    }

    /// The end of the word `pos` is in or just before.
    pub fn word_end_at(&self, pos: usize) -> usize {
        let rope = self.buffer.rope();
        let len = rope.len_chars();
        let mut i = pos;
        while i < len && class_of(rope.char(i)) == Class::Word {
            i += 1;
        }
        i
    }

    /// The start of the word before `pos`, skipping whitespace first.
    pub fn word_left(&self, pos: usize) -> usize {
        let rope = self.buffer.rope();
        let mut i = pos;
        while i > 0 && rope.char(i - 1).is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let class = class_of(rope.char(i - 1));
            while i > 0 && class_of(rope.char(i - 1)) == class {
                i -= 1;
            }
        }
        i
    }

    /// The end of the word after `pos`, skipping whitespace first.
    pub fn word_right(&self, pos: usize) -> usize {
        let rope = self.buffer.rope();
        let len = rope.len_chars();
        let mut i = pos;
        while i < len && rope.char(i).is_whitespace() {
            i += 1;
        }
        if i < len {
            let class = class_of(rope.char(i));
            while i < len && class_of(rope.char(i)) == class {
                i += 1;
            }
        }
        i
    }
}

/// Cells `c` takes when drawn at display column `at`.
/// The columns the inlay hints before character `i` take.
/// The widest line in `rope`, in columns without inlay hints, and which
/// line it is: one pass over the text, ASCII a byte at a time.
fn widest_in(rope: &ropey::Rope) -> (usize, usize) {
    widest_of(rope.chunks())
}

/// [`widest_in`] for text in pieces.
fn widest_of<'a>(chunks: impl Iterator<Item = &'a str>) -> (usize, usize) {
    let (mut best, mut best_line) = (0, 0);
    let (mut col, mut line, mut cr) = (0, 0, false);
    for chunk in chunks {
        let bytes = chunk.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\n' {
                let cols = col - usize::from(cr);
                if cols > best {
                    (best, best_line) = (cols, line);
                }
                (col, cr) = (0, false);
                line += 1;
                i += 1;
                continue;
            }
            cr = b == b'\r';
            if b < 0x80 {
                col += if b == b'\t' {
                    TAB_WIDTH - col % TAB_WIDTH
                } else {
                    char_width(b as char)
                };
                i += 1;
            } else {
                let c = chunk[i..].chars().next().unwrap_or(' ');
                col += advance(c, col);
                i += c.len_utf8();
            }
        }
    }
    if col > best {
        (best, best_line) = (col, line);
    }
    (best, best_line)
}

/// Goes through the characters of `chunks`, from character `i` at display
/// column `d`, calling `f` with each one's index and column until it says
/// to stop; then the index and column reached. ASCII a byte at a time, for
/// long lines.
fn walk<'a>(
    chunks: impl Iterator<Item = &'a str>,
    (mut i, mut d): (usize, usize),
    mut f: impl FnMut(usize, usize) -> bool,
) -> (usize, usize) {
    for chunk in chunks {
        let bytes = chunk.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if !f(i, d) {
                return (i, d);
            }
            let b = bytes[at];
            if b < 0x80 {
                d += if b == b'\t' {
                    TAB_WIDTH - d % TAB_WIDTH
                } else {
                    char_width(b as char)
                };
                at += 1;
            } else {
                let c = chunk[at..].chars().next().unwrap_or(' ');
                d += advance(c, d);
                at += c.len_utf8();
            }
            i += 1;
        }
    }
    (i, d)
}

pub fn inlay_cols(inlays: &[(usize, usize)], i: usize) -> usize {
    inlays
        .iter()
        .filter(|(at, _)| *at == i)
        .map(|(_, n)| n)
        .sum()
}

pub fn advance(c: char, at: usize) -> usize {
    if c == '\t' {
        TAB_WIDTH - at % TAB_WIDTH
    } else {
        char_width(c)
    }
}

#[derive(PartialEq, Eq)]
enum Class {
    Word,
    Punct,
    Space,
}

fn class_of(c: char) -> Class {
    if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else if c.is_whitespace() {
        Class::Space
    } else {
        Class::Punct
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::Edit;
    use crate::history::Kind;

    /// Columns walked from the line's start, one character at a time.
    fn plain_cols(chars: &[char]) -> Vec<usize> {
        let mut cols = vec![0];
        let mut d = 0;
        for &c in chars {
            d += advance(c, d);
            cols.push(d);
        }
        cols
    }

    fn check(doc: &Doc, line: usize) {
        let chars: Vec<char> = doc.line_chars(line);
        let cols = plain_cols(&chars);
        for col in (0..=chars.len()).step_by(61) {
            assert_eq!(doc.display_col(line, col), cols[col], "display_col({col})");
        }
        let last = *cols.last().unwrap();
        for target in (0..=last + 3).step_by(43) {
            // The character covering `target`, or the line's end.
            let want = if target >= last {
                chars.len()
            } else {
                cols[..chars.len()].partition_point(|&c| c <= target) - 1
            };
            assert_eq!(
                doc.col_at_display(line, target),
                want,
                "col_at_display({target})"
            );
            // The last tab stop at or before `target`.
            let (i, d) = doc.draw_from(line, target);
            assert_eq!(cols[i], d, "draw_from({target}) gives a true column");
            assert!(
                d <= target && d.is_multiple_of(TAB_WIDTH),
                "draw_from({target}) = {d}"
            );
            assert!(
                target - d < 4 * TAB_WIDTH + 8,
                "draw_from({target}) is near: {d}"
            );
        }
    }

    /// A long line's checkpoints give the columns walking from its start
    /// gives, with tabs and wide characters along it, and after an edit.
    #[test]
    fn checkpoints_give_the_columns_of_a_walk_from_the_start() {
        let mut line = String::new();
        for i in 0..40_000 {
            line.push(match i % 97 {
                0 => '\t',
                13 => '漢',
                50 => 'é',
                _ => char::from(b'a' + (i % 26) as u8),
            });
        }
        let mut doc = Doc::new(1, &line, None);
        assert!(doc.buffer.line_len(0) >= CHECKPOINT_LINE);
        check(&doc, 0);
        let edit = |doc: &mut Doc, at: usize, removed: &str, inserted: &str| {
            doc.buffer.begin(at, Kind::Other);
            doc.buffer.apply(Edit {
                at,
                removed: removed.into(),
                inserted: inserted.into(),
            });
            doc.buffer.commit(at);
            doc.sync();
        };
        // A tab moves every column after it: in the middle, keeping the
        // checkpoints before it, and early on.
        edit(&mut doc, 25_000, "", "\t漢");
        check(&doc, 0);
        edit(&mut doc, 10, "", "\tx");
        check(&doc, 0);
        // A line above moves the long line down.
        edit(&mut doc, 0, "", "top\n");
        check(&doc, 1);
        // Taking some of the line away.
        let at = doc.line_start(1) + 30_000;
        let gone: String = doc.buffer.rope().slice(at..at + 3).to_string();
        edit(&mut doc, at, &gone, "");
        check(&doc, 1);
    }
}
