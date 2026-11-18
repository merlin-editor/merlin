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
    /// Case is ignored: Match Case is off and the query has no capitals.
    pub ignore_case: bool,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Only the text that was selected is searched.
    pub in_selection: bool,
    /// The query isn't a regular expression that can be used.
    pub invalid: bool,
    /// While a large file's matches are counted in the background, how far
    /// through it the count is, in percent: `total` is so far.
    pub counting: Option<u8>,
}

pub(crate) use crate::search::Matcher;

/// Lines not yet counted that Find searches at once for the next match,
/// about 4 MB.
const SEEK_BUDGET: usize = 65_536;

impl Editor {
    /// Opens find, starting from the selection if it's on one line.
    pub(crate) fn open_find(&mut self, replace: bool) {
        let doc = self.doc();
        let selected = doc
            .selection()
            .map(|(s, e)| doc.slice(s, e))
            .filter(|t| !t.contains('\n'));
        let origin = doc.selection().map_or(doc.cursor, |(start, _)| start);
        // A selection over several lines is what to search in, as VS Code
        // does.
        let scope = doc
            .selection()
            .filter(|&(s, e)| doc.slice(s, e).contains('\n'));
        let find = self.find.get_or_insert_with(|| Find {
            query: String::new(),
            replace: None,
            in_replace: false,
            origin,
            match_case: false,
            whole_word: false,
            regex: false,
            scope: None,
            waiting: None,
        });
        if let Some(text) = &selected {
            find.query = text.clone();
            find.origin = origin;
        }
        find.scope = scope;
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
            KeyCode::Char('c') if key.alt => {
                find.match_case = !find.match_case;
                search = true;
            }
            KeyCode::Char('w') if key.alt => {
                find.whole_word = !find.whole_word;
                search = true;
            }
            KeyCode::Char('r') if key.alt => {
                find.regex = !find.regex;
                search = true;
            }
            KeyCode::Char('l') if key.alt => {
                self.toggle_in_selection();
                return;
            }
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

    /// How the find box matches, if it has a query that can be used. Case
    /// is ignored unless Match Case is on or the query has capitals.
    pub(crate) fn query(&self) -> Option<Matcher> {
        // With the find box closed, Vim's search, while it's tinted.
        match &self.find {
            Some(_) => self.matcher().ok().flatten(),
            None => self.vim_matcher(),
        }
    }

    /// The find box's matcher, `None` with no query, or an error for a
    /// regular expression that can't be used.
    fn matcher(&self) -> Result<Option<Matcher>, regex::Error> {
        let Some(find) = &self.find else {
            return Ok(None);
        };
        if find.query.is_empty() {
            return Ok(None);
        }
        let fold = !find.match_case && !find.query.chars().any(char::is_uppercase);
        let matcher = if find.regex {
            let mut source = find.query.clone();
            if find.whole_word {
                source = format!(r"\b(?:{source})\b");
            }
            if fold {
                source = format!("(?i){source}");
            }
            Matcher::regex(regex::Regex::new(&source)?)
        } else {
            Matcher::plain(&find.query, fold, find.whole_word, None)
        };
        Ok(Some(matcher.with_scope(find.scope)))
    }

    /// Every match in the document, left to right, as where each starts
    /// and how long it is.
    pub(crate) fn all_matches(&self, matcher: &Matcher) -> Vec<(usize, usize)> {
        let doc = self.doc();
        matcher.all(doc.buffer.rope(), doc.found(matcher).as_deref())
    }

    /// The match that starts at `pos`, if there is one: its length.
    pub(crate) fn match_at(&self, pos: usize) -> Option<usize> {
        let matcher = self.query()?;
        let doc = self.doc();
        let (line, col) = doc.line_col(pos);
        let start = doc.line_start(line);
        matcher
            .line(&doc.line_chars(line), start)
            .into_iter()
            .find(|&(m, _)| m == col)
            .map(|(_, len)| len)
    }

    /// Find in Selection: searches only the selection, or everything
    /// again.
    pub(crate) fn toggle_in_selection(&mut self) {
        let selection = self.doc().selection();
        let Some(find) = &mut self.find else { return };
        find.scope = match find.scope {
            Some(_) => None,
            None => selection,
        };
        let origin = find.scope.map_or(find.origin, |(a, _)| a);
        self.find_from(origin);
    }

    /// Moves to the first match at or after `from`, wrapping around. It
    /// searches forward from there and stops at the first, skipping the
    /// blocks of lines the counts say have none.
    pub(crate) fn find_from(&mut self, from: usize) {
        self.seek(from, true);
    }

    /// Moves to the last match before `before`, wrapping around.
    pub(crate) fn find_before(&mut self, before: usize) {
        self.seek(before, false);
    }

    /// Moves to the next match from `at`, or the previous. While a large
    /// file is counted in the background, only so much of what isn't
    /// counted yet is searched at once, so typing never waits on a query
    /// that matches nothing: it's tried again as the count comes in.
    fn seek(&mut self, at: usize, forward: bool) {
        if self.large().is_some() {
            return self.seek_large(at, forward);
        }
        let Some(matcher) = self.query() else {
            return;
        };
        let doc = self.doc();
        let found = doc.found(&matcher);
        let rope = doc.buffer.rope();
        let seek = match forward {
            true => matcher.seek_next(rope, at, found.as_deref(), SEEK_BUDGET, |_| true),
            false => matcher.seek_previous(rope, at, found.as_deref(), SEEK_BUDGET),
        };
        drop(found);
        if let Some(find) = &mut self.find {
            find.waiting = (seek == crate::search::Seek::Later).then_some((at, forward));
        }
        match seek {
            crate::search::Seek::Hit(pos, _) => self.go_to_match(Some(pos)),
            crate::search::Seek::None => self.go_to_match(None),
            crate::search::Seek::Later => {}
        }
    }

    /// More of the count came in: a search that waited for it, again.
    pub(crate) fn seek_again(&mut self) {
        if let Some((at, forward)) = self.find.as_ref().and_then(|f| f.waiting) {
            self.seek(at, forward);
        }
    }

    pub(crate) fn go_to_match(&mut self, hit: Option<usize>) {
        match hit {
            Some(pos) => self.set_cursor(pos),
            None => {
                let query = self.find.as_ref().map_or("", |f| f.query.as_str());
                self.error(format!("No match for {query}"));
            }
        }
    }

    /// Replaces the match at the cursor, if there is one, and moves to the
    /// next.
    pub(crate) fn replace_one(&mut self) {
        let Some(matcher) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let cursor = self.cursor();
        if let Some(len) = self.match_at(cursor) {
            let with = self.replacement(&matcher, cursor, &with);
            self.replace(cursor, cursor + len, &with, Kind::Other);
        }
        self.find_from(self.cursor());
    }

    /// What the match at `at` becomes.
    fn replacement(&self, matcher: &Matcher, at: usize, with: &str) -> String {
        let doc = self.doc();
        let (line, col) = doc.line_col(at);
        matcher.replacement(&doc.line_chars(line), col, with)
    }

    pub(crate) fn replace_all(&mut self) {
        let Some(matcher) = self.query() else {
            return;
        };
        let with = self
            .find
            .as_ref()
            .and_then(|f| f.replace.clone())
            .unwrap_or_default();
        let doc = self.doc();
        let edits: Vec<Edit> = self
            .all_matches(&matcher)
            .into_iter()
            .map(|(at, len)| Edit {
                at,
                removed: doc.slice(at, at + len),
                inserted: self.replacement(&matcher, at, &with),
            })
            .collect();
        match edits.len() {
            0 => {
                let query = self.find.as_ref().map_or("", |f| f.query.as_str());
                self.error(format!("No match for {query}"));
            }
            n => {
                self.apply_all(edits);
                self.notify(format!("Replaced {n}"));
            }
        }
    }

    pub(crate) fn find_view(&self) -> Option<FindView> {
        let find = self.find.as_ref()?;
        let matcher = self.matcher();
        let invalid = matcher.is_err();
        let doc = self.doc();
        let at = doc.selection().map(|(start, _)| start);
        let large = doc.large.as_deref().and_then(|l| l.found.as_ref());
        let (current, total, ignore_case, counting) = match matcher.ok().flatten() {
            // A large file's count, read from disk in the background.
            Some(m) if doc.large.is_some() => match large.filter(|f| f.matcher == m) {
                Some(found) => {
                    let l = doc.large.as_deref().expect("checked");
                    let (total, complete) = found.total(&l.file, &l.text);
                    let counting = (!complete).then(|| (found.progress() * 100.0) as u8);
                    (None, total as usize, m.folds(), counting)
                }
                None => (None, 0, m.folds(), Some(0)),
            },
            Some(m) => match doc.found(&m) {
                Some(found) => {
                    let rope = doc.buffer.rope();
                    let current = at.and_then(|a| found.index_of(rope, a));
                    let counting = (!found.complete()).then(|| (found.progress() * 100.0) as u8);
                    (current, found.total(), m.folds(), counting)
                }
                None => (None, 0, m.folds(), None),
            },
            None => (None, 0, !find.match_case, None),
        };
        Some(FindView {
            query: find.query.clone(),
            replace: find.replace.clone(),
            in_replace: find.in_replace,
            focused: self.focus == Focus::Find,
            current,
            total,
            ignore_case,
            match_case: find.match_case,
            whole_word: find.whole_word,
            regex: find.regex,
            in_selection: find.scope.is_some(),
            invalid,
            counting,
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
            FindAction::MatchCase => {
                find.match_case = !find.match_case;
                self.find_again();
            }
            FindAction::WholeWord => {
                find.whole_word = !find.whole_word;
                self.find_again();
            }
            FindAction::Regex => {
                find.regex = !find.regex;
                self.find_again();
            }
            FindAction::InSelection => self.toggle_in_selection(),
        }
    }

    /// Looks again from where find started, after a toggle changed.
    fn find_again(&mut self) {
        self.focus = Focus::Find;
        if let Some(origin) = self.find.as_ref().map(|f| f.origin) {
            self.find_from(origin);
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
    MatchCase,
    WholeWord,
    Regex,
    InSelection,
}
