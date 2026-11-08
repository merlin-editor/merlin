//! Find references (Shift+F12): a peek view that opens between the lines,
//! under the name, as VS Code's does. The chosen reference's lines are on
//! the left and every reference is on the right, by file. Enter opens the
//! chosen one, F4 and Shift+F4 step through them, Esc closes it.

use std::path::PathBuf;

use merlin_syntax::Syntax;

use super::views::SearchHit;
use super::{Editor, Effect, Focus};
use crate::key::{Key, KeyCode};
use crate::lang::Location;

/// How many rows the peek view opens between the lines, at most.
const PEEK_ROWS: usize = 15;

/// Lines of a reference's file shown above it in the preview.
const ABOVE: usize = 3;

pub(crate) struct Peek {
    pub(crate) doc: u64,
    pub(crate) group: usize,
    /// The line it opens under, and the name it's for.
    pub(crate) line: usize,
    pub(crate) name: String,
    pub(crate) rows: usize,
    pub(crate) files: Vec<PeekFile>,
    /// The chosen row of the list, and the first one shown.
    pub(crate) row: usize,
    pub(crate) top: usize,
    pub(crate) reading: usize,
}

pub(crate) struct PeekFile {
    pub(crate) path: PathBuf,
    /// Its text, once read.
    pub(crate) text: Option<String>,
    pub(crate) places: Vec<((u32, u32), (u32, u32))>,
    pub(crate) open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    File(usize),
    Place(usize, usize),
}

impl Peek {
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, file) in self.files.iter().enumerate() {
            rows.push(Row::File(i));
            if file.open {
                rows.extend((0..file.places.len()).map(|j| Row::Place(i, j)));
            }
        }
        rows
    }

    fn chosen(&self) -> Option<Row> {
        self.rows().get(self.row).copied()
    }

    fn places(&self) -> usize {
        self.files.iter().map(|f| f.places.len()).sum()
    }

    /// The rows the list has room for, under its header.
    fn list_rows(&self) -> usize {
        self.rows.saturating_sub(2).max(1)
    }

    /// Scrolls the list so the chosen row shows.
    fn reveal(&mut self) {
        let room = self.list_rows();
        if self.row < self.top {
            self.top = self.row;
        } else if self.row >= self.top + room {
            self.top = self.row + 1 - room;
        }
    }
}

