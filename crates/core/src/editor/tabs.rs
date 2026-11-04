//! Tabs as a window shows them: each editor group with its own strip when
//! there are several, tabs dragged to a new place or another group, and
//! the menu on a tab, as VS Code has it.

use super::context::ContextMenu;
use super::{Editor, Effect, Focus, TabView};

/// A tab's right-click menu.
pub(crate) const TAB_ITEMS: &[(&str, &str)] = &[
    ("file.close", "Close"),
    ("file.close_others", "Close Others"),
    ("file.close_right", "Close to the Right"),
    ("file.close_all", "Close All"),
    ("-", ""),
    ("file.copy_path", "Copy Path"),
    ("-", ""),
    ("view.split", "Split Right"),
    ("view.split_down", "Split Down"),
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

    /// The right-click menu on the focused tab.
    pub(crate) fn open_tab_menu(&mut self) {
        self.popup = None;
        self.context = Some(ContextMenu {
            items: TAB_ITEMS.iter().map(|(id, _)| *id).collect(),
            selected: None,
        });
        self.focus = Focus::Menu;
    }

    /// Closes the group's tabs that `keep` says to close, saved ones only;
    /// a file with changes stays open, and says so.
    fn close_tabs(&mut self, close: impl Fn(usize, usize) -> bool) {
        let keep = self.doc().id;
        let ids = self.groups[self.group].tabs.clone();
        let here = ids.iter().position(|&t| t == keep).unwrap_or(0);
        let mut unsaved = 0;
        for (k, id) in ids.into_iter().enumerate() {
            if !close(k, here) {
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
        if !self.doc().buffer.is_dirty() {
            self.close(false);
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
