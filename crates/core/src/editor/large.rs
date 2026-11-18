//! Files too large to load, shown read-only from disk (see
//! [`crate::bigfile`]): which window of the file to hold as the view
//! moves, where to go once one comes, the lines and a search counted in
//! the background, and what the bar, the gutter, the status bar and the
//! scrollbar say. Editing asks to load the file first.

use super::*;
use crate::bigfile::{Large, Then, WINDOW};

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
    pub(super) fn opened_large(&mut self, path: PathBuf, full: PathBuf, free: Option<u64>) {
        // Kept open: the pieces read the file as it was opened.
        let text = match crate::bigfile::FileText::open(&full) {
            Ok(text) => text,
            Err(err) => return self.error(format!("Can't open {}: {err}", path.display())),
        };
        let size = text.len();
        let large = Box::new(Large::new(text, free));
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
                kind: crate::work::Kind::Lines(large.text.clone()),
                text: Default::default(),
            }));
        }
        match matcher {
            Some(matcher) if large.found.as_ref().is_none_or(|f| f.matcher != matcher) => {
                next += 1;
                large.found = Some(crate::bigfile::FileFound {
                    matcher: matcher.clone(),
                    counts: vec![None; large.lines.len()],
                    job: Some(next),
                });
                fx.push(Effect::Work(crate::work::Job {
                    id: next,
                    kind: crate::work::Kind::FileCount {
                        text: large.text.clone(),
                        matcher,
                    },
                    text: Default::default(),
                }));
            }
            Some(_) => {}
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
                counts: found.counts.clone(),
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
        // The count has reached the window: read it again to number it.
        let numbered =
            large.first_line.is_none() && large.from(top_byte).anchor.is_some() && large.start > 0;
        if (near_top || near_end || numbered) && large.settled != Some(top_byte) {
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
        if here {
            self.large_mut().expect("checked").pending = None;
            return self.land(then);
        }
        let around = match &then {
            Then::Line(line, _) | Then::TopLine(line) => match large.block_of_line(*line) {
                // A little into the piece, so its start is in the window.
                Some((at, _)) => at + WINDOW / 2 - WINDOW / 8,
                None => {
                    // Not counted that far yet: it waits.
                    self.large_mut().expect("checked").pending = Some(then);
                    return;
                }
            },
            Then::Byte(b) | Then::Top(b) | Then::Match(b) => *b,
            Then::Stay => return,
        };
        self.large_mut().expect("checked").pending = None;
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
    /// then goes where it was asked for.
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
        let old = doc.buffer.rope();
        let byte_of = |pos: usize| large.start + old.char_to_byte(pos.min(old.len_chars())) as u64;
        let top = large.start + old.line_to_byte(doc.scroll.min(old.len_lines() - 1)) as u64;
        let cursor = byte_of(doc.cursor);
        let anchor = doc.anchor.map(byte_of);
        // The window's first line, from the old one's where they overlap.
        let first = w.first_line.or_else(|| {
            let f = large.first_line?;
            if w.start >= large.start && w.start <= large.end {
                Some(f + old.byte_to_line((w.start - large.start) as usize) as u64)
            } else if w.start < large.start && large.start <= w.end {
                f.checked_sub(w.text.byte_to_line((large.start - w.start) as usize) as u64)
            } else {
                None
            }
        });
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
        large.start = w.start;
        large.end = w.end;
        large.first_line = first;
        large.settled = Some(top);
        if w.size != large.size {
            large.size = w.size;
        }
        new.large = Some(large);
        *doc = new;
        if i == self.current && then != Then::Stay {
            self.land(then);
        }
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
        let Some(large) = self.large() else {
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
            "large.why" => {
                self.refuse_large_edit();
            }
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
                text: format!("Loading {} to edit…", size_label(large.size)),
                buttons: Vec::new(),
            });
        }
        if let Some(grew) = large.grew {
            return Some(super::bars::FileBarView {
                index: LARGE_BAR,
                badge: Some("READ-ONLY".into()),
                text: format!("The file grew on disk: +{}", size_label(grew)),
                buttons: vec![(1, "Go to End".into())],
            });
        }
        if large.bar_hidden {
            return None;
        }
        Some(super::bars::FileBarView {
            index: LARGE_BAR,
            badge: Some("READ-ONLY".into()),
            text: format!(
                "{} is too large to load. Scrolling, Find, Go to Line and copying work here; editing needs it loaded.",
                size_label(large.size)
            ),
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
            _ => {
                if let Some(large) = self.large_mut() {
                    if large.grew.is_some() {
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
        if size < large.size {
            let full = large.text.path.clone();
            let free = large.free;
            return self.opened_large(path.to_path_buf(), full, free);
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
        let old = large.lines.len();
        large.size = size;
        large.lines.truncate(old.saturating_sub(1));
        large.lines.resize(crate::bigfile::blocks(size), None);
        large.lines_job = None;
        if let Some(found) = &mut large.found {
            found.counts.truncate(old.saturating_sub(1));
            found.counts.resize(crate::bigfile::blocks(size), None);
            found.job = None;
            found.matcher = crate::search::Matcher::exact("\u{0}stale");
        }
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

    /// The status bar's note of the lines being counted.
    pub(crate) fn large_counting(&self) -> Option<String> {
        let large = self.large()?;
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
