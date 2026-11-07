//! Vim's keys in the text: Normal, Insert, Visual and Replace modes, with
//! motions, operators, text objects, counts, registers and the dot, as Vim
//! has them. merlin's own typing does Insert mode, so completion, closing
//! brackets and several cursors work there as they do without Vim.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::*;
use crate::doc::Doc;

mod line;
mod motion;
mod operate;
mod parse;
mod visual;
mod which;
pub use line::{VimLineItem, VimLineView};
use line::{search_regex, vim_regex};
use motion::*;
use parse::*;
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
