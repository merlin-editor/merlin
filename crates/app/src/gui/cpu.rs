//! The CPU's backend: a scene drawn into 0RGB pixels, for softbuffer when
//! there's no GPU or `MERLIN_RENDERER=cpu` asks, and for tests.

use super::scene::{Bitmaps, Clip, Edges, Kind, Prim, Scene};

/// Draws `scene` over `pixels`, `width` by `height`.
pub fn draw(scene: &Scene, bitmaps: &Bitmaps, pixels: &mut [u32], width: usize, height: usize) {
    let mut c = Pixels {
        pixels,
        width,
        height,
    };
    for prim in &scene.prims {
        c.prim(prim, bitmaps);
    }
}

struct Pixels<'a> {
    pixels: &'a mut [u32],
    width: usize,
    height: usize,
}

impl Pixels<'_> {
    fn prim(&mut self, prim: &Prim, bitmaps: &Bitmaps) {
        match *prim {
            Prim::Quad {
                rect,
                radius,
                fill,
                alpha,
                border,
                border_w,
            } => {
                if border_w > 0.0 && alpha < 255 {
                    self.ring(rect, radius, border_w, border);
                    let inner = [
                        rect[0] + border_w,
                        rect[1] + border_w,
                        (rect[2] - border_w).max(rect[0] + border_w),
                        (rect[3] - border_w).max(rect[1] + border_w),
                    ];
                    self.tint(inner, (radius - border_w).max(0.0), fill, alpha);
                } else if border_w > 0.0 {
                    self.round(rect, radius, border);
                    let inner = [
                        rect[0] + border_w,
                        rect[1] + border_w,
                        (rect[2] - border_w).max(rect[0] + border_w),
                        (rect[3] - border_w).max(rect[1] + border_w),
                    ];
                    self.round(inner, (radius - border_w).max(0.0), fill);
                } else if alpha == 255 {
                    self.round(rect, radius, fill);
                } else {
                    self.tint(rect, radius, fill, alpha);
                }
            }
            Prim::Shadow {
                rect,
                radius,
                blur,
                light,
            } => {
                for quad in Prim::shadow_quads(rect, radius, blur, light) {
                    self.prim(&quad, bitmaps);
                }
            }
            Prim::Line {
                a,
                b,
                width,
                colour,
            } => self.line(a, b, width, colour),
            Prim::Glyph {
                bitmap,
                pos,
                colour,
                clip,
            } => self.blit(bitmaps, bitmap, pos, Some(colour), clip),
            Prim::Image {
                bitmap,
                pos,
                tint,
                clip,
            } => self.blit(bitmaps, bitmap, pos, tint, clip),
        }
    }

    /// Edges in whole pixels, inside the window.
    fn snap(&self, r: Edges) -> (i32, i32, i32, i32) {
        (
            (r[0].round() as i32).clamp(0, self.width as i32),
            (r[1].round() as i32).clamp(0, self.height as i32),
            (r[2].round() as i32).clamp(0, self.width as i32),
            (r[3].round() as i32).clamp(0, self.height as i32),
        )
    }

    fn fill(&mut self, r: Edges, colour: u32) {
        let (x0, y0, x1, y1) = self.snap(r);
        for y in y0..y1 {
            let row = y as usize * self.width;
            self.pixels[row + x0 as usize..row + x1 as usize].fill(colour);
        }
    }

    /// A rectangle with corners rounded to `radius`, smoothed at the edges.
    fn round(&mut self, r: Edges, radius: f32, colour: u32) {
        if radius <= 0.0 {
            return self.fill(r, colour);
        }
        let [fx0, fy0, fx1, fy1] = r;
        let rad = radius.min((fx1 - fx0) / 2.0).min((fy1 - fy0) / 2.0);
        let (x0, y0, x1, y1) = self.snap(r);
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

    /// Darkens or tints what's drawn already, through a rounded shape.
    fn tint(&mut self, r: Edges, radius: f32, colour: u32, alpha: u8) {
        let [fx0, fy0, fx1, fy1] = r;
        let rad = radius
            .min((fx1 - fx0) / 2.0)
            .min((fy1 - fy0) / 2.0)
            .max(0.0);
        let (x0, y0, x1, y1) = self.snap(r);
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let dy = (fy0 + rad - py).max(py - (fy1 - rad)).max(0.0);
            let row = y as usize * self.width;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let dx = (fx0 + rad - px).max(px - (fx1 - rad)).max(0.0);
                let d = (dx * dx + dy * dy).sqrt() - rad;
                let cover = if rad > 0.0 {
                    (0.5 - d).clamp(0.0, 1.0)
                } else {
                    1.0
                };
                if cover > 0.0 {
                    let i = row + x as usize;
                    self.pixels[i] = blend(self.pixels[i], colour, (cover * alpha as f32) as u8);
                }
            }
        }
    }

    /// Only the outer `width` of a rounded rectangle, in `colour`, leaving
    /// what's inside as it was.
    fn ring(&mut self, r: Edges, radius: f32, width: f32, colour: u32) {
        let [fx0, fy0, fx1, fy1] = r;
        let rad = radius
            .min((fx1 - fx0) / 2.0)
            .min((fy1 - fy0) / 2.0)
            .max(0.0);
        let inner = (rad - width).max(0.0);
        let (x0, y0, x1, y1) = self.snap(r);
        // How far a point is inside the rectangle cut in by `by`, with
        // corners of `rad`: above 0.5 is in.
        let inside = |px: f32, py: f32, by: f32, rad: f32| {
            let dx = (fx0 + by + rad - px).max(px - (fx1 - by - rad)).max(0.0);
            let dy = (fy0 + by + rad - py).max(py - (fy1 - by - rad)).max(0.0);
            (0.5 - ((dx * dx + dy * dy).sqrt() - rad)).clamp(0.0, 1.0)
        };
        for y in y0..y1 {
            let py = y as f32 + 0.5;
            let row = y as usize * self.width;
            for x in x0..x1 {
                let px = x as f32 + 0.5;
                let cover = inside(px, py, 0.0, rad) - inside(px, py, width, inner);
                if cover > 0.0 {
                    let i = row + x as usize;
                    self.pixels[i] = blend(self.pixels[i], colour, (cover * 255.0) as u8);
                }
            }
        }
    }

    /// A straight line, smoothed.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], width: f32, colour: u32) {
        let [ax, ay] = a;
        let [bx, by] = b;
        let half = width / 2.0;
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

    /// A bitmap with its top left at `pos`, inside `clip`: coverage or a
    /// picture's alpha in `colour`, else in its own colours.
    fn blit(
        &mut self,
        bitmaps: &Bitmaps,
        id: super::scene::BitmapId,
        pos: [i32; 2],
        colour: Option<u32>,
        clip: Clip,
    ) {
        let b = bitmaps.get(id);
        let [cx0, cy0, cx1, cy1] = clip;
        let (cx0, cy0) = (cx0.max(0), cy0.max(0));
        let (cx1, cy1) = (cx1.min(self.width as i32), cy1.min(self.height as i32));
        let (w, h) = (b.width as i32, b.height as i32);
        for row in (cy0 - pos[1]).max(0)..(cy1 - pos[1]).min(h) {
            let y = pos[1] + row;
            for col in (cx0 - pos[0]).max(0)..(cx1 - pos[0]).min(w) {
                let x = pos[0] + col;
                let i = (row * w + col) as usize;
                let at = y as usize * self.width + x as usize;
                let dst = self.pixels[at];
                self.pixels[at] = match (b.kind, colour) {
                    (Kind::Mask, Some(fg)) => blend(dst, fg, b.data[i]),
                    (Kind::Mask, None) => dst,
                    (Kind::Rgba, _) => {
                        let p = &b.data[i * 4..i * 4 + 4];
                        let rgb = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
                        blend(dst, rgb, p[3])
                    }
                    (Kind::Premul, Some(tint)) => blend(dst, tint, b.data[i * 4 + 3]),
                    (Kind::Premul, None) => {
                        // The colour plus what shows through.
                        let a = b.data[i * 4 + 3];
                        if a == 0 {
                            continue;
                        }
                        let keep = 255 - a as u32;
                        [(16, 0), (8, 1), (0, 2)].iter().fold(0, |out, (shift, k)| {
                            let s = b.data[i * 4 + k] as u32;
                            let d = (dst >> shift) & 0xff;
                            out | ((s + d * keep / 255).min(255) << shift)
                        })
                    }
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

#[cfg(test)]
mod tests {
    use resvg::tiny_skia::Pixmap;

    use super::super::scene;

    const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/gui/snapshots/scene.png");

    /// The sample scene matches the stored picture, pixel for pixel.
    /// `MERLIN_BLESS=1` stores a new one.
    #[test]
    fn draws_the_sample_scene() {
        let (scene, bitmaps, (w, h)) = scene::sample();
        let mut pixels = vec![0u32; w * h];
        super::draw(&scene, &bitmaps, &mut pixels, w, h);
        let mut png = Pixmap::new(w as u32, h as u32).unwrap();
        for (out, p) in png.data_mut().chunks_mut(4).zip(&pixels) {
            out.copy_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8, 0xff]);
        }
        if std::env::var("MERLIN_BLESS").is_ok() {
            png.save_png(SNAPSHOT).unwrap();
            return;
        }
        let stored = Pixmap::load_png(SNAPSHOT).expect("no snapshot; run with MERLIN_BLESS=1");
        assert!(
            stored.data() == png.data(),
            "the sample scene changed; look at it with MERLIN_BLESS=1 and git diff"
        );
    }
}
