//! Call and type hierarchies: the functions that call the one at the
//! cursor or that it calls, and a type's bases or the types derived from
//! it, a level at a time, for the editor's Hierarchy view.

use std::collections::BTreeMap;

use merlin_plugin::{HierarchyItem, HierarchyMode, Location, LogLevel, host};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::Lsp;
use crate::protocol::{path_of, position, symbol_kind, uri};
use crate::server::Request;

/// The items the editor shows, by the number it knows each by, and the
/// root and way the tree goes now.
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Hierarchy {
    next: u32,
    items: BTreeMap<u32, Value>,
    root: Option<u32>,
    /// The server that sent them.
    program: String,
    /// Which way the tree goes now, as the editor's commands name it.
    way: String,
}

/// The LSP name of a mode, as the editor's commands say it.
fn mode_named(name: &str) -> Option<HierarchyMode> {
    Some(match name {
        "callers" => HierarchyMode::Callers,
        "callees" => HierarchyMode::Callees,
        "bases" => HierarchyMode::Bases,
        "derived" => HierarchyMode::Derived,
        _ => return None,
    })
}

fn calls(mode: HierarchyMode) -> bool {
    matches!(mode, HierarchyMode::Callers | HierarchyMode::Callees)
}

impl Lsp {
    /// `call-hierarchy` and `type-hierarchy`: the function or type at the
    /// cursor, then its callers or bases.
    pub(crate) fn ask_hierarchy(&mut self, mode: HierarchyMode) {
        let Some((path, spec, language)) = self.active() else {
            return;
        };
        let Ok(cursor) = host::active_cursor() else {
            return;
        };
        self.catch_up(&path, spec, language);
        let at = json!({
            "textDocument": {"uri": uri(&path)},
            "position": {"line": cursor.line, "character": cursor.column},
        });
        self.hierarchy.program = spec.program.to_string();
        self.prepare(at, mode);
    }

    fn prepare(&mut self, at: Value, mode: HierarchyMode) {
        let program = self.hierarchy.program.clone();
        let Some(server) = self.server(&program) else {
            return;
        };
        let method = if calls(mode) {
            "textDocument/prepareCallHierarchy"
        } else {
            "textDocument/prepareTypeHierarchy"
        };
        server.request(method, at, Request::HierarchyPrepare { mode });
    }

    /// `hierarchy-mode:<mode>`: the other way from the same root, or the
    /// other kind from where the root is.
    pub(crate) fn hierarchy_mode(&mut self, name: &str) {
        let (Some(mode), Some(root)) = (
            mode_named(name),
            self.hierarchy
                .root
                .and_then(|r| self.hierarchy.items.get(&r).cloned()),
        ) else {
            return;
        };
        if calls(mode) == self.root_is_calls() {
            let id = self.hierarchy.root.unwrap_or_default();
            self.level(id, mode, true);
        } else {
            // Calls from a type, or types from a function: prepared again
            // where the root is.
            let at = json!({
                "textDocument": {"uri": root["uri"]},
                "position": root["selectionRange"]["start"],
            });
            self.prepare(at, mode);
        }
    }

    /// Whether the root came from a call hierarchy: its item is kept with
    /// a mark.
    fn root_is_calls(&self) -> bool {
        self.hierarchy
            .root
            .and_then(|r| self.hierarchy.items.get(&r))
            .is_some_and(|i| i["merlinCalls"].as_bool() == Some(true))
    }

    /// `hierarchy-expand:<id>`: the level under an item, the way the tree
    /// goes now.
    pub(crate) fn hierarchy_expand(&mut self, id: u32, mode: HierarchyMode) {
        self.level(id, mode, false);
    }

    /// Asks for the level under item `id`: shown as the tree, or as one
    /// item's children.
    fn level(&mut self, id: u32, mode: HierarchyMode, show: bool) {
        let Some(mut item) = self.hierarchy.items.get(&id).cloned() else {
            return;
        };
        // Sent back as the server sent it.
        if let Some(map) = item.as_object_mut() {
            map.remove("merlinCalls");
        }
        if show {
            self.hierarchy.way = match mode {
                HierarchyMode::Callers => "callers",
                HierarchyMode::Callees => "callees",
                HierarchyMode::Bases => "bases",
                HierarchyMode::Derived => "derived",
            }
            .into();
        }
        let method = match mode {
            HierarchyMode::Callers => "callHierarchy/incomingCalls",
            HierarchyMode::Callees => "callHierarchy/outgoingCalls",
            HierarchyMode::Bases => "typeHierarchy/supertypes",
            HierarchyMode::Derived => "typeHierarchy/subtypes",
        };
        let program = self.hierarchy.program.clone();
        if let Some(server) = self.server(&program) {
            server.request(
                method,
                json!({"item": item}),
                Request::HierarchyLevel {
                    parent: id,
                    mode,
                    show,
                },
            );
        }
    }

