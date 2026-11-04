//! The code: the editor's screen of cells, drawn in the code's font inside
//! the editor island.

use std::collections::HashMap;
use std::env;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache, Weight,
};
use merlin_core::screen::{Screen, Style, Surface, WIDE_TAIL};

use super::canvas::{Canvas, glyph_bitmap};
use super::scene::{BitmapId, Bitmaps, Prim};
use crate::theme::{self, Palette};

/// Points, before the display's scale.
const FONT_SIZE: f32 = 13.5;
/// Rows are this many times the font size.
const LINE_HEIGHT: f32 = 1.55;
const TEXT: u32 = 0xd4d5da;
/// Families tried in order; the first one installed is used. The Nerd
/// Fonts carry the icons the core draws.
const FAMILIES: &[&str] = &[
    "JetBrainsMono Nerd Font",
    "JetBrainsMono NF",
    "JetBrains Mono",
    "FiraCode Nerd Font",
    "DejaVu Sans Mono",
];

/// A character's picture, rasterised once and kept in the bitmaps.
#[derive(Clone, Copy)]
pub(super) struct Glyph {
    pub(super) bitmap: BitmapId,
    pub(super) left: i32,
    pub(super) top: i32,
    width: usize,
}

pub struct Fonts {
    pub(super) system: FontSystem,
    pub(super) cache: SwashCache,
    pub(super) family: Option<String>,
    pub(super) size: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    /// From the top of a row.
    pub(super) baseline: i32,
    pub(super) line: Buffer,
    glyphs: HashMap<(char, bool, bool), Option<Glyph>>,
    /// Shaped lines and their glyphs, for the code drawn from text views.
    pub(super) shapes: super::code::Shapes,
    /// The bitmaps' generation the glyphs belong to.
    generation: u64,
}

impl Fonts {
    /// The code's font: `MERLIN_FONT` and `MERLIN_FONT_SIZE` if set, else
    /// what the packs ask for, else the first of [`FAMILIES`] installed.
    pub fn new(scale: f32, family: Option<&str>, size: Option<f32>) -> Self {
        let mut system = FontSystem::new();
        let size = env::var("MERLIN_FONT_SIZE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .or(size)
            .unwrap_or(FONT_SIZE)
            * scale;
        let wanted = env::var("MERLIN_FONT").ok().or(family.map(str::to_string));
        let installed = |name: &str| {
            system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == name))
        };
        let family = wanted
            .iter()
            .map(String::as_str)
            .chain(FAMILIES.iter().copied())
            .find(|name| installed(name))
            .map(str::to_string);
        let cell_h = (size * LINE_HEIGHT).round().max(1.0);
        let line = Buffer::new(&mut system, Metrics::new(size, cell_h));
        let mut fonts = Self {
            system,
            cache: SwashCache::new(),
            family,
            size,
            cell_w: 1,
            cell_h: cell_h as u32,
            baseline: 0,
            line,
            glyphs: HashMap::new(),
            shapes: Default::default(),
            generation: u64::MAX,
        };
        fonts.measure();
        fonts
    }
}

/// The face for a cell: the chosen family, bold or italic as it says.
pub(super) fn attrs(family: &Option<String>, bold: bool, italic: bool) -> Attrs<'_> {
    let family = match family {
        Some(name) => Family::Name(name),
        None => Family::Monospace,
    };
    let mut attrs = Attrs::new().family(family);
    if bold {
        attrs = attrs.weight(Weight::BOLD);
    }
    if italic {
        attrs = attrs.style(FontStyle::Italic);
    }
    attrs
}

impl Fonts {
    /// The cell's width and the baseline, from the font's `M`.
    fn measure(&mut self) {
        let attrs = attrs(&self.family, false, false);
        let line = &mut self.line;
        line.set_text("M", &attrs, Shaping::Advanced, None);
        line.shape_until_scroll(&mut self.system, false);
        if let Some(run) = line.layout_runs().next() {
            if let Some(g) = run.glyphs.first() {
                self.cell_w = g.w.round().max(1.0) as u32;
            }
            self.baseline = run.line_y.round() as i32;
        }
    }

