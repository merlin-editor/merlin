//! A document a plugin draws, such as a PDF: its toolbar, its pages'
//! thumbnails, and its pages, from the tiles the plugin drew. A tile not
//! there yet shows the page's thumbnail stretched, or hatching until that
//! comes; the tiles it lacks are asked for once the frame is drawn.

use super::*;
use merlin_core::{
    PX_PER_PT, PageSize, Pages, SurfaceAction, SurfaceView, THUMB, TILE, TileKey, page_layout,
};

/// The toolbar's height, and the thumbnails' column's width.
const BAR_H: f32 = 34.0;
const THUMBS_W: f32 = 112.0;

/// A document as drawn: where its pages went, for the mouse, and what it
/// lacks.
#[derive(Clone, Debug)]
pub struct SurfaceSeen {
    pub id: u32,
    pub plugin: String,
    /// The pages' room, and whether the editor knows its size.
    pub area: Rect,
    pub sized: bool,
    /// Each page shown and its rect, at `zoom`.
    pub pages: Vec<(u32, Rect)>,
    pub zoom: f32,
    /// Tiles it showed without having, the ones in view first.
    pub wanted: Vec<TileKey>,
}

impl SurfaceSeen {
    /// The page under logical (`x`, `y`), and the point on it in points.
    pub fn point(&self, x: f32, y: f32) -> Option<(u32, f32, f32)> {
        let k = PX_PER_PT * self.zoom;
        // The page it's on, or else the nearest above or below.
        let (page, r) = self
            .pages
            .iter()
            .min_by(|(_, a), (_, b)| {
                let d = |r: &Rect| (r.y - y).max(y - r.bottom()).max(0.0);
                d(a).total_cmp(&d(b))
            })
            .copied()?;
        Some((page, (x - r.x) / k, (y - r.y) / k))
    }
}

/// A page's thumbnail, the whole page a thumbnail wide.
fn thumb_key(page: u32, size: &PageSize) -> TileKey {
    let scale = THUMB as f32 / size.width.max(1.0);
    TileKey {
        page,
        x: 0,
        y: 0,
        width: THUMB,
        height: (size.height * scale).round().max(1.0) as u32,
        scale: (scale * 1000.0).round() as u32,
    }
}

