//! The palette as one search: commands, symbols, files and text at once,
//! each in a section with its best few, or one of them on its own. A scope
//! chip narrows it, as does its sign typed first: `>` commands, `@`
//! symbols, `/` text.
//!
//! Commands, open files' symbols and file names are in memory and come at
//! once. Text comes from the project search the Search view runs, asked
//! for a moment after typing stops.

use std::path::PathBuf;

use super::*;
use crate::doc::Doc;
use crate::editor::window::{PaletteExtra, PaletteIcon};

/// How long typing must stop before the text is searched.
const TEXT_WAIT_MS: u64 = 250;

/// With Everything, each section's most.
const COMMANDS_SHOWN: usize = 4;
const SYMBOLS_SHOWN: usize = 3;
const FILES_SHOWN: usize = 2;
const TEXT_SHOWN: usize = 1;

/// With nothing typed, how many recent ones.
const RECENT_SHOWN: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Scope {
    #[default]
    Everything,
    Files,
    Commands,
    Symbols,
    Text,
}

const SCOPES: [Scope; 5] = [
    Scope::Everything,
    Scope::Files,
    Scope::Commands,
    Scope::Symbols,
    Scope::Text,
];

impl Scope {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Scope::Everything => "Everything",
            Scope::Files => "Files",
            Scope::Commands => "Commands",
            Scope::Symbols => "Symbols",
            Scope::Text => "Text",
        }
    }

    fn sign(self) -> Option<char> {
        match self {
            Scope::Commands => Some('>'),
            Scope::Symbols => Some('@'),
            Scope::Text => Some('/'),
            _ => None,
        }
    }

    fn index(self) -> usize {
        SCOPES.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// One row: what Enter does with it, what it shows, and its key.
#[derive(Clone, Debug, Default)]
pub(crate) struct Row {
    pub id: String,
    pub label: String,
    pub key: Option<String>,
    pub extra: PaletteExtra,
}

/// The project's text, searched for the palette.
#[derive(Debug, Default)]
pub(crate) struct TextSearch {
    /// Bumped as the query changes, to time the pause.
    tick: u64,
    /// What was asked for last, and what came back for it.
    asked: String,
    hits: Vec<SearchHit>,
    searching: bool,
}

impl Editor {
    /// Opens the palette as the search, on a scope.
    pub(crate) fn open_search(&mut self, scope: Scope, fx: &mut Vec<Effect>) {
        self.palette = Some(Palette::new(
            Source::Search {
                scope,
                signed: false,
            },
            None,
            Vec::new(),
        ));
        self.focus = Focus::Palette;
        // The file names come from the Explorer's list, or are asked for.
        if self.tree.as_ref().is_none_or(|t| t.files.is_empty()) {
            fx.push(Effect::ListFiles);
        }
        self.search_changed();
    }

    /// The scope and query, while the palette is the search.
    fn search_state(&self) -> Option<(Scope, &str)> {
        let p = self.palette.as_ref()?;
        match p.source {
            Source::Search { scope, .. } => Some((scope, p.query.as_str())),
            _ => None,
        }
    }

    pub(crate) fn search_open(&self) -> bool {
        self.search_state().is_some()
    }

    /// The query or scope changed: the rows are worked out again, and the
    /// text waits for a pause.
    pub(crate) fn search_changed(&mut self) {
        self.palette_text.tick += 1;
        let rows = self.search_rows();
        if let Some(p) = &mut self.palette {
            p.selected = p.selected.min(rows.len().saturating_sub(1));
            p.rows = rows;
        }
    }

    /// A key in the palette while it's the search. Returns whether it
    /// took it.
    pub(crate) fn search_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let Some((scope, query)) = self.search_state() else {
            return false;
        };
        let empty = query.is_empty();
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let back = key.code == KeyCode::BackTab || key.shift;
                let i = scope.index();
                let next = if back { i + SCOPES.len() - 1 } else { i + 1 } % SCOPES.len();
                self.set_scope(SCOPES[next], false);
            }
            // Back to Everything from a scope, as if its sign went.
            KeyCode::Backspace if empty && scope != Scope::Everything => {
                self.set_scope(Scope::Everything, false);
            }
            KeyCode::Char(c)
                if empty
                    && scope == Scope::Everything
                    && !key.ctrl
                    && !key.alt
                    && let Some(to) = SCOPES.iter().find(|s| s.sign() == Some(c)) =>
            {
                self.set_scope(*to, true);
            }
            KeyCode::Enter if key.alt => {
                if matches!(scope, Scope::Text | Scope::Everything) && !empty {
                    let query = query.to_string();
                    self.close_palette();
                    self.show_view(views::SEARCH, fx);
                    self.view_submit(views::SEARCH, &query, fx);
                }
            }
            KeyCode::Enter => {
                let selected = self.palette.as_ref().map_or(0, |p| p.selected);
                let Some(row) = self.palette.as_ref().and_then(|p| p.rows.get(selected)) else {
                    return true;
                };
                if row.id.is_empty() {
                    return true;
                }
                let id = row.id.clone();
                self.close_palette();
                self.search_pick(&id, key.ctrl, fx);
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn set_scope(&mut self, scope: Scope, signed: bool) {
        if let Some(p) = &mut self.palette
            && let Source::Search { .. } = p.source
        {
            p.source = Source::Search { scope, signed };
            p.selected = 0;
        }
        self.search_changed();
    }

    /// A scope chip or a "more" link, by its index.
    pub(crate) fn search_scope_clicked(&mut self, i: usize) {
        if let Some(scope) = SCOPES.get(i) {
            self.set_scope(*scope, false);
        }
    }

    /// Does what a row is for: runs a command, opens a file, or goes to a
    /// symbol or a line. `side` opens it to the side.
    fn search_pick(&mut self, id: &str, side: bool, fx: &mut Vec<Effect>) {
        if let Some(command) = id.strip_prefix("c:") {
            self.recent_commands.retain(|c| c != command);
            self.recent_commands.insert(0, command.to_string());
            self.recent_commands.truncate(20);
            self.macro_command(command);
            return self.run_command_id(command, fx);
        }
        let (path, line, column) = if let Some(path) = id.strip_prefix("f:") {
            (path.to_string(), None, 0)
        } else if let Some(rest) = id.strip_prefix("s:").or_else(|| id.strip_prefix("t:")) {
            // path:line:column, the path maybe with colons of its own.
            let mut parts = rest.rsplitn(3, ':');
            let column = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
            let line = parts.next().and_then(|l| l.parse().ok());
            (parts.next().unwrap_or_default().to_string(), line, column)
        } else {
            return;
        };
        if side {
            self.run("view.split", fx);
        }
        match (path.is_empty(), line) {
            // A symbol in the current file, which may have no path.
            (true, Some(line)) => self.go_to_block(line as usize),
            (false, Some(line)) => self.open_at(PathBuf::from(&path), line, column, fx),
            (false, None) => self.open(PathBuf::from(&path), fx),
            (true, None) => {}
        }
    }

    /// Notes the file in front, for Recently opened.
    pub(crate) fn note_recent(&mut self) {
        let Some(path) = self.doc().buffer.path.clone() else {
            return;
        };
        if self.recent_files.first() != Some(&path) {
            self.recent_files.retain(|p| *p != path);
            self.recent_files.insert(0, path);
            self.recent_files.truncate(20);
        }
    }

    /// The project's files, as the Explorer lists them or as they were
    /// asked for.
    fn search_files(&self) -> &[String] {
        match &self.tree {
            Some(t) if !t.files.is_empty() => &t.files,
            _ => &self.palette_files,
        }
    }

    /// The project's files came, for the palette.
    pub(crate) fn search_files_came(&mut self, files: Vec<String>) {
        self.palette_files = files;
        self.search_changed();
    }

    /// The palette's text search waits on a pause: which one, and how long.
    /// `None` with nothing to search.
    pub fn palette_search_wait(&self) -> Option<(u64, u64)> {
        let text = self.search_text()?;
        (text != self.palette_text.asked).then_some((self.palette_text.tick, TEXT_WAIT_MS))
    }

    /// The text to search for, with the scope that shows it.
    fn search_text(&self) -> Option<String> {
        let (scope, query) = self.search_state()?;
        let query = query.trim();
        let enough = match scope {
            Scope::Text => !query.is_empty(),
            // A letter or two would match nearly every line.
            Scope::Everything => query.chars().count() >= 3,
            _ => false,
        };
        enough.then(|| query.to_string())
    }

    /// The pause went on long enough: the text is searched.
    pub(crate) fn palette_search_due(&mut self, tick: u64, fx: &mut Vec<Effect>) {
        if tick != self.palette_text.tick {
            return;
        }
        if let Some(text) = self.search_text() {
            self.palette_text.asked = text.clone();
            self.palette_text.hits.clear();
            self.palette_text.searching = true;
            fx.push(Effect::Search(text));
            self.search_changed();
        }
    }

    /// What a project search found, if the palette asked for it.
    pub(crate) fn palette_search_came(&mut self, query: &str, hits: &[SearchHit]) {
        if query == self.palette_text.asked {
            self.palette_text.hits = hits.to_vec();
            self.palette_text.searching = false;
            self.search_changed();
        }
    }

    /// The scope chips, footer and placeholder for the window.
    pub(crate) fn search_chrome(&self) -> Option<SearchChrome> {
        let p = self.palette.as_ref()?;
        let Source::Search { scope, signed } = p.source else {
            return None;
        };
        let scopes = SCOPES
            .iter()
            .map(|s| {
                let key = match s {
                    Scope::Files => self.key_hint("file.open"),
                    s => s.sign().map(String::from),
                };
                (s.label().to_string(), key, *s == scope)
            })
            .collect();
        let hints: &[(&str, &str)] = match scope {
            Scope::Text => &[
                ("Enter", "to go there"),
                ("Ctrl Enter", "to the side"),
                ("Alt Enter", "all in the Search view"),
            ],
            Scope::Commands => &[
                ("↑ ↓", "to move"),
                ("Enter", "to run"),
                ("Tab", "for the next scope"),
            ],
            _ if p.query.is_empty() => &[
                ("↑ ↓", "to move"),
                ("Enter", "to open"),
                ("Tab", "for the next scope"),
            ],
            _ => &[
                ("↑ ↓", "to move"),
                ("Enter", "to open"),
                ("Ctrl Enter", "to open to the side"),
                ("Tab", "for the next scope"),
            ],
        };
        Some(SearchChrome {
            scopes,
            sign: signed.then(|| scope.sign()).flatten(),
            placeholder: match scope {
                Scope::Everything => "Search files, commands, symbols and text",
                Scope::Files => "Search files by name",
                Scope::Commands => "Run a command",
                Scope::Symbols => "Go to a symbol",
                Scope::Text => "Search the project's text",
            }
            .into(),
            hints: hints
                .iter()
                .map(|(k, w)| (k.to_string(), w.to_string()))
                .collect(),
        })
    }

    /// Every row for the scope and query, sections in order.
    fn search_rows(&self) -> Vec<Row> {
        let Some((scope, query)) = self.search_state() else {
            return Vec::new();
        };
        let query = query.trim().to_lowercase();
        let all = scope == Scope::Everything;
        let mut rows = Vec::new();
        // A section: a heading over its first row, and a link to its scope
        // under the last when there are more.
        let section = |rows: &mut Vec<Row>,
                       title: Option<String>,
                       mut found: Vec<Row>,
                       shown: usize,
                       to: Scope,
                       more: Option<String>| {
            if found.is_empty() {
                return;
            }
            let left = found.len().saturating_sub(shown);
            found.truncate(shown);
            if let Some(first) = found.first_mut() {
                first.extra.heading = title;
            }
            if left > 0 || more.is_some() {
                let text = more.unwrap_or_else(|| format!("{left} more · Tab to see them all"));
                if let Some(last) = found.last_mut() {
                    last.extra.more = Some((text, to.index()));
                }
            }
            rows.extend(found);
        };
        if query.is_empty() {
            match scope {
                Scope::Everything => {
                    section(
                        &mut rows,
                        Some("Recently opened".into()),
                        self.recent_file_rows(),
                        RECENT_SHOWN,
                        Scope::Files,
                        None,
                    );
                    section(
                        &mut rows,
                        Some("Recently used".into()),
                        self.recent_command_rows(),
                        RECENT_SHOWN,
                        Scope::Commands,
                        None,
                    );
                }
                Scope::Files => {
                    let recent = self.recent_file_rows();
                    let seen: Vec<String> = recent.iter().map(|r| r.id.clone()).collect();
                    let rest = self
                        .search_files()
                        .iter()
                        .filter(|f| !f.ends_with('/'))
                        .map(|f| self.file_row(f, ""))
                        .filter(|r| !seen.contains(&r.id))
                        .take(PALETTE_LIMIT)
                        .collect();
                    section(
                        &mut rows,
                        Some("Recently opened".into()),
                        recent,
                        RECENT_SHOWN,
                        Scope::Files,
                        None,
                    );
                    let title = (!rows.is_empty()).then(|| "Files".to_string());
                    section(&mut rows, title, rest, PALETTE_LIMIT, Scope::Files, None);
                }
                Scope::Commands => {
                    let recent = self.recent_command_rows();
                    let rest = self.command_rows("");
                    section(
                        &mut rows,
                        Some("Recently used".into()),
                        recent,
                        RECENT_SHOWN,
                        Scope::Commands,
                        None,
                    );
                    let title = (!rows.is_empty()).then(|| "All commands".to_string());
                    section(&mut rows, title, rest, PALETTE_LIMIT, Scope::Commands, None);
                }
                Scope::Symbols => {
                    section(
                        &mut rows,
                        None,
                        self.symbol_rows("", true),
                        PALETTE_LIMIT,
                        Scope::Symbols,
                        None,
                    );
                }
                Scope::Text => {}
            }
            if rows.is_empty() && scope == Scope::Everything {
                rows.push(Row {
                    label: "Files you open and commands you run show here".into(),
                    extra: PaletteExtra {
                        muted: true,
                        ..Default::default()
                    },
                    ..Default::default()
                });
            }
            return rows;
        }
        let (n_commands, n_symbols, n_files, n_text) = match scope {
            Scope::Everything => (COMMANDS_SHOWN, SYMBOLS_SHOWN, FILES_SHOWN, TEXT_SHOWN),
            _ => (PALETTE_LIMIT, PALETTE_LIMIT, PALETTE_LIMIT, PALETTE_LIMIT),
        };
        let heading = |s: &str| all.then(|| s.to_string());
        if matches!(scope, Scope::Everything | Scope::Commands) {
            section(
                &mut rows,
                heading("Commands"),
                self.command_rows(&query),
                n_commands,
                Scope::Commands,
                None,
            );
        }
        if matches!(scope, Scope::Everything | Scope::Symbols) {
            section(
                &mut rows,
                heading("Symbols"),
                self.symbol_rows(&query, scope == Scope::Symbols),
                n_symbols,
                Scope::Symbols,
                None,
            );
        }
        if matches!(scope, Scope::Everything | Scope::Files) {
            section(
                &mut rows,
                heading("Files"),
                self.ranked_files(&query),
                n_files,
                Scope::Files,
                None,
            );
        }
        if matches!(scope, Scope::Everything | Scope::Text) {
            let text = &self.palette_text;
            let current = self.search_text().is_some_and(|t| t == text.asked);
            let hits: Vec<Row> = if current {
                text.hits
                    .iter()
                    .take(PALETTE_LIMIT)
                    .map(|h| text_row(h, &query))
                    .collect()
            } else {
                Vec::new()
            };
            let files = {
                let mut paths: Vec<&str> = text.hits.iter().map(|h| h.path.as_str()).collect();
                paths.dedup();
                paths.len()
            };
            let title = format!(
                "{} in {} file{}",
                text.hits.len(),
                files,
                if files == 1 { "" } else { "s" }
            );
            let waiting = self.search_text().is_some() && (!current || text.searching);
            if waiting {
                rows.push(Row {
                    label: "Searching…".into(),
                    extra: PaletteExtra {
                        heading: Some("Text".into()),
                        muted: true,
                        ..Default::default()
                    },
                    ..Default::default()
                });
            } else if all {
                let left = hits.len().saturating_sub(n_text);
                let more = (left > 0).then(|| {
                    let mut paths: Vec<&str> = text.hits[n_text..]
                        .iter()
                        .map(|h| h.path.as_str())
                        .collect();
                    paths.sort_unstable();
                    paths.dedup();
                    let n = paths.len();
                    format!(
                        "{left} more in {n} file{} · Tab to see them all",
                        if n == 1 { "" } else { "s" }
                    )
                });
                section(
                    &mut rows,
                    Some("Text".into()),
                    hits,
                    n_text,
                    Scope::Text,
                    more,
                );
            } else if !hits.is_empty() {
                section(&mut rows, Some(title), hits, n_text, Scope::Text, None);
            } else if current {
                rows.push(Row {
                    label: "Nothing in the project's text".into(),
                    extra: PaletteExtra {
                        muted: true,
                        ..Default::default()
                    },
                    ..Default::default()
                });
            }
        }
        rows
    }

    fn command_row(&self, id: &str, title: &str) -> Row {
        // Titles name their category before a colon: the row shows the
        // rest, and the category dimmed.
        let (category, name) = match title.split_once(": ") {
            Some((c, n)) => (c.to_string(), n.to_string()),
            None => (String::new(), title.to_string()),
        };
        Row {
            id: format!("c:{id}"),
            label: name,
            key: self.key_hint(id),
            extra: PaletteExtra {
                detail: category,
                icon: PaletteIcon::Command,
                ..Default::default()
            },
        }
    }

    fn command_rows(&self, query: &str) -> Vec<Row> {
        let commands = self.commands();
        let mut ranked: Vec<(i64, usize)> = commands
            .iter()
            .enumerate()
            .filter_map(|(i, (_, title))| Some((fuzzy::score(query, title)?, i)))
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), i));
        ranked
            .into_iter()
            .map(|(_, i)| self.command_row(&commands[i].0, &commands[i].1))
            .collect()
    }

    fn recent_command_rows(&self) -> Vec<Row> {
        let commands = self.commands();
        self.recent_commands
            .iter()
            .filter_map(|id| commands.iter().find(|(c, _)| c == id))
            .map(|(id, title)| self.command_row(id, title))
            .collect()
    }

    fn file_row(&self, path: &str, _query: &str) -> Row {
        let (folder, name) = match path.rsplit_once('/') {
            Some((f, n)) => (f.to_string(), n.to_string()),
            None => (String::new(), path.to_string()),
        };
        Row {
            id: format!("f:{path}"),
            extra: PaletteExtra {
                detail: folder,
                icon: PaletteIcon::File(name.clone()),
                ..Default::default()
            },
            label: name,
            key: None,
        }
    }

    fn recent_file_rows(&self) -> Vec<Row> {
        let current = self.doc().buffer.path.as_ref();
        self.recent_files
            .iter()
            .filter(|p| Some(*p) != current)
            .map(|p| self.file_row(&p.to_string_lossy(), ""))
            .collect()
    }

    /// Files whose names match best, then those whose folders do.
    fn ranked_files(&self, query: &str) -> Vec<Row> {
        let files = self.search_files();
        let mut ranked: Vec<(i64, usize)> = files
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.ends_with('/'))
            .filter_map(|(i, f)| {
                let name = f.rsplit('/').next().unwrap_or(f);
                let score = match fuzzy::score(query, name) {
                    Some(s) => s + 1000,
                    None => fuzzy::score(query, f)?,
                };
                Some((score, i))
            })
            .collect();
        ranked.sort_by_key(|&(score, i)| (Reverse(score), files[i].len(), i));
        ranked
            .into_iter()
            .take(PALETTE_LIMIT)
            .map(|(_, i)| self.file_row(&files[i], query))
            .collect()
    }

    /// The symbols of the open files, the current one's first. In the
    /// Symbols scope with nothing typed, the current file's in order.
    fn symbol_rows(&self, query: &str, scoped: bool) -> Vec<Row> {
        let current = self.doc().id;
        let mut docs: Vec<&Doc> = self
            .docs
            .iter()
            .filter(|d| d.page.is_none() && d.review.is_none())
            .collect();
        docs.sort_by_key(|d| d.id != current);
        if query.is_empty() {
            docs.truncate(1);
        }
        let mut ranked: Vec<(i64, usize, Row)> = Vec::new();
        for (n, doc) in docs.iter().enumerate() {
            let path = doc
                .buffer
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string());
            let file = path
                .as_deref()
                .map_or("Untitled", |p| p.rsplit('/').next().unwrap_or(p))
                .to_string();
            let symbols = crate::symbols::symbols(doc);
            // What each is in, from the nearest one less deep above it.
            let mut stack: Vec<String> = Vec::new();
            for s in &symbols {
                stack.truncate(s.depth);
                let inside = stack.last().cloned();
                let label = match s.kind.as_str() {
                    "#" => s.name.clone(),
                    kind => format!("{kind} {}", s.name),
                };
                stack.push(label.clone());
                if s.kind == "impl" && !scoped {
                    continue;
                }
                let score = if query.is_empty() {
                    0
                } else {
                    match fuzzy::score(query, &s.name) {
                        Some(score) => score,
                        None => continue,
                    }
                };
                let at = format!("{file}:{}", s.line + 1);
                let detail = match inside {
                    Some(i) => format!("{i} · {at}"),
                    None => at,
                };
                // The current file's go by line alone, as it may have no
                // path.
                let path = match &path {
                    _ if doc.id == current => String::new(),
                    Some(p) => p.clone(),
                    None => continue,
                };
                ranked.push((
                    score,
                    n,
                    Row {
                        id: format!("s:{path}:{}:0", s.line),
                        label: match (scoped && query.is_empty(), s.depth) {
                            (true, d) => format!("{}{label}", "  ".repeat(d)),
                            _ => label,
                        },
                        key: None,
                        extra: PaletteExtra {
                            detail,
                            icon: {
                                let (letter, kind) = symbol_letter(&s.kind);
                                PaletteIcon::Symbol(letter, kind)
                            },
                            ..Default::default()
                        },
                    },
                ));
            }
        }
        if !query.is_empty() {
            ranked.sort_by_key(|(score, n, _)| (Reverse(*score), *n));
        }
        ranked
            .into_iter()
            .take(PALETTE_LIMIT)
            .map(|(_, _, r)| r)
            .collect()
    }
}

