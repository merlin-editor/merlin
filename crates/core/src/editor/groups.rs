//! Editor groups, as VS Code splits the editor: in columns side by side,
//! each of which may be split down, and each group with its own tabs. A file open in two groups shares its text, but each group keeps
//! its own cursor and scroll.
//!
//! A document's own cursors, scroll and folds belong to the focused group.
//! Other groups keep theirs in `saved`, and are drawn from there, so each
//! group has its own extra cursors and its own folds, as VS Code has it.

use std::collections::{HashMap, HashSet};

use super::Editor;
use crate::buffer::Change;
use crate::doc::{Caret, Doc, Sub};

/// Where a group is in a document.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct View {
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub goal: Option<usize>,
    pub scroll: usize,
    pub sub: Sub,
    pub hscroll: usize,
    /// Cursors besides the primary one.
    pub extra: Vec<Caret>,
    /// Folded blocks, by where their first line starts.
    pub folds: Vec<usize>,
}

impl View {
    pub fn of(doc: &Doc) -> Self {
        Self {
            cursor: doc.cursor,
            anchor: doc.anchor,
            goal: doc.goal,
            scroll: doc.scroll,
            sub: doc.scroll_sub,
            hscroll: doc.hscroll,
            extra: doc.extra.clone(),
            folds: doc.folds.clone(),
        }
    }

    /// The view within a document's text, which another group may have
    /// shortened.
    pub fn clamped(self, doc: &Doc) -> Self {
        let len = doc.buffer.len();
        let rope = doc.buffer.rope();
        Self {
            cursor: self.cursor.min(len),
            anchor: self.anchor.map(|a| a.min(len)),
            scroll: self.scroll.min(doc.last_line()),
            extra: self
                .extra
                .into_iter()
                .filter(|c| c.cursor <= len && c.anchor.is_none_or(|a| a <= len))
                .collect(),
            // Only folds still at the start of a line.
            folds: self
                .folds
                .into_iter()
                .filter(|&f| f == 0 || (f <= len && rope.char(f - 1) == '\n'))
                .collect(),
            ..self
        }
    }

    /// Moves the cursor, selection and scroll along with edits made
    /// elsewhere, so the same text stays in view.
    pub fn follow(&mut self, changes: &[Change]) {
        for change in changes {
            self.cursor = change.map(self.cursor);
            self.anchor = self.anchor.map(|a| change.map(a));
            for caret in &mut self.extra {
                caret.cursor = change.map(caret.cursor);
                caret.anchor = caret.anchor.map(|a| change.map(a));
            }
            for fold in &mut self.folds {
                *fold = change.map(*fold);
            }
            let e = &change.edit;
            // Rows added or removed above the view shift it.
            if e.old_end.0 < self.scroll {
                self.scroll = (self.scroll + e.new_end.0).saturating_sub(e.old_end.0);
            } else if e.start.0 < self.scroll {
                self.scroll = e.start.0.max(self.scroll.min(e.new_end.0));
            }
        }
    }

    /// Puts the view back into a document.
    fn put(self, doc: &mut Doc) {
        let v = self.clamped(doc);
        doc.cursor = v.cursor;
        doc.anchor = v.anchor;
        doc.goal = v.goal;
        doc.scroll = v.scroll;
        doc.scroll_sub = v.sub;
        doc.hscroll = v.hscroll;
        doc.extra = v.extra;
        doc.extra_version = doc.buffer.version();
        doc.folds = v.folds;
    }

    /// The selected range, if it isn't empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.cursor).then(|| (anchor.min(self.cursor), anchor.max(self.cursor)))
    }
}

#[derive(Debug)]
pub(crate) struct Group {
    /// Document ids, in tab order.
    pub tabs: Vec<u64>,
    pub active: u64,
    /// Views of documents while another group has them.
    pub saved: HashMap<u64, View>,
    /// Under the group before it, in the same column, rather than in a
    /// column of its own.
    pub below: bool,
    /// Its share of the window: of the width, for the first group of a
    /// column, which speaks for the column; of its column's height, for
    /// every group.
    pub width: f32,
    pub height: f32,
    /// Tabs pinned at the start of the strip, which closing others skips.
    pub pinned: HashSet<u64>,
}

