//! A change since the last commit, opened in place under its lines from
//! its gutter bar, as VS Code's dirty diff peek shows it: what it was and
//! what it is, with Stage, Revert, and arrows to the next and previous
//! change. Alt+F5 and Alt+F3 go from change to change.

use std::ops::Range;

use super::{Editor, Effect};
use crate::diff::LineKind;

/// Most lines the peek shows; the rest are left out with a note.
const SHOWN: usize = 12;

/// The change open in place.
pub(crate) struct ChangePeek {
    pub doc: u64,
    pub group: usize,
    /// Which change, among the file's.
    pub index: usize,
    /// The file's changes, worked out for its text at `version`.
    pub changes: Vec<(Range<usize>, Range<usize>)>,
    pub version: u64,
}

/// The change open in place, as a window draws it in the rows after its
/// line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeView {
    /// The screen row it opens after.
    pub row: i32,
    /// Which change, from 1, and how many there are.
    pub index: usize,
    pub count: usize,
    /// What the lines were, and what they are, with `+` and `-`.
    pub lines: Vec<(char, String)>,
    /// Lines left out past the first `SHOWN`.
    pub more: usize,
    /// Whether all of it is staged, for Stage or Unstage.
    pub staged: bool,
    /// The rows it takes.
    pub rows: usize,
}

impl ChangePeek {
    /// The line it opens after: the change's last line, or the line above
    /// a deletion.
    fn line(&self) -> Option<usize> {
        let (_, new) = self.changes.get(self.index)?;
        Some(if new.is_empty() {
            new.start.saturating_sub(1)
        } else {
            new.end - 1
        })
    }
}

impl Editor {
    /// The current file's changes since the last commit.
    fn file_changes(&self) -> Vec<(Range<usize>, Range<usize>)> {
        let doc = self.doc();
        match &doc.git_base {
            Some(base) => crate::diff::runs(base, &doc.buffer.text()),
            None => Vec::new(),
        }
    }

    /// Opens the change at `line`, or closes it if it's the one open.
    pub(crate) fn toggle_change_peek(&mut self, line: usize) {
        let changes = self.file_changes();
        let at = changes.iter().position(|(_, new)| {
            new.contains(&line) || (new.is_empty() && new.start.min(self.doc().last_line()) == line)
        });
        let Some(index) = at else { return };
        if self
            .change_peek
            .as_ref()
            .is_some_and(|p| p.doc == self.doc().id && p.index == index)
        {
            self.change_peek = None;
            return;
        }
        self.peek = None;
        self.change_peek = Some(ChangePeek {
            doc: self.doc().id,
            group: self.pane(),
            index,
            changes,
            version: self.doc().buffer.version(),
        });
        self.reveal_change_peek();
    }

    /// Rows the change peek opens after `line` of the document drawn.
    pub(crate) fn change_gap(&self, line: usize) -> usize {
        match &self.change_peek {
            Some(p)
                if p.doc == self.doc().id && p.group == self.pane() && p.line() == Some(line) =>
            {
                let (old, new) = &p.changes[p.index];
                // Two rows for the header and the box's edges, then each
                // line, and one for the note of lines left out.
                2 + (old.len() + new.len()).min(SHOWN) + usize::from(old.len() + new.len() > SHOWN)
            }
            _ => 0,
        }
    }

    /// Whether the document drawn has a change open in place.
    pub(crate) fn change_peeking(&self) -> bool {
        self.change_peek
            .as_ref()
            .is_some_and(|p| p.doc == self.doc().id && p.group == self.pane())
    }

    /// The open change, drawn after screen row `row`.
    pub(crate) fn change_view(&self, row: i32) -> Option<ChangeView> {
        let p = self.change_peek.as_ref()?;
        let (old, new) = p.changes.get(p.index)?;
        let doc = self.doc();
        let base: Vec<&str> = doc.git_base.as_deref()?.split('\n').collect();
        let mut lines: Vec<(char, String)> = old
            .clone()
            .map(|l| ('-', base.get(l).copied().unwrap_or_default().to_string()))
            .collect();
        lines.extend(new.clone().map(|l| {
            let text: String = doc
                .line_chars(l)
                .into_iter()
                .take_while(|c| *c != '\n' && *c != '\r')
                .collect();
            ('+', text)
        }));
        let more = lines.len().saturating_sub(SHOWN);
        lines.truncate(SHOWN);
        let path = doc
            .buffer
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let staged = !(old.is_empty() && new.is_empty())
            && old
                .clone()
                .all(|l| super::compare::line_staged(&self.git, &path, LineKind::Removed, l))
            && new
                .clone()
                .all(|l| super::compare::line_staged(&self.git, &path, LineKind::Added, l));
        let rows = p.line().map_or(0, |line| self.change_gap(line));
        Some(ChangeView {
            row,
            index: p.index + 1,
            count: p.changes.len(),
            lines,
            more,
            staged,
            rows,
        })
    }

