//! Breakpoints: set in the gutter, their conditions, hit counts and log
//! messages in a box under the line, and kept across runs.

use super::*;

impl Editor {
    pub(crate) fn toggle_breakpoint(&mut self, path: &Path, line: u32, fx: &mut Vec<Effect>) {
        match self.debug.breakpoint_at(path, line) {
            Some(i) => {
                let b = self.debug.breakpoints.remove(i);
                self.debug.states.remove(&b.id);
            }
            None => {
                self.debug.next_id += 1;
                let id = self.debug.next_id;
                self.debug.breakpoints.push(Breakpoint {
                    id,
                    path: path.to_path_buf(),
                    line,
                    enabled: true,
                    ..Default::default()
                });
            }
        }
        self.breakpoints_changed(fx);
    }

    pub(super) fn menu_breakpoint_index(&self) -> Option<usize> {
        let path = self.doc().buffer.path.as_ref()?;
        self.debug.breakpoint_at(path, self.debug.menu_line?)
    }

    pub(super) fn menu_breakpoint(&self) -> Option<&Breakpoint> {
        self.menu_breakpoint_index()
            .map(|i| &self.debug.breakpoints[i])
    }

    /// The right-click menu on the breakpoint room of line `line`: add
    /// one, or change the one there.
    pub(crate) fn break_menu(&mut self, line: usize) {
        self.debug.menu_line = Some(line as u32);
        let items: Vec<&'static str> = match self.menu_breakpoint() {
            Some(b) => vec![
                "debug.bp_edit",
                if b.enabled {
                    "debug.bp_disable"
                } else {
                    "debug.bp_enable"
                },
                "debug.bp_remove",
            ],
            None => vec!["debug.bp_add", "debug.bp_condition", "debug.bp_log"],
        };
        self.context = Some(super::context::ContextMenu {
            items,
            selected: None,
            title: None,
        });
        self.focus = Focus::Menu;
    }

    /// Opens the box between the lines on the menu's line, with what the
    /// breakpoint there already has.
    pub(super) fn open_break_box(&mut self, field: BreakField) {
        let Some(line) = self.debug.menu_line else {
            return;
        };
        let text = self
            .menu_breakpoint()
            .map_or(String::new(), |b| match field {
                BreakField::Condition => b.condition.clone(),
                BreakField::Hit => b.hit.clone(),
                BreakField::Log => b.log.clone(),
            });
        self.debug.break_box = Some(BreakBox {
            doc: self.doc().id,
            line,
            field,
            caret: text.chars().count(),
            text,
        });
        self.focus = Focus::Editor;
    }

    /// The box, to draw, and the position it's under.
    pub(crate) fn break_box_view(&self) -> Option<(BreakBoxView, usize)> {
        let b = self
            .debug
            .break_box
            .as_ref()
            .filter(|b| b.doc == self.doc().id)?;
        let line = (b.line as usize).min(self.doc().last_line());
        Some((
            BreakBoxView {
                field: b.field.label().into(),
                text: b.text.clone(),
                caret: b.caret,
                hint: format!(
                    "{} · Tab for another kind · Enter sets it · Esc leaves it",
                    b.field.hint()
                ),
            },
            self.doc().line_start(line),
        ))
    }

    /// Keys while the breakpoint box is open: they edit its text. Tab
    /// goes to the next kind, Enter sets it, Esc leaves it as it was.
    pub(crate) fn break_box_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let id = self.doc().id;
        let Some(b) = self.debug.break_box.as_mut().filter(|b| b.doc == id) else {
            return false;
        };
        let byte = |text: &str, i: usize| text.char_indices().nth(i).map_or(text.len(), |c| c.0);
        let len = b.text.chars().count();
        match key.code {
            KeyCode::Esc => self.debug.break_box = None,
            KeyCode::Tab | KeyCode::BackTab => {
                b.field = match (b.field, key.code == KeyCode::BackTab) {
                    (BreakField::Condition, false) | (BreakField::Log, true) => BreakField::Hit,
                    (BreakField::Hit, false) | (BreakField::Condition, true) => BreakField::Log,
                    (BreakField::Log, false) | (BreakField::Hit, true) => BreakField::Condition,
                };
                b.text.clear();
                b.caret = 0;
            }
            KeyCode::Enter => {
                let b = self.debug.break_box.take().expect("open");
                self.set_break_field(b.line, b.field, b.text.trim(), fx);
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                let at = byte(&b.text, b.caret);
                b.text.insert(at, c);
                b.caret += 1;
            }
            KeyCode::Backspace if b.caret > 0 => {
                let at = byte(&b.text, b.caret - 1);
                b.text.remove(at);
                b.caret -= 1;
            }
            KeyCode::Delete if b.caret < len => {
                let at = byte(&b.text, b.caret);
                b.text.remove(at);
            }
            KeyCode::Left => b.caret = b.caret.saturating_sub(1),
            KeyCode::Right => b.caret = (b.caret + 1).min(len),
            KeyCode::Home => b.caret = 0,
            KeyCode::End => b.caret = len,
            _ => {}
        }
        true
    }

    /// Sets a breakpoint's condition, hit count or message, making the
    /// breakpoint if the line has none.
    pub(super) fn set_break_field(
        &mut self,
        line: u32,
        field: BreakField,
        text: &str,
        fx: &mut Vec<Effect>,
    ) {
        let Some(path) = self.doc().buffer.path.clone() else {
            return;
        };
        let i = match self.debug.breakpoint_at(&path, line) {
            Some(i) => i,
            None => {
                self.debug.next_id += 1;
                self.debug.breakpoints.push(Breakpoint {
                    id: self.debug.next_id,
                    path,
                    line,
                    enabled: true,
                    ..Default::default()
                });
                self.debug.breakpoints.len() - 1
            }
        };
        let b = &mut self.debug.breakpoints[i];
        match field {
            BreakField::Condition => b.condition = text.into(),
            BreakField::Hit => b.hit = text.into(),
            BreakField::Log => b.log = text.into(),
        }
        self.breakpoints_changed(fx);
    }

    /// Breakpoints and the chosen target, kept with the workspace.
    pub(crate) fn restore_breakpoints(
        &mut self,
        mut list: Vec<Breakpoint>,
        target: Option<String>,
    ) {
        for b in &mut list {
            self.debug.next_id += 1;
            b.id = self.debug.next_id;
        }
        self.debug.breakpoints = list;
        if target.is_some() {
            self.debug.chosen = target;
        }
        self.build_views();
    }

    pub(super) fn breakpoints_changed(&mut self, fx: &mut Vec<Effect>) {
        if self.debug.running() {
            self.send_debug("debug.breakpoints", fx);
        }
        self.build_views();
    }

    /// Whether the shown file can have breakpoints: a debugger is declared
    /// for its language, or it has some already.
    pub(crate) fn can_break_here(&self) -> bool {
        let Some(path) = self.doc().buffer.path.as_ref() else {
            return false;
        };
        let language = self.plugin_language().map(|l| l.id.clone());
        self.debug.breakpoints.iter().any(|b| b.path == *path)
            || language.is_some_and(|l| {
                self.debug
                    .debuggers
                    .iter()
                    .any(|d| d.languages.contains(&l))
            })
    }

    /// The gutter's mark for a line of the shown file.
    pub(crate) fn break_mark(&self, line: usize) -> Option<BreakMark> {
        let path = self.doc().buffer.path.as_ref()?;
        let b = self
            .debug
            .breakpoints
            .iter()
            .find(|b| b.path == *path && b.line as usize == line)?;
        Some(if !b.enabled {
            BreakMark::Disabled
        } else if self.debug.states.get(&b.id).is_some_and(|s| !s.set) {
            BreakMark::Unset
        } else if !b.log.is_empty() {
            BreakMark::Log
        } else if !b.condition.is_empty() || !b.hit.is_empty() {
            BreakMark::Conditional
        } else {
            BreakMark::Set
        })
    }

    /// Breakpoints, for the workspace to keep.
    pub fn breakpoints(&self) -> Vec<(PathBuf, u32, String, String, String, bool)> {
        self.debug
            .breakpoints
            .iter()
            .map(|b| {
                (
                    b.path.clone(),
                    b.line,
                    b.condition.clone(),
                    b.hit.clone(),
                    b.log.clone(),
                    b.enabled,
                )
            })
            .collect()
    }
}
