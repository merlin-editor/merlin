//! Vim's command line under the text: `:` for commands, with Vim's own
//! and merlin's by name, `:s` shown as it's typed, and `/` and `?` to
//! search, with matches tinted as the find bar tints them.

use super::*;

/// The line being typed.
#[derive(Debug, Default)]
pub(crate) struct Line {
    /// `:`, `/` or `?`.
    kind: char,
    text: String,
    /// In characters.
    cursor: usize,
    /// Going back through the earlier lines with ↑, and what was typed
    /// before.
    back: Option<(usize, String)>,
    /// The suggestion chosen with Tab.
    selected: usize,
    /// Where the cursor was, to go back to when a search is left.
    origin: usize,
    scroll: usize,
}

/// The line, for the window to draw under the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimLineView {
    /// `:` or `/`, and a range already there such as `'<,'>`.
    pub prefix: String,
    pub text: String,
    pub cursor: usize,
    /// On the right: what Enter does, a count of matches, or what's wrong.
    pub hint: String,
    pub error: bool,
    pub items: Vec<VimLineItem>,
    pub selected: usize,
}

/// A suggestion in the card over the line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimLineItem {
    pub name: String,
    pub about: String,
    /// One of merlin's commands rather than Vim's.
    pub merlin: bool,
}

/// Vim's commands the line knows, by their short name and full one.
const COMMANDS: &[(&str, &str, &str)] = &[
    ("w", "write", "Save"),
    ("wa", "wall", "Save every file"),
    ("q", "quit", "Close the file; q! without saving"),
    ("qa", "qall", "Quit merlin; qa! without saving"),
    ("wq", "wq", "Save and close"),
    ("x", "xit", "Save if changed, and close"),
    ("wqa", "wqall", "Save every file and quit"),
    ("e", "edit", "Open a file"),
    ("s", "substitute", "Replace in the line"),
    ("g", "global", "Run a command on the lines that match"),
    (
        "v",
        "vglobal",
        "Run a command on the lines that don't match",
    ),
    (
        "sort",
        "sort",
        "Sort the lines; ! reverses, u keeps one of each, n by number",
    ),
    ("noh", "nohlsearch", "Stop tinting the search's matches"),
    ("d", "delete", "Delete the lines"),
    ("y", "yank", "Yank the lines"),
    ("m", "move", "Move the lines below another"),
    ("t", "copy", "Copy the lines below another"),
    ("j", "join", "Join the lines"),
    ("norm", "normal", "Type Normal mode's keys on each line"),
    ("set", "set", "rnu, nornu, wrap or nowrap"),
    ("sp", "split", "Split the editor down"),
    ("vs", "vsplit", "Split the editor right"),
    ("bn", "bnext", "The next file"),
    ("bp", "bprevious", "The file before"),
    ("bd", "bdelete", "Close the file"),
    ("term", "terminal", "A new terminal"),
];

/// Which command a name is: its short name, from any start of its full
/// name at least as long as the short one, as Vim takes `:noh` or
/// `:nohl`.
fn command(name: &str) -> Option<&'static str> {
    COMMANDS
        .iter()
        .find(|(short, full, _)| {
            name == *short || (name.len() >= short.len() && full.starts_with(name))
        })
        .map(|(short, ..)| *short)
}

