//! Vim's keys in the text: Normal, Insert, Visual and Replace modes, with
//! motions, operators, text objects, counts, registers and the dot, as Vim
//! has them. merlin's own typing does Insert mode, so completion, closing
//! brackets and several cursors work there as they do without Vim.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::doc::Doc;

mod line;
mod which;
pub use line::{VimLineItem, VimLineView};
use line::{search_regex, vim_regex};
pub use which::{LeaderKey, WhichKeyItem, WhichKeyView};

/// How keys work in the text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Keymap {
    #[default]
    VsCode,
    Vim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimMode {
    Normal,
    Insert,
    Visual,
    VisualLine,
    VisualBlock,
    Replace,
}

impl VimMode {
    pub fn label(self) -> &'static str {
        match self {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "V-LINE",
            VimMode::VisualBlock => "V-BLOCK",
            VimMode::Replace => "REPLACE",
        }
    }

    fn visual(self) -> bool {
        matches!(
            self,
            VimMode::Visual | VimMode::VisualLine | VimMode::VisualBlock
        )
    }
}

/// The cursor's shape in each mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VimCursor {
    Block,
    Line,
    Underline,
    /// An operator waiting for its motion.
    HalfBlock,
}

/// What the status bar shows: the mode and the keys typed so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimView {
    pub mode: VimMode,
    pub pending: String,
    /// What the last command said, such as "3 of 12".
    pub note: Option<String>,
}

/// The settings under `[vim]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimSettings {
    /// Two letters typed quickly that leave Insert mode, such as `jk`.
    pub insert_escape: String,
    /// y, d and p use the system clipboard, as `clipboard=unnamedplus`.
    pub clipboard: bool,
    /// Ctrl keys that do merlin's thing in every mode, such as `ctrl+p`.
    pub keep: Vec<String>,
    /// The key merlin's own commands start with: `space`, or a letter.
    pub leader: String,
    /// Milliseconds on a prefix before which-key shows; 0 never.
    pub which_key_delay: u64,
}

impl Default for VimSettings {
    fn default() -> Self {
        Self {
            insert_escape: String::new(),
            clipboard: false,
            keep: KEEP.iter().map(|k| k.to_string()).collect(),
            leader: "space".into(),
            which_key_delay: 500,
        }
    }
}

/// The Ctrl keys merlin keeps unless the settings give them to Vim.
pub const KEEP: &[&str] = &[
    "ctrl+p",
    "ctrl+shift+p",
    "ctrl+s",
    "ctrl+tab",
    "ctrl+`",
    "ctrl+c",
];

/// The Ctrl keys Vim has a use for in Normal and Visual mode.
const VIM_CTRL: &str = "rvduoeyfbaxw[";

/// How long the second letter of `jk` may take, as Vim's `timeoutlen`.
const ESCAPE_WAIT: Duration = Duration::from_millis(1000);

// Keys that aren't characters, as the parser sees them.
const ESC: char = '\x1b';
const ENTER: char = '\r';
const BACKSPACE: char = '\x7f';
const LEFT: char = '\u{e000}';
const RIGHT: char = '\u{e001}';
const UP: char = '\u{e002}';
const DOWN: char = '\u{e003}';
const HOME: char = '\u{e004}';
const END: char = '\u{e005}';
const PAGE_UP: char = '\u{e006}';
const PAGE_DOWN: char = '\u{e007}';
const DELETE: char = '\u{e008}';

const fn ctrl(c: char) -> char {
    (c as u8 - b'a' + 1) as char
}

/// One-key motions.
const MOTIONS: &str =
    "hjklwWbBeE0^$GHML{}%nN*#;,|_+-\r \x7f\u{e000}\u{e001}\u{e002}\u{e003}\u{e004}\u{e005}";

#[derive(Clone, Debug, Default)]
struct Register {
    text: String,
    linewise: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
    Indent,
    Outdent,
    Lower,
    Upper,
    Toggle,
    Comment,
}

/// What a motion or text object covers, for an operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    start: usize,
    /// Past the last character.
    end: usize,
    linewise: bool,
}

enum Parsed {
    Wait,
    Bad,
    Done(Cmd),
}

struct Cmd {
    reg: Option<char>,
    count: Option<usize>,
    what: What,
}

enum What {
    Move(String),
    /// An operator and what it covers: a motion, a text object (`iw`), or
    /// the whole lines when it's doubled (`dd`).
    Op(Op, Option<usize>, Target),
    /// A text object, which Visual mode selects.
    Object(String),
    /// Anything else, by its keys, such as `x`, `gJ` or `ra`.
    Do(String),
}

enum Target {
    Motion(String),
    Object(String),
    Lines,
}

#[derive(Debug, Default)]
pub(crate) struct Vim {
    pub(crate) mode: Option<VimMode>,
    pending: Vec<char>,
    /// The keys of the command being typed, for `.`.
    pending_keys: Vec<Key>,
    registers: HashMap<char, Register>,
    last_change: Vec<Key>,
    /// The keys of a change still going on in Insert mode.
    change: Option<Vec<Key>>,
    replaying: bool,
    last_find: Option<(char, char)>,
    /// The last search, as Vim writes it, and whether it went forward.
    search: Option<(String, bool)>,
    /// The search's matches are tinted, until `:noh` or Esc.
    highlight: bool,
    /// The command line, while it's open.
    line: Option<line::Line>,
    /// The `:` lines and searches typed before, for ↑.
    history: Vec<String>,
    searches: Vec<String>,
    /// What the last command said, such as "3 of 12", for the status bar.
    pub(crate) note: Option<String>,
    /// The last `:s`, for `:&` and `&`.
    last_substitute: Option<(String, String)>,
    /// Where Visual mode began, and its cursor.
    anchor: usize,
    cursor: usize,
    last_visual: Option<(VimMode, usize, usize)>,
    /// The column j and k keep; `usize::MAX` after `$`.
    goal: Option<usize>,
    /// The undo steps the document had when a change began, to make them
    /// one.
    undo_from: Option<(u64, usize)>,
    /// The first letter of `jk`, and when it was typed.
    escape: Option<Instant>,
    marks: HashMap<char, (u64, usize)>,
    /// A put waiting for the clipboard: before the cursor, and how many.
    put: Option<(bool, usize)>,
}

impl Vim {
    pub(crate) fn new() -> Self {
        Self {
            mode: Some(VimMode::Normal),
            ..Default::default()
        }
    }

    pub(crate) fn mode(&self) -> VimMode {
        self.mode.unwrap_or(VimMode::Normal)
    }
}

/// The key as the parser sees it, or `None` for one Vim leaves to merlin.
fn token(key: &Key) -> Option<char> {
    use KeyCode::*;
    if key.alt {
        return None;
    }
    if key.ctrl {
        return match key.code {
            Char('[') if !key.shift => Some(ESC),
            Char(c) if c.is_ascii_lowercase() && !key.shift => Some(ctrl(c)),
            _ => None,
        };
    }
    if key.shift && !matches!(key.code, Char(_)) {
        return None;
    }
    Some(match key.code {
        Char(c) => c,
        Esc => ESC,
        Enter => ENTER,
        Backspace => BACKSPACE,
        Tab => '\t',
        Left => LEFT,
        Right => RIGHT,
        Up => UP,
        Down => DOWN,
        Home => HOME,
        End => END,
        PageUp => PAGE_UP,
        PageDown => PAGE_DOWN,
        Delete => DELETE,
        _ => return None,
    })
}

fn count_at(s: &[char], i: &mut usize) -> Option<usize> {
    let from = *i;
    if s.get(*i).is_some_and(|c| ('1'..='9').contains(c)) {
        while s.get(*i).is_some_and(|c| c.is_ascii_digit()) {
            *i += 1;
        }
    }
    (from < *i).then(|| {
        s[from..*i]
            .iter()
            .collect::<String>()
            .parse::<usize>()
            .unwrap_or(1)
            .min(99_999)
    })
}

