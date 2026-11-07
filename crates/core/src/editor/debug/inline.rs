//! A paused run in the text: the paused line, values beside the code and
//! under the pointer, and the card for an exception.

use super::*;

impl Editor {
    pub(crate) fn debug_paused(&self) -> bool {
        self.debug.paused().is_some()
    }

    /// While paused, whether the word from `start` to `end` is a variable
    /// of the frame shown; it becomes the hover's, and what's in it is
    /// asked for.
    pub(crate) fn hover_variable(
        &mut self,
        start: usize,
        end: usize,
        fx: &mut Vec<Effect>,
    ) -> bool {
        self.debug.hover = None;
        if self.debug.paused().is_none() {
            return false;
        }
        let name = self.doc().slice(start, end);
        let Some(v) = self.frame_variable(&name).cloned() else {
            return false;
        };
        let loaded = self
            .debug
            .session
            .as_ref()
            .is_some_and(|s| s.variables.contains_key(&v.children));
        if v.children != 0 && !loaded {
            self.send_debug(&format!("debug.expand:{}", v.children), fx);
        }
        self.debug.hover = Some(name);
        true
    }

    pub(crate) fn clear_hover_variable(&mut self) {
        self.debug.hover = None;
    }

    /// A variable of the frame shown by name, from its scopes as loaded.
    pub(super) fn frame_variable(&self, name: &str) -> Option<&DebugVariable> {
        let session = self.debug.session.as_ref()?;
        let scopes = session.variables.get(&0)?;
        scopes
            .iter()
            .filter_map(|s| session.variables.get(&s.children))
            .flatten()
            .find(|v| v.name == name)
    }

    /// The hover's value, as a code block: the variable with its type and
    /// value, then what's in it, a line each.
    pub(crate) fn hover_value_lines(&self) -> Vec<String> {
        let Some(name) = &self.debug.hover else {
            return Vec::new();
        };
        let Some(v) = self.frame_variable(name) else {
            return Vec::new();
        };
        let session = self.debug.session.as_ref().expect("a variable is in a run");
        let typed = match v.kind.is_empty() {
            true => v.name.clone(),
            false => format!("{}: {}", v.name, short_type(&v.kind)),
        };
        let mut lines = vec!["```".to_string()];
        let children = session
            .variables
            .get(&v.children)
            .filter(|_| v.children != 0);
        match (children, v.value.is_empty()) {
            (Some(_), _) | (None, true) => lines.push(typed),
            (None, false) => lines.push(format!("{typed} = {}", short_value(&v.value))),
        }
        if let Some(children) = children {
            for c in children.iter().take(12) {
                // Items of an array or a vector, as Rust indexes them.
                let name = match c.name.chars().all(|d| d.is_ascii_digit()) {
                    true => format!("[{}]", c.name),
                    false => c.name.clone(),
                };
                lines.push(format!("    {name} = {}", short_value(&c.value)));
            }
            if children.len() > 12 {
                lines.push(format!("    … {} more", children.len() - 12));
            }
        } else if v.children != 0 {
            lines.push("    …".into());
        }
        lines.push("```".into());
        lines
    }

    /// Whether the shown file's line is where the run is paused, and if
    /// so whether on an exception.
    pub(crate) fn paused_line(&self, line: usize) -> Option<bool> {
        let frame = self.debug.current_frame()?;
        let path = self.doc().buffer.path.as_ref()?;
        let same = frame.path.as_ref().is_some_and(|p| self.same_file(p, path));
        (same && frame.line as usize == line).then(|| self.paused_on_exception())
    }

    /// What to write after a line while paused: the plugin's values, or
    /// the frame's variables that the line names.
    pub(crate) fn inline_value(&self, line: usize, text: &str) -> Option<String> {
        let session = self.debug.session.as_ref()?;
        self.debug.paused()?;
        let path = self.doc().buffer.path.as_ref()?;
        if let Some((_, values)) = session.inline.iter().find(|(p, _)| self.same_file(p, path)) {
            return values
                .iter()
                .find(|(l, _)| *l as usize == line)
                .map(|(_, t)| t.clone());
        }
        let frame = self.debug.current_frame()?;
        if !frame.path.as_ref().is_some_and(|p| self.same_file(p, path))
            || line > frame.line as usize
            || line < self.function_start(frame.line as usize)
        {
            return None;
        }
        let scopes = session.variables.get(&0)?;
        let locals: Vec<&DebugVariable> = scopes
            .iter()
            .filter_map(|s| session.variables.get(&s.children))
            .flatten()
            .collect();
        let words: HashSet<&str> = text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
            .collect();
        let shown: Vec<String> = locals
            .iter()
            .filter(|v| !v.value.is_empty() && words.contains(v.name.as_str()))
            .take(4)
            .map(|v| format!("{} = {}", v.name, short_value(&v.value)))
            .collect();
        (!shown.is_empty()).then(|| shown.join(", "))
    }

    /// The first line of the function a line is in, as far as 40 lines
    /// up: where its `fn`, `def` or `function` is. Values show only from
    /// there, as the frame's variables are only that function's.
    pub(super) fn function_start(&self, line: usize) -> usize {
        let doc = self.doc();
        let top = line.saturating_sub(40);
        (top..=line)
            .rev()
            .find(|&l| {
                let text: String = doc.line_chars(l).into_iter().collect();
                let mut words = text.split_whitespace().skip_while(|w| {
                    w.starts_with("pub")
                        || matches!(*w, "async" | "const" | "unsafe" | "static" | "export")
                });
                matches!(words.next(), Some("fn" | "def" | "function" | "func"))
            })
            .unwrap_or(top)
    }

    /// While paused on an exception in the shown file: what it says, and
    /// the start of its line, for the card under it.
    pub(crate) fn exception_card(&self) -> Option<(String, usize)> {
        let session = self.debug.session.as_ref()?;
        let DebugState::Paused {
            reason,
            description,
            ..
        } = &session.state
        else {
            return None;
        };
        if reason != "exception" {
            return None;
        }
        let frame = self.debug.current_frame()?;
        let path = self.doc().buffer.path.as_ref()?;
        if !frame.path.as_ref().is_some_and(|p| self.same_file(p, path)) {
            return None;
        }
        let line = (frame.line as usize).min(self.doc().last_line());
        let message = match description.is_empty() {
            true => "An exception".to_string(),
            false => description.clone(),
        };
        Some((message, self.doc().line_start(line)))
    }

    pub(super) fn paused_on_exception(&self) -> bool {
        matches!(
            self.debug.session.as_ref().map(|s| &s.state),
            Some(DebugState::Paused { reason, .. }) if reason == "exception"
        )
    }
}
