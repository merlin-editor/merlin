//! The code, drawn in pixels from the core's [`TextView`]s: each line
//! shaped whole, so a font's ligatures join; selections and matches as
//! rounded shapes; thin carets at every cursor; guides as hairlines; and the
//! gutter's numbers, signs and fold arrows.
//!
//! Columns stay the core's: column `c` is at `c` times the font's advance,
//! so clicks and the text agree.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use cosmic_text::{CacheKey, Shaping, Wrap};
use merlin_core::TextView;
use merlin_core::lang::Severity;
use merlin_core::style::{Cell, Style, Surface};

use super::canvas::{Canvas, Face, Rect, Text, glyph_bitmap};
use super::grid::{Fonts, attrs};
use super::scene::{BitmapId, Prim};
use crate::settings::CursorStyle;
use crate::theme::{self, Look, Palette};

/// Shaped lines kept, most for the lines on screen as they scroll.
const SHAPED_LIMIT: usize = 4096;

/// A glyph of a shaped line: the character it starts at, and its picture.
#[derive(Clone, Copy)]
pub(super) struct Placed {
    index: usize,
    key: CacheKey,
    /// Where the shaper put it, from the character's column, in pixels.
    dx: i32,
    dy: i32,
}

#[derive(Clone, Copy)]
pub(super) struct Picture {
    bitmap: BitmapId,
    left: i32,
    top: i32,
}

/// How the carets look in a frame: their style, and how far on they are
/// in their blink, from 0 to 1, as opacity or, expanding, as height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub style: CursorStyle,
    pub on: f32,
    pub expand: bool,
}

/// What the painter keeps between frames.
#[derive(Default)]
pub struct Shapes {
    lines: HashMap<u64, Vec<Placed>>,
    pictures: HashMap<CacheKey, Option<Picture>>,
    generation: u64,
}

/// Where a character is drawn: its screen row in the line, column, and
/// width in columns.
struct Spot {
    row: usize,
    col: isize,
    width: usize,
}

