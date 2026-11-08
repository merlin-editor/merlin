//! The language item at the end of the status bar, as VS Code and Zed
//! have it, and the card it opens: what the file's language plugin
//! reports, such as its language server and how it's doing, then the
//! file's problems, format on save and tests.

use super::format::FormatOnSave;
use super::testing::Outcome;
use super::{Editor, Effect};
use crate::lang::{Severity, StatusLine, StatusState};

/// What the language card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageCardView {
    /// The language's name.
    pub title: String,
    /// The plugin it's from, such as `from the Rust plugin`; empty without
    /// one.
    pub from: String,
    /// Whether the plugin has settings to open.
    pub settings: bool,
    pub rows: Vec<LanguageRowView>,
}

/// A line in the language card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageRowView {
    pub name: String,
    /// Its dot's colour; none for grey.
    pub state: Option<StatusState>,
    pub detail: String,
    pub progress: Option<u8>,
    /// A command line under it, with Copy.
    pub hint: Option<String>,
    /// A button's label, and whether it's the one to press.
    pub button: Option<(String, bool)>,
    /// A link's label instead.
    pub link: Option<String>,
}

/// A mark drawn before a status bar item's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusMark {
    /// The language item's { }, coloured by how its language is doing, or
    /// turning while something is busy.
    Braces(StatusState),
    /// Something busy, turning.
    Spinner,
}

/// What a click in the language card is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageCardHit {
    /// A row's button or link.
    Row(usize),
    /// A row's Copy.
    Copy(usize),
    /// Plugin settings.
    Settings,
}

/// What a row's link does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    Problems,
    FormatOnSave,
    Tests,
}

impl Editor {
    /// A plugin's lines for the card, from one set.
    pub(crate) fn status_came(&mut self, plugin: String, set: String, lines: Vec<StatusLine>) {
        let key = (plugin, set);
        self.status_lines.retain(|(k, _)| *k != key);
        if !lines.is_empty() {
            self.status_lines.push((key, lines));
        }
    }

