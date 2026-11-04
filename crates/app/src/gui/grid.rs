//! The code's font: its family and size, the cell every character sits
//! in, and glyphs rasterised once and kept.

use std::collections::HashMap;
use std::env;

use cosmic_text::{
    Attrs, Buffer, Family, FeatureTag, FontFeatures, FontSystem, Metrics, Shaping,
    Style as FontStyle, SwashCache, Weight,
};

use super::canvas::glyph_bitmap;
use super::scene::{BitmapId, Bitmaps};

/// Points, before the display's scale.
pub const FONT_SIZE: f32 = 13.5;
/// Rows are this many times the font size, unless the settings say.
const LINE_HEIGHT: f32 = 1.55;
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
}

pub struct Fonts {
    pub(super) system: FontSystem,
    pub(super) cache: SwashCache,
    pub(super) family: Option<String>,
    /// Whether runs like `->` and `!=` join into one picture.
    pub(super) ligatures: bool,
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
    /// what the settings or packs ask for, else the first of [`FAMILIES`]
    /// installed. `line_height` is times the size below 8, else points.
    pub fn new(
        scale: f32,
        family: Option<&str>,
        size: Option<f32>,
        line_height: Option<f32>,
        ligatures: bool,
    ) -> Self {
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
        let cell_h = match line_height {
            Some(points) if points >= 8.0 => points * scale,
            Some(times) => size * times,
            None => size * LINE_HEIGHT,
        }
        .round()
        .max(size.ceil());
        let line = Buffer::new(&mut system, Metrics::new(size, cell_h));
        let mut fonts = Self {
            system,
            cache: SwashCache::new(),
            family,
            ligatures,
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

/// The face for a cell: the chosen family, bold or italic as it says,
/// with or without ligatures.
pub(super) fn attrs(
    family: &Option<String>,
    ligatures: bool,
    bold: bool,
    italic: bool,
) -> Attrs<'_> {
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
    if !ligatures {
        let mut off = FontFeatures::new();
        off.disable(FeatureTag::STANDARD_LIGATURES)
            .disable(FeatureTag::CONTEXTUAL_LIGATURES)
            .disable(FeatureTag::CONTEXTUAL_ALTERNATES);
        attrs = attrs.font_features(off);
    }
    attrs
}

impl Fonts {
    /// The cell's width and the baseline, from the font's `M`.
    fn measure(&mut self) {
        let attrs = attrs(&self.family, self.ligatures, false, false);
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
        let attrs = attrs(&self.family, self.ligatures, bold, italic);
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
            bitmap: bitmaps.add(glyph_bitmap(width, height, image.content, image.data)),
        })
    }
}
