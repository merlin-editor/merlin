//! Drawing in pixels: filled and rounded rectangles, lines, and text in the
//! interface's font or the code's, shaped once and kept.

use std::collections::HashMap;
use std::env;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, Shaping, Style as FontStyle, SwashCache,
    SwashContent, Weight,
};

/// Families for the interface, the first one installed wins. Inter comes
/// with merlin, so it's always there unless `MERLIN_UI_FONT` asks for
/// another.
const UI_FAMILIES: &[&str] = &["Inter", "Noto Sans", "Cantarell", "DejaVu Sans"];

/// Inter's regular, italic, medium and semibold faces, built in
/// (SIL Open Font License; see fonts/Inter-LICENSE.txt).
const INTER: &[&[u8]] = &[
    include_bytes!("../../fonts/Inter-Regular.ttf"),
    include_bytes!("../../fonts/Inter-Italic.ttf"),
    include_bytes!("../../fonts/Inter-Medium.ttf"),
    include_bytes!("../../fonts/Inter-SemiBold.ttf"),
];
/// Families for code. The Nerd Fonts carry the icons the core uses.
const MONO_FAMILIES: &[&str] = &[
    "JetBrainsMono Nerd Font",
    "JetBrainsMono NF",
    "JetBrains Mono",
    "FiraCode Nerd Font",
    "DejaVu Sans Mono",
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Smaller by `d` on every side.
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }
}

/// A text face: the interface's or the code's font, at a size in logical
/// pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Face {
    pub mono: bool,
    /// Tenths of a logical pixel, so faces can be hashed.
    pub size: u16,
    pub weight: u16,
    pub italic: bool,
}

impl Face {
    pub const fn ui(size: f32) -> Self {
        Self {
            mono: false,
            size: (size * 10.0) as u16,
            weight: 400,
            italic: false,
        }
    }

    pub const fn mono(size: f32) -> Self {
        Self {
            mono: true,
            ..Self::ui(size)
        }
    }

    pub const fn bold(self) -> Self {
        Self {
            weight: 600,
            ..self
        }
    }

    pub const fn medium(self) -> Self {
        Self {
            weight: 500,
            ..self
        }
    }

    pub const fn italic(self) -> Self {
        Self {
            italic: true,
            ..self
        }
    }
}

/// A shaped piece of text: its glyphs from its top left, and its size.
struct Shaped {
    glyphs: Vec<(CacheKey, i32, i32)>,
    width: f32,
    height: f32,
}

struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    colour: bool,
    data: Vec<u8>,
}

/// Fonts, with every shaped string and glyph picture kept.
pub struct Text {
    system: FontSystem,
    raster: SwashCache,
    ui: Option<String>,
    mono: Option<String>,
    scale: f32,
    shaped: HashMap<(String, Face, Option<u32>), Shaped>,
    glyphs: HashMap<CacheKey, Option<Glyph>>,
}

impl Text {
    /// Fonts for the interface and for icons from the code's font, as the
    /// packs ask for them, unless `MERLIN_UI_FONT` or `MERLIN_FONT` say.
    pub fn new(scale: f32, ui: Option<&str>, code: Option<&str>) -> Self {
        let mut system = FontSystem::new();
        for face in INTER {
            system.db_mut().load_font_data(face.to_vec());
        }
        let installed = |name: &str| {
            system
                .db()
                .faces()
                .any(|f| f.families.iter().any(|(n, _)| n == name))
        };
        let pick = |wanted: Option<String>, families: &[&str]| {
            wanted
                .iter()
                .map(String::as_str)
                .chain(families.iter().copied())
                .find(|name| installed(name))
                .map(str::to_string)
        };
        let ui = pick(
            env::var("MERLIN_UI_FONT").ok().or(ui.map(str::to_string)),
            UI_FAMILIES,
        );
        let mono = pick(
            env::var("MERLIN_FONT").ok().or(code.map(str::to_string)),
            MONO_FAMILIES,
        );
        Self {
            system,
            raster: SwashCache::new(),
            ui,
            mono,
            scale,
            shaped: HashMap::new(),
            glyphs: HashMap::new(),
        }
    }