/// The scope chips, the sign typed, the placeholder and the footer.
pub(crate) struct SearchChrome {
    pub scopes: Vec<(String, Option<String>, bool)>,
    pub sign: Option<char>,
    pub placeholder: String,
    pub hints: Vec<(String, String)>,
}

/// A symbol kind's letter, and whether it's a type, which has its own
/// colour.
fn symbol_letter(kind: &str) -> (char, bool) {
    match kind {
        "fn" | "function" | "def" | "method" | "func" => ('ƒ', false),
        "struct" | "class" | "enum" | "trait" | "type" | "interface" | "union" => ('T', true),
        "impl" => ('I', true),
        "mod" | "module" | "namespace" | "package" => ('M', false),
        "const" | "static" | "let" | "var" => ('C', false),
        "#" => ('#', false),
        k => (k.chars().next().unwrap_or('?').to_ascii_uppercase(), false),
    }
}

/// A line the text search found, with what matched tinted.
fn text_row(hit: &SearchHit, query: &str) -> Row {
    let text = hit.text.trim();
    let lower = text.to_lowercase();
    let hit_range = lower.find(query).map(|at| {
        let start = lower[..at].chars().count();
        (start, start + query.chars().count())
    });
    let name = hit.path.rsplit('/').next().unwrap_or(&hit.path).to_string();
    Row {
        id: format!("t:{}:{}:{}", hit.path, hit.line, hit.column),
        label: text.to_string(),
        key: None,
        extra: PaletteExtra {
            detail: format!("{name}:{}", hit.line + 1),
            icon: PaletteIcon::File(name),
            hit: hit_range,
            code: true,
            ..Default::default()
        },
    }
}
