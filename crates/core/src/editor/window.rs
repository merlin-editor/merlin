//! The editor in a window that draws its own frame. The window lays out the
//! title bar, activity rail, side bars, tabs, breadcrumbs, status bar,
//! palette and notices in pixels, from the [`WindowView`] here, and says what
//! was clicked with a [`UiEvent`]. The editor's screen is then only the
//! text: every editor group, with its gutter, scrollbar and popups.

use super::chrome::Action;
use super::context::{ContextView, PopupView};
use super::find::{FindAction, FindView};
use super::menu::MenuView;
use super::{Edge, Editor, Effect, Focus, Side, Source};
use crate::icons::{Icon, icon};
use crate::panel::{Node, Target};
use crate::style::Hue;

/// Something clicked in a window's frame.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    /// An activity rail item, by its index in [`WindowView::rail`].
    Rail(usize),
    /// A terminal session, by id, clicked in the list or in its pane.
    Terminal(u64),
    /// A session's kill button.
    TerminalKill(u64),
    /// A tab of the focused group, by its index in [`WindowView::tabs`].
    Tab {
        index: usize,
        clicks: u8,
    },
    CloseTab(usize),
    /// A breadcrumb, by its index in [`WindowView::crumbs`].
    Crumb(usize),
    /// A status bar item, by its index in [`WindowView::status`].
    Status(usize),
    /// A panel's focusable element, by its index among the panel's targets
    /// (see [`SideView::targets`]).
    Target {
        side: Side,
        index: usize,
        /// 2 for a double click; a file opens to stay then.
        clicks: u8,
    },
    /// A panel's checkbox, by its row's index among the panel's targets.
    Check {
        side: Side,
        index: usize,
    },
    /// A button that isn't one of a panel's targets, such as a header's.
    Press(String),
    /// One of a side's panel titles, to show that panel.
    PanelTitle {
        side: Side,
        name: String,
    },
    /// A palette row, by its index among all the palette's items.
    PaletteRow(usize),
    ClosePalette,
    /// A notice's button, by its index.
    ToastButton(usize),
    /// A right-click menu item, by its index in [`ContextView::items`].
    ContextItem(usize),
    CloseContext,
    /// The menu under a title of [`MenuView::titles`], an item of the open
    /// menu, or a click away from them.
    MenuTitle(usize),
    MenuItem(usize),
    CloseMenu,
    Find(FindAction),
    /// A tab of group `group` when there are several, as [`Self::Tab`].
    GroupTab {
        group: usize,
        index: usize,
        clicks: u8,
    },
    CloseGroupTab {
        group: usize,
        index: usize,
    },
    /// A breadcrumb of a group other than the focused one.
    GroupCrumb {
        group: usize,
        index: usize,
    },
    /// The focused tab dragged to group `group`, before its `at`th tab or
    /// at the end.
    DropTab {
        group: usize,
        at: Option<usize>,
    },
    /// The focused tab dropped on a side of group `group`'s code: it moves
    /// to a new group there.
    SplitTab {
        group: usize,
        edge: Edge,
    },
    /// A file of a side's view (its `index`th element) dropped on group
    /// `group`'s code: it opens there, or in a new group on `edge`.
    DropEntry {
        side: Side,
        index: usize,
        group: usize,
        edge: Option<Edge>,
    },
    /// The sashes between groups were dragged: each group's new width and
    /// height shares, as in [`super::tabs::GroupView`].
    GroupSizes(Vec<(f32, f32)>),
    /// A dialog's `n`th button.
    DialogButton(usize),
    /// A right click on the focused tab.
    TabMenu,
    /// A right click on a side's element, or on its empty space.
    TargetMenu {
        side: Side,
        index: usize,
    },
    SideMenu(Side),
    /// A button in the hover card.
    Hover(super::context::HoverButton),
    /// A row of the code action list, by its index.
    Action(usize),
}