    fn shape(&mut self, text: &str, face: Face, wrap: Option<f32>) -> &Shaped {
        let wrap_px = wrap.map(|w| (w * self.scale) as u32);
        let key = (text.to_string(), face, wrap_px);
        if !self.shaped.contains_key(&key) {
            // Strings come and go as the user types; don't keep them all.
            if self.shaped.len() > 4000 {
                self.shaped.clear();
            }
            let size = face.size as f32 / 10.0 * self.scale;
            let line = (size * 1.4).round();
            let mut buffer = Buffer::new(&mut self.system, Metrics::new(size, line));
            buffer.set_size(wrap_px.map(|w| w as f32), None);
            let family = match (face.mono, &self.ui, &self.mono) {
                (false, Some(name), _) => Family::Name(name),
                (true, _, Some(name)) => Family::Name(name),
                (false, None, _) => Family::SansSerif,
                (true, _, None) => Family::Monospace,
            };
            let mut attrs = Attrs::new().family(family).weight(Weight(face.weight));
            if face.italic {
                attrs = attrs.style(FontStyle::Italic);
            }
            buffer.set_text(text, &attrs, Shaping::Advanced, None);
            buffer.shape_until_scroll(&mut self.system, false);
            let mut glyphs = Vec::new();
            let mut width: f32 = 0.0;
            let mut height: f32 = 0.0;
            for run in buffer.layout_runs() {
                width = width.max(run.line_w);
                height = height.max(run.line_top + run.line_height);
                for g in run.glyphs {
                    let p = g.physical((0.0, run.line_y), 1.0);
                    glyphs.push((p.cache_key, p.x, p.y));
                }
            }
            let shaped = Shaped {
                glyphs,
                width: width / self.scale,
                height: height.max(line) / self.scale,
            };
            self.shaped.insert(key.clone(), shaped);
        }
        &self.shaped[&key]
    }

    /// The width of `text` in logical pixels.
    pub fn width(&mut self, text: &str, face: Face) -> f32 {
        self.shape(text, face, None).width
    }

    /// The height of `text` wrapped at `width`.
    pub fn wrapped_height(&mut self, text: &str, face: Face, width: f32) -> f32 {
        self.shape(text, face, Some(width)).height
    }

    /// `text` cut to fit `max`, ending in an ellipsis if it had to be cut.
    pub fn fit(&mut self, text: &str, face: Face, max: f32) -> String {
        if self.width(text, face) <= max {
            return text.to_string();
        }
        let chars: Vec<char> = text.chars().collect();
        let (mut lo, mut hi) = (0, chars.len());
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let candidate: String = chars[..mid].iter().collect::<String>() + "…";
            if self.width(&candidate, face) <= max {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        chars[..lo].iter().collect::<String>() + "…"
    }

    fn glyph(&mut self, key: CacheKey) -> Option<&Glyph> {
        if !self.glyphs.contains_key(&key) {
            let glyph = self
                .raster
                .get_image_uncached(&mut self.system, key)
                .map(|image| Glyph {
                    left: image.placement.left,
                    top: image.placement.top,
                    width: image.placement.width as usize,
                    height: image.placement.height as usize,
                    colour: image.content == SwashContent::Color,
                    data: match image.content {
                        SwashContent::SubpixelMask => image
                            .data
                            .chunks(4)
                            .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                            .collect(),
                        _ => image.data,
                    },
                });
            self.glyphs.insert(key, glyph);
        }
        self.glyphs.get(&key)?.as_ref()
    }

    /// Draws `text` with its top left at (`x`, `y`), clipped to `clip`,
    /// wrapped at `wrap` if given. Returns its width.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        face: Face,
        x: f32,
        y: f32,
        colour: u32,
        clip: Rect,
        wrap: Option<f32>,
    ) -> f32 {
        let s = self.scale;
        let (glyphs, width) = {
            let shaped = self.shape(text, face, wrap);
            (shaped.glyphs.clone(), shaped.width)
        };
        let (ox, oy) = ((x * s).round() as i32, (y * s).round() as i32);
        let clip = canvas.physical(clip);
        for (key, gx, gy) in glyphs {
            let Some(glyph) = self.glyph(key) else {
                continue;
            };
            canvas.blit(
                glyph,
                ox + gx + glyph.left,
                oy + gy - glyph.top,
                colour,
                clip,
            );
        }
        width
    }

    /// Draws `text` centred vertically in a row from `y` of height `h`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_in_row(
        &mut self,
        canvas: &mut Canvas,
        text: &str,
        face: Face,
        x: f32,
        y: f32,
        h: f32,
        colour: u32,
        clip: Rect,
    ) -> f32 {
        let line = (face.size as f32 / 10.0 * 1.4).round();
        self.draw(
            canvas,
            text,
            face,
            x,
            y + (h - line) / 2.0,
            colour,
            clip,
            None,
        )
    }
}

/// The window's pixels, drawn on in logical coordinates.
pub struct Canvas<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    pub scale: f32,
}