impl Fonts {
    /// Draws `view` in its group's text area, physical (`ox`, `oy`) to
    /// (`ox` + `w`, `oy` + `h`), with the view's grid from its top left.
    /// `caret` is how the carets look and how far on they are in their
    /// blink.
    /// `glide` moves the lines down by that many pixels, while scrolling
    /// eases to where the view is; the part of the top row scrolled off
    /// moves them up. `text` draws what's written in the interface's font,
    /// such as problems after their lines.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_code(
        &mut self,
        view: &TextView,
        canvas: &mut Canvas,
        text: &mut Text,
        [ox, oy, w, h]: [f32; 4],
        p: &Palette,
        caret: Caret,
        glide: f32,
    ) {
        let (cw, ch) = (self.cell_w as f32, self.cell_h as f32);
        let glide = glide - view.top_frac * ch;
        let first_prim = canvas.scene.prims.len();
        let s = canvas.scale;
        let area = view.area;
        let gutter_x = ox + area.x as f32 * cw;
        let text_x = gutter_x + view.gutter as f32 * cw;
        let right = (text_x + view.columns as f32 * cw).min(ox + w);
        let top = oy + area.y as f32 * ch;
        // The rows the view has, and the part of one more the area fits.
        let bottom = (top + (area.h as f32 + 1.0) * ch).min(oy + h);
        let clip = [text_x as i32, top as i32, right as i32, bottom as i32];
        canvas.fill_px(
            gutter_x as usize,
            top as usize,
            (right - gutter_x) as usize,
            (bottom - top) as usize,
            p.island,
        );
        let look = |style: Style, syntax, mark, tint| {
            theme::cell(
                p,
                &Cell {
                    ch: ' ',
                    style,
                    syntax,
                    mark,
                    surface: Surface::Editor,
                    tint,
                },
            )
        };
        for l in &view.lines {
            let y0 = top + l.row as f32 * ch + glide;
            if y0 + l.shown as f32 * ch <= top || y0 >= bottom {
                continue;
            }
            let rows = l.shown as f32;
            // Where each character goes, as the core places it, and each
            // inlay hint before the character it's with.
            let mut spots = Vec::with_capacity(l.chars.len());
            let mut chips = Vec::with_capacity(l.inlays.len());
            let (mut d, mut r) = (0usize, 0usize);
            for i in 0..=l.chars.len() {
                while r + 1 < l.rows.len() && i >= l.rows[r + 1].start {
                    r += 1;
                }
                let row = &l.rows[r];
                for inlay in l.inlays.iter().filter(|h| h.col == i) {
                    let col = (row.indent + d - row.col) as isize - view.hscroll as isize;
                    chips.push((
                        Spot {
                            row: r,
                            col,
                            width: inlay.cols,
                        },
                        &inlay.label,
                    ));
                    d += inlay.cols;
                }
                let Some(&c) = l.chars.get(i) else { break };
                let width = advance(c, d);
                let col = (row.indent + d - row.col) as isize - view.hscroll as isize;
                spots.push(Spot { row: r, col, width });
                d += width;
            }
            let end_col = l.rows.last().map_or(0, |row| row.indent + d - row.col) as isize
                - view.hscroll as isize;
            let x_of = |col: isize| text_x + col as f32 * cw;
            let row_y = |row: usize| y0 + row as f32 * ch;

            // The lines of the commit whose card is open get a soft tint.
            if l.blame_cell.as_ref().is_some_and(|c| c.hovered) && l.base == Style::Normal {
                let tint = theme::mix(p.accent, p.island, 0.08);
                quad(canvas, [text_x, y0, right, y0 + rows * ch], 0.0, tint);
            }
            // The line's own band: the current line, or a review's colour.
            if l.base != Style::Normal {
                let bg = if l.base == Style::CurrentLine {
                    p.current_line
                } else {
                    look(l.base, Default::default(), None, None)
                        .bg
                        .unwrap_or(p.island)
                };
                quad(canvas, [text_x, y0, right, y0 + rows * ch], 0.0, bg);
            }
            // A line with an error or warning written after it is tinted.
            if let Some((severity, _)) = &l.note
                && *severity <= Severity::Warning
            {
                canvas.scene.prims.push(Prim::Quad {
                    rect: [text_x, y0, right, y0 + rows * ch],
                    radius: 0.0,
                    fill: note_colour(p, *severity),
                    alpha: 18,
                    border: 0,
                    border_w: 0.0,
                });
            }

            // The line a run is paused on: amber, or red on an exception,
            // with a bar at its left.
            if let Some(exception) = l.paused {
                let colour = if exception { p.error } else { p.warning };
                canvas.scene.prims.push(Prim::Quad {
                    rect: [text_x, y0, right, y0 + rows * ch],
                    radius: 0.0,
                    fill: colour,
                    alpha: 34,
                    border: 0,
                    border_w: 0.0,
                });
                quad(canvas, [text_x, y0, text_x + 3.0 * s, y0 + rows * ch], 0.0, colour);
            }

            // Selections and matches: a rounded box for each run of one
            // style on a row.
            let mut i = 0;
            while i < l.chars.len() {
                let style = l.styles[i];
                let marked =
                    !matches!(style, Style::Normal | Style::CurrentLine) && style != l.base;
                if !marked || spots[i].row >= l.shown as usize {
                    i += 1;
                    continue;
                }
                let row = spots[i].row;
                let mut j = i;
                while j < l.chars.len() && l.styles[j] == style && spots[j].row == row {
                    j += 1;
                }
                let x0 = x_of(spots[i].col);
                let mut x1 = x_of(spots[j - 1].col + spots[j - 1].width as isize);
                // A selection running on past the line's end.
                if style == Style::Selection && j == l.chars.len() && l.eol_selected {
                    x1 += cw * 0.6;
                }
                let bg = look(style, Default::default(), None, None)
                    .bg
                    .unwrap_or(p.selection);
                let y = row_y(row);
                let radius = if style == Style::Selection { 3.0 } else { 2.0 } * s;
                clipped_quad(canvas, [x0, y, x1, y + ch], radius, bg, text_x, right);
                i = j;
            }
            if l.eol_selected
                && l.chars
                    .last()
                    .is_none_or(|_| l.styles.last() != Some(&Style::Selection))
            {
                let row = l.rows.len() - 1;
                if row < l.shown as usize {
                    let x0 = x_of(end_col);
                    let y = row_y(row);
                    clipped_quad(
                        canvas,
                        [x0, y, x0 + cw * 0.6, y + ch],
                        3.0 * s,
                        p.selection,
                        text_x,
                        right,
                    );
                }
            }

            // Indent guides: hairlines where the core puts them.
            for (i, guide) in l.guides.iter().enumerate() {
                let Some(active) = *guide else { continue };
                let spot = &spots[i];
                if spot.col < 0 || spot.row >= l.shown as usize {
                    continue;
                }
                let x = x_of(spot.col).round() + 0.5;
                if x >= right {
                    continue;
                }
                let colour = if active { p.guide_active } else { p.guide };
                let y = row_y(spot.row);
                canvas.scene.prims.push(Prim::Quad {
                    rect: [x, y, x + s.max(1.0), y + ch],
                    radius: 0.0,
                    fill: colour,
                    alpha: 255,
                    border: 0,
                    border_w: 0.0,
                });
            }

            // The text, shaped whole.
            let looks: Vec<Look> = (0..l.chars.len())
                .map(|i| look(l.styles[i], l.syntax[i], l.marks[i], l.brackets[i]))
                .collect();
            let placed = self.shape(&l.chars, &looks);
            for g in placed.iter() {
                let i = g.index;
                let spot = &spots[i];
                if l.guides[i].is_some() || spot.row >= l.shown as usize {
                    continue;
                }
                let x = x_of(spot.col);
                if x + spot.width as f32 * cw <= text_x || x >= right {
                    continue;
                }
                let Some(pic) = self.picture(g.key, canvas) else {
                    continue;
                };
                let y = row_y(spot.row);
                canvas.scene.prims.push(Prim::Glyph {
                    bitmap: pic.bitmap,
                    pos: [
                        x.round() as i32 + g.dx + pic.left,
                        y as i32 + self.baseline + g.dy - pic.top,
                    ],
                    colour: looks[i].fg.unwrap_or(p.text),
                    clip: [
                        clip[0],
                        (y as i32).max(clip[1]),
                        clip[2],
                        ((y + ch) as i32).min(clip[3]),
                    ],
                });
            }

            // Problems: a wave under each run of marked characters.
            let mut i = 0;
            while i < l.chars.len() {
                let lk = &looks[i];
                if !lk.underline || spots[i].row >= l.shown as usize {
                    i += 1;
                    continue;
                }
                let row = spots[i].row;
                let mut j = i;
                while j < l.chars.len()
                    && looks[j].underline == lk.underline
                    && looks[j].underline_colour == lk.underline_colour
                    && spots[j].row == row
                {
                    j += 1;
                }
                let colour = lk.underline_colour.or(lk.fg).unwrap_or(p.error);
                let x0 = x_of(spots[i].col).max(text_x);
                let x1 = x_of(spots[j - 1].col + spots[j - 1].width as isize).min(right);
                let base = row_y(row) + self.baseline as f32 + 3.0 * s;
                underline(canvas, x0, x1, base, s, colour, lk.curly);
                i = j;
            }

            // Colours written in the code: a bar in that colour under each,
            // split where it wraps.
            for &(start, len, rgba) in &l.swatches {
                let mut i = start;
                while i < start + len && i < l.chars.len() {
                    let row = spots[i].row;
                    let mut j = i;
                    while j < start + len && j < l.chars.len() && spots[j].row == row {
                        j += 1;
                    }
                    if row < l.shown as usize {
                        let x0 = x_of(spots[i].col).max(text_x);
                        let x1 = x_of(spots[j - 1].col + spots[j - 1].width as isize).min(right);
                        let y = row_y(row) + self.baseline as f32 + 2.0 * s;
                        if x1 > x0 {
                            // A colour close to the code's own background
                            // gets an edge, so it doesn't vanish.
                            let faint = near(rgba >> 8, p.island);
                            canvas.scene.prims.push(Prim::Quad {
                                rect: [x0, y, x1, y + 3.0 * s],
                                radius: 1.5 * s,
                                fill: rgba >> 8,
                                // See-through colours still show.
                                alpha: (rgba & 0xff).max(96) as u8,
                                border: p.muted,
                                border_w: if faint { s } else { 0.0 },
                            });
                        }
                    }
                    i = j;
                }
            }

            // A folded block: a small pill after its first line.
            if l.folded && end_col >= 0 {
                let x = x_of(end_col) + cw * 0.5;
                let y = row_y(l.rows.len() - 1);
                if x + cw * 2.0 < right {
                    quad(
                        canvas,
                        [x, y + ch * 0.25, x + cw * 2.0, y + ch * 0.75],
                        ch * 0.25,
                        p.chip,
                    );
                    for k in 0..3 {
                        let cx = x + cw * (0.55 + 0.45 * k as f32);
                        quad(
                            canvas,
                            [cx - s, y + ch * 0.5 - s, cx + s, y + ch * 0.5 + s],
                            s,
                            p.muted,
                        );
                    }
                }
            }

            // Inlay hints: quiet chips, half a column of room either side
            // of their text, which keeps to the code's columns.
            for (spot, label) in &chips {
                if spot.row >= l.shown as usize {
                    continue;
                }
                let x0 = x_of(spot.col) + cw * 0.2;
                let x1 = x_of(spot.col + spot.width as isize) - cw * 0.2;
                if x1 <= text_x || x0 >= right {
                    continue;
                }
                let y = row_y(spot.row);
                let pad = (ch * 0.1).round();
                clipped_quad(
                    canvas,
                    [x0, y + pad, x1, y + ch - pad],
                    3.0 * s,
                    p.raised,
                    text_x,
                    right,
                );
                let mut x = x_of(spot.col) + cw * 0.5;
                for c in label.chars() {
                    let w = merlin_core::style::char_width(c).max(1);
                    if x >= text_x && x + w as f32 * cw <= right {
                        self.wide_glyph(c, canvas, x, y, w, p.muted);
                    }
                    x += w as f32 * cw;
                }
            }

            // The line's worst problem, written a little after its end.
            if let Some((severity, message)) = &l.note {
                let fold = if l.folded { 3.0 } else { 0.0 };
                let x = x_of(end_col.max(0)) + cw * (3.0 + fold);
                let y = row_y(l.rows.len() - 1);
                if (l.rows.len() as u16) <= l.shown && x < right - cw * 4.0 {
                    let ts = text.scale();
                    let size = (ch / ts * 0.6).round();
                    let clip = Rect::new(x / ts, y / ts, (right - x) / ts, ch / ts);
                    text.draw_in_row(
                        canvas,
                        message,
                        Face::ui(size),
                        x / ts,
                        y / ts,
                        ch / ts,
                        note_colour(p, *severity),
                        clip,
                    );
                }
            }

            // Values while paused, in a quiet chip a little after the end.
            if let Some(values) = &l.values {
                let x = x_of(end_col.max(0)) + cw * 3.0;
                let y = row_y(l.rows.len() - 1);
                if (l.rows.len() as u16) <= l.shown && x < right - cw * 6.0 {
                    let ts = text.scale();
                    let size = (ch / ts * 0.58).round();
                    let face = Face::ui(size);
                    let fitted = text.fit(values, face, (right - x) / ts - 16.0);
                    let w = text.width(&fitted, face) * ts + 12.0 * s;
                    let pad = (ch * 0.16).round();
                    quad(
                        canvas,
                        [x, y + pad, (x + w).min(right), y + ch - pad],
                        4.0 * s,
                        theme::mix(p.info, p.island, 0.12),
                    );
                    let clip = Rect::new(x / ts, y / ts, (right - x) / ts, ch / ts);
                    text.draw_in_row(
                        canvas,
                        &fitted,
                        face,
                        x / ts + 6.0,
                        y / ts,
                        ch / ts,
                        p.info,
                        clip,
                    );
                }
            }

            // Who last changed the cursor's line, faint, a little after it.
            if let Some(blame) = &l.blame {
                let x = x_of(end_col.max(0)) + cw * 4.0;
                let y = row_y(l.rows.len() - 1);
                if (l.rows.len() as u16) <= l.shown && x < right - cw * 4.0 {
                    let ts = text.scale();
                    let size = (ch / ts * 0.55).round();
                    let clip = Rect::new(x / ts, y / ts, (right - x) / ts, ch / ts);
                    let face = Face::ui(size);
                    let fitted = text.fit(blame, face, (right - x) / ts - 8.0);
                    text.draw_in_row(
                        canvas,
                        &fitted,
                        face,
                        x / ts,
                        y / ts,
                        ch / ts,
                        p.faint,
                        clip,
                    );
                }
            }

            // The blame column, at the gutter's left: a bar coloured by the
            // commit's age, and who, when and why on a run's first line.
            if let Some(cell) = &l.blame_cell {
                let width = view.blame_cols as f32 * cw;
                let band = [gutter_x, y0, gutter_x + width - cw, y0 + rows * ch];
                if cell.hovered {
                    let tint = theme::mix(p.accent, p.island, 0.08);
                    quad(canvas, [gutter_x, y0, band[2], y0 + rows * ch], 0.0, tint);
                }
                if cell.first && l.line > 0 {
                    quad(canvas, [band[0], y0, band[2], y0 + s], 0.0, p.line);
                }
                let colour = if cell.committed {
                    theme::mix(p.accent, p.faint, 0.25 + 0.75 * cell.age)
                } else {
                    p.accent
                };
                let bar = (3.0 * s).round();
                quad(
                    canvas,
                    [
                        gutter_x,
                        y0 + 2.0 * s,
                        gutter_x + bar,
                        y0 + rows * ch - 2.0 * s,
                    ],
                    s,
                    colour,
                );
                if !cell.text.is_empty() {
                    let ts = text.scale();
                    let size = (ch / ts * 0.55).round();
                    let face = Face::ui(size);
                    let x = (gutter_x + bar + cw * 0.8) / ts;
                    let room = (band[2] - cw * 0.5) / ts - x;
                    let fitted = text.fit(&cell.text, face, room);
                    let clip = Rect::new(x, y0 / ts, room.max(0.0), ch / ts);
                    let fg = if cell.committed { p.muted } else { p.accent };
                    text.draw_in_row(canvas, &fitted, face, x, y0 / ts, ch / ts, fg, clip);
                }
            }

            // The gutter: the number, a sign, and the fold arrow. Signs come
            // after the blame column.
            let gutter_x = gutter_x + view.blame_cols as f32 * cw;
            let number_look = look(l.number_style, Default::default(), None, None);
            let digits_x = ox + area.x as f32 * cw + view.signs as usize as f32 * cw;
            for (k, c) in l.number.chars().enumerate() {
                if c == ' ' {
                    continue;
                }
                let x = digits_x + k as f32 * cw;
                self.cell_glyph(c, canvas, x, y0, number_look.fg.unwrap_or(p.faint));
            }
            if let Some((sign, style)) = l.sign {
                let colour = look(style, Default::default(), None, None)
                    .fg
                    .unwrap_or(p.faint);
                let bar = (2.0 * s).max(2.0);
                match sign {
                    '▎' => quad(
                        canvas,
                        [gutter_x + s, y0, gutter_x + s + bar, y0 + rows * ch],
                        s,
                        colour,
                    ),
                    '‾' => {
                        let (a, b) = (gutter_x + s, gutter_x + cw * 0.8);
                        canvas.scene.prims.push(Prim::Line {
                            a: [a, y0 + s],
                            b: [b, y0 + s],
                            width: bar,
                            colour,
                        });
                    }
                    ' ' => {}
                    other => self.cell_glyph(other, canvas, gutter_x, y0, colour),
                }
            }
            // The code actions' lightbulb, in its room left of the number.
            if view.bulb == Some(l.line) {
                let x0 = gutter_x + view.signs as usize as f32 * cw;
                let size = (ch * 0.72).min(cw * 2.0 - 2.0 * s).round();
                let (bx, by) = (
                    (x0 + (cw * 2.0 - size) / 2.0).round(),
                    (y0 + (ch - size) / 2.0).round(),
                );
                quad(
                    canvas,
                    [bx, by, bx + size, by + size],
                    3.0 * s,
                    theme::mix(p.warning, p.island, 0.2),
                );
                // A bulb: its glass, and the base under it.
                let (cx, cy, r) = (bx + size / 2.0, by + size * 0.42, size * 0.22);
                quad(canvas, [cx - r, cy - r, cx + r, cy + r], r, p.warning);
                let w = r * 0.55;
                quad(
                    canvas,
                    [cx - w, cy + r * 0.6, cx + w, cy + r * 1.55],
                    s * 0.5,
                    p.warning,
                );
            }
            // A breakpoint's dot in the same room, and the arrow of the
            // line a run is paused on over it.
            if view.bulb != Some(l.line) && (l.breakpoint.is_some() || l.paused.is_some()) {
                let x0 = gutter_x + view.signs as usize as f32 * cw;
                let d = (ch * 0.5).min(cw * 1.6).round();
                let (bx, by) = ((x0 + cw - d / 2.0).round(), (y0 + (ch - d) / 2.0).round());
                use merlin_core::BreakMark as B;
                if let Some(mark) = l.breakpoint {
                    let (fill, border) = match mark {
                        B::Set => (p.error, p.error),
                        B::Conditional => (p.island, p.error),
                        B::Log => (p.error, p.error),
                        B::Unset => (p.island, p.muted),
                        B::Disabled => (theme::mix(p.error, p.island, 0.55), 0),
                    };
                    let radius = if mark == B::Log { 2.0 * s } else { d / 2.0 };
                    let inset = if mark == B::Log { d * 0.12 } else { 0.0 };
                    canvas.scene.prims.push(Prim::Quad {
                        rect: [bx + inset, by + inset, bx + d - inset, by + d - inset],
                        radius,
                        fill,
                        alpha: 255,
                        border,
                        border_w: if border == 0 { 0.0 } else { 1.5 * s },
                    });
                }
                if let Some(exception) = l.paused {
                    let colour = if exception { p.error } else { p.warning };
                    let (ax, ah) = (x0 + cw * 0.15, ch * 0.5);
                    let ay = y0 + (ch - ah) / 2.0;
                    let w = cw * 1.7;
                    // The arrow's body, then its point as narrowing lines.
                    quad(canvas, [ax, ay, ax + w * 0.5, ay + ah], 1.5 * s, colour);
                    let steps = (w * 0.5 / s).ceil() as usize;
                    for i in 0..steps {
                        let t = i as f32 / steps as f32;
                        let x = ax + w * 0.5 + t * w * 0.5;
                        let half = ah / 2.0 * (1.0 - t);
                        canvas.scene.prims.push(Prim::Line {
                            a: [x, ay + ah / 2.0 - half],
                            b: [x, ay + ah / 2.0 + half],
                            width: 1.2 * s,
                            colour,
                        });
                    }
                }
            }
            if let Some(folded) = l.fold_arrow {
                let cx = (text_x - cw * 0.5) / s;
                let cy = (y0 + ch * 0.5) / s;
                canvas.chevron(cx, cy, cw * 0.55 / s, !folded, p.muted);
            }
        }

        // What an input method is composing: over the text at the cursor,
        // underlined, with its own cursor, until it's typed.
        if let Some((text, at)) = &view.preedit
            && let Some((cx, cy)) = view.carets.first().and_then(|c| c.cell)
        {
            let x0 = text_x + cx as f32 * cw;
            let y = top + cy as f32 * ch + glide;
            let cells: usize = text
                .chars()
                .map(|c| merlin_core::style::char_width(c).max(1))
                .sum();
            quad(
                canvas,
                [x0, y, x0 + cells as f32 * cw, y + ch],
                0.0,
                p.island,
            );
            let mut x = x0;
            let mut caret_x = None;
            for (i, c) in text.char_indices() {
                if *at == Some(i) {
                    caret_x = Some(x);
                }
                let w = merlin_core::style::char_width(c).max(1);
                self.wide_glyph(c, canvas, x, y, w, p.text);
                x += w as f32 * cw;
            }
            underline(canvas, x0, x, y + ch - 2.0 * s, s, p.text, false);
            let beam = (2.0 * s).round().max(2.0);
            let cx = caret_x.unwrap_or(x);
            quad(
                canvas,
                [cx, y + s, cx + beam, y + ch - s],
                beam / 2.0,
                p.accent,
            );
        }

        // The cursors, every one of them, in the style the settings ask.
        if view.focused && caret.on > 0.0 && view.preedit.is_none() {
            let thin = s.round().max(1.0);
            let beam = (2.0 * s).round().max(2.0);
            // An underline has no height to expand: it fades instead.
            let underline = matches!(
                caret.style,
                CursorStyle::Underline | CursorStyle::UnderlineThin
            );
            let expand = caret.expand && !underline;
            let alpha = if expand {
                255
            } else {
                (caret.on * 255.0) as u8
            };
            for at in &view.carets {
                let Some((cx, cy)) = at.cell else { continue };
                let x = (text_x + cx as f32 * cw).round();
                let y = top + cy as f32 * ch + glide;
                if x < text_x - beam || x >= right {
                    continue;
                }
                // Expanding, it grows from its middle.
                let (y0, y1) = (y + s, y + ch - s);
                let half = (y1 - y0) * (1.0 - if expand { caret.on } else { 1.0 }) / 2.0;
                let (y0, y1) = (y0 + half, y1 - half);
                let mut bar = |rect: [f32; 4], radius: f32, alpha: u8| {
                    canvas.scene.prims.push(Prim::Quad {
                        rect,
                        radius,
                        fill: p.accent,
                        alpha,
                        border: 0,
                        border_w: 0.0,
                    })
                };
                match caret.style {
                    CursorStyle::Line | CursorStyle::LineThin => {
                        let w = if caret.style == CursorStyle::Line {
                            beam
                        } else {
                            thin
                        };
                        let x = x - (w / 2.0).floor();
                        bar([x, y0, x + w, y1], w / 2.0, alpha);
                    }
                    // Over its character, which shows through.
                    CursorStyle::Block => bar([x, y0, x + cw, y1], s, (alpha as f32 * 0.55) as u8),
                    CursorStyle::BlockOutline => {
                        bar([x, y0, x + cw, y0 + thin], 0.0, alpha);
                        bar([x, y1 - thin, x + cw, y1], 0.0, alpha);
                        bar([x, y0, x + thin, y1], 0.0, alpha);
                        bar([x + cw - thin, y0, x + cw, y1], 0.0, alpha);
                    }
                    CursorStyle::Underline | CursorStyle::UnderlineThin => {
                        let h = if caret.style == CursorStyle::Underline {
                            beam
                        } else {
                            thin
                        };
                        bar([x, y + ch - s - h, x + cw, y + ch - s], 0.0, alpha);
                    }
                }
            }
        }

        // Sticky scroll: the blocks' first lines pinned over the top, with a
        // shadow under them.
        if !view.sticky.is_empty() {
            let rows = view.sticky.len() as u16;
            let pinned = TextView {
                area: merlin_core::editor::CellRect { h: rows, ..area },
                lines: view.sticky.clone(),
                carets: Vec::new(),
                focused: false,
                sticky: Vec::new(),
                minimap: None,
                top_frac: 0.0,
                ..view.clone()
            };
            self.draw_code(
                &pinned,
                canvas,
                text,
                [ox, oy, w, rows as f32 * ch],
                p,
                Caret { on: 0.0, ..caret },
                0.0,
            );
            let y = top + rows as f32 * ch;
            for (k, alpha) in [(0.0, 60u8), (1.0, 35), (2.0, 15)] {
                canvas.scene.prims.push(Prim::Quad {
                    rect: [gutter_x, y + k * s, right, y + (k + 1.0) * s],
                    radius: 0.0,
                    fill: 0x000000,
                    alpha,
                    border: 0,
                    border_w: 0.0,
                });
            }
        }
        clip_rows(&mut canvas.scene.prims[first_prim..], top, bottom);
    }

    /// The glyphs of a line, shaped once and kept, with the characters
    /// they start at.
    fn shape(&mut self, chars: &[char], looks: &[Look]) -> Vec<Placed> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        chars.hash(&mut hash);
        for lk in looks {
            (lk.bold, lk.italic).hash(&mut hash);
        }
        let key = hash.finish();
        if let Some(found) = self.shapes.lines.get(&key) {
            return found.clone();
        }
        // Tabs and controls are drawn as nothing; the core spaces them.
        let text: String = chars
            .iter()
            .map(|&c| if c.is_control() { ' ' } else { c })
            .collect();
        // Byte offsets back to characters.
        let mut index_of = vec![0usize; text.len() + 1];
        for (i, (b, c)) in text.char_indices().enumerate() {
            for k in 0..c.len_utf8() {
                index_of[b + k] = i;
            }
        }
        // Runs of one face.
        let mut spans: Vec<(&str, cosmic_text::Attrs)> = Vec::new();
        let bytes: Vec<usize> = text
            .char_indices()
            .map(|(b, _)| b)
            .chain([text.len()])
            .collect();
        let mut i = 0;
        while i < chars.len() {
            let face = (looks[i].bold, looks[i].italic);
            let mut j = i + 1;
            while j < chars.len() && (looks[j].bold, looks[j].italic) == face {
                j += 1;
            }
            spans.push((
                &text[bytes[i]..bytes[j]],
                attrs(&self.family, self.ligatures, face.0, face.1),
            ));
            i = j;
        }
        let default = attrs(&self.family, self.ligatures, false, false);
        let line = &mut self.line;
        line.set_wrap(Wrap::None);
        line.set_rich_text(spans, &default, Shaping::Advanced, None);
        line.shape_until_scroll(&mut self.system, false);
        let mut placed = Vec::new();
        // Each glyph goes at its character's column, keeping only its own
        // offset from the shaper: the font's advance is a little off the
        // cell's, and would add up along the line.
        for run in line.layout_runs() {
            for g in run.glyphs.iter() {
                let index = index_of[g.start.min(text.len())];
                let physical = g.physical((-g.x, 0.0), 1.0);
                placed.push(Placed {
                    index,
                    key: physical.cache_key,
                    dx: physical.x,
                    dy: physical.y,
                });
            }
        }
        if self.shapes.lines.len() > SHAPED_LIMIT {
            self.shapes.lines.clear();
        }
        self.shapes.lines.insert(key, placed.clone());
        placed
    }

    /// A glyph's picture, rasterised once into the bitmaps.
    fn picture(&mut self, key: CacheKey, canvas: &mut Canvas) -> Option<Picture> {
        if self.shapes.generation != canvas.bitmaps.generation() {
            self.shapes.pictures.clear();
            self.shapes.generation = canvas.bitmaps.generation();
        }
        if let Some(found) = self.shapes.pictures.get(&key) {
            return *found;
        }
        let picture = self
            .cache
            .get_image_uncached(&mut self.system, key)
            .map(|image| Picture {
                left: image.placement.left,
                top: image.placement.top,
                bitmap: canvas.bitmaps.add(glyph_bitmap(
                    image.placement.width,
                    image.placement.height,
                    image.content,
                    image.data,
                )),
            });
        self.shapes.pictures.insert(key, picture);
        picture
    }

    /// One character in a cell at physical (`x`, `y`), as the gutter has.
    /// A character `cells` wide at physical (`x`, `y`).
    fn wide_glyph(
        &mut self,
        c: char,
        canvas: &mut Canvas,
        x: f32,
        y: f32,
        cells: usize,
        colour: u32,
    ) {
        let Some(glyph) = self.glyph(c, false, false, canvas.bitmaps) else {
            return;
        };
        let (cw, ch) = (self.cell_w as i32 * cells as i32, self.cell_h as i32);
        let (x, y) = (x as i32, y as i32);
        canvas.scene.prims.push(Prim::Glyph {
            bitmap: glyph.bitmap,
            pos: [x + glyph.left, y + self.baseline - glyph.top],
            colour,
            clip: [x, y, x + cw, y + ch],
        });
    }

    fn cell_glyph(&mut self, c: char, canvas: &mut Canvas, x: f32, y: f32, colour: u32) {
        let Some(glyph) = self.glyph(c, false, false, canvas.bitmaps) else {
            return;
        };
        let (cw, ch) = (self.cell_w as i32, self.cell_h as i32);
        let (x, y) = (x as i32, y as i32);
        canvas.scene.prims.push(Prim::Glyph {
            bitmap: glyph.bitmap,
            pos: [x + glyph.left, y + self.baseline - glyph.top],
            colour,
            clip: [x, y, x + cw, y + ch],
        });
    }
}

