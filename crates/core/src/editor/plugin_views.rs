//! Plugins' own views, as their manifests declare them: each has a place
//! (the left side bar, the right one, the panel, or an editor tab), an
//! icon, a badge, a toolbar and a text for when it's empty. The place is
//! only the plugin's default: the user can move a view, and the workspace
//! remembers where.

use std::path::PathBuf;

use super::context::ContextMenu;
use super::{Editor, Focus, Plugin, Side};
use crate::panel::{Node, Panel, Tone};

/// Where a view goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Place {
    /// In the activity rail, with the built-in views.
    Left,
    /// In the secondary side bar, under its own row of tabs.
    #[default]
    Right,
    /// A tab in the panel, beside Problems, Output and Terminal.
    Bottom,
    /// A tab in the editor area.
    Tab,
}

impl Place {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "left" => Self::Left,
            "right" => Self::Right,
            "bottom" => Self::Bottom,
            "tab" => Self::Tab,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Bottom => "bottom",
            Self::Tab => "tab",
        }
    }

    fn side(self) -> Side {
        match self {
            Self::Left => Side::Left,
            Self::Right => Side::Right,
            Self::Bottom => Side::Bottom,
            Self::Tab => Side::Tab,
        }
    }
}

/// A view's or a toolbar button's icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewIcon {
    /// An SVG file in the plugin's folder, drawn in the colour of the text
    /// around it.
    Svg(PathBuf),
    /// A glyph, from a name merlin knows (`plus`, `refresh`, …) or given as
    /// itself.
    Glyph(String),
}

impl ViewIcon {
    /// Reads a manifest's `icon`: a file ending `.svg` in `dir`, a glyph's
    /// name, or one or two characters as they are.
    pub fn parse(text: &str, dir: &std::path::Path) -> Option<Self> {
        let text = text.trim();
        if text.ends_with(".svg") {
            return Some(Self::Svg(dir.join(text)));
        }
        if let Some(glyph) = glyph_named(text) {
            return Some(Self::Glyph(glyph.into()));
        }
        (!text.is_empty() && text.chars().count() <= 2).then(|| Self::Glyph(text.into()))
    }

    /// The glyph to draw when there's no picture.
    pub fn glyph(&self) -> Option<&str> {
        match self {
            Self::Glyph(g) => Some(g),
            Self::Svg(_) => None,
        }
    }

    pub fn svg(&self) -> Option<&PathBuf> {
        match self {
            Self::Svg(path) => Some(path),
            Self::Glyph(_) => None,
        }
    }
}

/// Glyphs a manifest can name, for toolbars and views without a picture.
const GLYPHS: &[(&str, &str)] = &[
    ("plus", "+"),
    ("add", "+"),
    ("refresh", "↻"),
    ("reload", "↻"),
    ("download", "⇣"),
    ("upload", "⇡"),
    ("more", "⋯"),
    ("close", "×"),
    ("search", "⌕"),
    ("filter", "⏷"),
    ("play", "▷"),
    ("run", "▷"),
    ("stop", "■"),
    ("check", "✓"),
    ("gear", "⚙"),
    ("settings", "⚙"),
    ("list", "☰"),
    ("chat", "💬"),
    ("star", "★"),
    ("bell", "🔔"),
    ("trash", "🗑"),
    ("edit", "✎"),
    ("expand", "⤢"),
    ("collapse", "⤡"),
];

fn glyph_named(name: &str) -> Option<&'static str> {
    GLYPHS.iter().find(|(n, _)| *n == name).map(|(_, g)| *g)
}

/// A button in a view's header that runs one of its plugin's commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolButton {
    /// The plugin's command, by its id without the plugin's name.
    pub command: String,
    pub icon: Option<ViewIcon>,
}

/// A view, as `[[views]]` in a plugin's manifest declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSpec {
    pub id: String,
    pub title: String,
    pub icon: Option<ViewIcon>,
    pub place: Place,
    /// Shown while the plugin gives the view nothing to show.
    pub empty: String,
    pub toolbar: Vec<ToolButton>,
}

/// What a plugin view's entry knows beyond its panel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewInfo {
    /// The plugin that draws it.
    pub plugin: String,
    /// Its id within the plugin; empty for a panel from `render-panel`.
    pub id: String,
    pub icon: Option<ViewIcon>,
    /// Where the plugin puts it.
    pub place: Place,
    pub empty: String,
    pub toolbar: Vec<ToolButton>,
    pub badge: u32,
    /// Left out of the rail and the side bars' tabs, until shown again.
    pub hidden: bool,
}

