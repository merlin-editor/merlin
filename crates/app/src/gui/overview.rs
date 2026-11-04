//! Beside each group's code, as VS Code has them: the scrollbar, whose
//! ruler marks changes, matches, selections and problems down the whole
//! file, with a thumb that shows while the mouse is near or the text moves;
//! a scrollbar along the bottom for long lines; and the minimap, the file
//! in tiny blocks of its colours, with a slider for the view.
//!
//! The geometry here serves both drawing and the mouse, so a thumb is
//! dragged exactly where it's drawn.

use merlin_core::TextView;
use merlin_core::editor::MinimapView;
use merlin_core::style::Style;

use super::canvas::{Canvas, Rect};
use crate::theme::{self, Palette};

/// The vertical scrollbar's width by default, and the minimap's with its
/// margin at its own scale.
pub const SCROLLBAR_W: f32 = 14.0;
pub const MINIMAP_W: f32 = 86.0;
/// The horizontal scrollbar's height, over the text's last row.
const HBAR_H: f32 = 10.0;
/// A minimap line's height and a character's width, at its own scale.
const MAP_LINE: f32 = 2.0;
const MAP_CHAR: f32 = 0.7;
/// The shortest a thumb gets, to stay easy to grab.
const MIN_THUMB: f32 = 24.0;

/// A thumb in its track: where it's drawn, and how its place maps to the
/// scroll position along the track.
#[derive(Clone, Copy, Debug)]
pub struct Thumb {
    pub rect: Rect,
    /// How far the thumb can move, in pixels, and the scroll that covers.
    travel: f32,
    range: f64,
}

impl Thumb {
    /// The scroll position for the thumb's leading edge `offset` pixels
    /// along its track.
    pub fn position(&self, offset: f32) -> f64 {
        if self.travel <= 0.0 {
            return 0.0;
        }
        (offset / self.travel).clamp(0.0, 1.0) as f64 * self.range
    }
}

/// The vertical thumb in `track`, when the file is taller than the view.
/// The last line can scroll to the top, so there's a view's worth more.
pub fn vthumb(track: Rect, v: &TextView) -> Option<Thumb> {
    let rows = v.area.h as f64;
    let total = v.total_rows as f64;
    if total <= rows || track.h <= 0.0 {
        return None;
    }
    let content = total + rows - 1.0;
    let len = (track.h * (rows / content) as f32).clamp(MIN_THUMB.min(track.h), track.h);
    let travel = track.h - len;
    let range = total - 1.0;
    let y = track.y + travel * (v.top_row / range).clamp(0.0, 1.0) as f32;
    Some(Thumb {
        rect: Rect::new(track.x, y, track.w, len),
        travel,
        range,
    })
}

/// The track along the bottom of `text`, past the gutter.
pub fn htrack(text: Rect, v: &TextView, cell_w: f32) -> Rect {
    let x = text.x + v.gutter as f32 * cell_w;
    Rect::new(
        x,
        text.bottom() - HBAR_H,
        (text.right() - x).max(0.0),
        HBAR_H,
    )
}

/// The horizontal thumb, when a line is wider than the view.
pub fn hthumb(track: Rect, v: &TextView) -> Option<Thumb> {
    let widest = v.scrollbar.widest? as f64;
    let cols = v.columns as f64;
    if widest <= cols || track.w <= 0.0 {
        return None;
    }
    let len = (track.w * (cols / widest) as f32).clamp(MIN_THUMB.min(track.w), track.w);
    let travel = track.w - len;
    let range = widest - cols;
    let x = track.x + travel * (v.hscroll as f64 / range).clamp(0.0, 1.0) as f32;
    Some(Thumb {
        rect: Rect::new(x, track.y, len, track.h),
        travel,
        range,
    })
}

/// The minimap's lines in `area`: the first shown, and the slider over
/// the lines in view.
#[derive(Clone, Copy, Debug)]
pub struct Map {
    pub first: usize,
    pub slider: Rect,
    thumb: Thumb,
    /// A line's height and a character's width, scaled as the map is.
    line: f32,
    char: f32,
}

impl Map {
    pub fn new(area: Rect, v: &TextView, map: &MinimapView) -> Self {
        let lines = map.lines.len();
        // Scaled up, it's wider.
        let scale = (area.w / MINIMAP_W).max(1.0);
        let (line, char) = (MAP_LINE * scale, MAP_CHAR * scale);
        let fit = (area.h / line).floor().max(1.0) as usize;
        let visible = (v.area.h as usize).min(lines).max(1);
        let top = v.scroll.min(lines.saturating_sub(1));
        // Longer than fits: it scrolls with the view, end to end.
        let first = if lines <= fit {
            0
        } else {
            let along = top as f64 / (lines - visible).max(1) as f64;
            (along.min(1.0) * (lines - fit) as f64) as usize
        };
        let shown = lines.min(fit) as f32 * line;
        let len = visible as f32 * line;
        let y = area.y + (top.saturating_sub(first) as f32 + v.top_frac) * line;
        Self {
            first,
            slider: Rect::new(area.x, y, area.w, len),
            thumb: Thumb {
                rect: Rect::new(area.x, y, area.w, len),
                travel: (shown - len).max(0.0),
                range: lines.saturating_sub(visible) as f64,
            },
            line,
            char,
        }
    }