    pub(super) fn glyph(
        &mut self,
        ch: char,
        bold: bool,
        italic: bool,
        bitmaps: &mut Bitmaps,
    ) -> Option<Glyph> {
        if self.generation != bitmaps.generation() {
            self.glyphs.clear();
            self.generation = bitmaps.generation();
        }
        if !self.glyphs.contains_key(&(ch, bold, italic)) {
            let glyph = self.rasterise(ch, bold, italic, bitmaps);
            self.glyphs.insert((ch, bold, italic), glyph);
        }
        self.glyphs[&(ch, bold, italic)]
    }

    fn rasterise(
        &mut self,
        ch: char,
        bold: bool,
        italic: bool,
        bitmaps: &mut Bitmaps,
    ) -> Option<Glyph> {
        let attrs = attrs(&self.family, bold, italic);
        let line = &mut self.line;
        line.set_text(&ch.to_string(), &attrs, Shaping::Advanced, None);
        line.shape_until_scroll(&mut self.system, false);
        let physical = line
            .layout_runs()
            .next()
            .and_then(|run| run.glyphs.first())
            .map(|g| g.physical((0.0, 0.0), 1.0));
        let physical = physical?;
        let image = self
            .cache
            .get_image_uncached(&mut self.system, physical.cache_key)?;
        let (width, height) = (image.placement.width, image.placement.height);
        Some(Glyph {
            left: image.placement.left + physical.x,
            top: image.placement.top - physical.y,
            width: width as usize,
            bitmap: bitmaps.add(glyph_bitmap(width, height, image.content, image.data)),
        })
    }

    /// Draws `screen` with its top left at physical pixel (`ox`, `oy`) on
    /// `canvas`, in the palette's colours.
    pub fn draw(
        &mut self,
        screen: &Screen,
        canvas: &mut Canvas,
        (ox, oy): (usize, usize),
        p: &Palette,
    ) {
        let (w, h) = (canvas.width, canvas.height);
        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let cells = |x: u16, y: u16| (ox + x as usize * cw, oy + y as usize * ch);
        // Backgrounds first, so a glyph wider than its cell isn't covered
        // by the next one's; a row's run of one colour is one rectangle.
        for y in 0..screen.height {
            let mut run: Option<(u16, u32)> = None;
            for x in 0..=screen.width {
                // Plain cells of the text show the code drawn under them.
                let bg = (x < screen.width)
                    .then(|| screen.cell(x, y))
                    .and_then(|cell| {
                        let plain = cell.ch == ' '
                            && cell.style == Style::Normal
                            && cell.surface == Surface::Editor
                            && cell.tint.is_none();
                        (!plain).then(|| theme::cell(p, &cell).bg.unwrap_or(p.island))
                    });
                match run {
                    Some((_, colour)) if Some(colour) == bg => continue,
                    Some((from, colour)) => {
                        let (px, py) = cells(from, y);
                        canvas.fill_px(px, py, (x - from) as usize * cw, ch, colour);
                    }
                    None => {}
                }
                run = bg.map(|colour| (x, colour));
            }
        }
        for y in 0..screen.height {
            for x in 0..screen.width {
                let cell = screen.cell(x, y);
                if cell.ch == ' ' || cell.ch == WIDE_TAIL {
                    continue;
                }
                let look = theme::cell(p, &cell);
                let fg = look.fg.unwrap_or(TEXT);
                let (px, py) = cells(x, y);
                let wide = x + 1 < screen.width && screen.cell(x + 1, y).ch == WIDE_TAIL;
                let span = if wide { 2 * cw } else { cw };
                if box_drawing(cell.ch, canvas, px, py, cw, ch, fg) {
                    continue;
                }
                // Room for an icon wider than its cell, when the next is blank.
                let room = if !wide && x + 1 < screen.width && screen.cell(x + 1, y).ch == ' ' {
                    2 * cw
                } else {
                    span
                };
                let Some(glyph) = self.glyph(cell.ch, look.bold, look.italic, canvas.bitmaps)
                else {
                    continue;
                };
                // Centre glyphs that are narrower than a wide character's
                // two cells, as CJK and emoji are drawn.
                let offset = if wide && glyph.width < span {
                    ((span - glyph.width) / 2) as i32 - glyph.left.max(0)
                } else {
                    0
                };
                canvas.scene.prims.push(Prim::Glyph {
                    bitmap: glyph.bitmap,
                    pos: [
                        px as i32 + glyph.left + offset,
                        py as i32 + self.baseline - glyph.top,
                    ],
                    colour: fg,
                    clip: [
                        px as i32,
                        py as i32,
                        (px + room).min(w) as i32,
                        (py + ch).min(h) as i32,
                    ],
                });
            }
        }
        for y in 0..screen.height {
            for x in 0..screen.width {
                let look = theme::cell(p, &screen.cell(x, y));
                if !look.underline {
                    continue;
                }
                let colour = look.underline_colour.or(look.fg).unwrap_or(TEXT);
                let (px, py) = cells(x, y);
                let under = py + (self.baseline as usize + 2).min(ch - 1);
                let thick = (self.size / 14.0).round().max(1.0) as usize;
                if !look.curly {
                    canvas.fill_px(px, under + 2 * thick, cw, thick, colour);
                    continue;
                }
                for dx in 0..cw {
                    // A wave for problems, two pixels up and down.
                    let phase = ((px + dx) as f32 / (cw as f32 / 2.0)) * std::f32::consts::PI;
                    let lift = ((phase.sin() + 1.0) * thick as f32).round() as usize;
                    let row = (under + 2 * thick).saturating_sub(lift);
                    canvas.fill_px(px + dx, row, 1, thick, colour);
                }
            }
        }
        if let Some((x, y)) = screen.cursor {
            let (px, py) = cells(x, y);
            let beam = (self.size / 7.0).round().max(2.0) as usize;
            canvas.fill_px(px, py, beam, ch, p.accent);
        }
    }
}

