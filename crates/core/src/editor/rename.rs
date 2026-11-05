//! Rename (F2): a box over the name at the cursor says how far the name
//! reaches. Enter renames it everywhere; Shift+Enter reviews it first, in
//! the bottom panel's Rename tab, a place a row, with the chosen file's
//! diff in a review tab. Places can be left out before it's applied.

use std::collections::HashMap;
use std::path::PathBuf;

use super::{Editor, Effect, Focus, Side};
use crate::doc::Doc;
use crate::icons::{Icon, file_icon, icon};
use crate::key::{Key, KeyCode};
use crate::lang::{FileEdit, RenameInfo, TextEdit};
use crate::panel::{Button, Check, Entry, Node, Swap, Tone};

/// The Rename tab of the bottom panel, and its review tabs' plugin.
pub(crate) const RENAME: &str = "merlin.rename";
pub(crate) const APPLY: &str = "rename:apply";
pub(crate) const DISCARD: &str = "rename:discard";

/// The box over the name being renamed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenameBox {
    pub(crate) doc: u64,
    /// The name's place in the text, and the name.
    pub(crate) span: (usize, usize),
    pub(crate) old: String,
    pub(crate) text: String,
    /// The caret, in characters; with all of it selected, typing replaces
    /// it.
    pub(crate) caret: usize,
    pub(crate) all: bool,
    /// How many places and files use the name, once the server says.
    pub(crate) reach: Option<(u32, u32)>,
}

/// The rename box, for the window to draw over the name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameView {
    pub text: String,
    pub caret: usize,
    /// Whether all of it is selected.
    pub selected: bool,
    pub note: String,
    /// The old name's length in characters, to outline it in the text.
    pub name: usize,
}

/// A rename being reviewed.
#[derive(Default)]
pub(crate) struct Review {
    pub(crate) old: String,
    pub(crate) new: String,
    pub(crate) files: Vec<ReviewFile>,
    /// The chosen row, as [`Review::rows`] lists them.
    pub(crate) row: usize,
    /// Files still being read.
    pub(crate) reading: usize,
    /// What the bottom panel showed before, to go back to.
    pub(crate) was: (bool, Option<String>),
}

pub(crate) struct ReviewFile {
    pub(crate) path: PathBuf,
    /// Its text as the edits are for, once read.
    pub(crate) text: Option<String>,
    /// The open document's revision, so edits aren't made to changed text.
    pub(crate) revision: Option<u64>,
    pub(crate) places: Vec<(TextEdit, bool)>,
    pub(crate) open: bool,
}

/// A row of the Rename tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Row {
    File(usize),
    Place(usize, usize),
}

impl Review {
    pub(crate) fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (i, file) in self.files.iter().enumerate() {
            rows.push(Row::File(i));
            if file.open {
                rows.extend((0..file.places.len()).map(|j| Row::Place(i, j)));
            }
        }
        rows
    }

    fn row(&self) -> Option<Row> {
        self.rows().get(self.row).copied()
    }

    fn ticked(&self) -> usize {
        self.files
            .iter()
            .map(|f| f.places.iter().filter(|p| p.1).count())
            .sum()
    }

    fn places(&self) -> usize {
        self.files.iter().map(|f| f.places.len()).sum()
    }
}

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

/// Where each line of `text` starts, in characters.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, c) in text.chars().enumerate() {
        if c == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// A UTF-16 line and column of `text` as a character offset.
fn offset(text: &str, starts: &[usize], (line, column): (u32, u32)) -> usize {
    let Some(&start) = starts.get(line as usize) else {
        return text.chars().count();
    };
    let line = text.chars().skip(start).take_while(|&c| c != '\n');
    start + char_column(&line.collect::<String>(), column)
}

/// Where an edit was in the old text and is in the new, in characters.
type Moved = ((usize, usize), (usize, usize));

/// `text` with `edits` made, and where each edit was and is now, in
/// characters.
fn apply(text: &str, edits: &[&TextEdit]) -> (String, Vec<Moved>) {
    let starts = line_starts(text);
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|(a, b, new)| {
            let a = offset(text, &starts, *a);
            (a, offset(text, &starts, *b).max(a), new.as_str())
        })
        .collect();
    spans.sort_by_key(|s| s.0);
    let (mut out, mut at, mut moved) = (String::new(), 0, Vec::new());
    for (a, b, new) in spans {
        if a < at {
            continue;
        }
        out.extend(&chars[at..a]);
        let now = out.chars().count();
        out.push_str(new);
        moved.push(((a, b), (now, now + new.chars().count())));
        at = b;
    }
    out.extend(&chars[at.min(chars.len())..]);
    (out, moved)
}