    /// The top line for the slider's top `offset` pixels down the map.
    pub fn position(&self, offset: f32) -> f64 {
        self.thumb.position(offset)
    }

    /// The line under `y`.
    pub fn line_at(&self, area: Rect, y: f32) -> usize {
        self.first + ((y - area.y).max(0.0) / self.line) as usize
    }
}

/// How the bars look now: the thumbs' opacity, as they fade in and out,
/// and what the mouse is on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shown {
    pub alpha: f32,
    pub thumb_hovered: bool,
    pub dragging: bool,
    pub map_hovered: bool,
}

fn mark_colour(p: &Palette, style: Style) -> u32 {
    match style {
        Style::GitAdded => p.success,
        Style::GitChanged => p.info,
        Style::GitRemoved | Style::GutterError => p.error,
        Style::ScrollMatch => p.find_current,
        Style::Selection => p.muted,
        _ => p.warning,
    }
}

/// The vertical scrollbar: the ruler's marks in three lanes, git on the
/// left, matches and selections in the middle, problems on the right, and
/// the thumb over them.
pub fn draw_vbar(canvas: &mut Canvas, p: &Palette, track: Rect, v: &TextView, shown: Shown) {
    canvas.fill(Rect::new(track.x, track.y, 1.0, track.h), p.line);
    let lane = track.w / 3.0;
    for &(at, style) in &v.scrollbar.marks {
        let x = match style {
            Style::GitAdded | Style::GitChanged | Style::GitRemoved => track.x + 1.0,
            Style::ScrollMatch | Style::Selection => track.x + lane,
            _ => track.x + 2.0 * lane,
        };
        let y = track.y + at * track.h;
        canvas.fill(Rect::new(x, y, lane - 1.0, 2.0), mark_colour(p, style));
    }
    if let Some(thumb) = vthumb(track, v) {
        draw_thumb(canvas, p, thumb.rect, shown);
    }
}

/// The horizontal scrollbar's thumb, over the last row.
pub fn draw_hbar(canvas: &mut Canvas, p: &Palette, track: Rect, v: &TextView, shown: Shown) {
    if let Some(thumb) = hthumb(track, v) {
        draw_thumb(canvas, p, thumb.rect, shown);
    }
}

fn draw_thumb(canvas: &mut Canvas, p: &Palette, rect: Rect, shown: Shown) {
    let base = if shown.dragging {
        150.0
    } else if shown.thumb_hovered {
        110.0
    } else {
        70.0
    };
    let alpha = (base * shown.alpha.clamp(0.0, 1.0)) as u8;
    if alpha > 0 {
        canvas.tint(rect.inset(2.0), 3.0, p.muted, alpha);
    }
}

/// The minimap: each line's code in blocks of its colours, lines with
/// marks lit across, and the slider over the view while the mouse is on it.
pub fn draw_minimap(canvas: &mut Canvas, p: &Palette, area: Rect, v: &TextView, shown: Shown) {
    let Some(map) = &v.minimap else { return };
    let geom = Map::new(area, v, map);
    let (line_h, char_w) = (geom.line, geom.char);
    let fit = (area.h / line_h).floor() as usize;
    let left = area.x + 4.0;
    for &(line, style) in &map.marks {
        if line < geom.first || line >= geom.first + fit {
            continue;
        }
        let y = area.y + (line - geom.first) as f32 * line_h;
        let colour = mark_colour(p, style);
        match style {
            // Changes are a stripe down the left edge, as in the gutter.
            Style::GitAdded | Style::GitChanged | Style::GitRemoved => {
                canvas.fill(Rect::new(area.x, y, 2.0, line_h), colour)
            }
            Style::Selection => canvas.tint(Rect::new(area.x, y, area.w, line_h), 0.0, colour, 60),
            _ => canvas.tint(Rect::new(area.x, y, area.w, line_h), 0.0, colour, 110),
        }
    }
    for (i, runs) in map.lines.iter().skip(geom.first).take(fit).enumerate() {
        let y = area.y + i as f32 * line_h;
        for &(col, len, syntax) in runs {
            let x = left + col as f32 * char_w;
            if x >= area.right() {
                break;
            }
            let w = (len as f32 * char_w).min(area.right() - x);
            let colour = theme::code(p, syntax).unwrap_or(p.text);
            canvas.tint(Rect::new(x, y, w, line_h * 0.8), 0.0, colour, 170);
        }
    }
    if shown.map_hovered || shown.dragging {
        let alpha = if shown.dragging { 45 } else { 28 };
        canvas.tint(geom.slider, 0.0, p.text, alpha);
    }
}

/// A shadow under the top edge of the code once it's scrolled, as VS
/// Code draws one under the tabs.
pub fn draw_shadow(canvas: &mut Canvas, area: Rect, v: &TextView) {
    if v.top_row <= 0.0 {
        return;
    }
    for (k, alpha) in [(0.0, 55u8), (1.0, 30), (2.0, 12)] {
        canvas.tint(
            Rect::new(area.x, area.y + k, area.w, 1.0),
            0.0,
            0x000000,
            alpha,
        );
    }
}
