//! Code actions: what a language server offers to do at the cursor, such
//! as fixing a problem. Ctrl+. or the lightbulb in the gutter lists them,
//! grouped as Quick Fix, Refactor and Source, and the hover card over a
//! problem offers its likeliest fix. Choosing one asks the plugin, which
//! answers with the edits to make.

use std::collections::VecDeque;

use super::editing::map_pos;
use super::{Editor, Effect, Focus, Popup};
use crate::edit::Edit;
use crate::history::Kind;
use crate::key::{Key, KeyCode};
use crate::lang::{ActionKind, CodeAction, FileEdit, Severity};

/// What a `code-actions` request was for, so its answer goes there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionsFor {
    /// The list Ctrl+. or the lightbulb opens.
    Menu,
    /// The fix the hover card over a problem offers.
    Hover,
}

#[derive(Default)]
pub(crate) struct Actions {
    /// The requests sent, oldest first: plugins answer in turn.
    pub(crate) asked: VecDeque<ActionsFor>,
    /// Edits for files being opened, to make once they are.
    pub(crate) pending: Vec<FileEdit>,
}

impl Editor {
    /// Ctrl+.: lists the code actions at the cursor.
    pub(crate) fn code_actions(&mut self, fx: &mut Vec<Effect>) {
        if !self.ask_actions(ActionsFor::Menu, fx) {
            self.notify("No language server offers code actions here".into());
        }
    }

    /// Asks the language server for the actions at the cursor, or at the
    /// hovered problem. Returns whether there's one to ask.
    pub(crate) fn ask_actions(&mut self, purpose: ActionsFor, fx: &mut Vec<Effect>) -> bool {
        if !self.has_lsp("code-actions") {
            return false;
        }
        self.actions.asked.push_back(purpose);
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: "code-actions".into(),
        });
        true
    }

    /// The plugin's answer: a list to choose from, or the hover card's fix.
    pub(crate) fn show_code_actions(&mut self, mut actions: Vec<CodeAction>) {
        let purpose = self.actions.asked.pop_front().unwrap_or(ActionsFor::Menu);
        // Grouped, the server's order kept within each group.
        actions.sort_by_key(|a| a.kind);
        match purpose {
            ActionsFor::Hover => {
                if let Some(Popup::Hover { fixes, .. }) = &mut self.popup {
                    *fixes = actions;
                }
            }
            ActionsFor::Menu => self.open_actions(actions),
        }
    }

    /// Lists `actions` by the cursor, the likeliest fix chosen.
    fn open_actions(&mut self, actions: Vec<CodeAction>) {
        if actions.is_empty() {
            return self.notify("No code actions here".into());
        }
        if self.focus != Focus::Editor {
            return;
        }
        let selected = actions.iter().position(|a| a.preferred).unwrap_or(0);
        self.hover_anchor = None;
        self.probe = None;
        self.popup = Some(Popup::Actions { actions, selected });
    }

    /// Asks the plugin to do the action with id `id`.
    pub(crate) fn choose_action(&mut self, id: String, fx: &mut Vec<Effect>) {
        self.popup = None;
        self.hover_anchor = None;
        fx.push(Effect::PluginPick {
            plugin: "lsp".into(),
            picker: "code-actions".into(),
            item: id,
        });
    }

    /// The `i`th row of the list, clicked.
    pub(crate) fn action_clicked(&mut self, i: usize, fx: &mut Vec<Effect>) {
        if let Some(Popup::Actions { actions, .. }) = &self.popup
            && let Some(action) = actions.get(i)
        {
            let id = action.id.clone();
            self.choose_action(id, fx);
        }
    }

    /// A key while the list shows: the arrows move, Enter chooses, and
    /// anything else puts it away. Returns whether it used the key.
    pub(crate) fn actions_key(&mut self, key: Key, fx: &mut Vec<Effect>) -> bool {
        let Some(Popup::Actions { actions, selected }) = &mut self.popup else {
            return false;
        };
        let plain = !key.ctrl && !key.alt;
        match key.code {
            KeyCode::Up if plain => {
                *selected = selected.checked_sub(1).unwrap_or(actions.len() - 1);
                true
            }
            KeyCode::Down if plain => {
                *selected = (*selected + 1) % actions.len();
                true
            }
            KeyCode::Enter | KeyCode::Tab if plain => {
                let id = actions[*selected].id.clone();
                self.choose_action(id, fx);
                true
            }
            KeyCode::Esc => {
                self.popup = None;
                true
            }
            _ => {
                self.popup = None;
                false
            }
        }
    }

    /// The hover card's fix: the server's preferred quick fix, else its
    /// first.
    pub(crate) fn hover_fix(fixes: &[CodeAction]) -> Option<&CodeAction> {
        let quick = || fixes.iter().filter(|a| a.kind == ActionKind::QuickFix);
        quick().find(|a| a.preferred).or_else(|| quick().next())
    }

    /// The hover card's More fixes: the whole list, at the problem.
    pub(crate) fn more_fixes(&mut self) {
        let Some(Popup::Hover { fixes, .. }) = &self.popup else {
            return;
        };
        let fixes = fixes.clone();
        if let Some(at) = self.hover_anchor {
            self.set_cursor(at);
        }
        self.open_actions(fixes);
    }

    /// The line the lightbulb shows on: the cursor's, when it has a problem
    /// a language server could fix.
    pub(crate) fn bulb_line(&self) -> Option<usize> {
        if !self.has_lsp("code-actions") || self.focus != Focus::Editor {
            return None;
        }
        let doc = self.doc();
        let line = doc.line_col(doc.cursor).0;
        doc.marks
            .iter()
            .any(|m| m.severity <= Severity::Info && doc.line_col(m.start).0 == line)
            .then_some(line)
    }

    /// Makes a plugin's edits: in open files at once, each file one undo
    /// step; other files are opened first, and left unsaved.
    pub(crate) fn apply_file_edits(&mut self, edits: Vec<FileEdit>, fx: &mut Vec<Effect>) {
        for file in edits {
            if self.doc_at(&file.path).is_some() {
                self.edit_file(&file);
            } else {
                fx.push(Effect::Open(file.path.clone()));
                self.actions.pending.push(file);
            }
        }
    }

    /// Makes the edits waiting for `path` now it's open.
    pub(crate) fn opened_for_edits(&mut self, path: &std::path::Path) {
        let (now, later): (Vec<FileEdit>, Vec<FileEdit>) =
            std::mem::take(&mut self.actions.pending)
                .into_iter()
                .partition(|f| f.path == path);
        self.actions.pending = later;
        for file in now {
            self.edit_file(&file);
        }
    }

    fn edit_file(&mut self, file: &FileEdit) {
        let Some(doc) = self.doc_at(&file.path) else {
            return;
        };
        let mut edits: Vec<Edit> = file
            .edits
            .iter()
            .map(|(start, end, text)| {
                let at = doc.pos_at_utf16(start.0, start.1);
                let end = doc.pos_at_utf16(end.0, end.1).max(at);
                Edit {
                    at,
                    removed: doc.slice(at, end),
                    inserted: text.clone(),
                }
            })
            .collect();
        edits.sort_by_key(|e| e.at);
        doc.buffer.begin(doc.cursor, Kind::Other);
        // Right to left, so each edit's offset is still right.
        for edit in edits.iter().rev() {
            doc.buffer.apply(edit.clone());
        }
        doc.cursor = map_pos(doc.cursor, &edits);
        doc.anchor = doc.anchor.map(|a| map_pos(a, &edits));
        doc.buffer.commit(doc.cursor);
    }
}
