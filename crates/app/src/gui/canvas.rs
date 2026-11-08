//! Drawing in logical pixels: filled and rounded rectangles, lines,
//! pictures, and text in the interface's font or the code's, shaped once
//! and kept. It's recorded as a [`Scene`] for a backend to draw.

use std::collections::HashMap;
use std::env;

use super::scene::{Bitmap, BitmapId, Bitmaps, Clip, Edges, Kind, Prim, Scene};
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

    /// The part inside both.
    pub fn intersect(&self, o: Rect) -> Rect {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        let (r, b) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
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

/// A glyph's bitmap and where it sits from the pen.
#[derive(Clone, Copy)]
struct Glyph {
    bitmap: BitmapId,
    left: i32,
    top: i32,
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
    /// The bitmaps' generation the glyphs belong to.
    generation: u64,
}

impl Text {
    /// Fonts for the interface and for icons from the code's font, as the
    /// packs ask for them, unless `MERLIN_UI_FONT` or `MERLIN_FONT` say.
    pub fn new(scale: f32, ui: Option<&str>, code: Option<&str>) -> Self {
        let mut system = super::fonts::system();
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
            generation: u64::MAX,
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

    /// Logical height of a single line of `text`.
    pub fn height(&mut self, text: &str, face: Face) -> f32 {
        self.shape(text, face, None).height
    }

    /// The cursor position (x, y, line_height) for `cursor` (byte offset) in `text`.
    pub fn cursor_pos(
        &mut self,
        text: &str,
        face: Face,
        wrap: Option<f32>,
        cursor: usize,
    ) -> (f32, f32, f32) {
        let wrap_px = wrap.map(|w| (w * self.scale) as u32);
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

        let s = self.scale;
        let cursor = cursor.min(text.len());
        let target_line = text[..cursor].chars().filter(|c| *c == '\n').count();
        let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_cursor = cursor - line_start;

        let mut fallback_top = 0.0;
        let mut fallback_height = line;
        let mut fallback_x = 0.0;

        for run in buffer.layout_runs() {
            fallback_top = run.line_top;
            fallback_height = run.line_height;
            fallback_x = run.line_w;

            if run.line_i < target_line {
                continue;
            }
            if run.line_i > target_line {
                break;
            }

            if run.glyphs.is_empty() {
                return (0.0, run.line_top / s, run.line_height / s);
            }

            let first_start = run.glyphs.first().unwrap().start;
            let last_end = run.glyphs.last().unwrap().end;

            if line_cursor < first_start {
                return (0.0, run.line_top / s, run.line_height / s);
            }

            if line_cursor <= last_end {
                for g in run.glyphs {
                    if line_cursor <= g.start {
                        return (g.x / s, run.line_top / s, run.line_height / s);
                    }
                    if line_cursor <= g.end {
                        let ratio =
                            (line_cursor - g.start) as f32 / (g.end - g.start).max(1) as f32;
                        return (
                            (g.x + g.w * ratio) / s,
                            run.line_top / s,
                            run.line_height / s,
                        );
                    }
                }
                let last_g = run.glyphs.last().unwrap();
                return (
                    (last_g.x + last_g.w) / s,
                    run.line_top / s,
                    run.line_height / s,
                );
            }
            let last_g = run.glyphs.last().unwrap();
            fallback_x = last_g.x + last_g.w;
        }

        (fallback_x / s, fallback_top / s, fallback_height / s)
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

    fn glyph(&mut self, key: CacheKey, bitmaps: &mut Bitmaps) -> Option<Glyph> {
        if self.generation != bitmaps.generation() {
            self.glyphs.clear();
            self.generation = bitmaps.generation();
        }
        if !self.glyphs.contains_key(&key) {
            let glyph = self
                .raster
                .get_image_uncached(&mut self.system, key)
                .map(|image| Glyph {
                    left: image.placement.left,
                    top: image.placement.top,
                    bitmap: bitmaps.add(glyph_bitmap(
                        image.placement.width,
                        image.placement.height,
                        image.content,
                        image.data,
                    )),
                });
            self.glyphs.insert(key, glyph);
        }
        self.glyphs[&key]
    }

    /// Physical pixels to a logical one, as `draw` places text.
    pub fn scale(&self) -> f32 {
        self.scale
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
            let Some(glyph) = self.glyph(key, canvas.bitmaps) else {
                continue;
            };
            canvas.scene.prims.push(Prim::Glyph {
                bitmap: glyph.bitmap,
                pos: [ox + gx + glyph.left, oy + gy - glyph.top],
                colour,
                clip,
            });
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

/// A glyph's picture from swash as a bitmap: coverage, or RGBA for a
/// colour glyph. Subpixel masks would need the panel's layout; their
/// average is plain coverage.
pub fn glyph_bitmap(width: u32, height: u32, content: SwashContent, data: Vec<u8>) -> Bitmap {
    let (kind, data) = match content {
        SwashContent::Color => (Kind::Rgba, data),
        SwashContent::SubpixelMask => (
            Kind::Mask,
            data.chunks(4)
                .map(|p| ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8)
                .collect(),
        ),
        SwashContent::Mask => (Kind::Mask, data),
    };
    Bitmap {
        width,
        height,
        kind,
        data,
    }
}

/// The window's drawing, recorded in logical coordinates as a scene in
/// physical pixels.
pub struct Canvas<'a> {
    pub scene: &'a mut Scene,
    pub bitmaps: &'a mut Bitmaps,
    pub width: usize,
    pub height: usize,
    pub scale: f32,
}

impl Canvas<'_> {
    /// A logical rectangle in whole physical pixels: (x0, y0, x1, y1).
    fn physical(&self, r: Rect) -> Clip {
        let s = self.scale;
        [
            ((r.x * s).round() as i32).clamp(0, self.width as i32),
            ((r.y * s).round() as i32).clamp(0, self.height as i32),
            (((r.x + r.w) * s).round() as i32).clamp(0, self.width as i32),
            (((r.y + r.h) * s).round() as i32).clamp(0, self.height as i32),
        ]
    }

    /// A logical rectangle's edges in physical pixels.
    fn edges(&self, r: Rect) -> Edges {
        let s = self.scale;
        [r.x * s, r.y * s, (r.x + r.w) * s, (r.y + r.h) * s]
    }

    pub fn fill(&mut self, r: Rect, colour: u32) {
        self.round(r, 0.0, colour);
    }

    /// A rectangle with corners rounded to `radius`, smoothed at the edges.
    pub fn round(&mut self, r: Rect, radius: f32, colour: u32) {
        self.tint(r, radius, colour, 255);
    }

    /// A rounded outline `width` wide.
    pub fn outline(&mut self, r: Rect, radius: f32, width: f32, colour: u32, inside: u32) {
        let s = self.scale;
        self.scene.prims.push(Prim::Quad {
            rect: self.edges(r),
            radius: radius * s,
            fill: inside,
            alpha: 255,
            border: colour,
            border_w: width * s,
        });
    }

    /// A rounded rectangle edged in `colour`, and filled with it at
    /// `alpha`, so what's below shows through.
    pub fn frame(&mut self, r: Rect, radius: f32, width: f32, colour: u32, alpha: u8) {
        let s = self.scale;
        self.scene.prims.push(Prim::Quad {
            rect: self.edges(r),
            radius: radius * s,
            fill: colour,
            alpha,
            border: colour,
            border_w: width * s,
        });
    }

    /// A soft shadow under a floating box; lighter for a light theme.
    pub fn shadow(&mut self, r: Rect, radius: f32, light: bool) {
        let s = self.scale;
        self.scene.prims.push(Prim::Shadow {
            rect: self.edges(r),
            radius: radius * s,
            blur: 10.0 * s,
            light,
        });
    }

    /// Darkens or tints what's drawn already, through a rounded shape.
    pub fn tint(&mut self, r: Rect, radius: f32, colour: u32, alpha: u8) {
        let s = self.scale;
        self.scene.prims.push(Prim::Quad {
            rect: self.edges(r),
            radius: radius.max(0.0) * s,
            fill: colour,
            alpha,
            border: 0,
            border_w: 0.0,
        });
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
        self.scene.prims.push(Prim::Line {
            a: [a.0 * s, a.1 * s],
            b: [b.0 * s, b.1 * s],
            width: width * s,
            colour,
        });
    }

    /// Draws a picture with its top left at logical (`x`, `y`): in its own
    /// colours, or as a shape in `tint`.
    pub fn picture(&mut self, pic: Picture, x: f32, y: f32, tint: Option<u32>, clip: Rect) {
        let pos = [
            (x * self.scale).round() as i32,
            (y * self.scale).round() as i32,
        ];
        let clip = self.physical(clip);
        self.scene.prims.push(Prim::Image {
            bitmap: pic.bitmap,
            pos,
            tint,
            clip,
        });
    }
}

/// An SVG drawn at one size, kept in the bitmaps.
#[derive(Clone, Copy)]
pub struct Picture {
    bitmap: BitmapId,
}

/// SVG icons, each drawn once per size and kept until the bitmaps are
/// cleared.
#[derive(Default)]
pub struct Pictures {
    drawn: HashMap<(std::path::PathBuf, u32), Option<Picture>>,
    /// Plugins' pictures, scaled: by version and size in physical pixels.
    scaled: HashMap<(u64, u32, u32), Picture>,
    generation: u64,
}

impl Pictures {
    /// `file` drawn to fit a square `px` physical pixels wide, or `None` if
    /// it can't be read.
    pub fn get(
        &mut self,
        file: &std::path::Path,
        px: u32,
        bitmaps: &mut Bitmaps,
    ) -> Option<Picture> {
        if self.generation != bitmaps.generation() {
            self.drawn.clear();
            self.generation = bitmaps.generation();
        }
        let key = (file.to_path_buf(), px);
        if !self.drawn.contains_key(&key) {
            let picture = Self::draw(file, px).map(|bitmap| Picture {
                bitmap: bitmaps.add(bitmap),
            });
            self.drawn.insert(key.clone(), picture);
        }
        *self.drawn.get(&key)?
    }

    /// A plugin's picture scaled to `w` by `h` physical pixels, its corners
    /// rounded to `radius`, scaled once for each size.
    pub fn plugin(
        &mut self,
        image: &merlin_core::PluginImage,
        w: u32,
        h: u32,
        radius: f32,
        bitmaps: &mut Bitmaps,
    ) -> Picture {
        if self.generation != bitmaps.generation() {
            self.drawn.clear();
            self.scaled.clear();
            self.generation = bitmaps.generation();
        }
        // Old versions' sizes go now and then; their bitmaps go when the
        // bitmaps start again.
        if self.scaled.len() > 256 {
            self.scaled.clear();
        }
        let key = (image.version, w.max(1), h.max(1));
        *self.scaled.entry(key).or_insert_with(|| {
            let mut data = scale(&image.pixels, image.width, image.height, key.1, key.2);
            round_corners(&mut data, key.1, key.2, radius);
            Picture {
                bitmap: bitmaps.add(Bitmap {
                    width: key.1,
                    height: key.2,
                    kind: Kind::Premul,
                    data,
                }),
            }
        })
    }

    fn draw(file: &std::path::Path, px: u32) -> Option<Bitmap> {
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
        Some(Bitmap {
            width: px.max(1),
            height: px.max(1),
            kind: Kind::Premul,
            data: pixmap.take(),
        })
    }
}

/// Premultiplied RGBA scaled from `sw` by `sh` to `dw` by `dh`, each
/// pixel the average of the source it covers: smooth going down, square
/// pixels going up.
pub fn scale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if (sw, sh) == (dw, dh) {
        return src.to_vec();
    }
    // What each output column or row takes of the source's: (first, weights).
    let spans = |from: u32, to: u32| -> Vec<(usize, Vec<f32>)> {
        let step = from as f32 / to as f32;
        (0..to)
            .map(|i| {
                let (a, b) = (i as f32 * step, (i as f32 + 1.0) * step);
                let first = a.floor() as usize;
                let last = (b.ceil() as usize).min(from as usize).max(first + 1);
                let weights: Vec<f32> = (first..last)
                    .map(|j| (b.min(j as f32 + 1.0) - a.max(j as f32)).max(0.0) / step)
                    .collect();
                (first, weights)
            })
            .collect()
    };
    let (xs, ys) = (spans(sw, dw), spans(sh, dh));
    // Across, then down.
    let mut across = vec![0f32; dw as usize * sh as usize * 4];
    for y in 0..sh as usize {
        for (x, (first, weights)) in xs.iter().enumerate() {
            let out = &mut across[(y * dw as usize + x) * 4..][..4];
            for (k, w) in weights.iter().enumerate() {
                let i = (y * sw as usize + first + k) * 4;
                for c in 0..4 {
                    out[c] += src[i + c] as f32 * w;
                }
            }
        }
    }
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    for (y, (first, weights)) in ys.iter().enumerate() {
        for x in 0..dw as usize {
            let mut px = [0f32; 4];
            for (k, w) in weights.iter().enumerate() {
                let i = ((first + k) * dw as usize + x) * 4;
                for c in 0..4 {
                    px[c] += across[i + c] * w;
                }
            }
            let o = (y * dw as usize + x) * 4;
            for c in 0..4 {
                out[o + c] = px[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Fades premultiplied RGBA out past corners rounded to `radius`.
fn round_corners(data: &mut [u8], w: u32, h: u32, radius: f32) {
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    if r <= 0.0 {
        return;
    }
    let n = r.ceil() as u32;
    for y in 0..n.min(h) {
        for x in 0..n.min(w) {
            // Distance past the corner's circle, from the pixel's middle.
            let (dx, dy) = (r - (x as f32 + 0.5), r - (y as f32 + 0.5));
            let cover = (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if cover >= 1.0 {
                continue;
            }
            for (cx, cy) in [
                (x, y),
                (w - 1 - x, y),
                (x, h - 1 - y),
                (w - 1 - x, h - 1 - y),
            ] {
                let i = ((cy * w + cx) * 4) as usize;
                for c in &mut data[i..i + 4] {
                    *c = (*c as f32 * cover).round() as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_scale_by_the_area_they_cover() {
        // Two by one, white then clear, halved: half white.
        let src = [255, 255, 255, 255, 0, 0, 0, 0];
        assert_eq!(scale(&src, 2, 1, 1, 1), [128, 128, 128, 128]);
        // Doubled: square pixels.
        let up = scale(&src, 2, 1, 4, 1);
        assert_eq!(&up[..8], &[255; 8]);
        assert_eq!(&up[8..], &[0; 8]);
        // A corner rounded away, the middle left.
        let mut solid = vec![255u8; 10 * 10 * 4];
        round_corners(&mut solid, 10, 10, 4.0);
        assert_eq!(solid[3], 0);
        assert_eq!(solid[(5 * 10 + 5) * 4 + 3], 255);
    }

    #[test]
    fn cursor_pos_multiline_and_wrapped() {
        let mut text = Text::new(1.0, None, None);
        let face = Face::ui(13.0);

        // Empty text
        let (x, y, h) = text.cursor_pos("", face, Some(200.0), 0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);
        assert!(h > 10.0);

        // Single line
        let (x0, y0, _) = text.cursor_pos("hello", face, Some(200.0), 0);
        assert_eq!(x0, 0.0);
        assert_eq!(y0, 0.0);

        let (x5, y5, _) = text.cursor_pos("hello", face, Some(200.0), 5);
        assert!(x5 > 0.0);
        assert_eq!(y5, 0.0);

        // Multi-line with newline
        let s = "hello\nworld";
        let (x_end0, y_end0, _) = text.cursor_pos(s, face, Some(200.0), 5);
        assert_eq!(x_end0, x5);
        assert_eq!(y_end0, 0.0);

        let (x_start1, y_start1, _) = text.cursor_pos(s, face, Some(200.0), 6);
        assert_eq!(x_start1, 0.0);
        assert!(y_start1 > 10.0);

        let (x_end1, y_end1, _) = text.cursor_pos(s, face, Some(200.0), 11);
        assert!(x_end1 > 0.0);
        assert_eq!(y_end1, y_start1);

        // Trailing newline
        let s_trail = "hello\n";
        let (x_nl, y_nl, _) = text.cursor_pos(s_trail, face, Some(200.0), 6);
        assert_eq!(x_nl, 0.0);
        assert!(y_nl > 10.0);
    }
}
