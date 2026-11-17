//! An agent's changes, reviewed in the editor (AgentChat.html): each one
//! tinted, what it took out shown in place, Undo and Keep under it, and a
//! bar at the bottom to step through the changes and the files.

use std::ops::Range;
use std::path::{Path, PathBuf};

use super::agents::Change;
use super::{Editor, Effect};

/// Most lines taken out that show in place; the rest are counted.
const SHOWN: usize = 8;

/// Rows opened between lines for an agent's change: what it took out, or
/// its Undo and Keep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentHunkView {
    /// The screen row it opens after.
    pub row: i32,
    /// Lines taken out, and how many more weren't shown; empty for the
    /// buttons' row.
    pub removed: Vec<String>,
    pub more: usize,
    /// Undo and Keep, for change `index` (from 0), with who made it.
    pub buttons: Option<(usize, String)>,
    pub rows: usize,
}

/// The bar at the bottom of a file an agent changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentBarView {
    pub file: String,
    /// The change the cursor is at or before, from 1, and how many.
    pub index: usize,
    pub count: usize,
    /// The next file with changes to review, if another.
    pub next_file: Option<String>,
}

/// The rows an agent's changes open after each line, for a document at a
/// version and its changes' texts before.
#[derive(Default)]
pub(crate) struct RowsCache {
    key: Option<(u64, u64, u64)>,
    rows: std::collections::HashMap<usize, usize>,
}

/// One of an agent's changes to the open file: lines it was, lines it is.
pub(crate) struct Hunks {
    pub runs: Vec<(Range<usize>, Range<usize>)>,
    pub before: Vec<String>,
    pub who: String,
}

