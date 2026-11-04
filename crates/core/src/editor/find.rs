//! Find and replace in the open file.

use super::*;

/// The find box, as a window draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindView {
    pub query: String,
    /// The replacement, when replacing.
    pub replace: Option<String>,
    /// Whether typing goes to the replacement.
    pub in_replace: bool,
    /// Whether keys go to the box.
    pub focused: bool,
    /// Which match is selected, from 1, and how many there are.
    pub current: Option<usize>,
    pub total: usize,
    /// Case is ignored: the query has no capitals.
    pub ignore_case: bool,
}

pub(super) fn same(a: char, b: char, fold: bool) -> bool {
    a == b || (fold && a.to_lowercase().eq(b.to_lowercase()))
}

/// Where `query` starts in `chars`, without overlaps.
pub(super) fn line_matches(chars: &[char], query: &[char], fold: bool) -> Vec<usize> {
    let mut found = Vec::new();
    let mut i = 0;
    while i + query.len() <= chars.len() {
        if chars[i..i + query.len()]
            .iter()
            .zip(query)
            .all(|(a, b)| same(*a, *b, fold))
        {
            found.push(i);
            i += query.len();
        } else {
            i += 1;
        }
    }
    found
}

impl Editor {
    /// Opens find, starting from the selection if it's on one line.
    pub(crate) fn open_find(&mut self, replace: bool) {
        let doc = self.doc();
        let selected = doc
            .selection()
            .map(|(s, e)| doc.slice(s, e))
            .filter(|t| !t.contains('\n'));
        let origin = doc.selection().map_or(doc.cursor, |(start, _)| start);
        let find = self.find.get_or_insert_with(|| Find {
            query: String::new(),
            replace: None,
            in_replace: false,
            origin,
        });
        if let Some(text) = &selected {
            find.query = text.clone();
            find.origin = origin;
        }
        if replace {
            find.replace.get_or_insert_default();
        }
        find.in_replace = replace && !find.query.is_empty();
        self.focus = Focus::Find;
        if selected.is_some() {
            self.find_from(origin);
        }
    }

    pub(crate) fn find_key(&mut self, key: Key) {
        let cursor = self.cursor();
        let Some(find) = &mut self.find else {
            self.focus = Focus::Editor;
            return;
        };
        let mut search = false;
        match key.code {
            KeyCode::Esc => {
                self.find = None;
                self.focus = Focus::Editor;
            }
            KeyCode::Tab | KeyCode::BackTab if find.replace.is_some() => {
                find.in_replace = !find.in_replace
            }
            KeyCode::Down => self.find_from(cursor + 1),
            KeyCode::Up => self.find_before(cursor),
            KeyCode::Enter if find.in_replace && key.alt => self.replace_all(),
            KeyCode::Enter if find.in_replace => self.replace_one(),
            KeyCode::Enter => self.find_from(cursor + 1),
            KeyCode::Backspace => {
                find.field().pop();
                search = !find.in_replace;
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                find.field().push(c);
                search = !find.in_replace;
            }
            _ => {}
        }
        if search && let Some(find) = &self.find {
            self.find_from(find.origin);
        }
    }

    /// The query as characters, and whether to ignore case: only when the
    /// query is all lowercase.
    pub(crate) fn query(&self) -> Option<(Vec<char>, bool)> {
        let query = &self.find.as_ref()?.query;
        let fold = !query.chars().any(char::is_uppercase);
        Some((query.chars().collect::<Vec<_>>(), fold)).filter(|(q, _)| !q.is_empty())
    }

    /// Every match in the document, left to right.
    pub(crate) fn all_matches(&self, query: &[char], fold: bool) -> Vec<usize> {
        let doc = self.doc();
        (0..=doc.last_line())
            .flat_map(|line| {
                let start = doc.line_start(line);
                line_matches(&doc.line_chars(line), query, fold)
                    .into_iter()
                    .map(move |m| start + m)
            })
            .collect()
    }