/// A character offset in `text` as its line and column.
fn line_col(starts: &[usize], at: usize) -> (usize, usize) {
    let line = starts.partition_point(|&s| s <= at).saturating_sub(1);
    (line, at - starts[line])
}

impl Editor {
    /// F2: a box over the name at the cursor, with all of it selected.
    pub(crate) fn rename(&mut self, fx: &mut Vec<Effect>) {
        let doc = self.doc();
        if doc.review.is_some() || doc.page.is_some() {
            return;
        }
        if !self.has_lsp("rename") {
            return self.notify("No language server renames in this file".into());
        }
        let at = doc.cursor;
        let (start, end) = (doc.word_start_at(at), doc.word_end_at(at));
        let old = doc.slice(start, end);
        if old.is_empty() {
            return self.notify("Put the cursor on a name to rename it".into());
        }
        self.popup = None;
        self.hover_anchor = None;
        self.open_rename_box((start, end), old);
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: "rename".into(),
        });
    }

    fn open_rename_box(&mut self, span: (usize, usize), old: String) {
        self.rename_box = Some(RenameBox {
            doc: self.doc().id,
            span,
            caret: old.chars().count(),
            text: old.clone(),
            old,
            all: true,
            reach: None,
        });
    }

    /// The server's answer: the name's own place, and how far it reaches;
    /// or why it can't be renamed, which closes the box.
    pub(crate) fn show_rename(&mut self, info: Result<RenameInfo, String>) {
        let info = match info {
            Ok(info) => info,
            Err(why) => {
                self.rename_box = None;
                return self.notify(why);
            }
        };
        let doc = self.doc();
        let start = doc.pos_at_utf16(info.start.0, info.start.1);
        let end = doc.pos_at_utf16(info.end.0, info.end.1).max(start);
        let old = doc.slice(start, end);
        let id = doc.id;
        // From the palette, there's no box yet.
        let open = self.rename_box.as_ref().is_some_and(|b| b.doc == id);
        if !open {
            if self.focus != Focus::Editor || old.is_empty() {
                return;
            }
            self.open_rename_box((start, end), old.clone());
        }
        let Some(b) = &mut self.rename_box else {
            return;
        };
        // Untouched, it shows the name as the server has it.
        if b.text == b.old && b.all {
            b.caret = old.chars().count();
            b.text = old.clone();
        }
        b.span = (start, end);
        b.old = old;
        b.reach = Some((info.places, info.files));
    }

    pub(crate) fn rename_view(&self) -> Option<RenameView> {
        let b = self
            .rename_box
            .as_ref()
            .filter(|b| b.doc == self.doc().id)?;
        let note = match b.reach {
            None => "Finding where it's used…".to_string(),
            Some((places, files)) => {
                let places = match places {
                    1 => "1 place".to_string(),
                    n => format!("{n} places"),
                };
                let files = match files {
                    0 | 1 => String::new(),
                    n => format!(" in {n} files"),
                };
                format!("Enter to rename {places}{files} · Shift+Enter to review first")
            }
        };
        Some(RenameView {
            text: b.text.clone(),
            caret: b.caret,
            selected: b.all,
            note,
            name: b.old.chars().count(),
        })
    }

    /// Where the box goes: the name's start.
    pub(crate) fn rename_anchor(&self) -> Option<usize> {
        let b = self
            .rename_box
            .as_ref()
            .filter(|b| b.doc == self.doc().id)?;
        Some(b.span.0)
    }

    /// Keys while the box is open: they edit its name, never the text.
    /// Returns whether the box took the key.
    pub(crate) fn rename_box_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let id = self.doc().id;
        let Some(b) = self.rename_box.as_mut().filter(|b| b.doc == id) else {
            return false;
        };
        let len = b.text.chars().count();
        let byte = |text: &str, i: usize| text.char_indices().nth(i).map_or(text.len(), |c| c.0);
        // With all of it selected, an edit replaces it and a move ends it.
        let clear = |b: &mut RenameBox| {
            if b.all {
                b.text.clear();
                b.caret = 0;
                b.all = false;
            }
        };
        match key.code {
            KeyCode::Esc => self.rename_box = None,
            KeyCode::Enter => {
                let b = self.rename_box.take().unwrap();
                let new = b.text.trim().to_string();
                if new.is_empty() || new == b.old {
                    return true;
                }
                let review = key.shift;
                self.renaming = Some((b.old.clone(), new.clone(), review));
                if review {
                    self.notify("Finding what the rename changes…".into());
                }
                fx.push(Effect::PluginSubmit {
                    plugin: "lsp".into(),
                    input: if review { "rename-review" } else { "rename" }.into(),
                    text: new,
                });
            }
            KeyCode::Char('a') if key.ctrl => {
                b.all = true;
                b.caret = len;
            }
            KeyCode::Char('v') if key.ctrl => fx.push(Effect::ReadClipboard),
            KeyCode::Char('c') if key.ctrl => fx.push(Effect::Copy(b.text.clone())),
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                clear(b);
                let at = byte(&b.text, b.caret);
                b.text.insert(at, c);
                b.caret += 1;
            }
            KeyCode::Backspace => {
                if b.all {
                    clear(b);
                } else if b.caret > 0 {
                    let at = byte(&b.text, b.caret - 1);
                    b.text.remove(at);
                    b.caret -= 1;
                }
            }
            KeyCode::Delete => {
                if b.all {
                    clear(b);
                } else if b.caret < len {
                    let at = byte(&b.text, b.caret);
                    b.text.remove(at);
                }
            }
            KeyCode::Left => {
                b.caret = if b.all { 0 } else { b.caret.saturating_sub(1) };
                b.all = false;
            }
            KeyCode::Right => {
                b.caret = if b.all { len } else { (b.caret + 1).min(len) };
                b.all = false;
            }
            KeyCode::Home => {
                b.caret = 0;
                b.all = false;
            }
            KeyCode::End => {
                b.caret = len;
                b.all = false;
            }
            // Everything else waits until the box is closed.
            _ => {}
        }
        true
    }

    /// Pasted while the box is open: into its name. Returns whether it
    /// took it.
    pub(crate) fn rename_paste(&mut self, text: &str) -> bool {
        let id = self.doc().id;
        let Some(b) = self.rename_box.as_mut().filter(|b| b.doc == id) else {
            return false;
        };
        let text: String = text.lines().next().unwrap_or_default().trim().into();
        if b.all {
            b.text.clear();
            b.caret = 0;
            b.all = false;
        }
        let at = b
            .text
            .char_indices()
            .nth(b.caret)
            .map_or(b.text.len(), |c| c.0);
        b.text.insert_str(at, &text);
        b.caret += text.chars().count();
        true
    }

    /// A rename's edits came: made, it says what it did.
    pub(crate) fn rename_arrived(&mut self, edits: &[FileEdit]) {
        let Some((old, new, false)) = self.renaming.take() else {
            return;
        };
        let places: usize = edits.iter().map(|f| f.edits.len()).sum();
        self.notify(say_renamed(&old, &new, places, edits.len()));
    }

    /// The rename's edits, to review: the files that aren't open are read
    /// first.
    pub(crate) fn show_rename_review(&mut self, edits: Vec<FileEdit>, fx: &mut Vec<Effect>) {
        let Some((old, new, true)) = self.renaming.take() else {
            return;
        };
        self.message = None;
        let mut review = Review {
            old,
            new,
            ..Default::default()
        };
        let mut unread = Vec::new();
        for file in edits {
            let mut places: Vec<(TextEdit, bool)> =
                file.edits.into_iter().map(|e| (e, true)).collect();
            places.sort_by_key(|p| p.0.0);
            let open = self.doc_at(&file.path);
            let (text, revision) = match open {
                Some(doc) => (Some(doc.buffer.text()), Some(doc.buffer.revision())),
                None => {
                    unread.push(file.path.clone());
                    (None, None)
                }
            };
            review.files.push(ReviewFile {
                path: file.path,
                text,
                revision,
                places,
                open: true,
            });
        }
        review.reading = unread.len();
        // The first place is chosen.
        review.row = 1;
        self.rename_review = Some(review);
        if unread.is_empty() {
            self.start_review();
        } else {
            fx.push(Effect::ReadFiles(unread));
        }
    }

    /// Files read for the review.
    pub(crate) fn files_read(&mut self, files: Vec<(PathBuf, Option<String>)>) {
        let Some(review) = &mut self.rename_review else {
            return;
        };
        let mut texts: HashMap<PathBuf, Option<String>> = files.into_iter().collect();
        for file in &mut review.files {
            if file.text.is_none()
                && let Some(text) = texts.remove(&file.path)
            {
                file.text = Some(text.unwrap_or_default());
                review.reading = review.reading.saturating_sub(1);
            }
        }
        if review.reading == 0 {
            self.start_review();
        }
    }

    /// Shows the Rename tab and the chosen file's diff.
    fn start_review(&mut self) {
        let bar = self.bar(Side::Bottom);
        let was = (bar.visible, bar.active.clone());
        if let Some(review) = &mut self.rename_review {
            review.was = was;
        }
        self.set_rename_panel(true);
        self.show_bottom(RENAME);
        self.select(Side::Bottom, 1);
        self.show_review_file();
    }

    /// Whether the Rename tab shows: only while reviewing.
    fn set_rename_panel(&mut self, on: bool) {
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.name == RENAME) {
            plugin.panel = on.then(|| crate::panel::Panel {
                title: "Rename".into(),
                nodes: Vec::new(),
            });
        }
    }

    /// The Rename tab's rows.
    pub(crate) fn rename_nodes(&self) -> Vec<Node> {
        let Some(review) = &self.rename_review else {
            return Vec::new();
        };
        let ticked = review.ticked();
        let files = review.files.len();
        let detail = format!(
            "{ticked} of {} places in {files} {}",
            review.places(),
            if files == 1 { "file" } else { "files" }
        );
        let mut nodes = vec![Node::Header {
            title: vec![
                ("Rename ".into(), Tone::Normal),
                (review.old.clone(), Tone::Error),
                (" to ".into(), Tone::Normal),
                (review.new.clone(), Tone::Success),
            ],
            detail,
            buttons: vec![
                (
                    Button {
                        id: DISCARD.into(),
                        label: "Discard".into(),
                    },
                    "Esc".into(),
                ),
                (
                    Button {
                        id: APPLY.into(),
                        label: format!("Apply {ticked}"),
                    },
                    "Enter".into(),
                ),
            ],
        }];
        for row in review.rows() {
            let entry = match row {
                Row::File(i) => {
                    let file = &review.files[i];
                    let on = file.places.iter().filter(|p| p.1).count();
                    let name = file
                        .path
                        .file_name()
                        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
                    Entry {
                        id: format!("rn:{i}"),
                        icon: Some(file_icon(&name, self.nerd)),
                        twisty: Some(icon(
                            if file.open { Icon::Open } else { Icon::Closed },
                            self.nerd,
                        )),
                        check: Some(match on {
                            0 => Check::Off,
                            n if n == file.places.len() => Check::On,
                            _ => Check::Some,
                        }),
                        label: name,
                        detail: file
                            .path
                            .parent()
                            .map_or(String::new(), |p| p.display().to_string()),
                        badge: if on == file.places.len() {
                            format!(" {on} ")
                        } else {
                            format!(" {on} of {} ", file.places.len())
                        },
                        badge_tone: Tone::Badge,
                        ..Default::default()
                    }
                }
                Row::Place(i, j) => {
                    let file = &review.files[i];
                    let ((start, end, new), on) = &file.places[j];
                    let line = file
                        .text
                        .as_deref()
                        .and_then(|t| t.lines().nth(start.0 as usize))
                        .unwrap_or_default();
                    let a = char_column(line, start.1);
                    let b = if end.0 == start.0 {
                        char_column(line, end.1).max(a)
                    } else {
                        line.chars().count()
                    };
                    let chars: Vec<char> = line.chars().collect();
                    let before: String = chars[..a].iter().collect();
                    Entry {
                        id: format!("rn:{i}:{j}"),
                        depth: 1,
                        check: Some(if *on { Check::On } else { Check::Off }),
                        tone: if *on { Tone::Normal } else { Tone::Muted },
                        swap: Some(Swap {
                            line: start.0 as usize + 1,
                            before: before.trim_start().to_string(),
                            old: chars[a..b].iter().collect(),
                            new: if *on { new.clone() } else { String::new() },
                            after: chars[b..].iter().collect(),
                        }),
                        detail: if *on {
                            String::new()
                        } else {
                            "left out".into()
                        },
                        ..Default::default()
                    }
                }
            };
            nodes.push(Node::Entry(entry));
        }
        nodes
    }

    /// The chosen row's file as a diff in a review tab, at its place.
    fn show_review_file(&mut self) {
        let Some(review) = &self.rename_review else {
            return;
        };
        let Some(row) = review.row() else {
            return;
        };
        let (i, place) = match row {
            Row::File(i) => (i, None),
            Row::Place(i, j) => (i, Some(j)),
        };
        let file = &review.files[i];
        let old = file.text.clone().unwrap_or_default();
        let ticked: Vec<&TextEdit> = file.places.iter().filter(|p| p.1).map(|p| &p.0).collect();
        let (new, moved) = apply(&old, &ticked);
        let path = file.path.clone();
        let line = place.map(|j| file.places[j].0.0.0 as usize);
        let back_to = match self.docs.iter().find(|d| is_rename_review(d)) {
            Some(d) => d.review.as_ref().map_or(0, |r| r.back_to),
            None => self.doc().id,
        };
        let id = self.new_id();
        let mut doc = Doc::review(
            id,
            (RENAME.into(), "rename".into(), path, back_to),
            &old,
            &new,
        );
        let (old_starts, new_starts) = (line_starts(&old), line_starts(&new));
        let words = word_spans(&doc, &moved, &old_starts, &new_starts);
        let shown = line.and_then(|line| {
            let r = doc.review.as_ref()?;
            r.lines
                .iter()
                .position(|&(kind, n)| kind == crate::diff::LineKind::Added && n == line)
        });
        if let Some(r) = &mut doc.review {
            r.words = words;
        }
        if let Some(at) = shown {
            doc.cursor = doc.line_start(at);
        }
        doc.scroll = doc.line_col(doc.cursor).0.saturating_sub(3);
        match self.docs.iter().position(is_rename_review) {
            Some(i) => {
                self.docs[i] = doc;
                self.current = i;
            }
            None => {
                self.current += 1;
                self.docs.insert(self.current, doc);
            }
        }
        self.colour_review(self.current);
        self.build_views();
    }

    /// Keys in the Rename tab, and in its review tab: Space leaves the
    /// chosen place in or out, Enter applies, Esc discards. Returns
    /// whether the key was used.
    pub(crate) fn rename_review_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        if self.rename_review.is_none() || key.ctrl || key.alt {
            return false;
        }
        let in_panel = self.focus == Focus::Bottom;
        match key.code {
            KeyCode::Enter => self.apply_review(fx),
            KeyCode::Esc => self.discard_review(),
            KeyCode::Char(' ') => self.toggle_row(),
            KeyCode::Up | KeyCode::Down if in_panel => {
                let rows = self.rename_review.as_ref().map_or(0, |r| r.rows().len());
                let index = self.selected_index(Side::Bottom);
                let next = if key.code == KeyCode::Up {
                    index.saturating_sub(1)
                } else {
                    (index + 1).min(rows.saturating_sub(1))
                };
                self.choose_row(next);
            }
            KeyCode::Left | KeyCode::Right if in_panel => {
                self.fold_row(key.code == KeyCode::Right);
            }
            _ => return false,
        }
        true
    }

    /// Row `row` chosen, by key or click: its file's diff shows.
    pub(crate) fn choose_row(&mut self, row: usize) {
        let Some(review) = &mut self.rename_review else {
            return;
        };
        let file = |r: Option<Row>| match r {
            Some(Row::File(i) | Row::Place(i, _)) => Some(i),
            None => None,
        };
        let before = review.row();
        review.row = row;
        let after = review.row();
        self.select(Side::Bottom, row);
        if before != after || file(before) != file(after) {
            self.show_review_file();
            self.focus = Focus::Bottom;
        }
    }

    /// Space, or a click on a box: a place in or out, or all of a file's.
    pub(crate) fn toggle_row(&mut self) {
        let Some(review) = &mut self.rename_review else {
            return;
        };
        match review.row() {
            Some(Row::Place(i, j)) => review.files[i].places[j].1 ^= true,
            Some(Row::File(i)) => {
                let places = &mut review.files[i].places;
                let on = !places.iter().all(|p| p.1);
                places.iter_mut().for_each(|p| p.1 = on);
            }
            None => return,
        }
        self.show_review_file();
    }

    /// Left and Right on a file: hides or shows its places.
    fn fold_row(&mut self, open: bool) {
        let Some(review) = &mut self.rename_review else {
            return;
        };
        let i = match review.row() {
            Some(Row::File(i)) => i,
            Some(Row::Place(i, _)) if !open => i,
            _ => return,
        };
        review.files[i].open = open;
        review.row = review
            .rows()
            .iter()
            .position(|r| *r == Row::File(i))
            .unwrap_or(0);
        let row = review.row;
        self.build_views();
        self.select(Side::Bottom, row);
    }

    /// A click on a row of the Rename tab, by its entry id.
    pub(crate) fn rename_click(&mut self, id: &str, fx: &mut Vec<Effect>) {
        match id {
            APPLY => return self.apply_review(fx),
            DISCARD => return self.discard_review(),
            _ => {}
        }
        let index = self.selected_index(Side::Bottom);
        // A file's arrow, or its row clicked again, folds it.
        let file = self.rename_review.as_ref().and_then(|r| r.row());
        self.choose_row(index);
        if let Some(Row::File(i)) = file
            && Some(Row::File(i)) == self.rename_review.as_ref().and_then(|r| r.row())
        {
            let open = self.rename_review.as_ref().is_some_and(|r| r.files[i].open);
            self.fold_row(!open);
        }
    }

    /// Makes the places left in, each file one undo step.
    fn apply_review(&mut self, fx: &mut Vec<Effect>) {
        let Some(review) = self.rename_review.take() else {
            return;
        };
        let mut edits = Vec::new();
        let mut skipped = 0;
        for file in review.files {
            let changed = file.revision.is_some()
                && self.doc_at(&file.path).map(|d| d.buffer.revision()) != file.revision;
            let places: Vec<TextEdit> = file
                .places
                .into_iter()
                .filter(|p| p.1)
                .map(|p| p.0)
                .collect();
            if changed {
                skipped += 1;
            } else if !places.is_empty() {
                edits.push(FileEdit {
                    path: file.path,
                    edits: places,
                });
            }
        }
        let places = edits.iter().map(|f| f.edits.len()).sum();
        let files = edits.len();
        self.end_review(review.was);
        self.apply_file_edits(edits, fx);
        let mut said = say_renamed(&review.old, &review.new, places, files);
        if skipped > 0 {
            said.push_str(&format!("; {skipped} changed since and left as they are"));
        }
        self.notify(said);
    }

    fn discard_review(&mut self) {
        if let Some(review) = self.rename_review.take() {
            self.end_review(review.was);
            self.notify("Nothing renamed".into());
        }
    }

    /// Closes the review tab and the Rename tab, back to how they were.
    fn end_review(&mut self, (visible, active): (bool, Option<String>)) {
        if let Some(i) = self.docs.iter().position(is_rename_review) {
            let back_to = self.docs[i].review.as_ref().map(|r| r.back_to);
            self.docs.remove(i);
            if self.current >= i {
                self.current = self.current.saturating_sub(1);
            }
            if let Some(j) = self.docs.iter().position(|d| Some(d.id) == back_to) {
                self.current = j;
            }
        }
        self.set_rename_panel(false);
        let bar = self.bar_mut(Side::Bottom);
        bar.visible = visible;
        bar.active = active;
        self.focus = Focus::Editor;
        self.build_views();
    }

    /// What the status bar says while a rename is reviewed.
    pub(crate) fn reviewing_rename(&self) -> bool {
        self.rename_review.is_some()
    }
}