/// No tabs yet, with an even share of the room: a share of nothing would
/// leave it no room beside groups with theirs.
impl Default for Group {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            saved: HashMap::new(),
            below: false,
            width: 1.0,
            height: 1.0,
            pinned: HashSet::new(),
        }
    }
}

impl Group {
    pub fn new(id: u64) -> Self {
        Self {
            tabs: vec![id],
            active: id,
            ..Self::default()
        }
    }

    /// Puts the pinned tabs first, each lot in its own order.
    pub fn order_pinned(&mut self) {
        let pinned = &self.pinned;
        self.tabs.sort_by_key(|id| !pinned.contains(id));
    }
}

/// A side of a group, to split it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Up,
    Down,
}

/// A group being drawn, other than the focused one.
#[derive(Clone, Debug)]
pub(crate) struct Drawing {
    pub group: usize,
    pub doc: usize,
    pub view: View,
}

/// Most groups at once.
const MAX_GROUPS: usize = 4;

impl Editor {
    /// The group being drawn or worked in.
    pub(crate) fn pane(&self) -> usize {
        self.drawing
            .borrow()
            .as_ref()
            .map_or(self.group, |d| d.group)
    }

    /// Whether another group than the focused one is being drawn.
    pub(crate) fn drawing_other(&self) -> bool {
        self.drawing.borrow().is_some()
    }

    /// The cursor, selection and scroll of the document shown, in the group
    /// being drawn.
    pub(crate) fn view(&self) -> View {
        match &*self.drawing.borrow() {
            Some(d) => d.view.clone(),
            None => View::of(self.doc()),
        }
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.docs.iter().position(|d| d.id == id)
    }

    /// The group's tabs, as indexes into the documents.
    pub(crate) fn pane_tabs(&self) -> Vec<usize> {
        self.groups[self.pane()]
            .tabs
            .iter()
            .filter_map(|&id| self.index_of(id))
            .collect()
    }

    /// The document shown in the group being drawn.
    pub(crate) fn pane_doc(&self) -> usize {
        self.drawing
            .borrow()
            .as_ref()
            .map_or(self.current, |d| d.doc)
    }

    /// Draws `draw` as if group `g` were focused, with its own view.
    pub(crate) fn as_group<T>(&self, g: usize, draw: impl FnOnce() -> T) -> T {
        if g == self.group {
            return draw();
        }
        let group = &self.groups[g];
        let Some(doc) = self.index_of(group.active) else {
            return draw();
        };
        let view = group
            .saved
            .get(&group.active)
            .map(|v| v.clone().clamped(&self.docs[doc]))
            .unwrap_or_else(|| View::of(&self.docs[doc]));
        *self.drawing.borrow_mut() = Some(Drawing {
            group: g,
            doc,
            view,
        });
        let out = draw();
        *self.drawing.borrow_mut() = None;
        out
    }

    /// Makes another group the one keys go to.
    pub(crate) fn focus_group(&mut self, g: usize) {
        if g == self.group || g >= self.groups.len() {
            return;
        }
        let id = self.doc().id;
        let view = View::of(self.doc());
        self.groups[self.group].saved.insert(id, view);
        self.group = g;
        let active = self.groups[g].active;
        if let Some(i) = self.index_of(active) {
            self.current = i;
            self.take_over(active);
        }
        self.shown = (self.group, active);
    }

    /// Gives the focused group a document's own view: other groups showing
    /// it keep a copy of theirs first.
    fn take_over(&mut self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        let live = View::of(&self.docs[i]);
        for (g, group) in self.groups.iter_mut().enumerate() {
            if g != self.group && group.tabs.contains(&id) {
                group.saved.entry(id).or_insert_with(|| live.clone());
            }
        }
        if let Some(view) = self.groups[self.group].saved.remove(&id) {
            view.put(&mut self.docs[i]);
        }
    }

    /// Opens the current document in a new group on `edge` of the focused
    /// one, as VS Code's Split Editor commands do.
    pub(crate) fn split(&mut self, edge: Edge) {
        self.split_to(self.group, edge, false);
    }

