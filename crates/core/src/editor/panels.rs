//! Panels in the two side bars: the built-in views on the left, plugin
//! panels on the right. Each side bar shows one panel at a time, with the
//! others' titles above it, and keeps its own focus and scrolling.

use super::{Editor, Effect, Focus, Plugin, Side};
use crate::editor::Command;
use crate::key::{Key, KeyCode};
use crate::panel::{Node, Panel, Target};

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
    /// Shown before its panel had an input: the first one that comes
    /// takes focus.
    pub want_input: bool,
}

impl Side {
    pub(crate) fn focus(self) -> Focus {
        match self {
            Side::Left => Focus::SideBar,
            Side::Right => Focus::Panel,
            Side::Bottom => Focus::Bottom,
            Side::Tab => Focus::Editor,
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
            Focus::Editor if self.tab_view().is_some() => Some(Side::Tab),
            _ => None,
        }
    }

    /// Records a plugin's commands and binds their keys. A key that's
    /// malformed, lacks Ctrl or Alt, or is taken is skipped with a warning.
    pub(crate) fn register(&mut self, plugin: String, commands: Vec<Command>) {
        // Started again, or stopped: its pictures go with the old instance.
        self.drop_images(&plugin);
        let mut keys = Vec::new();
        let mut problems = Vec::new();
        for command in &commands {
            let Some(spec) = &command.key else {
                continue;
            };
            let builtin = super::keymap::builtin();
            let taken = |key: Key| {
                builtin.iter().any(|b| b.keys == [key])
                    || self
                        .plugins
                        .iter()
                        .filter(|p| p.name != plugin)
                        .any(|p| p.keys.iter().any(|(k, id)| *k == key && id != &command.id))
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
        self.rebuild_keymap();
        if !problems.is_empty() {
            self.error(format!("Plugin {plugin}: {}", problems.join(", ")));
        }
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
                    view: None,
                });
                self.plugins.len() - 1
            }
        };
        &mut self.plugins[index]
    }

    pub(crate) fn panel_names(&self, side: Side) -> Vec<String> {
        if side == Side::Tab {
            return self.tab_view().into_iter().collect();
        }
        self.plugins
            .iter()
            .filter(|p| p.side == side && p.panel.is_some())
            .filter(|p| p.view.as_ref().is_none_or(|v| !v.hidden))
            .map(|p| p.name.clone())
            .collect()
    }

    /// The plugin view the focused editor tab shows, if it shows one.
    pub(crate) fn tab_view(&self) -> Option<String> {
        let (page, _) = self.doc().page.as_ref()?;
        let key = super::plugin_views::page_view(page)?;
        self.plugins
            .iter()
            .any(|p| p.name == key)
            .then(|| key.to_string())
    }

    /// Whether a plugin has a panel to show.
    pub fn has_panel(&self, plugin: &str) -> bool {
        self.plugins
            .iter()
            .any(|p| p.name == plugin && p.panel.is_some())
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

    /// Whether the focused element is a multi-line input.
    pub(crate) fn is_focused_input_multiline(&self, side: Side) -> bool {
        let Some((_, panel)) = self.active(side) else {
            return false;
        };
        let Some(Target::Input(id)) = panel.targets().into_iter().nth(self.selected_index(side))
        else {
            return false;
        };
        panel
            .nodes
            .iter()
            .any(|node| matches!(node, Node::Input { id: i, multiline: true, .. } if *i == id))
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
        self.focus = side.focus();
        match self
            .panel_targets(side)
            .iter()
            .position(|t| matches!(t, Target::Input(_)))
        {
            Some(i) => {
                self.select(side, i);
                self.bar_mut(side).want_input = false;
            }
            None => self.bar_mut(side).want_input = true,
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
            let Some(plugin) = self.owner(plugin).map(str::to_string) else {
                return self.view_click(&button, fx);
            };
            fx.push(Effect::PluginClick { plugin, button });
        }
    }

    pub(crate) fn panel_key(&mut self, side: Side, key: Key, fx: &mut Vec<Effect>) {
        if side == Side::Left && (self.explorer_key(key, fx) || self.scm_key(key, fx)) {
            return;
        }
        if side == Side::Bottom
            && self.active_panel_name(side).as_deref() == Some(super::rename::RENAME)
            && self.rename_review_key(key, fx)
        {
            return;
        }
        let count = self.panel_targets(side).len();
        let page = self.text_height().saturating_sub(2).max(1);
        let input = self.focused_input(side);
        let index = self.selected_index(side);
        let is_multiline = self.is_focused_input_multiline(side);
        // Keys in a box keep it focused by its id, so nodes the plugin
        // adds above it, as a reply, don't move focus off it.
        if let Some((_, id)) = &input {
            self.bar_mut(side).target = Some(Target::Input(id.clone()));
        }
        match key.code {
            KeyCode::Esc => self.focus = Focus::Editor,
            KeyCode::BackTab => self.select(side, index.saturating_sub(1)),
            KeyCode::Tab => self.select(side, (index + 1).min(count.saturating_sub(1))),
            KeyCode::Up => {
                if is_multiline && let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    let line_start = text[..*cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    if line_start > 0 {
                        let col = *cur - line_start;
                        let prev_line_end = line_start - 1;
                        let prev_line_start = text[..prev_line_end]
                            .rfind('\n')
                            .map(|i| i + 1)
                            .unwrap_or(0);
                        let prev_line_len = prev_line_end - prev_line_start;
                        let mut new_cur = prev_line_start + col.min(prev_line_len);
                        while !text.is_char_boundary(new_cur) {
                            new_cur -= 1;
                        }
                        *cur = new_cur;
                        return;
                    }
                }
                self.select(side, index.saturating_sub(1));
            }
            KeyCode::Down => {
                if is_multiline && let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    if let Some(nl) = text[*cur..].find('\n').map(|i| *cur + i) {
                        let line_start = text[..*cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
                        let col = *cur - line_start;
                        let next_line_start = nl + 1;
                        let next_line_end = text[next_line_start..]
                            .find('\n')
                            .map(|i| next_line_start + i)
                            .unwrap_or(text.len());
                        let next_line_len = next_line_end - next_line_start;
                        let mut new_cur = next_line_start + col.min(next_line_len);
                        while !text.is_char_boundary(new_cur) {
                            new_cur -= 1;
                        }
                        *cur = new_cur;
                        return;
                    }
                }
                self.select(side, (index + 1).min(count.saturating_sub(1)));
            }
            KeyCode::PageUp => self.select(side, index.saturating_sub(page)),
            KeyCode::PageDown => self.select(side, (index + page).min(count.saturating_sub(1))),
            KeyCode::Left => {
                if let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    while !text.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    if *cur > 0 {
                        let prev = text[..*cur].chars().next_back().map_or(1, |c| c.len_utf8());
                        *cur -= prev;
                    }
                }
            }
            KeyCode::Right => {
                if let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    while !text.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    if *cur < text.len() {
                        let next = text[*cur..].chars().next().map_or(1, |c| c.len_utf8());
                        *cur += next;
                    }
                }
            }
            KeyCode::Home => {
                if let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    let line_start = text[..*cur].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    *cur = line_start;
                }
            }
            KeyCode::End => {
                if let Some(key) = &input {
                    let text = self.inputs.get(key).map(|s| s.as_str()).unwrap_or("");
                    let cur = self.input_cursors.entry(key.clone()).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    let line_end = text[*cur..]
                        .find('\n')
                        .map(|i| *cur + i)
                        .unwrap_or(text.len());
                    *cur = line_end;
                }
            }
            KeyCode::Enter => match input {
                Some(input_key) => {
                    let should_submit = !is_multiline || key.ctrl;
                    if !should_submit {
                        let text = self.inputs.entry(input_key.clone()).or_default();
                        let cur = self.input_cursors.entry(input_key).or_insert(text.len());
                        *cur = (*cur).min(text.len());
                        while !text.is_char_boundary(*cur) {
                            *cur = cur.saturating_sub(1);
                        }
                        text.insert(*cur, '\n');
                        *cur += 1;
                        return;
                    }
                    // The built-in views keep what was typed, as a query.
                    let owner = self.owner(&input_key.0).map(str::to_string);
                    let text = if owner.is_none() {
                        self.inputs.get(&input_key).cloned().unwrap_or_default()
                    } else {
                        self.inputs.remove(&input_key).unwrap_or_default()
                    };
                    self.input_cursors.remove(&input_key);
                    if !text.trim().is_empty() {
                        if let Some(plugin) = owner {
                            fx.push(Effect::PluginSubmit {
                                plugin,
                                input: input_key.1,
                                text,
                            });
                        } else {
                            if input_key.0 == super::debug::DEBUG {
                                self.debug_submit(&input_key.1, &text, fx);
                            } else {
                                self.view_submit(&input_key.0, &text, fx);
                            }
                        }
                    }
                }
                None => self.click_target(side, index, fx),
            },
            KeyCode::Backspace => {
                if let Some(key) = input {
                    let text = self.inputs.entry(key.clone()).or_default();
                    let cur = self.input_cursors.entry(key).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    while !text.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    if *cur > 0 {
                        let prev = text[..*cur].chars().next_back().map_or(1, |c| c.len_utf8());
                        text.drain((*cur - prev)..*cur);
                        *cur -= prev;
                    }
                }
            }
            KeyCode::Delete => {
                if let Some(key) = input {
                    let text = self.inputs.entry(key.clone()).or_default();
                    let cur = self.input_cursors.entry(key).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    while !text.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    if *cur < text.len() {
                        let next = text[*cur..].chars().next().map_or(1, |c| c.len_utf8());
                        text.drain(*cur..(*cur + next));
                    }
                }
            }
            KeyCode::Char(c) if !key.ctrl && !key.alt => {
                if let Some(key) = input {
                    let text = self.inputs.entry(key.clone()).or_default();
                    let cur = self.input_cursors.entry(key).or_insert(text.len());
                    *cur = (*cur).min(text.len());
                    while !text.is_char_boundary(*cur) {
                        *cur = cur.saturating_sub(1);
                    }
                    text.insert(*cur, c);
                    *cur += c.len_utf8();
                }
            }
            _ => {}
        }
    }

    /// A panel's title as its side shows it, and a count to show after it.
    /// The side bar and the bottom panel use capitals, as VS Code does.
    pub(crate) fn panel_title(&self, side: Side, name: &str) -> (String, Option<String>) {
        let mut title = self
            .plugins
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.panel.as_ref())
            .map_or(name.to_string(), |p| p.title.clone());
        // A task's terminal is named after it, as VS Code's are.
        if name == super::terminal::TERMINAL
            && let Some(task) = self
                .terminals
                .sessions
                .iter()
                .find(|s| s.task && Some(s.id) == self.terminals.active)
        {
            title = format!("{title}: {}", task.title);
        }
        if side == Side::Right {
            return (title, None);
        }
        let badge = (name == super::views::PROBLEMS && !self.problems.is_empty())
            .then(|| self.problems.len().to_string());
        (title.to_uppercase(), badge)
    }
}
