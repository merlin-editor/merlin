//! The Hierarchy view: who calls a function and what it calls, or a
//! type's bases and the types derived from it, as a tree opened a level at
//! a time, from the language server's call and type hierarchies.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::{Editor, Effect};
use crate::lang::Location;
use crate::panel::{Chip, Entry, Node, Tone};

pub(crate) const HIERARCHY: &str = "merlin.hierarchy";

/// Which way the tree goes from its root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HierarchyMode {
    Callers,
    Callees,
    Bases,
    Derived,
}

impl HierarchyMode {
    fn id(self) -> &'static str {
        match self {
            Self::Callers => "callers",
            Self::Callees => "callees",
            Self::Bases => "bases",
            Self::Derived => "derived",
        }
    }

    fn calls(self) -> bool {
        matches!(self, Self::Callers | Self::Callees)
    }

    /// What an item with nothing under it says.
    fn none(self) -> &'static str {
        match self {
            Self::Callers => "no callers",
            Self::Callees => "calls nothing",
            Self::Bases => "no bases",
            Self::Derived => "nothing derives from it",
        }
    }
}

/// A function or type in the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HierarchyItem {
    /// The plugin's own, to expand it by.
    pub id: u32,
    pub name: String,
    pub detail: String,
    pub kind: String,
    pub location: Location,
    pub calls: u32,
}

pub(crate) struct Hierarchy {
    plugin: String,
    mode: HierarchyMode,
    root: HierarchyItem,
    /// Each item's level under it, once it came.
    children: HashMap<u32, Vec<HierarchyItem>>,
    open: HashSet<u32>,
}

impl Editor {
    /// Asks the language server for the hierarchy at the cursor: callers
    /// of a function, or a type's bases.
    pub(crate) fn ask_hierarchy(&mut self, calls: bool, fx: &mut Vec<Effect>) {
        let command = if calls {
            "call-hierarchy"
        } else {
            "type-hierarchy"
        };
        match self.lsp_plugin(command) {
            Some(plugin) => fx.push(Effect::PluginCommand {
                plugin,
                command: command.into(),
            }),
            None => self.notify(format!(
                "No language server here shows {}",
                if calls { "calls" } else { "types" }
            )),
        }
    }

    pub(crate) fn hierarchy_came(
        &mut self,
        plugin: String,
        mode: HierarchyMode,
        root: HierarchyItem,
        children: Vec<HierarchyItem>,
        fx: &mut Vec<Effect>,
    ) {
        let mut known = HashMap::new();
        known.insert(root.id, children);
        self.hierarchy = Some(Hierarchy {
            plugin,
            mode,
            open: HashSet::from([root.id]),
            root,
            children: known,
        });
        self.show_view(HIERARCHY, fx);
    }

    pub(crate) fn hierarchy_children(
        &mut self,
        plugin: &str,
        parent: u32,
        children: Vec<HierarchyItem>,
    ) {
        if let Some(h) = self.hierarchy.as_mut().filter(|h| h.plugin == plugin) {
            h.children.insert(parent, children);
        }
    }

    /// The view's rows: which kind and way, the root, and the tree.
    pub(crate) fn hierarchy_nodes(&self) -> Vec<Node> {
        let Some(h) = &self.hierarchy else {
            return vec![Node::Styled(
                "Right-click a function for Show Call Hierarchy, or a class for Show Type Hierarchy."
                    .into(),
                Tone::Muted,
            )];
        };
        let chip = |id: &str, label: &str, on: bool| Chip {
            id: id.into(),
            label: label.into(),
            tone: if on { Tone::Strong } else { Tone::Muted },
        };
        let calls = h.mode.calls();
        let ways = if calls {
            [("callers", "Callers"), ("callees", "Callees")]
        } else {
            [("bases", "Bases"), ("derived", "Derived")]
        };
        let mut nodes = vec![
            Node::Chips {
                chips: vec![
                    chip("hy:calls", "Calls", calls),
                    chip("hy:types", "Types", !calls),
                ],
                hint: String::new(),
            },
            Node::entry(self.hierarchy_entry(&h.root, 0, None, h.mode)),
            Node::Chips {
                chips: ways
                    .iter()
                    .map(|(id, label)| chip(&format!("hy:mode:{id}"), label, h.mode.id() == *id))
                    .collect(),
                hint: String::new(),
            },
            Node::Divider,
        ];
        self.hierarchy_level(h, h.root.id, 0, &mut nodes, &mut vec![h.root.id]);
        nodes
    }

