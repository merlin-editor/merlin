//! What floats over the text in a window: the right-click menu, and hover
//! and completion as cards by the text they're about. Hover also comes
//! from resting the mouse on a word, as other editors do it.

use super::{Editor, Effect, Focus, Popup};
use crate::key::{Key, KeyCode};

/// The right-click menu's commands, by id; `-` is a separator. Plugin
/// commands that aren't loaded are left out.
const ITEMS: &[(&str, &str)] = &[
    ("lsp.definition", "Go to Definition"),
    ("lsp.hover", "Show Hover"),
    ("-", ""),
    ("edit.select_all_matches", "Change All Occurrences"),
    ("edit.expand_selection", "Expand Selection"),
    ("-", ""),
    ("edit.cut", "Cut"),
    ("edit.copy", "Copy"),
    ("edit.paste", "Paste"),
    ("-", ""),
    ("edit.comment", "Toggle Line Comment"),
    ("-", ""),
    ("palette.open", "Command Palette…"),
];

pub(crate) struct ContextMenu {
    /// Command ids, `-` for separators.
    pub(crate) items: Vec<&'static str>,
    pub(crate) selected: Option<usize>,
}

/// The right-click menu, as the window draws it where it was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextView {
    pub items: Vec<ContextItem>,
    pub selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextItem {
    /// Empty for a separator.
    pub label: String,
    pub key: Option<String>,
}

/// A card by the text: what the cursor's on, or what can complete it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupView {
    /// The cell, in the text's grid, it's anchored to: it goes below it,
    /// or above when there's no room.
    pub anchor: (u16, u16),
    pub kind: PopupKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupKind {
    /// Markdown, a line each.
    Hover(Vec<String>),
    /// Labels and details, and the one Enter takes.
    Completion {
        items: Vec<(String, String)>,
        selected: usize,
    },
}