/// A pattern as Vim writes it, as a regular expression. Lower case only
/// matches either case, as merlin's find does; `\c` and `\C` say.
pub(crate) fn vim_regex(pattern: &str) -> Result<regex::Regex, String> {
    let mut out = String::new();
    let mut fold = !pattern.chars().any(char::is_uppercase);
    let mut very_magic = false;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if very_magic {
            if c == '<' || c == '>' {
                out.push_str(r"\b");
            } else {
                out.push(c);
            }
            continue;
        }
        match c {
            '\\' => match chars.next() {
                Some('<' | '>') => out.push_str(r"\b"),
                Some(c @ ('(' | ')' | '|' | '+' | '?' | '{' | '}')) => out.push(c),
                Some('=') => out.push('?'),
                Some('c') => fold = true,
                Some('C') => fold = false,
                Some('v') => very_magic = true,
                Some(
                    c @ ('s' | 'S' | 'd' | 'D' | 'w' | 'W' | 'n' | 't' | '.' | '*' | '[' | ']'
                    | '^' | '$' | '\\' | '/'),
                ) => {
                    out.push('\\');
                    out.push(c);
                }
                Some(c) => out.push_str(&regex::escape(&c.to_string())),
                None => out.push_str(r"\\"),
            },
            '(' | ')' | '|' | '+' | '?' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    let source = if fold { format!("(?i){out}") } else { out };
    regex::Regex::new(&source).map_err(|_| format!("Can't search for {pattern}"))
}

/// A replacement as Vim writes it, as the regex crate's: `&` and `\1`
/// for what matched, `\r` for a line break.
fn vim_replacement(rep: &str) -> String {
    let mut out = String::new();
    let mut chars = rep.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => out.push_str(&format!("${{{d}}}")),
                Some('r' | 'n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('&') => out.push('&'),
                Some('$') => out.push_str("$$"),
                Some(other) => out.push(other),
                None => out.push('\\'),
            },
            '&' => out.push_str("${0}"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out
}

/// `s/pattern/replacement/flags`, with any delimiter: the three parts.
fn split_substitute(args: &str) -> Option<(String, String, String)> {
    let mut chars = args.chars();
    let delim = chars.next()?;
    if delim.is_alphanumeric() || delim == '\\' || delim == ' ' {
        return None;
    }
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for c in chars {
        if escaped {
            if c != delim {
                parts.last_mut()?.push('\\');
            }
            parts.last_mut()?.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delim && parts.len() < 3 {
            parts.push(String::new());
        } else {
            parts.last_mut()?.push(c);
        }
    }
    if escaped {
        parts.last_mut()?.push('\\');
    }
    while parts.len() < 3 {
        parts.push(String::new());
    }
    let flags = parts.pop()?;
    let rep = parts.pop()?;
    let pat = parts.pop()?;
    Some((pat, rep, flags))
}

/// A `:` line taken apart: the range's text, the command and `!`, and the
/// rest.
struct Ex<'a> {
    range: &'a str,
    name: &'a str,
    bang: bool,
    args: &'a str,
}

fn split_ex(text: &str) -> Ex<'_> {
    let text = text.trim_start_matches([' ', ':']);
    // The range: numbers, `.`, `$`, `%`, marks, `+`, `-`, `,` and `;`.
    let mut end = 0;
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_digit() || ".$%+-,; ".contains(c) {
            i += 1;
        } else if c == '\'' && i + 1 < bytes.len() {
            i += 2;
        } else {
            break;
        }
        end = i;
    }
    let byte_end: usize = bytes[..end].iter().map(|c| c.len_utf8()).sum();
    let (range, rest) = text.split_at(byte_end);
    let rest = rest.trim_start();
    // Letters make a name; otherwise one sign such as `&`, `<` or `>`.
    let name_len = rest
        .char_indices()
        .take_while(|(_, c)| c.is_ascii_alphabetic())
        .last()
        .map_or_else(
            || {
                rest.chars()
                    .next()
                    .filter(|c| "&<>".contains(*c))
                    .map_or(0, |c| c.len_utf8())
            },
            |(i, c)| i + c.len_utf8(),
        );
    // `s`, `g` and `v` run straight into their pattern: `s/a/b/`, `g!/a/d`.
    let next = rest.get(1..).and_then(|r| r.chars().next());
    let runs_in = (rest.starts_with('s')
        && next.is_some_and(|c| !c.is_alphanumeric() && c != ' ' && c != '!'))
        || ((rest.starts_with('g') || rest.starts_with('v'))
            && next.is_some_and(|c| "/!#".contains(c)));
    let name_len = if runs_in { 1 } else { name_len };
    let (name, rest) = rest.split_at(name_len);
    let (bang, args) = match rest.strip_prefix('!') {
        Some(args) => (true, args),
        None => (false, rest),
    };
    Ex {
        range: range.trim(),
        name,
        bang,
        args: args.trim_start(),
    }
}

impl Editor {
    /// Opens the line: `:` with `text` already in it, or `/` and `?`.
    pub(super) fn vim_open_line(&mut self, kind: char, text: &str) {
        let (origin, scroll) = (self.doc().cursor, self.doc().scroll);
        let visual = self.vim_mode().filter(|m| m.visual());
        let origin = match visual {
            Some(_) => self.vim.as_ref().map_or(origin, |v| v.cursor),
            None => origin,
        };
        let vim = self.vim_mut();
        vim.line = Some(Line {
            kind,
            text: text.to_string(),
            cursor: text.chars().count(),
            origin,
            scroll,
            ..Default::default()
        });
    }

    fn vim_close_line(&mut self) -> Option<Line> {
        self.vim.as_mut().and_then(|v| v.line.take())
    }

