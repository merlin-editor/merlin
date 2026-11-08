//! Editing at the cursors: typing, deleting, indenting, comments, the
//! clipboard and moving between lines.

use super::*;

/// Brackets and quotes that close themselves, as VS Code's defaults have it.
const PAIRS: &[(char, char)] = &[
    ('(', ')'),
    ('[', ']'),
    ('{', '}'),
    ('"', '"'),
    ('\'', '\''),
    ('`', '`'),
];

/// What may follow the cursor for an opener to close itself: VS Code's
/// `autoCloseBefore`, plus the end of the line.
fn closes_before(next: Option<char>) -> bool {
    next.is_none_or(|c| c.is_whitespace() || ";:.,=}])>\"'`".contains(c))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Where `pos` ends up after `edits`, given left to right in the original
/// text's offsets.
pub(super) fn map_pos(pos: usize, edits: &[Edit]) -> usize {
    let mut shift = 0isize;
    for edit in edits {
        let removed = edit.removed.chars().count();
        let inserted = edit.inserted.chars().count() as isize;
        if pos >= edit.at + removed {
            shift += inserted - removed as isize;
        } else if pos > edit.at {
            shift += edit.at as isize + inserted - pos as isize;
        }
    }
    (pos as isize + shift) as usize
}

impl Editor {
    pub(crate) fn editor_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        use KeyCode::*;
        if self.popup_key(key) {
            return;
        }
        // A page is drawn by the window; there's no text to type in.
        if self.doc().page.is_some() {
            return;
        }
        if self.in_review() && self.review_key(key, fx) {
            return;
        }
        // Bound keys ran before this; Alt with anything else types
        // nothing.
        if key.alt {
            return;
        }
        if self.has_carets() {
            // Esc goes back to one cursor; commands such as undo run once;
            // everything else runs at every cursor.
            if key.code == Esc && !key.ctrl && !key.shift {
                return self.clear_carets();
            }
            let kind = match key.code {
                Char(_) if !key.ctrl => Kind::Typing,
                _ => Kind::Other,
            };
            return self.each_caret(kind, |e, _| e.caret_key(key));
        }
        self.caret_key(key);
    }

    /// A key at one cursor.
    pub(crate) fn caret_key(&mut self, key: Key) {
        use KeyCode::*;
        let page = self.text_height().max(1) as isize;
        if matches!(
            key.code,
            Left | Right | Up | Down | Home | End | PageUp | PageDown
        ) {
            let doc = self.doc_mut();
            let selection = doc.selection();
            if key.shift {
                let cursor = doc.cursor;
                doc.anchor.get_or_insert(cursor);
            } else {
                doc.anchor = None;
                // Left and Right first collapse a selection to that side.
                if let (Some((start, end)), false, Left | Right) = (selection, key.ctrl, key.code) {
                    doc.cursor = if key.code == Left { start } else { end };
                    doc.goal = None;
                    return;
                }
            }
            match (key.code, key.ctrl) {
                (Left, false) => doc.cursor = doc.cursor.saturating_sub(1),
                (Right, false) => doc.cursor = (doc.cursor + 1).min(doc.buffer.len()),
                (Left, true) => doc.cursor = doc.word_left(doc.cursor),
                (Right, true) => doc.cursor = doc.word_right(doc.cursor),
                (Up, _) => self.move_lines(-1),
                (Down, _) => self.move_lines(1),
                (PageUp, _) => self.move_lines(-page),
                (PageDown, _) => self.move_lines(page),
                (Home, true) => doc.cursor = 0,
                (End, true) => doc.cursor = doc.buffer.len(),
                (Home, false) => {
                    let (line, col) = doc.line_col(doc.cursor);
                    let indent = doc
                        .line_chars(line)
                        .iter()
                        .take_while(|c| c.is_whitespace())
                        .count();
                    // First non-blank, then column 0.
                    doc.cursor = doc.line_start(line) + if col == indent { 0 } else { indent };
                }
                (End, false) => {
                    let (line, _) = doc.line_col(doc.cursor);
                    doc.cursor = doc.line_start(line) + doc.buffer.line_len(line);
                }
                _ => {}
            }
            if !matches!(key.code, Up | Down | PageUp | PageDown) {
                self.doc_mut().goal = None;
            }
            return;
        }
        // Word deletes run at each cursor, so they aren't commands.
        if key.ctrl && !key.shift && matches!(key.code, Backspace | Delete) {
            let doc = self.doc();
            let cursor = doc.cursor;
            let (start, end) = match (doc.selection(), key.code) {
                (Some(sel), _) => sel,
                (None, Backspace) => (doc.word_left(cursor), cursor),
                (None, _) => (cursor, doc.word_right(cursor)),
            };
            if start < end {
                self.replace(start, end, "", Kind::Other);
            }
            self.doc_mut().goal = None;
            return;
        }
        if key.ctrl {
            return;
        }
        let doc = self.doc();
        let cursor = doc.cursor;
        match key.code {
            Esc => self.doc_mut().anchor = None,
            Char(c) => self.type_char(c),
            Enter => self.newline(),
            Tab => match doc.selection() {
                Some((start, end)) if doc.line_col(start).0 != doc.line_col(end).0 => {
                    self.indent_lines(true)
                }
                _ => {
                    // To the next tab stop.
                    let unit = self.indent();
                    let (line, col) = doc.line_col(doc.selection().map_or(cursor, |s| s.0));
                    let text = if unit.tabs {
                        "\t".to_string()
                    } else {
                        " ".repeat(unit.width - doc.display_col(line, col) % unit.width)
                    };
                    self.type_text(&text, Kind::Other)
                }
            },
            BackTab => self.indent_lines(false),
            Backspace => match doc.selection() {
                Some((start, end)) => self.replace(start, end, "", Kind::Other),
                None if cursor > 0 => self.backspace(),
                None => {}
            },
            Delete => match doc.selection() {
                Some((start, end)) => self.replace(start, end, "", Kind::Other),
                None if cursor < doc.buffer.len() => {
                    self.replace(cursor, cursor + 1, "", Kind::Other)
                }
                None => {}
            },
            _ => {}
        }
        self.doc_mut().goal = None;
    }

    pub(crate) fn replace(&mut self, start: usize, end: usize, text: &str, kind: Kind) {
        if self.in_review() {
            return self.notify("A review is read-only: y accepts, n rejects".into());
        }
        let doc = self.doc_mut();
        let removed = doc.slice(start, end);
        doc.buffer.begin(doc.cursor, kind);
        doc.buffer.apply(Edit {
            at: start,
            removed,
            inserted: text.to_string(),
        });
        doc.cursor = start + text.chars().count();
        doc.anchor = None;
        doc.buffer.commit(doc.cursor);
    }

    /// Types or inserts `text`, replacing the selection.
    pub(crate) fn type_text(&mut self, text: &str, kind: Kind) {
        let doc = self.doc();
        match doc.selection() {
            Some((start, end)) => self.replace(start, end, text, Kind::Other),
            None => self.replace(doc.cursor, doc.cursor, text, kind),
        }
    }

    /// Applies `edits`, given left to right, as one undo step, keeping the
    /// cursor and selection on the same text.
    pub(crate) fn apply_all(&mut self, edits: Vec<Edit>) {
        let doc = self.doc_mut();
        doc.buffer.begin(doc.cursor, Kind::Other);
        // Right to left, so each edit's offset is still right.
        for edit in edits.iter().rev() {
            doc.buffer.apply(edit.clone());
        }
        doc.cursor = map_pos(doc.cursor, &edits);
        doc.anchor = doc.anchor.map(|a| map_pos(a, &edits));
        doc.buffer.commit(doc.cursor);
    }

    /// Indents or dedents every line the selection touches.
    pub(crate) fn indent_lines(&mut self, indent: bool) {
        let doc = self.doc();
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let first = doc.line_col(start).0;
        let (mut last, end_col) = doc.line_col(end);
        // A selection ending at column 0 doesn't include that line.
        if last > first && end_col == 0 {
            last -= 1;
        }
        let unit = self.indent();
        let doc = self.doc();
        let mut edits = Vec::new();
        for line in first..=last {
            let at = doc.line_start(line);
            let chars = doc.line_chars(line);
            if indent {
                if !chars.is_empty() {
                    edits.push(Edit {
                        at,
                        removed: String::new(),
                        inserted: unit.unit(),
                    });
                }
            } else {
                let n = if chars.first() == Some(&'\t') {
                    1
                } else {
                    chars
                        .iter()
                        .take(unit.width)
                        .take_while(|c| **c == ' ')
                        .count()
                };
                if n > 0 {
                    edits.push(Edit {
                        at,
                        removed: chars[..n].iter().collect(),
                        inserted: String::new(),
                    });
                }
            }
        }
        if !edits.is_empty() {
            self.apply_all(edits);
        }
    }

    /// Comments out the lines the selection touches with the language's
    /// line comment, or uncomments them if they all are.
    pub(crate) fn toggle_comment(&mut self) {
        let doc = self.doc();
        let ext = doc
            .buffer
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let from_plugin = self.plugin_language().and_then(|l| l.line_comment.clone());
        let doc = self.doc();
        let prefix: &str = match ext {
            _ if from_plugin.is_some() => from_plugin.as_deref().unwrap_or_default(),
            "rs" | "js" | "jsx" | "ts" | "tsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "go"
            | "java" | "kt" | "swift" | "cs" | "zig" | "json" => "//",
            "py" | "toml" | "sh" | "yaml" | "yml" | "rb" | "pl" | "conf" => "#",
            "lua" | "sql" | "hs" => "--",
            _ => return self.notify("No line comments in this kind of file".into()),
        };
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let first = doc.line_col(start).0;
        let (mut last, end_col) = doc.line_col(end);
        if last > first && end_col == 0 {
            last -= 1;
        }
        // Each non-blank line: where it starts, its indent and the rest.
        let lines: Vec<(usize, usize, String)> = (first..=last)
            .filter_map(|line| {
                let chars = doc.line_chars(line);
                let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
                (indent < chars.len()).then(|| {
                    (
                        doc.line_start(line),
                        indent,
                        chars[indent..].iter().collect(),
                    )
                })
            })
            .collect();
        let commented =
            !lines.is_empty() && lines.iter().all(|(_, _, rest)| rest.starts_with(prefix));
        let column = lines
            .iter()
            .map(|(_, indent, _)| *indent)
            .min()
            .unwrap_or(0);
        let edits: Vec<Edit> = lines
            .iter()
            .map(|(at, indent, rest)| {
                if commented {
                    let space = rest[prefix.len()..].starts_with(' ') as usize;
                    Edit {
                        at: at + indent,
                        removed: rest[..prefix.len() + space].to_string(),
                        inserted: String::new(),
                    }
                } else {
                    Edit {
                        at: at + column,
                        removed: String::new(),
                        inserted: format!("{prefix} "),
                    }
                }
            })
            .collect();
        if !edits.is_empty() {
            self.apply_all(edits);
        }
    }

    /// Copies the selection, or the whole line without one, and cuts it if
    /// asked.
    pub(crate) fn copy(&mut self, cut: bool, fx: &mut Vec<Effect>) {
        if self.focus != Focus::Editor {
            return;
        }
        if self.has_carets() {
            // Each cursor's piece, top to bottom: selections a line each,
            // or whole lines.
            let (texts, lines) = self.caret_texts();
            let text = if lines {
                texts.concat()
            } else {
                texts.join("\n")
            };
            self.clipboard = text.clone();
            fx.push(Effect::Copy(text));
            if cut {
                self.each_caret(Kind::Other, |e, _| {
                    let doc = e.doc();
                    let (start, end) = match doc.selection() {
                        Some(sel) => sel,
                        None if lines => {
                            let line = doc.line_col(doc.cursor).0;
                            let end = if line < doc.last_line() {
                                doc.line_start(line + 1)
                            } else {
                                doc.buffer.len()
                            };
                            (doc.line_start(line), end)
                        }
                        None => return,
                    };
                    e.replace(start, end, "", Kind::Other);
                });
            }
            return;
        }
        let doc = self.doc();
        let (start, end, text) = match doc.selection() {
            Some((start, end)) => (start, end, doc.slice(start, end)),
            None => {
                let (line, _) = doc.line_col(doc.cursor);
                let start = doc.line_start(line);
                let end = if line < doc.last_line() {
                    doc.line_start(line + 1)
                } else {
                    doc.buffer.len()
                };
                let mut text = doc.slice(start, end);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                (start, end, text)
            }
        };
        self.clipboard = text.clone();
        fx.push(Effect::Copy(text));
        if cut && start < end {
            self.replace(start, end, "", Kind::Other);
        }
    }

    pub(crate) fn set_cursor(&mut self, cursor: usize) {
        let doc = self.doc_mut();
        doc.extra.clear();
        doc.cursor = cursor.min(doc.buffer.len());
        doc.goal = None;
        doc.anchor = None;
    }

    pub(crate) fn move_lines(&mut self, delta: isize) {
        if self.row_based() {
            return self.move_rows(delta);
        }
        let doc = self.doc_mut();
        let (line, col) = doc.line_col(doc.cursor);
        let goal = match doc.goal {
            Some(goal) => goal,
            None => {
                let goal = doc.display_col(line, col);
                doc.goal = Some(goal);
                goal
            }
        };
        let target = (line as isize + delta).clamp(0, doc.last_line() as isize) as usize;
        doc.cursor = doc.pos_at(target, goal);
    }

    pub(crate) fn paste(&mut self, text: &str) {
        let first_line = text.lines().next().unwrap_or("");
        match self.focus {
            Focus::Editor => {
                let text = text.replace("\r\n", "\n");
                // One line per cursor, when there are as many lines as
                // cursors, as VS Code pastes.
                let count = self.carets().len();
                let lines: Vec<&str> = text
                    .strip_suffix('\n')
                    .unwrap_or(&text)
                    .split('\n')
                    .collect();
                let spread = count > 1 && lines.len() == count;
                self.each_caret(Kind::Other, |e, k| {
                    let piece = if spread { lines[k] } else { text.as_str() };
                    e.type_text(piece, Kind::Other)
                });
            }
            Focus::Find => {
                if let Some(find) = &mut self.find {
                    find.field().push_str(first_line);
                    if !find.in_replace {
                        let origin = find.origin;
                        self.find_from(origin);
                    }
                }
            }
            Focus::Palette => {
                if let Some(palette) = &mut self.palette {
                    palette.query.push_str(first_line);
                    palette.selected = 0;
                }
            }
            Focus::Panel | Focus::SideBar | Focus::Bottom => {
                let side = self.focused_side().unwrap_or(Side::Right);
                if let Some(input) = self.focused_input(side) {
                    let is_multi = self.is_focused_input_multiline(side);
                    let to_insert = if is_multi { text } else { first_line };
                    let entry = self.inputs.entry(input.clone()).or_default();
                    let cur = self.input_cursors.entry(input).or_insert(entry.len());
                    *cur = (*cur).min(entry.len());
                    while !entry.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    entry.insert_str(*cur, to_insert);
                    *cur += to_insert.len();
                }
            }
            Focus::Menu => {}
        }
    }

    // Typing as editors do it --------------------------------------------------

    fn char_at(&self, pos: usize) -> Option<char> {
        self.doc().buffer.rope().get_char(pos)
    }

    /// Types `c`, closing brackets and quotes, typing over closers and
    /// putting a closing bracket back to its opener's indent.
    fn type_char(&mut self, c: char) {
        let doc = self.doc();
        let cursor = doc.cursor;
        let selection = doc.selection();
        let (prev, next) = (
            cursor.checked_sub(1).and_then(|p| self.char_at(p)),
            self.char_at(cursor),
        );
        let pair = PAIRS.iter().find(|(open, _)| *open == c).copied();
        if self.auto_close {
            // Wrap the selection in the pair, keeping it selected.
            if let (Some((open, close)), Some((start, end))) = (pair, selection) {
                let inner = self.doc().slice(start, end);
                self.replace(start, end, &format!("{open}{inner}{close}"), Kind::Other);
                let doc = self.doc_mut();
                doc.anchor = Some(start + 1);
                doc.cursor = end + 1;
                return;
            }
            // Over the closer that's already there.
            if selection.is_none() && next == Some(c) && PAIRS.iter().any(|(_, close)| *close == c)
            {
                self.doc_mut().cursor += 1;
                return;
            }
            if let (Some((open, close)), None) = (pair, selection) {
                let quote = open == close;
                // Quotes don't close after a word (it's an apostrophe), or
                // after `&` and `<`, where `'` starts a Rust lifetime.
                let after_word = prev.is_some_and(|p| is_word(p) || (quote && "&<".contains(p)));
                if closes_before(next) && !(quote && (after_word || prev == Some(c))) {
                    self.replace(cursor, cursor, &format!("{open}{close}"), Kind::Typing);
                    self.doc_mut().cursor -= 1;
                    return;
                }
            }
        }
        // A closing bracket alone on its line goes back a level.
        if selection.is_none() && "}])".contains(c) {
            let doc = self.doc();
            let (line, col) = doc.line_col(cursor);
            let before = &doc.line_chars(line)[..col];
            if col > 0 && before.iter().all(|ch| *ch == ' ' || *ch == '\t') {
                let unit = self.indent();
                let width = doc.display_col(line, col);
                let back = unit.to_column(width.saturating_sub(1) / unit.width * unit.width);
                let start = doc.line_start(line);
                return self.replace(start, cursor, &format!("{back}{c}"), Kind::Other);
            }
        }
        self.type_text(&c.to_string(), Kind::Typing);
    }

    /// Enter: the line's indent again, a level more after an opener, and
    /// between a pair of brackets a line of its own for what goes inside.
    fn newline(&mut self) {
        let doc = self.doc();
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let (line, col) = doc.line_col(start);
        let chars = doc.line_chars(line);
        let indent: String = chars[..col]
            .iter()
            .take_while(|c| **c == ' ' || **c == '\t')
            .collect();
        let last = chars[..col]
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .copied();
        let opener = PAIRS
            .iter()
            .find(|(open, close)| open != close && Some(*open) == last)
            .map(|(_, close)| *close);
        // And after what the language says opens a block, such as `:`.
        let before: String = chars[..col].iter().collect();
        let before = before.trim_end();
        let opens = opener.is_some()
            || self
                .plugin_language()
                .is_some_and(|l| l.indent_after.iter().any(|e| before.ends_with(e.as_str())));
        let unit = self.indent().unit();
        // A line of only indentation keeps none of it.
        let from = if indent.chars().count() == col && col > 0 {
            doc.line_start(line)
        } else {
            start
        };
        if opener.is_some() && opener == self.char_at(end) {
            let text = format!("\n{indent}{unit}\n{indent}");
            self.replace(from, end, &text, Kind::Other);
            let doc = self.doc_mut();
            doc.cursor = from + 1 + indent.chars().count() + unit.chars().count();
            return;
        }
        let more = if opens { unit.as_str() } else { "" };
        self.replace(from, end, &format!("\n{indent}{more}"), Kind::Other);
    }

    /// Backspace: an empty pair goes at once, and in indentation a whole
    /// level goes.
    fn backspace(&mut self) {
        let doc = self.doc();
        let cursor = doc.cursor;
        let (prev, next) = (self.char_at(cursor - 1), self.char_at(cursor));
        if self.auto_close
            && PAIRS
                .iter()
                .any(|&(open, close)| Some(open) == prev && Some(close) == next)
        {
            return self.replace(cursor - 1, cursor + 1, "", Kind::Other);
        }
        let (line, col) = doc.line_col(cursor);
        let unit = self.indent();
        let before = &doc.line_chars(line)[..col];
        // In the indent, back to the last tab stop. At the line's start
        // it's the line break that goes.
        if !unit.tabs && col > 0 && before.iter().all(|c| *c == ' ') {
            let n = col - (col - 1) / unit.width * unit.width;
            return self.replace(cursor - n, cursor, "", Kind::Other);
        }
        self.replace(cursor - 1, cursor, "", Kind::Other);
    }

    /// The open file's extension, or "".
    pub(crate) fn extension(&self) -> &str {
        self.doc()
            .buffer
            .path
            .as_ref()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or("")
    }

    // Lines ------------------------------------------------------------------

    /// The first and last lines the selection touches; a selection ending
    /// at column 0 doesn't touch that line.
    fn touched_lines(&self) -> (usize, usize) {
        let doc = self.doc();
        let (start, end) = doc.selection().unwrap_or((doc.cursor, doc.cursor));
        let first = doc.line_col(start).0;
        let (mut last, end_col) = doc.line_col(end);
        if last > first && end_col == 0 {
            last -= 1;
        }
        (first, last)
    }

    /// Where `line` ends, before its line break.
    pub(crate) fn line_end(&self, line: usize) -> usize {
        let doc = self.doc();
        doc.line_start(line) + doc.buffer.line_len(line)
    }

    /// Alt+Up and Down: the lines move past the one above or below, with
    /// the cursor and selection on them.
    pub(crate) fn move_lines_by(&mut self, down: bool) {
        self.clear_carets();
        let (first, last) = self.touched_lines();
        let doc = self.doc();
        if (!down && first == 0) || (down && last >= doc.last_line()) {
            return;
        }
        let other = if down { last + 1 } else { first - 1 };
        let block = doc.slice(doc.line_start(first), self.line_end(last));
        let neighbour = doc.slice(doc.line_start(other), self.line_end(other));
        let (from, to, text) = if down {
            (
                doc.line_start(first),
                self.line_end(other),
                format!("{neighbour}\n{block}"),
            )
        } else {
            (
                doc.line_start(other),
                self.line_end(last),
                format!("{block}\n{neighbour}"),
            )
        };
        let shift = neighbour.chars().count() as isize + 1;
        let shift = if down { shift } else { -shift };
        let (cursor, anchor) = (doc.cursor, doc.anchor);
        self.replace(from, to, &text, Kind::Other);
        let moved = |p: usize| (p as isize + shift) as usize;
        let doc = self.doc_mut();
        doc.cursor = moved(cursor);
        doc.anchor = anchor.map(moved);
    }

    /// Shift+Alt+Up and Down: a copy of the lines above or below, with the
    /// cursor on the copy.
    pub(crate) fn copy_lines(&mut self, down: bool) {
        self.clear_carets();
        let (first, last) = self.touched_lines();
        let doc = self.doc();
        let block = doc.slice(doc.line_start(first), self.line_end(last));
        let (cursor, anchor) = (doc.cursor, doc.anchor);
        let at = doc.line_start(first);
        self.replace(at, at, &format!("{block}\n"), Kind::Other);
        let shift = if down { block.chars().count() + 1 } else { 0 };
        let doc = self.doc_mut();
        doc.cursor = cursor + shift;
        doc.anchor = anchor.map(|a| a + shift);
    }

    /// Ctrl+Shift+K: the lines go, and the cursor stays in its column.
    pub(crate) fn delete_lines(&mut self) {
        let (first, last) = self.touched_lines();
        let doc = self.doc();
        let goal = doc.display_col(doc.line_col(doc.cursor).0, doc.line_col(doc.cursor).1);
        let (from, to) = if last < doc.last_line() {
            (doc.line_start(first), doc.line_start(last + 1))
        } else {
            // The last line takes the break before it instead.
            (doc.line_start(first).saturating_sub(1), doc.buffer.len())
        };
        self.replace(from, to, "", Kind::Other);
        let doc = self.doc_mut();
        let line = first.min(doc.last_line());
        doc.cursor = doc.pos_at(line, goal);
    }

    /// Ctrl+Enter and Ctrl+Shift+Enter: a new line below or above, at this
    /// one's indent, wherever the cursor is in it.
    pub(crate) fn insert_line(&mut self, below: bool) {
        let doc = self.doc();
        let line = doc.line_col(doc.cursor).0;
        let indent: String = doc
            .line_chars(line)
            .into_iter()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect();
        if below {
            let end = self.line_end(line);
            self.replace(end, end, &format!("\n{indent}"), Kind::Other);
        } else {
            let start = doc.line_start(line);
            self.replace(start, start, &format!("{indent}\n"), Kind::Other);
            self.doc_mut().cursor = start + indent.chars().count();
        }
    }

    /// Ctrl+L: selects the line, and the next each time again.
    pub(crate) fn select_line(&mut self) {
        let (first, last) = self.touched_lines();
        let doc = self.doc();
        let selected_whole = doc
            .selection()
            .is_some_and(|(s, e)| s == doc.line_start(first) && e == doc.line_start(last + 1));
        let last = if selected_whole { last + 1 } else { last };
        let end = if last < doc.last_line() {
            doc.line_start(last + 1)
        } else {
            doc.buffer.len()
        };
        let start = doc.line_start(first);
        let doc = self.doc_mut();
        doc.anchor = Some(start);
        doc.cursor = end;
        doc.goal = None;
    }

    /// Ctrl+Shift+\\: to the bracket that matches the one at or before the
    /// cursor, or out to the brackets around it.
    pub(crate) fn jump_to_bracket(&mut self) {
        let doc = self.doc();
        let rope = doc.buffer.rope();
        let cursor = doc.cursor;
        let brackets = [('(', ')'), ('[', ']'), ('{', '}')];
        let at = |p: usize| rope.get_char(p);
        // The bracket next to the cursor, after it first.
        let here = [Some(cursor), cursor.checked_sub(1)]
            .into_iter()
            .flatten()
            .find(|&p| at(p).is_some_and(|c| "()[]{}".contains(c)));
        let target = match here {
            Some(p) => {
                let c = at(p).unwrap_or(' ');
                match brackets.iter().find(|(o, cl)| *o == c || *cl == c) {
                    Some(&(open, close)) if c == open => scan(rope, p + 1, true, open, close),
                    Some(&(open, close)) => scan(rope, p, false, open, close),
                    None => None,
                }
            }
            // Inside: to the closer of the innermost pair around.
            None => brackets
                .iter()
                .filter_map(|&(open, close)| scan(rope, cursor, true, open, close))
                .min(),
        };
        if let Some(p) = target {
            self.set_cursor(p);
        }
    }
}

/// From `from`, the bracket that balances: forward to a `close`, or back
/// to an `open`. Gives up after a long way, to stay quick.
pub(super) fn scan(
    rope: &ropey::Rope,
    from: usize,
    forward: bool,
    open: char,
    close: char,
) -> Option<usize> {
    const LIMIT: usize = 200_000;
    let mut depth = 0usize;
    if forward {
        for (i, c) in rope.chars_at(from).enumerate().take(LIMIT) {
            if c == open {
                depth += 1;
            } else if c == close {
                if depth == 0 {
                    return Some(from + i);
                }
                depth -= 1;
            }
        }
    } else {
        let mut chars = rope.chars_at(from);
        let mut p = from;
        while let Some(c) = chars.prev() {
            p -= 1;
            if from - p > LIMIT {
                break;
            }
            if c == close {
                depth += 1;
            } else if c == open {
                if depth == 0 {
                    return Some(p);
                }
                depth -= 1;
            }
        }
    }
    None
}