    /// Opens the current document in a new group on `edge` of group `g`,
    /// moving it there from the focused group if `moving`, as dropping a
    /// tab on a group's side does.
    pub(crate) fn split_to(&mut self, g: usize, edge: Edge, moving: bool) {
        if g >= self.groups.len() {
            return;
        }
        let id = self.doc().id;
        let from = self.group;
        // A group's only tab, dropped on its own side: nothing to split.
        if moving && from == g && self.groups[g].tabs.len() == 1 {
            return;
        }
        if self.groups.len() >= MAX_GROUPS {
            return self.notify(format!("At most {MAX_GROUPS} editor groups"));
        }
        // The column `g` is in: its first group and its last.
        let head = (0..=g).rev().find(|&k| !self.groups[k].below).unwrap_or(0);
        let mut end = g;
        while self.groups.get(end + 1).is_some_and(|n| n.below) {
            end += 1;
        }
        let mut group = Group::new(id);
        let at = match edge {
            Edge::Right => end + 1,
            Edge::Left => head,
            Edge::Down | Edge::Up => {
                // Half of the group's height each, in its column's width.
                let old = &mut self.groups[g];
                old.height /= 2.0;
                group.height = old.height;
                group.width = old.width;
                if edge == Edge::Down {
                    group.below = true;
                    g + 1
                } else {
                    group.below = old.below;
                    old.below = true;
                    g
                }
            }
        };
        self.groups.insert(at, group);
        let from = if from >= at { from + 1 } else { from };
        let mut new = at;
        if moving {
            let src = &mut self.groups[from];
            let pos = src.tabs.iter().position(|&t| t == id).unwrap_or(0);
            src.tabs.retain(|&t| t != id);
            src.saved.remove(&id);
            src.pinned.remove(&id);
            if let Some(&next) = src.tabs.get(pos.min(src.tabs.len().saturating_sub(1))) {
                src.active = next;
            }
            if src.tabs.is_empty() {
                self.remove_group(from);
                if new > from {
                    new -= 1;
                }
            }
        } else {
            let view = View::of(self.doc());
            self.groups[from].saved.insert(id, view);
        }
        self.group = new;
        self.take_over(id);
        self.shown = (new, id);
    }

    /// Pins the current tab at the start of its group's strip, or unpins
    /// it after the pinned ones.
    pub(crate) fn pin(&mut self, pin: bool) {
        let id = self.doc().id;
        let group = &mut self.groups[self.group];
        if pin {
            // Last of the pinned ones.
            group.tabs.retain(|&t| t != id);
            let at = group
                .tabs
                .iter()
                .filter(|t| group.pinned.contains(t))
                .count();
            group.tabs.insert(at, id);
            group.pinned.insert(id);
            if self.preview == Some(id) {
                self.preview = None;
            }
        } else {
            group.pinned.remove(&id);
            group.order_pinned();
        }
    }

    /// Takes a group out, keeping the next one in its column.
    fn remove_group(&mut self, g: usize) {
        let head = !self.groups[g].below;
        self.groups.remove(g);
        if head && let Some(next) = self.groups.get_mut(g) {
            next.below = false;
        }
    }

    /// Moves a tab of the focused group to position `at` (or the end) of
    /// group `g`, which it then focuses, as dragging a tab does in VS Code.
    pub(crate) fn move_tab(&mut self, id: u64, g: usize, at: Option<usize>) {
        if g >= self.groups.len() || self.doc().id != id {
            return;
        }
        let from = self.group;
        if g == from {
            let group = &mut self.groups[g];
            group.tabs.retain(|&t| t != id);
            let at = at.unwrap_or(group.tabs.len()).min(group.tabs.len());
            group.tabs.insert(at, id);
            group.order_pinned();
            return;
        }
        let view = View::of(self.doc());
        let src = &mut self.groups[from];
        let pos = src.tabs.iter().position(|&t| t == id).unwrap_or(0);
        src.tabs.retain(|&t| t != id);
        src.saved.remove(&id);
        if let Some(&next) = src.tabs.get(pos.min(src.tabs.len().saturating_sub(1))) {
            src.active = next;
        }
        src.pinned.remove(&id);
        let dst = &mut self.groups[g];
        dst.tabs.retain(|&t| t != id);
        dst.tabs
            .insert(at.unwrap_or(dst.tabs.len()).min(dst.tabs.len()), id);
        dst.order_pinned();
        dst.active = id;
        dst.saved.insert(id, view);
        let mut g = g;
        if self.groups[from].tabs.is_empty() {
            self.remove_group(from);
            if g > from {
                g -= 1;
            }
        }
        self.group = g;
        self.take_over(id);
        self.shown = (g, id);
    }