/// A motion at the start of `s`, and how many keys it took.
fn motion(s: &[char]) -> Result<Option<usize>, ()> {
    let Some(&c) = s.first() else {
        return Ok(None);
    };
    if MOTIONS.contains(c) {
        return Ok(Some(1));
    }
    match c {
        'f' | 'F' | 't' | 'T' | '\'' | '`' => Ok(s.get(1).map(|_| 2)),
        'g' => match s.get(1) {
            None => Ok(None),
            Some('g' | 'e' | 'E' | '_' | '0' | '^' | '$' | 'j' | 'k') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

/// A text object at the start of `s`, such as `iw` or `a(`.
fn object(s: &[char]) -> Result<Option<usize>, ()> {
    match s.first() {
        Some('i' | 'a') => match s.get(1) {
            None => Ok(None),
            Some(c) if "wWsp()b[]{}B<>\"'`fc".contains(*c) => Ok(Some(2)),
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

/// An operator at the start of `s`: which, and how many keys.
fn operator(s: &[char]) -> Result<Option<(Op, usize)>, ()> {
    let one = match s.first() {
        Some('d') => Op::Delete,
        Some('c') => Op::Change,
        Some('y') => Op::Yank,
        Some('>') => Op::Indent,
        Some('<') => Op::Outdent,
        Some('g') => {
            return match s.get(1) {
                None => Ok(None),
                Some('~') => Ok(Some((Op::Toggle, 2))),
                Some('u') => Ok(Some((Op::Lower, 2))),
                Some('U') => Ok(Some((Op::Upper, 2))),
                Some('c') => Ok(Some((Op::Comment, 2))),
                Some(_) => Err(()),
            };
        }
        _ => return Err(()),
    };
    Ok(Some((one, 1)))
}

/// Normal mode's other commands, one key or two.
fn action(s: &[char], visual: bool) -> Result<Option<usize>, ()> {
    let Some(&c) = s.first() else {
        return Ok(None);
    };
    let one = if visual {
        "xXsSCDYRpPJuUoOIA~vV:/?\x1b\x16"
    } else {
        "iaIAoOxXsSCDYpPJuR.vV~K&:/?\x1b\x12\x16\x01\x18\x0f\x05\x19\x04\x15\x06\x02\t\u{e006}\u{e007}\u{e008}"
    };
    if one.contains(c) {
        return Ok(Some(1));
    }
    match c {
        'r' | 'm' => Ok(s.get(1).map(|_| 2)),
        'q' | '@' if !visual => Ok(s.get(1).map(|_| 2)),
        'z' if !visual => match s.get(1) {
            None => Ok(None),
            Some('z' | 't' | 'b' | '.' | '-' | '\r' | 'o' | 'c' | 'a' | 'R' | 'M') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        '[' | ']' if !visual => match s.get(1) {
            None => Ok(None),
            Some('c' | 'd') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        '\x17' if !visual => match s.get(1) {
            None => Ok(None),
            Some('w' | 'W' | 'v' | 's' | 'c' | 'q' | 'o' | 'h' | 'l' | 'j' | 'k' | '\x17') => {
                Ok(Some(2))
            }
            Some(_) => Err(()),
        },
        'Z' if !visual => match s.get(1) {
            None => Ok(None),
            Some('Z' | 'Q') => Ok(Some(2)),
            Some(_) => Err(()),
        },
        'g' => match s.get(1) {
            None => Ok(None),
            Some('v' | 'J' | 'd' | 'D' | 'r' | 'h' | 'a' | 't' | 'T' | 'i' | 'I' | 'x') => {
                Ok(Some(2))
            }
            Some(_) => Err(()),
        },
        _ => Err(()),
    }
}

fn parse(s: &[char], visual: bool) -> Parsed {
    let mut i = 0;
    let mut reg = None;
    if s.first() == Some(&'"') {
        match s.get(1) {
            None => return Parsed::Wait,
            Some(&r) => reg = Some(r),
        }
        i = 2;
    }
    let count = count_at(s, &mut i);
    let rest = &s[i..];
    if rest.is_empty() {
        return Parsed::Wait;
    }
    let done = |what| Parsed::Done(Cmd { reg, count, what });
    let text = |n: usize| rest[..n].iter().collect::<String>();
    if !visual {
        match operator(rest) {
            Ok(None) => return Parsed::Wait,
            Ok(Some((op, n))) => {
                let mut j = n;
                let count2 = count_at(rest, &mut j);
                let after = &rest[j..];
                let Some(&next) = after.first() else {
                    return Parsed::Wait;
                };
                // Doubled, for whole lines: dd, >>, gcc, guu or gugu.
                let last = rest[n - 1];
                if next == last {
                    return done(What::Op(op, count2, Target::Lines));
                }
                if n == 2 && next == 'g' {
                    return match after.get(1) {
                        None => Parsed::Wait,
                        Some(c) if *c == last => done(What::Op(op, count2, Target::Lines)),
                        _ => match motion(after) {
                            Ok(Some(m)) => done(What::Op(
                                op,
                                count2,
                                Target::Motion(after[..m].iter().collect()),
                            )),
                            _ => Parsed::Bad,
                        },
                    };
                }
                return match object(after) {
                    Ok(None) => Parsed::Wait,
                    Ok(Some(m)) => done(What::Op(
                        op,
                        count2,
                        Target::Object(after[..m].iter().collect()),
                    )),
                    Err(()) => match motion(after) {
                        Ok(None) => Parsed::Wait,
                        Ok(Some(m)) => done(What::Op(
                            op,
                            count2,
                            Target::Motion(after[..m].iter().collect()),
                        )),
                        Err(()) => Parsed::Bad,
                    },
                };
            }
            Err(()) => {}
        }
    } else {
        match object(rest) {
            Ok(None) => return Parsed::Wait,
            Ok(Some(n)) => return done(What::Object(text(n))),
            Err(()) => {}
        }
        // An operator acts on the selection at once.
        match rest {
            ['d' | 'c' | 'y' | '>' | '<', ..] => return done(What::Do(text(1))),
            ['g', next, ..] if "~uUcJv".contains(*next) => return done(What::Do(text(2))),
            _ => {}
        }
    }
    match motion(rest) {
        Ok(None) => return Parsed::Wait,
        Ok(Some(n)) => return done(What::Move(text(n))),
        Err(()) => {}
    }
    match action(rest, visual) {
        Ok(None) => Parsed::Wait,
        Ok(Some(n)) => done(What::Do(text(n))),
        Err(()) => Parsed::Bad,
    }
}

/// The commands that change the text, which `.` repeats.
fn is_change(what: &What) -> bool {
    match what {
        What::Op(op, ..) => *op != Op::Yank,
        What::Do(keys) => {
            let c = keys.chars().next().unwrap_or(' ');
            "iaIAoOxXsSCDpPJR~r\x01\x18".contains(c) || matches!(keys.as_str(), "gJ" | "gi" | "gI")
        }
        _ => false,
    }
}

fn ch(doc: &Doc, p: usize) -> char {
    doc.buffer.rope().get_char(p).unwrap_or('\n')
}

fn line_end(doc: &Doc, line: usize) -> usize {
    doc.line_start(line) + doc.buffer.line_len(line)
}

/// Where the next line starts, or the end of the text.
fn next_line(doc: &Doc, line: usize) -> usize {
    if line < doc.last_line() {
        doc.line_start(line + 1)
    } else {
        doc.buffer.len()
    }
}

fn first_non_blank(doc: &Doc, line: usize) -> usize {
    let indent = doc
        .line_chars(line)
        .iter()
        .take_while(|c| **c == ' ' || **c == '\t')
        .count();
    doc.line_start(line) + indent
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    Word,
    Punct,
}

fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// An empty line starts at `p`.
fn empty_line_at(doc: &Doc, p: usize) -> bool {
    ch(doc, p) == '\n' && (p == 0 || ch(doc, p - 1) == '\n') && p < doc.buffer.len()
}

fn word_forward(doc: &Doc, p: usize, big: bool) -> usize {
    let len = doc.buffer.len();
    let mut i = p;
    if i >= len {
        return len;
    }
    let k = class(ch(doc, i), big);
    if k != Class::Space {
        while i < len && class(ch(doc, i), big) == k {
            i += 1;
        }
    }
    while i < len && class(ch(doc, i), big) == Class::Space {
        if i != p && empty_line_at(doc, i) {
            break;
        }
        i += 1;
    }
    i
}

fn word_end(doc: &Doc, p: usize, big: bool) -> usize {
    let len = doc.buffer.len();
    let mut i = p + 1;
    while i < len && class(ch(doc, i), big) == Class::Space {
        i += 1;
    }
    if i >= len {
        return len.saturating_sub(1);
    }
    let k = class(ch(doc, i), big);
    while i + 1 < len && class(ch(doc, i + 1), big) == k {
        i += 1;
    }
    i
}

fn word_back(doc: &Doc, p: usize, big: bool) -> usize {
    if p == 0 {
        return 0;
    }
    let mut i = p - 1;
    while i > 0 && class(ch(doc, i), big) == Class::Space {
        if empty_line_at(doc, i) {
            return i;
        }
        i -= 1;
    }
    let k = class(ch(doc, i), big);
    while i > 0 && class(ch(doc, i - 1), big) == k {
        i -= 1;
    }
    i
}

fn word_end_back(doc: &Doc, p: usize, big: bool) -> usize {
    let mut i = p;
    let k = class(ch(doc, i), big);
    if k != Class::Space {
        while i > 0 && class(ch(doc, i), big) == k {
            i -= 1;
        }
    }
    while i > 0 && class(ch(doc, i), big) == Class::Space {
        if empty_line_at(doc, i) {
            return i;
        }
        i -= 1;
    }
    i
}

fn blank(doc: &Doc, line: usize) -> bool {
    doc.line_chars(line).iter().all(|c| c.is_whitespace())
}

fn swap_case(c: char) -> String {
    if c.is_uppercase() {
        c.to_lowercase().collect()
    } else {
        c.to_uppercase().collect()
    }
}

impl Editor {
    pub(crate) fn vim_mode(&self) -> Option<VimMode> {
        self.vim.as_ref().map(|v| v.mode())
    }

    /// The mode and keys so far, for the status bar.
    pub(crate) fn vim_view(&self) -> Option<VimView> {
        let vim = self.vim.as_ref()?;
        let pending = vim
            .pending
            .iter()
            .map(|c| match *c {
                ESC => "⎋".to_string(),
                c if (c as u32) < 27 => format!("^{}", ((c as u8) + b'A' - 1) as char),
                c => c.to_string(),
            })
            .collect();
        Some(VimView {
            mode: vim.mode(),
            pending,
            note: vim.note.clone(),
        })
    }

    /// The cursor's shape, for the window.
    pub(crate) fn vim_cursor(&self) -> Option<VimCursor> {
        let vim = self.vim.as_ref()?;
        Some(match vim.mode() {
            VimMode::Insert => VimCursor::Line,
            VimMode::Replace => VimCursor::Underline,
            _ if parse_waits_for_motion(&vim.pending) => VimCursor::HalfBlock,
            _ => VimCursor::Block,
        })
    }

    /// Where Visual mode's cursor is, which the selection's end isn't.
    pub(crate) fn vim_visual_cursor(&self) -> Option<usize> {
        let vim = self.vim.as_ref()?;
        vim.mode().visual().then_some(vim.cursor)
    }

    /// Line numbers count from the cursor's line: in Normal and Visual
    /// mode, with the setting on.
    pub(crate) fn relative_numbers(&self) -> bool {
        self.relative_numbers && self.vim_mode().is_some_and(|m| m != VimMode::Insert)
    }

    pub(crate) fn set_keymap(&mut self, keymap: Keymap) {
        match (keymap, self.vim.is_some()) {
            (Keymap::Vim, false) => {
                self.vim = Some(Vim::new());
                self.vim_clamp();
            }
            (Keymap::VsCode, true) => self.vim = None,
            _ => {}
        }
    }

    /// A key in the text, which Vim takes or leaves to merlin. Returns
    /// whether it took it.
    pub(crate) fn vim_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let Some(vim) = &self.vim else {
            return false;
        };
        if self.focus != Focus::Editor || self.doc().page.is_some() || self.in_review() {
            return false;
        }
        // The command line takes every key while it's open.
        if vim.line.is_some() {
            self.vim_line_key(key, fx);
            return true;
        }
        let mode = vim.mode();
        let kept = self.vim_settings.keep.iter().any(|k| *k == key.spec());
        let Some(t) = token(&key) else {
            return false;
        };
        if kept && t != ESC {
            return false;
        }
        match mode {
            VimMode::Insert | VimMode::Replace => self.vim_insert_key(key, t, fx),
            _ => {
                if (t as u32) < 27 && t != ESC && t != ENTER && t != '\t' {
                    let letter = ((t as u8) + b'a' - 1) as char;
                    if !VIM_CTRL.contains(letter) {
                        return false;
                    }
                }
                self.vim_command_key(key, t, fx);
                true
            }
        }
    }

    fn vim_insert_key(&mut self, key: Key, t: char, fx: &mut Vec<Effect>) -> bool {
        let jk: Vec<char> = self.vim_settings.insert_escape.chars().collect();
        let vim = self.vim.as_mut().expect("vim");
        if let Some(change) = &mut vim.change
            && !vim.replaying
        {
            change.push(key);
        }
        if t == ESC {
            self.vim_leave_insert(fx);
            return true;
        }
        // The second letter of `jk`, quickly after the first: the first
        // goes, and so does Insert mode.
        if jk.len() == 2
            && t == jk[1]
            && let Some(at) = vim.escape.take()
            && at.elapsed() < ESCAPE_WAIT
        {
            {
                let doc = self.doc();
                let cursor = doc.cursor;
                if cursor > 0 && ch(doc, cursor - 1) == jk[0] {
                    self.each_caret(Kind::Typing, |e, _| {
                        let c = e.doc().cursor;
                        if c > 0 {
                            e.replace(c - 1, c, "", Kind::Typing);
                        }
                    });
                    let vim = self.vim.as_mut().expect("vim");
                    if let Some(change) = &mut vim.change {
                        change.pop();
                        change.pop();
                        change.push(Key::plain(KeyCode::Esc));
                    }
                    self.vim_leave_insert(fx);
                    return true;
                }
            }
        }
        let vim = self.vim.as_mut().expect("vim");
        vim.escape = (jk.len() == 2 && t == jk[0]).then(Instant::now);
        if vim.mode() == VimMode::Replace {
            if let KeyCode::Char(c) = key.code
                && !key.ctrl
            {
                self.each_caret(Kind::Typing, |e, _| {
                    let doc = e.doc();
                    let at = doc.cursor;
                    let end = if ch(doc, at) == '\n' || at >= doc.buffer.len() {
                        at
                    } else {
                        at + 1
                    };
                    e.replace(at, end, &c.to_string(), Kind::Typing);
                });
                return true;
            }
            if t == BACKSPACE {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc_mut();
                    doc.cursor = doc.cursor.saturating_sub(1);
                });
                return true;
            }
        }
        false
    }

    fn vim_leave_insert(&mut self, _fx: &mut Vec<Effect>) {
        let (id, at) = (self.doc().id, self.doc().cursor);
        let vim = self.vim.as_mut().expect("vim");
        vim.marks.insert('^', (id, at));
        vim.mode = Some(VimMode::Normal);
        vim.escape = None;
        if let Some(change) = vim.change.take() {
            vim.last_change = change;
        }
        self.vim_join_undo();
        // Back over the last character typed, as Vim does.
        self.each_caret(Kind::Other, |e, _| {
            let doc = e.doc_mut();
            doc.anchor = None;
            let (_, col) = doc.line_col(doc.cursor);
            if col > 0 {
                doc.cursor -= 1;
            }
        });
        self.popup = None;
    }

    /// The undo steps since the change began become one.
    fn vim_join_undo(&mut self) {
        let id = self.doc().id;
        let vim = self.vim.as_mut().expect("vim");
        if let Some((doc, from)) = vim.undo_from.take()
            && doc == id
        {
            self.doc_mut().buffer.join_undo(from);
        }
    }

    fn vim_command_key(&mut self, key: Key, t: char, fx: &mut Vec<Effect>) {
        let vim = self.vim.as_mut().expect("vim");
        // A selection made some other way, as with the mouse, is Visual
        // mode's.
        if vim.mode() == VimMode::Normal && vim.pending.is_empty() {
            let doc = self.doc();
            if let (Some(anchor), Some(_)) = (doc.anchor, doc.selection())
                && !self.has_carets()
            {
                let cursor = doc.cursor;
                let vim = self.vim.as_mut().expect("vim");
                vim.mode = Some(VimMode::Visual);
                vim.anchor = anchor;
                vim.cursor = if cursor > anchor { cursor - 1 } else { cursor };
            }
        }
        let vim = self.vim.as_mut().expect("vim");
        if vim.pending.is_empty() {
            vim.note = None;
        } else if t == BACKSPACE {
            // Up a level, as which-key says.
            vim.pending.pop();
            vim.pending_keys.pop();
            return;
        } else if t == ESC {
            vim.pending.clear();
            vim.pending_keys.clear();
            return;
        }
        vim.pending.push(t);
        vim.pending_keys.push(key);
        // `q` stops a recording.
        if vim.pending == ['q'] && self.recording.is_some() {
            vim.pending.clear();
            vim.pending_keys.clear();
            return self.macro_stop();
        }
        if self.vim_leader(fx) {
            return;
        }
        let vim = self.vim.as_mut().expect("vim");
        let visual = vim.mode().visual();
        match parse(&vim.pending, visual) {
            Parsed::Wait => {}
            Parsed::Bad => {
                let keys = show_keys(&vim.pending.iter().collect::<String>());
                vim.pending.clear();
                vim.pending_keys.clear();
                self.macro_fail(format!("{keys} isn't a command"));
            }
            Parsed::Done(cmd) => {
                vim.pending.clear();
                let keys = std::mem::take(&mut vim.pending_keys);
                let change = !visual && is_change(&cmd.what) && !vim.replaying;
                if change || visual {
                    let steps = self.doc().buffer.undo_steps();
                    let id = self.doc().id;
                    self.vim.as_mut().expect("vim").undo_from = Some((id, steps));
                }
                self.vim_run(cmd, fx);
                let vim = self.vim.as_mut().expect("vim");
                if vim.mode() == VimMode::Insert || vim.mode() == VimMode::Replace {
                    if change {
                        vim.change = Some(keys);
                    }
                } else {
                    if change {
                        vim.last_change = keys;
                    }
                    self.vim_join_undo();
                    self.vim_clamp();
                }
            }
        }
    }

    /// In Normal mode the cursor is on a character, never past the last.
    fn vim_clamp(&mut self) {
        let mode = self.vim_mode();
        if mode != Some(VimMode::Normal) {
            return;
        }
        let fix = |doc: &mut Doc| {
            let (line, col) = doc.line_col(doc.cursor);
            let len = doc.buffer.line_len(line);
            if len > 0 && col >= len {
                doc.cursor = doc.line_start(line) + len - 1;
            }
            doc.anchor = None;
        };
        let doc = self.doc_mut();
        fix(doc);
        let extra = std::mem::take(&mut doc.extra);
        let mut fixed = Vec::new();
        for c in extra {
            let mut d = (c.cursor, c.anchor);
            let (line, col) = doc.line_col(d.0);
            let len = doc.buffer.line_len(line);
            if len > 0 && col >= len {
                d.0 = doc.line_start(line) + len - 1;
            }
            d.1 = None;
            fixed.push(crate::doc::Caret {
                cursor: d.0,
                anchor: d.1,
            });
        }
        doc.extra = fixed;
    }

    fn vim_run(&mut self, cmd: Cmd, fx: &mut Vec<Effect>) {
        let mode = self.vim_mode().unwrap_or(VimMode::Normal);
        if mode.visual() {
            return self.vim_visual(cmd, fx);
        }
        let n = cmd.count.unwrap_or(1);
        match cmd.what {
            What::Move(m) => {
                let carets = self.has_carets();
                let jump = "GnN*#%{}'`".contains(m.chars().next().unwrap_or(' ')) || m == "gg";
                if jump && !carets {
                    let (id, at) = (self.doc().id, self.doc().cursor);
                    self.vim.as_mut().expect("vim").marks.insert('\'', (id, at));
                }
                let before = self.doc().cursor;
                self.each_caret(Kind::Other, |e, _| {
                    if let Some((to, _)) = e.vim_motion(&m, cmd.count, None) {
                        let doc = e.doc_mut();
                        doc.cursor = to;
                        doc.anchor = None;
                    }
                });
                // A motion with nowhere to go stops a macro, as in Vim.
                if self.doc().cursor == before && must_move(&m) {
                    self.macro_fail(format!("{} had nowhere to go", show_keys(&m)));
                }
                if matches!(m.as_str(), "n" | "N" | "*" | "#") {
                    self.vim_count_note();
                }
            }
            What::Op(op, count2, target) => {
                let count = match (cmd.count, count2) {
                    (None, None) => None,
                    (a, b) => Some(a.unwrap_or(1) * b.unwrap_or(1)),
                };
                let reg = cmd.reg;
                let mut enter = false;
                self.each_caret(Kind::Other, |e, _| {
                    let span = match &target {
                        Target::Lines => {
                            let doc = e.doc();
                            let line = doc.line_col(doc.cursor).0;
                            let last = (line + count.unwrap_or(1) - 1).min(doc.last_line());
                            Some(Span {
                                start: doc.line_start(line),
                                end: next_line(doc, last),
                                linewise: true,
                            })
                        }
                        Target::Object(o) => e.vim_object(o, count),
                        Target::Motion(m) => e.vim_motion_span(m, count, op),
                    };
                    match span {
                        Some(span) => enter |= e.vim_operate(op, span, reg, fx),
                        None => e.macro_fail("an operator had nothing to act on".into()),
                    }
                });
                if enter {
                    self.vim_enter(VimMode::Insert);
                }
            }
            What::Object(_) => {}
            What::Do(keys) => self.vim_do(&keys, cmd.reg, cmd.count, n, fx),
        }
    }

    /// Where a motion goes from the cursor, and whether an operator over it
    /// takes the character it lands on (`e`, `$`, `f`) or whole lines
    /// (`j`, `G`). `None` when it can't go, as `f` with no such letter.
    fn vim_motion(&mut self, m: &str, count: Option<usize>, op: Option<Op>) -> Option<(usize, u8)> {
        // 0: exclusive, 1: inclusive, 2: whole lines.
        const EXCL: u8 = 0;
        const INCL: u8 = 1;
        const LINES: u8 = 2;
        let n = count.unwrap_or(1);
        let height = self.text_height().max(1);
        let goal_in = self.vim.as_ref().and_then(|v| v.goal);
        let vim_find = self.vim.as_ref().and_then(|v| v.last_find);
        let last_search = self.vim.as_ref().and_then(|v| v.search.clone());
        let id = self.doc().id;
        let marks = self
            .vim
            .as_ref()
            .map(|v| v.marks.clone())
            .unwrap_or_default();
        let doc = self.doc();
        let p = if self.vim_mode().is_some_and(|m| m.visual()) {
            self.vim.as_ref().map_or(doc.cursor, |v| v.cursor)
        } else {
            doc.cursor
        };
        let (line, col) = doc.line_col(p);
        let len = doc.buffer.len();
        let line_len = doc.buffer.line_len(line);
        let start = doc.line_start(line);
        let mut keep_goal = false;
        let mut goal_out = None;
        let at_line = |l: usize, goal: Option<usize>| -> usize {
            let l = l.min(doc.last_line());
            match goal {
                Some(usize::MAX) => line_end(doc, l)
                    .saturating_sub((doc.buffer.line_len(l) > 0 && op.is_none()) as usize),
                Some(g) => {
                    let p = doc.pos_at(l, g);
                    let ll = doc.buffer.line_len(l);
                    if op.is_none() && ll > 0 && p >= doc.line_start(l) + ll {
                        doc.line_start(l) + ll - 1
                    } else {
                        p
                    }
                }
                None => first_non_blank(doc, l),
            }
        };
        let first = m.chars().next().unwrap_or(' ');
        let second = m.chars().nth(1);
        let result = match (first, second) {
            ('h' | LEFT, None) => (start + col.saturating_sub(n), EXCL),
            (BACKSPACE, None) => (p.saturating_sub(n), EXCL),
            ('l' | RIGHT, None) => {
                let max = if op.is_some() || line_len == 0 {
                    line_len
                } else {
                    line_len - 1
                };
                (start + (col + n).min(max), EXCL)
            }
            (' ', None) => ((p + n).min(len), EXCL),
            ('j' | 'k' | UP | DOWN, None) | ('g', Some('j' | 'k')) => {
                let down = matches!(first, 'j' | DOWN) || second == Some('j');
                let goal = goal_in.unwrap_or_else(|| doc.display_col(line, col));
                keep_goal = true;
                goal_out = Some(goal);
                let target = if down {
                    (line + n).min(doc.last_line())
                } else {
                    line.checked_sub(n)?
                };
                if down && line == doc.last_line() {
                    return None;
                }
                (at_line(target, Some(goal)), LINES)
            }
            ('w' | 'W', None) => {
                let big = first == 'W';
                let mut q = p;
                for _ in 0..n {
                    q = word_forward(doc, q, big);
                }
                // `dw` on a line's last word stops at its end.
                if op.is_some() {
                    let (ql, _) = doc.line_col(q.min(len));
                    if ql > line && q <= first_non_blank(doc, ql) {
                        let mut l = ql - 1;
                        while l > line && blank(doc, l) {
                            l -= 1;
                        }
                        q = line_end(doc, l).max(p);
                    }
                }
                (q, EXCL)
            }
            ('b' | 'B', None) => {
                let mut q = p;
                for _ in 0..n {
                    q = word_back(doc, q, first == 'B');
                }
                (q, EXCL)
            }
            ('e' | 'E', None) => {
                let mut q = p;
                for _ in 0..n {
                    q = word_end(doc, q, first == 'E');
                }
                (q, INCL)
            }
            ('g', Some('e' | 'E')) => {
                let mut q = p;
                for _ in 0..n {
                    q = word_end_back(doc, q, second == Some('E'));
                }
                (q, INCL)
            }
            ('0' | HOME, None) | ('g', Some('0')) => (start, EXCL),
            ('^', None) | ('g', Some('^')) => (first_non_blank(doc, line), EXCL),
            ('$' | END, None) | ('g', Some('$')) => {
                keep_goal = true;
                goal_out = Some(usize::MAX);
                let l = (line + n - 1).min(doc.last_line());
                let e = line_end(doc, l);
                let back = (op.is_none() && doc.buffer.line_len(l) > 0) as usize;
                (e - back, if op.is_some() { EXCL } else { INCL })
            }
            ('g', Some('_')) => {
                let l = (line + n - 1).min(doc.last_line());
                let chars = doc.line_chars(l);
                let trail = chars.iter().rev().take_while(|c| c.is_whitespace()).count();
                let last = (chars.len() - trail).saturating_sub(1);
                (doc.line_start(l) + last, INCL)
            }
            ('|', None) => (start + (n - 1).min(line_len.saturating_sub(1)), EXCL),
            ('G', None) => {
                let l = count.map_or(doc.last_line(), |c| c - 1);
                (first_non_blank(doc, l.min(doc.last_line())), LINES)
            }
            ('g', Some('g')) => (first_non_blank(doc, (n - 1).min(doc.last_line())), LINES),
            ('H', None) => {
                let l = (doc.scroll + n - 1).min(doc.last_line());
                (first_non_blank(doc, l), LINES)
            }
            ('M', None) => {
                let bottom = (doc.scroll + height - 1).min(doc.last_line());
                (first_non_blank(doc, (doc.scroll + bottom) / 2), LINES)
            }
            ('L', None) => {
                let bottom = (doc.scroll + height - 1).min(doc.last_line());
                (
                    first_non_blank(doc, bottom.saturating_sub(n - 1).max(doc.scroll)),
                    LINES,
                )
            }
            ('\r' | '+', None) => {
                let l = line + n;
                if l > doc.last_line() {
                    return None;
                }
                (first_non_blank(doc, l), LINES)
            }
            ('-', None) => (first_non_blank(doc, line.checked_sub(n)?), LINES),
            ('_', None) => (
                first_non_blank(doc, (line + n - 1).min(doc.last_line())),
                LINES,
            ),
            ('}', None) => {
                let mut l = line;
                for _ in 0..n {
                    while l < doc.last_line() && blank(doc, l) {
                        l += 1;
                    }
                    while l < doc.last_line() && !blank(doc, l) {
                        l += 1;
                    }
                }
                if l == doc.last_line() && !blank(doc, l) {
                    (line_end(doc, l), if op.is_some() { EXCL } else { INCL })
                } else {
                    (doc.line_start(l), EXCL)
                }
            }
            ('{', None) => {
                let mut l = line;
                for _ in 0..n {
                    while l > 0 && blank(doc, l) {
                        l -= 1;
                    }
                    while l > 0 && !blank(doc, l) {
                        l -= 1;
                    }
                }
                (doc.line_start(l), EXCL)
            }
            ('%', None) => {
                let chars = doc.line_chars(line);
                let at = (col..chars.len()).find(|&i| "()[]{}".contains(chars[i]))?;
                let pos = start + at;
                let c = chars[at];
                let (open, close) = match c {
                    '(' | ')' => ('(', ')'),
                    '[' | ']' => ('[', ']'),
                    _ => ('{', '}'),
                };
                let rope = doc.buffer.rope();
                let to = if c == open {
                    editing::scan(rope, pos + 1, true, open, close)?
                } else {
                    editing::scan(rope, pos, false, open, close)?
                };
                (to, INCL)
            }
            ('f' | 'F' | 't' | 'T', Some(target)) => {
                let to = find_in_line(doc, p, first, target, n)?;
                self.vim.as_mut().expect("vim").last_find = Some((first, target));
                (
                    to,
                    if first == 'f' || first == 't' {
                        INCL
                    } else {
                        EXCL
                    },
                )
            }
            (';' | ',', None) => {
                let (kind, target) = vim_find?;
                let kind = if first == ',' {
                    match kind {
                        'f' => 'F',
                        'F' => 'f',
                        't' => 'T',
                        _ => 't',
                    }
                } else {
                    kind
                };
                // `;` after `t` goes past the letter it stopped before.
                let from = match kind {
                    't' if ch(doc, p + 1) == target => p + 1,
                    'T' if p > 0 && ch(doc, p - 1) == target => p - 1,
                    _ => p,
                };
                let to = find_in_line(doc, from, kind, target, n)?;
                (
                    to,
                    if kind == 'f' || kind == 't' {
                        INCL
                    } else {
                        EXCL
                    },
                )
            }
            ('*' | '#', None) => {
                let (s, e) = (doc.word_start_at(p), doc.word_end_at(p));
                let (s, e) = if s < e {
                    (s, e)
                } else {
                    let s = doc.word_end_at(word_forward(doc, p, false));
                    (doc.word_start_at(s), s)
                };
                if s >= e {
                    return None;
                }
                let word = doc.slice(s, e);
                let forward = first == '*';
                // The word is letters, digits and `_`, which need no escaping.
                let pattern = format!(r"\<{word}\>");
                let to = search_regex(doc, s, &vim_regex(&pattern).ok()?, forward, n)?;
                let vim = self.vim.as_mut().expect("vim");
                vim.search = Some((pattern, forward));
                vim.highlight = true;
                (to, EXCL)
            }
            ('n' | 'N', None) => {
                let (pattern, forward) = last_search?;
                let forward = forward == (first == 'n');
                let to = search_regex(doc, p, &vim_regex(&pattern).ok()?, forward, n)?;
                self.vim.as_mut().expect("vim").highlight = true;
                (to, EXCL)
            }
            ('\'' | '`', Some(mark)) => {
                let (doc_id, at) = *marks.get(&mark)?;
                if doc_id != id {
                    return None;
                }
                let at = at.min(len);
                if first == '\'' {
                    (first_non_blank(doc, doc.line_col(at).0), LINES)
                } else {
                    (at, EXCL)
                }
            }
            _ => return None,
        };
        let vim = self.vim.as_mut().expect("vim");
        vim.goal = if keep_goal { goal_out } else { None };
        Some(result)
    }

    /// What an operator with a motion covers.
    fn vim_motion_span(&mut self, m: &str, count: Option<usize>, op: Op) -> Option<Span> {
        // `cw` is `ce`, as Vim has it, when on a word.
        let doc = self.doc();
        let on_word = !ch(doc, doc.cursor).is_whitespace();
        let m = match m {
            "w" if op == Op::Change && on_word => "e",
            "W" if op == Op::Change && on_word => "E",
            m => m,
        };
        // `ce` on a word's last letter changes just that word.
        let from = self.doc().cursor;
        let (to, kind) = if op == Op::Change && (m == "e" || m == "E") && on_word {
            let doc = self.doc();
            let big = m == "E";
            let k = class(ch(doc, from), big);
            let mut end = from;
            while end + 1 < doc.buffer.len() && class(ch(doc, end + 1), big) == k {
                end += 1;
            }
            let mut to = end;
            for _ in 1..count.unwrap_or(1) {
                to = word_end(doc, to, big);
            }
            (to, 1)
        } else {
            self.vim_motion(m, count, Some(op))?
        };
        let doc = self.doc();
        Some(if kind == 2 {
            let (a, b) = (doc.line_col(from).0, doc.line_col(to).0);
            let (a, b) = (a.min(b), a.max(b));
            Span {
                start: doc.line_start(a),
                end: next_line(doc, b),
                linewise: true,
            }
        } else {
            let (a, b) = (from.min(to), from.max(to));
            let b = if kind == 1 {
                (b + 1).min(doc.buffer.len())
            } else {
                b
            };
            Span {
                start: a,
                end: b,
                linewise: false,
            }
        })
    }

    /// What a text object such as `iw`, `a(` or `ip` covers at the cursor.
    fn vim_object(&self, o: &str, count: Option<usize>) -> Option<Span> {
        let doc = self.doc();
        let p = if self.vim_mode().is_some_and(|m| m.visual()) {
            self.vim.as_ref().map_or(doc.cursor, |v| v.cursor)
        } else {
            doc.cursor
        };
        let mut chars = o.chars();
        let around = chars.next() == Some('a');
        let what = chars.next()?;
        let len = doc.buffer.len();
        let span = |start, end| {
            Some(Span {
                start,
                end,
                linewise: false,
            })
        };
        match what {
            'w' | 'W' => {
                let big = what == 'W';
                let k = class(ch(doc, p), big);
                let (line, _) = doc.line_col(p);
                let (ls, le) = (doc.line_start(line), line_end(doc, line));
                let mut s = p;
                while s > ls && class(ch(doc, s - 1), big) == k {
                    s -= 1;
                }
                let mut e = p;
                while e < le && class(ch(doc, e), big) == k {
                    e += 1;
                }
                if around {
                    if k != Class::Space {
                        // The spaces after, or else the ones before.
                        let mut e2 = e;
                        while e2 < le && class(ch(doc, e2), big) == Class::Space {
                            e2 += 1;
                        }
                        if e2 > e {
                            e = e2;
                        } else {
                            while s > ls && class(ch(doc, s - 1), big) == Class::Space {
                                s -= 1;
                            }
                        }
                    } else {
                        let k2 = class(ch(doc, e), big);
                        while e < le && class(ch(doc, e), big) == k2 {
                            e += 1;
                        }
                    }
                }
                span(s, e.max(s + 1).min(le.max(s + 1)))
            }
            '(' | ')' | 'b' | '[' | ']' | '{' | '}' | 'B' | '<' | '>' => {
                let (open, close) = match what {
                    '(' | ')' | 'b' => ('(', ')'),
                    '[' | ']' => ('[', ']'),
                    '{' | '}' | 'B' => ('{', '}'),
                    _ => ('<', '>'),
                };
                let rope = doc.buffer.rope();
                // On the opener itself, or else the one around.
                let mut o = if ch(doc, p) == open {
                    p
                } else {
                    editing::scan(rope, p, false, open, close)?
                };
                for _ in 1..count.unwrap_or(1) {
                    o = editing::scan(rope, o, false, open, close)?;
                }
                let c = editing::scan(rope, o + 1, true, open, close)?;
                if around {
                    span(o, c + 1)
                } else {
                    // A block whose brackets are on lines of their own: the
                    // lines between.
                    let (ol, _) = doc.line_col(o);
                    let (cl, _) = doc.line_col(c);
                    if cl > ol + 1 && o + 1 == line_end(doc, ol) && c == first_non_blank(doc, cl) {
                        Some(Span {
                            start: doc.line_start(ol + 1),
                            end: doc.line_start(cl),
                            linewise: true,
                        })
                    } else {
                        span(o + 1, c)
                    }
                }
            }
            '"' | '\'' | '`' => {
                let (line, col) = doc.line_col(p);
                let chars = doc.line_chars(line);
                let start = doc.line_start(line);
                let quotes: Vec<usize> = chars
                    .iter()
                    .enumerate()
                    .filter(|(i, c)| **c == what && (*i == 0 || chars[i - 1] != '\\'))
                    .map(|(i, _)| i)
                    .collect();
                // The pair around the cursor, or else the first after it.
                let pair = quotes
                    .chunks(2)
                    .filter(|w| w.len() == 2)
                    .find(|w| w[0] <= col && col <= w[1])
                    .or_else(|| {
                        quotes
                            .chunks(2)
                            .filter(|w| w.len() == 2)
                            .find(|w| w[0] > col)
                    })?;
                let (a, b) = (pair[0], pair[1]);
                if around {
                    let mut e = b + 1;
                    while e < chars.len() && chars[e] == ' ' {
                        e += 1;
                    }
                    span(start + a, start + e)
                } else {
                    span(start + a + 1, start + b)
                }
            }
            'p' => {
                let (line, _) = doc.line_col(p);
                let kind = blank(doc, line);
                let mut a = line;
                while a > 0 && blank(doc, a - 1) == kind {
                    a -= 1;
                }
                let mut b = line;
                for i in 0..count.unwrap_or(1) {
                    if i > 0 {
                        b += 1;
                    }
                    let k = blank(doc, b.min(doc.last_line()));
                    while b < doc.last_line() && blank(doc, b + 1) == k {
                        b += 1;
                    }
                }
                if around {
                    while b < doc.last_line() && blank(doc, b + 1) {
                        b += 1;
                    }
                }
                Some(Span {
                    start: doc.line_start(a),
                    end: next_line(doc, b.min(doc.last_line())),
                    linewise: true,
                })
            }
            's' => {
                // From after the last full stop to the next.
                let mut s = p;
                while s > 0 && !".!?".contains(ch(doc, s - 1)) && !empty_line_at(doc, s - 1) {
                    s -= 1;
                }
                while s < p && ch(doc, s).is_whitespace() {
                    s += 1;
                }
                let mut e = p;
                while e < len && !".!?".contains(ch(doc, e)) && !empty_line_at(doc, e) {
                    e += 1;
                }
                let mut e = (e + 1).min(len);
                if around {
                    while e < len && ch(doc, e) == ' ' {
                        e += 1;
                    }
                }
                span(s, e)
            }
            'f' | 'c' => {
                // Functions and classes, from the language plugin's symbols.
                let kinds: &[&str] = if what == 'f' {
                    &["function", "method", "constructor", "fn"]
                } else {
                    &[
                        "class",
                        "struct",
                        "interface",
                        "enum",
                        "impl",
                        "object",
                        "trait",
                    ]
                };
                let symbol = doc
                    .layers
                    .symbols
                    .iter()
                    .flatten()
                    .filter(|s| s.start <= p && p < s.end)
                    .filter(|s| kinds.iter().any(|k| s.kind.to_lowercase().contains(k)))
                    .max_by_key(|s| s.depth)?;
                if around {
                    let (a, _) = doc.line_col(symbol.start);
                    let (b, _) = doc.line_col(symbol.end.saturating_sub(1));
                    Some(Span {
                        start: doc.line_start(a),
                        end: next_line(doc, b),
                        linewise: true,
                    })
                } else {
                    let text = doc.slice(symbol.start, symbol.end);
                    let open = symbol.start + text.chars().position(|c| c == '{' || c == ':')?;
                    let close = if ch(doc, open) == '{' {
                        editing::scan(doc.buffer.rope(), open + 1, true, '{', '}')?
                    } else {
                        symbol.end
                    };
                    let (ol, _) = doc.line_col(open);
                    let (cl, _) = doc.line_col(close);
                    if cl > ol + 1 {
                        Some(Span {
                            start: doc.line_start(ol + 1),
                            end: if ch(doc, open) == '{' {
                                doc.line_start(cl)
                            } else {
                                next_line(doc, cl)
                            },
                            linewise: true,
                        })
                    } else {
                        span(open + 1, close)
                    }
                }
            }
            _ => None,
        }
    }

    /// Puts text in a register, and in the clipboard where it should go.
    fn vim_yank(
        &mut self,
        reg: Option<char>,
        mut text: String,
        linewise: bool,
        deleting: bool,
        fx: &mut Vec<Effect>,
    ) {
        if reg == Some('_') {
            return;
        }
        if linewise && !text.ends_with('\n') {
            text.push('\n');
        }
        let clipboard = self.vim_settings.clipboard;
        let vim = self.vim.as_mut().expect("vim");
        let r = Register {
            text: text.clone(),
            linewise,
        };
        match reg {
            Some(c @ 'a'..='z') => {
                vim.registers.insert(c, r.clone());
            }
            Some(c @ 'A'..='Z') => {
                let entry = vim.registers.entry(c.to_ascii_lowercase()).or_default();
                entry.text.push_str(&text);
                entry.linewise |= linewise;
            }
            _ => {}
        }
        if deleting {
            if linewise || text.contains('\n') {
                for i in (1..9).rev() {
                    let from = char::from_digit(i, 10).unwrap_or('1');
                    let to = char::from_digit(i + 1, 10).unwrap_or('9');
                    if let Some(r) = vim.registers.get(&from).cloned() {
                        vim.registers.insert(to, r);
                    }
                }
                vim.registers.insert('1', r.clone());
            } else {
                vim.registers.insert('-', r.clone());
            }
        } else if reg.is_none() {
            vim.registers.insert('0', r.clone());
        }
        vim.registers.insert('"', r);
        if matches!(reg, Some('+' | '*')) || (reg.is_none() && clipboard) {
            self.clipboard = text.clone();
            fx.push(Effect::Copy(text));
        }
    }

    /// Runs an operator over what it covers. Returns whether it leaves
    /// the cursor in Insert mode.
    fn vim_operate(&mut self, op: Op, span: Span, reg: Option<char>, fx: &mut Vec<Effect>) -> bool {
        let Span {
            mut start,
            end,
            linewise,
        } = span;
        let doc = self.doc();
        let text = doc.slice(start, end);
        let (first_line, _) = doc.line_col(start);
        match op {
            Op::Yank => {
                self.vim_yank(reg, text, linewise, false, fx);
                let doc = self.doc_mut();
                if linewise {
                    let (line, col) = doc.line_col(doc.cursor);
                    if first_line < line {
                        doc.cursor =
                            doc.line_start(first_line) + col.min(doc.buffer.line_len(first_line));
                    }
                } else {
                    doc.cursor = start.min(doc.cursor).max(start);
                }
                false
            }
            Op::Delete => {
                self.vim_yank(reg, text.clone(), linewise, true, fx);
                // The last line has no line break after it: take the one
                // before.
                let len = self.doc().buffer.len();
                if linewise && end == len && start > 0 && !text.ends_with('\n') {
                    start -= 1;
                }
                self.replace(start, end, "", Kind::Other);
                let doc = self.doc_mut();
                if linewise {
                    let line = doc.line_col(start.min(doc.buffer.len())).0;
                    let line = if start < span.start { line + 1 } else { line };
                    doc.cursor = first_non_blank(doc, line.min(doc.last_line()));
                }
                false
            }
            Op::Change => {
                if linewise {
                    // The lines' text goes; the first one's indent stays.
                    let doc = self.doc();
                    let indent: String = doc
                        .line_chars(first_line)
                        .iter()
                        .take_while(|c| **c == ' ' || **c == '\t')
                        .collect();
                    let cut_end = if text.ends_with('\n') { end - 1 } else { end };
                    self.vim_yank(reg, text, true, true, fx);
                    self.replace(start, cut_end, &indent, Kind::Other);
                } else {
                    self.vim_yank(reg, text, false, true, fx);
                    self.replace(start, end, "", Kind::Other);
                }
                true
            }
            Op::Indent | Op::Outdent => {
                let doc = self.doc_mut();
                doc.anchor = Some(start);
                doc.cursor = if linewise {
                    end.saturating_sub(1).max(start)
                } else {
                    end
                };
                self.indent_lines(op == Op::Indent);
                let doc = self.doc_mut();
                doc.anchor = None;
                doc.cursor = first_non_blank(doc, first_line);
                false
            }
            Op::Lower | Op::Upper | Op::Toggle => {
                let changed: String = text
                    .chars()
                    .map(|c| match op {
                        Op::Lower => c.to_lowercase().collect::<String>(),
                        Op::Upper => c.to_uppercase().collect(),
                        _ => swap_case(c),
                    })
                    .collect();
                if changed != text {
                    self.replace(start, end, &changed, Kind::Other);
                }
                self.doc_mut().cursor = start;
                false
            }
            Op::Comment => {
                let doc = self.doc_mut();
                doc.anchor = Some(start);
                doc.cursor = end.saturating_sub(1).max(start);
                self.toggle_comment();
                let doc = self.doc_mut();
                doc.anchor = None;
                doc.cursor = first_non_blank(doc, first_line.min(doc.last_line()));
                false
            }
        }
    }

    /// What a register holds, as text for a macro.
    pub(crate) fn vim_register_text(&self, r: char) -> Option<String> {
        let reg = self.vim.as_ref()?.registers.get(&r)?;
        let mut text = reg.text.clone();
        if reg.linewise && text.ends_with('\n') {
            text.pop();
        }
        Some(text)
    }

    pub(crate) fn vim_set_register(&mut self, r: char, text: &str) {
        if let Some(vim) = &mut self.vim {
            vim.registers.insert(
                r,
                Register {
                    text: text.to_string(),
                    linewise: false,
                },
            );
        }
    }

    /// Back to Normal mode from wherever Vim is, as between a macro's
    /// cursors.
    pub(crate) fn vim_reset(&mut self) {
        let Some(vim) = &mut self.vim else {
            return;
        };
        vim.pending.clear();
        vim.pending_keys.clear();
        vim.line = None;
        match vim.mode() {
            VimMode::Insert | VimMode::Replace => self.vim_leave_insert(&mut Vec::new()),
            m if m.visual() => {
                self.vim_leave_visual();
                self.vim_clamp();
            }
            _ => {}
        }
    }

    pub(crate) fn vim_enter(&mut self, mode: VimMode) {
        let vim = self.vim.as_mut().expect("vim");
        vim.mode = Some(mode);
        vim.goal = None;
        vim.escape = None;
        if !mode.visual() {
            self.doc_mut().anchor = None;
        }
    }

    /// Normal mode's other commands.
    fn vim_do(
        &mut self,
        keys: &str,
        reg: Option<char>,
        count: Option<usize>,
        n: usize,
        fx: &mut Vec<Effect>,
    ) {
        let mut chars = keys.chars();
        let first = chars.next().unwrap_or(' ');
        let second = chars.next();
        let height = self.text_height().max(1);
        match (first, second) {
            (ESC, None) => {
                if self.has_carets() {
                    self.clear_carets();
                }
                self.vim.as_mut().expect("vim").highlight = false;
            }
            (':', None) => {
                let range = match count {
                    Some(n) if n > 1 => format!(".,.+{}", n - 1),
                    _ => String::new(),
                };
                self.vim_open_line(':', &range);
            }
            ('/' | '?', None) => self.vim_open_line(first, ""),
            ('q', Some(r)) if r.is_ascii_alphanumeric() || r == '"' => self.macro_record(Some(r)),
            ('@', Some(r)) => {
                let r = match r {
                    '@' => match self.last_macro_register {
                        Some(r) => r,
                        None => return self.macro_fail("no macro had run yet".into()),
                    },
                    r => r.to_ascii_lowercase(),
                };
                match self.vim_register_text(r) {
                    Some(text) if !text.is_empty() => {
                        self.last_macro_register = Some(r);
                        self.macro_play(format!("@{r}"), &text, n);
                    }
                    _ => {
                        self.macro_fail(format!("@{r} was empty"));
                        self.vim.as_mut().expect("vim").note = Some(format!("@{r} is empty"));
                    }
                }
            }
            ('&', None) => self.vim_ex("s", None, fx),
            ('i', None) => self.vim_enter(VimMode::Insert),
            ('a', None) => {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc_mut();
                    if ch(doc, doc.cursor) != '\n' && doc.cursor < doc.buffer.len() {
                        doc.cursor += 1;
                    }
                });
                self.vim_enter(VimMode::Insert);
            }
            ('I', None) | ('g', Some('I')) => {
                let plain = second == Some('I');
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc_mut();
                    let line = doc.line_col(doc.cursor).0;
                    doc.cursor = if plain {
                        doc.line_start(line)
                    } else {
                        first_non_blank(doc, line)
                    };
                });
                self.vim_enter(VimMode::Insert);
            }
            ('A', None) => {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc_mut();
                    let line = doc.line_col(doc.cursor).0;
                    doc.cursor = line_end(doc, line);
                });
                self.vim_enter(VimMode::Insert);
            }
            ('o' | 'O', None) => {
                self.each_caret(Kind::Other, |e, _| e.insert_line(first == 'o'));
                self.vim_enter(VimMode::Insert);
            }
            ('x' | DELETE, None) => self.vim_op_motion(Op::Delete, "l", count, reg, fx),
            ('X', None) => self.vim_op_motion(Op::Delete, "h", count, reg, fx),
            ('s', None) => {
                self.vim_op_motion(Op::Change, "l", count, reg, fx);
                self.vim_enter(VimMode::Insert);
            }
            ('S', None) => self.vim_op_lines(Op::Change, n, reg, fx),
            ('C', None) => {
                self.vim_op_motion(Op::Change, "$", count, reg, fx);
                self.vim_enter(VimMode::Insert);
            }
            ('D', None) => self.vim_op_motion(Op::Delete, "$", count, reg, fx),
            ('Y', None) => self.vim_op_lines(Op::Yank, n, reg, fx),
            ('p' | 'P', None) => self.vim_put(reg, first == 'P', n, fx),
            ('J', None) | ('g', Some('J')) => {
                let spaces = second.is_none();
                self.each_caret(Kind::Other, |e, _| e.vim_join(n.max(2) - 1, spaces));
            }
            ('u', None) => {
                for _ in 0..n {
                    self.run("edit.undo", fx);
                }
                self.doc_mut().anchor = None;
                self.clear_carets();
            }
            ('\x12', None) => {
                for _ in 0..n {
                    self.run("edit.redo", fx);
                }
                self.doc_mut().anchor = None;
                self.clear_carets();
            }
            ('.', None) => self.vim_repeat(count, fx),
            ('r', Some(c)) => {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc();
                    let at = doc.cursor;
                    let (line, col) = doc.line_col(at);
                    if col + n > doc.buffer.line_len(line) {
                        return;
                    }
                    if c == ENTER {
                        e.replace(at, at + n, "\n", Kind::Other);
                    } else {
                        let with: String = std::iter::repeat_n(c, n).collect();
                        e.replace(at, at + n, &with, Kind::Other);
                        e.doc_mut().cursor = at + n - 1;
                    }
                });
            }
            ('R', None) => self.vim_enter(VimMode::Replace),
            ('~', None) => {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc();
                    let at = doc.cursor;
                    let end = line_end(doc, doc.line_col(at).0).min(at + n);
                    if at >= end {
                        return;
                    }
                    let text: String = doc.slice(at, end).chars().map(swap_case).collect();
                    e.replace(at, end, &text, Kind::Other);
                    e.doc_mut().cursor = end;
                });
            }
            ('v', None) => self.vim_visual_start(VimMode::Visual),
            ('V', None) => self.vim_visual_start(VimMode::VisualLine),
            ('\x16', None) => self.vim_visual_start(VimMode::VisualBlock),
            ('g', Some('v')) => {
                let Some((mode, anchor, cursor)) = self.vim.as_ref().and_then(|v| v.last_visual)
                else {
                    return;
                };
                let len = self.doc().buffer.len();
                let vim = self.vim.as_mut().expect("vim");
                vim.mode = Some(mode);
                vim.anchor = anchor.min(len);
                vim.cursor = cursor.min(len);
                self.vim_show_visual();
            }
            ('m', Some(c)) => {
                let (id, at) = (self.doc().id, self.doc().cursor);
                self.vim.as_mut().expect("vim").marks.insert(c, (id, at));
            }
            ('\x01' | '\x18', None) => {
                let by = if first == '\x01' {
                    n as i64
                } else {
                    -(n as i64)
                };
                self.each_caret(Kind::Other, |e, _| e.vim_add(by));
            }
            ('z', Some('o')) => self.run("edit.unfold", fx),
            ('z', Some('c')) => self.run("edit.fold", fx),
            ('z', Some('a')) => {
                let line = self.doc().line_col(self.doc().cursor).0;
                self.toggle_fold(line);
            }
            ('z', Some('R')) => self.run("edit.unfold_all", fx),
            ('z', Some('M')) => self.run("edit.fold_all", fx),
            (']' | '[', Some('c')) => {
                let id = if first == ']' {
                    "go.next_change"
                } else {
                    "go.prev_change"
                };
                for _ in 0..n {
                    self.run(id, fx);
                }
            }
            (']' | '[', Some('d')) => self.vim_problem(first == ']', n),
            ('\x17', Some(c)) => {
                let groups = self.groups.len();
                match c {
                    'w' | 'l' | 'j' | '\x17' => self.focus_group((self.group + 1) % groups),
                    'W' | 'h' | 'k' => self.focus_group((self.group + groups - 1) % groups),
                    'v' => self.run("view.split", fx),
                    's' => self.run("view.split_down", fx),
                    'o' => self.run("file.close_others", fx),
                    _ => self.run("file.close", fx),
                }
            }
            ('z', Some(c)) => {
                let doc = self.doc_mut();
                let line = doc.line_col(doc.cursor).0;
                doc.scroll = match c {
                    't' | '\r' => line,
                    'b' | '-' => (line + 1).saturating_sub(height),
                    _ => line.saturating_sub(height / 2),
                };
                if c == '\r' || c == '.' || c == '-' {
                    doc.cursor = first_non_blank(doc, line);
                }
            }
            ('\x05' | '\x19', None) => {
                let doc = self.doc_mut();
                let last = doc.last_line();
                doc.scroll = if first == '\x05' {
                    (doc.scroll + n).min(last)
                } else {
                    doc.scroll.saturating_sub(n)
                };
                let (line, col) = doc.line_col(doc.cursor);
                let keep = line.clamp(doc.scroll, (doc.scroll + height - 1).min(last));
                if keep != line {
                    doc.cursor = doc.pos_at(keep, col);
                }
                self.snap_scroll();
            }
            ('\x04' | '\x15' | '\x06' | '\x02' | PAGE_UP | PAGE_DOWN, None) => {
                let half = matches!(first, '\x04' | '\x15');
                let rows = if half {
                    count.unwrap_or(height / 2).max(1)
                } else {
                    height.saturating_sub(2).max(1) * n
                };
                let down = matches!(first, '\x04' | '\x06' | PAGE_DOWN);
                let doc = self.doc_mut();
                let last = doc.last_line();
                let (line, _) = doc.line_col(doc.cursor);
                let target = if down {
                    (line + rows).min(last)
                } else {
                    line.saturating_sub(rows)
                };
                doc.scroll = if down {
                    (doc.scroll + rows).min(last)
                } else {
                    doc.scroll.saturating_sub(rows)
                };
                doc.cursor = first_non_blank(doc, target);
                self.snap_scroll();
            }
            ('K', None) | ('g', Some('h')) => self.run("view.hover", fx),
            ('g', Some('d')) => self.run("go.definition", fx),
            ('g', Some('D')) => self.run("go.definition", fx),
            ('g', Some('r')) => self.run("go.references", fx),
            ('g', Some('a')) => self.run("edit.quick_fix", fx),
            ('g', Some('t')) => self.run("file.next", fx),
            ('g', Some('T')) => self.run("file.prev", fx),
            ('g', Some('i')) => {
                if let Some(&(id, at)) = self.vim.as_ref().and_then(|v| v.marks.get(&'^'))
                    && id == self.doc().id
                {
                    self.set_cursor(at.min(self.doc().buffer.len()));
                }
                self.vim_enter(VimMode::Insert);
            }
            ('\x0f', None) => self.run("nav.back", fx),
            ('Z', Some('Z')) => {
                self.run("file.save", fx);
                self.run("file.close", fx);
            }
            ('Z', Some('Q')) => self.run("file.close", fx),
            _ => {}
        }
    }

    /// `]d` and `[d`: to the next or previous problem.
    fn vim_problem(&mut self, forward: bool, n: usize) {
        let doc = self.doc();
        let mut starts: Vec<usize> = doc.marks.iter().map(|m| m.start).collect();
        starts.sort();
        starts.dedup();
        if starts.is_empty() {
            return self.vim_note("No problems in this file".into());
        }
        let mut at = doc.cursor;
        for _ in 0..n {
            at = if forward {
                starts
                    .iter()
                    .copied()
                    .find(|&s| s > at)
                    .unwrap_or(starts[0])
            } else {
                starts
                    .iter()
                    .rev()
                    .copied()
                    .find(|&s| s < at)
                    .unwrap_or(starts[starts.len() - 1])
            };
        }
        self.set_cursor(at);
    }

    /// The scroll stays inside the text.
    fn snap_scroll(&mut self) {
        let doc = self.doc_mut();
        doc.scroll = doc.scroll.min(doc.last_line());
    }

    fn vim_op_motion(
        &mut self,
        op: Op,
        m: &str,
        count: Option<usize>,
        reg: Option<char>,
        fx: &mut Vec<Effect>,
    ) {
        self.each_caret(Kind::Other, |e, _| {
            if let Some(span) = e.vim_motion_span(m, count, op)
                && span.start < span.end
            {
                e.vim_operate(op, span, reg, fx);
            }
        });
    }

    fn vim_op_lines(&mut self, op: Op, n: usize, reg: Option<char>, fx: &mut Vec<Effect>) {
        let mut enter = false;
        self.each_caret(Kind::Other, |e, _| {
            let doc = e.doc();
            let line = doc.line_col(doc.cursor).0;
            let last = (line + n - 1).min(doc.last_line());
            let span = Span {
                start: doc.line_start(line),
                end: next_line(doc, last),
                linewise: true,
            };
            enter |= e.vim_operate(op, span, reg, fx);
        });
        if enter {
            self.vim_enter(VimMode::Insert);
        }
    }

    /// `J`: the next lines join this one, with a space between unless
    /// `spaces` is off.
    fn vim_join(&mut self, times: usize, spaces: bool) {
        for _ in 0..times {
            let doc = self.doc();
            let line = doc.line_col(doc.cursor).0;
            if line >= doc.last_line() {
                return;
            }
            let at = line_end(doc, line);
            let next = doc.line_chars(line + 1);
            let lead = if spaces {
                next.iter().take_while(|c| c.is_whitespace()).count()
            } else {
                0
            };
            let rest_empty = lead == next.len();
            let ends_space = at > doc.line_start(line) && ch(doc, at - 1) == ' ';
            let with = if !spaces || rest_empty || ends_space || next.get(lead) == Some(&')') {
                ""
            } else {
                " "
            };
            let next_start = doc.line_start(line + 1);
            self.replace(at, next_start + lead, with, Kind::Other);
            self.doc_mut().cursor = at;
        }
    }

    /// Ctrl+A and Ctrl+X: the number at or after the cursor goes up or
    /// down.
    fn vim_add(&mut self, by: i64) {
        let doc = self.doc();
        let (line, col) = doc.line_col(doc.cursor);
        let chars = doc.line_chars(line);
        let Some(mut s) = (col..chars.len()).find(|&i| chars[i].is_ascii_digit()) else {
            return;
        };
        while s > 0 && chars[s - 1].is_ascii_digit() {
            s -= 1;
        }
        if s > 0 && chars[s - 1] == '-' {
            s -= 1;
        }
        let mut e = s + 1;
        while e < chars.len() && chars[e].is_ascii_digit() {
            e += 1;
        }
        let text: String = chars[s..e].iter().collect();
        let Ok(value) = text.parse::<i64>() else {
            return;
        };
        let new = value.saturating_add(by).to_string();
        let start = doc.line_start(line);
        self.replace(start + s, start + e, &new, Kind::Other);
        self.doc_mut().cursor = start + s + new.chars().count() - 1;
    }

    /// `p` and `P`: what a register holds, after or before the cursor.
    fn vim_put(&mut self, reg: Option<char>, before: bool, n: usize, fx: &mut Vec<Effect>) {
        if matches!(reg, Some('+' | '*')) || (reg.is_none() && self.vim_settings.clipboard) {
            self.vim.as_mut().expect("vim").put = Some((before, n));
            fx.push(Effect::ReadClipboard);
            return;
        }
        let name = reg.map_or('"', |r| r.to_ascii_lowercase());
        let Some(r) = self
            .vim
            .as_ref()
            .and_then(|v| v.registers.get(&name))
            .cloned()
        else {
            return self.notify(format!("Nothing in register {name}"));
        };
        self.vim_put_text(&r.text, r.linewise, before, n);
    }

    /// The clipboard's text, for a put that waited for it. Returns whether
    /// there was one.
    pub(crate) fn vim_clipboard_put(&mut self, text: &str) -> bool {
        let Some((before, n)) = self.vim.as_mut().and_then(|v| v.put.take()) else {
            return false;
        };
        let text = text.replace("\r\n", "\n");
        let unnamed = self
            .vim
            .as_ref()
            .and_then(|v| v.registers.get(&'"'))
            .cloned();
        let linewise = match unnamed {
            Some(r) if r.text == text => r.linewise,
            _ => text.ends_with('\n'),
        };
        let steps = self.doc().buffer.undo_steps();
        self.vim_put_text(&text, linewise, before, n);
        self.doc_mut().buffer.join_undo(steps);
        self.vim_clamp();
        true
    }

    fn vim_put_text(&mut self, text: &str, linewise: bool, before: bool, n: usize) {
        let mode = self.vim_mode().unwrap_or(VimMode::Normal);
        let all = text.repeat(n);
        if mode.visual() {
            // The selection goes, and the text takes its place.
            let span = self.vim_visual_span();
            let doc = self.doc();
            let old = doc.slice(span.start, span.end);
            let mut with = all.clone();
            if span.linewise && !with.ends_with('\n') {
                with.push('\n');
            } else if !span.linewise && linewise {
                with = with.trim_end_matches('\n').to_string();
            }
            self.replace(span.start, span.end, &with, Kind::Other);
            self.doc_mut().cursor = span.start;
            let vim = self.vim.as_mut().expect("vim");
            vim.registers.insert(
                '"',
                Register {
                    text: old,
                    linewise: span.linewise,
                },
            );
            self.vim_enter(VimMode::Normal);
            return;
        }
        let all = &all;
        self.each_caret(Kind::Other, |e, _| {
            let doc = e.doc();
            let (line, _) = doc.line_col(doc.cursor);
            if linewise {
                let (at, with) = if before {
                    (doc.line_start(line), all.clone())
                } else if line < doc.last_line() {
                    (doc.line_start(line + 1), all.clone())
                } else {
                    // The last line: a line break first.
                    (
                        doc.buffer.len(),
                        format!("\n{}", all.trim_end_matches('\n')),
                    )
                };
                e.replace(at, at, &with, Kind::Other);
                let doc = e.doc_mut();
                let first = doc.line_col(at).0 + (with.starts_with('\n') as usize);
                doc.cursor = first_non_blank(doc, first.min(doc.last_line()));
            } else {
                let at = if before || ch(doc, doc.cursor) == '\n' || doc.cursor >= doc.buffer.len()
                {
                    doc.cursor
                } else {
                    doc.cursor + 1
                };
                e.replace(at, at, all, Kind::Other);
                let doc = e.doc_mut();
                doc.cursor = if all.contains('\n') {
                    at
                } else {
                    at + all.chars().count().saturating_sub(1)
                };
            }
        });
    }

    /// `.`: the last change again, with a new count if given.
    fn vim_repeat(&mut self, count: Option<usize>, fx: &mut Vec<Effect>) {
        let Some(vim) = self.vim.as_mut() else {
            return;
        };
        let mut keys = vim.last_change.clone();
        if keys.is_empty() {
            return;
        }
        if let Some(count) = count {
            // The old count goes, after any register.
            let skip = if keys.first().is_some_and(|k| k.code == KeyCode::Char('"')) {
                2
            } else {
                0
            };
            let digits = keys[skip..]
                .iter()
                .take_while(|k| matches!(k.code, KeyCode::Char(c) if c.is_ascii_digit()))
                .count();
            keys.drain(skip..skip + digits);
            for (i, c) in count.to_string().chars().enumerate() {
                keys.insert(skip + i, Key::plain(KeyCode::Char(c)));
            }
        }
        vim.replaying = true;
        let steps = self.doc().buffer.undo_steps();
        for key in keys.clone() {
            if !self.vim_key(key, fx) {
                self.editor_key(key, fx);
            }
        }
        let vim = self.vim.as_mut().expect("vim");
        vim.replaying = false;
        vim.last_change = keys;
        // In Insert mode still, if the change was cut short.
        if vim.mode() == VimMode::Insert {
            self.vim_leave_insert(fx);
        }
        self.doc_mut().buffer.join_undo(steps);
    }

    fn vim_visual_start(&mut self, mode: VimMode) {
        self.clear_carets();
        let cursor = self.doc().cursor;
        let vim = self.vim.as_mut().expect("vim");
        vim.mode = Some(mode);
        vim.anchor = cursor;
        vim.cursor = cursor;
        vim.goal = None;
        self.vim_show_visual();
    }

    /// What Visual mode covers, from where it began to its cursor and
    /// taking both.
    fn vim_visual_span(&self) -> Span {
        let vim = self.vim.as_ref().expect("vim");
        let doc = self.doc();
        let (a, c) = (vim.anchor.min(vim.cursor), vim.anchor.max(vim.cursor));
        match vim.mode() {
            VimMode::VisualLine => Span {
                start: doc.line_start(doc.line_col(a).0),
                end: next_line(doc, doc.line_col(c).0),
                linewise: true,
            },
            _ => Span {
                start: a,
                end: if ch(doc, c) == '\n' {
                    c + 1
                } else {
                    (c + 1).min(doc.buffer.len())
                },
                linewise: false,
            },
        }
        .clamp(doc.buffer.len())
    }

    /// The lines and columns Visual block mode covers.
    fn vim_block(&self) -> (usize, usize, usize, usize) {
        let vim = self.vim.as_ref().expect("vim");
        let doc = self.doc();
        let (al, ac) = doc.line_col(vim.anchor);
        let (cl, cc) = doc.line_col(vim.cursor);
        let (ad, cd) = (doc.display_col(al, ac), doc.display_col(cl, cc));
        let right = if vim.goal == Some(usize::MAX) {
            usize::MAX
        } else {
            ad.max(cd)
        };
        (al.min(cl), al.max(cl), ad.min(cd), right)
    }

    /// Visual mode's selection, drawn as merlin's.
    fn vim_show_visual(&mut self) {
        let vim = self.vim.as_ref().expect("vim");
        let (mode, anchor, cursor) = (vim.mode(), vim.anchor, vim.cursor);
        let doc = self.doc_mut();
        doc.extra.clear();
        let len = doc.buffer.len();
        match mode {
            VimMode::Visual => {
                if cursor >= anchor {
                    doc.anchor = Some(anchor);
                    doc.cursor = (cursor + 1).min(len);
                } else {
                    doc.anchor = Some((anchor + 1).min(len));
                    doc.cursor = cursor;
                }
            }
            VimMode::VisualLine => {
                let (al, cl) = (doc.line_col(anchor).0, doc.line_col(cursor).0);
                if cl >= al {
                    doc.anchor = Some(doc.line_start(al));
                    doc.cursor = next_line(doc, cl);
                } else {
                    doc.anchor = Some(next_line(doc, al));
                    doc.cursor = doc.line_start(cl);
                }
            }
            VimMode::VisualBlock => {
                let (top, bottom, left, right) = self.vim_block();
                let doc = self.doc_mut();
                let cursor_line = doc.line_col(cursor).0;
                let mut carets = Vec::new();
                for line in top..=bottom {
                    let s = doc.pos_at(line, left);
                    let e = if right == usize::MAX {
                        line_end(doc, line)
                    } else {
                        (doc.pos_at(line, right) + 1).min(line_end(doc, line))
                    };
                    if doc.display_col(line, s - doc.line_start(line)) < left && line != cursor_line
                    {
                        continue;
                    }
                    let caret = crate::doc::Caret {
                        cursor: e.max(s),
                        anchor: (e > s).then_some(s),
                    };
                    carets.push((line == cursor_line, caret));
                }
                let primary = carets.iter().position(|(p, _)| *p).unwrap_or_default();
                let (_, main) = carets.remove(primary);
                doc.cursor = main.cursor;
                doc.anchor = main.anchor;
                doc.extra = carets.into_iter().map(|(_, c)| c).collect();
                doc.extra_version += 1;
            }
            _ => {}
        }
    }

    fn vim_leave_visual(&mut self) {
        let vim = self.vim.as_mut().expect("vim");
        vim.last_visual = Some((vim.mode(), vim.anchor, vim.cursor));
        vim.mode = Some(VimMode::Normal);
        let cursor = vim.cursor;
        self.clear_carets();
        let doc = self.doc_mut();
        doc.anchor = None;
        doc.cursor = cursor.min(doc.buffer.len());
    }

    fn vim_visual(&mut self, cmd: Cmd, fx: &mut Vec<Effect>) {
        let n = cmd.count.unwrap_or(1);
        let mode = self.vim_mode().unwrap_or(VimMode::Visual);
        let reg = cmd.reg;
        match cmd.what {
            What::Move(m) => {
                if let Some((to, _)) = self.vim_motion(&m, cmd.count, None) {
                    self.vim.as_mut().expect("vim").cursor = to;
                    self.vim_show_visual();
                }
            }
            What::Object(o) => {
                if let Some(span) = self.vim_object(&o, cmd.count) {
                    let vim = self.vim.as_mut().expect("vim");
                    vim.anchor = span.start;
                    vim.cursor = span.end.saturating_sub(1).max(span.start);
                    if span.linewise && vim.mode() == VimMode::Visual {
                        vim.mode = Some(VimMode::VisualLine);
                    }
                    self.vim_show_visual();
                }
            }
            What::Op(..) => {}
            What::Do(keys) => {
                let mut chars = keys.chars();
                let first = chars.next().unwrap_or(' ');
                let second = chars.next();
                let block = mode == VimMode::VisualBlock;
                let op = match (first, second) {
                    ('d' | 'x', None) => Some(Op::Delete),
                    ('c' | 's', None) => Some(Op::Change),
                    ('y', None) => Some(Op::Yank),
                    ('>', None) => Some(Op::Indent),
                    ('<', None) => Some(Op::Outdent),
                    ('~', None) | ('g', Some('~')) => Some(Op::Toggle),
                    ('u', None) | ('g', Some('u')) => Some(Op::Lower),
                    ('U', None) | ('g', Some('U')) => Some(Op::Upper),
                    ('g', Some('c')) => Some(Op::Comment),
                    _ => None,
                };
                let linewise_op = matches!(first, 'X' | 'D' | 'Y' | 'S' | 'R') && !block;
                let op = op.or(match first {
                    'X' | 'D' => Some(Op::Delete),
                    'Y' => Some(Op::Yank),
                    'S' | 'R' | 'C' => Some(Op::Change),
                    _ => None,
                });
                if let Some(op) = op {
                    if block && !matches!(op, Op::Indent | Op::Outdent | Op::Comment) {
                        return self.vim_block_op(op, first == 'C' || first == 'D', reg, fx);
                    }
                    let mut span = self.vim_visual_span();
                    if linewise_op || matches!(op, Op::Indent | Op::Outdent | Op::Comment) || block
                    {
                        let doc = self.doc();
                        let (a, b) = (
                            doc.line_col(span.start).0,
                            doc.line_col(span.end.saturating_sub(1).max(span.start)).0,
                        );
                        span = Span {
                            start: doc.line_start(a),
                            end: next_line(doc, b),
                            linewise: true,
                        };
                    }
                    self.vim_leave_visual();
                    let times = if matches!(op, Op::Indent | Op::Outdent) {
                        n
                    } else {
                        1
                    };
                    let mut enter = false;
                    for _ in 0..times {
                        enter = self.vim_operate(op, span, reg, fx);
                    }
                    if !enter && op != Op::Yank {
                        let start = span.start.min(self.doc().buffer.len());
                        if !matches!(op, Op::Indent | Op::Outdent | Op::Comment) || !span.linewise {
                            self.doc_mut().cursor = start;
                        }
                    }
                    if enter {
                        self.vim_enter(VimMode::Insert);
                    }
                    return;
                }
                match (first, second) {
                    (ESC, None) => self.vim_leave_visual(),
                    ('v' | 'V' | '\x16', None) => {
                        let want = match first {
                            'v' => VimMode::Visual,
                            'V' => VimMode::VisualLine,
                            _ => VimMode::VisualBlock,
                        };
                        if want == mode {
                            let cursor = self.vim.as_ref().map_or(0, |v| v.cursor);
                            self.vim_leave_visual();
                            self.doc_mut().cursor = cursor;
                        } else {
                            self.vim.as_mut().expect("vim").mode = Some(want);
                            self.vim_show_visual();
                        }
                    }
                    ('o' | 'O', None) => {
                        let vim = self.vim.as_mut().expect("vim");
                        std::mem::swap(&mut vim.anchor, &mut vim.cursor);
                        self.vim_show_visual();
                    }
                    ('J', None) | ('g', Some('J')) => {
                        let span = self.vim_visual_span();
                        let doc = self.doc();
                        let (a, b) = (
                            doc.line_col(span.start).0,
                            doc.line_col(span.end.saturating_sub(1).max(span.start)).0,
                        );
                        self.vim_leave_visual();
                        self.set_cursor(self.doc().line_start(a));
                        self.vim_join((b - a).max(1), second.is_none());
                    }
                    ('p' | 'P', None) => {
                        let name = reg.map_or('"', |r| r.to_ascii_lowercase());
                        if matches!(reg, Some('+' | '*'))
                            || (reg.is_none() && self.vim_settings.clipboard)
                        {
                            self.vim.as_mut().expect("vim").put = Some((first == 'P', n));
                            fx.push(Effect::ReadClipboard);
                            return;
                        }
                        if let Some(r) = self
                            .vim
                            .as_ref()
                            .and_then(|v| v.registers.get(&name))
                            .cloned()
                        {
                            self.vim_put_text(&r.text, r.linewise, false, n);
                        }
                    }
                    ('r', Some(c)) => {
                        let span = self.vim_visual_span();
                        let text: String = self
                            .doc()
                            .slice(span.start, span.end)
                            .chars()
                            .map(|x| if x == '\n' { '\n' } else { c })
                            .collect();
                        self.vim_leave_visual();
                        self.replace(span.start, span.end, &text, Kind::Other);
                        self.doc_mut().cursor = span.start;
                    }
                    ('I' | 'A', None) if block => {
                        let (top, bottom, left, right) = self.vim_block();
                        self.vim_leave_visual();
                        let doc = self.doc();
                        let mut carets = Vec::new();
                        for line in top..=bottom {
                            let at = if first == 'I' {
                                doc.pos_at(line, left)
                            } else if right == usize::MAX {
                                line_end(doc, line)
                            } else {
                                (doc.pos_at(line, right) + 1).min(line_end(doc, line))
                            };
                            carets.push(crate::doc::Caret::at(at));
                        }
                        self.put_cursors(&carets);
                        self.vim_enter(VimMode::Insert);
                    }
                    ('I' | 'A', None) => {
                        let span = self.vim_visual_span();
                        self.vim_leave_visual();
                        let at = if first == 'I' {
                            span.start
                        } else {
                            span.end.min(self.doc().buffer.len())
                        };
                        self.set_cursor(at);
                        self.vim_enter(VimMode::Insert);
                    }
                    (':', None) => {
                        let span = self.vim_visual_span();
                        let id = self.doc().id;
                        let end = span.end.saturating_sub(1).max(span.start);
                        let vim = self.vim.as_mut().expect("vim");
                        vim.marks.insert('<', (id, span.start));
                        vim.marks.insert('>', (id, end));
                        self.vim_leave_visual();
                        self.vim_open_line(':', "'<,'>");
                    }
                    ('/' | '?', None) => self.vim_open_line(first, ""),
                    _ => {}
                }
            }
        }
    }

    /// An operator over Visual block mode's columns.
    fn vim_block_op(&mut self, op: Op, to_end: bool, reg: Option<char>, fx: &mut Vec<Effect>) {
        let (top, bottom, left, right) = self.vim_block();
        let right = if to_end { usize::MAX } else { right };
        self.vim_leave_visual();
        let doc = self.doc();
        let mut pieces = Vec::new();
        let mut spans = Vec::new();
        for line in top..=bottom {
            let s = doc.pos_at(line, left);
            let e = if right == usize::MAX {
                line_end(doc, line)
            } else {
                (doc.pos_at(line, right) + 1).min(line_end(doc, line))
            };
            let e = e.max(s);
            pieces.push(doc.slice(s, e));
            spans.push((s, e));
        }
        let deleting = op != Op::Yank;
        self.vim_yank(reg, pieces.join("\n"), false, deleting, fx);
        match op {
            Op::Yank => self.set_cursor(spans[0].0),
            Op::Delete | Op::Change => {
                let edits: Vec<Edit> = spans
                    .iter()
                    .filter(|(s, e)| s < e)
                    .map(|&(s, e)| Edit {
                        at: s,
                        removed: self.doc().slice(s, e),
                        inserted: String::new(),
                    })
                    .collect();
                if !edits.is_empty() {
                    self.apply_all(edits.clone());
                }
                // Each line's start, after the edits above it.
                let carets: Vec<crate::doc::Caret> = spans
                    .iter()
                    .map(|&(s, _)| crate::doc::Caret::at(editing::map_pos(s, &edits)))
                    .collect();
                if op == Op::Change {
                    self.put_cursors(&carets);
                    self.vim_enter(VimMode::Insert);
                } else {
                    self.set_cursor(carets[0].cursor);
                }
            }
            _ => {
                let mut edits = Vec::new();
                for &(s, e) in &spans {
                    let text = self.doc().slice(s, e);
                    let changed: String = text
                        .chars()
                        .map(|c| match op {
                            Op::Lower => c.to_lowercase().collect::<String>(),
                            Op::Upper => c.to_uppercase().collect(),
                            _ => swap_case(c),
                        })
                        .collect();
                    if changed != text {
                        edits.push(Edit {
                            at: s,
                            removed: text,
                            inserted: changed,
                        });
                    }
                }
                if !edits.is_empty() {
                    self.apply_all(edits);
                }
                self.set_cursor(spans[0].0);
            }
        }
    }
}

