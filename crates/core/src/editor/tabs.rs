//! Tabs as a window shows them: each editor group with its own strip when
//! there are several, tabs dragged to a new place or another group, and
//! the menu on a tab, as VS Code has it.

use super::context::ContextMenu;
use std::path::{Path, PathBuf};

use super::{Edge, Editor, Effect, Focus, Side, TabView};

/// A tab's right-click menu.
pub(crate) const TAB_ITEMS: &[(&str, &str)] = &[
    ("file.close", "Close"),
    ("file.close_others", "Close Others"),
    ("file.close_right", "Close to the Right"),
    ("file.close_saved", "Close Saved"),
    ("file.close_all", "Close All"),
    ("-", ""),
    ("file.copy_path", "Copy Path"),
    ("file.copy_relative_path", "Copy Relative Path"),
    ("file.reveal", "Reveal in Explorer View"),
    ("-", ""),
    ("file.keep", "Keep Open"),
    ("file.pin", "Pin"),
    ("file.unpin", "Unpin"),
    ("-", ""),
    ("view.split_up", "Split Up"),
    ("view.split_down", "Split Down"),
    ("view.split_left", "Split Left"),
    ("view.split", "Split Right"),
];

/// An editor group: its tabs and breadcrumbs, and where it goes. Groups
/// are in columns left to right; one `below` is under the group before it,
/// in the same column.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupView {
    pub tabs: Vec<TabView>,
    pub crumbs: Vec<String>,
    pub focused: bool,
    pub below: bool,
    /// Its share of the width, for the first group of a column, which
    /// speaks for the column; of its column's height, for every group.
    pub width: f32,
    pub height: f32,
    /// Its tab shown is in the merge editor: the code is the Result, under
    /// Incoming and Current.
    pub merging: bool,
    /// In the merge editor, the Result alone.
    pub merge_inline: bool,
    /// Two files compared: each side's name and its note, over its half.
    pub compare: Option<[(String, String); 2]>,
    /// Its file changed on disk while merlin was closed: the file's id and
    /// name, for the bar over its text.
    pub away: Option<(u64, String)>,
    /// The page its tab shows instead of text, such as Settings.
    pub page: Option<String>,
}

impl Editor {
    /// The tabs of the group being drawn.
    pub(crate) fn tab_views(&self) -> Vec<TabView> {
        self.pane_tabs()
            .into_iter()
            .map(|i| {
                let doc = &self.docs[i];
                TabView {
                    name: doc.name(),
                    icon: self.tab_icon(i),
                    active: i == self.pane_doc(),
                    dirty: doc.buffer.is_dirty(),
                    preview: self.preview == Some(doc.id),
                    pinned: self.groups[self.pane()].pinned.contains(&doc.id),
                    merging: self.merges.iter().any(|m| m.doc == doc.id),
                }
            })
            .collect()
    }

    /// Every group, for a window to lay out and draw.
    pub(crate) fn group_strips(&self) -> Vec<GroupView> {
        (0..self.groups.len())
            .map(|g| {
                self.as_group(g, || GroupView {
                    tabs: self.tab_views(),
                    crumbs: if self.breadcrumbs && self.shows_crumbs() {
                        self.crumbs().into_iter().map(|(t, _)| t).collect()
                    } else {
                        Vec::new()
                    },
                    focused: g == self.group,
                    below: self.groups[g].below,
                    width: self.groups[g].width,
                    height: self.groups[g].height,
                    merging: self.merging().is_some(),
                    merge_inline: self.merging().is_some() && self.merge_inline(),
                    compare: self.compare_titles().map(|(l, r)| [l, r]),
                    away: self.away_bar(),
                    page: self.docs[self.pane_doc()]
                        .page
                        .as_ref()
                        .map(|(n, _)| n.clone()),
                })
            })
            .collect()
    }

    /// A click on group `g`'s `index`th tab.
    pub(crate) fn group_tab(&mut self, g: usize, index: usize, clicks: u8) {
        if g < self.groups.len() && g != self.group {
            self.focus_group(g);
        }
        if let Some(doc) = self.pane_tabs().get(index).copied() {
            self.current = doc;
            self.focus = Focus::Editor;
            if clicks >= 2 {
                self.keep_open();
            }
        }
    }

    /// The focused tab, dragged and dropped: before group `g`'s `at`th tab,
    /// or at its end.
    pub(crate) fn drop_tab_at(&mut self, g: usize, at: Option<usize>) {
        let id = self.doc().id;
        // Before itself, or just after, it stays where it is.
        let at = match at {
            Some(i) if g == self.group => {
                let tabs = &self.groups[g].tabs;
                let from = tabs.iter().position(|&t| t == id).unwrap_or(0);
                Some(if i > from { i - 1 } else { i })
            }
            other => other,
        };
        self.move_tab(id, g, at);
    }

