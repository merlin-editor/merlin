//! Small parts the rest is drawn from: icons, chips, badges, buttons and
//! checkboxes.

use super::*;

impl Frame<'_> {
    /// A button showing the icon theme's `id` icon, or `glyph`.
    pub(super) fn icon_button(&mut self, r: Rect, id: &str, glyph: char, on: bool, hit: Hit) {
        let tip = match id {
            "menu" => Some("Application Menu (F10)"),
            "layout-left" => Some("Toggle Primary Side Bar (Ctrl+B)"),
            "layout-panel" => Some("Toggle Panel (Ctrl+J)"),
            "layout-right" => Some("Toggle Secondary Side Bar"),
            "split" => Some("Split Editor Right (Ctrl+\\)"),
            "more" => Some("More Actions…"),
            _ => None,
        };
        if let Some(tip) = tip {
            self.tips.push((r, tip.to_string()));
        }
        let p = self.p;
        let hovered = self.hovered(r);
        if hovered || on {
            self.canvas
                .round(r, 6.0, if on { p.raised } else { p.hover });
        }
        let colour = if on || hovered { p.bright } else { p.muted };
        self.hit(r, hit);
        if let Some(icon) = self.icons.ui(id).cloned() {
            let size = (r.h - 12.0).max(10.0);
            let (x, y) = (r.x + (r.w - size) / 2.0, r.y + (r.h - size) / 2.0);
            if self.draw_icon(&icon, x, y, size, colour, r) {
                return;
            }
        }
        let face = Face::mono(15.0);
        let gw = self.text.width(&glyph.to_string(), face);
        let clip = r;
        self.text.draw_in_row(
            &mut self.canvas,
            &glyph.to_string(),
            face,
            r.x + (r.w - gw) / 2.0,
            r.y,
            r.h,
            colour,
            clip,
        );
    }

    /// Draws an icon in a `size` square at (`x`, `y`), in `colour` unless
    /// it has its own. Returns whether it could be drawn.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_icon(
        &mut self,
        icon: &Icon,
        x: f32,
        y: f32,
        size: f32,
        colour: u32,
        clip: Rect,
    ) -> bool {
        match icon {
            Icon::Svg { file, tint } => {
                let px = (size * self.canvas.scale).round() as u32;
                let Some(picture) = self.pictures.get(file, px, self.canvas.bitmaps) else {
                    return false;
                };
                let tint = match tint {
                    Tint::Own => None,
                    Tint::Colour(c) => Some(*c),
                    Tint::Current => Some(colour),
                };
                self.canvas.picture(picture, x, y, tint, clip);
                true
            }
            Icon::Glyph { text, colour: own } => {
                let face = Face::mono(size * 0.85);
                let w = self.text.width(text, face);
                self.text.draw_in_row(
                    &mut self.canvas,
                    text,
                    face,
                    x + (size - w) / 2.0,
                    y,
                    size,
                    own.unwrap_or(colour),
                    clip,
                );
                true
            }
            Icon::Badge { text, colour: own } => {
                self.badge(text, x, y, size, own.unwrap_or(colour));
                true
            }
        }
    }

    /// A file's icon from the icon theme, else as the theme's fallback has
    /// it. Returns the room it took.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::gui) fn file_icon(
        &mut self,
        name: &str,
        core: Option<(char, merlin_core::style::Hue)>,
        x: f32,
        y: f32,
        size: f32,
        clip: Rect,
    ) -> f32 {
        let p = self.p;
        let colour = core.map_or(p.muted, |(_, hue)| theme::hue(&p, hue));
        if let Some(icon) = self.icons.file(name).cloned()
            && self.draw_icon(&icon, x, y, size, colour, clip)
        {
            return size + 6.0;
        }
        let ext = name
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .filter(|ext| !ext.is_empty() && ext.len() <= 4);
        match (self.icons.fallback, ext, core) {
            (Fallback::None, ..) => 0.0,
            (Fallback::Badge, Some(ext), _) => {
                self.badge(ext, x, y, size, colour);
                size + 6.0
            }
            (_, _, Some((glyph, _))) => {
                let icon = Icon::Glyph {
                    text: glyph.to_string(),
                    colour: Some(colour),
                };
                self.draw_icon(&icon, x, y, size, colour, clip);
                size + 6.0
            }
            _ => 0.0,
        }
    }

    /// A key or label in a small grey box, ending at `right`.
    pub(super) fn chip(&mut self, label: &str, right: f32, y: f32, end: bool) -> f32 {
        let p = self.p;
        let face = Face::ui(11.0);
        let tw = self.text.width(label, face);
        let x = if end { right - tw - 12.0 } else { right };
        let r = Rect::new(x, y, tw + 12.0, 16.0);
        self.canvas.round(r, 4.0, p.chip);
        self.text.draw_in_row(
            &mut self.canvas,
            label,
            face,
            x + 6.0,
            y,
            16.0,
            p.chip_text,
            r,
        );
        r.w
    }

    /// A plugin's name in a blue chip, ending at `right`: what a task or
    /// test came from. Its width.
    pub(super) fn plugin_chip(&mut self, label: &str, right: f32, y: f32) -> f32 {
        let p = self.p;
        let face = Face::ui(11.0);
        let tw = self.text.width(label, face);
        let r = Rect::new(right - tw - 12.0, y, tw + 12.0, 16.0);
        self.canvas.round(r, 4.0, mix(p.accent, p.widget, 0.18));
        self.text.draw_in_row(
            &mut self.canvas,
            label,
            face,
            r.x + 6.0,
            y,
            16.0,
            p.accent,
            r,
        );
        r.w
    }

    /// A file type's letters in a small rounded square of its colour.
    pub(super) fn badge(&mut self, ext: &str, x: f32, y: f32, size: f32, colour: u32) {
        let p = self.p;
        let r = Rect::new(x, y, size, size);
        self.canvas.round(r, 4.0, mix(colour, p.island, 0.22));
        let label: String = ext.chars().take(2).collect::<String>().to_uppercase();
        let face = Face::ui(size * 0.5).bold();
        let lw = self.text.width(&label, face);
        self.text.draw_in_row(
            &mut self.canvas,
            &label,
            face,
            x + (size - lw) / 2.0,
            y,
            size,
            colour,
            r,
        );
    }

    /// A button's ground and text: the accent for the primary one, a chip
    /// for the rest, lighter while hovered.
    pub(super) fn button_colours(&self, primary: bool, hovered: bool) -> (u32, u32) {
        let p = self.p;
        if primary {
            let bg = if hovered {
                mix(p.accent, 0xffffff, 0.88)
            } else {
                p.accent
            };
            (bg, p.ground)
        } else {
            (if hovered { p.border } else { p.chip }, p.bright)
        }
    }

    /// `glyph` in the middle of `r`.
    pub(super) fn centred(&mut self, glyph: &str, face: Face, r: Rect, colour: u32, clip: Rect) {
        let gw = self.text.width(glyph, face);
        self.text.draw_in_row(
            &mut self.canvas,
            glyph,
            face,
            r.x + (r.w - gw) / 2.0,
            r.y,
            r.h,
            colour,
            clip,
        );
    }

    pub(super) fn button(
        &mut self,
        label: &str,
        x: f32,
        y: f32,
        focused: bool,
        primary: bool,
    ) -> f32 {
        let p = self.p;
        let face = Face::ui(13.0).medium();
        let tw = self.text.width(label, face);
        let r = Rect::new(x, y, tw + 28.0, 30.0);
        let (bg, fg) = self.button_colours(primary, self.hovered(r));
        if focused {
            self.canvas.outline(
                Rect::new(x - 2.0, y - 2.0, r.w + 4.0, 34.0),
                9.0,
                1.5,
                p.accent,
                p.island,
            );
        }
        self.canvas.round(r, 7.0, bg);
        self.text
            .draw_in_row(&mut self.canvas, label, face, x + 14.0, y, 30.0, fg, r);
        r.w
    }

    /// A button of `label` and its key, ending at `right`: its width.
    pub(super) fn key_button(
        &mut self,
        label: &str,
        key: &str,
        right: f32,
        y: f32,
        primary: bool,
    ) -> f32 {
        let p = self.p;
        let (face, chip) = (Face::ui(12.5), Face::ui(11.0));
        let (lw, kw) = (self.text.width(label, face), self.text.width(key, chip));
        let w = lw + kw + 12.0 + 8.0 + 20.0;
        let r = Rect::new(right - w, y, w, 26.0);
        let (bg, fg) = self.button_colours(primary, self.hovered(r));
        let (chip_bg, chip_fg) = if primary {
            (mix(p.ground, bg, 0.18), p.ground)
        } else {
            (mix(p.chip_text, bg, 0.12), p.chip_text)
        };
        self.canvas.round(r, 6.0, bg);
        self.text
            .draw_in_row(&mut self.canvas, label, face, r.x + 10.0, y, 26.0, fg, r);
        let k = Rect::new(r.x + 10.0 + lw + 8.0, y + 5.0, kw + 12.0, 16.0);
        self.canvas.round(k, 4.0, chip_bg);
        self.text.draw_in_row(
            &mut self.canvas,
            key,
            chip,
            k.x + 6.0,
            k.y,
            16.0,
            chip_fg,
            r,
        );
        w
    }

    /// A checkbox at `x`, centred on `mid`.
    pub(super) fn checkbox(&mut self, check: Check, x: f32, mid: f32) {
        let p = self.p;
        let b = Rect::new(x, mid - 7.0, 14.0, 14.0);
        match check {
            Check::On => {
                self.canvas.round(b, 4.0, p.accent);
                self.canvas
                    .line((x + 3.5, mid + 0.2), (x + 6.0, mid + 2.8), 1.8, p.ground);
                self.canvas
                    .line((x + 6.0, mid + 2.8), (x + 10.8, mid - 2.6), 1.8, p.ground);
            }
            Check::Some => {
                self.canvas.frame(b, 4.0, 1.5, p.accent, 0);
                self.canvas
                    .round(Rect::new(x + 3.5, mid - 1.0, 7.0, 2.0), 1.0, p.accent);
            }
            Check::Off => {
                self.canvas
                    .frame(b, 4.0, 1.5, mix(p.muted, p.island, 0.5), 0);
            }
        }
    }

    /// The terminal panel's button icons, drawn in strokes: 1 new, 2
    /// split, 3 kill, 4 maximise, 5 restore, else close.
    pub(super) fn toolbar_icon(&mut self, icon: u8, r: Rect, c: u32) {
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let line = |canvas: &mut Canvas, a: (f32, f32), b: (f32, f32)| {
            canvas.line((cx + a.0, cy + a.1), (cx + b.0, cy + b.1), 1.4, c)
        };
        match icon {
            1 => {
                line(&mut self.canvas, (-5.0, 0.0), (5.0, 0.0));
                line(&mut self.canvas, (0.0, -5.0), (0.0, 5.0));
            }
            2 => {
                self.canvas
                    .frame(Rect::new(cx - 6.0, cy - 5.0, 12.0, 10.0), 2.0, 1.3, c, 0);
                line(&mut self.canvas, (0.0, -5.0), (0.0, 5.0));
            }
            3 => {
                line(&mut self.canvas, (-5.5, -3.5), (5.5, -3.5));
                line(&mut self.canvas, (-2.0, -5.5), (2.0, -5.5));
                line(&mut self.canvas, (-4.0, -3.5), (-3.2, 5.5));
                line(&mut self.canvas, (4.0, -3.5), (3.2, 5.5));
                line(&mut self.canvas, (-3.2, 5.5), (3.2, 5.5));
            }
            4 => {
                line(&mut self.canvas, (-4.5, 2.5), (0.0, -2.0));
                line(&mut self.canvas, (0.0, -2.0), (4.5, 2.5));
            }
            5 => {
                line(&mut self.canvas, (-4.5, -2.0), (0.0, 2.5));
                line(&mut self.canvas, (0.0, 2.5), (4.5, -2.0));
            }
            _ => {
                line(&mut self.canvas, (-4.0, -4.0), (4.0, 4.0));
                line(&mut self.canvas, (4.0, -4.0), (-4.0, 4.0));
            }
        }
    }
}