    /// A key while the line is open: it takes them all.
    pub(super) fn vim_line_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        use KeyCode::*;
        let Some(line) = self.vim.as_mut().and_then(|v| v.line.as_mut()) else {
            return;
        };
        let at = line
            .text
            .char_indices()
            .nth(line.cursor)
            .map_or(line.text.len(), |(i, _)| i);
        let mut edited = true;
        match (key.code, key.ctrl) {
            (Esc, _) | (Char('['), true) | (Char('c'), true) => {
                let line = self.vim_close_line().expect("line");
                if line.kind != ':' {
                    self.vim_search_back(&line);
                }
                return;
            }
            (Enter, _) => {
                let items = self.vim_line_items();
                let line = self.vim_close_line().expect("line");
                return self.vim_line_run(line, items, fx);
            }
            (Backspace, false) if line.text.is_empty() => {
                let line = self.vim_close_line().expect("line");
                if line.kind != ':' {
                    self.vim_search_back(&line);
                }
                return;
            }
            (Backspace, false) | (Char('h'), true) => {
                if line.cursor > 0 {
                    let prev = line.text[..at].char_indices().last().map_or(0, |(i, _)| i);
                    line.text.replace_range(prev..at, "");
                    line.cursor -= 1;
                }
            }
            (Delete, _) => {
                if at < line.text.len() {
                    let next = line.text[at..].chars().next().map_or(0, |c| c.len_utf8());
                    line.text.replace_range(at..at + next, "");
                }
            }
            (Char('w'), true) | (Backspace, true) => {
                let before: Vec<char> = line.text[..at].chars().collect();
                let mut i = before.len();
                while i > 0 && before[i - 1] == ' ' {
                    i -= 1;
                }
                while i > 0 && before[i - 1] != ' ' {
                    i -= 1;
                }
                let from: usize = before[..i].iter().map(|c| c.len_utf8()).sum();
                line.text.replace_range(from..at, "");
                line.cursor = i;
            }
            (Char('u'), true) => {
                line.text.replace_range(..at, "");
                line.cursor = 0;
            }
            (Left, _) => {
                line.cursor = line.cursor.saturating_sub(1);
                edited = false;
            }
            (Right, _) => {
                line.cursor = (line.cursor + 1).min(line.text.chars().count());
                edited = false;
            }
            (Home, _) | (Char('b'), true) => {
                line.cursor = 0;
                edited = false;
            }
            (End, _) | (Char('e'), true) => {
                line.cursor = line.text.chars().count();
                edited = false;
            }
            (Up | Down, _) => {
                let up = key.code == Up;
                let vim = self.vim_mut();
                let line = vim.line.as_mut().expect("line");
                let history = if line.kind == ':' {
                    &vim.history
                } else {
                    &vim.searches
                };
                let (from, typed) = line
                    .back
                    .clone()
                    .unwrap_or((history.len(), line.text.clone()));
                // The earlier lines that start as what was typed.
                let found = if up {
                    (0..from).rev().find(|&i| history[i].starts_with(&typed))
                } else {
                    (from + 1..history.len()).find(|&i| history[i].starts_with(&typed))
                };
                match found {
                    Some(i) => {
                        line.text = history[i].clone();
                        line.back = Some((i, typed));
                    }
                    None if !up => {
                        line.text = typed;
                        line.back = None;
                    }
                    None => {}
                }
                line.cursor = line.text.chars().count();
            }
            (Tab | BackTab, _) => {
                let n = self.vim_line_items().len();
                let line = self
                    .vim
                    .as_mut()
                    .and_then(|v| v.line.as_mut())
                    .expect("line");
                if n > 0 {
                    line.selected = if key.code == Tab && !key.shift {
                        (line.selected + 1) % n
                    } else {
                        (line.selected + n - 1) % n
                    };
                }
                edited = false;
            }
            (Char(c), false) if !key.alt => {
                line.text.insert(at, c);
                line.cursor += 1;
            }
            _ => edited = false,
        }
        if edited {
            let line = self
                .vim
                .as_mut()
                .and_then(|v| v.line.as_mut())
                .expect("line");
            line.selected = 0;
            line.back = None;
            if line.kind != ':' {
                self.vim_search_as_typed();
            }
        }
    }

    /// A search moves to its first match as it's typed.
    fn vim_search_as_typed(&mut self) {
        let Some(line) = self.vim.as_ref().and_then(|v| v.line.as_ref()) else {
            return;
        };
        let (text, forward, origin, scroll) = (
            line.text.clone(),
            line.kind == '/',
            line.origin,
            line.scroll,
        );
        let found = vim_regex(&text)
            .ok()
            .filter(|_| !text.is_empty())
            .and_then(|re| search_regex(self.doc(), origin, &re, forward, 1));
        self.vim_search_cursor(found.unwrap_or(origin));
        if found.is_none() {
            self.doc_mut().scroll = scroll;
        }
    }

    fn vim_search_cursor(&mut self, at: usize) {
        let visual = self.vim_mode().is_some_and(|m| m.visual());
        if visual {
            self.vim_mut().cursor = at;
            self.vim_show_visual();
        } else {
            let doc = self.doc_mut();
            doc.cursor = at;
            doc.anchor = None;
        }
    }

    /// A search left with Esc: the cursor goes back.
    fn vim_search_back(&mut self, line: &Line) {
        self.vim_search_cursor(line.origin);
        self.doc_mut().scroll = line.scroll;
    }

    /// What the window draws for the line, while it's open.
    pub(crate) fn vim_line_view(&self) -> Option<VimLineView> {
        let line = self.vim.as_ref()?.line.as_ref()?;
        let items = self.vim_line_items();
        let (hint, error) = if line.kind == ':' {
            self.vim_line_hint(&line.text)
        } else if line.text.is_empty() {
            (
                "Enter searches · Esc leaves · ↑ the last ones".into(),
                false,
            )
        } else {
            match vim_regex(&line.text) {
                Err(e) => (e, true),
                Ok(re) => match self.vim_match_count(&re) {
                    (_, 0) => ("Not found".into(), true),
                    (at, n) => (
                        format!("{at} of {n} · Enter goes there · Esc goes back"),
                        false,
                    ),
                },
            }
        };
        Some(VimLineView {
            prefix: line.kind.to_string(),
            text: line.text.clone(),
            cursor: line.cursor,
            hint,
            error,
            selected: line.selected.min(items.len().saturating_sub(1)),
            items,
        })
    }

    /// Which match the cursor is on, counted from the top, and how many
    /// there are.
    fn vim_match_count(&self, re: &regex::Regex) -> (usize, usize) {
        let doc = self.doc();
        let text = doc.buffer.text();
        let rope = doc.buffer.rope();
        let at = self.vim_visual_cursor().unwrap_or(doc.cursor);
        let mut n = 0;
        let mut here = 0;
        for m in re.find_iter(&text).take(100_000) {
            n += 1;
            if rope.byte_to_char(m.start()) <= at {
                here = n;
            }
        }
        (here.max(1), n)
    }

    /// The right of the line while typing a command.
    fn vim_line_hint(&self, text: &str) -> (String, bool) {
        let ex = split_ex(text);
        if ex.name == "s" || ex.name == "&" {
            return match self.vim_substitute_plan(&ex) {
                Ok((edits, lines)) => {
                    let n = edits.len();
                    if n == 0 {
                        ("Nothing matches · Esc leaves".into(), false)
                    } else {
                        let s = if lines == 1 { "" } else { "s" };
                        (
                            format!(
                                "{n} in {lines} line{s} · Enter replaces · Esc leaves · ↑ the last ones"
                            ),
                            false,
                        )
                    }
                }
                Err(e) => (e, !text.trim().ends_with(['s', '/'])),
            };
        }
        ("Enter runs · Esc leaves · ↑ the last ones".into(), false)
    }

    /// The suggestions over the line: Vim's commands that start as typed,
    /// then merlin's that match.
    pub(crate) fn vim_line_items(&self) -> Vec<VimLineItem> {
        let Some(line) = self.vim.as_ref().and_then(|v| v.line.as_ref()) else {
            return Vec::new();
        };
        if line.kind != ':' {
            return Vec::new();
        }
        let ex = split_ex(&line.text);
        let word = ex.name;
        if word.is_empty() || !ex.args.is_empty() || ex.bang || line.text.ends_with(' ') {
            return Vec::new();
        }
        let ranged = !ex.range.is_empty();
        let mut items: Vec<VimLineItem> = COMMANDS
            .iter()
            .filter(|(short, full, _)| short.starts_with(word) || full.starts_with(word))
            .map(|(short, _, about)| VimLineItem {
                name: format!(":{short}"),
                about: match (*short, ranged) {
                    ("s", true) if ex.range == "'<,'>" => "Replace in the selection".into(),
                    ("s", _) => "Replace in the line · :%s for the file".into(),
                    _ => about.to_string(),
                },
                merlin: false,
            })
            .take(5)
            .collect();
        if !ranged && word.len() >= 2 {
            let mut found: Vec<(i64, String)> = self
                .commands()
                .into_iter()
                .filter_map(|(_, title)| {
                    let name = title.split_once(": ").map_or(title.as_str(), |(_, n)| n);
                    crate::fuzzy::score(word, name).map(|s| (s, title.clone()))
                })
                .collect();
            found.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
            for (_, title) in found.into_iter().take(8 - items.len().min(4)) {
                let (category, name) = title.split_once(": ").unwrap_or(("", title.as_str()));
                items.push(VimLineItem {
                    name: name.to_string(),
                    about: category.to_string(),
                    merlin: true,
                });
            }
        }
        items
    }

    /// Enter on the line.
    fn vim_line_run(&mut self, line: Line, items: Vec<VimLineItem>, fx: &mut Vec<Effect>) {
        let vim = self.vim_mut();
        if line.kind != ':' {
            if line.text.is_empty() {
                // `/` and Enter searches for the last one again.
                if let Some((pattern, _)) = vim.search.clone() {
                    vim.search = Some((pattern, line.kind == '/'));
                }
            } else {
                vim.searches.retain(|s| *s != line.text);
                vim.searches.push(line.text.clone());
                vim.search = Some((line.text.clone(), line.kind == '/'));
            }
            vim.highlight = true;
            let Some((pattern, forward)) = vim.search.clone() else {
                return;
            };
            let found = vim_regex(&pattern)
                .ok()
                .and_then(|re| search_regex(self.doc(), line.origin, &re, forward, 1));
            match found {
                Some(at) => {
                    let id = self.doc().id;
                    self.vim
                        .as_mut()
                        .expect("vim")
                        .marks
                        .insert('\'', (id, line.origin));
                    self.vim_search_cursor(at);
                    self.vim_count_note();
                }
                None => {
                    self.vim_search_back(&line);
                    self.vim_note(format!("Not found: {pattern}"));
                }
            }
            return;
        }
        if !line.text.trim().is_empty() {
            vim.history.retain(|s| *s != line.text);
            vim.history.push(line.text.clone());
        }
        let steps = self.doc().buffer.undo_steps();
        self.vim_ex(&line.text, items.get(line.selected), fx);
        self.doc_mut().buffer.join_undo(steps);
        self.vim_clamp();
    }

    pub(super) fn vim_note(&mut self, note: String) {
        if let Some(vim) = self.vim.as_mut() {
            vim.note = Some(note);
        }
    }

    /// "3 of 12" in the status bar, after a search moves.
    pub(super) fn vim_count_note(&mut self) {
        let Some((pattern, _)) = self.vim.as_ref().and_then(|v| v.search.clone()) else {
            return;
        };
        if let Ok(re) = vim_regex(&pattern) {
            let (at, n) = self.vim_match_count(&re);
            self.vim_note(format!("{at} of {n}"));
        }
    }

    /// The search's matches, for the text to tint while it's on.
    pub(crate) fn vim_matcher(&self) -> Option<super::super::find::Matcher> {
        let vim = self.vim.as_ref()?;
        let pattern = match vim.line.as_ref() {
            Some(line) if line.kind != ':' && !line.text.is_empty() => line.text.clone(),
            // `:s` shows its own.
            Some(line) if line.kind == ':' => return None,
            _ if vim.highlight => vim.search.as_ref()?.0.clone(),
            _ => return None,
        };
        vim_regex(&pattern)
            .ok()
            .map(super::super::find::Matcher::regex)
    }

    /// Lines from a range such as `%`, `.,+3`, `'<,'>` or `5`, counted
    /// from 0; the cursor's line without one.
    fn vim_range(&self, range: &str) -> Result<(usize, usize), String> {
        let doc = self.doc();
        let cur = doc.line_col(doc.cursor).0;
        let last = doc.last_line();
        let range = range.trim();
        if range.is_empty() {
            return Ok((cur, cur));
        }
        if range == "%" {
            return Ok((0, last));
        }
        let address = |text: &str| -> Result<usize, String> {
            let text = text.trim();
            let mut chars = text.chars().peekable();
            let mut line = match chars.peek() {
                Some('.') => {
                    chars.next();
                    cur as i64
                }
                Some('$') => {
                    chars.next();
                    last as i64
                }
                Some('\'') => {
                    chars.next();
                    let mark = chars.next().ok_or("A mark needs its letter")?;
                    let vim = self.vim.as_ref().expect("vim");
                    let &(id, at) = vim
                        .marks
                        .get(&mark)
                        .filter(|(id, _)| *id == doc.id)
                        .ok_or(format!("No mark {mark} here"))?;
                    debug_assert_eq!(id, doc.id);
                    doc.line_col(at.min(doc.buffer.len())).0 as i64
                }
                Some(c) if c.is_ascii_digit() => {
                    let digits: String =
                        std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
                    digits.parse::<i64>().unwrap_or(1) - 1
                }
                _ => cur as i64,
            };
            // Offsets after: `+3`, `-`, `+`.
            while let Some(sign) = chars.next() {
                let digits: String =
                    std::iter::from_fn(|| chars.next_if(|c| c.is_ascii_digit())).collect();
                let n = if digits.is_empty() {
                    1
                } else {
                    digits.parse::<i64>().unwrap_or(1)
                };
                match sign {
                    '+' => line += n,
                    '-' => line -= n,
                    ' ' => {}
                    _ => return Err(format!("Can't read the range {range}")),
                }
            }
            Ok(line.clamp(0, last as i64) as usize)
        };
        let (a, b) = match range.split_once([',', ';']) {
            Some((a, b)) => (address(a)?, address(b)?),
            None => {
                let a = address(range)?;
                (a, a)
            }
        };
        Ok((a.min(b), a.max(b)))
    }

    /// What `:s` would change: the edits, and on how many lines.
    fn vim_substitute_plan(&self, ex: &Ex) -> Result<(Vec<Edit>, usize), String> {
        let (pat, rep, flags) = if ex.name == "&" {
            let (p, r) = self
                .vim
                .as_ref()
                .and_then(|v| v.last_substitute.clone())
                .ok_or("No substitution to repeat")?;
            (p, r, ex.args.to_string())
        } else {
            split_substitute(ex.args).ok_or("Write it as s/this/that/")?
        };
        let pat = if pat.is_empty() {
            self.vim
                .as_ref()
                .and_then(|v| v.search.as_ref().map(|s| s.0.clone()))
                .ok_or("No search to use")?
        } else {
            pat
        };
        let mut pattern = pat.clone();
        if flags.contains('i') {
            pattern.insert_str(0, r"\c");
        } else if flags.contains('I') {
            pattern.insert_str(0, r"\C");
        }
        let re = vim_regex(&pattern)?;
        let rep = vim_replacement(&rep);
        let all = flags.contains('g');
        let (a, b) = self.vim_range(ex.range)?;
        let doc = self.doc();
        let mut edits = Vec::new();
        let mut lines = 0;
        for line in a..=b {
            let text: String = doc.line_chars(line).into_iter().collect();
            let start = doc.line_start(line);
            let mut any = false;
            for caps in re.captures_iter(&text) {
                let m = caps.get(0).expect("a match");
                let mut with = String::new();
                caps.expand(&rep, &mut with);
                let at = start + text[..m.start()].chars().count();
                edits.push(Edit {
                    at,
                    removed: m.as_str().to_string(),
                    inserted: with,
                });
                any = true;
                if !all {
                    break;
                }
            }
            lines += any as usize;
        }
        Ok((edits, lines))
    }

    /// The `:s` being typed, for the text to show: what goes, struck
    /// through, and what comes, after it.
    pub(crate) fn vim_preview(&self) -> Vec<Edit> {
        let Some(line) = self.vim.as_ref().and_then(|v| v.line.as_ref()) else {
            return Vec::new();
        };
        if line.kind != ':' {
            return Vec::new();
        }
        let ex = split_ex(&line.text);
        if ex.name != "s" {
            return Vec::new();
        }
        self.vim_substitute_plan(&ex)
            .map(|(e, _)| e)
            .unwrap_or_default()
    }

    /// Runs a `:` command.
    pub(super) fn vim_ex(
        &mut self,
        text: &str,
        chosen: Option<&VimLineItem>,
        fx: &mut Vec<Effect>,
    ) {
        let ex = split_ex(text);
        if ex.name.is_empty() && ex.range.is_empty() {
            return;
        }
        // A line number alone goes there.
        if ex.name.is_empty() {
            match self.vim_range(ex.range) {
                Ok((_, b)) => {
                    let id = self.doc().id;
                    let at = self.doc().cursor;
                    self.vim_mut().marks.insert('\'', (id, at));
                    let doc = self.doc();
                    let to = first_non_blank(doc, b);
                    self.set_cursor(to);
                }
                Err(e) => self.vim_note(e),
            }
            return;
        }
        let Some(name) = command(ex.name).or(match ex.name {
            "&" => Some("&"),
            ">" => Some(">"),
            "<" => Some("<"),
            _ => None,
        }) else {
            // One of merlin's, by name.
            if let Some(item) = chosen.filter(|i| i.merlin) {
                let title = format!("{}: {}", item.about, item.name);
                if let Some((id, _)) = self.commands().into_iter().find(|(_, t)| *t == title) {
                    return self.run_command_id(&id, fx);
                }
            }
            return self.vim_note(format!("Not a command: {}", ex.name));
        };
        let lines = self.vim_range(ex.range);
        let id = self.doc().id;
        match name {
            "w" => self.run("file.save", fx),
            "wa" => {
                let back = self.current;
                for i in 0..self.docs.len() {
                    if self.docs[i].buffer.is_dirty() && self.docs[i].buffer.path.is_some() {
                        self.current = i;
                        self.run("file.save", fx);
                    }
                }
                self.current = back.min(self.docs.len() - 1);
            }
            "q" | "bd" => self.close(ex.bang),
            "qa" => {
                if ex.bang {
                    self.quit(fx)
                } else {
                    self.run("app.quit", fx)
                }
            }
            "wq" | "x" => {
                if name == "x" && !self.doc().buffer.is_dirty() {
                    return self.close(false);
                }
                self.closing.push(id);
                self.run("file.save", fx);
            }
            "wqa" => {
                self.quitting = true;
                self.save_all_then_quit(fx);
            }
            "e" => {
                if ex.args.is_empty() {
                    return self.vim_note("Name a file: :e src/main.rs".into());
                }
                let path = std::path::Path::new(ex.args);
                let path = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    self.root.join(path)
                };
                fx.push(Effect::Open(path));
            }
            "noh" => self.vim_mut().highlight = false,
            "s" | "&" => self.ex_substitute(&ex),
            "g" | "v" => self.ex_global(&ex, name, lines, fx),
            "sort" => self.ex_sort(&ex, lines),
            "d" | "y" | ">" | "<" | "j" | "m" | "t" | "norm" => self.ex_lines(&ex, name, lines, fx),
            "set" => {
                for option in ex.args.split_whitespace() {
                    match option {
                        "rnu" | "relativenumber" => self.relative_numbers = true,
                        "nornu" | "norelativenumber" => self.relative_numbers = false,
                        "wrap" => self.wrap = true,
                        "nowrap" => self.wrap = false,
                        "nu" | "number" => {}
                        other => {
                            return self.vim_note(format!("Not an option merlin has: {other}"));
                        }
                    }
                }
            }
            "sp" => self.run("view.split_down", fx),
            "vs" => self.run("view.split", fx),
            "bn" => self.run("file.next", fx),
            "bp" => self.run("file.prev", fx),
            "term" => self.run("terminal.new", fx),
            _ => {}
        }
    }

    /// `:s` and `:&`: substitutes on the range's lines, or counts with `n`.
    fn ex_substitute(&mut self, ex: &Ex) {
        let plan = self.vim_substitute_plan(ex);
        match plan {
            Ok((edits, n_lines)) if !edits.is_empty() => {
                let count_only = split_substitute(ex.args).is_some_and(|(_, _, f)| f.contains('n'));
                let n = edits.len();
                if !count_only {
                    let last = edits.last().map_or(0, |e| e.at);
                    self.apply_all(edits);
                    let doc = self.doc();
                    let line = doc.line_col(last.min(doc.buffer.len())).0;
                    let to = first_non_blank(doc, line);
                    self.set_cursor(to);
                }
                let s = if n == 1 { "" } else { "s" };
                let l = if n_lines == 1 { "" } else { "s" };
                let what = if count_only { "match" } else { "replaced" };
                self.vim_note(format!("{n} {what}{s} on {n_lines} line{l}"));
                if let Some((pat, rep, _)) = split_substitute(ex.args) {
                    let vim = self.vim_mut();
                    if !pat.is_empty() {
                        vim.search = Some((pat.clone(), true));
                    }
                    vim.last_substitute = Some((pat, rep));
                }
            }
            Ok(_) => self.vim_note("Nothing matches".into()),
            Err(e) => self.vim_note(e),
        }
    }

    /// `:g` and `:v`: a command on each line that matches, or doesn't.
    fn ex_global(
        &mut self,
        ex: &Ex,
        name: &str,
        lines: Result<(usize, usize), String>,
        fx: &mut Vec<Effect>,
    ) {
        let keep = name == "g" && !ex.bang;
        let Some(delim) = ex.args.chars().next() else {
            return self.vim_note("Write it as g/pattern/command".into());
        };
        let mut parts = ex.args[delim.len_utf8()..].splitn(2, delim);
        let pattern = parts.next().unwrap_or_default();
        let command = parts.next().unwrap_or("p").trim();
        let re = match vim_regex(pattern) {
            Ok(re) => re,
            Err(e) => return self.vim_note(e),
        };
        let (a, b) = if ex.range.is_empty() {
            (0, self.doc().last_line())
        } else {
            match lines {
                Ok(l) => l,
                Err(e) => return self.vim_note(e),
            }
        };
        let doc = self.doc();
        let hits: Vec<usize> = (a..=b)
            .filter(|&l| re.is_match(&doc.line_chars(l).into_iter().collect::<String>()) == keep)
            .collect();
        // Bottom up, so the lines above keep their numbers.
        for &line in hits.iter().rev() {
            let n = line + 1;
            self.vim_ex(&format!("{n}{command}"), None, fx);
        }
        let n = hits.len();
        self.vim_note(format!("{n} line{}", if n == 1 { "" } else { "s" }));
    }

    /// `:sort`: the range's lines, or all of them, in order.
    fn ex_sort(&mut self, ex: &Ex, lines: Result<(usize, usize), String>) {
        let (a, b) = if ex.range.is_empty() {
            (0, self.doc().last_line())
        } else {
            match lines {
                Ok(l) => l,
                Err(e) => return self.vim_note(e),
            }
        };
        let doc = self.doc();
        let mut rows: Vec<String> = (a..=b)
            .map(|l| doc.line_chars(l).into_iter().collect())
            .collect();
        let numeric = ex.args.contains('n');
        let fold = ex.args.contains('i');
        let number = |s: &String| -> i64 {
            let digits: String = s
                .chars()
                .skip_while(|c| !c.is_ascii_digit() && *c != '-')
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            digits.parse().unwrap_or(i64::MIN)
        };
        if numeric {
            rows.sort_by_key(number);
        } else if fold {
            rows.sort_by_key(|s| s.to_lowercase());
        } else {
            rows.sort();
        }
        if ex.bang {
            rows.reverse();
        }
        if ex.args.contains('u') {
            rows.dedup_by(|x, y| {
                if fold {
                    x.to_lowercase() == y.to_lowercase()
                } else {
                    x == y
                }
            });
        }
        let (start, end) = (doc.line_start(a), line_end(doc, b));
        let with = rows.join("\n");
        if doc.slice(start, end) != with {
            self.replace(start, end, &with, Kind::Other);
        }
        self.set_cursor(start);
    }

    /// The commands on a range of lines: `:d`, `:y`, `:>`, `:<`, `:j`, `:m`,
    /// `:t` and `:norm`.
    fn ex_lines(
        &mut self,
        ex: &Ex,
        name: &str,
        lines: Result<(usize, usize), String>,
        fx: &mut Vec<Effect>,
    ) {
        let (a, b) = match lines {
            Ok(l) => l,
            Err(e) => return self.vim_note(e),
        };
        let doc = self.doc();
        let span = Span {
            start: doc.line_start(a),
            end: next_line(doc, b),
            linewise: true,
        };
        let reg = ex
            .args
            .chars()
            .next()
            .filter(|c| c.is_ascii_alphabetic() || *c == '_' || *c == '+');
        match name {
            "d" => {
                self.vim_operate(Op::Delete, span, reg, fx);
            }
            "y" => {
                let text = self.doc().slice(span.start, span.end);
                self.vim_yank(reg, text, true, false, fx);
            }
            ">" | "<" => {
                self.vim_operate(
                    if name == ">" { Op::Indent } else { Op::Outdent },
                    span,
                    None,
                    fx,
                );
            }
            "j" => {
                self.set_cursor(span.start);
                self.vim_join((b - a).max(1), !ex.bang);
            }
            "m" | "t" => {
                let target = match ex.args.trim() {
                    "0" => None,
                    to => match self.vim_range(to) {
                        Ok((_, t)) => Some(t),
                        Err(e) => return self.vim_note(e),
                    },
                };
                if name == "m" && target.is_some_and(|t| (a..=b).contains(&t)) {
                    return self.vim_note("Can't move lines into themselves".into());
                }
                let mut text = self.doc().slice(span.start, span.end);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                let doc = self.doc();
                let mut at = target.map_or(0, |t| next_line(doc, t));
                let at_end = at == doc.buffer.len() && !doc.slice(0, at).ends_with('\n') && at > 0;
                let insert = if at_end {
                    format!("\n{}", text.trim_end_matches('\n'))
                } else {
                    text.clone()
                };
                if name == "m" {
                    // Take them out first, from wherever they are.
                    if at > span.start {
                        self.vim_operate(Op::Delete, span, Some('_'), fx);
                        at -= span.end - span.start;
                    } else {
                        self.vim_operate(Op::Delete, span, Some('_'), fx);
                    }
                }
                let at = at.min(self.doc().buffer.len());
                self.replace(at, at, &insert, Kind::Other);
                let doc = self.doc();
                let line = doc.line_col(at + insert.starts_with('\n') as usize).0;
                let to = first_non_blank(doc, line.min(doc.last_line()));
                self.set_cursor(to);
            }
            _ => {
                // `:normal`: the keys on each line, top down.
                let keys: Vec<Key> = ex
                    .args
                    .chars()
                    .map(|c| Key::plain(KeyCode::Char(c)))
                    .collect();
                for line in a..=b {
                    if line > self.doc().last_line() {
                        break;
                    }
                    let start = self.doc().line_start(line);
                    self.set_cursor(start);
                    for &key in &keys {
                        if !self.vim_key(key, fx) {
                            self.editor_key(key, fx);
                        }
                    }
                    if self.vim_mode() != Some(VimMode::Normal) {
                        self.vim_key(Key::plain(KeyCode::Esc), fx);
                    }
                }
            }
        }
    }

    /// One of merlin's commands, as the palette runs them: a plugin's
    /// goes to its plugin.
    pub(crate) fn run_command_id(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let plugin = self
            .plugins
            .iter()
            .find(|p| id.strip_prefix(&p.name).is_some_and(|r| r.starts_with('.')))
            .map(|p| p.name.clone());
        match plugin {
            Some(plugin) => fx.push(Effect::PluginCommand {
                command: id[plugin.len() + 1..].to_string(),
                plugin,
            }),
            None => self.run(id, fx),
        }
    }
}

