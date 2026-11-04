//! Panels in the two side bars: the built-in views on the left, plugin
//! panels on the right. Each side bar shows one panel at a time, with the
//! others' titles above it, and keeps its own focus and scrolling.

use super::{EDITOR_KEYS, Editor, Effect, Focus, GLOBAL_KEYS, Plugin, Side, bound};
use crate::editor::Command;
use crate::key::{Key, KeyCode};
use crate::panel::{self, Panel, Row, Target};
use crate::screen::str_width;

/// One side bar's state.
#[derive(Default)]
pub(crate) struct Bar {
    pub visible: bool,
    /// The panel chosen to show.
    pub active: Option<String>,
    /// The focused element, kept by identity so elements added above it
    /// don't move focus.
    pub target: Option<Target>,
    /// Where the focused element was, for when it goes away.
    pub index: usize,
    /// Rows scrolled: down from the top on the left, where views read like
    /// lists, and up from the newest content on the right, where plugin
    /// panels read like chats.
    pub scroll: usize,
    /// Whether it scrolls to keep the focused element in view. The wheel
    /// turns it off until the next key.
    pub follow: bool,
}

/// A laid-out panel, for drawing and for mouse clicks.
pub(crate) struct PanelView {
    /// Where its content starts, and how wide it is.
    pub x: u16,
    pub inner: u16,
    pub active: String,
    /// Each panel's title: start and end column, and its name.
    pub titles: Vec<(u16, u16, String)>,
    pub title_row: u16,
    /// Rows by screen row.
    pub rows: Vec<(u16, Row)>,
    /// Rows hidden above the body.
    pub overflow: usize,
    /// The bar's scroll, after following focus.
    pub scroll: usize,
}

impl Side {
    pub(crate) fn focus(self) -> Focus {
        match self {
            Side::Left => Focus::SideBar,
            Side::Right => Focus::Panel,
            Side::Bottom => Focus::Bottom,
        }
    }
}

impl Editor {
    pub(crate) fn bar(&self, side: Side) -> &super::panels::Bar {
        &self.bars[side as usize]
    }

    pub(crate) fn bar_mut(&mut self, side: Side) -> &mut Bar {
        &mut self.bars[side as usize]
    }

    /// The side bar that has focus, if one does.
    pub(crate) fn focused_side(&self) -> Option<Side> {
        match self.focus {
            Focus::SideBar => Some(Side::Left),
            Focus::Panel => Some(Side::Right),
            Focus::Bottom => Some(Side::Bottom),
            _ => None,
        }
    }

    /// Records a plugin's commands and binds their keys. A key that's
    /// malformed, lacks Ctrl or Alt, or is taken is skipped with a warning.
    pub(crate) fn register(&mut self, plugin: String, commands: Vec<Command>) {
        let mut keys = Vec::new();
        let mut problems = Vec::new();
        for command in &commands {
            let Some(spec) = &command.key else {
                continue;
            };
            let taken = |key: Key| {
                bound(GLOBAL_KEYS, key).is_some()
                    || bound(EDITOR_KEYS, key).is_some()
                    || self
                        .plugins
                        .iter()
                        .filter(|p| p.name != plugin)
                        .any(|p| p.keys.iter().any(|(k, _)| *k == key))
            };
            match Key::parse(spec) {
                Some(key) if !(key.ctrl || key.alt) || !matches!(key.code, KeyCode::Char(_)) => {
                    problems.push(format!("{spec} needs Ctrl or Alt and a letter"))
                }
                Some(key) if taken(key) || keys.iter().any(|(k, _)| *k == key) => {
                    problems.push(format!("{spec} is taken"))
                }
                Some(key) => keys.push((key, command.id.clone())),
                None => problems.push(format!("{spec} isn't a key")),
            }
        }
        let entry = self.plugin_mut(&plugin);
        entry.commands = commands;
        entry.keys = keys;
        entry.error = None;
        if !problems.is_empty() {
            self.error(format!("Plugin {plugin}: {}", problems.join(", ")));
        }
    }

    /// The key that runs a command, for the palette and menus.
    pub(crate) fn key_hint(&self, id: &str) -> Option<String> {
        let builtin = GLOBAL_KEYS
            .iter()
            .chain(EDITOR_KEYS)
            .find(|(_, c)| *c == id);
        if let Some((spec, _)) = builtin {
            return Key::parse(spec).map(|k| k.to_string());
        }
        self.plugins.iter().find_map(|p| {
            let local = id.strip_prefix(&p.name)?.strip_prefix('.')?;
            let (key, _) = p.keys.iter().find(|(_, c)| c == local)?;
            Some(key.to_string())
        })
    }

    /// A plugin's entry, made on first use. Plugins' panels go on the
    /// right.
    pub(super) fn plugin_mut(&mut self, name: &str) -> &mut Plugin {
        let index = match self.plugins.iter().position(|p| p.name == name) {
            Some(i) => i,
            None => {
                self.plugins.push(Plugin {
                    name: name.to_string(),
                    commands: Vec::new(),
                    keys: Vec::new(),
                    panel: None,
                    error: None,
                    side: Side::Right,
                });
                self.plugins.len() - 1
            }
        };
        &mut self.plugins[index]
    }