/// What the window draws around the text.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowView {
    pub project: String,
    pub branch: Option<String>,
    pub rail: Vec<RailItem>,
    pub left: Option<SideView>,
    pub right: Option<SideView>,
    pub bottom: Option<SideView>,
    /// Whether each side shows, for the title bar's layout buttons:
    /// left, bottom, right.
    pub shown: [bool; 3],
    pub tabs: Vec<TabView>,
    pub crumbs: Vec<String>,
    pub status: Vec<StatusView>,
    /// The problem under the cursor, for the middle of the status bar.
    pub problem: Option<(String, bool)>,
    pub palette: Option<PaletteView>,
    pub toast: Option<ToastView>,
    pub context: Option<ContextView>,
    pub popup: Option<PopupView>,
    pub signature: Option<super::context::SignatureView>,
    pub menu: Option<MenuView>,
    pub find: Option<FindView>,
    /// Every editor group, to lay out and draw; `tabs` and `crumbs` are
    /// the focused one's.
    pub groups: Vec<super::tabs::GroupView>,
    /// Where keys go.
    pub focus: Focus,
    /// The page the active tab shows instead of text, by name, such as a
    /// plugin's.
    pub page: Option<String>,
    /// Whether the minimap shows beside the code.
    pub minimap: bool,
    /// A question to answer before anything else.
    pub dialog: Option<super::DialogView>,
    /// The terminal panel, when it shows.
    pub terminal: Option<super::TerminalView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailItem {
    /// What it shows, for icon themes: `explorer`, `search`,
    /// `source-control`, `outline`, or `plugin` for a plugin's panel.
    pub id: &'static str,
    pub glyph: char,
    pub label: String,
    pub active: bool,
    pub badge: usize,
}