/// Columns `c` takes at column `col`, tabs to the next stop, as the core
/// counts them.
fn advance(c: char, col: usize) -> usize {
    merlin_core::doc::advance(c, col)
}

/// Cuts what was drawn to the rows from `top` to `bottom`, as a row part
/// scrolled off, or one more than fits, needs: boxes are cut, lines and
/// glyphs outside go.
fn clip_rows(prims: &mut [Prim], top: f32, bottom: f32) {
    let gone = [0.0, 0.0, 0.0, 0.0];
    for prim in prims {
        match prim {
            Prim::Quad { rect, .. } | Prim::Shadow { rect, .. } => {
                if rect[3] <= top || rect[1] >= bottom {
                    *rect = gone;
                } else {
                    rect[1] = rect[1].max(top);
                    rect[3] = rect[3].min(bottom);
                }
            }
            Prim::Line { a, b, .. } => {
                if a[1].max(b[1]) < top || a[1].min(b[1]) > bottom {
                    (*a, *b) = ([-64.0, -64.0], [-64.0, -64.0]);
                }
            }
            Prim::Glyph { clip, .. } | Prim::Image { clip, .. } => {
                clip[1] = clip[1].max(top.floor() as i32);
                clip[3] = clip[3].min(bottom.ceil() as i32);
            }
        }
    }
}

