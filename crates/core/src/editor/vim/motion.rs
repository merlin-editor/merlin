//! Where Vim's motions and text objects go: words, lines, paragraphs,
//! brackets and quotes, and `f`, `t` and the rest along a line.

use super::*;

pub(super) fn ch(doc: &Doc, p: usize) -> char {
    doc.buffer.rope().get_char(p).unwrap_or('\n')
}

pub(super) fn line_end(doc: &Doc, line: usize) -> usize {
    doc.line_start(line) + doc.buffer.line_len(line)
}

/// Where the next line starts, or the end of the text.
pub(super) fn next_line(doc: &Doc, line: usize) -> usize {
    if line < doc.last_line() {
        doc.line_start(line + 1)
    } else {
        doc.buffer.len()
    }
}

pub(super) fn first_non_blank(doc: &Doc, line: usize) -> usize {
    let indent = doc
        .line_chars(line)
        .iter()
        .take_while(|c| **c == ' ' || **c == '\t')
        .count();
    doc.line_start(line) + indent
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub(super) enum Class {
    Space,
    Word,
    Punct,
}

pub(super) fn class(c: char, big: bool) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if big || c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// An empty line starts at `p`.
pub(super) fn empty_line_at(doc: &Doc, p: usize) -> bool {
    ch(doc, p) == '\n' && (p == 0 || ch(doc, p - 1) == '\n') && p < doc.buffer.len()
}

pub(super) fn word_forward(doc: &Doc, p: usize, big: bool) -> usize {
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

pub(super) fn word_end(doc: &Doc, p: usize, big: bool) -> usize {
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

pub(super) fn word_back(doc: &Doc, p: usize, big: bool) -> usize {
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

pub(super) fn word_end_back(doc: &Doc, p: usize, big: bool) -> usize {
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

pub(super) fn blank(doc: &Doc, line: usize) -> bool {
    doc.line_chars(line).iter().all(|c| c.is_whitespace())
}

/// Motions that fail when they can't move, which stops a macro.
pub(super) fn must_move(m: &str) -> bool {
    m.trim_start_matches(|c: char| c.is_ascii_digit())
        .starts_with(|c: char| {
            "hjklwWbBeE{}nN*#fFtT;,+-\r \x7f".contains(c) || (LEFT..=DOWN).contains(&c)
        })
}

/// `f`, `F`, `t` and `T`: the `n`th `target` on the line.
pub(super) fn find_in_line(
    doc: &Doc,
    p: usize,
    kind: char,
    target: char,
    n: usize,
) -> Option<usize> {
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

impl Editor {
    /// Where a motion goes from the cursor, and whether an operator over it
    /// takes the character it lands on (`e`, `$`, `f`) or whole lines
    /// (`j`, `G`). `None` when it can't go, as `f` with no such letter.
    pub(super) fn vim_motion(
        &mut self,
        m: &str,
        count: Option<usize>,
        op: Option<Op>,
    ) -> Option<(usize, u8)> {
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
                self.vim_mut().last_find = Some((first, target));
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
                let vim = self.vim_mut();
                vim.search = Some((pattern, forward));
                vim.highlight = true;
                (to, EXCL)
            }
            ('n' | 'N', None) => {
                let (pattern, forward) = last_search?;
                let forward = forward == (first == 'n');
                let to = search_regex(doc, p, &vim_regex(&pattern).ok()?, forward, n)?;
                self.vim_mut().highlight = true;
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
        let vim = self.vim_mut();
        vim.goal = if keep_goal { goal_out } else { None };
        Some(result)
    }

    /// What an operator with a motion covers.
    pub(super) fn vim_motion_span(
        &mut self,
        m: &str,
        count: Option<usize>,
        op: Op,
    ) -> Option<Span> {
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
    pub(super) fn vim_object(&self, o: &str, count: Option<usize>) -> Option<Span> {
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
}