/// A side bar or the bottom panel, showing one panel.
#[derive(Clone, Debug, PartialEq)]
pub struct SideView {
    pub title: String,
    /// Panels that can show here, by name and title, and whether each is the
    /// one showing: the bottom panel's Problems and Output, or plugin panels.
    pub panels: Vec<(String, String, bool)>,
    pub nodes: Vec<Node>,
    /// For each node, the index of its first focusable element, if it has
    /// one: what a click on it sends as [`UiEvent::Target`].
    pub targets: Vec<Option<usize>>,
    /// The focused element's index among the panel's targets.
    pub selected: usize,
    /// Whether keys go to this side.
    pub focused: bool,
    /// Text typed into each input, by input id.
    pub inputs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabView {
    pub name: String,
    pub icon: (char, Hue),
    pub active: bool,
    pub dirty: bool,
    pub preview: bool,
    /// Pinned at the start of the strip, drawn as only its icon.
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusView {
    pub text: String,
    /// Whether it sits at the right end.
    pub right: bool,
    pub clickable: bool,
    /// Drawn in the accent colour, as changes waiting for review are.
    pub accent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteView {
    /// A plugin picker's title, or none for commands and files.
    pub title: Option<String>,
    pub query: String,
    /// What it lists: "Commands", "Files", "Go to Line" or "Symbols".
    pub kind: &'static str,
    /// Each item's label and its key, if it has one.
    pub items: Vec<(String, Option<String>)>,
    pub selected: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastView {
    pub text: String,
    pub error: bool,
    pub buttons: Vec<String>,
}

impl Editor {
    /// The activity rail's items: the built-in views, then plugin panels.
    pub(crate) fn rail_items(&self) -> Vec<(RailItem, Action)> {
        use super::views::{CHANGES, EXPLORER, OUTLINE, PLUGINS, SEARCH};
        let left = self
            .bar(Side::Left)
            .visible
            .then(|| self.active_panel_name(Side::Left))
            .flatten();
        let right = self
            .bar(Side::Right)
            .visible
            .then(|| self.active_panel_name(Side::Right))
            .flatten();
        let mut items = Vec::new();
        for (view, glyph, label, id) in [
            (EXPLORER, Icon::Explorer, "Explorer", "explorer"),
            (SEARCH, Icon::Search, "Search", "search"),
            (
                CHANGES,
                Icon::SourceControl,
                "Source Control",
                "source-control",
            ),
            (OUTLINE, Icon::Outline, "Outline", "outline"),
            (PLUGINS, Icon::Plugin, "Plugins", "plugins"),
        ] {
            items.push((
                RailItem {
                    id,
                    glyph: icon(glyph, self.nerd),
                    label: label.into(),
                    active: left.as_deref() == Some(view),
                    badge: if view == CHANGES {
                        self.git.files.len()
                    } else {
                        0
                    },
                },
                Action::View(view),
            ));
        }
        for name in self.panel_names(Side::Right) {
            // Unchanged, not in the side bar's capitals.
            let label = self.panel_title(Side::Right, &name).0;
            items.push((
                RailItem {
                    id: "plugin",
                    glyph: icon(Icon::Plugin, self.nerd),
                    label,
                    active: right.as_deref() == Some(name.as_str()),
                    badge: 0,
                },
                Action::Plugin(name),
            ));
        }
        items
    }

    fn side_view(&self, side: Side) -> Option<SideView> {
        if !self.bar(side).visible {
            return None;
        }
        let (name, panel) = self.active(side)?;
        let mut next = 0;
        let targets = panel
            .nodes
            .iter()
            .map(|node| {
                let count = target_count(node);
                let first = (count > 0).then_some(next);
                next += count;
                first
            })
            .collect();
        let inputs = panel
            .targets()
            .into_iter()
            .filter_map(|t| match t {
                Target::Input(id) => {
                    let text = self
                        .inputs
                        .get(&(name.to_string(), id.clone()))
                        .cloned()
                        .unwrap_or_default();
                    Some((id, text))
                }
                Target::Button(_) => None,
            })
            .collect();
        let active = name.to_string();
        let panels = self
            .panel_names(side)
            .into_iter()
            .map(|n| {
                let title = self.panel_title(Side::Right, &n).0;
                let on = n == active;
                (n, title, on)
            })
            .collect();
        Some(SideView {
            title: panel.title.clone(),
            panels,
            nodes: panel.nodes.clone(),
            targets,
            selected: self.selected_index(side),
            focused: self.focused_side() == Some(side),
            inputs,
        })
    }

    pub fn window_view(&self) -> WindowView {
        let groups = self.group_strips();
        let tabs = self.tab_views();
        let crumbs = if self.shows_crumbs() {
            self.crumbs().into_iter().map(|(text, _)| text).collect()
        } else {
            Vec::new()
        };
        let status = self
            .status_items()
            .into_iter()
            .map(|item| StatusView {
                text: item.text.trim().to_string(),
                right: item.right,
                clickable: item.action.is_some(),
                accent: item.style == crate::style::Style::StatusAccent,
            })
            .collect();
        let problem = self.cursor_mark().map(|m| {
            (
                m.message.lines().next().unwrap_or_default().to_string(),
                m.severity == crate::lang::Severity::Error,
            )
        });
        let palette = self.palette.as_ref().map(|p| PaletteView {
            title: p.title.clone(),
            query: p.query.clone(),
            kind: match p.source {
                Source::Commands => "Commands",
                Source::Files => "Files",
                Source::Line => "Go to Line",
                Source::Symbols => "Symbols",
                Source::Picker { .. } => "Pick",
                Source::Themes { .. } => "Themes",
            },
            items: self
                .palette_items()
                .into_iter()
                .map(|(id, label)| {
                    let key = matches!(p.source, Source::Commands)
                        .then(|| self.key_hint(&id))
                        .flatten();
                    (label, key)
                })
                .collect(),
            selected: p.selected,
        });
        let toast = self.message.as_ref().map(|m| ToastView {
            text: m.text.clone(),
            error: m.error,
            buttons: m.actions.iter().map(|(label, _)| label.clone()).collect(),
        });
        WindowView {
            project: self.project.clone(),
            branch: self.git.branch.clone(),
            rail: self
                .rail_items()
                .into_iter()
                .map(|(item, _)| item)
                .collect(),
            left: self.side_view(Side::Left),
            right: self.side_view(Side::Right),
            bottom: self.side_view(Side::Bottom),
            shown: [
                self.bar(Side::Left).visible,
                self.bar(Side::Bottom).visible,
                self.bar(Side::Right).visible,
            ],
            tabs,
            crumbs,
            status,
            problem,
            palette,
            toast,
            context: self.context_view(),
            popup: self.popup_view(),
            signature: self.signature_view(),
            menu: self.menu_view(),
            find: self.find_view(),
            groups,
            focus: self.focus,
            page: self.doc().page.as_ref().map(|(name, _)| name.clone()),
            minimap: self.minimap,
            dialog: self.dialog_view(),
            terminal: self.terminal_view(),
        }
    }

    pub(crate) fn ui(&mut self, event: UiEvent, fx: &mut Vec<Effect>) {
        match event {
            UiEvent::ContextItem(i) => self.context_item(i, fx),
            UiEvent::Hover(button) => self.hover_button(button, fx),
            UiEvent::Action(i) => self.action_clicked(i, fx),
            UiEvent::CloseContext => self.close_context(),
            UiEvent::MenuTitle(i) => self.menu_title(i),
            UiEvent::MenuItem(i) => self.menu_item(i, fx),
            UiEvent::CloseMenu => self.close_menu(),
            UiEvent::Find(what) => self.find_ui(what),
            UiEvent::Terminal(id) => self.focus_terminal(id),
            UiEvent::TerminalKill(id) => self.kill_terminal(id, fx),
            UiEvent::GroupTab {
                group,
                index,
                clicks,
            } => self.group_tab(group, index, clicks),
            UiEvent::CloseGroupTab { group, index } => {
                self.group_tab(group, index, 1);
                self.close(false);
            }
            UiEvent::GroupCrumb { group, index } => {
                if group < self.groups.len() && group != self.group {
                    self.focus_group(group);
                }
                if let Some((_, Some(line))) = self.crumbs().into_iter().nth(index) {
                    self.go_to_block(line);
                }
            }
            UiEvent::DropTab { group, at } => self.drop_tab_at(group, at),
            UiEvent::SplitTab { group, edge } => self.split_to(group, edge, true),
            UiEvent::DropEntry {
                side,
                index,
                group,
                edge,
            } => self.drop_entry(side, index, group, edge, fx),
            UiEvent::DialogButton(i) => self.answer(i, fx),
            UiEvent::GroupSizes(sizes) => {
                for (group, (w, h)) in self.groups.iter_mut().zip(sizes) {
                    group.width = w.max(0.05);
                    group.height = h.max(0.05);
                }
            }
            UiEvent::TabMenu => self.open_tab_menu(),
            UiEvent::TargetMenu { side, index } => {
                if self.active_panel_name(side).as_deref() == Some(super::views::EXPLORER) {
                    let row = match self.panel_targets(side).into_iter().nth(index) {
                        Some(crate::panel::Target::Button(id))
                            if id.starts_with("d:") || id.starts_with("f:") =>
                        {
                            Some(id)
                        }
                        _ => None,
                    };
                    self.open_explorer_menu(row);
                }
            }
            UiEvent::SideMenu(side) => {
                if self.active_panel_name(side).as_deref() == Some(super::views::EXPLORER) {
                    self.open_explorer_menu(None);
                }
            }
            UiEvent::Rail(i) => {
                if let Some((_, action)) = self.rail_items().into_iter().nth(i) {
                    self.act(action, fx);
                }
            }
            UiEvent::Tab { index, clicks } => {
                if let Some(doc) = self.pane_tabs().get(index).copied() {
                    self.current = doc;
                    self.focus = Focus::Editor;
                    if clicks >= 2 {
                        self.keep_open();
                    }
                }
            }
            UiEvent::CloseTab(index) => {
                if let Some(doc) = self.pane_tabs().get(index).copied() {
                    self.current = doc;
                    self.focus = Focus::Editor;
                    self.close(false);
                }
            }
            UiEvent::Crumb(i) => {
                if let Some((_, Some(line))) = self.crumbs().into_iter().nth(i) {
                    self.go_to_block(line);
                }
            }
            UiEvent::Status(i) => {
                if let Some(action) = self
                    .status_items()
                    .into_iter()
                    .nth(i)
                    .and_then(|s| s.action)
                {
                    self.act(action, fx);
                }
            }
            UiEvent::Target {
                side,
                index,
                clicks,
            } => {
                self.clicks = clicks.max(1);
                self.click_target(side, index, fx)
            }
            UiEvent::Check { side, index } => {
                self.click_target(side, index, fx);
                if side == Side::Bottom && self.reviewing_rename() {
                    self.toggle_row();
                }
            }
            UiEvent::Press(id) => self.view_click(&id, fx),
            UiEvent::PanelTitle { side, name } => {
                if side == Side::Bottom && name == super::terminal::TERMINAL {
                    self.run("terminal.toggle", fx);
                } else if side == Side::Bottom {
                    self.show_bottom(&name);
                } else {
                    self.bar_mut(side).active = Some(name);
                    self.focus_panel(side);
                }
            }
            UiEvent::PaletteRow(i) => {
                if let Some(p) = &mut self.palette {
                    p.selected = i;
                    self.palette_key(crate::key::Key::plain(crate::key::KeyCode::Enter), fx);
                }
            }
            UiEvent::ClosePalette => {
                self.close_palette();
            }
            UiEvent::ToastButton(i) => {
                if let Some(command) = self
                    .message
                    .as_ref()
                    .and_then(|m| m.actions.get(i))
                    .map(|(_, c)| c.clone())
                {
                    self.message = None;
                    self.run(&command, fx);
                }
            }
        }
    }
}

/// How many focusable elements a node has, in the order
/// [`crate::panel::Panel::targets`] lists them.
fn target_count(node: &Node) -> usize {
    match node {
        Node::Button(_) | Node::Item { .. } | Node::Input { .. } => 1,
        Node::Buttons(bs) => bs.len(),
        Node::Entry(e) => 1 + e.action.is_some() as usize,
        _ => 0,
    }
}
