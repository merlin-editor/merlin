//! A plugin's bar over its languages' files, for something about the
//! project to fix: clangd guessing the flags without compile commands,
//! with Configure with CMake or Set Include Folders.

use super::{Editor, Effect};
use crate::panel::Button;

pub(crate) struct FileBar {
    pub plugin: String,
    pub id: String,
    pub text: String,
    pub buttons: Vec<Button>,
}

/// A bar as a group shows it: what it says, and its buttons' labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileBarView {
    /// Its place among the bars, which its presses carry.
    pub index: usize,
    pub text: String,
    /// The buttons there are commands for, by their place.
    pub buttons: Vec<(usize, String)>,
}

impl Editor {
    /// Whether the editor has a command `id`: its own, or a plugin's as
    /// `<plugin>.<id>`.
    fn command_known(&self, id: &str) -> bool {
        if id.starts_with("settings.") {
            return true;
        }
        match id.split_once('.') {
            Some((plugin, command)) => self
                .plugins
                .iter()
                .any(|p| p.name == plugin && p.commands.iter().any(|c| c.id == command)),
            None => false,
        }
    }

    /// The bar over the file shown: the first whose plugin adds its
    /// language.
    pub(crate) fn file_bar(&self) -> Option<FileBarView> {
        if self.away_bar().is_some() {
            return None;
        }
        let doc = &self.docs[self.pane_doc()];
        let path = doc.buffer.path.as_ref()?;
        let (index, bar) = self.file_bars.iter().enumerate().find(|(_, b)| {
            self.languages
                .iter()
                .any(|(plugin, lang)| *plugin == b.plugin && lang.matches(path))
        })?;
        let buttons = bar
            .buttons
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.id.strip_prefix("run:")
                    .is_none_or(|command| self.command_known(command))
            })
            .map(|(i, b)| (i, b.label.clone()))
            .collect();
        Some(FileBarView {
            index,
            text: bar.text.clone(),
            buttons,
        })
    }

    /// A press on a bar: `bar:<bar>:<button>`, or `bar:<bar>:x` for ✕.
    pub(crate) fn bar_press(&mut self, id: &str, fx: &mut Vec<Effect>) {
        let Some((index, what)) = id
            .strip_prefix("bar:")
            .and_then(|r| r.split_once(':'))
            .and_then(|(i, w)| Some((i.parse::<usize>().ok()?, w)))
        else {
            return;
        };
        let Some(bar) = self.file_bars.get(index) else {
            return;
        };
        let plugin = bar.plugin.clone();
        if what == "x" {
            let bar = self.file_bars.remove(index);
            return self.run(&format!("plugin.click:{plugin}:dismiss:{}", bar.id), fx);
        }
        let Some(button) = what.parse::<usize>().ok().and_then(|i| bar.buttons.get(i)) else {
            return;
        };
        match button.id.strip_prefix("run:") {
            Some(command) => {
                let command = command.to_string();
                self.run(&command, fx);
            }
            None => {
                let id = button.id.clone();
                self.run(&format!("plugin.click:{plugin}:{id}"), fx);
            }
        }
    }
}