impl Editor {
    /// Opens the right-click menu for cell (`x`, `y`). In the text, the
    /// cursor goes there first, unless it's in the selection.
    pub(crate) fn open_context(&mut self, x: u16, y: u16) {
        let text = self.layout().text;
        if !text.contains(x, y) || self.doc().page.is_some() {
            return;
        }
        self.popup = None;
        self.probe = None;
        let pos = self.pos_at_screen(x, y);
        let inside = self
            .carets()
            .iter()
            .filter_map(|c| c.selection())
            .any(|(a, b)| (a..=b).contains(&pos));
        if !inside {
            self.set_cursor(pos);
        }
        let loaded = |id: &str| {
            let Some((plugin, command)) = id.split_once('.') else {
                return true;
            };
            if !["lsp"].contains(&plugin) {
                return true;
            }
            self.plugins
                .iter()
                .any(|p| p.name == plugin && p.commands.iter().any(|c| c.id == command))
        };
        let mut items: Vec<&'static str> = ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| loaded(id))
            .collect();
        // No separator first, last or twice.
        items.dedup_by(|a, b| *a == "-" && *b == "-");
        while items.first() == Some(&"-") {
            items.remove(0);
        }
        while items.last() == Some(&"-") {
            items.pop();
        }
        self.context = Some(ContextMenu {
            items,
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    pub(crate) fn close_context(&mut self) {
        self.context = None;
        if self.focus == Focus::Menu && self.menu.is_none() {
            self.focus = Focus::Editor;
        }
    }

    pub(crate) fn context_view(&self) -> Option<ContextView> {
        let menu = self.context.as_ref()?;
        let items = menu
            .items
            .iter()
            .map(|&id| match id {
                "-" => ContextItem {
                    label: String::new(),
                    key: None,
                },
                _ => ContextItem {
                    label: ITEMS
                        .iter()
                        .chain(super::tabs::TAB_ITEMS)
                        .find(|(i, _)| *i == id)
                        .map_or(id, |(_, l)| l)
                        .to_string(),
                    key: self.key_hint(id),
                },
            })
            .collect();
        Some(ContextView {
            items,
            selected: menu.selected,
        })
    }

    /// Runs the menu's `index`th item.
    pub(crate) fn context_item(&mut self, index: usize, fx: &mut Vec<Effect>) {
        let Some(id) = self
            .context
            .as_ref()
            .and_then(|m| m.items.get(index))
            .copied()
        else {
            return;
        };
        if id == "-" {
            return;
        }
        self.close_context();
        match id.split_once('.') {
            Some((plugin, command)) if self.plugins.iter().any(|p| p.name == plugin) => {
                fx.push(Effect::PluginCommand {
                    plugin: plugin.into(),
                    command: command.into(),
                })
            }
            _ => self.run(id, None, fx),
        }
    }

    /// Keys while the menu is open: arrows, Enter, Esc.
    pub(crate) fn context_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        let Some(menu) = &mut self.context else {
            return;
        };
        let n = menu.items.len();
        let step = |from: Option<usize>, down: bool, items: &[&str]| {
            let mut i = from.unwrap_or(if down { n - 1 } else { 0 });
            for _ in 0..n {
                i = if down { (i + 1) % n } else { (i + n - 1) % n };
                if items[i] != "-" {
                    break;
                }
            }
            Some(i)
        };
        match key.code {
            KeyCode::Esc => self.close_context(),
            KeyCode::Down => menu.selected = step(menu.selected, true, &menu.items),
            KeyCode::Up => menu.selected = step(menu.selected, false, &menu.items),
            KeyCode::Enter => {
                if let Some(i) = menu.selected {
                    self.context_item(i, fx);
                }
            }
            _ => {}
        }
    }

    /// The mouse rested on cell (`x`, `y`): asks the language server what's
    /// there, without moving the cursor.
    pub(crate) fn hover_at(&mut self, x: u16, y: u16, fx: &mut Vec<Effect>) {
        let text = self.layout().text;
        let hoverable = self.focus == Focus::Editor
            && self.popup.is_none()
            && self.context.is_none()
            && text.contains(x, y)
            && (x as usize) >= text.x as usize + self.gutter();
        let has_hover = self
            .plugins
            .iter()
            .any(|p| p.name == "lsp" && p.commands.iter().any(|c| c.id == "hover"));
        if !hoverable || !has_hover {
            return;
        }
        let pos = self.pos_at_screen(x, y);
        let doc = self.doc();
        let (start, end) = (doc.word_start_at(pos), doc.word_end_at(pos));
        // Only words; past a line's end there's nothing to ask about.
        if start == end || !(start..end).contains(&pos) {
            return;
        }
        self.probe = Some((start, end));
        fx.push(Effect::PluginCommand {
            plugin: "lsp".into(),
            command: "hover".into(),
        });
    }

    /// The mouse moved: a hover it brought up goes once it leaves the word.
    pub(crate) fn pointer_moved(&mut self, x: u16, y: u16) {
        let Some((start, end)) = self.probe else {
            return;
        };
        let text = self.layout().text;
        let pos = text.contains(x, y).then(|| self.pos_at_screen(x, y));
        if pos.is_none_or(|p| !(start..end).contains(&p)) {
            self.probe = None;
            if matches!(self.popup, Some(Popup::Hover(_))) && self.hover_anchor.is_some() {
                self.popup = None;
                self.hover_anchor = None;
            }
        }
    }

    /// Where the cursor counts as being for plugins: the hovered word
    /// while a hover is asked for, else the cursor.
    pub(crate) fn plugin_cursor(&self) -> usize {
        self.probe.map_or(self.doc().cursor, |(start, _)| start)
    }

    pub(crate) fn popup_view(&self) -> Option<PopupView> {
        let popup = self.popup.as_ref()?;
        let at = self.hover_anchor.unwrap_or(self.doc().cursor);
        let text = self.layout().text;
        let (x, y) = self.cell_of(at)?;
        let anchor = (x.saturating_sub(text.x), y.saturating_sub(text.y));
        let kind = match popup {
            Popup::Hover(lines) => PopupKind::Hover(lines.clone()),
            Popup::Completion { selected, .. } => PopupKind::Completion {
                items: self
                    .completions()
                    .iter()
                    .map(|c| (c.label.clone(), c.detail.clone()))
                    .collect(),
                selected: *selected,
            },
        };
        Some(PopupView { anchor, kind })
    }
}
