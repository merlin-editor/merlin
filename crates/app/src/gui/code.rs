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
use merlin_core::screen::{Cell, Style, Surface};

use super::canvas::{Canvas, glyph_bitmap};
use super::grid::{Fonts, attrs};
use super::scene::{BitmapId, Prim};
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
    /// Draws `view` with the grid's top left at physical (`ox`, `oy`).
    /// `caret_on` is the blink's phase; carets show only while it's on.
    /// `glide` moves the lines down by that many pixels, while scrolling
    /// eases to where the view is.
    pub fn draw_code(
        &mut self,
        view: &TextView,
        canvas: &mut Canvas,
        (ox, oy): (usize, usize),
        p: &Palette,
        caret_on: bool,
        glide: f32,
    ) {
        let (cw, ch) = (self.cell_w as f32, self.cell_h as f32);
        let s = canvas.scale;
        let area = view.area;
        let gutter_x = ox as f32 + area.x as f32 * cw;
        let text_x = gutter_x + view.gutter as f32 * cw;
        let right = text_x + view.columns as f32 * cw;
        let top = oy as f32 + area.y as f32 * ch;
        let bottom = top + area.h as f32 * ch;
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
            // Where each character goes, as the core places it.
            let mut spots = Vec::with_capacity(l.chars.len());
            let (mut d, mut r) = (0usize, 0usize);
            for (i, &c) in l.chars.iter().enumerate() {
                while r + 1 < l.rows.len() && i >= l.rows[r + 1].start {
                    r += 1;
                }
                let width = advance(c, d);
                let row = &l.rows[r];
                let col = (row.indent + d - row.col) as isize - view.hscroll as isize;
                spots.push(Spot { row: r, col, width });
                d += width;
            }
            let end_col = l.rows.last().map_or(0, |row| row.indent + d - row.col) as isize
                - view.hscroll as isize;
            let x_of = |col: isize| text_x + col as f32 * cw;
            let row_y = |row: usize| y0 + row as f32 * ch;

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

            // The gutter: the number, a sign, and the fold arrow.
            let number_look = look(l.number_style, Default::default(), None, None);
            let digits_x = gutter_x + view.signs as usize as f32 * cw;
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
            if let Some(folded) = l.fold_arrow {
                let cx = (text_x - cw * 0.5) / s;
                let cy = (y0 + ch * 0.5) / s;
                canvas.chevron(cx, cy, cw * 0.55 / s, !folded, p.muted);
            }
        }

        // The cursors: thin bars, every one of them.
        if view.focused && caret_on {
            let beam = (2.0 * s).round().max(2.0);
            for caret in &view.carets {
                let Some((cx, cy)) = caret.cell else { continue };
                let x = (text_x + cx as f32 * cw).round() - (beam / 2.0).floor();
                let y = top + cy as f32 * ch + glide;
                if x < text_x - beam || x >= right {
                    continue;
                }
                quad(
                    canvas,
                    [x, y + s, x + beam, y + ch - s],
                    beam / 2.0,
                    p.accent,
                );
            }
        }
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
                attrs(&self.family, face.0, face.1),
            ));
            i = j;
        }
        let default = attrs(&self.family, false, false);
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
