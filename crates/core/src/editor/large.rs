//! Files too large to load, read from disk a window at a time (see
//! [`crate::bigfile`]): which window to hold as the view moves, where to go
//! once one comes, the lines and a search counted in the background, and
//! what the bar, the gutter, the status bar and the scrollbar say.
//!
//! Edited in place (`docs/large-edit.md`), each edit goes into the window,
//! which is put back in the text's pieces after it. A window left with
//! undo is kept, so undo goes back through the windows in turn. Read-only,
//! editing asks to load the file first.

use super::*;
use crate::bigfile::{Edited, Large, Then, WINDOW};

/// The bar over a large file, among the bars: past any plugin's.
pub(crate) const LARGE_BAR: usize = usize::MAX;

/// How much of the file loading it takes in memory, against its size: the
/// rope's own share, and a little for what the editor keeps beside it.
const LOADED: f64 = 1.15;

impl Editor {
    /// The current document's large file, if it's one.
    pub(crate) fn large(&self) -> Option<&Large> {
        self.doc().large.as_deref()
    }

    fn large_mut(&mut self) -> Option<&mut Large> {
        self.doc_mut().large.as_deref_mut()
    }

    /// A file too large to load: opened as its first window, read-only,
    /// with its lines counted in the background.
    pub(super) fn opened_large(
        &mut self,
        path: PathBuf,
        full: PathBuf,
        free: Option<u64>,
        editable: bool,
    ) {
        // Kept open: the pieces read the file as it was opened.
        let text = match crate::bigfile::FileText::open(&full) {
            Ok(text) => text,
            Err(err) => return self.error(format!("Can't open {}: {err}", path.display())),
        };
        let size = text.len();
        let mut large = Box::new(Large::new(text, free, editable));
        large.saved_as = Some(path.clone());
        // Open already, as after it changed on disk: read again from the
        // same place.
        if let Some(i) = self.docs.iter().position(|d| {
            d.review.is_none() && d.large.is_some() && d.buffer.path.as_ref() == Some(&path)
        }) {
            let doc = &mut self.docs[i];
            let top = doc
                .large
                .as_ref()
                .map(|l| l.start + doc.buffer.rope().line_to_byte(doc.scroll) as u64)
                .unwrap_or(0);
            let mut large = large;
            large.pending = Some(Then::Top(top.min(size)));
            large.bar_hidden = doc.large.as_ref().is_some_and(|l| l.bar_hidden);
            doc.large = Some(large);
            return;
        }
        self.opened_doc(path, ropey::Rope::new(), Some(large));
    }

    /// The background work a large file waits on: its lines, and the
    /// search's matches while Find has a query.
    pub(super) fn large_jobs(&mut self, fx: &mut Vec<Effect>) {
        let matcher = self.find.as_ref().and_then(|_| self.query());
        let mut next = self.next_job;
        let Some(large) = self.large_mut() else {
            return;
        };
        if !large.lines_done() && large.lines_job.is_none() {
            next += 1;
            large.lines_job = Some(next);
            fx.push(Effect::Work(crate::work::Job {
                id: next,
                kind: crate::work::Kind::Lines(large.file.clone()),
                text: Default::default(),
            }));
        }
        match matcher {
            Some(matcher) => {
                if large.found.as_ref().is_none_or(|f| f.matcher != matcher) {
                    large.found = Some(crate::bigfile::FileFound::new(
                        matcher.clone(),
                        &large.file,
                        &large.text,
                    ));
                }
                // What's new since: at first all of it; after an edit, the
                // window's text and the places it cuts the file.
                let found = large.found.as_mut().expect("set");
                let parts = found.wanted(&large.file, &large.text, large.generation);
                if !parts.is_empty() {
                    next += 1;
                    found.jobs.push(next);
                    fx.push(Effect::Work(crate::work::Job {
                        id: next,
                        kind: crate::work::Kind::FileCount {
                            file: large.file.clone(),
                            text: large.text.clone(),
                            matcher,
                            parts,
                        },
                        text: Default::default(),
                    }));
                }
            }
            None => large.found = None,
        }
        self.next_job = next;
    }