    /// The plugin that adds the shown file's language, if one does.
    fn language_owner(&self) -> Option<&str> {
        let ext = self.extension();
        self.languages
            .iter()
            .find(|(_, l)| l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .map(|(plugin, _)| plugin.as_str())
    }

    /// The lines for the shown file, with the plugin each is from.
    fn file_status(&self) -> Vec<(&str, &StatusLine)> {
        let ext = self.extension();
        if ext.is_empty() {
            return Vec::new();
        }
        let owner = self.language_owner();
        self.status_lines
            .iter()
            .flat_map(|((plugin, _), lines)| lines.iter().map(move |l| (plugin.as_str(), l)))
            .filter(|(plugin, l)| match l.extensions.is_empty() {
                true => owner == Some(*plugin),
                false => l.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)),
            })
            .collect()
    }

    /// How the shown file's language is doing: its worst line's state, or
    /// none with nothing reported.
    pub(crate) fn language_state(&self) -> Option<StatusState> {
        self.file_status().iter().map(|(_, l)| l.state).max()
    }

    /// What a busy line says, for beside the language item: its name and
    /// detail, with its progress.
    pub(crate) fn language_busy(&self) -> Option<String> {
        let (_, line) = self
            .file_status()
            .into_iter()
            .find(|(_, l)| l.state == StatusState::Busy)?;
        let what = line.detail.split(" · ").next().unwrap_or(&line.detail);
        Some(match line.progress {
            Some(p) => format!("{}: {what} {p}%", line.name),
            None => format!("{}: {what}", line.name),
        })
    }

    /// `language.status`: the card opens, or closes.
    pub(crate) fn toggle_language_card(&mut self) {
        self.language_card = !self.language_card;
    }

    /// The card's rows: the plugins', then the file's own.
    fn language_rows(&self) -> Vec<(LanguageRowView, Option<RowDoes>)> {
        let mut rows = Vec::new();
        for (plugin, line) in self.file_status() {
            let pressing = matches!(line.state, StatusState::Warning | StatusState::Error);
            rows.push((
                LanguageRowView {
                    name: line.name.clone(),
                    state: Some(line.state),
                    detail: line.detail.clone(),
                    progress: line.progress,
                    hint: line.hint.clone(),
                    button: line
                        .action
                        .as_ref()
                        .map(|(label, _)| (label.clone(), pressing)),
                    link: None,
                },
                line.action
                    .as_ref()
                    .map(|(_, command)| RowDoes::Plugin(plugin.to_string(), command.clone())),
            ));
        }
        if self.language_owner().is_none() && rows.is_empty() {
            return rows;
        }
        let doc = self.doc();
        let count = |s: Severity| doc.marks.iter().filter(|m| m.severity == s).count();
        let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
        let plural = |n: usize, what: &str| match n {
            1 => format!("1 {what}"),
            n => format!("{n} {what}s"),
        };
        let (state, detail) = match (errors, warnings) {
            (0, 0) => (StatusState::Ready, "None in this file".to_string()),
            (0, w) => (
                StatusState::Warning,
                format!("{} in this file", plural(w, "warning")),
            ),
            (e, 0) => (
                StatusState::Error,
                format!("{} in this file", plural(e, "error")),
            ),
            (e, w) => (
                StatusState::Error,
                format!(
                    "{} and {} in this file",
                    plural(e, "error"),
                    plural(w, "warning")
                ),
            ),
        };
        rows.push(self.link_row("Problems", Some(state), detail, "Problems", Link::Problems));
        if self.has_lsp("format") {
            let how = match self.format_on_save {
                FormatOnSave::Off => "Off",
                FormatOnSave::File => "The whole file",
                FormatOnSave::ChangedLines => "Changed lines",
            };
            rows.push(self.link_row(
                "Format on save",
                None,
                how.into(),
                "Change",
                Link::FormatOnSave,
            ));
        }
        let tests = self.doc_tests();
        if !tests.tests.is_empty() {
            let outcomes: Vec<Option<Outcome>> = tests
                .tests
                .iter()
                .map(|(_, _, key)| self.tests.results.get(key).map(|r| r.outcome))
                .collect();
            let failed = outcomes
                .iter()
                .filter(|o| **o == Some(Outcome::Failed))
                .count();
            let ran = outcomes.iter().filter(|o| o.is_some()).count();
            let n = tests.tests.len();
            let (state, how) = match (failed, ran) {
                (0, 0) => (None, "not run".to_string()),
                (0, r) if r == n => (Some(StatusState::Ready), "all passed".to_string()),
                (0, r) => (Some(StatusState::Ready), format!("{r} passed")),
                (f, _) => (Some(StatusState::Error), format!("{f} failed")),
            };
            let detail = format!("{n} in this file, {how}");
            rows.push(self.link_row("Tests", state, detail, "Run", Link::Tests));
        }
        rows
    }

    fn link_row(
        &self,
        name: &str,
        state: Option<StatusState>,
        detail: String,
        link: &str,
        does: Link,
    ) -> (LanguageRowView, Option<RowDoes>) {
        (
            LanguageRowView {
                name: name.into(),
                state,
                detail,
                progress: None,
                hint: None,
                button: None,
                link: Some(link.into()),
            },
            Some(RowDoes::Link(does)),
        )
    }

    /// The card, while it's open.
    pub(crate) fn language_card_view(&self) -> Option<LanguageCardView> {
        if !self.language_card {
            return None;
        }
        let owner = self
            .language_owner()
            .or_else(|| self.file_status().first().map(|(p, _)| *p));
        let title = |p: &str| {
            self.plugin_list
                .iter()
                .find(|s| s.name == p)
                .map_or(p.to_string(), |s| s.title.clone())
        };
        Some(LanguageCardView {
            title: self.language(),
            from: owner.map_or(String::new(), |p| {
                format!("from {}", plugin_name(&title(p)))
            }),
            settings: owner.is_some(),
            rows: self
                .language_rows()
                .into_iter()
                .map(|(row, _)| row)
                .collect(),
        })
    }

    /// A click in the card.
    pub(crate) fn language_card_click(&mut self, hit: LanguageCardHit, fx: &mut Vec<Effect>) {
        let rows = self.language_rows();
        match hit {
            LanguageCardHit::Row(i) => match rows.into_iter().nth(i).and_then(|(_, does)| does) {
                Some(RowDoes::Plugin(plugin, command)) => {
                    fx.push(Effect::PluginCommand { plugin, command });
                }
                Some(RowDoes::Link(link)) => {
                    self.language_card = false;
                    match link {
                        Link::Problems => self.run("view.problems", fx),
                        Link::FormatOnSave => self.open_settings(None, fx),
                        Link::Tests => self.run("tests.run_file", fx),
                    }
                }
                None => {}
            },
            LanguageCardHit::Copy(i) => {
                if let Some(hint) = rows.into_iter().nth(i).and_then(|(r, _)| r.hint) {
                    fx.push(Effect::Copy(hint));
                    self.notify("Copied".into());
                }
            }
            LanguageCardHit::Settings => {
                self.language_card = false;
                if let Some(owner) = self.language_owner().map(String::from) {
                    self.open_settings(Some(owner), fx);
                }
            }
        }
    }
}

/// What a row's button or link does.
enum RowDoes {
    /// Runs a command in a plugin.
    Plugin(String, String),
    Link(Link),
}

/// A plugin's title as the card names it: `the Rust plugin`, or a title
/// that says what it is already, such as `Language Servers`.
fn plugin_name(title: &str) -> String {
    match title.split_whitespace().count() {
        1 => format!("the {title} plugin"),
        _ => title.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::plugin_name;

    #[test]
    fn a_one_word_title_is_called_a_plugin() {
        assert_eq!(plugin_name("Rust"), "the Rust plugin");
        assert_eq!(plugin_name("Language Servers"), "Language Servers");
    }
}
