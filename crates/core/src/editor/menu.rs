//! The menu bar, as in VS Code. Its items are commands, so each shows the
//! key that runs it and the palette finds the same things.

use super::{Editor, Effect, Focus};
use crate::key::{Key, KeyCode};

/// Each menu's title and commands; `-` is a separator.
const MENUS: &[(&str, &[&str])] = &[
    (
        "File",
        &[
            "file.new",
            "file.open",
            "file.reopen",
            "-",
            "file.save",
            "file.save_as",
            "-",
            "file.close",
            "file.close_all",
            "-",
            "app.quit",
        ],
    ),
    (
        "Edit",
        &[
            "edit.undo",
            "edit.redo",
            "-",
            "edit.cut",
            "edit.copy",
            "edit.paste",
            "-",
            "find.open",
            "find.replace",
            "view.search",
            "-",
            "edit.comment",
        ],
    ),
    (
        "Selection",
        &[
            "edit.select_all",
            "-",
            "edit.cursor_above",
            "edit.cursor_below",
            "edit.add_next",
            "edit.select_all_matches",
            "-",
            "edit.indent",
            "edit.outdent",
        ],
    ),
    (
        "View",
        &[
            "palette.open",
            "-",
            "view.explorer",
            "view.search",
            "view.changes",
            "view.outline",
            "-",
            "view.problems",
            "view.output",
            "-",
            "view.sidebar",
            "view.panel",
            "view.secondary",
            "view.minimap",
            "view.wrap",
            "view.split",
            "view.split_down",
            "view.breadcrumbs",
            "view.sticky_scroll",
        ],
    ),
    (
        "Go",
        &[
            "file.open",
            "go.line",
            "go.symbol",
            "-",
            "nav.back",
            "-",
            "file.next",
            "file.prev",
        ],
    ),
    ("Help", &["palette.open", "-", "app.about"]),
];

/// The menus, as a window draws them: the titles, the one open, and its
/// items, separators with empty labels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuView {
    pub titles: Vec<String>,
    pub open: usize,
    pub items: Vec<super::ContextItem>,
    pub selected: Option<usize>,
}

/// A menu item: command id, label and key. The id `-` is a separator.
type Item = (&'static str, String, String);

/// An open menu.
pub(crate) struct Menu {
    pub index: usize,
    /// The highlighted item, if any.
    pub item: Option<usize>,
}

impl Editor {
    /// Opens a menu by title, or the first one.
    pub(crate) fn open_menu(&mut self, title: Option<&str>) {
        let index = title
            .and_then(|t| MENUS.iter().position(|(m, _)| m.eq_ignore_ascii_case(t)))
            .unwrap_or(0);
        self.menu = Some(Menu {
            index,
            item: Some(0),
        });
        self.popup = None;
        self.focus = Focus::Menu;
    }

    pub(crate) fn close_menu(&mut self) {
        self.menu = None;
        if self.focus == Focus::Menu {
            self.focus = Focus::Editor;
        }
    }

    /// A menu's items: command id, label and key.
    fn menu_items(&self, index: usize) -> Vec<Item> {
        let commands = self.commands();
        MENUS[index]
            .1
            .iter()
            .map(|&id| {
                if id == "-" {
                    return (id, String::new(), String::new());
                }
                let title = commands
                    .iter()
                    .find(|(c, _)| c == id)
                    .map_or(id.to_string(), |(_, t)| t.clone());
                // "View: Explorer" reads as "Explorer" under View.
                let label = title
                    .split_once(": ")
                    .map_or(title.clone(), |(_, l)| l.to_string());
                (id, label, self.key_hint(id).unwrap_or_default())
            })
            .collect()
    }

    fn step_item(&mut self, down: bool) {
        let Some(menu) = &self.menu else { return };
        let items = MENUS[menu.index].1;
        let n = items.len();
        let mut i = menu.item.unwrap_or(if down { n - 1 } else { 0 });
        for _ in 0..n {
            i = if down { (i + 1) % n } else { (i + n - 1) % n };
            if items[i] != "-" {
                break;
            }
        }
        if let Some(menu) = &mut self.menu {
            menu.item = Some(i);
        }
    }

    fn run_item(&mut self, item: usize, fx: &mut Vec<Effect>) {
        let Some(menu) = &self.menu else { return };
        let id = MENUS[menu.index].1[item];
        if id == "-" {
            return;
        }
        self.close_menu();
        self.run(id, None, fx);
    }

    pub(crate) fn menu_key(&mut self, key: Key, fx: &mut Vec<Effect>) {
        if self.context.is_some() {
            return self.context_key(key, fx);
        }
        let Some(menu) = &mut self.menu else {
            self.focus = Focus::Editor;
            return;
        };
        let n = MENUS.len();
        match key.code {
            KeyCode::Esc => self.close_menu(),
            KeyCode::Left => {
                menu.index = (menu.index + n - 1) % n;
                menu.item = Some(0);
            }
            KeyCode::Right => {
                menu.index = (menu.index + 1) % n;
                menu.item = Some(0);
            }
            KeyCode::Up => self.step_item(false),
            KeyCode::Down => self.step_item(true),
            KeyCode::Enter => {
                if let Some(item) = menu.item {
                    self.run_item(item, fx);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn menu_view(&self) -> Option<MenuView> {
        let menu = self.menu.as_ref()?;
        Some(MenuView {
            titles: MENUS.iter().map(|(t, _)| t.to_string()).collect(),
            open: menu.index,
            items: self
                .menu_items(menu.index)
                .into_iter()
                .map(|(id, label, key)| super::ContextItem {
                    label: if id == "-" { String::new() } else { label },
                    key: (!key.is_empty()).then_some(key),
                })
                .collect(),
            selected: menu.item,
        })
    }

    /// The window's menu: a title the mouse is over opens its menu.
    pub(crate) fn menu_title(&mut self, index: usize) {
        if let Some(menu) = &mut self.menu
            && index < MENUS.len()
            && menu.index != index
        {
            menu.index = index;
            menu.item = None;
        }
    }

    pub(crate) fn menu_item(&mut self, item: usize, fx: &mut Vec<Effect>) {
        if self.menu.is_some() {
            self.run_item(item, fx);
        }
    }
}
