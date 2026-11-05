//! What's open, to bring back next time in the same folder, as VS Code
//! reopens a workspace: the editor groups, their tabs, and where each was
//! in each file.
//!
//! The host saves [`Editor::workspace`] as text, and to restore it opens the
//! files with [`Event::Opened`](super::Event::Opened) and then sends
//! [`Event::Restore`](super::Event::Restore).

use std::path::PathBuf;

use super::Editor;
use super::groups::{Group, View};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspace {
    pub groups: Vec<GroupState>,
    /// The focused group.
    pub focused: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GroupState {
    /// Under the group before it, rather than in a column of its own.
    pub below: bool,
    pub tabs: Vec<Tab>,
    /// The tab shown.
    pub active: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub path: PathBuf,
    /// The cursor's line and column, in characters.
    pub line: usize,
    pub column: usize,
    /// The first line in view.
    pub scroll: usize,
    /// Pinned at the start of its group's tabs.
    pub pinned: bool,
}

impl Workspace {
    /// One line per group and per tab:
    ///
    /// ```text
    /// focus 1
    /// group right 0
    /// tab 12:4 3 src/main.rs
    /// pin 0:0 0 Cargo.toml
    /// group down 1
    /// ```
    pub fn to_text(&self) -> String {
        let mut out = format!("focus {}\n", self.focused);
        for group in &self.groups {
            let side = if group.below { "down" } else { "right" };
            out += &format!("group {side} {}\n", group.active);
            for tab in &group.tabs {
                out += &format!(
                    "{} {}:{} {} {}\n",
                    if tab.pinned { "pin" } else { "tab" },
                    tab.line,
                    tab.column,
                    tab.scroll,
                    tab.path.display()
                );
            }
        }
        out
    }

    /// Reads [`Workspace::to_text`]'s format, skipping lines it can't.
    pub fn parse(text: &str) -> Self {
        let mut ws = Workspace::default();
        for line in text.lines() {
            let mut words = line.splitn(4, ' ');
            match (words.next(), words.next(), words.next(), words.next()) {
                (Some("focus"), Some(n), None, None) => ws.focused = n.parse().unwrap_or(0),
                (Some("group"), Some(side), Some(active), None) => ws.groups.push(GroupState {
                    below: side == "down",
                    tabs: Vec::new(),
                    active: active.parse().unwrap_or(0),
                }),
                (Some(kind @ ("tab" | "pin")), Some(at), Some(scroll), Some(path)) => {
                    let Some(group) = ws.groups.last_mut() else {
                        continue;
                    };
                    let (line, column) = at.split_once(':').unwrap_or((at, "0"));
                    group.tabs.push(Tab {
                        path: PathBuf::from(path),
                        line: line.parse().unwrap_or(0),
                        column: column.parse().unwrap_or(0),
                        scroll: scroll.parse().unwrap_or(0),
                        pinned: kind == "pin",
                    });
                }
                _ => {}
            }
        }
        ws
    }

    /// Every file it has open, once each.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = Vec::new();
        for tab in self.groups.iter().flat_map(|g| &g.tabs) {
            if !files.contains(&tab.path) {
                files.push(tab.path.clone());
            }
        }
        files
    }
}

impl Editor {
    /// The groups and their files, as they are now. Files without a name
    /// and reviews are left out.
    pub fn workspace(&self) -> Workspace {
        let mut groups = Vec::new();
        for (g, group) in self.groups.iter().enumerate() {
            let mut state = GroupState {
                below: group.below,
                ..GroupState::default()
            };
            for &id in &group.tabs {
                let Some(i) = self.index_of(id) else { continue };
                let doc = &self.docs[i];
                let Some(path) = doc.buffer.path.clone() else {
                    continue;
                };
                if doc.review.is_some() {
                    continue;
                }
                let view = match group.saved.get(&id) {
                    Some(view) if g != self.group || id != self.doc().id => view.clone(),
                    _ => View::of(doc),
                }
                .clamped(doc);
                let (line, column) = doc.line_col(view.cursor);
                if id == group.active {
                    state.active = state.tabs.len();
                }
                state.tabs.push(Tab {
                    path,
                    line,
                    column,
                    scroll: view.scroll,
                    pinned: group.pinned.contains(&id),
                });
            }
            if state.tabs.is_empty() {
                continue;
            }
            if groups.is_empty() {
                state.below = false;
            }
            groups.push((g, state));
        }
        let focused = groups
            .iter()
            .position(|(g, _)| *g == self.group)
            .unwrap_or(0);
        Workspace {
            groups: groups.into_iter().map(|(_, s)| s).collect(),
            focused,
        }
    }

    /// Lays the groups out as `ws` has them, with the files it names that
    /// are open by now.
    pub(crate) fn restore(&mut self, ws: Workspace) {
        let mut groups: Vec<Group> = Vec::new();
        // Each group's view of each of its files.
        let mut views: Vec<Vec<(u64, View)>> = Vec::new();
        let mut focused = 0;
        for (g, state) in ws.groups.into_iter().enumerate() {
            let mut group = Group {
                below: state.below && !groups.is_empty(),
                ..Group::default()
            };
            let mut these = Vec::new();
            let mut active = None;
            for (t, tab) in state.tabs.iter().enumerate() {
                let Some(i) = self
                    .docs
                    .iter()
                    .position(|d| d.buffer.path.as_ref() == Some(&tab.path))
                else {
                    continue;
                };
                let doc = &self.docs[i];
                let line = tab.line.min(doc.last_line());
                let column = tab.column.min(doc.buffer.line_len(line));
                let view = View {
                    cursor: doc.line_start(line) + column,
                    scroll: tab.scroll.min(doc.last_line()),
                    ..View::default()
                };
                if !group.tabs.contains(&doc.id) {
                    group.tabs.push(doc.id);
                    these.push((doc.id, view));
                }
                if tab.pinned {
                    group.pinned.insert(doc.id);
                }
                if t == state.active {
                    active = Some(doc.id);
                }
            }
            let Some(&first) = group.tabs.first() else {
                continue;
            };
            group.active = active.unwrap_or(first);
            if g == ws.focused {
                focused = groups.len();
            }
            groups.push(group);
            views.push(these);
        }
        if groups.is_empty() {
            return;
        }
        // Files the workspace doesn't name, such as one opened on the
        // command line, stay in the focused group.
        for doc in &self.docs {
            if !doc.is_blank() && !groups.iter().any(|g| g.tabs.contains(&doc.id)) {
                groups[focused].tabs.push(doc.id);
            }
        }
        // The focused group's views go into the documents; other groups'
        // into their own, unless no group nearer has the file.
        let mut placed: Vec<u64> = Vec::new();
        let order = std::iter::once(focused).chain((0..groups.len()).filter(|&g| g != focused));
        for g in order {
            for (id, view) in views[g].clone() {
                let Some(i) = self.index_of(id) else { continue };
                if placed.contains(&id) {
                    groups[g].saved.insert(id, view);
                } else {
                    let doc = &mut self.docs[i];
                    doc.cursor = view.cursor;
                    doc.anchor = None;
                    doc.goal = None;
                    doc.scroll = view.scroll;
                    doc.hscroll = 0;
                    placed.push(id);
                }
            }
        }
        // A blank document the files replaced goes.
        self.docs
            .retain(|d| !d.is_blank() || groups.iter().any(|g| g.tabs.contains(&d.id)));
        let active = groups[focused].active;
        self.groups = groups;
        self.group = focused;
        self.current = self.index_of(active).unwrap_or(0);
        self.shown = (focused, active);
    }
}