impl Editor {
    /// The workspace path of `path`, made whole.
    fn whole(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    /// The agent tab and change for the file open now, if an agent changed
    /// it and it's waiting to be kept.
    pub(crate) fn agent_change_here(&self) -> Option<(usize, usize)> {
        let here = self.whole(self.doc().buffer.path.as_deref()?);
        self.agents.tabs.iter().enumerate().find_map(|(i, t)| {
            t.changes
                .iter()
                .position(|c| self.whole(&c.path) == here)
                .map(|k| (i, k))
        })
    }

    /// The open file's changes from an agent, against how it was.
    pub(crate) fn agent_hunks(&self) -> Option<Hunks> {
        let (i, k) = self.agent_change_here()?;
        let tab = &self.agents.tabs[i];
        let change: &Change = &tab.changes[k];
        let now = self.doc().buffer.text();
        let runs = crate::diff::runs(&change.before, &now);
        if runs.is_empty() {
            return None;
        }
        let name = self.agent_label(&tab.key, tab.mode.as_deref());
        Some(Hunks {
            runs,
            before: change.before.split('\n').map(str::to_string).collect(),
            who: name,
        })
    }

    /// Who made a change, as "◇ opencode · build".
    fn agent_label(&self, key: &str, mode: Option<&str>) -> String {
        let spec = self.agents.specs.iter().find(|s| s.key() == key);
        let who = spec.map_or_else(|| key.to_string(), |s| format!("{} {}", s.icon, s.name));
        match mode {
            Some(m) => format!("{who} · {m}"),
            None => who,
        }
    }

    /// The rows an agent's changes open after `line`, at screen row `row`:
    /// what was taken out over the first line of a change, its Undo and
    /// Keep under its last.
    pub(crate) fn agent_gaps(
        &self,
        hunks: &Hunks,
        line: usize,
        mut row: i32,
    ) -> Vec<AgentHunkView> {
        let mut out = Vec::new();
        for (n, (old, new)) in hunks.runs.iter().enumerate() {
            // Taken out: over the change, so after the line before it.
            let above = new.start.checked_sub(1);
            let removed_after = if old.is_empty() {
                None
            } else if new.is_empty() || above.is_some() {
                above.or(Some(0))
            } else {
                Some(new.end - 1)
            };
            if removed_after == Some(line) {
                let lines: Vec<String> = old
                    .clone()
                    .map(|l| hunks.before.get(l).cloned().unwrap_or_default())
                    .collect();
                let more = lines.len().saturating_sub(SHOWN);
                let removed: Vec<String> = lines.into_iter().take(SHOWN).collect();
                let rows = removed.len() + usize::from(more > 0);
                out.push(AgentHunkView {
                    row,
                    removed,
                    more,
                    buttons: None,
                    rows,
                });
                row += rows as i32;
            }
            let buttons_after = if new.is_empty() {
                new.start.saturating_sub(1)
            } else {
                new.end - 1
            };
            if buttons_after == line {
                out.push(AgentHunkView {
                    row,
                    removed: Vec::new(),
                    more: 0,
                    buttons: Some((n, hunks.who.clone())),
                    rows: 1,
                });
                row += 1;
            }
        }
        out
    }

    /// The rows an agent's changes open after `line` of the open file.
    pub(crate) fn agent_gap(&self, line: usize) -> usize {
        if self.doc().review.is_some() || self.in_compare() {
            return 0;
        }
        let Some((i, k)) = self.agent_change_here() else {
            return 0;
        };
        let before = &self.agents.tabs[i].changes[k].before;
        let key = (
            self.doc().id,
            self.doc().buffer.version(),
            before.len() as u64 ^ (self.agents.tabs[i].changes.len() as u64) << 48,
        );
        let mut cache = self.agent_rows.borrow_mut();
        if cache.key != Some(key) {
            cache.key = Some(key);
            cache.rows.clear();
            if let Some(h) = self.agent_hunks() {
                let mut lines: Vec<usize> = Vec::new();
                for (_, new) in &h.runs {
                    lines.push(new.start.saturating_sub(1));
                    lines.push(new.end.saturating_sub(1));
                }
                lines.sort_unstable();
                lines.dedup();
                for l in lines {
                    let rows: usize = self.agent_gaps(&h, l, 0).iter().map(|g| g.rows).sum();
                    if rows > 0 {
                        cache.rows.insert(l, rows);
                    }
                }
            }
        }
        cache.rows.get(&line).copied().unwrap_or(0)
    }

    /// The lines an agent's changes open rows after.
    pub(crate) fn agent_gap_lines(&self) -> Vec<usize> {
        // Asking for one line's rows makes them all.
        self.agent_gap(0);
        self.agent_rows.borrow().rows.keys().copied().collect()
    }

    /// Whether `line` of the open file is one an agent added or changed.
    pub(crate) fn agent_added(hunks: &Hunks, line: usize) -> bool {
        hunks.runs.iter().any(|(_, new)| new.contains(&line))
    }

    /// The bar at the bottom of the open file, when an agent changed it.
    pub(crate) fn agent_bar(&self, hunks: &Hunks) -> AgentBarView {
        let cur = self.doc().line_col(self.doc().cursor).0;
        let index = hunks
            .runs
            .iter()
            .position(|(_, new)| new.end > cur || new.contains(&cur))
            .unwrap_or(hunks.runs.len() - 1);
        let here = self.doc().buffer.path.clone().unwrap_or_default();
        let next_file = self
            .agents
            .tabs
            .iter()
            .flat_map(|t| &t.changes)
            .find(|c| self.whole(&c.path) != self.whole(&here))
            .map(|c| self.shown_name(&c.path));
        AgentBarView {
            file: self.shown_name(&here),
            index: index + 1,
            count: hunks.runs.len(),
            next_file,
        }
    }

    fn shown_name(&self, path: &Path) -> String {
        path.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    }

    /// Undo or Keep one of the open file's changes from an agent.
    pub(crate) fn agent_hunk(&mut self, index: usize, keep: bool, fx: &mut Vec<Effect>) {
        let Some((i, k)) = self.agent_change_here() else {
            return;
        };
        let Some(hunks) = self.agent_hunks() else {
            return;
        };
        let Some((old, new)) = hunks.runs.get(index).cloned() else {
            return;
        };
        let now = self.doc().buffer.text();
        let now_lines: Vec<&str> = now.split('\n').collect();
        if keep {
            // How it was, with this change in: kept, it's no longer one.
            let mut before = hunks.before.clone();
            before.splice(old, now_lines[new].iter().map(|s| s.to_string()));
            self.agents.tabs[i].changes[k].before = before.join("\n");
        } else {
            // The file, with this change taken back.
            let mut lines: Vec<String> = now_lines.iter().map(|s| s.to_string()).collect();
            lines.splice(new, hunks.before[old].iter().cloned());
            let text = lines.join("\n");
            self.replace_open_text(text, fx);
        }
        self.agent_settle(i, k);
    }

    /// Undo or Keep all the open file's changes from an agent.
    pub(crate) fn agent_hunk_file(&mut self, keep: bool, fx: &mut Vec<Effect>) {
        let Some((i, k)) = self.agent_change_here() else {
            return;
        };
        if keep {
            self.agents.tabs[i].changes[k].before = self.doc().buffer.text();
        } else {
            let before = self.agents.tabs[i].changes[k].before.clone();
            self.replace_open_text(before, fx);
        }
        self.agent_settle(i, k);
    }

    /// Moves the cursor to the next or previous change in the open file.
    pub(crate) fn agent_hunk_step(&mut self, forward: bool) {
        let Some(hunks) = self.agent_hunks() else {
            return;
        };
        let cur = self.doc().line_col(self.doc().cursor).0;
        let starts: Vec<usize> = hunks.runs.iter().map(|(_, n)| n.start).collect();
        let to = if forward {
            starts.iter().find(|&&s| s > cur).or(starts.first())
        } else {
            starts.iter().rev().find(|&&s| s < cur).or(starts.last())
        };
        if let Some(&line) = to {
            let at = self.doc().line_start(line.min(self.doc().last_line()));
            let doc = self.doc_mut();
            doc.cursor = at;
            doc.anchor = None;
            self.reveal_cursor();
        }
    }

    /// Opens the next file an agent changed.
    pub(crate) fn agent_next_file(&mut self, fx: &mut Vec<Effect>) {
        let here = self.doc().buffer.path.clone().unwrap_or_default();
        let next = self
            .agents
            .tabs
            .iter()
            .flat_map(|t| &t.changes)
            .find(|c| self.whole(&c.path) != self.whole(&here))
            .map(|c| c.path.clone());
        if let Some(path) = next {
            fx.push(Effect::Open(self.whole(&path)));
        }
    }

    /// The open file's text replaced as one edit, which Undo takes back,
    /// and saved.
    fn replace_open_text(&mut self, text: String, fx: &mut Vec<Effect>) {
        let doc = self.doc_mut();
        let now = doc.buffer.text();
        if now == text {
            return;
        }
        doc.buffer.begin(doc.cursor, crate::history::Kind::Other);
        doc.buffer.apply(crate::edit::Edit {
            at: 0,
            removed: now,
            inserted: text.clone(),
        });
        doc.cursor = doc.cursor.min(doc.buffer.len());
        doc.anchor = None;
        doc.buffer.commit(doc.cursor);
        if let Some(path) = doc.buffer.path.clone() {
            fx.push(Effect::Save {
                doc: doc.id,
                path,
                text,
                revision: doc.buffer.revision(),
            });
        }
    }

    /// After Undo or Keep: a change that's no longer one is dropped, and
    /// the chat shows what's left.
    fn agent_settle(&mut self, i: usize, k: usize) {
        let now = self.doc().buffer.text();
        let change = &mut self.agents.tabs[i].changes[k];
        if change.before == now {
            self.agents.tabs[i].changes.remove(k);
        } else {
            (change.added, change.removed) = super::agents::line_counts(&change.before, &now);
        }
        let key = self.agents.tabs[i].key.clone();
        self.draw_agent(&key);
    }
}
