//! Word wrap, as VS Code's View › Word Wrap does it: long lines break at
//! the last space that fits, or anywhere in a word too long for a row, and
//! carry on indented as far as the line is.
//!
//! Lines map to screen rows here, wrapped or not, once any line takes other
//! than one row: folded lines take none.

use super::Editor;
use crate::doc::advance;

/// One screen row of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    /// The line's first character on this row.
    pub start: usize,
    /// That character's display column in the whole line.
    pub col: usize,
    /// Blank cells before it, so continued rows line up with the indent.
    pub indent: usize,
}

const ONE_ROW: Row = Row {
    start: 0,
    col: 0,
    indent: 0,
};

/// Breaks a line into rows of at most `width` cells. A space that doesn't
/// fit hangs off the end of its row, as in VS Code.
pub(crate) fn wrap(chars: &[char], width: usize) -> Vec<Row> {
    let mut rows = vec![ONE_ROW];
    if width < 2 {
        return rows;
    }
    let mut d = 0;
    let lead: usize = chars
        .iter()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .fold(0, |d, &c| d + advance(c, d));
    let indent = if lead < width / 2 { lead } else { 0 };
    // Where the row may break: after the last space, if any.
    let mut opening: Option<(usize, usize)> = None;
    for (i, &c) in chars.iter().enumerate() {
        let w = advance(c, d);
        let row = *rows.last().unwrap();
        let room = width - row.indent;
        if d + w - row.col > room && !c.is_whitespace() && i > row.start {
            let (start, col) = opening.filter(|&(s, _)| s > row.start).unwrap_or((i, d));
            rows.push(Row { start, col, indent });
            opening = None;
        }
        d += w;
        if c.is_whitespace() && i >= lead.min(chars.len()) {
            opening = Some((i + 1, d));
        }
    }
    rows
}

impl Editor {
    /// Whether long lines wrap: when it's on, and not in a side-by-side
    /// review.
    pub(crate) fn wrapping(&self) -> bool {
        self.wrap && self.split_rows().is_none()
    }

    /// Whether lines can take other than one row each: when they wrap, or
    /// some are folded away.
    pub(crate) fn row_based(&self) -> bool {
        self.wrapping() || !self.view().folds.is_empty()
    }

    /// The rows a line takes on screen: none if it's folded away.
    pub(crate) fn line_rows(&self, line: usize) -> Vec<Row> {
        if self.is_hidden(line) {
            return Vec::new();
        }
        if !self.wrapping() {
            return vec![ONE_ROW];
        }
        wrap(&self.doc().line_chars(line), self.text_columns())
    }

    /// A line's rows, as if it showed.
    fn shown_rows(&self, line: usize) -> Vec<Row> {
        let rows = self.line_rows(line);
        if rows.is_empty() { vec![ONE_ROW] } else { rows }
    }

    /// Which of a line's rows a character is on.
    pub(crate) fn row_of(rows: &[Row], i: usize) -> usize {
        rows.iter().rposition(|r| r.start <= i).unwrap_or(0)
    }

    /// The screen row, counted from the top of the view, where a line's
    /// row `r` is drawn. Lines above the view give `None`.
    pub(crate) fn screen_row(&self, line: usize, r: usize) -> Option<usize> {
        let v = self.view();
        if line < v.scroll || self.is_hidden(line) {
            return None;
        }
        if !self.row_based() {
            return Some(line - v.scroll);
        }
        let height = self.text_height();
        let mut rows = 0;
        for l in v.scroll..line {
            rows += self.line_rows(l).len();
            // No need to count further than the view.
            if rows > height {
                return Some(rows + r);
            }
        }
        Some(rows + r)
    }

    /// Where a character is, as its line's row and its cell from the left
    /// of the text.
    pub(crate) fn visual(&self, pos: usize) -> (usize, usize, usize) {
        let doc = self.doc();
        let (line, col) = doc.line_col(pos);
        let rows = self.shown_rows(line);
        let r = Self::row_of(&rows, col);
        let x = rows[r].indent + doc.display_col(line, col) - rows[r].col;
        (line, r, x)
    }