    /// Asks for the window the view needs: the first, one further along
    /// as the view nears an edge, one where a jump goes, or a search past
    /// the window.
    pub(super) fn ask_window(&mut self, fx: &mut Vec<Effect>) {
        let height = self.text_height().max(1);
        let doc = self.doc();
        let id = doc.id;
        let Some(large) = doc.large.as_deref() else {
            return;
        };
        if let Some((from, forward)) = large.seek
            && !large.seeking
            && let Some(found) = &large.found
        {
            fx.push(Effect::SeekFile {
                doc: id,
                text: large.text.clone(),
                matcher: found.matcher.clone(),
                from,
                forward,
                counts: found.empty_blocks(&large.file, &large.text),
            });
            let large = self.large_mut().expect("checked");
            large.seek = None;
            large.seeking = true;
            return;
        }
        if large.asked.is_some() {
            return;
        }
        if !large.has_window() {
            return self.ask_large(0, Then::Stay, fx);
        }
        if let Some(then) = large.pending.clone() {
            return self.go_large(then, fx);
        }
        let rope = doc.buffer.rope();
        let lines = rope.len_lines();
        let top = doc.scroll.min(lines - 1);
        let byte = |line: usize| large.start + rope.line_to_byte(line.min(lines)) as u64;
        let (top_byte, bottom_byte) = (byte(top), byte(top + height));
        let near_top = large.start > 0 && top_byte < large.start + WINDOW / 4;
        let near_end = large.end < large.size && bottom_byte + WINDOW / 4 > large.end;
        if (near_top || near_end) && large.settled != Some(top_byte) {
            self.ask_large(top_byte, Then::Stay, fx);
        }
    }

    fn ask_large(&mut self, around: u64, then: Then, fx: &mut Vec<Effect>) {
        let id = self.doc().id;
        let Some(large) = self.large_mut() else {
            return;
        };
        let from = large.from(around);
        large.asked = Some((around, then));
        fx.push(Effect::ReadWindow {
            doc: id,
            text: large.text.clone(),
            from,
        });
    }

    /// Goes where `then` says: in the window held if it's there, or once
    /// the window there is read, or for a line, once it's counted.
    fn go_large(&mut self, then: Then, fx: &mut Vec<Effect>) {
        let height = self.text_height() as u64;
        let Some(large) = self.large() else {
            return;
        };
        let lines = self.doc().buffer.rope().len_lines() as u64;
        let held = |b: u64| {
            b >= large.start && (b < large.end || (b == large.end && large.end == large.size))
        };
        // The window has the place, with a view's height to spare where
        // the file goes on.
        let here = match &then {
            Then::Stay => true,
            Then::Line(line, _) | Then::TopLine(line) => large.first_line.is_some_and(|f| {
                *line >= f
                    && *line < f + lines
                    && (f == 0 || *line >= f + height)
                    && (large.end == large.size || *line + 2 * height < f + lines)
            }),
            Then::Byte(b) | Then::Top(b) | Then::Match(b) => held(*b),
        };
        // Read again closer at most a few times, as the place of a line in
        // the file is known only to a block.
        if here || large.tries >= 3 {
            let large = self.large_mut().expect("checked");
            large.pending = None;
            large.tries = 0;
            return self.land(then);
        }
        let per_line = large.bytes_per_line(lines as usize);
        let near = match (&then, large.first_line) {
            // Past the window held: on from its edge, by the lines between.
            (Then::Line(line, _) | Then::TopLine(line), Some(f)) if large.tries > 0 => {
                if *line >= f + lines {
                    Some(large.end + ((*line - (f + lines)) as f64 * per_line) as u64 + WINDOW / 4)
                } else if *line < f {
                    Some(
                        large
                            .start
                            .saturating_sub(((f - *line) as f64 * per_line) as u64 + WINDOW / 4),
                    )
                } else {
                    Some(large.start + ((*line - f) as f64 * per_line) as u64)
                }
            }
            _ => None,
        };
        let around = match &then {
            Then::Line(..) | Then::TopLine(..) if near.is_some() => near.expect("checked"),
            Then::Line(line, _) | Then::TopLine(line) => match large.place_of_line(*line) {
                // A little past it, as it's a block or so before the line.
                Some(at) => at + WINDOW / 2 - WINDOW / 8,
                None => {
                    // Not counted that far yet: it waits.
                    self.large_mut().expect("checked").pending = Some(then);
                    return;
                }
            },
            Then::Byte(b) | Then::Top(b) | Then::Match(b) => *b,
            Then::Stay => return,
        };
        let large = self.large_mut().expect("checked");
        large.pending = None;
        large.tries += 1;
        self.ask_large(around, then, fx);
    }