impl Canvas<'_> {
    /// A logical rectangle in whole physical pixels: (x0, y0, x1, y1).
    fn physical(&self, r: Rect) -> (i32, i32, i32, i32) {
        let s = self.scale;
        (
            ((r.x * s).round() as i32).clamp(0, self.width as i32),
            ((r.y * s).round() as i32).clamp(0, self.height as i32),
            (((r.x + r.w) * s).round() as i32).clamp(0, self.width as i32),
            (((r.y + r.h) * s).round() as i32).clamp(0, self.height as i32),
        )
    }

    pub fn fill(&mut self, r: Rect, colour: u32) {
        let (x0, y0, x1, y1) = self.physical(r);
        for y in y0..y1 {
            let row = y as usize * self.width;
            self.pixels[row + x0 as usize..row + x1 as usize].fill(colour);
        }
    }

    /// A rectangle with corners rounded to `radius`, smoothed at the edges.
    pub fn round(&mut self, r: Rect, radius: f32, colour: u32) {
        if radius <= 0.0 {
            return self.fill(r, colour);
        }
        let s = self.scale;
        let rad = (radius * s).min(r.w * s / 2.0).min(r.h * s / 2.0);
        let (x0, y0, x1, y1) = self.physical(r);
        let (fx0, fy0) = (r.x * s, r.y * s);
        let (fx1, fy1) = ((r.x + r.w) * s, (r.y + r.h) * s);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let dy = if py < fy0 + rad {
                fy0 + rad - py
            } else if py > fy1 - rad {
                py - (fy1 - rad)
            } else {
                0.0
            };
            let row = y as usize * self.width;
            if dy == 0.0 {
                self.pixels[row + x0 as usize..row + x1 as usize].fill(colour);
                continue;
            }
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dx = if px < fx0 + rad {
                    fx0 + rad - px
                } else if px > fx1 - rad {
                    px - (fx1 - rad)
                } else {
                    0.0
                };
                let d = (dx * dx + dy * dy).sqrt() - rad;
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let i = row + x as usize;
                    self.pixels[i] = blend(self.pixels[i], colour, (cover * 255.0) as u8);
                }
            }
        }
    }

    /// A rounded outline `width` wide.
    pub fn outline(&mut self, r: Rect, radius: f32, width: f32, colour: u32, inside: u32) {
        self.round(r, radius, colour);
        self.round(r.inset(width), (radius - width).max(0.0), inside);
    }

    /// A soft shadow under a floating box; lighter for a light theme.
    pub fn shadow(&mut self, r: Rect, radius: f32, light: bool) {
        let layers: [(f32, u8); 3] = if light {
            [(10.0, 6), (6.0, 9), (3.0, 12)]
        } else {
            [(10.0, 18), (6.0, 26), (3.0, 34)]
        };
        for (grow, alpha) in layers {
            let g = Rect::new(
                r.x - grow,
                r.y - grow + 4.0,
                r.w + 2.0 * grow,
                r.h + 2.0 * grow,
            );
            self.tint(g, radius + grow, 0x000000, alpha);
        }
    }

    /// Darkens or tints what's drawn already, through a rounded shape.
    pub fn tint(&mut self, r: Rect, radius: f32, colour: u32, alpha: u8) {
        let s = self.scale;
        let rad = (radius * s).min(r.w * s / 2.0).min(r.h * s / 2.0).max(0.0);
        let (x0, y0, x1, y1) = self.physical(r);
        let (fx0, fy0) = (r.x * s, r.y * s);
        let (fx1, fy1) = ((r.x + r.w) * s, (r.y + r.h) * s);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let dy = (fy0 + rad - py).max(py - (fy1 - rad)).max(0.0);
            let row = y as usize * self.width;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dx = (fx0 + rad - px).max(px - (fx1 - rad)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt() - rad;
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let i = row + x as usize;
                    self.pixels[i] = blend(self.pixels[i], colour, (cover * alpha as f32) as u8);
                }
            }
        }
    }

    /// A chevron pointing right, or down when `open`.
    pub fn chevron(&mut self, cx: f32, cy: f32, size: f32, open: bool, colour: u32) {
        let h = size / 2.0;
        let (a, b, c) = if open {
            (
                (cx - h, cy - h / 2.0),
                (cx, cy + h / 2.0),
                (cx + h, cy - h / 2.0),
            )
        } else {
            (
                (cx - h / 2.0, cy - h),
                (cx + h / 2.0, cy),
                (cx - h / 2.0, cy + h),
            )
        };
        self.line(a, b, 1.5, colour);
        self.line(b, c, 1.5, colour);
    }

    /// A straight line, smoothed.
    pub fn line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, colour: u32) {
        let s = self.scale;
        let (ax, ay, bx, by) = (a.0 * s, a.1 * s, b.0 * s, b.1 * s);
        let half = width * s / 2.0;
        let (x0, x1) = (ax.min(bx) - half - 1.0, ax.max(bx) + half + 1.0);
        let (y0, y1) = (ay.min(by) - half - 1.0, ay.max(by) + half + 1.0);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in (y0.max(0.0) as usize)..(y1.min(self.height as f32) as usize) {
            for x in (x0.max(0.0) as usize)..(x1.min(self.width as f32) as usize) {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
                let (qx, qy) = (ax + t * dx - px, ay + t * dy - py);
                let d = (qx * qx + qy * qy).sqrt() - half;
                let cover = (0.5 - d).clamp(0.0, 1.0);
                if cover > 0.0 {
                    let i = y * self.width + x;
                    self.pixels[i] = blend(self.pixels[i], colour, (cover * 255.0) as u8);
                }
            }
        }
    }

    fn blit(&mut self, g: &Glyph, gx: i32, gy: i32, fg: u32, clip: (i32, i32, i32, i32)) {
        let (cx0, cy0, cx1, cy1) = clip;
        for row in 0..g.height as i32 {
            let y = gy + row;
            if y < cy0 || y >= cy1 {
                continue;
            }
            for col in 0..g.width as i32 {
                let x = gx + col;
                if x < cx0 || x >= cx1 {
                    continue;
                }
                let i = row as usize * g.width + col as usize;
                let at = y as usize * self.width + x as usize;
                self.pixels[at] = if g.colour {
                    let p = &g.data[i * 4..i * 4 + 4];
                    let rgb = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                    blend(self.pixels[at], rgb, p[3])
                } else {
                    blend(self.pixels[at], fg, g.data[i])
                };
            }
        }
    }
}