impl Frame<'_> {
    /// A document's tab in `r`: its toolbar, its thumbnails if shown, and
    /// its pages.
    pub(crate) fn surface(&mut self, sv: &SurfaceView, r: Rect) -> SurfaceSeen {
        let p = self.p;
        self.canvas.fill(r, p.island);
        let bar = Rect::new(r.x, r.y, r.w, BAR_H);
        self.surface_bar(sv, bar);
        self.canvas
            .fill(Rect::new(r.x, bar.bottom() - 1.0, r.w, 1.0), p.border);
        let mut body = Rect::new(r.x, bar.bottom(), r.w, (r.h - BAR_H).max(0.0));
        let mut seen = SurfaceSeen {
            id: sv.id,
            plugin: sv.plugin.clone(),
            area: body,
            sized: false,
            pages: Vec::new(),
            zoom: sv.zoom,
            wanted: Vec::new(),
        };
        let pages = match &sv.pages {
            Pages::Ready(pages) => pages,
            Pages::Opening => {
                self.surface_note("Opening…", p.muted, body);
                return seen;
            }
            Pages::Failed(why) => {
                self.surface_note(why, p.error, body);
                return seen;
            }
        };
        if sv.thumbs && body.w > THUMBS_W * 3.0 {
            let strip = Rect::new(body.x, body.y, THUMBS_W, body.h);
            self.surface_thumbs(sv, pages, strip, &mut seen.wanted);
            self.canvas.fill(
                Rect::new(strip.right() - 1.0, strip.y, 1.0, strip.h),
                p.border,
            );
            body = Rect::new(strip.right(), body.y, body.w - THUMBS_W, body.h);
        }
        seen.area = body;
        seen.sized = sv.area == (body.w, body.h);
        self.surface_pages(sv, pages, body, &mut seen);
        seen
    }

    /// A line in the middle of the pages' room: Opening…, or why it can't.
    fn surface_note(&mut self, text: &str, colour: u32, r: Rect) {
        let f = Face::ui(12.5);
        let text = self.text.fit(text, f, (r.w - 40.0).max(20.0));
        let w = self.text.width(&text, f);
        self.text.draw_in_row(
            &mut self.canvas,
            &text,
            f,
            r.x + (r.w - w) / 2.0,
            r.y + r.h / 3.0,
            20.0,
            colour,
            r,
        );
    }

    /// The toolbar: thumbnails, the page and its arrows, the zoom, Fit
    /// Width, and which plugin draws it.
    fn surface_bar(&mut self, sv: &SurfaceView, r: Rect) {
        let p = self.p;
        let id = sv.id;
        let y = r.y + 5.0;
        let mut x = r.x + 10.0;
        let ui = |action| Hit::Ui(UiEvent::Surface { id, action });
        x += self.surface_tool("▤", x, y, sv.thumbs, ui(SurfaceAction::Thumbs), r) + 10.0;
        self.canvas
            .fill(Rect::new(x, r.y + 9.0, 1.0, 16.0), p.border);
        x += 11.0;
        let count = match &sv.pages {
            Pages::Ready(pages) => pages.len(),
            _ => 0,
        };
        x += self.surface_tool("‹", x, y, false, ui(SurfaceAction::Previous), r) + 6.0;
        // The page, in a small box, then how many.
        let f = Face::ui(12.0);
        let now = (sv.current + 1).min(count.max(1)).to_string();
        let nw = self.text.width(&now, f).max(10.0) + 16.0;
        let boxed = Rect::new(x, y, nw, 24.0);
        self.canvas.outline(boxed, 5.0, 1.0, p.border, p.chip);
        self.text
            .draw_in_row(&mut self.canvas, &now, f, x + 8.0, y, 24.0, p.bright, r);
        x += nw + 8.0;
        let of = format!("of {count}");
        x += self
            .text
            .draw_in_row(&mut self.canvas, &of, f, x, y, 24.0, p.muted, r)
            + 6.0;
        x += self.surface_tool("›", x, y, false, ui(SurfaceAction::Next), r) + 10.0;
        self.canvas
            .fill(Rect::new(x, r.y + 9.0, 1.0, 16.0), p.border);
        x += 11.0;
        x += self.surface_tool("−", x, y, false, ui(SurfaceAction::ZoomOut), r) + 6.0;
        let zoom = format!("{}%", (sv.zoom * 100.0).round());
        x += self
            .text
            .draw_in_row(&mut self.canvas, &zoom, f, x, y, 24.0, p.text, r)
            + 6.0;
        x += self.surface_tool("+", x, y, false, ui(SurfaceAction::ZoomIn), r) + 6.0;
        self.surface_tool("Fit Width", x, y, sv.fit, ui(SurfaceAction::Fit), r);
        // Who draws it, at the right.
        let from = format!("{} plugin", sv.from);
        let fw = self.text.width(&from, Face::ui(11.0));
        let fx = r.right() - 12.0 - fw;
        if fx > x + 90.0 {
            self.text.draw_in_row(
                &mut self.canvas,
                &from,
                Face::ui(11.0),
                fx,
                y,
                24.0,
                p.muted,
                r,
            );
        }
    }

    /// A toolbar button: its label on a small tile when on or hovered.
    /// Returns its width.
    fn surface_tool(&mut self, label: &str, x: f32, y: f32, on: bool, hit: Hit, clip: Rect) -> f32 {
        let p = self.p;
        let f = Face::ui(12.0);
        let w = (self.text.width(label, f) + 12.0).max(24.0);
        let b = Rect::new(x, y, w, 24.0);
        if on {
            self.canvas.round(b, 5.0, p.chip);
        } else if self.hovered(b) {
            self.canvas.round(b, 5.0, p.raised);
        }
        let tw = self.text.width(label, f);
        let colour = if on { p.bright } else { p.text };
        self.text.draw_in_row(
            &mut self.canvas,
            label,
            f,
            x + (w - tw) / 2.0,
            y,
            24.0,
            colour,
            clip,
        );
        self.hit(b, hit);
        w
    }

    /// The pages' thumbnails down the left, the current one outlined,
    /// scrolled to keep it in view.
    fn surface_thumbs(
        &mut self,
        sv: &SurfaceView,
        pages: &[PageSize],
        r: Rect,
        wanted: &mut Vec<TileKey>,
    ) {
        let p = self.p;
        let tw = 64.0;
        let step = |size: &PageSize| tw * size.height / size.width.max(1.0) + 26.0;
        // From the top, unless the current one would be past the bottom.
        let to_current: f32 = pages.iter().take(sv.current).map(step).sum();
        let mut y = r.y + 10.0
            - (to_current + step(&pages[sv.current.min(pages.len() - 1)]) - (r.h - 20.0)).max(0.0);
        for (i, size) in pages.iter().enumerate() {
            let th = tw * size.height / size.width.max(1.0);
            let t = Rect::new(r.x + (r.w - tw) / 2.0, y, tw, th);
            y += th + 26.0;
            if t.bottom() < r.y || t.y > r.bottom() {
                continue;
            }
            let key = thumb_key(i as u32, size);
            if i == sv.current {
                self.canvas
                    .outline(t.inset(-2.0), 3.0, 2.0, p.accent, 0xf4f2ec);
            }
            self.canvas.round(t, 2.0, 0xf4f2ec);
            match sv.tiles.get(&key) {
                Some(image) => self.surface_image(image, t, r),
                None => wanted.push(key),
            }
            let n = (i + 1).to_string();
            let nw = self.text.width(&n, Face::ui(10.5));
            self.text.draw_in_row(
                &mut self.canvas,
                &n,
                Face::ui(10.5),
                t.x + (tw - nw) / 2.0,
                t.bottom() + 2.0,
                18.0,
                p.muted,
                r,
            );
            self.hit(
                Rect::new(r.x, t.y - 4.0, r.w, th + 24.0).intersect(r),
                Hit::Ui(UiEvent::Surface {
                    id: sv.id,
                    action: SurfaceAction::Page(i),
                }),
            );
        }
    }

    /// The pages in view: each its thumbnail stretched, then the tiles
    /// drawn at the screen's resolution over it, then what's selected and
    /// what Find matched.
    fn surface_pages(
        &mut self,
        sv: &SurfaceView,
        pages: &[PageSize],
        r: Rect,
        seen: &mut SurfaceSeen,
    ) {
        let p = self.p;
        self.canvas.fill(r, p.ground);
        self.hit(r, Hit::Surface(sv.id));
        let (rects, _) = page_layout(pages, sv.zoom, r.w);
        let s = self.canvas.scale;
        // Pixels a point for tiles, as the screen draws them.
        let scale = sv.zoom * PX_PER_PT * s;
        let scale_key = (scale * 1000.0).round() as u32;
        let mut thumbs = Vec::new();
        for (i, (rect, size)) in rects.iter().zip(pages).enumerate() {
            let page = Rect::new(
                r.x + rect[0] - sv.hscroll,
                r.y + rect[1] - sv.scroll,
                rect[2],
                rect[3],
            );
            if page.bottom() < r.y || page.y > r.bottom() {
                continue;
            }
            seen.pages.push((i as u32, page));
            let shown = page.intersect(r);
            // Its shadow kept inside the room: where the page runs past
            // it, the shadow's end is under the page.
            let inside = Rect::new(r.x, r.y + 16.0, r.w, (r.h - 32.0).max(0.0));
            let shade = page.intersect(inside);
            if shade.w > 0.0 && shade.h > 0.0 {
                self.canvas.shadow(shade, 2.0, false);
            }
            self.canvas.fill(shown, 0xffffff);
            let thumb = thumb_key(i as u32, size);
            let has_thumb = sv.tiles.get(&thumb).is_some();
            if let Some(image) = sv.tiles.get(&thumb) {
                self.surface_image(image, page, r);
            } else {
                thumbs.push(thumb);
            }
            // The tiles across and down the page, as many as show.
            let (pw, ph) = (
                (size.width * scale).ceil() as u32,
                (size.height * scale).ceil() as u32,
            );
            let mut pending = false;
            for ty in (0..ph).step_by(TILE as usize) {
                for tx in (0..pw).step_by(TILE as usize) {
                    let key = TileKey {
                        page: i as u32,
                        x: tx,
                        y: ty,
                        width: TILE.min(pw - tx),
                        height: TILE.min(ph - ty),
                        scale: scale_key,
                    };
                    let at = Rect::new(
                        page.x + tx as f32 / s,
                        page.y + ty as f32 / s,
                        key.width as f32 / s,
                        key.height as f32 / s,
                    );
                    if at.bottom() < r.y
                        || at.y > r.bottom()
                        || at.right() < r.x
                        || at.x > r.right()
                    {
                        continue;
                    }
                    match sv.tiles.get(&key) {
                        Some(image) => self.surface_image(image, at, r),
                        None => {
                            pending = true;
                            seen.wanted.push(key);
                        }
                    }
                }
            }
            // Nothing to show yet: hatched, saying so.
            if pending && !has_thumb {
                self.hatch(shown, 0xe4e1d8);
                let label = format!("Rendering page {}…", i + 1);
                let f = Face::ui(11.5);
                let lw = self.text.width(&label, f);
                let mid = Rect::new(
                    shown.x + (shown.w - lw) / 2.0 - 8.0,
                    shown.y + (shown.h.min(200.0) - 22.0) / 2.0,
                    lw + 16.0,
                    22.0,
                );
                self.canvas.round(mid, 4.0, 0xfaf9f5);
                self.text.draw_in_row(
                    &mut self.canvas,
                    &label,
                    f,
                    mid.x + 8.0,
                    mid.y,
                    22.0,
                    0x6d6a63,
                    r,
                );
            }
        }
        // What's selected, then Find's matches, the current one strongest.
        let k = PX_PER_PT * sv.zoom;
        let to_rect = |seen: &SurfaceSeen, (page, b): &(u32, [f32; 4])| {
            let (_, pr) = seen.pages.iter().find(|(p, _)| p == page)?;
            Some(Rect::new(pr.x + b[0] * k, pr.y + b[1] * k, b[2] * k, b[3] * k).intersect(r))
        };
        for b in &sv.selected {
            if let Some(hr) = to_rect(seen, b) {
                self.canvas.tint(hr, 1.0, p.accent, 90);
            }
        }
        for (i, b) in sv.hits.iter().enumerate() {
            if let Some(hr) = to_rect(seen, b) {
                if Some(i) == sv.hit {
                    self.canvas.frame(hr.inset(-1.0), 2.0, 1.0, 0xb8860b, 150);
                } else {
                    self.canvas.tint(hr, 1.0, p.warning, 110);
                }
            }
        }
        // Thumbnails stand in while tiles come, so they're asked for last.
        seen.wanted.extend(thumbs);
    }

    /// A drawn tile or thumbnail, fitted to `at`.
    fn surface_image(&mut self, image: &merlin_core::PluginImage, at: Rect, clip: Rect) {
        let s = self.canvas.scale;
        let (w, h) = ((at.w * s).round() as u32, (at.h * s).round() as u32);
        if w == 0 || h == 0 {
            return;
        }
        let picture = self.pictures.plugin(image, w, h, 0.0, self.canvas.bitmaps);
        self.canvas.picture(picture, at.x, at.y, None, clip);
    }
}