/// The `n`th match of `re` from `p`, going round the end.
pub(super) fn search_regex(
    doc: &Doc,
    p: usize,
    re: &regex::Regex,
    forward: bool,
    n: usize,
) -> Option<usize> {
    let text = doc.buffer.text();
    let rope = doc.buffer.rope();
    let hits: Vec<usize> = re
        .find_iter(&text)
        .take(100_000)
        .map(|m| rope.byte_to_char(m.start()))
        .collect();
    if hits.is_empty() {
        return None;
    }
    let mut at = p;
    for _ in 0..n {
        at = if forward {
            hits.iter().copied().find(|&h| h > at).unwrap_or(hits[0])
        } else {
            hits.iter()
                .rev()
                .copied()
                .find(|&h| h < at)
                .unwrap_or(hits[hits.len() - 1])
        };
    }
    Some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vim_patterns_become_regular_expressions() {
        let re = vim_regex(r"\<foo\(bar\)\+\>").unwrap();
        assert!(re.is_match("a foobarbar b"));
        assert!(!re.is_match("afoobar"));
        let re = vim_regex("a(b)").unwrap();
        assert!(re.is_match("A(B)"), "lower case matches either");
        assert_eq!(vim_replacement(r"<&\1>\r$"), "<${0}${1}>\n$$");
    }

    #[test]
    fn a_line_comes_apart() {
        let ex = split_ex("'<,'>s/a/b/g");
        assert_eq!((ex.range, ex.name, ex.args), ("'<,'>", "s", "/a/b/g"));
        let ex = split_ex("%sort! u");
        assert_eq!(
            (ex.range, ex.name, ex.bang, ex.args),
            ("%", "sort", true, "u")
        );
        let ex = split_ex("12");
        assert_eq!((ex.range, ex.name), ("12", ""));
        assert_eq!(command("nohl"), Some("noh"));
        assert_eq!(command("write"), Some("w"));
        assert_eq!(
            split_substitute(r"#a\#b#c#"),
            Some(("a#b".into(), "c".into(), "".into()))
        );
    }
}