    /// Works the changes out again after an edit, keeping the one open on
    /// the change nearest where it was, and closing it once there's none.
    pub(crate) fn follow_change_peek(&mut self) {
        let Some(p) = &self.change_peek else { return };
        let Some(i) = self.docs.iter().position(|d| d.id == p.doc) else {
            self.change_peek = None;
            return;
        };
        let doc = &self.docs[i];
        if doc.buffer.version() == p.version {
            return;
        }
        let changes = match &doc.git_base {
            Some(base) => crate::diff::runs(base, &doc.buffer.text()),
            None => Vec::new(),
        };
        let version = doc.buffer.version();
        if changes.is_empty() {
            self.change_peek = None;
            return;
        }
        if let Some(p) = &mut self.change_peek {
            p.index = p.index.min(changes.len() - 1);
            p.changes = changes;
            p.version = version;
        }
    }

    /// Alt+F5 and Alt+F3 in a file: the cursor goes to the next or previous
    /// change, and an open change follows it.
    pub(crate) fn go_to_change(&mut self, forward: bool) {
        let changes = self.file_changes();
        if changes.is_empty() {
            return self.notify("No changes since the last commit".into());
        }
        let line = self.doc().line_col(self.doc().cursor).0;
        let starts: Vec<usize> = changes.iter().map(|(_, n)| n.start).collect();
        let index = if forward {
            starts.iter().position(|&s| s > line).unwrap_or(0)
        } else {
            starts
                .iter()
                .rposition(|&s| s < line)
                .unwrap_or(starts.len() - 1)
        };
        let target = starts[index].min(self.doc().last_line());
        let pos = self.doc().line_start(target);
        self.set_cursor(pos);
        if self.change_peeking() {
            self.peek = None;
            self.change_peek = Some(ChangePeek {
                doc: self.doc().id,
                group: self.pane(),
                index,
                changes,
                version: self.doc().buffer.version(),
            });
            self.reveal_change_peek();
        }
    }

    /// The open change's ↑ and ↓.
    pub(crate) fn step_change_peek(&mut self, forward: bool) {
        let Some(p) = &self.change_peek else { return };
        let count = p.changes.len();
        if count == 0 {
            return;
        }
        let index = if forward {
            (p.index + 1) % count
        } else {
            (p.index + count - 1) % count
        };
        let start = p.changes[index].1.start;
        if let Some(p) = &mut self.change_peek {
            p.index = index;
        }
        let target = start.min(self.doc().last_line());
        let pos = self.doc().line_start(target);
        self.set_cursor(pos);
        self.reveal_change_peek();
    }

    /// Scrolls so the open change and its lines show.
    fn reveal_change_peek(&mut self) {
        let Some(line) = self.change_peek.as_ref().and_then(ChangePeek::line) else {
            return;
        };
        let rows = self.change_gap(line);
        let height = self.text_height();
        let doc = self.doc_mut();
        if line + 1 + rows > doc.scroll + height {
            doc.scroll = (line + 1 + rows).saturating_sub(height);
        }
        if line < doc.scroll {
            doc.scroll = line;
        }
    }

    /// The open change's Stage: stages it, or unstages it when it's all
    /// staged. Git sees the file on disk, so it's saved first.
    pub(crate) fn stage_change_peek(&mut self, fx: &mut Vec<Effect>) {
        let Some(view) = self.change_view(0) else {
            return;
        };
        let Some(p) = &self.change_peek else { return };
        let (old, new) = p.changes[p.index].clone();
        let doc = self.doc();
        if doc.buffer.is_dirty() {
            let name = doc.name();
            return self.notify(format!("Save {name} first, so git sees the change"));
        }
        let Some(path) = doc
            .buffer
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
        else {
            return;
        };
        let parts: Vec<(LineKind, usize)> = old
            .map(|l| (LineKind::Removed, l))
            .chain(new.map(|l| (LineKind::Added, l)))
            .collect();
        self.stage_parts(&path, &parts, !view.staged, fx);
    }

    /// The open change's Revert: its lines go back to the last commit's,
    /// as an edit Ctrl+Z undoes.
    pub(crate) fn revert_change_peek(&mut self) {
        let Some(p) = &self.change_peek else { return };
        let (old, new) = p.changes[p.index].clone();
        let doc = self.doc();
        let Some(base) = doc.git_base.clone() else {
            return;
        };
        let text = doc.buffer.text();
        let base_lines: Vec<&str> = base.split('\n').collect();
        let lines: Vec<&str> = text.split('\n').collect();
        let mut out: Vec<&str> = lines[..new.start.min(lines.len())].to_vec();
        out.extend(&base_lines[old.start.min(base_lines.len())..old.end.min(base_lines.len())]);
        out.extend(&lines[new.end.min(lines.len())..]);
        let reverted = out.join("\n");
        let doc = self.doc_mut();
        super::replace_text(doc, &text, &reverted);
        self.change_peek = None;
    }
}
