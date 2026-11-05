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

/// How the find box matches text, from its query and toggles.
pub(crate) struct Matcher {
    pattern: Pattern,
    whole_word: bool,
    /// Only matches inside this range count.
    scope: Option<(usize, usize)>,
}

enum Pattern {
    Plain { query: Vec<char>, fold: bool },
    Regex(regex::Regex),
}

impl Matcher {
    /// Exactly `needle`, case and all.
    pub(crate) fn exact(needle: &[char]) -> Self {
        Self {
            pattern: Pattern::Plain {
                query: needle.to_vec(),
                fold: false,
            },
            whole_word: false,
            scope: None,
        }
    }

    /// Whether case is ignored.
    pub(crate) fn folds(&self) -> bool {
        match &self.pattern {
            Pattern::Plain { fold, .. } => *fold,
            Pattern::Regex(re) => re.as_str().starts_with("(?i)"),
        }
    }

    /// The matches in a line's characters, as where each starts and how
    /// long it is, without overlaps. `start` is where the line starts in
    /// the document, for the scope.
    pub(crate) fn line(&self, chars: &[char], start: usize) -> Vec<(usize, usize)> {
        let mut found: Vec<(usize, usize)> = match &self.pattern {
            Pattern::Plain { query, fold } => line_matches(chars, query, *fold)
                .into_iter()
                .map(|m| (m, query.len()))
                .filter(|&(m, len)| !self.whole_word || whole_word(chars, m, len))
                .collect(),
            Pattern::Regex(re) => {
                let text: String = chars.iter().collect();
                let col = char_cols(&text);
                re.find_iter(&text)
                    .filter(|m| !m.is_empty())
                    .map(|m| (col[m.start()], col[m.end()] - col[m.start()]))
                    .collect()
            }
        };
        if let Some((a, b)) = self.scope {
            found.retain(|&(m, len)| start + m >= a && start + m + len <= b);
        }
        found
    }

    /// What the match at `at` in `chars` is replaced with: `with`, and for
    /// a regular expression with `$1` and the like filled in.
    pub(crate) fn replacement(&self, chars: &[char], at: usize, with: &str) -> String {
        let Pattern::Regex(re) = &self.pattern else {
            return with.to_string();
        };
        let text: String = chars.iter().collect();
        let byte = text.char_indices().nth(at).map_or(text.len(), |(b, _)| b);
        let mut out = String::new();
        if let Some(caps) = re.captures_at(&text, byte) {
            caps.expand(with, &mut out);
        }
        out
    }
}

/// For each byte offset of `text` that starts a character (and its end),
/// which character that is.
fn char_cols(text: &str) -> Vec<usize> {
    let mut cols = vec![0; text.len() + 1];
    let mut n = 0;
    for (b, c) in text.char_indices() {
        cols[b..b + c.len_utf8()].fill(n);
        n += 1;
    }
    cols[text.len()] = n;
    cols
}

/// Whether the `len` characters at `m` are a whole word: no word
/// character just before or after.
fn whole_word(chars: &[char], m: usize, len: usize) -> bool {
    let word = |c: &char| c.is_alphanumeric() || *c == '_';
    !(m > 0 && chars.get(m - 1).is_some_and(word)) && !chars.get(m + len).is_some_and(word)
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
        self.matcher().ok().flatten()
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
        let pattern = if find.regex {
            let mut source = find.query.clone();
            if find.whole_word {
                source = format!(r"\b(?:{source})\b");
            }
            if fold {
                source = format!("(?i){source}");
            }
            Pattern::Regex(regex::Regex::new(&source)?)
        } else {
            Pattern::Plain {
                query: find.query.chars().collect(),
                fold,
            }
        };
        Ok(Some(Matcher {
            pattern,
            whole_word: find.whole_word && !find.regex,
            scope: find.scope,
        }))
    }

    /// Every match in the document, left to right, as where each starts
    /// and how long it is.
    pub(crate) fn all_matches(&self, matcher: &Matcher) -> Vec<(usize, usize)> {
        let doc = self.doc();
        let lines = match matcher.scope {
            Some((a, b)) => doc.line_col(a).0..=doc.line_col(b).0,
            None => 0..=doc.last_line(),
        };
        lines
            .flat_map(|line| {
                let start = doc.line_start(line);
                matcher
                    .line(&doc.line_chars(line), start)
                    .into_iter()
                    .map(move |(m, len)| (start + m, len))
            })
            .collect()
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

    /// Moves to the first match at or after `from`, wrapping around. Matches
    /// come from the current text every time, so edits can't leave stale ones.
    pub(crate) fn find_from(&mut self, from: usize) {
        let Some(matcher) = self.query() else {
            return;
        };
        let matches = self.all_matches(&matcher);
        let hit = matches.iter().find(|m| m.0 >= from).or(matches.first());
        self.go_to_match(hit.map(|m| m.0));
    }

    /// Moves to the last match before `before`, wrapping around.
    pub(crate) fn find_before(&mut self, before: usize) {
        let Some(matcher) = self.query() else {
            return;
        };
        let matches = self.all_matches(&matcher);
        let hit = matches
            .iter()
            .rev()
            .find(|m| m.0 < before)
            .or(matches.last());
        self.go_to_match(hit.map(|m| m.0));
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
        let (matches, ignore_case) = match matcher.ok().flatten() {
            Some(m) => (self.all_matches(&m), m.folds()),
            None => (Vec::new(), !find.match_case),
        };
        let at = self.doc().selection().map(|(start, _)| start);
        Some(FindView {
            query: find.query.clone(),
            replace: find.replace.clone(),
            in_replace: find.in_replace,
            focused: self.focus == Focus::Find,
            current: at
                .and_then(|a| matches.iter().position(|&m| m.0 == a))
                .map(|i| i + 1),
            total: matches.len(),
            ignore_case,
            match_case: find.match_case,
            whole_word: find.whole_word,
            regex: find.regex,
            in_selection: find.scope.is_some(),
            invalid,
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
