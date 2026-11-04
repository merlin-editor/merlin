//! The code: the editor's screen of cells, drawn in the code's font inside
//! the editor island.

use std::collections::HashMap;
use std::env;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache,
    SwashContent, Weight,
};
use merlin_core::screen::{Screen, WIDE_TAIL};

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

/// A character's picture, rasterised once and kept.
struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    /// Colour glyphs (emoji) are RGBA; the rest are coverage.
    colour: bool,
    data: Vec<u8>,
}

pub struct Fonts {
    system: FontSystem,
    cache: SwashCache,
    family: Option<String>,
    size: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    /// From the top of a row.
    baseline: i32,
    line: Buffer,
    glyphs: HashMap<(char, bool, bool), Option<Glyph>>,
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
        };
        fonts.measure();
        fonts
    }
}

/// The face for a cell: the chosen family, bold or italic as it says.
fn attrs(family: &Option<String>, bold: bool, italic: bool) -> Attrs<'_> {
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

    fn glyph(&mut self, ch: char, bold: bool, italic: bool) -> Option<&Glyph> {
        if !self.glyphs.contains_key(&(ch, bold, italic)) {
            let glyph = self.rasterise(ch, bold, italic);
            self.glyphs.insert((ch, bold, italic), glyph);
        }
        self.glyphs.get(&(ch, bold, italic))?.as_ref()
    }

    fn rasterise(&mut self, ch: char, bold: bool, italic: bool) -> Option<Glyph> {
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
        Some(Glyph {
            left: image.placement.left + physical.x,
            top: image.placement.top - physical.y,
            width: image.placement.width as usize,
            height: image.placement.height as usize,
            colour: image.content == SwashContent::Color,
            data: match image.content {
                // Subpixel masks would need the panel's layout; their
                // average is plain coverage.
                SwashContent::SubpixelMask => image
                    .data
                    .chunks(4)
                    .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                    .collect(),
                _ => image.data,
            },
        })
    }

    /// Draws `screen` with its top left at physical pixel (`ox`, `oy`) in
    /// `frame`, `w` by `h` pixels, in the palette's colours.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        screen: &Screen,
        frame: &mut [u32],
        w: usize,
        h: usize,
        (ox, oy): (usize, usize),
        p: &Palette,
    ) {
        let (cw, ch) = (self.cell_w as usize, self.cell_h as usize);
        let cells = |x: u16, y: u16| (ox + x as usize * cw, oy + y as usize * ch);
        // Backgrounds first, so a glyph wider than its cell isn't covered
        // by the next one's.
        for y in 0..screen.height {
            for x in 0..screen.width {
                let look = theme::cell(p, &screen.cell(x, y));
                let (px, py) = cells(x, y);
                fill(frame, w, h, px, py, cw, ch, look.bg.unwrap_or(p.island));
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
                if box_drawing(cell.ch, frame, w, h, px, py, cw, ch, fg) {
                    continue;
                }
                // Room for an icon wider than its cell, when the next is blank.
                let room = if !wide && x + 1 < screen.width && screen.cell(x + 1, y).ch == ' ' {
                    2 * cw
                } else {
                    span
                };
                let baseline = self.baseline;
                let Some(glyph) = self.glyph(cell.ch, look.bold, look.italic) else {
                    continue;
                };
                // Centre glyphs that are narrower than a wide character's
                // two cells, as CJK and emoji are drawn.
                let offset = if wide && glyph.width < span {
                    ((span - glyph.width) / 2) as i32 - glyph.left.max(0)
                } else {
                    0
                };
                blit(
                    frame,
                    w,
                    (px, py, room, ch),
                    px as i32 + glyph.left + offset,
                    py as i32 + baseline - glyph.top,
                    glyph,
                    fg,
                );
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
                for dx in 0..cw {
                    // A wave for problems, two pixels up and down.
                    let lift = if look.curly {
                        let phase = ((px + dx) as f32 / (cw as f32 / 2.0)) * std::f32::consts::PI;
                        ((phase.sin() + 1.0) * thick as f32).round() as usize
                    } else {
                        0
                    };
                    let row = (under + 2 * thick).saturating_sub(lift);
                    fill(frame, w, h, px + dx, row, 1, thick, colour);
                }
            }
        }
        if let Some((x, y)) = screen.cursor {
            let (px, py) = cells(x, y);
            let beam = (self.size / 7.0).round().max(2.0) as usize;
            fill(frame, w, h, px, py, beam, ch, p.accent);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill(frame: &mut [u32], w: usize, h: usize, x: usize, y: usize, fw: usize, fh: usize, c: u32) {
    for row in y.min(h)..(y + fh).min(h) {
        let start = row * w;
        frame[start + x.min(w)..start + (x + fw).min(w)].fill(c);
    }
}

/// Draws `glyph` with its origin at (`gx`, `gy`), inside `clip`.
fn blit(
    frame: &mut [u32],
    w: usize,
    clip: (usize, usize, usize, usize),
    gx: i32,
    gy: i32,
    glyph: &Glyph,
    fg: u32,
) {
    let h = frame.len() / w;
    let (cx, cy, cw, ch) = clip;
    let (x0, x1) = (cx as i32, ((cx + cw).min(w)) as i32);
    let (y0, y1) = (cy as i32, ((cy + ch).min(h)) as i32);
    for row in 0..glyph.height as i32 {
        let y = gy + row;
        if y < y0 || y >= y1 {
            continue;
        }
        for col in 0..glyph.width as i32 {
            let x = gx + col;
            if x < x0 || x >= x1 {
                continue;
            }
            let i = (row as usize) * glyph.width + col as usize;
            let at = y as usize * w + x as usize;
            frame[at] = if glyph.colour {
                let p = &glyph.data[i * 4..i * 4 + 4];
                let rgb = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                blend(frame[at], rgb, p[3])
            } else {
                blend(frame[at], fg, glyph.data[i])
            };
        }
    }
}

fn blend(bg: u32, fg: u32, alpha: u8) -> u32 {
    if alpha == 0 {
        return bg;
    }
    if alpha == 255 {
        return fg;
    }
    let a = alpha as u32;
    [16, 8, 0].iter().fold(0, |out, shift| {
        let (f, b) = ((fg >> shift) & 0xff, (bg >> shift) & 0xff);
        out | (((f * a + b * (255 - a)) / 255) << shift)
    })
}

/// Lines and blocks that must meet the next cell's, drawn by hand: a font's
/// versions stop short when rows are taller than the font.
#[allow(clippy::too_many_arguments)]
fn box_drawing(
    c: char,
    frame: &mut [u32],
    w: usize,
    h: usize,
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
            fill(frame, w, h, x, y, cw / 2, ch, fg);
            return true;
        }
        '▐' => {
            fill(frame, w, h, x + cw / 2, y, cw - cw / 2, ch, fg);
            return true;
        }
        '▕' => {
            fill(frame, w, h, x + cw - cw / 8, y, (cw / 8).max(1), ch, fg);
            return true;
        }
        '█' => {
            fill(frame, w, h, x, y, cw, ch, fg);
            return true;
        }
        _ => return false,
    };
    let (left, right, up, down) = arms;
    if left {
        fill(frame, w, h, x, my, mx + t - x, t, fg);
    }
    if right {
        fill(frame, w, h, mx, my, x + cw - mx, t, fg);
    }
    if up {
        fill(frame, w, h, mx, y, t, my + t - y, fg);
    }
    if down {
        fill(frame, w, h, mx, my, t, y + ch - my, fg);
    }
    true
}