/// A right-click menu plugins can add to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPlace {
    Editor,
    Explorer,
    Tab,
}

impl MenuPlace {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "editor" => Self::Editor,
            "explorer" => Self::Explorer,
            "tab" => Self::Tab,
            _ => return None,
        })
    }
}

/// One of a plugin's commands in a right-click menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub place: MenuPlace,
    /// The plugin's command, without its name.
    pub command: String,
    /// What the menu says; the command's title without.
    pub title: Option<String>,
}

/// What a plugin's item in a right-click menu does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Slot {
    /// A row's menu button: a click, to `on-click`.
    Click(String),
    /// A command from `[[menus]]`.
    Command(String),
}

/// A view's name among the side bars' panels: its plugin's, then its id.
pub fn view_key(plugin: &str, id: &str) -> String {
    format!("{plugin}/{id}")
}

/// An editor tab's page name, for a view opened in one.
pub(crate) fn tab_page(key: &str) -> String {
    format!("view:{key}")
}

/// The view an editor tab's page shows, by its page name.
pub(crate) fn page_view(page: &str) -> Option<&str> {
    page.strip_prefix("view:")
}

/// A plugin row's menu items, by place; a row offers up to this many.
const ROW_ITEMS: &[&str] = &[
    "row.menu.0",
    "row.menu.1",
    "row.menu.2",
    "row.menu.3",
    "row.menu.4",
    "row.menu.5",
    "row.menu.6",
    "row.menu.7",
    "row.menu.8",
    "row.menu.9",
    "row.menu.10",
    "row.menu.11",
];

/// The right-click menu on a view's icon or tab.
pub(crate) const VIEW_ITEMS: &[(&str, &str)] = &[
    ("view.move_left", "Move to the Left Side Bar"),
    ("view.move_right", "Move to the Right Side Bar"),
    ("view.move_bottom", "Move to the Panel"),
    ("view.move_tab", "Open as an Editor Tab"),
    ("-", ""),
    ("view.hide", "Hide"),
    ("view.reset", "Reset Location"),
];

impl Editor {
    /// A plugin's views, as its manifest declares them: each gets an
    /// entry where it goes, and views it no longer has go. None, when it's
    /// unloaded or turned off.
    pub(crate) fn register_views(&mut self, plugin: &str, views: Vec<ViewSpec>) {
        let prefix = format!("{plugin}/");
        let keys: Vec<String> = views.iter().map(|v| view_key(plugin, &v.id)).collect();
        self.plugins
            .retain(|p| !p.name.starts_with(&prefix) || keys.contains(&p.name));
        // Tabs showing a view that's gone close.
        let gone: Vec<String> = self
            .docs
            .iter()
            .filter_map(|d| d.page.as_ref())
            .filter_map(|(page, _)| page_view(page))
            .filter(|k| k.starts_with(&prefix) && !keys.iter().any(|key| key == k))
            .map(tab_page)
            .collect();
        for page in gone {
            self.close_pages(&page);
        }
        for (spec, key) in views.into_iter().zip(keys) {
            let place = self.moved_view(&key).unwrap_or(spec.place);
            let info = ViewInfo {
                plugin: plugin.to_string(),
                id: spec.id,
                icon: spec.icon,
                place: spec.place,
                empty: spec.empty,
                toolbar: spec.toolbar,
                badge: 0,
                hidden: self.hidden_views.contains(&key),
            };
            let title = spec.title;
            match self.plugins.iter_mut().find(|p| p.name == key) {
                Some(entry) => {
                    entry.side = place.side();
                    let badge = entry.view.as_ref().map_or(0, |v| v.badge);
                    entry.view = Some(ViewInfo { badge, ..info });
                    match &mut entry.panel {
                        Some(panel) => panel.title = title,
                        None => {
                            entry.panel = Some(Panel {
                                title,
                                nodes: Vec::new(),
                            })
                        }
                    }
                }
                None => self.plugins.push(Plugin {
                    name: key,
                    commands: Vec::new(),
                    keys: Vec::new(),
                    panel: Some(Panel {
                        title,
                        nodes: Vec::new(),
                    }),
                    error: None,
                    side: place.side(),
                    view: Some(info),
                }),
            }
        }
    }