    pub(crate) fn match_at(&self, pos: usize) -> bool {
        let Some((query, fold)) = self.query() else {
            return false;
        };
        let doc = self.doc();
        let (line, col) = doc.line_col(pos);
        let chars = doc.line_chars(line);
        chars.len() >= col + query.len()
            && chars[col..col + query.len()]
                .iter()
                .zip(&query)
                .all(|(a, b)| same(*a, *b, fold))
    }

    /// Moves to the first match at or after `from`, wrapping around. Matches
    /// come from the current text every time, so edits can't leave stale ones.
    pub(crate) fn find_from(&mut self, from: usize) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let matches = self.all_matches(&query, fold);
        let hit = matches.iter().find(|m| **m >= from).or(matches.first());
        self.go_to_match(hit.copied(), &query);
    }

    /// Moves to the last match before `before`, wrapping around.
    pub(crate) fn find_before(&mut self, before: usize) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let matches = self.all_matches(&query, fold);
        let hit = matches
            .iter()
            .rev()
            .find(|m| **m < before)
            .or(matches.last());
        self.go_to_match(hit.copied(), &query);
    }

    pub(crate) fn go_to_match(&mut self, hit: Option<usize>, query: &[char]) {
        match hit {
            Some(pos) => self.set_cursor(pos),
            None => self.error(format!("No match for {}", query.iter().collect::<String>())),
        }
    }

    /// Replaces the match at the cursor, if there is one, and moves to the
    /// next.
    pub(crate) fn replace_one(&mut self) {
        let Some((query, _)) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let cursor = self.cursor();
        if self.match_at(cursor) {
            self.replace(cursor, cursor + query.len(), &with, Kind::Other);
        }
        self.find_from(self.cursor());
    }

    pub(crate) fn replace_all(&mut self) {
        let Some((query, fold)) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let doc = self.doc();
        let edits: Vec<Edit> = self
            .all_matches(&query, fold)
            .into_iter()
            .map(|at| Edit {
                at,
                removed: doc.slice(at, at + query.len()),
                inserted: with.clone(),
            })
            .collect();
        match edits.len() {
            0 => self.error(format!("No match for {}", query.iter().collect::<String>())),
            n => {
                self.apply_all(edits);
                self.notify(format!("Replaced {n}"));
            }
        }
    }

    pub(crate) fn find_view(&self) -> Option<FindView> {
        let find = self.find.as_ref()?;
        let (matches, ignore_case) = match self.query() {
            Some((query, fold)) => (self.all_matches(&query, fold), fold),
            None => (Vec::new(), true),
        };
        let at = self.doc().selection().map(|(start, _)| start);
        Some(FindView {
            query: find.query.clone(),
            replace: find.replace.clone(),
            in_replace: find.in_replace,
            focused: self.focus == Focus::Find,
            current: at
                .and_then(|a| matches.iter().position(|&m| m == a))
                .map(|i| i + 1),
            total: matches.len(),
            ignore_case,
        })
    }

    /// A button or field of the window's find box.
    pub(crate) fn find_ui(&mut self, what: FindAction) {
        let cursor = self.cursor();
        let Some(find) = &mut self.find else { return };
        match what {
            FindAction::Next => self.find_from(cursor + 1),
            FindAction::Previous => self.find_before(cursor),
            FindAction::Close => {
                self.find = None;
                self.focus = Focus::Editor;
            }
            FindAction::ToggleReplace => {
                if find.replace.is_some() {
                    find.replace = None;
                    find.in_replace = false;
                } else {
                    find.replace = Some(String::new());
                }
                self.focus = Focus::Find;
            }
            FindAction::Field(replace) => {
                find.in_replace = replace && find.replace.is_some();
                self.focus = Focus::Find;
            }
            FindAction::ReplaceOne => self.replace_one(),
            FindAction::ReplaceAll => self.replace_all(),
        }
    }
}

/// What a click on the window's find box does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindAction {
    Next,
    Previous,
    Close,
    ToggleReplace,
    /// Typing goes to the query, or with `true` the replacement.
    Field(bool),
    ReplaceOne,
    ReplaceAll,
}
