//! A plugin's drawing in a view: its shapes scaled to the view's width, in
//! the theme's colours and the editor's fonts, cut to what shows. Shapes
//! with an id are clicked as the plugin's buttons are.

use super::*;
use merlin_core::{DrawColour, Drawing, Shape};

/// How many straight pieces a curve is drawn as.
const CURVE_STEPS: usize = 16;

impl Frame<'_> {
    /// `d` from (`x`, `y`), `w` wide, for the view named `view`.
    pub(super) fn drawing(&mut self, d: &Drawing, view: &str, x: f32, y: f32, w: f32, clip: Rect) {
        let k = w / d.width.max(1.0);
        let at = |p: [f32; 2]| (x + p[0] * k, y + p[1] * k);
        for item in &d.items {
            let hit = match &item.shape {
                Shape::Rect {
                    rect,
                    radius,
                    fill,
                    stroke,
                } => {
                    let r = Rect::new(x + rect[0] * k, y + rect[1] * k, rect[2] * k, rect[3] * k);
                    let shown = r.intersect(clip);
                    if shown.w > 0.0 && shown.h > 0.0 {
                        // Cut by the view's edge, the cut side is square.
                        let whole = (shown.w - r.w).abs() < 0.01 && (shown.h - r.h).abs() < 0.01;
                        let radius = if whole { radius * k } else { 0.0 };
                        match (fill, stroke) {
                            (Some(f), Some(s)) => {
                                let (f, s) = (self.draw_colour(f), self.draw_colour(s));
                                self.canvas.outline(shown, radius, 1.0, s, f);
                            }
                            (Some(f), None) => {
                                let f = self.draw_colour(f);
                                self.canvas.round(shown, radius, f);
                            }
                            (None, Some(s)) => {
                                let s = self.draw_colour(s);
                                self.canvas.frame(shown, radius, 1.0, s, 0);
                            }
                            (None, None) => {}
                        }
                    }
                    r
                }
                Shape::Line {
                    from,
                    to,
                    width,
                    colour,
                } => {
                    let c = self.draw_colour(colour);
                    let (a, b) = (at(*from), at(*to));
                    self.clipped_line(a, b, (width * k).max(0.5), c, clip);
                    bounds(&[a, b], 4.0)
                }
                Shape::Curve {
                    points,
                    width,
                    colour,
                } => {
                    let c = self.draw_colour(colour);
                    let mut flat = Vec::new();
                    if let Some(&first) = points.first() {
                        flat.push(at(first));
                        for seg in points[1..].chunks_exact(3) {
                            let p0 = *flat.last().unwrap();
                            let (p1, p2, p3) = (at(seg[0]), at(seg[1]), at(seg[2]));
                            for i in 1..=CURVE_STEPS {
                                let t = i as f32 / CURVE_STEPS as f32;
                                let u = 1.0 - t;
                                let b = |a: f32, b: f32, c: f32, d: f32| {
                                    u * u * u * a
                                        + 3.0 * u * u * t * b
                                        + 3.0 * u * t * t * c
                                        + t * t * t * d
                                };
                                flat.push((b(p0.0, p1.0, p2.0, p3.0), b(p0.1, p1.1, p2.1, p3.1)));
                            }
                        }
                    }
                    for pair in flat.windows(2) {
                        self.clipped_line(pair[0], pair[1], (width * k).max(0.5), c, clip);
                    }
                    bounds(&flat, 4.0)
                }
                Shape::Text {
                    at: p,
                    text,
                    size,
                    colour,
                    mono,
                } => {
                    let c = self.draw_colour(colour);
                    let px = (size * k).clamp(4.0, 200.0);
                    let face = if *mono { Face::mono(px) } else { Face::ui(px) };
                    let (tx, ty) = at(*p);
                    let row = px * 1.3;
                    let tw =
                        self.text
                            .draw_in_row(&mut self.canvas, text, face, tx, ty, row, c, clip);
                    Rect::new(tx, ty, tw, row)
                }
                Shape::Image { rect, image, .. } => {
                    let r = Rect::new(x + rect[0] * k, y + rect[1] * k, rect[2] * k, rect[3] * k);
                    let s = self.canvas.scale;
                    let (pw, ph) = ((r.w * s).round() as u32, (r.h * s).round() as u32);
                    if let Some(image) = image
                        && pw > 0
                        && ph > 0
                        && r.intersect(clip).h > 0.0
                    {
                        let picture = self
                            .pictures
                            .plugin(image, pw, ph, 0.0, self.canvas.bitmaps);
                        self.canvas.picture(picture, r.x, r.y, None, clip);
                    }
                    r
                }
            };
            if !item.id.is_empty() {
                let hit = hit.intersect(clip);
                if hit.w > 0.0 && hit.h > 0.0 {
                    let press = format!("draw:{view}:{}", item.id);
                    self.hit(hit, Hit::Ui(UiEvent::Press(press)));
                }
            }
        }
    }

    /// A theme's colour by name, or the drawing's own.
    fn draw_colour(&self, c: &DrawColour) -> u32 {
        let p = &self.p;
        match c {
            DrawColour::Rgb(rgb) => *rgb,
            DrawColour::Theme(name) => match name.as_str() {
                "bright" => p.bright,
                "muted" => p.muted,
                "faint" => p.faint,
                "accent" => p.accent,
                "error" => p.error,
                "warning" => p.warning,
                "success" => p.success,
                "info" => p.info,
                "border" => p.border,
                "chip" => p.chip,
                "raised" => p.raised,
                "ground" => p.ground,
                "island" => p.island,
                _ => p.text,
            },
        }
    }

    /// A line cut to `clip`, as lines aren't cut when drawn.
    fn clipped_line(&mut self, a: (f32, f32), b: (f32, f32), width: f32, colour: u32, clip: Rect) {
        // Liang–Barsky: how far along the line it enters and leaves.
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, q) in [
            (-dx, a.0 - clip.x),
            (dx, clip.right() - a.0),
            (-dy, a.1 - clip.y),
            (dy, clip.bottom() - a.1),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return;
                }
                continue;
            }
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
        }
        if t0 > t1 {
            return;
        }
        let at = |t: f32| (a.0 + dx * t, a.1 + dy * t);
        self.canvas.line(at(t0), at(t1), width, colour);
    }
}

/// The box around some points, grown by `pad` for the mouse.
fn bounds(points: &[(f32, f32)], pad: f32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in points {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 > x1 {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    Rect::new(x0 - pad, y0 - pad, x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad)
}