pub(crate) fn is_rename_review(doc: &Doc) -> bool {
    doc.review.as_ref().is_some_and(|r| r.plugin == RENAME)
}

fn say_renamed(old: &str, new: &str, places: usize, files: usize) -> String {
    let places = match places {
        1 => "1 place".to_string(),
        n => format!("{n} places"),
    };
    let files = match files {
        0 | 1 => String::new(),
        n => format!(" in {n} files"),
    };
    format!("Renamed {old} to {new}: {places}{files}")
}

/// The old and new names' places in the review's diff, as its buffer's
/// characters, to tint.
fn word_spans(
    doc: &Doc,
    moved: &[Moved],
    old_starts: &[usize],
    new_starts: &[usize],
) -> Vec<(usize, usize)> {
    use crate::diff::LineKind;
    let Some(review) = &doc.review else {
        return Vec::new();
    };
    let mut spans = Vec::new();
    for (row, &(kind, number)) in review.lines.iter().enumerate() {
        let (side, starts, old) = match kind {
            LineKind::Removed => (review.old_numbers[row], old_starts, true),
            LineKind::Added => (number, new_starts, false),
            _ => continue,
        };
        let start = doc.line_start(row);
        for &(was, now) in moved {
            let (a, b) = if old { was } else { now };
            let (line, col) = line_col(starts, a);
            if line == side && b > a {
                spans.push((start + col, start + col + (b - a)));
            }
        }
    }
    spans
}