/// The peek view, for the window to draw in the rows it opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeekView {
    /// The screen row its first row is on, as a line's `row`, and how many
    /// it takes.
    pub row: i32,
    pub rows: u16,
    pub name: String,
    pub summary: String,
    /// The list's rows from the first shown, and which is chosen.
    pub items: Vec<PeekItem>,
    pub selected: Option<usize>,
    /// The chosen reference's lines, with its file's references marked.
    pub preview: Vec<PreviewLine>,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeekItem {
    File {
        name: String,
        /// Its file type's icon and colour, as the Explorer has them.
        icon: (char, crate::style::Hue),
        folder: String,
        count: usize,
        open: bool,
    },
    /// A reference: its line number, the text around it, and where it is
    /// in that text, by character.
    Place {
        line: usize,
        text: String,
        mark: (usize, usize),
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewLine {
    /// From 1.
    pub number: usize,
    pub text: String,
    /// Colours by character, as the text's own are.
    pub colours: Vec<(usize, usize, Syntax)>,
    /// The references on the line, by character.
    pub marks: Vec<(usize, usize)>,
    /// Whether it's the chosen reference's line, to scroll to.
    pub current: bool,
}

/// Characters of a list row's text kept before its reference, when the
/// reference is further along than that.
const LEAD: usize = 12;

/// UTF-16 `column` of `line` as a character offset in it.
fn char_column(line: &str, column: u32) -> usize {
    let mut units = 0;
    for (i, c) in line.chars().enumerate() {
        if units >= column as usize {
            return i;
        }
        units += c.len_utf16();
    }
    line.chars().count()
}

impl Editor {
    /// Shift+F12: asks where the name at the cursor is used.
    pub(crate) fn find_references(&mut self, fx: &mut Vec<Effect>) {
        let Some(plugin) = self.lsp_plugin("references") else {
            return;
        };
        let doc = self.doc();
        let at = doc.cursor;
        let name = doc.slice(doc.word_start_at(at), doc.word_end_at(at));
        if name.is_empty() {
            return self.notify("Put the cursor on a name to find its references".into());
        }
        let asked = (doc.id, doc.line_col(at).0, name);
        self.peek = None;
        self.peek_asked = Some(asked);
        fx.push(Effect::PluginCommand {
            plugin,
            command: "references".into(),
        });
    }

    /// The server's answer: the peek opens once the files are read.
    pub(crate) fn show_references(&mut self, places: Vec<Location>, fx: &mut Vec<Effect>) {
        let Some((doc, line, name)) = self.peek_asked.take() else {
            return;
        };
        if self.doc().id != doc || self.focus != Focus::Editor {
            return;
        }
        if places.is_empty() {
            return self.notify(format!("No references to {name}"));
        }
        let mut files: Vec<PeekFile> = Vec::new();
        for place in places {
            let range = (place.start, place.end);
            match files.iter_mut().find(|f| f.path == place.path) {
                Some(file) => file.places.push(range),
                None => files.push(PeekFile {
                    path: place.path,
                    text: None,
                    places: vec![range],
                    open: true,
                }),
            }
        }
        let mut unread = Vec::new();
        for file in &mut files {
            file.places.sort();
            file.places.dedup();
            match self.doc_at(&file.path) {
                Some(d) => file.text = Some(d.buffer.text()),
                None => unread.push(file.path.clone()),
            }
        }
        let rows = PEEK_ROWS.min(self.text_height().saturating_sub(4)).max(6);
        self.popup = None;
        self.peek = Some(Peek {
            doc,
            group: self.group,
            line,
            name,
            rows,
            files,
            row: 1,
            top: 0,
            reading: unread.len(),
        });
        if !unread.is_empty() {
            fx.push(Effect::ReadFiles(unread));
        }
        self.reveal_peek();
    }

    /// Files read for the peek view.
    pub(crate) fn peek_files_read(&mut self, files: &[(PathBuf, Option<String>)]) {
        let Some(peek) = &mut self.peek else {
            return;
        };
        for file in &mut peek.files {
            if file.text.is_none()
                && let Some((_, text)) = files.iter().find(|(p, _)| *p == file.path)
            {
                file.text = Some(text.clone().unwrap_or_default());
                peek.reading = peek.reading.saturating_sub(1);
            }
        }
    }

    /// Scrolls so the line and the peek under it show.
    fn reveal_peek(&mut self) {
        let Some(peek) = &self.peek else {
            return;
        };
        let (line, rows) = (peek.line, peek.rows);
        let height = self.text_height();
        let fits = self
            .screen_row(line, 0)
            .is_some_and(|row| row + 1 + rows <= height);
        if !fits {
            self.set_top(line.saturating_sub(2), 0);
        }
    }

    /// The rows the peek view opens after `line` of the document drawn.
    pub(crate) fn peek_gap(&self, line: usize) -> usize {
        self.references_gap(line)
            + self.change_gap(line)
            + self.merge_gap(line)
            + self.test_gap(line)
    }

    /// Rows the references peek opens after `line`.
    pub(crate) fn references_gap(&self, line: usize) -> usize {
        match &self.peek {
            Some(p) if p.line == line && p.doc == self.doc().id && p.group == self.pane() => p.rows,
            _ => 0,
        }
    }

    /// Whether the document drawn has a peek open in it: the references, or
    /// a change.
    pub(crate) fn peeking(&self) -> bool {
        self.change_peeking()
            || self
                .peek
                .as_ref()
                .is_some_and(|p| p.doc == self.doc().id && p.group == self.pane())
    }

    /// The peek view, drawn after `row`, the screen row of its line's last
    /// row.
    pub(crate) fn peek_view(&self, row: i32) -> Option<PeekView> {
        let peek = self.peek.as_ref()?;
        let files = peek.files.len();
        let places = peek.places();
        let summary = format!(
            "{places} reference{} in {files} file{}",
            if places == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        );
        // The line's text from a little before the reference, so it shows.
        let line_of = |file: &PeekFile, j: usize| {
            let (start, end) = file.places[j];
            let text = file.text.as_deref().unwrap_or_default();
            let line = text.lines().nth(start.0 as usize).unwrap_or_default();
            let chars: Vec<char> = line.chars().collect();
            let a = char_column(line, start.1);
            let b = if end.0 == start.0 {
                char_column(line, end.1).max(a)
            } else {
                chars.len()
            };
            let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
            let from = if a > indent + LEAD * 2 {
                a - LEAD
            } else {
                indent
            };
            let lead = if from > indent { "…" } else { "" };
            let shown: String = chars[from.min(a)..].iter().collect();
            let n = lead.chars().count();
            let mark = (a - from.min(a) + n, b - from.min(a) + n);
            (
                start.0 as usize + 1,
                format!("{lead}{}", shown.trim_end()),
                mark,
            )
        };
        let items = peek
            .rows()
            .into_iter()
            .skip(peek.top)
            .take(peek.list_rows())
            .map(|row| match row {
                Row::File(i) => {
                    let file = &peek.files[i];
                    let name = file
                        .path
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    PeekItem::File {
                        icon: crate::icons::file_icon(&name, self.nerd),
                        name,
                        folder: file
                            .path
                            .parent()
                            .map_or(String::new(), |p| p.display().to_string()),
                        count: file.places.len(),
                        open: file.open,
                    }
                }
                Row::Place(i, j) => {
                    let (line, text, mark) = line_of(&peek.files[i], j);
                    PeekItem::Place { line, text, mark }
                }
            })
            .collect();
        let selected = peek.row.checked_sub(peek.top);
        Some(PeekView {
            row,
            rows: peek.rows as u16,
            name: peek.name.clone(),
            summary,
            items,
            selected,
            preview: self.peek_preview(peek),
            focused: self.focus == Focus::Editor,
        })
    }

    /// The chosen reference's file around it, coloured, its references
    /// marked.
    fn peek_preview(&self, peek: &Peek) -> Vec<PreviewLine> {
        let (i, j) = match peek.chosen() {
            Some(Row::Place(i, j)) => (i, j),
            Some(Row::File(i)) => (i, 0),
            None => return Vec::new(),
        };
        let file = &peek.files[i];
        let Some(text) = &file.text else {
            return Vec::new();
        };
        let at = file.places.get(j).map_or(0, |p| p.0.0 as usize);
        let first = at.saturating_sub(ABOVE);
        let shown = peek.rows.saturating_sub(2).max(1);
        let lines: Vec<&str> = text.lines().skip(first).take(shown).collect();
        // Colours: an open file's own, else the snippet's from its
        // language's plugin.
        let open = self
            .docs
            .iter()
            .find(|d| d.review.is_none() && d.buffer.path.as_deref() == Some(&file.path));
        let snippet = lines.join("\n");
        let spans = match open {
            Some(doc) => {
                let start = doc.line_start(first);
                doc.spans(first..first + lines.len())
                    .into_iter()
                    .map(|(a, b, k)| (a.saturating_sub(start), b.saturating_sub(start), k))
                    .collect()
            }
            None => {
                let ext = file
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_default();
                self.colours.get(&ext, &snippet).unwrap_or_default()
            }
        };
        let mut offset = 0;
        lines
            .iter()
            .enumerate()
            .map(|(k, line)| {
                let len = line.chars().count();
                let colours = spans
                    .iter()
                    .filter(|&&(a, b, _)| b > offset && a < offset + len)
                    .map(|&(a, b, kind)| (a.saturating_sub(offset), (b - offset).min(len), kind))
                    .collect();
                offset += len + 1;
                let number = first + k;
                let marks = file
                    .places
                    .iter()
                    .filter(|(s, _)| s.0 as usize == number)
                    .map(|(s, e)| {
                        let a = char_column(line, s.1);
                        let b = if e.0 == s.0 {
                            char_column(line, e.1)
                        } else {
                            len
                        };
                        (a, b.max(a))
                    })
                    .collect();
                PreviewLine {
                    number: number + 1,
                    text: line.to_string(),
                    colours,
                    marks,
                    current: number == at,
                }
            })
            .collect()
    }

    /// Keys while the peek is open: ↑ ↓ move, Enter opens, F4 and Shift+F4
    /// step through the references, Esc closes. Anything else closes it
    /// and goes on as usual. Returns whether it used the key.
    pub(crate) fn peek_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if !self.peeking() {
            return false;
        }
        let plain = !key.ctrl && !key.alt;
        match key.code {
            KeyCode::Esc => self.peek = None,
            KeyCode::Up if plain => self.move_peek(-1),
            KeyCode::Down if plain => self.move_peek(1),
            KeyCode::Left | KeyCode::Right if plain => self.fold_peek(key.code == KeyCode::Right),
            KeyCode::Enter if plain => self.open_peeked(fx),
            KeyCode::F(4) if plain => self.step_peek(!key.shift),
            _ => {
                self.peek = None;
                return false;
            }
        }
        true
    }

    fn move_peek(&mut self, by: isize) {
        let Some(peek) = &mut self.peek else {
            return;
        };
        let last = peek.rows().len().saturating_sub(1);
        peek.row = peek.row.saturating_add_signed(by).min(last);
        peek.reveal();
    }

    /// F4: the next reference, through every file's, round to the first.
    fn step_peek(&mut self, forward: bool) {
        let Some(peek) = &mut self.peek else {
            return;
        };
        for file in &mut peek.files {
            file.open = true;
        }
        let rows = peek.rows();
        let n = rows.len();
        let step = |r: usize| {
            if forward {
                (r + 1) % n
            } else {
                (r + n - 1) % n
            }
        };
        let mut row = step(peek.row);
        while matches!(rows[row], Row::File(_)) && row != peek.row {
            row = step(row);
        }
        peek.row = row;
        peek.reveal();
    }

    /// Left and Right: hides or shows a file's references.
    fn fold_peek(&mut self, open: bool) {
        let Some(peek) = &mut self.peek else {
            return;
        };
        let i = match peek.chosen() {
            Some(Row::File(i)) => i,
            Some(Row::Place(i, _)) if !open => i,
            _ => return,
        };
        peek.files[i].open = open;
        peek.row = peek
            .rows()
            .iter()
            .position(|r| *r == Row::File(i))
            .unwrap_or(0);
        peek.reveal();
    }

    /// Enter: opens the chosen reference; on a file, folds it.
    fn open_peeked(&mut self, fx: &mut Vec<Effect>) {
        let Some(peek) = &self.peek else {
            return;
        };
        match peek.chosen() {
            Some(Row::Place(i, j)) => {
                let path = peek.files[i].path.clone();
                let (start, _) = peek.files[i].places[j];
                self.peek = None;
                self.open_at(path, start.0, start.1, fx);
            }
            Some(Row::File(i)) => {
                let open = peek.files[i].open;
                self.fold_peek(!open);
            }
            None => {}
        }
    }

    /// A click on the list's `i`th row shown: chooses it; a second click,
    /// or a double one, opens it.
    pub(crate) fn peek_clicked(&mut self, i: usize, clicks: u8, fx: &mut Vec<Effect>) {
        let Some(peek) = &mut self.peek else {
            return;
        };
        let row = peek.top + i;
        let again = row == peek.row;
        peek.row = row;
        self.focus = Focus::Editor;
        if clicks > 1 || (again && matches!(peek.chosen(), Some(Row::File(_)))) {
            self.open_peeked(fx);
        }
    }

    /// The list scrolled by the wheel.
    pub(crate) fn scroll_peek(&mut self, rows: isize) {
        if let Some(peek) = &mut self.peek {
            let most = peek.rows().len().saturating_sub(peek.list_rows());
            peek.top = peek.top.saturating_add_signed(rows).min(most);
        }
    }

    /// Every reference in the Search view, to keep while the peek closes.
    pub(crate) fn peek_to_search(&mut self, fx: &mut Vec<Effect>) {
        let Some(peek) = self.peek.take() else {
            return;
        };
        let mut hits = Vec::new();
        for file in &peek.files {
            let text = file.text.as_deref().unwrap_or_default();
            for (start, _) in &file.places {
                hits.push(SearchHit {
                    path: file.path.display().to_string(),
                    line: start.0,
                    column: start.1,
                    text: text
                        .lines()
                        .nth(start.0 as usize)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
        self.search.query = peek.name;
        self.search.hits = hits;
        self.search.searching = false;
        self.inputs.insert(
            (super::views::SEARCH.to_string(), "query".to_string()),
            self.search.query.clone(),
        );
        self.show_view(super::views::SEARCH, fx);
    }
}
