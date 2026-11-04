//! The menu bar, as in VS Code. Its items are commands, so each shows the
//! key that runs it and the palette finds the same things.

use super::{Editor, Effect, Focus};
use crate::key::{Key, KeyCode};
use crate::screen::{Screen, Style, str_width};

/// Each menu's title and commands; `-` is a separator.
const MENUS: &[(&str, &[&str])] = &[
    (
        "File",
        &["file.open", "file.save", "-", "file.close", "-", "app.quit"],
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

    /// Each title's start and end column.
    pub(crate) fn menu_titles(&self) -> Vec<(u16, u16)> {
        let mut x = 1;
        MENUS
            .iter()
            .map(|(title, _)| {
                let w = str_width(title) as u16 + 2;
                let span = (x, x + w);
                x += w;
                span
            })
            .collect()
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

    /// The open menu's box, frame included: left edge, width, and its
    /// items, which start on row 2.
    fn dropdown(&self) -> Option<(u16, u16, Vec<Item>)> {
        let menu = self.menu.as_ref()?;
        let items = self.menu_items(menu.index);
        let label_w = items
            .iter()
            .map(|(_, l, _)| str_width(l))
            .max()
            .unwrap_or(0);
        let key_w = items
            .iter()
            .map(|(_, _, k)| str_width(k))
            .max()
            .unwrap_or(0);
        let width = (label_w + key_w + 8).min(self.width as usize) as u16;
        let x = self.menu_titles()[menu.index]
            .0
            .min(self.width.saturating_sub(width));
        Some((x, width, items))
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

    /// A click while a menu is open, or on the menu bar. Returns whether
    /// the menu used it.
    pub(crate) fn menu_click(&mut self, x: u16, y: u16, fx: &mut Vec<Effect>) -> bool {
        if y == 0 && self.layout().menu.h > 0 {
            match self
                .menu_titles()
                .iter()
                .position(|(a, b)| (*a..*b).contains(&x))
            {
                Some(i) if self.menu.as_ref().is_some_and(|m| m.index == i) => self.close_menu(),
                Some(i) => {
                    self.menu = Some(Menu {
                        index: i,
                        item: None,
                    });
                    self.popup = None;
                    self.focus = Focus::Menu;
                }
                None => {
                    self.close_menu();
                    if let Some(action) = self.title_action(x) {
                        self.act(action, fx);
                    }
                }
            }
            return true;
        }
        let Some((left, width, items)) = self.dropdown() else {
            return false;
        };
        let row = y as usize;
        if (left..left + width).contains(&x) && (2..2 + items.len()).contains(&row) {
            self.run_item(row - 2, fx);
        } else {
            self.close_menu();
        }
        true
    }

    pub(crate) fn render_menu_bar(&self, s: &mut Screen) {
        let bar = self.layout().menu;
        if bar.h == 0 {
            return;
        }
        s.fill(0, 0, bar.w, Style::MenuBar);
        let open = self.menu.as_ref().map(|m| m.index);
        for (i, (x0, x1)) in self.menu_titles().into_iter().enumerate() {
            let style = if open == Some(i) {
                Style::MenuOpen
            } else {
                Style::MenuBar
            };
            s.print(x0, 0, &format!(" {} ", MENUS[i].0), style, x1 - x0);
        }
        self.render_title_items(s);
        let hovered = self.hover.filter(|&(_, y)| y == 0 && self.menu.is_none());
        for (x0, x1) in self.menu_titles() {
            if hovered.is_some_and(|(x, _)| (x0..x1).contains(&x)) {
                s.hover(x0, 0, x1 - x0);
            }
        }
    }

    pub(crate) fn render_menu(&self, s: &mut Screen) {
        let Some((x, width, items)) = self.dropdown() else {
            return;
        };
        let selected = self.menu.as_ref().and_then(|m| m.item);
        let height = (items.len() as u16 + 2).min(s.height.saturating_sub(1));
        s.frame(x, 1, width, height, Style::WidgetFrame, Style::Menu);
        // Inside the frame.
        let (x, width) = (x + 1, width - 2);
        for (i, (id, label, key)) in items.iter().enumerate() {
            let y = 2 + i as u16;
            if y + 1 > height {
                break;
            }
            if *id == "-" {
                s.put(x - 1, y, '├', Style::WidgetFrame);
                s.print(x, y, &"─".repeat(width as usize), Style::WidgetFrame, width);
                s.put(x + width, y, '┤', Style::WidgetFrame);
                continue;
            }
            let style = if selected == Some(i) {
                Style::MenuSelected
            } else {
                Style::Menu
            };
            s.fill(x, y, width, style);
            s.print(x + 1, y, label, style, width.saturating_sub(2));
            let kw = str_width(key) as u16;
            let hint = if selected == Some(i) {
                style
            } else {
                Style::MenuMuted
            };
            s.print(x + width - kw - 1, y, key, hint, kw);
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
