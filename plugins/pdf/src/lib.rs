//! PDF files in a tab. The editor scrolls and zooms; this draws the parts
//! of pages that come into view, with hayro, and says where their words
//! are, so selecting, Find and Copy work as in any tab.

use std::collections::HashMap;
use std::sync::Arc;

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::{Affine, BezPath, Point, Rect};
use hayro::vello_cpu::RenderContext;
use merlin_plugin::prelude::*;

/// What a glyph's outline is measured in.
const UNITS_PER_EM: f64 = 1000.0;

#[derive(Default)]
struct PdfPlugin {
    docs: HashMap<u32, Doc>,
}

struct Doc {
    pdf: Pdf,
    /// Each page's words, once asked for.
    text: HashMap<u32, Vec<TextBox>>,
}

impl Plugin for PdfPlugin {
    fn open_document(
        &mut self,
        surface: u32,
        _editor: &str,
        _path: &str,
        data: Vec<u8>,
    ) -> Result<Vec<PageSize>, String> {
        let pdf = Pdf::new(Arc::new(data)).map_err(|e| format!("not a PDF it can read: {e:?}"))?;
        let sizes = pdf
            .pages()
            .iter()
            .map(|page| {
                let (width, height) = page.render_dimensions();
                PageSize { width, height }
            })
            .collect();
        self.docs.insert(
            surface,
            Doc {
                pdf,
                text: HashMap::new(),
            },
        );
        Ok(sizes)
    }

    fn render_tile(&mut self, surface: u32, tile: Tile) -> Result<RgbaPicture, String> {
        let doc = self.docs.get(&surface).ok_or("no such document")?;
        let pages = doc.pdf.pages();
        let page = pages.get(tile.page as usize).ok_or("no such page")?;
        let (w, h) = (tile.width.clamp(1, 4096), tile.height.clamp(1, 4096));
        let mut ctx = RenderContext::new(w as u16, h as u16);
        let transform = Affine::translate((-(tile.x as f64), -(tile.y as f64)))
            * Affine::scale(tile.scale as f64)
            * page.initial_transform(true).to_kurbo();
        let cache = hayro::RenderCache::new();
        hayro::render_into(
            page,
            &cache,
            &InterpreterSettings::default(),
            &hayro::RenderSettings::default(),
            &mut ctx,
            transform,
        );
        ctx.flush();
        let mut pixmap = hayro::vello_cpu::Pixmap::new(w as u16, h as u16);
        ctx.render_with(
            &mut pixmap,
            &mut hayro::vello_cpu::Resources::default(),
            hayro::vello_cpu::RasterizerSettings {
                target_init: hayro::vello_cpu::TargetInit::Clear(
                    hayro::vello_cpu::color::palette::css::WHITE,
                ),
                ..Default::default()
            },
        );
        // On white, every pixel is opaque, so premultiplied is plain.
        Ok(RgbaPicture {
            width: w,
            height: h,
            pixels: pixmap.data_as_u8_slice().to_vec(),
        })
    }

    fn page_text(&mut self, surface: u32, page: u32) -> Vec<TextBox> {
        let Some(doc) = self.docs.get_mut(&surface) else {
            return Vec::new();
        };
        if let Some(text) = doc.text.get(&page) {
            return text.clone();
        }
        let pages = doc.pdf.pages();
        let Some(p) = pages.get(page as usize) else {
            return Vec::new();
        };
        let (w, h) = p.render_dimensions();
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            p.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, w as f64, h as f64),
            &cache,
            p.xref(),
            InterpreterSettings::default(),
        );
        let mut words = Words::default();
        interpret_page(p, &mut context, &mut words);
        let boxes = words.finish();
        doc.text.insert(page, boxes.clone());
        boxes
    }

    fn close_document(&mut self, surface: u32) {
        self.docs.remove(&surface);
    }
}

/// A glyph placed on the page: its text, where its baseline starts, and
/// its size.
struct Placed {
    text: String,
    origin: Point,
    size: f64,
}

/// A device that draws nothing and keeps where each glyph went, then
/// joins them into words.
#[derive(Default)]
struct Words {
    glyphs: Vec<Placed>,
}

impl Words {
    /// Glyphs on one line, close together and not spaces, make a word.
    fn finish(self) -> Vec<TextBox> {
        let mut out: Vec<TextBox> = Vec::new();
        let mut word: Option<(String, f64, f64, f64, f64)> = None; // text, x0, x1, base, size
        let flush = |word: &mut Option<(String, f64, f64, f64, f64)>, out: &mut Vec<TextBox>| {
            if let Some((text, x0, x1, base, size)) = word.take()
                && !text.trim().is_empty()
            {
                out.push(TextBox {
                    x: x0 as f32,
                    y: (base - size * 0.8) as f32,
                    width: (x1 - x0).max(size * 0.2) as f32,
                    height: size as f32,
                    text,
                });
            }
        };
        for (i, g) in self.glyphs.iter().enumerate() {
            // How wide: up to the next glyph on the line, or half its size.
            let next = self.glyphs.get(i + 1).filter(|n| {
                (n.origin.y - g.origin.y).abs() < g.size * 0.3 && n.origin.x > g.origin.x
            });
            let end = next.map_or(g.origin.x + g.size * 0.5, |n| {
                n.origin.x.min(g.origin.x + g.size * 1.2)
            });
            if g.text.chars().all(char::is_whitespace) {
                flush(&mut word, &mut out);
                continue;
            }
            match &mut word {
                Some((text, _, x1, base, size))
                    if (g.origin.y - *base).abs() < *size * 0.3
                        && g.origin.x - *x1 < *size * 0.25
                        && g.origin.x >= *x1 - *size * 0.5 =>
                {
                    text.push_str(&g.text);
                    *x1 = end;
                }
                _ => {
                    flush(&mut word, &mut out);
                    word = Some((g.text.clone(), g.origin.x, end, g.origin.y, g.size));
                }
            }
        }
        flush(&mut word, &mut out);
        out
    }
}

impl<'a> Device<'a> for Words {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for glyph in run.glyphs() {
            let m = props.transform * glyph.transform();
            let origin = m * Point::ZERO;
            let up = m * Point::new(0.0, UNITS_PER_EM) - origin;
            let text = match glyph.as_unicode() {
                Some(hayro::hayro_interpret::hayro_cmap::BfString::Char(c)) => c.to_string(),
                Some(hayro::hayro_interpret::hayro_cmap::BfString::String(s)) => s,
                None => continue,
            };
            self.glyphs.push(Placed {
                text,
                origin,
                size: up.hypot(),
            });
        }
    }
}

merlin_plugin::export!(PdfPlugin);