    /// Keeps an item the server sent, under a new number.
    fn keep(&mut self, mut item: Value, calls: bool) -> u32 {
        self.hierarchy.next += 1;
        let id = self.hierarchy.next;
        item["merlinCalls"] = json!(calls);
        self.hierarchy.items.insert(id, item);
        id
    }

    /// The editor's form of an item: where it is, or where it's called.
    fn item(&self, id: u32, at: Option<&Value>, calls: u32) -> Option<HierarchyItem> {
        let item = self.hierarchy.items.get(&id)?;
        let range = at.unwrap_or(&item["selectionRange"]);
        Some(HierarchyItem {
            id,
            name: item["name"].as_str()?.to_string(),
            detail: item["detail"].as_str().unwrap_or_default().to_string(),
            kind: symbol_kind(item["kind"].as_u64().unwrap_or(0)).into(),
            location: Location {
                path: path_of(item["uri"].as_str()?)?,
                start: position(&range["start"]),
                end: position(&range["end"]),
            },
            calls,
        })
    }

    /// The server's items for `prepare…`: the first is the root, and its
    /// first level is asked for.
    pub(crate) fn hierarchy_prepared(&mut self, result: &Value, mode: HierarchyMode) {
        let Some(first) = result.as_array().and_then(|a| a.first()).cloned() else {
            let what = if calls(mode) { "function" } else { "type" };
            return host::notice(LogLevel::Info, &format!("No {what} at the cursor"), &[]);
        };
        self.hierarchy.items.clear();
        let id = self.keep(first, calls(mode));
        self.hierarchy.root = Some(id);
        self.level(id, mode, true);
    }

    /// A level's answer: incoming and outgoing calls, or types.
    pub(crate) fn hierarchy_level(
        &mut self,
        result: &Value,
        parent: u32,
        mode: HierarchyMode,
        show: bool,
    ) {
        let mut children = Vec::new();
        for entry in result.as_array().into_iter().flatten() {
            let (item, ranges) = match mode {
                HierarchyMode::Callers => (entry["from"].clone(), entry["fromRanges"].as_array()),
                HierarchyMode::Callees => (entry["to"].clone(), entry["fromRanges"].as_array()),
                _ => (entry.clone(), None),
            };
            if item.is_null() {
                continue;
            }
            let count = ranges.map_or(0, |r| r.len() as u32);
            // A caller shows where it calls; a callee where it is, as the
            // calls are in the caller's file.
            let at = match mode {
                HierarchyMode::Callers => ranges.and_then(|r| r.first()).cloned(),
                _ => None,
            };
            let id = self.keep(item, calls(mode));
            if let Some(child) = self.item(id, at.as_ref(), count) {
                children.push(child);
            }
        }
        if show {
            let Some(root) = self.item(parent, None, 0) else {
                return;
            };
            host::show_hierarchy(mode, &root, &children);
        } else {
            host::hierarchy_children(parent, &children);
        }
    }

    /// The editor's commands for hierarchies, if `id` is one.
    pub(crate) fn hierarchy_command(&mut self, id: &str) -> bool {
        match id {
            "call-hierarchy" => self.ask_hierarchy(HierarchyMode::Callers),
            "type-hierarchy" => self.ask_hierarchy(HierarchyMode::Bases),
            _ => {
                if let Some(mode) = id.strip_prefix("hierarchy-mode:") {
                    self.hierarchy_mode(mode);
                } else if let Some((item, mode)) = id
                    .strip_prefix("hierarchy-expand:")
                    .and_then(|i| i.parse::<u32>().ok())
                    .map(|i| (i, self.expand_mode()))
                {
                    self.hierarchy_expand(item, mode);
                } else {
                    return false;
                }
            }
        }
        true
    }

    /// The way the tree goes now, as last shown.
    fn expand_mode(&self) -> HierarchyMode {
        mode_named(&self.hierarchy.way).unwrap_or(HierarchyMode::Callers)
    }
}
