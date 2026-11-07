//! Vim's Visual modes: by character, by line and by block, and the
//! commands on what's selected.

use super::*;

impl Editor {
    pub(super) fn vim_visual_start(&mut self, mode: VimMode) {
        self.clear_carets();
        let cursor = self.doc().cursor;
        let vim = self.vim_mut();
        vim.mode = Some(mode);
        vim.anchor = cursor;
        vim.cursor = cursor;
        vim.goal = None;
        self.vim_show_visual();
    }

    /// What Visual mode covers, from where it began to its cursor and
    /// taking both.
    pub(super) fn vim_visual_span(&self) -> Span {
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
    pub(super) fn vim_block(&self) -> (usize, usize, usize, usize) {
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
    pub(super) fn vim_show_visual(&mut self) {
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

    pub(super) fn vim_leave_visual(&mut self) {
        let vim = self.vim_mut();
        vim.last_visual = Some((vim.mode(), vim.anchor, vim.cursor));
        vim.mode = Some(VimMode::Normal);
        let cursor = vim.cursor;
        self.clear_carets();
        let doc = self.doc_mut();
        doc.anchor = None;
        doc.cursor = cursor.min(doc.buffer.len());
    }

    pub(super) fn vim_visual(&mut self, cmd: Cmd, fx: &mut Vec<Effect>) {
        let n = cmd.count.unwrap_or(1);
        let mode = self.vim_mode().unwrap_or(VimMode::Visual);
        let reg = cmd.reg;
        match cmd.what {
            What::Move(m) => {
                if let Some((to, _)) = self.vim_motion(&m, cmd.count, None) {
                    self.vim_mut().cursor = to;
                    self.vim_show_visual();
                }
            }
            What::Object(o) => {
                if let Some(span) = self.vim_object(&o, cmd.count) {
                    let vim = self.vim_mut();
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
                            self.vim_mut().mode = Some(want);
                            self.vim_show_visual();
                        }
                    }
                    ('o' | 'O', None) => {
                        let vim = self.vim_mut();
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
                            self.vim_mut().put = Some((first == 'P', n));
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
                        let vim = self.vim_mut();
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
    pub(super) fn vim_block_op(
        &mut self,
        op: Op,
        to_end: bool,
        reg: Option<char>,
        fx: &mut Vec<Effect>,
    ) {
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
                    let changed = change_case(op, &text);
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
