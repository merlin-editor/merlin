//! Editing at the cursors: typing, deleting, indenting, comments, the
//! clipboard and moving between lines.

use super::*;

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
        if key.alt {
            return;
        }
        if self.has_carets() {
            // Esc goes back to one cursor; commands such as undo run once;
            // everything else runs at every cursor.
            if key.code == Esc && !key.ctrl && !key.shift {
                return self.clear_carets();
            }
            if !(key.ctrl && bound(EDITOR_KEYS, key).is_some()) {
                let kind = match key.code {
                    Char(_) if !key.ctrl => Kind::Typing,
                    _ => Kind::Other,
                };
                return self.each_caret(kind, |e, _| e.caret_key(key, fx));
            }
        }
        self.caret_key(key, fx);
    }

    /// A key at one cursor.
    pub(crate) fn caret_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
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
        if key.ctrl {
            if let Some(id) = bound(EDITOR_KEYS, key) {
                self.run(id, None, fx);
            }
            return;
        }
        let doc = self.doc();
        let cursor = doc.cursor;
        match key.code {
            Esc => self.doc_mut().anchor = None,
            Char(c) => self.type_text(&c.to_string(), Kind::Typing),
            Enter => {
                let at = doc.selection().map_or(cursor, |(start, _)| start);
                let (line, col) = doc.line_col(at);
                let indent: String = doc
                    .line_chars(line)
                    .into_iter()
                    .take(col)
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                self.type_text(&format!("\n{indent}"), Kind::Other);
            }
            Tab => match doc.selection() {
                Some((start, end)) if doc.line_col(start).0 != doc.line_col(end).0 => {
                    self.indent_lines(true)
                }
                _ => self.type_text(&" ".repeat(TAB_WIDTH), Kind::Other),
            },
            BackTab => self.indent_lines(false),
            Backspace => match doc.selection() {
                Some((start, end)) => self.replace(start, end, "", Kind::Other),
                None if cursor > 0 => self.replace(cursor - 1, cursor, "", Kind::Other),
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
        let mut edits = Vec::new();
        for line in first..=last {
            let at = doc.line_start(line);
            let chars = doc.line_chars(line);
            if indent {
                if !chars.is_empty() {
                    edits.push(Edit {
                        at,
                        removed: String::new(),
                        inserted: " ".repeat(TAB_WIDTH),
                    });
                }
            } else {
                let n = if chars.first() == Some(&'\t') {
                    1
                } else {
                    chars
                        .iter()
                        .take(TAB_WIDTH)
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
                    self.inputs.entry(input).or_default().push_str(first_line);
                }
            }
            Focus::Menu => {}
        }
    }
}
