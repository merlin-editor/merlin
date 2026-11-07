//! Vim's operators on a span, its registers, and putting, joining and
//! adding to numbers.

use super::*;

impl Editor {
    /// Puts text in a register, and in the clipboard where it should go.
    pub(super) fn vim_yank(
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
    pub(super) fn vim_operate(
        &mut self,
        op: Op,
        span: Span,
        reg: Option<char>,
        fx: &mut Vec<Effect>,
    ) -> bool {
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
                let changed = change_case(op, &text);
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

    pub(super) fn vim_op_motion(
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

    pub(super) fn vim_op_lines(
        &mut self,
        op: Op,
        n: usize,
        reg: Option<char>,
        fx: &mut Vec<Effect>,
    ) {
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
    pub(super) fn vim_join(&mut self, times: usize, spaces: bool) {
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
    pub(super) fn vim_add(&mut self, by: i64) {
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
    pub(super) fn vim_put(
        &mut self,
        reg: Option<char>,
        before: bool,
        n: usize,
        fx: &mut Vec<Effect>,
    ) {
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

    pub(super) fn vim_put_text(&mut self, text: &str, linewise: bool, before: bool, n: usize) {
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
}
