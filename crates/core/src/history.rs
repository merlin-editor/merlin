use crate::doc::Caret;
use crate::edit::Edit;

/// Identifies a state of the buffer. Revision 0 is the text as loaded, and an
/// id is never reused, so "is this the saved revision?" is always meaningful.
pub type Revision = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Single characters typed in a row, merged into one undo step per word.
    Typing,
    Other,
}

#[derive(Clone, Debug)]
pub struct Change {
    pub id: Revision,
    pub edits: Vec<Edit>,
    /// Every cursor before and after, the primary one first, so undo puts
    /// back all of them.
    pub cursors_before: Vec<Caret>,
    pub cursors_after: Vec<Caret>,
    kind: Kind,
}

/// Undo history. The buffer is dirty exactly when its current revision isn't
/// the saved one, so there is no dirty flag to get out of step with undo.
#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
    next_id: Revision,
    open: Option<Change>,
    depth: usize,
}

impl History {
    pub fn revision(&self) -> Revision {
        self.undo.last().map_or(0, |c| c.id)
    }

    pub fn in_transaction(&self) -> bool {
        self.depth > 0
    }

    /// Starts a transaction. A nested transaction joins the outermost one,
    /// which becomes a single undo step when it commits.
    pub fn begin(&mut self, cursors: Vec<Caret>, kind: Kind) {
        if self.depth == 0 {
            self.open = Some(Change {
                id: 0,
                edits: Vec::new(),
                cursors_after: cursors.clone(),
                cursors_before: cursors,
                kind,
            });
        }
        self.depth += 1;
    }

    pub fn record(&mut self, edit: Edit) {
        self.open
            .as_mut()
            .expect("edit outside a transaction")
            .edits
            .push(edit);
    }

    /// Ends a transaction. `saved` is the revision on disk: typing is never
    /// merged into it, or the buffer would look clean while differing from
    /// the file.
    pub fn commit(&mut self, cursors: Vec<Caret>, saved: Revision) {
        assert!(self.depth > 0, "commit without begin");
        self.depth -= 1;
        if self.depth > 0 {
            return;
        }
        let mut change = self.open.take().expect("open transaction");
        if change.edits.is_empty() {
            return;
        }
        self.redo.clear();
        if let Some(last) = self.undo.last_mut()
            && last.id != saved
            && continues_typing(last, &change)
        {
            last.edits.append(&mut change.edits);
            last.cursors_after = cursors;
            return;
        }
        change.cursors_after = cursors;
        self.next_id += 1;
        change.id = self.next_id;
        self.undo.push(change);
    }

    /// How many undo steps there are.
    pub fn steps(&self) -> usize {
        self.undo.len()
    }

    /// Makes the steps from `from` on one, as Vim's undo takes back a
    /// whole change typed in Insert mode. Never across `saved`, or the
    /// buffer would look clean while it differs from the file.
    pub fn join_from(&mut self, from: usize, saved: Revision) {
        let len = self.undo.len();
        if self.depth > 0 || len <= from + 1 {
            return;
        }
        let from = match self.undo[from..len - 1].iter().rposition(|c| c.id == saved) {
            Some(i) => from + i + 1,
            None => from,
        };
        if len <= from + 1 {
            return;
        }
        let rest = self.undo.split_off(from + 1);
        let first = self.undo.last_mut().expect("a step");
        for mut change in rest {
            first.edits.append(&mut change.edits);
            first.cursors_after = change.cursors_after;
            first.id = change.id;
        }
        first.kind = Kind::Other;
    }

    /// Takes the newest change off the undo stack. The caller reverts it.
    pub fn undo(&mut self) -> Option<Change> {
        if self.in_transaction() {
            return None;
        }
        let change = self.undo.pop()?;
        self.redo.push(change.clone());
        Some(change)
    }

    /// Puts the newest undone change back. The caller reapplies it.
    pub fn redo(&mut self) -> Option<Change> {
        if self.in_transaction() {
            return None;
        }
        let change = self.redo.pop()?;
        self.undo.push(change.clone());
        Some(change)
    }
}

fn continues_typing(last: &Change, next: &Change) -> bool {
    if last.kind != Kind::Typing || next.kind != Kind::Typing {
        return false;
    }
    let starts_word = |prev: &Edit, edit: &Edit| {
        edit.inserted.starts_with(char::is_whitespace)
            && !prev.inserted.ends_with(char::is_whitespace)
    };
    // Typing at several cursors: as many inserts as the keystroke before,
    // which had one per cursor too.
    let n = next.edits.len();
    if n > 1 {
        let Some(tail) = last.edits.len().checked_sub(n).map(|at| &last.edits[at..]) else {
            return false;
        };
        return tail
            .iter()
            .zip(&next.edits)
            .all(|(prev, edit)| prev.is_insert() && edit.is_insert() && !starts_word(prev, edit));
    }
    let (Some(prev), [edit]) = (last.edits.last(), next.edits.as_slice()) else {
        return false;
    };
    prev.is_insert() && edit.is_insert() && edit.at == prev.end() && !starts_word(prev, edit)
}
