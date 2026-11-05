//! A terminal's screen, drawn cell by cell in the code's font.

use alacritty_terminal::vte::ansi::CursorShape;

use super::canvas::Canvas;
use super::grid::Fonts;
use super::scene::Prim;
use crate::terminal::{Colours, Screen};

impl Fonts {
    /// Draws `screen` with its top left at physical (`x`, `y`). The cursor
    /// is a block when the terminal has the keys, and hollow when not.
    pub fn draw_terminal(
        &mut self,
        screen: &Screen,
        canvas: &mut Canvas,
        (x, y): (f32, f32),
        colours: &Colours,
        focused: bool,
    ) {
        let (cw, ch) = (self.cell_w as f32, self.cell_h as f32);
        let quad = |canvas: &mut Canvas, rect: [f32; 4], fill: u32| {
            canvas.scene.prims.push(Prim::Quad {
                rect,
                radius: 0.0,
                fill,
                alpha: 255,
                border: 0,
                border_w: 0.0,
            });
        };
        let line_w = (ch / 14.0).max(1.0).round();
        for row in 0..screen.lines {
            let cells = &screen.cells[row * screen.cols..(row + 1) * screen.cols];
            let top = y + row as f32 * ch;
            // Backgrounds, a run of one colour at a time.
            let mut col = 0;
            while col < cells.len() {
                let Some(bg) = cells[col].bg else {
                    col += 1;
                    continue;
                };
                let start = col;
                while col < cells.len() && cells[col].bg == Some(bg) {
                    col += 1;
                }
                let x0 = (x + start as f32 * cw).round();
                let x1 = (x + col as f32 * cw).round();
                quad(canvas, [x0, top, x1, top + ch], bg);
            }
            for (col, cell) in cells.iter().enumerate() {
                let left = (x + col as f32 * cw).round();
                let width = if cell.wide { 2.0 * cw } else { cw };
                if cell.underline {
                    let base = top + self.baseline as f32 + 2.0 * line_w;
                    quad(canvas, [left, base, left + width, base + line_w], cell.fg);
                }
                if cell.strike {
                    let mid = top + ch * 0.55;
                    quad(canvas, [left, mid, left + width, mid + line_w], cell.fg);
                }
                if cell.c == ' ' || cell.c == '\0' {
                    continue;
                }
                let Some(glyph) = self.glyph(cell.c, cell.bold, cell.italic, canvas.bitmaps) else {
                    continue;
                };
                let (gx, gy) = (left as i32, top as i32);
                canvas.scene.prims.push(Prim::Glyph {
                    bitmap: glyph.bitmap,
                    pos: [gx + glyph.left, gy + self.baseline - glyph.top],
                    colour: cell.fg,
                    clip: [gx, gy, gx + width.ceil() as i32, gy + ch as i32],
                });
            }
        }
        // The cursor.
        let Some((col, row, shape)) = screen.cursor else {
            return;
        };
        if screen.scrolled > 0 {
            return;
        }
        let cell = screen.cells.get(row * screen.cols + col).copied();
        let left = (x + col as f32 * cw).round();
        let top = y + row as f32 * ch;
        let width = if cell.is_some_and(|c| c.wide) {
            2.0 * cw
        } else {
            cw
        };
        let colour = colours.text;
        let bar = (ch / 9.0).max(2.0).round();
        match shape {
            CursorShape::Beam => quad(canvas, [left, top, left + bar, top + ch], colour),
            CursorShape::Underline => quad(
                canvas,
                [left, top + ch - bar, left + width, top + ch],
                colour,
            ),
            _ if !focused => canvas.scene.prims.push(Prim::Quad {
                rect: [left, top, left + width, top + ch],
                radius: 0.0,
                fill: colour,
                alpha: 0,
                border: colour,
                border_w: line_w,
            }),
            _ => {
                quad(canvas, [left, top, left + width, top + ch], colour);
                // The character under it, in the background's colour.
                if let Some(cell) = cell.filter(|c| c.c != ' ')
                    && let Some(glyph) = self.glyph(cell.c, cell.bold, cell.italic, canvas.bitmaps)
                {
                    let (gx, gy) = (left as i32, top as i32);
                    canvas.scene.prims.push(Prim::Glyph {
                        bitmap: glyph.bitmap,
                        pos: [gx + glyph.left, gy + self.baseline - glyph.top],
                        colour: cell.bg.unwrap_or(colours.background),
                        clip: [gx, gy, gx + width as i32, gy + ch as i32],
                    });
                }
            }
        }
    }
}