    /// Steps through the focused group's tabs.
    pub(crate) fn next_tab(&mut self, forward: bool) {
        let tabs = &self.groups[self.group].tabs;
        let Some(at) = tabs.iter().position(|&id| id == self.doc().id) else {
            return;
        };
        let n = tabs.len();
        let next = tabs[if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        }];
        if let Some(i) = self.index_of(next) {
            self.current = i;
        }
    }

    /// Closes the current tab in a group that shares its document with
    /// another, leaving the document open there. Returns whether it did.
    pub(crate) fn close_shared_tab(&mut self) -> bool {
        let id = self.doc().id;
        let elsewhere = self
            .groups
            .iter()
            .enumerate()
            .any(|(g, group)| g != self.group && group.tabs.contains(&id));
        if !elsewhere {
            return false;
        }
        let group = &mut self.groups[self.group];
        let at = group.tabs.iter().position(|&t| t == id).unwrap_or(0);
        group.tabs.retain(|&t| t != id);
        if group.tabs.is_empty() {
            self.close_group();
        } else {
            let next = group.tabs[at.min(group.tabs.len() - 1)];
            if let Some(i) = self.index_of(next) {
                self.current = i;
            }
        }
        true
    }

    /// Removes the focused group, once it has no tabs, and focuses the one
    /// to its left.
    fn close_group(&mut self) {
        if self.groups.len() < 2 {
            return;
        }
        self.remove_group(self.group);
        self.group = self.group.saturating_sub(1).min(self.groups.len() - 1);
        let active = self.groups[self.group].active;
        if let Some(i) = self.index_of(active) {
            self.current = i;
            self.take_over(active);
        }
        self.shown = (self.group, active);
    }

    /// Brings the groups up to date with the open documents, after an
    /// event: closed ones leave their tabs, new ones join the focused
    /// group, and the focused group shows the current document.
    pub(crate) fn sync_groups(&mut self) {
        let open: Vec<u64> = self.docs.iter().map(|d| d.id).collect();
        for group in &mut self.groups {
            group.tabs.retain(|id| open.contains(id));
            group.saved.retain(|id, _| open.contains(id));
            group.pinned.retain(|id| open.contains(id));
        }
        // Groups left without tabs close, unless it's the last.
        while self.groups.len() > 1 {
            let Some(g) = self.groups.iter().position(|g| g.tabs.is_empty()) else {
                break;
            };
            self.remove_group(g);
            if self.group > g || self.group >= self.groups.len() {
                self.group = self.group.saturating_sub(1);
            }
        }
        let current = self.docs[self.current].id;
        // New documents open in the focused group, after its active tab.
        for &id in &open {
            let anywhere = self.groups.iter().any(|g| g.tabs.contains(&id));
            if !anywhere || (id == current && !self.groups[self.group].tabs.contains(&id)) {
                let group = &mut self.groups[self.group];
                let at = group
                    .tabs
                    .iter()
                    .position(|&t| t == group.active)
                    .map_or(group.tabs.len(), |p| p + 1);
                group.tabs.insert(at, id);
            }
        }
        for group in &mut self.groups {
            if !group.tabs.contains(&group.active) {
                group.active = *group.tabs.last().unwrap_or(&current);
            }
            group.order_pinned();
        }
        let (was_group, was) = self.shown;
        if (was_group, was) != (self.group, current) {
            // The group switched documents: keep where it was in the old
            // one, and go back to where it was in the new one.
            if was_group == self.group
                && was != current
                && let Some(i) = self.index_of(was)
            {
                let view = View::of(&self.docs[i]);
                self.groups[self.group].saved.insert(was, view);
            }
            self.groups[self.group].active = current;
            self.shown = (self.group, current);
            // Go back to where this group was in the document, unless
            // something just moved the cursor there, such as going to a
            // definition: then the view is still another group's.
            let i = self.current;
            let live = View::of(&self.docs[i]);
            let theirs = self
                .groups
                .iter()
                .enumerate()
                .any(|(g, group)| g != self.group && group.saved.get(&current) == Some(&live));
            match self.groups[self.group].saved.remove(&current) {
                Some(view) if theirs => view.put(&mut self.docs[i]),
                // Opened here from another group's view: its place and
                // folds come along, as in VS Code, but not its cursors.
                None if theirs => self.docs[i].extra.clear(),
                _ => {}
            }
        }
    }
}