    /// Puts the cursor or the view where `then` says, in the window held.
    fn land(&mut self, then: Then) {
        let half = self.text_height() / 2;
        let doc = self.doc();
        let Some(large) = doc.large.as_deref() else {
            return;
        };
        let rope = doc.buffer.rope();
        let local =
            |b: u64| rope.byte_to_char((b.clamp(large.start, large.end) - large.start) as usize);
        let line_of = |l: u64| {
            let first = large.first_line.unwrap_or(0);
            (l.saturating_sub(first) as usize).min(rope.len_lines() - 1)
        };
        // The cursor to put, if any, and the line for the top.
        let (cursor, top) = match then {
            Then::Stay => return,
            Then::Line(line, col) => {
                let l = line_of(line);
                let pos = doc.line_start(l) + col.min(doc.buffer.line_len(l));
                (Some(pos), l.saturating_sub(half))
            }
            Then::TopLine(line) => (None, line_of(line)),
            Then::Top(b) => (None, rope.char_to_line(local(b))),
            Then::Byte(b) | Then::Match(b) => {
                let pos = local(b);
                (Some(pos), rope.char_to_line(pos).saturating_sub(half))
            }
        };
        match cursor {
            Some(pos) => {
                self.set_cursor(pos);
                self.set_top(top, 0);
            }
            None => {
                self.set_top(top, 0);
                self.keep_cursor_in_view();
            }
        }
    }

    /// The cursor onto the view, when the view moved away from it.
    fn keep_cursor_in_view(&mut self) {
        let height = self.text_height().max(1);
        let doc = self.doc();
        let (line, _) = doc.line_col(doc.cursor);
        let top = doc.scroll;
        if line < top || line >= top + height {
            let pos = doc.line_start(top.min(doc.last_line()));
            self.set_cursor(pos);
        }
    }

