//! Folding, as VS Code folds by indentation: a line followed by lines
//! indented further starts a block, and folding it hides those lines.
//! Folded lines take no rows on screen, so everything that maps lines to
//! rows (see `wrap.rs`) skips them.

use std::ops::Range;

use super::Editor;
use super::guides::indent;
use crate::doc::Doc;

/// The last line of the block `line` starts, if the lines after it are
/// indented further. Blank lines at the end of a block aren't part of it.
pub(crate) fn block_end(doc: &Doc, line: usize) -> Option<usize> {
    // A plugin's ranges, when it sent some, as VS Code's folding range
    // providers replace indentation.
    if let Some(folds) = &doc.layers.folds {
        let start = doc.line_start(line);
        return folds
            .iter()
            .filter(|(a, _)| *a == start)
            .map(|(_, b)| doc.line_col(*b).0)
            .filter(|&end| end > line)
            .max();
    }
    let base = indent(doc, line)?;
    let mut end = None;
    for l in line + 1..=doc.last_line() {
        match indent(doc, l) {
            None => continue,
            Some(i) if i > base => end = Some(l),
            _ => break,
        }
    }
    end
}

/// The lines hidden by a document's folds, in order, without overlaps.
pub(crate) fn hidden(doc: &Doc) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for &f in &doc.folds {
        let line = doc.line_col(f).0;
        let Some(end) = block_end(doc, line) else {
            continue;
        };
        match ranges.last_mut() {
            Some(last) if line < last.end => last.end = last.end.max(end + 1),
            _ => ranges.push(line + 1..end + 1),
        }
    }
    ranges
}

impl Editor {
    /// The lines folded away in the document drawn, cached per version.
    pub(crate) fn hidden_lines(&self) -> Vec<Range<usize>> {
        let doc = self.doc();
        if doc.folds.is_empty() {
            return Vec::new();
        }
        let key = (doc.id, doc.buffer.version(), doc.folds.clone());
        let mut cache = self.fold_cache.borrow_mut();
        if let Some((k, ranges)) = cache.as_ref()
            && *k == key
        {
            return ranges.clone();
        }
        let ranges = hidden(doc);
        *cache = Some((key, ranges.clone()));
        ranges
    }

    pub(crate) fn is_hidden(&self, line: usize) -> bool {
        self.hidden_lines().iter().any(|r| r.contains(&line))
    }

    /// The block around the cursor: the nearest line at or above it whose
    /// block reaches the cursor's line.
    fn block_at_cursor(&self) -> Option<usize> {
        let doc = self.doc();
        let cur = doc.line_col(doc.cursor).0;
        (0..=cur)
            .rev()
            .take(5000)
            .find(|&l| block_end(doc, l).is_some_and(|end| end >= cur))
    }

    /// Folds the block around the cursor, or the one around that if it is
    /// folded already, and puts the cursor on its first line.
    pub(crate) fn fold(&mut self) {
        let Some(mut line) = self.block_at_cursor() else {
            return;
        };
        let doc = self.doc();
        while doc.folds.contains(&doc.line_start(line)) {
            // Already folded: fold the block outside it.
            let outer = (0..line)
                .rev()
                .find(|&l| block_end(doc, l).is_some_and(|end| end >= line));
            match outer {
                Some(o) => line = o,
                None => return,
            }
        }
        let start = doc.line_start(line);
        let doc = self.doc_mut();
        doc.folds.push(start);
        doc.folds.sort_unstable();
        if doc.line_col(doc.cursor).0 != line {
            doc.cursor = start + doc.buffer.line_len(line);
            doc.anchor = None;
        }
    }

    /// Unfolds the block the cursor's line starts or is folded inside.
    pub(crate) fn unfold(&mut self) {
        let doc = self.doc();
        let cur = doc.line_col(doc.cursor).0;
        let keep: Vec<usize> = doc
            .folds
            .iter()
            .copied()
            .filter(|&f| {
                let line = doc.line_col(f).0;
                let reaches = block_end(doc, line).is_some_and(|end| end >= cur);
                !(line == cur || (line < cur && reaches))
            })
            .collect();
        self.doc_mut().folds = keep;
    }

    /// Folds or unfolds the block a line starts, as a click on its arrow
    /// does.
    pub(crate) fn toggle_fold(&mut self, line: usize) {
        let doc = self.doc();
        if block_end(doc, line).is_none() {
            return;
        }
        let start = doc.line_start(line);
        let doc = self.doc_mut();
        if doc.folds.contains(&start) {
            doc.folds.retain(|&f| f != start);
        } else {
            doc.folds.push(start);
            doc.folds.sort_unstable();
            let cur = doc.line_col(doc.cursor).0;
            if cur > line && block_end(doc, line).is_some_and(|end| cur <= end) {
                doc.cursor = start + doc.buffer.line_len(line);
                doc.anchor = None;
            }
        }
    }

    /// Folds every block, outermost ones included.
    pub(crate) fn fold_all(&mut self) {
        let doc = self.doc();
        let folds: Vec<usize> = (0..=doc.last_line())
            .filter(|&l| block_end(doc, l).is_some())
            .map(|l| doc.line_start(l))
            .collect();
        self.doc_mut().folds = folds;
        // A cursor folded away goes to the first line of the block
        // outermost around it.
        let cur = self.doc().line_col(self.doc().cursor).0;
        if let Some(range) = self.hidden_lines().into_iter().find(|r| r.contains(&cur)) {
            let line = range.start - 1;
            let doc = self.doc_mut();
            doc.cursor = doc.line_start(line) + doc.buffer.line_len(line);
            doc.anchor = None;
        }
    }

    pub(crate) fn unfold_all(&mut self) {
        self.doc_mut().folds.clear();
    }

    /// Keeps every view's top line out of folded blocks: a view scrolled
    /// into one starts at the block's first line instead. Folds belong to
    /// the document, so a fold in one group can hide another's top line.
    pub(crate) fn scroll_out_of_folds(&mut self) {
        for i in 0..self.docs.len() {
            if self.docs[i].folds.is_empty() {
                continue;
            }
            let ranges = hidden(&self.docs[i]);
            let out = |scroll: usize| {
                ranges
                    .iter()
                    .find(|r| r.contains(&scroll))
                    .map_or(scroll, |r| r.start - 1)
            };
            let doc = &mut self.docs[i];
            doc.scroll = out(doc.scroll);
            let id = doc.id;
            for group in &mut self.groups {
                if let Some(view) = group.saved.get_mut(&id) {
                    view.scroll = out(view.scroll);
                }
            }
        }
    }

    /// Unfolds whatever hides the cursor, after something moved it there,
    /// such as Go to Line or a search.
    pub(crate) fn reveal_cursor(&mut self) {
        if self.doc().folds.is_empty() {
            return;
        }
        let doc = self.doc();
        let cur = doc.line_col(doc.cursor).0;
        let keep: Vec<usize> = doc
            .folds
            .iter()
            .copied()
            .filter(|&f| {
                let line = doc.line_col(f).0;
                !(line < cur && block_end(doc, line).is_some_and(|end| end >= cur))
            })
            .collect();
        self.doc_mut().folds = keep;
    }
}
