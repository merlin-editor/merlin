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
    /// The same for a plugin's view, where it lives.
    Plugin(String),
}

/// A status bar item: its text, style, and what clicking does.
pub(crate) struct StatusItem {
    pub text: String,
    pub style: Style,
    pub action: Option<Action>,
    /// The language's { }, or a spinner, before the text.
    pub mark: Option<super::StatusMark>,
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
                let side = self
                    .plugins
                    .iter()
                    .find(|p| p.name == name)
                    .map_or(Side::Right, |p| p.side);
                let shown = self.bar(side).visible
                    && self.active_panel_name(side).as_deref() == Some(name.as_str());
                if shown {
                    self.bar_mut(side).visible = false;
                    if self.focus == side.focus() {
                        self.focus = Focus::Editor;
                    }
                } else {
                    self.show_plugin_view(&name);
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
            .filter(|d| {
                d.review.as_ref().is_some_and(|r| {
                    !r.plugin.is_empty()
                        && r.plugin != super::blame::COMMIT
                        && r.plugin != super::compare_files::COMPARE
                })
            })
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
        if let Some(status) = self.debug_status() {
            left.push((
                format!(" {status} "),
                Style::Status,
                Some(Action::View(super::debug::DEBUG)),
            ));
        }
        if let Some(branch) = &self.git.branch {
            left.push((
                format!(" {} {branch} ", icon(Icon::Branch, self.nerd)),
                Style::Status,
                Some(Action::View(CHANGES)),
            ));
            // Commits to pull and push, which a click syncs; while it runs,
            // what's happening.
            let sync = &self.git.sync;
            if let Some(kind) = self.git.syncing.filter(|k| *k != super::SyncKind::Fetch) {
                let doing = match kind {
                    super::SyncKind::Push => "Pushing…",
                    super::SyncKind::Pull => "Pulling…",
                    super::SyncKind::Publish => "Publishing…",
                    _ => "Syncing…",
                };
                left.push((format!(" ⟳ {doing} "), Style::Status, None));
            } else if sync.ahead > 0 || sync.behind > 0 {
                left.push((
                    format!(" ↓{} ↑{} ", sync.behind, sync.ahead),
                    Style::Status,
                    Some(Action::Run("scm.sync_all")),
                ));
            }
        }
        // Plugins' work under way, a click away from Cancel.
        if let Some(text) = self.progress_status() {
            left.push((
                format!(" ⟳ {text} "),
                Style::Status,
                Some(Action::Run("progress.show")),
            ));
        }
        let (_, failed) = self.test_counts();
        if failed > 0 {
            let s = if failed == 1 { "" } else { "s" };
            left.push((
                format!(" ✕ {failed} test{s} failed "),
                Style::StatusError,
                Some(Action::Run("view.tests")),
            ));
        }
        // Tasks still running, a click away from their terminal.
        for run in self.running_tasks() {
            left.push((
                format!(" ● {} ", run.name),
                Style::Status,
                Some(Action::Run("task.show")),
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
        let conflicts = self.git.files.iter().filter(|(_, s)| *s == '!').count();
        if let Some(merge) = &self.git.merge {
            let style = if merge.conflicts.is_empty() {
                Style::StatusAccent
            } else {
                Style::StatusError
            };
            left.push((
                format!(" {}: {} ", merge.doing(), merge.summary().to_lowercase()),
                style,
                Some(Action::View(CHANGES)),
            ));
        } else if conflicts > 0 {
            let s = if conflicts == 1 { "" } else { "s" };
            left.push((
                format!(" {conflicts} conflict{s} "),
                Style::StatusError,
                Some(Action::View(CHANGES)),
            ));
        }
        if self.blame_shown() {
            left.push((
                " Blame ".into(),
                Style::StatusAccent,
                Some(Action::Run("git.blame")),
            ));
        }
        if self.waiting_format.is_some() {
            left.push((" Formatting… ".into(), Style::Status, None));
        }

        // In Vim's Visual mode, its cursor rather than the selection's end.
        let at = self
            .vim_visual_cursor()
            .map_or(doc.cursor, |p| p.min(doc.buffer.len()));
        let (line, col) = doc.line_col(at);
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
            (
                format!(
                    "Keymap: {}",
                    if self.vim.is_some() { "Vim" } else { "VS Code" }
                ),
                Some(Action::Run("vim.toggle")),
            ),
        ];
        // What a language server is busy with, then the language, which
        // opens its card.
        let card = Some(Action::Run("language.status"));
        let busy = self.language_busy();
        if let Some(busy) = &busy {
            right.push((busy.clone(), card.clone()));
        }
        right.push((self.language(), card));
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
                mark: None,
                right: false,
            });
        }
        let last = right.len() - 1;
        let state = self.language_state();
        for (i, (text, action)) in right.into_iter().enumerate() {
            let mark = match i {
                _ if i == last => state.map(super::StatusMark::Braces),
                _ if busy.as_deref() == Some(text.as_str()) => Some(super::StatusMark::Spinner),
                _ => None,
            };
            items.push(StatusItem {
                text: format!(" {text}  "),
                style: Style::Status,
                action,
                mark,
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

    /// The language of the active file, as the status bar names it: as a
    /// plugin adds it, or Markdown, the one the editor knows itself.
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
        match path.and_then(|p| p.extension()).and_then(|e| e.to_str()) {
            Some("md" | "markdown") => "Markdown",
            _ => "Plain Text",
        }
        .into()
    }

    /// Errors and warnings from every plugin, in every file.
    pub(crate) fn problem_counts(&self) -> (usize, usize) {
        let all = self.diagnostics.values().flatten();
        let count = |s: Severity| all.clone().filter(|d| d.severity == s).count();
        (count(Severity::Error), count(Severity::Warning))
    }
}