    /// A window of a large file came: it takes the place of the one held,
    /// keeping the cursor, the selection and the view on the same text,
    /// then goes where it was asked for. One read from text since edited
    /// is read again. The window left, if it has undo, is kept for it.
    pub(super) fn window_read(
        &mut self,
        id: u64,
        window: Result<Box<crate::bigfile::Window>, String>,
    ) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        let doc = &mut self.docs[i];
        let Some(mut large) = doc.large.take() else {
            return;
        };
        let (_, then) = large.asked.take().unwrap_or((0, Then::Stay));
        let w = match window {
            Ok(w) => *w,
            Err(err) => {
                doc.large = Some(large);
                let name = doc.name();
                return self.error(format!("Can't read {name}: {err}"));
            }
        };
        if w.generation != large.generation {
            if then != Then::Stay {
                large.pending = Some(then);
            }
            large.settled = None;
            doc.large = Some(large);
            return;
        }
        large.lines.edges.extend(w.edges.iter().copied());
        let old = doc.buffer.rope();
        let byte_of = |pos: usize| large.start + old.char_to_byte(pos.min(old.len_chars())) as u64;
        let top = large.start + old.line_to_byte(doc.scroll.min(old.len_lines() - 1)) as u64;
        let cursor = byte_of(doc.cursor);
        let anchor = doc.anchor.map(byte_of);
        let rope = w.text.clone();
        let inside = |b: u64| b >= w.start && b <= w.end;
        let local = |b: u64| rope.byte_to_char((b.clamp(w.start, w.end) - w.start) as usize);
        let mut new = Doc::from_rope(doc.id, w.text, doc.buffer.path.clone());
        new.config = doc.config.clone();
        new.hscroll = doc.hscroll;
        if inside(cursor) {
            new.cursor = local(cursor);
            new.anchor = anchor.filter(|&a| inside(a)).map(local);
        } else {
            // Scrolled away from it, past the window: the view's top.
            new.cursor = local(top);
        }
        if inside(top) {
            let line = rope.char_to_line(local(top));
            new.scroll = line;
            new.scroll_sub = crate::doc::Sub {
                line,
                row: 0,
                frac: doc.scroll_sub.frac,
            };
        }
        large.synced = new.buffer.version();
        let left = std::mem::replace(doc, new);
        keep_undo(&mut large, left.buffer, false);
        large.start = w.start;
        large.end = w.end;
        large.size = w.size;
        large.settled = Some(top);
        large.renumber();
        doc.large = Some(large);
        doc.sync_large();
        // Where it was going: there now if this window has it, or read
        // again closer.
        if then != Then::Stay
            && let Some(large) = self.docs[i].large.as_deref_mut()
        {
            large.pending = Some(then);
        }
    }

    /// Puts what's been typed in the window into the text, once per
    /// change, so what's read and counted from the text has it.
    pub(super) fn sync_large(&mut self) {
        self.doc_mut().sync_large();
    }

    /// Undo or redo in a large file edited in place: in the window, or
    /// past it, in the window edited before, brought back with its undo.
    /// Whether it was one.
    pub(crate) fn undo_large(&mut self, redo: bool) -> bool {
        if self.large().is_none() {
            return false;
        }
        if self.refuse_large_edit() {
            return true;
        }
        for _ in 0..2 {
            let doc = self.doc_mut();
            let done = if redo {
                doc.buffer.redo()
            } else {
                doc.buffer.undo()
            };
            if let Some(cursors) = done {
                self.put_cursors(&cursors);
                let doc = self.doc_mut();
                if let Some(large) = doc.large.as_deref_mut() {
                    large.undoing = true;
                }
                doc.sync_large();
                return true;
            }
            // Past this window: the one edited before, or undone after.
            let doc = self.doc_mut();
            doc.sync_large();
            let large = doc.large.as_deref_mut().expect("checked");
            let Some(next) = (if redo {
                large.redos.pop()
            } else {
                large.undos.pop()
            }) else {
                break;
            };
            let mut new = Doc::from_rope(doc.id, ropey::Rope::new(), doc.buffer.path.clone());
            new.config = doc.config.clone();
            new.buffer = next.buffer;
            let _ = new.buffer.take_changes();
            let len = new.buffer.rope().len_bytes() as u64;
            let mut large = doc.large.take().expect("checked");
            large.synced = new.buffer.version();
            let left = std::mem::replace(doc, new);
            // The window left goes on the other stack, to come back to.
            keep_undo(&mut large, left.buffer, !redo);
            large.start = next.start;
            large.end = next.start + len;
            large.settled = None;
            large.renumber();
            doc.large = Some(large);
            doc.sync_large();
        }
        self.notify(
            if redo {
                "Nothing to redo"
            } else {
                "Nothing to undo"
            }
            .into(),
        );
        true
    }

    /// Saves a large file: its text, edits and all, streamed to a new file
    /// that takes the old one's place. With nothing changed, only a Save
    /// As elsewhere writes.
    pub(super) fn save_large(&mut self, fx: &mut Vec<Effect>) {
        self.sync_large();
        let doc = self.doc();
        let (id, dirty) = (doc.id, doc.buffer.is_dirty());
        let Some(path) = doc.buffer.path.clone() else {
            return;
        };
        let Some(large) = self.large_mut() else {
            return;
        };
        if large.saving.is_some() {
            return;
        }
        if !dirty && large.saved_as.as_ref() == Some(&path) {
            return;
        }
        large.saving = Some(0);
        large.save_error = None;
        large.bar_hidden = false;
        fx.push(Effect::SaveLarge {
            doc: id,
            path,
            text: large.text.clone(),
        });
    }

    /// How far a large file's save has got.
    pub(super) fn large_saving(&mut self, id: u64, written: u64) {
        if let Some(i) = self.index_of(id)
            && let Some(large) = self.docs[i].large.as_deref_mut()
            && large.saving.is_some()
        {
            large.saving = Some(written);
        }
    }

    /// A large file's save is done: the text is now the new file, read
    /// from it, with its lines as counted while writing, and the edits
    /// saved. Undo still goes back past it. Or it failed, or was
    /// cancelled, and the file is as it was.
    pub(super) fn large_saved(
        &mut self,
        id: u64,
        path: PathBuf,
        full: PathBuf,
        result: crate::bigfile::Saved,
        fx: &mut Vec<Effect>,
    ) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        let doc = &mut self.docs[i];
        let name = doc.name();
        let Some(large) = doc.large.as_deref_mut() else {
            return;
        };
        large.saving = None;
        let blocks = match result {
            Ok(blocks) => blocks,
            Err(err) => {
                self.quitting = false;
                self.closing.retain(|&c| c != id);
                if err == crate::bigfile::SaveError::Cancelled {
                    large.save_error = None;
                    return self.notify(format!("Save cancelled: {name} is as it was"));
                }
                large.save_error = Some(err);
                return;
            }
        };
        // The new file, read from now on: the same text, so the window,
        // the cursor and undo stay as they are.
        let file = match crate::bigfile::FileText::open(&full) {
            Ok(file) if file.len() == large.size => file,
            Ok(_) | Err(_) => {
                let err = "it saved, but reads back a different size".to_string();
                large.save_error = Some(crate::bigfile::SaveError::Failed(err));
                return;
            }
        };
        let mut lines = crate::bigfile::FileLines::new(file.len());
        for (b, count) in blocks.into_iter().enumerate() {
            if let Some(slot) = lines.blocks.get_mut(b) {
                *slot = Some(count);
            }
        }
        for at in [large.start, large.end] {
            if at % crate::bigfile::BLOCK != 0
                && at < file.len()
                && let Ok(breaks) = file.file_breaks(at)
            {
                lines.edges.insert(at, breaks);
            }
        }
        large.file = file.clone();
        large.text = file;
        large.lines = lines;
        large.lines_job = None;
        large.generation += 1;
        large.found = None;
        large.changed = false;
        large.save_error = None;
        large.saved_as = Some(path);
        for kept in large.undos.iter_mut().chain(large.redos.iter_mut()) {
            let revision = kept.buffer.revision();
            kept.buffer.mark_saved(revision);
        }
        large.renumber();
        let revision = doc.buffer.revision();
        doc.buffer.mark_saved(revision);
        doc.buffer.elsewhere = false;
        self.after_save(id, fx);
    }

    /// A search past the window found its match, or none in the file.
    pub(super) fn file_sought(&mut self, id: u64, at: Option<u64>) {
        let Some(i) = self.index_of(id) else {
            return;
        };
        let Some(large) = self.docs[i].large.as_deref_mut() else {
            return;
        };
        large.seeking = false;
        match at {
            Some(at) => large.pending = Some(Then::Match(at)),
            None if i == self.current => self.go_to_match(None),
            None => {}
        }
    }

    /// Find's next or previous match in a large file: in the window if
    /// it's there, or past it, read from disk.
    pub(super) fn seek_large(&mut self, at: usize, forward: bool) {
        let Some(matcher) = self.query() else {
            return;
        };
        let doc = self.doc();
        let rope = doc.buffer.rope();
        let at = at.min(rope.len_chars());
        let hit = match forward {
            true => matcher.next(rope, at, None, |p| p >= at).map(|h| h.0),
            false => matcher
                .previous(rope, at, None)
                .map(|h| h.0)
                .filter(|&p| p < at),
        };
        if let Some(pos) = hit {
            return self.go_to_match(Some(pos));
        }
        let Some(large) = self.large_mut() else {
            return;
        };
        // On from the window's edge, the side it's going.
        let from = if forward { large.end } else { large.start };
        large.seek = Some((from, forward));
    }

    /// The line of the file a line of the window is, once it's counted.
    pub(crate) fn file_line(&self, line: usize) -> Option<u64> {
        match self.large() {
            Some(large) => large.first_line.map(|f| f + line as u64),
            None => Some(line as u64),
        }
    }

    /// The scrollbar's view of a large file: the view's top row in the
    /// whole file, and all its rows, estimated from what's counted until
    /// the count is done.
    pub(crate) fn large_rows(&self) -> Option<(f64, usize)> {
        let large = self.large()?;
        let lines = self.doc().buffer.rope().len_lines();
        let per_line = large.bytes_per_line(lines);
        let total = match large.total_lines() {
            Some(t) => t as usize,
            None => (large.size as f64 / per_line).ceil() as usize,
        }
        .max(lines);
        let first = match large.first_line {
            Some(f) => f as f64,
            None => large.start as f64 / per_line,
        };
        Some((first + self.top_row(), total))
    }

    /// Scrolls a large file to row `row` of the whole file, as a drag of
    /// the scrollbar does: in the window if it's there, or where the
    /// window there is read.
    pub(crate) fn scroll_large(&mut self, row: f64) -> bool {
        let height = self.text_height() as f64;
        let Some(large) = self.large() else {
            return false;
        };
        let lines = self.doc().buffer.rope().len_lines() as f64;
        let per_line = large.bytes_per_line(lines as usize);
        let first = match large.first_line {
            Some(f) => f as f64,
            None => large.start as f64 / per_line,
        };
        let local = row - first;
        let at_start = large.start == 0;
        let at_end = large.end == large.size;
        if (local >= 0.0 || at_start) && (local + 2.0 * height < lines || at_end) {
            self.set_top_row(local.max(0.0));
            return true;
        }
        let then = match large.total_lines() {
            Some(_) => Then::TopLine(row.max(0.0) as u64),
            None => Then::Top((row.max(0.0) * per_line) as u64),
        };
        if let Some(large) = self.large_mut() {
            large.pending = Some(then);
        }
        true
    }

    /// Moves to line `line` of a large file, column `col`: now, or once
    /// it's counted and read.
    pub(crate) fn go_to_large(&mut self, line: u64, col: usize) -> bool {
        let Some(large) = self.large_mut() else {
            return false;
        };
        large.pending = Some(Then::Line(line, col));
        true
    }

    /// Ctrl+Home and Ctrl+End in a large file: the file's ends, not the
    /// window's.
    pub(crate) fn large_end(&mut self, end: bool) -> bool {
        let Some(large) = self.large_mut() else {
            return false;
        };
        large.pending = Some(Then::Byte(if end { large.size } else { 0 }));
        true
    }

    /// Whether the current file can't be edited as it is: a large file
    /// read from disk. Says so, offering to load it.
    pub(crate) fn refuse_large_edit(&mut self) -> bool {
        // Being written: an edit would land on text that's moving.
        if self.large().is_some_and(|l| l.saving.is_some()) {
            let name = self.doc().name();
            self.notify(format!(
                "{name} is being saved: editing waits until it's written"
            ));
            return true;
        }
        let Some(large) = self.large().filter(|l| !l.editable) else {
            return false;
        };
        let name = self.doc().name();
        let needs = large.size as f64 * LOADED;
        let fits = large.free.is_none_or(|free| needs <= free as f64);
        let text = match large.free {
            Some(free) if fits => format!(
                "{name} is open read-only. Loading it to edit takes about {} of memory ({} is free).",
                size_label(needs as u64),
                size_label(free)
            ),
            Some(free) => format!(
                "{name} is open read-only. Editing needs it loaded: about {}, but only {} is free.",
                size_label(needs as u64),
                size_label(free)
            ),
            None => format!(
                "{name} is open read-only. Loading it to edit takes about {} of memory.",
                size_label(needs as u64)
            ),
        };
        self.notify(text);
        // The last button is the one lit.
        self.add_action("Keep Read-only", "large.keep");
        if fits {
            self.add_action("Load to Edit", "large.load");
        }
        true
    }

    /// The large file's commands: Load to Edit, Keep Read-only, Go to End.
    pub(crate) fn large_command(&mut self, id: &str, fx: &mut Vec<Effect>) {
        match id {
            "large.load" => {
                let Some(path) = self.doc().buffer.path.clone() else {
                    return;
                };
                if let Some(large) = self.large_mut() {
                    large.loading = true;
                    large.bar_hidden = false;
                }
                self.message = None;
                fx.push(Effect::OpenWhole(path));
            }
            "large.keep" => self.message = None,
            "large.cancel" => fx.push(Effect::CancelSave(self.doc().id)),
            "large.retry" => self.run("file.save", fx),
            "large.why" => self.large_why(),
            "large.end" => {
                if let Some(large) = self.large_mut() {
                    large.grew = None;
                }
                self.large_end(true);
            }
            _ => {}
        }
    }

    /// The bar over a large file: why it's read-only and Load to Edit, or
    /// that it's loading, or that it grew.
    pub(crate) fn large_bar(&self) -> Option<super::bars::FileBarView> {
        let doc = &self.docs[self.pane_doc()];
        let large = doc.large.as_deref()?;
        if large.loading {
            return Some(super::bars::FileBarView {
                index: LARGE_BAR,
                badge: Some("LOADING".into()),
                calm: true,
                text: format!("Loading {} to edit…", size_label(large.size)),
                progress: None,
                buttons: Vec::new(),
            });
        }
        if let Some(written) = large.saving {
            return Some(super::bars::FileBarView {
                index: LARGE_BAR,
                badge: Some("SAVING".into()),
                calm: true,
                text: format!(
                    "Saving… {} of {}. Editing waits until it's written.",
                    size_label(written),
                    size_label(large.size)
                ),
                buttons: vec![(2, "Cancel".into())],
                progress: Some((written * 1000 / large.size.max(1)) as u16),
            });
        }
        match &large.save_error {
            Some(crate::bigfile::SaveError::NoRoom { need, free }) => {
                return Some(super::bars::FileBarView {
                    index: LARGE_BAR,
                    badge: Some("NO ROOM".into()),
                    calm: false,
                    text: format!(
                        "Saving needs {} free on disk; {} is. Nothing was written.",
                        size_label(*need),
                        size_label(*free)
                    ),
                    buttons: vec![(4, "Save As…".into())],
                    progress: None,
                });
            }
            Some(crate::bigfile::SaveError::Failed(why)) => {
                return Some(super::bars::FileBarView {
                    index: LARGE_BAR,
                    badge: Some("NOT SAVED".into()),
                    calm: false,
                    text: format!("The save stopped: {why}. The file is as it was."),
                    buttons: vec![(3, "Retry".into()), (4, "Save As…".into())],
                    progress: None,
                });
            }
            _ => {}
        }
        if let Some(grew) = large.grew {
            return Some(super::bars::FileBarView {
                index: LARGE_BAR,
                badge: Some("READ-ONLY".into()),
                calm: false,
                text: format!("The file grew on disk: +{}", size_label(grew)),
                progress: None,
                buttons: vec![(1, "Go to End".into())],
            });
        }
        if large.bar_hidden {
            return None;
        }
        if large.editable {
            // About a second a GB, on an SSD.
            let secs = (large.size as f64 / 1e9).ceil().max(1.0);
            return Some(super::bars::FileBarView {
                index: LARGE_BAR,
                badge: Some("FROM DISK".into()),
                calm: true,
                text: format!(
                    "{}, edited from disk. Colours, git and plugins are off; saving writes the whole file (about {secs} s).",
                    size_label(large.size)
                ),
                progress: None,
                buttons: Vec::new(),
            });
        }
        Some(super::bars::FileBarView {
            index: LARGE_BAR,
            badge: Some("READ-ONLY".into()),
            calm: false,
            text: format!(
                "{} is too large to load. Scrolling, Find, Go to Line and copying work here; editing needs it loaded.",
                size_label(large.size)
            ),
            progress: None,
            buttons: vec![(0, "Load to Edit…".into())],
        })
    }

    /// A press on the large file's bar.
    pub(crate) fn large_bar_press(&mut self, what: &str, fx: &mut Vec<Effect>) {
        match what {
            "0" => {
                self.refuse_large_edit();
            }
            "1" => self.large_command("large.end", fx),
            "2" => self.large_command("large.cancel", fx),
            "3" => self.large_command("large.retry", fx),
            "4" => self.run("file.save_as", fx),
            _ => {
                if let Some(large) = self.large_mut() {
                    if large.save_error.is_some() {
                        large.save_error = None;
                    } else if large.grew.is_some() {
                        large.grew = None;
                    } else {
                        large.bar_hidden = true;
                    }
                }
            }
        }
    }

    /// A large file open read-only changed size on disk: a log that grew
    /// shows its new lines, and anything else is read again.
    pub(super) fn large_resized(&mut self, path: &std::path::Path, size: u64) {
        let Some(i) = self
            .docs
            .iter()
            .position(|d| d.large.is_some() && d.buffer.path.as_deref() == Some(path))
        else {
            return;
        };
        let height = self.text_height();
        let doc = &mut self.docs[i];
        let large = doc.large.as_deref_mut().expect("checked");
        // Edited: the pieces read the file as it was opened, through the
        // handle kept, so the edits stay right as a log grows.
        if !large.untouched() || doc.buffer.is_dirty() {
            if size < large.file.len() {
                let name = doc.name();
                self.notify_warning(format!(
                    "{name} was cut short on disk, while edited here: parts not edited may read wrong"
                ));
            }
            return;
        }
        if size < large.size {
            let full = large.text.path.clone();
            let (free, editable) = (large.free, large.editable);
            return self.opened_large(path.to_path_buf(), full, free, editable);
        }
        let grew = size - large.size;
        if grew == 0 {
            return;
        }
        // The file as it is now: one piece, as it's read-only.
        match crate::bigfile::FileText::open(&large.text.path) {
            Ok(text) if text.len() >= large.size => large.text = text,
            _ => return,
        }
        let (size, grew) = (large.text.len(), large.text.len() - large.size);
        // The last piece and the new ones are counted again.
        let old = large.lines.blocks.len();
        large.size = size;
        large.file = large.text.clone();
        large.lines.size = size;
        large.lines.blocks.truncate(old.saturating_sub(1));
        large
            .lines
            .blocks
            .resize(crate::bigfile::blocks(size), None);
        large.lines_job = None;
        // Counted again, over the file as it is now.
        large.found = None;
        // Shown to the end: the window reads on, and the view follows.
        let at_end = large.end + grew == size && {
            let rope = doc.buffer.rope();
            doc.scroll + height + 1 >= rope.len_lines()
        };
        let large = doc.large.as_deref_mut().expect("checked");
        if at_end {
            large.pending = Some(Then::Byte(size));
        } else {
            large.grew = Some(large.grew.unwrap_or(0) + grew);
        }
    }

    /// A large file's text, loaded whole to edit, takes the place of its
    /// window, at the same place. Whether `path` was such a file.
    pub(super) fn loaded_whole(&mut self, path: &std::path::Path, text: &ropey::Rope) -> bool {
        let Some(i) = self
            .docs
            .iter()
            .position(|d| d.large.is_some() && d.buffer.path.as_deref() == Some(path))
        else {
            return false;
        };
        let doc = &mut self.docs[i];
        let large = doc.large.take().expect("checked");
        let old = doc.buffer.rope();
        let byte_of = |pos: usize| large.start + old.char_to_byte(pos.min(old.len_chars())) as u64;
        let top = large.start + old.line_to_byte(doc.scroll.min(old.len_lines() - 1)) as u64;
        let (cursor, anchor) = (byte_of(doc.cursor), doc.anchor.map(byte_of));
        let local = |b: u64| text.byte_to_char((b as usize).min(text.len_bytes()));
        let mut new = Doc::from_rope(doc.id, text.clone(), doc.buffer.path.clone());
        new.config = doc.config.clone();
        new.cursor = local(cursor);
        new.anchor = anchor.map(local);
        new.scroll = text.char_to_line(local(top));
        new.scroll_sub = crate::doc::Sub {
            line: new.scroll,
            row: 0,
            frac: 0.0,
        };
        *doc = new;
        true
    }

    /// The status bar's position in a large file: its line once counted,
    /// or the byte until then.
    pub(crate) fn large_position(&self, line: usize, col: usize) -> Option<String> {
        let large = self.large()?;
        Some(match large.first_line {
            Some(f) => format!("Ln {}, Col {}", thousands(f + line as u64 + 1), col + 1),
            None => {
                let doc = self.doc();
                let byte = large.start + doc.buffer.rope().line_to_byte(line) as u64;
                format!("Ln ·, Col {} · byte {}", col + 1, thousands(byte))
            }
        })
    }

    /// The status bar's note of how the large file is open.
    pub(crate) fn large_mode(&self) -> Option<&'static str> {
        let large = self.large()?;
        Some(if large.editable {
            "Edited from disk"
        } else {
            "Read-only"
        })
    }

    /// What the status bar's note says when clicked.
    fn large_why(&mut self) {
        if !self.large().is_some_and(|l| l.editable) {
            self.refuse_large_edit();
            return;
        }
        let name = self.doc().name();
        self.notify(format!(
            "{name} is too large to load, so it's read from disk a part at a time and edited there. Colours, git and plugins are off for it."
        ));
    }

    /// The status bar's note of the lines being counted.
    pub(crate) fn large_counting(&self) -> Option<String> {
        let large = self.large()?;
        if let Some(written) = large.saving {
            return Some(format!("Saving {}%", written * 100 / large.size.max(1)));
        }
        (!large.lines_done()).then(|| {
            format!(
                "Counting lines {}%",
                (large.lines_progress() * 100.0) as u32
            )
        })
    }

    /// The most lines a large file is known or thought to have, for the
    /// gutter's width and Go to Line.
    pub(crate) fn large_lines(&self) -> Option<u64> {
        self.large_rows().map(|(_, total)| total as u64)
    }
}