/// Whether two colours are close enough that one is hard to see on the
/// other.
fn near(a: u32, b: u32) -> bool {
    let d = |sh: u32| ((a >> sh) & 0xff).abs_diff((b >> sh) & 0xff);
    d(16) + d(8) + d(0) < 72
}

/// The colour problems of a severity are written in.
fn note_colour(p: &Palette, severity: Severity) -> u32 {
    match severity {
        Severity::Error => p.error,
        Severity::Warning => p.warning,
        _ => p.info,
    }
}

fn quad(canvas: &mut Canvas, rect: [f32; 4], radius: f32, fill: u32) {
    canvas.scene.prims.push(Prim::Quad {
        rect,
        radius,
        fill,
        alpha: 255,
        border: 0,
        border_w: 0.0,
    });
}

/// A rounded box cut to the text's left and right edges.
fn clipped_quad(canvas: &mut Canvas, r: [f32; 4], radius: f32, fill: u32, left: f32, right: f32) {
    let rect = [r[0].max(left), r[1], r[2].min(right), r[3]];
    if rect[2] > rect[0] {
        quad(canvas, rect, radius, fill);
    }
}

/// A straight or wavy line under text from `x0` to `x1` at `y`.
fn underline(canvas: &mut Canvas, x0: f32, x1: f32, y: f32, s: f32, colour: u32, curly: bool) {
    let width = s.max(1.0);
    if !curly {
        canvas.scene.prims.push(Prim::Line {
            a: [x0, y],
            b: [x1, y],
            width,
            colour,
        });
        return;
    }
    // A zigzag of short smoothed lines reads as a wave at this size.
    let step = 2.0 * s;
    let amp = 1.2 * s;
    let mut x = x0;
    let mut up = true;
    while x < x1 {
        let next = (x + step).min(x1);
        let (ya, yb) = if up {
            (y + amp, y - amp)
        } else {
            (y - amp, y + amp)
        };
        canvas.scene.prims.push(Prim::Line {
            a: [x, ya],
            b: [next, yb],
            width,
            colour,
        });
        x = next;
        up = !up;
    }
}