impl Span {
    fn clamp(self, len: usize) -> Self {
        Span {
            start: self.start.min(len),
            end: self.end.min(len),
            linewise: self.linewise,
        }
    }
}

/// Whether the keys so far are an operator waiting for its motion.
/// Motions that fail when they can't move, which stops a macro.
fn must_move(m: &str) -> bool {
    m.trim_start_matches(|c: char| c.is_ascii_digit())
        .starts_with(|c: char| {
            "hjklwWbBeE{}nN*#fFtT;,+-\r \x7f".contains(c) || (LEFT..=DOWN).contains(&c)
        })
}

/// Keys as the status bar shows them.
fn show_keys(keys: &str) -> String {
    keys.chars()
        .map(|c| match c {
            ESC => "⎋".to_string(),
            ENTER => "↵".to_string(),
            c if (c as u32) < 27 => format!("^{}", ((c as u8) + b'A' - 1) as char),
            c => c.to_string(),
        })
        .collect()
}

fn parse_waits_for_motion(pending: &[char]) -> bool {
    let mut i = 0;
    if pending.first() == Some(&'"') {
        i = 2;
    }
    let _ = count_at(pending, &mut i);
    let rest = pending.get(i..).unwrap_or_default();
    match operator(rest) {
        Ok(Some((_, n))) => {
            let mut j = n;
            let _ = count_at(rest, &mut j);
            matches!(parse(pending, false), Parsed::Wait) && j >= n
        }
        _ => false,
    }
}

/// `f`, `F`, `t` and `T`: the `n`th `target` on the line.
fn find_in_line(doc: &Doc, p: usize, kind: char, target: char, n: usize) -> Option<usize> {
    let (line, col) = doc.line_col(p);
    let chars = doc.line_chars(line);
    let start = doc.line_start(line);
    let mut at = col;
    let forward = kind == 'f' || kind == 't';
    for i in 0..n {
        // `t` again from just before a match stays: look past it.
        let from = if i == 0 && kind == 't' { at + 1 } else { at };
        at = if forward {
            (from + 1..chars.len()).find(|&i| chars[i] == target)?
        } else {
            let from = if i == 0 && kind == 'T' {
                at.checked_sub(1)?
            } else {
                at
            };
            (0..from).rev().find(|&i| chars[i] == target)?
        };
    }
    Some(
        start
            + match kind {
                't' => at - 1,
                'T' => at + 1,
                _ => at,
            },
    )
}