/// Keeps a window's buffer when it has undo or redo to give: on the undo
/// stack, or with `redo`, the redo stack.
fn keep_undo(large: &mut Large, mut buffer: crate::buffer::Buffer, redo: bool) {
    if buffer.undo_steps() == 0 && buffer.redo_steps() == 0 {
        return;
    }
    let _ = buffer.take_changes();
    buffer.elsewhere = false;
    let kept = Edited {
        start: large.start,
        buffer,
    };
    if redo {
        large.redos.push(kept);
    } else {
        large.undos.push(kept);
    }
}

impl Doc {
    /// Puts the window's text into a large file's pieces when it's changed
    /// since, and says whether the file has changes anywhere.
    pub(crate) fn sync_large(&mut self) {
        let version = self.buffer.version();
        let Some(large) = self.large.as_deref_mut() else {
            return;
        };
        if large.synced != version && large.has_window() {
            let rope = self.buffer.rope().clone();
            let (text, kept) = large
                .text
                .splice_keeping(large.start, large.end, rope.clone());
            large.text = text;
            large.end = large.start + rope.len_bytes() as u64;
            large.size = large.text.len();
            large.generation += 1;
            // The search's counts of what's kept whole hold.
            if let Some(found) = &mut large.found {
                found.spliced(&kept);
            }
            large.settled = None;
            // A new edit: what was undone can't be redone.
            if !std::mem::take(&mut large.undoing) {
                large.redos.clear();
            }
        }
        large.synced = version;
        self.buffer.elsewhere = large.changed || large.undos.iter().any(|e| e.buffer.is_dirty());
    }
}

/// A size as people read it: "14.2 GB", "512 MB", "3.4 MB".
pub(crate) fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{bytes} bytes"),
        _ if value < 100.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

/// A number with commas between its thousands.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_numbers_read_well() {
        assert_eq!(size_label(14_200_000_000 / 1000 * 1000), "13.2 GB");
        assert_eq!(size_label(3_565_158), "3.4 MB");
        assert_eq!(size_label(512), "512 bytes");
        assert_eq!(thousands(203_118_450), "203,118,450");
        assert_eq!(thousands(12), "12");
    }
}