/// Lines and blocks that must meet the next cell's, drawn by hand: a font's
/// versions stop short when rows are taller than the font.
#[allow(clippy::too_many_arguments)]
fn box_drawing(
    c: char,
    canvas: &mut Canvas,
    x: usize,
    y: usize,
    cw: usize,
    ch: usize,
    fg: u32,
) -> bool {
    let t = (cw / 8).max(1);
    let (mx, my) = (x + cw / 2 - t / 2, y + ch / 2 - t / 2);
    // Which arms reach out from the middle: left, right, up, down.
    let arms = match c {
        '─' => (true, true, false, false),
        '│' => (false, false, true, true),
        '├' => (false, true, true, true),
        '┤' => (true, false, true, true),
        '┼' => (true, true, true, true),
        '┌' | '╭' => (false, true, false, true),
        '┐' | '╮' => (true, false, false, true),
        '└' | '╰' => (false, true, true, false),
        '┘' | '╯' => (true, false, true, false),
        '┬' => (true, true, false, true),
        '┴' => (true, true, true, false),
        '▌' => {
            canvas.fill_px(x, y, cw / 2, ch, fg);
            return true;
        }
        '▐' => {
            canvas.fill_px(x + cw / 2, y, cw - cw / 2, ch, fg);
            return true;
        }
        '▕' => {
            canvas.fill_px(x + cw - cw / 8, y, (cw / 8).max(1), ch, fg);
            return true;
        }
        '█' => {
            canvas.fill_px(x, y, cw, ch, fg);
            return true;
        }
        _ => return false,
    };
    let (left, right, up, down) = arms;
    if left {
        canvas.fill_px(x, my, mx + t - x, t, fg);
    }
    if right {
        canvas.fill_px(mx, my, x + cw - mx, t, fg);
    }
    if up {
        canvas.fill_px(mx, y, t, my + t - y, fg);
    }
    if down {
        canvas.fill_px(mx, my, t, y + ch - my, fg);
    }
    true
}