    /// An item's level, and the levels under those opened, depth first.
    fn hierarchy_level(
        &self,
        h: &Hierarchy,
        parent: u32,
        depth: usize,
        nodes: &mut Vec<Node>,
        path: &mut Vec<u32>,
    ) {
        let Some(children) = h.children.get(&parent) else {
            nodes.push(Node::Styled("…".into(), Tone::Muted));
            return;
        };
        if children.is_empty() {
            let mut e = Entry {
                id: String::new(),
                label: h.mode.none().into(),
                depth,
                tone: Tone::Muted,
                ..Default::default()
            };
            e.twisty = Some(' ');
            nodes.push(Node::entry(e));
            return;
        }
        for item in children {
            // A recursive call stops at its first repeat.
            let repeat = path.contains(&item.id);
            let open = h.open.contains(&item.id) && !repeat;
            nodes.push(Node::entry(self.hierarchy_entry(
                item,
                depth,
                Some(open),
                h.mode,
            )));
            if open {
                path.push(item.id);
                self.hierarchy_level(h, item.id, depth + 1, nodes, path);
                path.pop();
            }
        }
    }

    fn hierarchy_entry(
        &self,
        item: &HierarchyItem,
        depth: usize,
        open: Option<bool>,
        mode: HierarchyMode,
    ) -> Entry {
        let file = item
            .location
            .path
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let at = format!("{file}:{}", item.location.start.0 + 1);
        Entry {
            id: format!("hy:go:{}", item.id),
            label: item.name.clone(),
            // clangd's detail can be the name again.
            detail: if item.detail.is_empty() || item.detail == item.name {
                at
            } else {
                format!("{}  {at}", item.detail)
            },
            depth,
            icon: Some(super::views::symbol_icon(&item.kind)),
            fold: open,
            badge: match item.calls {
                n if n > 1 && mode.calls() => format!("{n} calls"),
                _ => String::new(),
            },
            badge_tone: Tone::Muted,
            ..Default::default()
        }
    }

    fn hierarchy_item(&self, id: u32) -> Option<HierarchyItem> {
        let h = self.hierarchy.as_ref()?;
        std::iter::once(&h.root)
            .chain(h.children.values().flatten())
            .find(|i| i.id == id)
            .cloned()
    }

    /// A click in the view: a switch, or an item, which opens where it is.
    pub(crate) fn hierarchy_click(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let Some(rest) = id.strip_prefix("hy:") else {
            return false;
        };
        let Some(h) = &self.hierarchy else {
            return true;
        };
        let plugin = h.plugin.clone();
        let mode = match rest {
            "calls" if !h.mode.calls() => Some("callers"),
            "types" if h.mode.calls() => Some("bases"),
            _ => rest.strip_prefix("mode:"),
        };
        if let Some(mode) = mode {
            fx.push(Effect::PluginCommand {
                plugin,
                command: format!("hierarchy-mode:{mode}"),
            });
        } else if let Some(item) = rest
            .strip_prefix("go:")
            .and_then(|i| i.parse().ok())
            .and_then(|i| self.hierarchy_item(i))
        {
            let (line, column) = item.location.start;
            self.open_at(PathBuf::from(&item.location.path), line, column, fx);
        }
        true
    }

    /// An item's arrow: its level opened, asked for the first time, or
    /// closed.
    pub(crate) fn hierarchy_fold(&mut self, id: &str, fx: &mut Vec<Effect>) -> bool {
        let Some(item) = id
            .strip_prefix("hy:go:")
            .and_then(|i| i.parse::<u32>().ok())
        else {
            return false;
        };
        let Some(h) = self.hierarchy.as_mut() else {
            return true;
        };
        if !h.open.remove(&item) {
            h.open.insert(item);
            if !h.children.contains_key(&item) {
                fx.push(Effect::PluginCommand {
                    plugin: h.plugin.clone(),
                    command: format!("hierarchy-expand:{item}"),
                });
            }
        }
        true
    }
}
