//! The editor in a window that draws its own frame. The window lays out the
//! title bar, activity rail, side bars, tabs, breadcrumbs, status bar,
//! palette and notices in pixels, from the [`WindowView`] here, and says what
//! was clicked with a [`UiEvent`]. The editor's screen is then only the
//! text: every editor group, with its gutter, scrollbar and popups.

use super::chrome::Action;
use super::{Editor, Effect, Focus, Side, Source};
use crate::icons::{Icon, icon};
use crate::panel::{Node, Target};
use crate::screen::Hue;

/// Something clicked in a window's frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiEvent {
    /// An activity rail item, by its index in [`WindowView::rail`].
    Rail(usize),
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
    },
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
    /// Where keys go.
    pub focus: Focus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RailItem {
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
    /// Has the window draw the frame, leaving the screen to the text.
    pub fn set_windowed(&mut self, on: bool) {
        self.windowed = on;
    }

    pub fn windowed(&self) -> bool {
        self.windowed
    }

    /// The activity rail's items: the built-in views, then plugin panels.
    pub(crate) fn rail_items(&self) -> Vec<(RailItem, Action)> {
        use super::views::{CHANGES, EXPLORER, OUTLINE, SEARCH};
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
        for (view, glyph, label) in [
            (EXPLORER, Icon::Explorer, "Explorer"),
            (SEARCH, Icon::Search, "Search"),
            (CHANGES, Icon::SourceControl, "Source Control"),
            (OUTLINE, Icon::Outline, "Outline"),
        ] {
            items.push((
                RailItem {
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
        let tabs = self
            .pane_tabs()
            .into_iter()
            .map(|i| {
                let doc = &self.docs[i];
                TabView {
                    name: doc.name(),
                    icon: self.tab_icon(i),
                    active: i == self.pane_doc(),
                    dirty: doc.buffer.is_dirty(),
                    preview: self.preview == Some(doc.id),
                }
            })
            .collect();
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
                accent: item.style == crate::screen::Style::StatusAccent,
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
            focus: self.focus,
        }
    }

    pub(crate) fn ui(&mut self, event: UiEvent, armed: Option<&'static str>, fx: &mut Vec<Effect>) {
        match event {
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
                    self.close(armed == Some("file.close"));
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
            UiEvent::Target { side, index } => self.click_target(side, index, fx),
            UiEvent::PanelTitle { side, name } => {
                if side == Side::Bottom {
                    self.show_bottom(&name);
                } else {
                    self.bar_mut(side).active = Some(name);
                    self.focus_panel(side);
                }
            }
            UiEvent::PaletteRow(i) => {
                if let Some(p) = &mut self.palette {
                    p.selected = i;
                    self.palette_key(crate::key::Key::plain(crate::key::KeyCode::Enter), None, fx);
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
                    self.run(&command, armed, fx);
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