    /// A panel a plugin drew. A view of its manifest keeps its title and
    /// its place when the plugin gives it nothing.
    pub(crate) fn set_panel(&mut self, name: &str, panel: Option<Panel>) {
        let entry = self.plugin_mut(name);
        match (&mut entry.panel, entry.view.is_some(), panel) {
            (Some(shown), true, panel) => {
                shown.nodes = panel.map(|p| p.nodes).unwrap_or_default();
            }
            (_, _, panel) => entry.panel = panel,
        }
        // Shown before it had an input: its first one takes focus now.
        for side in [Side::Left, Side::Right, Side::Bottom, Side::Tab] {
            if self.bar(side).want_input
                && self.active_panel_name(side).as_deref() == Some(name)
                && let Some(i) = self
                    .panel_targets(side)
                    .iter()
                    .position(|t| matches!(t, crate::panel::Target::Input(_)))
            {
                self.select(side, i);
                self.bar_mut(side).want_input = false;
            }
        }
    }

    /// Where the user moved a view, if they did.
    fn moved_view(&self, key: &str) -> Option<Place> {
        self.view_places
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, p)| *p)
    }

    /// The plugin that draws a panel, if a plugin does.
    pub(crate) fn owner(&self, name: &str) -> Option<&str> {
        let entry = self.plugins.iter().find(|p| p.name == name)?;
        match &entry.view {
            Some(view) => Some(&view.plugin),
            // A panel from `render-panel`: the plugin's own entry. The
            // built-in views' names start `merlin.`, which a plugin's can't.
            None => (!entry.name.starts_with("merlin.")).then_some(entry.name.as_str()),
        }
    }

    /// The plugin views that show now, by name: the panel each side bar
    /// shows, and views open in editor tabs on screen.
    pub fn visible_views(&self) -> Vec<String> {
        let mut shown = Vec::new();
        for side in [Side::Left, Side::Right, Side::Bottom] {
            if self.bar(side).visible
                && let Some(name) = self.active_panel_name(side)
                && self.view_info(&name).is_some()
            {
                shown.push(name);
            }
        }
        for g in 0..self.groups.len() {
            let doc = self.as_group(g, || self.pane_doc());
            if let Some((page, _)) = &self.docs[doc].page
                && let Some(key) = page_view(page)
                && !shown.iter().any(|s| s == key)
            {
                shown.push(key.to_string());
            }
        }
        shown
    }

    pub(crate) fn view_info(&self, name: &str) -> Option<&ViewInfo> {
        self.plugins
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.view.as_ref())
    }

    /// Sets a view's badge, by the plugin and the view's id.
    pub(crate) fn set_view_badge(&mut self, plugin: &str, id: &str, count: u32) {
        let key = view_key(plugin, id);
        if let Some(view) = self
            .plugins
            .iter_mut()
            .find(|p| p.name == key)
            .and_then(|p| p.view.as_mut())
        {
            view.badge = count;
        }
    }

    /// Shows a view where it lives, and focuses it.
    pub(crate) fn show_plugin_view(&mut self, key: &str) {
        let Some(entry) = self.plugins.iter_mut().find(|p| p.name == key) else {
            return;
        };
        if let Some(view) = &mut entry.view {
            view.hidden = false;
        }
        self.hidden_views.retain(|k| k != key);
        let side = entry.side;
        let title = entry
            .panel
            .as_ref()
            .map_or(key.to_string(), |p| p.title.clone());
        if side == Side::Tab {
            self.open_page(&tab_page(key), &title);
            return self.focus_panel(Side::Tab);
        }
        self.bar_mut(side).active = Some(key.to_string());
        self.focus_panel(side);
    }

    /// A plugin view's panel as it's drawn: what the plugin gave, or the
    /// view's empty text while that's nothing.
    pub(crate) fn shown_panel<'a>(&'a self, name: &str, panel: &'a Panel) -> Panel {
        match self.view_info(name) {
            Some(view) if panel.nodes.is_empty() && !view.empty.is_empty() => Panel {
                title: panel.title.clone(),
                nodes: vec![Node::Styled(view.empty.clone(), Tone::Muted)],
            },
            _ => panel.clone(),
        }
    }

    /// Opens the right-click menu for a plugin view: where it can move.
    pub(crate) fn open_view_menu(&mut self, key: String) {
        let Some(entry) = self.plugins.iter().find(|p| p.name == key) else {
            return;
        };
        let Some(view) = &entry.view else {
            return;
        };
        let here = entry.side;
        let moved = here != view.place.side();
        let plugin_title = self
            .plugin_list
            .iter()
            .find(|p| p.name == view.plugin)
            .map_or(view.plugin.clone(), |p| p.title.clone());
        let title = entry
            .panel
            .as_ref()
            .map_or(key.clone(), |p| p.title.clone());
        let items = VIEW_ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|&id| match id {
                "view.move_left" => here != Side::Left,
                "view.move_right" => here != Side::Right,
                "view.move_bottom" => here != Side::Bottom,
                "view.move_tab" => here != Side::Tab,
                "view.hide" => here != Side::Tab,
                "view.reset" => moved,
                _ => true,
            })
            .collect();
        self.popup = None;
        self.view_menu = Some(key);
        self.context = Some(ContextMenu {
            items,
            selected: None,
            title: Some(format!("{title} · from {plugin_title}")),
        });
        self.focus = Focus::Menu;
    }

    /// A right-click on a plugin view's row: the menu the row offers.
    pub(crate) fn plugin_row_menu(&mut self, name: &str, side: Side, index: usize) {
        let Some(plugin) = self.owner(name).map(str::to_string) else {
            return;
        };
        let Some(crate::panel::Target::Button(id)) =
            self.panel_targets(side).into_iter().nth(index)
        else {
            return;
        };
        let menu = self.active(side).and_then(|(_, panel)| {
            panel.nodes.iter().find_map(|node| match node {
                Node::Entry(e) if e.id == id && !e.menu.is_empty() => Some(e.menu.clone()),
                _ => None,
            })
        });
        let Some(menu) = menu else { return };
        let items = ROW_ITEMS.iter().take(menu.len()).copied().collect();
        self.popup = None;
        self.menu_slots = menu
            .into_iter()
            .map(|b| (plugin.clone(), b.label, Slot::Click(b.id)))
            .collect();
        self.context = Some(ContextMenu {
            items,
            selected: None,
            title: None,
        });
        self.focus = Focus::Menu;
    }

    /// A plugin's item in a menu: its label, by its place.
    pub(crate) fn row_menu_label(&self, id: &str) -> Option<String> {
        let i: usize = id.strip_prefix("row.menu.")?.parse().ok()?;
        Some(self.menu_slots.get(i)?.1.clone())
    }

    /// A plugin's item in a menu picked: its click or command goes to it.
    pub(crate) fn row_menu_picked(&mut self, id: &str, fx: &mut Vec<super::Effect>) -> bool {
        let Some(i) = id
            .strip_prefix("row.menu.")
            .and_then(|i| i.parse::<usize>().ok())
        else {
            return false;
        };
        if let Some((plugin, _, slot)) = std::mem::take(&mut self.menu_slots).into_iter().nth(i) {
            fx.push(match slot {
                Slot::Click(button) => super::Effect::PluginClick { plugin, button },
                Slot::Command(command) => super::Effect::PluginCommand { plugin, command },
            });
        }
        true
    }

    /// Plugins' commands in right-click menus.
    pub(crate) fn register_menus(&mut self, plugin: &str, items: Vec<MenuItem>) {
        self.menus.retain(|(p, _)| p != plugin);
        self.menus
            .extend(items.into_iter().map(|i| (plugin.to_string(), i)));
    }

    /// Adds plugins' commands for `place` to the menu just opened, after
    /// a line, remembering the file it's about for `menu-target`.
    pub(crate) fn add_plugin_items(&mut self, place: MenuPlace, target: Option<String>) {
        self.menu_target = target;
        self.menu_slots.clear();
        for (plugin, item) in &self.menus {
            if item.place != place || self.menu_slots.len() >= ROW_ITEMS.len() {
                continue;
            }
            // Only a command the plugin has, by the menu's title or its own.
            let Some(command) = self
                .plugins
                .iter()
                .find(|p| p.name == *plugin)
                .and_then(|p| p.commands.iter().find(|c| c.id == item.command))
            else {
                continue;
            };
            let label = item.title.clone().unwrap_or_else(|| command.title.clone());
            self.menu_slots
                .push((plugin.clone(), label, Slot::Command(item.command.clone())));
        }
        if self.menu_slots.is_empty() {
            return;
        }
        if let Some(menu) = &mut self.context {
            menu.items.push("-");
            menu.items
                .extend(ROW_ITEMS.iter().take(self.menu_slots.len()).copied());
        }
    }

    /// The file the last right-click menu was opened on.
    pub fn menu_target(&self) -> Option<String> {
        self.menu_target.clone()
    }

    /// Runs one of [`VIEW_ITEMS`] on the view whose menu was opened.
    pub(crate) fn view_menu_command(&mut self, id: &str) -> bool {
        let place = match id {
            "view.move_left" => Some(Place::Left),
            "view.move_right" => Some(Place::Right),
            "view.move_bottom" => Some(Place::Bottom),
            "view.move_tab" => Some(Place::Tab),
            "view.hide" | "view.reset" => None,
            _ => return false,
        };
        let Some(key) = self.view_menu.take() else {
            return true;
        };
        match (id, place) {
            ("view.hide", _) => self.hide_view(&key),
            ("view.reset", _) => {
                let default = self.view_info(&key).map(|v| v.place);
                if let Some(default) = default {
                    self.view_places.retain(|(k, _)| *k != key);
                    self.place_view(&key, default);
                }
            }
            (_, Some(place)) => self.move_view(&key, place),
            _ => {}
        }
        true
    }

    /// Moves a view where the user wants it, and remembers.
    pub(crate) fn move_view(&mut self, key: &str, place: Place) {
        let Some(default) = self.view_info(key).map(|v| v.place) else {
            return;
        };
        self.view_places.retain(|(k, _)| k != key);
        if place != default {
            self.view_places.push((key.to_string(), place));
        }
        self.place_view(key, place);
    }

    /// Whether a panel is a plugin's view, which can move.
    pub fn is_plugin_view(&self, name: &str) -> bool {
        self.view_info(name).is_some()
    }

    /// Moves a view and shows it there.
    fn place_view(&mut self, key: &str, place: Place) {
        let Some(entry) = self.plugins.iter_mut().find(|p| p.name == key) else {
            return;
        };
        let from = entry.side;
        entry.side = place.side();
        if from == Side::Tab {
            let page = tab_page(key);
            self.close_pages(&page);
        } else if self.bar(from).active.as_deref() == Some(key) {
            self.bar_mut(from).active = None;
        }
        self.show_plugin_view(key);
    }

    /// Leaves a view out of the rail and the side bars' tabs. View: Show
    /// in the palette brings it back.
    fn hide_view(&mut self, key: &str) {
        let Some(entry) = self.plugins.iter_mut().find(|p| p.name == key) else {
            return;
        };
        if let Some(view) = &mut entry.view {
            view.hidden = true;
        }
        let side = entry.side;
        if !self.hidden_views.iter().any(|k| k == key) {
            self.hidden_views.push(key.to_string());
        }
        if self.active_panel_name(side).as_deref() == Some(key) {
            self.bar_mut(side).active = None;
            if self.panel_names(side).is_empty() {
                self.bar_mut(side).visible = false;
                if self.focus == side.focus() {
                    self.focus = Focus::Editor;
                }
            }
        }
    }

    /// Closes every tab showing a page.
    fn close_pages(&mut self, page: &str) {
        let shows = |d: &crate::doc::Doc| d.page.as_ref().is_some_and(|(n, _)| n == page);
        if !self.docs.iter().any(shows) {
            return;
        }
        self.docs.retain(|d| !shows(d));
        if self.docs.is_empty() {
            let id = self.new_id();
            self.docs.push(crate::doc::Doc::new(id, "", None));
        }
        self.current = self.current.min(self.docs.len() - 1);
        self.sync_groups();
    }

    /// The palette, listing the views to show: the right side bar's +.
    pub(crate) fn list_views(&mut self, fx: &mut Vec<super::Effect>) {
        self.open_search(super::search_all::Scope::Commands, fx);
        if let Some(palette) = &mut self.palette {
            palette.query = "View: Show ".into();
        }
        self.search_changed();
    }

    /// The palette's View: Show commands, one for each plugin view.
    pub(crate) fn view_commands(&self) -> Vec<(String, String)> {
        self.plugins
            .iter()
            .filter(|p| p.view.is_some())
            .filter_map(|p| {
                let title = &p.panel.as_ref()?.title;
                Some((
                    format!("view.show:{}", p.name),
                    format!("View: Show {title}"),
                ))
            })
            .collect()
    }

    /// Where the user put views, for the workspace to remember, and the
    /// views they hid.
    pub(crate) fn view_layout(&self) -> (Vec<(String, Place)>, Vec<String>) {
        (self.view_places.clone(), self.hidden_views.clone())
    }

    /// Brings back where the user put views.
    pub(crate) fn restore_view_layout(
        &mut self,
        places: Vec<(String, Place)>,
        hidden: Vec<String>,
    ) {
        self.view_places = places;
        self.hidden_views = hidden;
        let entries: Vec<(String, Place)> = self
            .plugins
            .iter()
            .filter_map(|p| Some((p.name.clone(), p.view.as_ref()?.place)))
            .collect();
        for (key, default) in entries {
            let place = self.moved_view(&key).unwrap_or(default);
            let hidden = self.hidden_views.contains(&key);
            if let Some(entry) = self.plugins.iter_mut().find(|p| p.name == key) {
                entry.side = place.side();
                if let Some(view) = &mut entry.view {
                    view.hidden = hidden;
                }
            }
        }
    }
}