    pub(crate) fn panel_names(&self, side: Side) -> Vec<String> {
        self.plugins
            .iter()
            .filter(|p| p.side == side && p.panel.is_some())
            .map(|p| p.name.clone())
            .collect()
    }

    /// The panel shown: the chosen one if it has a panel, else the first.
    pub(crate) fn active_panel_name(&self, side: Side) -> Option<String> {
        let names = self.panel_names(side);
        self.bar(side)
            .active
            .clone()
            .filter(|a| names.contains(a))
            .or_else(|| names.into_iter().next())
    }

    pub(crate) fn active(&self, side: Side) -> Option<(&str, &Panel)> {
        let name = self.active_panel_name(side)?;
        self.plugins
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| Some((p.name.as_str(), p.panel.as_ref()?)))
    }

    pub(crate) fn panel_targets(&self, side: Side) -> Vec<Target> {
        self.active(side)
            .map(|(_, p)| p.targets())
            .unwrap_or_default()
    }

    /// The focused input, by plugin and input id.
    pub(crate) fn focused_input(&self, side: Side) -> Option<(String, String)> {
        let (plugin, panel) = self.active(side)?;
        match panel.targets().into_iter().nth(self.selected_index(side))? {
            Target::Input(id) => Some((plugin.to_string(), id)),
            Target::Button(_) => None,
        }
    }

    /// The focused element's index. If it went away (a permission was
    /// answered, say), focus goes to the first input, so typing isn't lost on
    /// whichever button slid into its place; without an input, it stays at
    /// the same position.
    pub(crate) fn selected_index(&self, side: Side) -> usize {
        let targets = self.panel_targets(side);
        let bar = self.bar(side);
        let current = bar.target.as_ref();
        current
            .and_then(|t| targets.iter().position(|x| x == t))
            .or_else(|| {
                current?;
                targets.iter().position(|t| matches!(t, Target::Input(_)))
            })
            .unwrap_or_else(|| bar.index.min(targets.len().saturating_sub(1)))
    }

    pub(crate) fn select(&mut self, side: Side, index: usize) {
        let target = self.panel_targets(side).into_iter().nth(index);
        let bar = self.bar_mut(side);
        bar.index = index;
        bar.target = target;
    }

    /// Shows and focuses a side bar, on its panel's first input if it has
    /// one.
    pub(crate) fn focus_panel(&mut self, side: Side) {
        let bar = self.bar_mut(side);
        bar.visible = true;
        bar.scroll = 0;
        bar.follow = true;
        self.focus = side.focus();
        if let Some(i) = self
            .panel_targets(side)
            .iter()
            .position(|t| matches!(t, Target::Input(_)))
        {
            self.select(side, i);
        }
    }

    /// Steps the right side bar through plugin panels, then hides it.
    pub(crate) fn toggle_plugin_panels(&mut self) {
        let side = Side::Right;
        if !self.bar(side).visible || self.focus != Focus::Panel {
            return self.focus_panel(side);
        }
        let names = self.panel_names(side);
        let current = self.active_panel_name(side);
        let at = names.iter().position(|n| Some(n) == current.as_ref());
        match at.and_then(|i| names.get(i + 1)) {
            Some(next) => {
                self.bar_mut(side).active = Some(next.clone());
                self.focus_panel(side);
            }
            None => {
                let bar = self.bar_mut(side);
                bar.active = names.first().cloned();
                bar.visible = false;
                self.focus = Focus::Editor;
            }
        }
    }

    pub(crate) fn click_target(&mut self, side: Side, index: usize, fx: &mut Vec<Effect>) {
        self.focus = side.focus();
        self.select(side, index);
        let Some((plugin, panel)) = self.active(side) else {
            return;
        };
        if let Some(Target::Button(button)) = panel.targets().into_iter().nth(index) {
            let plugin = plugin.to_string();
            if side != Side::Right {
                return self.view_click(&button, fx);
            }
            fx.push(Effect::PluginClick { plugin, button });
        }
    }

    pub(crate) fn scroll_panel(&mut self, side: Side, up: bool, rows: usize) {
        let view = self.panel_view(side);
        let overflow = view.as_ref().map_or(0, |v| v.overflow);
        let now = view.as_ref().map(|v| v.scroll);
        let bar = self.bar_mut(side);
        // Scroll from where the view is now, even if it followed focus there.
        bar.scroll = now.unwrap_or(bar.scroll);
        bar.follow = false;
        // On the left, scrolling up goes towards the top; on the right,
        // towards older content. Both move the view up.
        let away = (side == Side::Right) == up;
        bar.scroll = if away {
            (bar.scroll + rows).min(overflow)
        } else {
            bar.scroll.saturating_sub(rows)
        };
    }

    pub(crate) fn panel_key(&mut self, side: Side, key: Key, fx: &mut Vec<Effect>) {
        self.bar_mut(side).follow = true;
        if side == Side::Left && self.explorer_key(key, fx) {
            return;
        }
        let count = self.panel_targets(side).len();
        let page = self.text_height().saturating_sub(2).max(1);
        let input = self.focused_input(side);
        let index = self.selected_index(side);
        match key.code {
            KeyCode::Esc => self.focus = Focus::Editor,
            KeyCode::Up | KeyCode::BackTab => self.select(side, index.saturating_sub(1)),
            KeyCode::Down | KeyCode::Tab => {
                self.select(side, (index + 1).min(count.saturating_sub(1)))
            }
            KeyCode::PageUp => self.scroll_panel(side, true, page),
            KeyCode::PageDown => self.scroll_panel(side, false, page),
            KeyCode::Enter => match input {
                Some(key) => {
                    // The built-in views keep what was typed, as a query.
                    let text = if side != Side::Right {
                        self.inputs.get(&key).cloned().unwrap_or_default()
                    } else {
                        self.inputs.remove(&key).unwrap_or_default()
                    };
                    if !text.trim().is_empty() {
                        self.bar_mut(side).scroll = 0;
                        if side != Side::Right {
                            self.view_submit(&key.0, &text, fx);
                        } else {
                            fx.push(Effect::PluginSubmit {
                                plugin: key.0,
                                input: key.1,
                                text,
                            });
                        }
                    }
                }
                None => self.click_target(side, index, fx),
            },
            KeyCode::Backspace => {
                if let Some(key) = input {
                    self.inputs.entry(key).or_default().pop();
                }
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                if let Some(key) = input {
                    self.inputs.entry(key).or_default().push(c);
                }
            }
            _ => {}
        }
    }

    /// A panel's title as its side shows it, and a count to show after it.
    /// The side bar and the bottom panel use capitals, as VS Code does.
    pub(crate) fn panel_title(&self, side: Side, name: &str) -> (String, Option<String>) {
        let title = self
            .plugins
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.panel.as_ref())
            .map_or(name.to_string(), |p| p.title.clone());
        if side == Side::Right {
            return (title, None);
        }
        let badge = (name == super::views::PROBLEMS && !self.problems.is_empty())
            .then(|| self.problems.len().to_string());
        (title.to_uppercase(), badge)
    }

    /// Lays out a side bar's active panel where it's drawn.
    pub(crate) fn panel_view(&self, side: Side) -> Option<PanelView> {
        let area = self.layout().side(side)?;
        let (x, inner) = (area.x + 1, area.w.saturating_sub(2));
        let (title_row, top, bottom) = (area.y, area.y + 1, area.bottom());
        let (active, panel) = self.active(side)?;

        // The side bar names only the view shown; the activity bar picks it.
        let names = match side {
            Side::Left => vec![active.to_string()],
            _ => self.panel_names(side),
        };
        let mut titles = Vec::new();
        let mut tx = 0;
        for name in names {
            let (title, badge) = self.panel_title(side, &name);
            // A badge is a space, then the count padded in a pill.
            let full = str_width(&title) + badge.map_or(0, |b| str_width(&b) + 3);
            let w = (full as u16).min(inner.saturating_sub(tx));
            titles.push((x + tx, x + tx + w, name));
            tx += w + 3;
        }

        let bar = self.bar(side);
        let focused = (self.focus == side.focus()).then_some(self.selected_index(side));
        let colour = |lang: &str, text: &str| self.colours.get(lang, text);
        let layout = panel::layout(
            panel,
            inner as usize,
            focused,
            |id| {
                self.inputs
                    .get(&(active.to_string(), id.to_string()))
                    .cloned()
                    .unwrap_or_default()
            },
            &colour,
        );
        let rows_h = bottom.saturating_sub(top) as usize;
        let skip = layout.footer.len().saturating_sub(rows_h);
        let footer: Vec<Row> = layout.footer.into_iter().skip(skip).collect();
        let body_h = rows_h - footer.len();
        let overflow = layout.body.len().saturating_sub(body_h);
        let mut start = match side {
            Side::Right => overflow - bar.scroll.min(overflow),
            _ => bar.scroll.min(overflow),
        };
        // Keep the focused element in view.
        if let Some(target) = focused.filter(|_| bar.follow)
            && let Some(row) = layout
                .body
                .iter()
                .position(|r| r.hits.iter().any(|h| h.2 == target))
        {
            if row < start {
                start = row;
            } else if row >= start + body_h {
                start = row + 1 - body_h;
            }
        }
        let scroll = match side {
            Side::Right => overflow - start.min(overflow),
            _ => start,
        };
        let rows = layout
            .body
            .into_iter()
            .skip(start)
            .take(body_h)
            .enumerate()
            .map(|(i, r)| (top + i as u16, r))
            .chain(
                footer
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| (top + (body_h + i) as u16, r)),
            )
            .collect();
        Some(PanelView {
            x,
            inner,
            active: active.to_string(),
            titles,
            title_row,
            rows,
            overflow,
            scroll,
        })
    }
}