    /// A file dragged from a side's view, its `index`th element, and
    /// dropped on group `g`: it opens there, kept, or on `edge` of it in a
    /// new group, once it's read.
    pub(crate) fn drop_entry(
        &mut self,
        side: Side,
        index: usize,
        g: usize,
        edge: Option<Edge>,
        fx: &mut Vec<Effect>,
    ) {
        let file = match self.panel_targets(side).into_iter().nth(index) {
            Some(crate::panel::Target::Button(id)) => match id.strip_prefix("f:") {
                Some(file) => PathBuf::from(file),
                None => return,
            },
            _ => return,
        };
        if g < self.groups.len() {
            self.focus_group(g);
        }
        if let Some(edge) = edge {
            self.opening_split = Some((file.clone(), edge));
        }
        self.open(file.clone(), fx);
        if self.doc().buffer.path.as_ref() == Some(&file) {
            self.keep_open();
            self.split_opened(&file);
        }
    }

    /// A file dropped on a group's side has arrived: it moves into a new
    /// group there.
    pub(crate) fn split_opened(&mut self, path: &Path) {
        let Some((file, edge)) = self.opening_split.take() else {
            return;
        };
        if file != path {
            self.opening_split = Some((file, edge));
            return;
        }
        self.sync_groups();
        self.split_to(self.group, edge, true);
    }

    /// The right-click menu on the focused tab.
    pub(crate) fn open_tab_menu(&mut self) {
        self.popup = None;
        let id = self.doc().id;
        let pinned = self.groups[self.group].pinned.contains(&id);
        let preview = self.preview == Some(id);
        // Only what applies to this tab.
        let items = TAB_ITEMS
            .iter()
            .map(|(id, _)| *id)
            .filter(|&item| match item {
                "file.keep" => preview,
                "file.pin" => !pinned,
                "file.unpin" => pinned,
                _ => true,
            })
            .collect();
        self.context = Some(ContextMenu {
            items,
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    /// Closes the group's tabs that `keep` says to close, saved ones only;
    /// a file with changes stays open, and says so.
    fn close_tabs(&mut self, close: impl Fn(usize, usize) -> bool) {
        let keep = self.doc().id;
        let ids = self.groups[self.group].tabs.clone();
        let pinned = self.groups[self.group].pinned.clone();
        let here = ids.iter().position(|&t| t == keep).unwrap_or(0);
        let mut unsaved = 0;
        for (k, id) in ids.into_iter().enumerate() {
            if !close(k, here) || pinned.contains(&id) {
                continue;
            }
            let Some(i) = self.index_of(id) else { continue };
            if self.docs[i].buffer.is_dirty() {
                unsaved += 1;
                continue;
            }
            self.current = i;
            self.close(false);
            self.sync_groups();
        }
        if let Some(i) = self.index_of(keep) {
            self.current = i;
        }
        self.sync_groups();
        if unsaved > 0 {
            let s = if unsaved == 1 { "" } else { "s" };
            self.notify(format!("{unsaved} file{s} with unsaved changes left open"));
        }
    }

    pub(crate) fn close_others(&mut self) {
        self.close_tabs(|k, here| k != here);
    }

    pub(crate) fn close_right(&mut self) {
        self.close_tabs(|k, here| k > here);
    }

    pub(crate) fn close_all(&mut self) {
        self.close_tabs(|_, _| true);
        let id = self.doc().id;
        if !self.doc().buffer.is_dirty() && !self.groups[self.group].pinned.contains(&id) {
            self.close(false);
        }
    }

    /// Closes the group's tabs without unsaved changes, the current one
    /// too.
    pub(crate) fn close_saved(&mut self) {
        let unsaved: Vec<u64> = self
            .docs
            .iter()
            .filter(|d| d.buffer.is_dirty())
            .map(|d| d.id)
            .collect();
        let tabs = self.groups[self.group].tabs.clone();
        self.close_tabs(|k, _| tabs.get(k).is_some_and(|id| !unsaved.contains(id)));
        let id = self.doc().id;
        if !self.doc().buffer.is_dirty() && !self.groups[self.group].pinned.contains(&id) {
            self.close(false);
            self.sync_groups();
        }
    }

    pub(crate) fn copy_relative_path(&mut self, fx: &mut Vec<Effect>) {
        match &self.doc().buffer.path {
            Some(path) => {
                let text = path
                    .strip_prefix(&self.root)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                self.clipboard = text.clone();
                fx.push(Effect::Copy(text));
            }
            None => self.notify("This file has no path yet".into()),
        }
    }

    pub(crate) fn copy_path(&mut self, fx: &mut Vec<Effect>) {
        match &self.doc().buffer.path {
            Some(path) => {
                let text = path.display().to_string();
                self.clipboard = text.clone();
                fx.push(Effect::Copy(text));
            }
            None => self.notify("This file has no path yet".into()),
        }
    }
}
