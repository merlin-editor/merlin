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
}

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
            extra: Vec::new(),
            extra_version: 0,
            folds: Vec::new(),
            layers: Layers::default(),
            page: None,
            indent,
            config: Default::default(),
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
            Some(base) => diff::signs(base, &self.buffer.text()),
            None => Vec::new(),
        };
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

    /// Where `col` on `line` is drawn, counting tabs, wide characters and
    /// the inlay hints before it. A hint at `col` itself comes after: the
    /// cursor there stays by the text before.
    pub fn display_col(&self, line: usize, col: usize) -> usize {
        let inlays = self.line_inlays(line);
        let mut d = 0;
        for (i, c) in self.line_chars(line).into_iter().take(col).enumerate() {
            d += inlay_cols(&inlays, i);
            d += advance(c, d);
        }
        d
    }

    /// The character column drawn at display column `target`. A click on
    /// an inlay hint lands before the character it's before.
    pub fn col_at_display(&self, line: usize, target: usize) -> usize {
        let inlays = self.line_inlays(line);
        let mut d = 0;
        let chars = self.line_chars(line);
        for (i, c) in chars.iter().enumerate() {
            d += inlay_cols(&inlays, i);
            if d > target {
                return i;
            }
            let w = advance(*c, d);
            if d + w > target {
                return i;
            }
            d += w;
        }
        chars.len()
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