    /// The position at cell `x` of a line's row `r`. Past the end of a
    /// row gives its last character, or the line's end on its last row.
    pub(crate) fn pos_in_row(&self, line: usize, r: usize, x: usize) -> usize {
        let doc = self.doc();
        let rows = self.shown_rows(line);
        let r = r.min(rows.len() - 1);
        let row = rows[r];
        let target = row.col + x.saturating_sub(row.indent);
        let col = doc.col_at_display(line, target).max(row.start);
        let col = match rows.get(r + 1) {
            Some(next) => col.min(next.start.saturating_sub(1).max(row.start)),
            None => col,
        };
        doc.line_start(line) + col
    }

    /// Moves the cursor `delta` rows up or down, keeping its column, when
    /// lines wrap.
    pub(crate) fn move_rows(&mut self, delta: isize) {
        let cursor = self.doc().cursor;
        let (mut line, mut r, x) = self.visual(cursor);
        let goal = *self.doc_mut().goal.get_or_insert(x);
        let last = self.doc().last_line();
        for _ in 0..delta.unsigned_abs() {
            if delta < 0 {
                if r > 0 {
                    r -= 1;
                    continue;
                }
                // The line above, past any folded away.
                let Some(above) = (0..line).rev().find(|&l| !self.line_rows(l).is_empty()) else {
                    break;
                };
                line = above;
                r = self.line_rows(line).len() - 1;
            } else if r + 1 < self.line_rows(line).len() {
                r += 1;
            } else {
                let Some(below) = (line + 1..=last).find(|&l| !self.line_rows(l).is_empty()) else {
                    break;
                };
                line = below;
                r = 0;
            }
        }
        let pos = self.pos_in_row(line, r, goal);
        self.doc_mut().cursor = pos;
    }

    /// Scrolls so the cursor's row shows, when lines wrap.
    pub(crate) fn scroll_to_cursor_row(&mut self) {
        let height = self.text_height().max(1);
        let cursor = self.doc().cursor;
        let (line, r, _) = self.visual(cursor);
        let wrapping = self.wrapping();
        let doc = self.doc_mut();
        if wrapping {
            doc.hscroll = 0;
        }
        if line < doc.scroll {
            doc.scroll = line;
            return;
        }
        while self.doc().scroll < line && self.screen_row(line, r).is_some_and(|row| row >= height)
        {
            self.doc_mut().scroll += 1;
        }
    }

    /// The line and its row drawn at a screen row of the view, or the line
    /// past the end.
    pub(crate) fn line_at_row(&self, row: usize) -> (usize, usize) {
        let doc = self.doc();
        let v = self.view();
        let mut seen = 0;
        let mut line = v.scroll;
        while line <= doc.last_line() {
            let n = self.line_rows(line).len();
            if row < seen + n {
                return (line, row - seen);
            }
            seen += n;
            line += 1;
        }
        (doc.last_line() + 1, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts(text: &str, width: usize) -> Vec<usize> {
        let chars: Vec<char> = text.chars().collect();
        wrap(&chars, width).iter().map(|r| r.start).collect()
    }

    #[test]
    fn breaks_after_the_last_space_that_fits() {
        assert_eq!(starts("one two three four", 10), [0, 8]);
        assert_eq!(starts("one two three four", 7), [0, 8, 14]);
    }

    #[test]
    fn breaks_a_word_too_long_for_a_row() {
        assert_eq!(starts("abcdefghijkl", 5), [0, 5, 10]);
    }

    #[test]
    fn short_lines_take_one_row() {
        assert_eq!(starts("short", 10), [0]);
        assert_eq!(starts("", 10), [0]);
    }

    #[test]
    fn continued_rows_keep_the_indent() {
        let chars: Vec<char> = "    aaaa bbbb cccc".chars().collect();
        let rows = wrap(&chars, 14);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].indent, 4);
        assert_eq!(rows[1].start, 14);
    }
}
