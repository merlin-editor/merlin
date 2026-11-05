//! The window's frame around the text, as VS Code has it: the activity bar,
//! the title bar's command center and layout buttons, status bar items and
//! notices. Each part says where its clickable pieces are, so drawing and
//! the mouse agree.

use super::views::CHANGES;
use super::{Editor, Effect, Focus, Side};
use crate::icons::{Icon, icon};
use crate::lang::Severity;
use crate::style::{Style, str_width};

/// What clicking part of the frame does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Run(&'static str),
    /// Shows a built-in view in the side bar, or hides the side bar if it's
    /// showing it already.
    View(&'static str),
    /// The same for a plugin's panel in the secondary side bar.
    Plugin(String),
}

/// A status bar item: its text, style, and what clicking does.
pub(crate) struct StatusItem {
    pub text: String,
    pub style: Style,
    pub action: Option<Action>,
    /// Whether it's one of the cursor and file facts at the right end.
    pub right: bool,
}

impl Editor {
    pub(crate) fn act(&mut self, action: Action, fx: &mut Vec<Effect>) {
        match action {
            Action::Run(command) => self.run(command, fx),
            Action::View(view) => {
                let shown = self.bar(Side::Left).visible
                    && self.active_panel_name(Side::Left).as_deref() == Some(view);
                if shown {
                    self.toggle_side_bar(fx);
                } else {
                    self.show_view(view, fx);
                }
            }
            Action::Plugin(name) => {
                let shown = self.bar(Side::Right).visible
                    && self.active_panel_name(Side::Right).as_deref() == Some(name.as_str());
                if shown {
                    self.bar_mut(Side::Right).visible = false;
                    if self.focus == Focus::Panel {
                        self.focus = Focus::Editor;
                    }
                } else {
                    self.bar_mut(Side::Right).active = Some(name);
                    self.focus_panel(Side::Right);
                }
            }
        }
    }

    /// The status bar's items, laid out: what needs doing and the branch
    /// and problems on the left, the cursor and file on the right.
    pub(crate) fn status_items(&self) -> Vec<StatusItem> {
        // The window lays out its own status bar, with room for everything.
        let width = u16::MAX / 2;
        let doc = self.doc();
        let mut left: Vec<(String, Style, Option<Action>)> = Vec::new();
        let reviews = self
            .docs
            .iter()
            .filter(|d| d.review.as_ref().is_some_and(|r| !r.plugin.is_empty()))
            .filter(|d| !super::rename::is_rename_review(d))
            .count();
        if reviews > 0 {
            let what = if reviews == 1 { "change" } else { "changes" };
            left.push((
                format!(
                    " {} {reviews} {what} to review ",
                    icon(Icon::Diff, self.nerd)
                ),
                Style::StatusAccent,
                Some(Action::Run("review.next")),
            ));
        }
        if let Some(branch) = &self.git.branch {
            left.push((
                format!(" {} {branch} ", icon(Icon::Branch, self.nerd)),
                Style::Status,
                Some(Action::View(CHANGES)),
            ));
        }
        let (errors, warnings) = self.problem_counts();
        left.push((
            format!(
                " {} {errors}  {} {warnings} ",
                icon(Icon::Error, self.nerd),
                icon(Icon::Warning, self.nerd)
            ),
            Style::Status,
            Some(Action::Run("view.problems")),
        ));
        if self.reviewing_rename() {
            left.push((
                format!(" {} ", self.review_hint()),
                Style::StatusAccent,
                None,
            ));
        } else if let Some((staged, changes)) = self.staged_count() {
            let s = if changes == 1 { "" } else { "s" };
            left.push((
                format!(" Staged {staged} of {changes} change{s} "),
                Style::StatusAccent,
                None,
            ));
            left.push((format!("  {}", self.review_hint()), Style::Status, None));
        } else if doc.review.is_some() {
            left.push((format!("  {}", self.review_hint()), Style::Status, None));
        }
        if self.waiting_format.is_some() {
            left.push((" Formatting… ".into(), Style::Status, None));
        }

        let (line, col) = doc.line_col(doc.cursor);
        let mut position = format!("Ln {}, Col {}", line + 1, col + 1);
        if !doc.extra.is_empty() {
            position.push_str(&format!(" ({} selections)", doc.extra.len() + 1));
        } else if let Some((a, b)) = doc.selection() {
            position.push_str(&format!(" ({} selected)", b - a));
        }
        let rope = doc.buffer.rope();
        let crlf = rope.len_lines() > 1 && rope.line(0).to_string().ends_with("\r\n");
        let indent = self.indent().label();
        let mut right: Vec<(String, Option<Action>)> = vec![
            (position, Some(Action::Run("go.line"))),
            (indent, None),
            ("UTF-8".into(), None),
            ((if crlf { "CRLF" } else { "LF" }).into(), None),
            (self.language(), None),
        ];
        let left_w: usize = left.iter().map(|(t, ..)| str_width(t)).sum();
        // A diagnostic under the cursor needs room more than the encoding.
        let wanted = |r: &[(String, Option<Action>)]| {
            r.iter().map(|(t, _)| str_width(t) + 3).sum::<usize>() + left_w + 1
        };
        let room =
            (width as usize).saturating_sub(if self.cursor_mark().is_some() { 40 } else { 0 });
        while right.len() > 2 && wanted(&right) > room {
            right.remove(1);
        }

        let mut items = Vec::new();
        for (text, style, action) in left {
            items.push(StatusItem {
                text,
                style,
                action,
                right: false,
            });
        }
        for (text, action) in right {
            items.push(StatusItem {
                text: format!(" {text}  "),
                style: Style::Status,
                action,
                right: true,
            });
        }
        items
    }

    /// The diagnostic under the cursor, or else on its line.
    pub(crate) fn cursor_mark(&self) -> Option<&crate::lang::Mark> {
        let doc = self.doc();
        let line = doc.line_col(doc.cursor).0;
        doc.marks
            .iter()
            .filter(|m| m.start <= doc.cursor && doc.cursor <= m.end.max(m.start + 1))
            .chain(doc.marks.iter().filter(|m| doc.line_col(m.start).0 == line))
            .next()
    }

    /// The language of the active file, as the status bar names it.
    pub(crate) fn language(&self) -> String {
        if let Some(lang) = self.plugin_language() {
            return lang.name.clone();
        }
        let doc = self.doc();
        let path = doc
            .review
            .as_ref()
            .map(|r| &r.path)
            .or(doc.buffer.path.as_ref());
        let name = match path.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
            Some("md") => "Markdown",
            Some("rs") => "Rust",
            Some("py" | "pyi") => "Python",
            Some("js" | "mjs" | "cjs" | "jsx") => "JavaScript",
            Some("json") => "JSON",
            Some("toml") => "TOML",
            Some("ts" | "tsx") => "TypeScript",
            Some("go") => "Go",
            Some("c" | "h") => "C",
            Some("cc" | "cpp" | "hpp") => "C++",
            Some("sh") => "Shell",
            Some("lua") => "Lua",
            Some("yaml" | "yml") => "YAML",
            Some("html") => "HTML",
            Some("css") => "CSS",
            _ => "Plain Text",
        };
        name.into()
    }

    /// Errors and warnings from every plugin, in every file.
    pub(crate) fn problem_counts(&self) -> (usize, usize) {
        let all = self.diagnostics.values().flatten();
        let count = |s: Severity| all.clone().filter(|d| d.severity == s).count();
        (count(Severity::Error), count(Severity::Warning))
    }
}