pub fn blend(bg: u32, fg: u32, alpha: u8) -> u32 {
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

/// An SVG drawn at one size: premultiplied RGBA.
pub struct Picture {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

/// SVG icons, each drawn once per size and kept.
#[derive(Default)]
pub struct Pictures {
    drawn: HashMap<(std::path::PathBuf, u32), Option<Picture>>,
}

impl Pictures {
    /// `file` drawn to fit a square `px` physical pixels wide, or `None` if
    /// it can't be read.
    pub fn get(&mut self, file: &std::path::Path, px: u32) -> Option<&Picture> {
        let key = (file.to_path_buf(), px);
        if !self.drawn.contains_key(&key) {
            // A pack being edited makes new pictures; don't keep them all.
            if self.drawn.len() > 2000 {
                self.drawn.clear();
            }
            let picture = Self::draw(file, px);
            self.drawn.insert(key.clone(), picture);
        }
        self.drawn.get(&key)?.as_ref()
    }

    /// Forgets every picture, for when packs change on disk.
    pub fn clear(&mut self) {
        self.drawn.clear();
    }

    fn draw(file: &std::path::Path, px: u32) -> Option<Picture> {
        use resvg::{tiny_skia, usvg};
        let data = std::fs::read(file).ok()?;
        let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).ok()?;
        let size = tree.size();
        let scale = px as f32 / size.width().max(size.height());
        let (dx, dy) = (
            (px as f32 - size.width() * scale) / 2.0,
            (px as f32 - size.height() * scale) / 2.0,
        );
        let mut pixmap = tiny_skia::Pixmap::new(px.max(1), px.max(1))?;
        let transform = tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        Some(Picture {
            width: px as usize,
            height: px as usize,
            rgba: pixmap.take(),
        })
    }
}

impl Canvas<'_> {
    /// Draws a picture with its top left at logical (`x`, `y`): in its own
    /// colours, or as a shape in `tint`.
    pub fn picture(&mut self, pic: &Picture, x: f32, y: f32, tint: Option<u32>, clip: Rect) {
        let (ox, oy) = (
            (x * self.scale).round() as i32,
            (y * self.scale).round() as i32,
        );
        let (cx0, cy0, cx1, cy1) = self.physical(clip);
        for row in 0..pic.height as i32 {
            let py = oy + row;
            if py < cy0 || py >= cy1 {
                continue;
            }
            for col in 0..pic.width as i32 {
                let px = ox + col;
                if px < cx0 || px >= cx1 {
                    continue;
                }
                let i = (row as usize * pic.width + col as usize) * 4;
                let a = pic.rgba[i + 3];
                if a == 0 {
                    continue;
                }
                let at = py as usize * self.width + px as usize;
                self.pixels[at] = match tint {
                    Some(colour) => blend(self.pixels[at], colour, a),
                    None => {
                        // Premultiplied: the colour plus what shows through.
                        let dst = self.pixels[at];
                        let keep = 255 - a as u32;
                        [(16, 0), (8, 1), (0, 2)].iter().fold(0, |out, (shift, k)| {
                            let s = pic.rgba[i + k] as u32;
                            let d = (dst >> shift) & 0xff;
                            out | ((s + d * keep / 255).min(255) << shift)
                        })
                    }
                };
            }
        }
    }
}
