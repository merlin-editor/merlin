//! What a frame draws, recorded in order in physical pixels, for a backend
//! to draw: the CPU's ([`super::cpu`]) or the GPU's ([`super::gpu`]).
//!
//! Colours are 0xRRGGBB. Glyphs and pictures are drawn once into
//! [`Bitmaps`] and named by their [`BitmapId`]; a backend keeps its own
//! copies (the GPU's atlases) until the bitmaps are cleared.

/// A frame's drawing, back to front.
#[derive(Default)]
pub struct Scene {
    pub prims: Vec<Prim>,
}

impl Scene {
    /// Empties the scene for the next frame, keeping its room.
    pub fn clear(&mut self) {
        self.prims.clear();
    }
}

/// A rectangle by its edges, in physical pixels: (x0, y0, x1, y1).
pub type Edges = [f32; 4];
/// A clip in whole physical pixels: (x0, y0, x1, y1), the ends excluded.
pub type Clip = [i32; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Prim {
    /// A rectangle with corners rounded to `radius`, smoothed at the edges,
    /// in `fill` at `alpha` over what's below. With `border_w` above zero,
    /// the outer `border_w` is in `border` instead.
    Quad {
        rect: Edges,
        radius: f32,
        fill: u32,
        alpha: u8,
        border: u32,
        border_w: f32,
    },
    /// A soft shadow under a floating box, spreading `blur` out from it
    /// and sitting a little low; lighter for a light theme.
    Shadow {
        rect: Edges,
        radius: f32,
        blur: f32,
        light: bool,
    },
    /// A straight line from `a` to `b`, `width` wide, smoothed.
    Line {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        colour: u32,
    },
    /// A glyph's bitmap with its top left at `pos`: coverage in `colour`,
    /// or a colour glyph (emoji) in its own.
    Glyph {
        bitmap: BitmapId,
        pos: [i32; 2],
        colour: u32,
        clip: Clip,
    },
    /// A picture's bitmap with its top left at `pos`, in its own colours,
    /// or as a shape in `tint`.
    Image {
        bitmap: BitmapId,
        pos: [i32; 2],
        tint: Option<u32>,
        clip: Clip,
    },
}

impl Prim {
    /// The layers a shadow is drawn in: how far each grows out from the box
    /// (as a part of the blur), and its alpha.
    pub fn shadow_layers(light: bool) -> [(f32, u8); 3] {
        if light {
            [(1.0, 6), (0.6, 9), (0.3, 12)]
        } else {
            [(1.0, 18), (0.6, 26), (0.3, 34)]
        }
    }

    /// A shadow's layers as plain translucent quads, as both backends draw
    /// them.
    pub fn shadow_quads(rect: Edges, radius: f32, blur: f32, light: bool) -> [Prim; 3] {
        Self::shadow_layers(light).map(|(part, alpha)| {
            let grow = blur * part;
            let drop = blur * 0.4;
            Prim::Quad {
                rect: [
                    rect[0] - grow,
                    rect[1] - grow + drop,
                    rect[2] + grow,
                    rect[3] + grow + drop,
                ],
                radius: radius + grow,
                fill: 0x000000,
                alpha,
                border: 0,
                border_w: 0.0,
            }
        })
    }
}

/// How a bitmap's bytes are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// One byte of coverage a pixel.
    Mask,
    /// RGBA, not premultiplied: colour glyphs.
    Rgba,
    /// RGBA, premultiplied: pictures.
    Premul,
}

pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub kind: Kind,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitmapId(pub u32);

/// Every glyph and picture drawn so far. Clearing it starts a new
/// generation; whoever keeps ids checks the generation and forgets theirs
/// when it moves on.
#[derive(Default)]
pub struct Bitmaps {
    list: Vec<Bitmap>,
    bytes: usize,
    generation: u64,
}

impl Bitmaps {
    /// Past this many bytes the bitmaps start again, between frames.
    pub const LIMIT: usize = 64 << 20;

    pub fn add(&mut self, bitmap: Bitmap) -> BitmapId {
        self.bytes += bitmap.data.len();
        self.list.push(bitmap);
        BitmapId(self.list.len() as u32 - 1)
    }

    pub fn get(&self, id: BitmapId) -> &Bitmap {
        &self.list[id.0 as usize]
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Forgets every bitmap, for when packs or fonts change.
    pub fn clear(&mut self) {
        self.list = Vec::new();
        self.bytes = 0;
        self.generation += 1;
    }
}

/// A small scene with one of everything, for the backends' tests.
#[cfg(test)]
pub fn sample() -> (Scene, Bitmaps, (usize, usize)) {
    let mut bitmaps = Bitmaps::default();
    // A soft dot of coverage, a colour square and a premultiplied ring.
    let dot = |x: u32, y: u32| {
        let (dx, dy) = (x as f32 - 4.5, y as f32 - 5.5);
        ((5.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0) * 255.0) as u8
    };
    let mask = bitmaps.add(Bitmap {
        width: 10,
        height: 12,
        kind: Kind::Mask,
        data: (0..12)
            .flat_map(|y| (0..10).map(move |x| dot(x, y)))
            .collect(),
    });
    let rgba = bitmaps.add(Bitmap {
        width: 6,
        height: 6,
        kind: Kind::Rgba,
        data: (0..36)
            .flat_map(|i| {
                [
                    0xe0,
                    0x40,
                    (i * 7) as u8,
                    if i % 5 == 0 { 0x80 } else { 0xff },
                ]
            })
            .collect(),
    });
    let ring = bitmaps.add(Bitmap {
        width: 12,
        height: 12,
        kind: Kind::Premul,
        data: (0..144)
            .flat_map(|i| {
                let (x, y) = ((i % 12) as f32 - 5.5, (i / 12) as f32 - 5.5);
                let a = (1.5 - ((x * x + y * y).sqrt() - 4.0).abs()).clamp(0.0, 1.0);
                let a = (a * 255.0) as u8;
                [a / 2, a / 3, a, a]
            })
            .collect(),
    });
    let quad = |rect: Edges, radius: f32, fill: u32, alpha: u8| Prim::Quad {
        rect,
        radius,
        fill,
        alpha,
        border: 0,
        border_w: 0.0,
    };
    let all = [0, 0, 96, 64];
    let prims = vec![
        quad([0.0, 0.0, 96.0, 64.0], 0.0, 0x1e1f24, 255),
        quad([4.0, 4.0, 44.0, 30.0], 7.5, 0x2b2d35, 255),
        quad([6.3, 34.2, 30.7, 58.9], 5.0, 0x3d6be0, 255),
        Prim::Shadow {
            rect: [52.0, 8.0, 88.0, 32.0],
            radius: 6.0,
            blur: 6.0,
            light: false,
        },
        Prim::Quad {
            rect: [52.0, 8.0, 88.0, 32.0],
            radius: 6.0,
            fill: 0x30323a,
            alpha: 255,
            border: 0x585b66,
            border_w: 1.0,
        },
        quad([36.0, 36.0, 92.0, 60.0], 4.0, 0xffffff, 40),
        Prim::Line {
            a: [10.0, 10.0],
            b: [16.0, 16.0],
            width: 1.5,
            colour: 0xc8c8d0,
        },
        Prim::Line {
            a: [16.0, 16.0],
            b: [22.0, 10.0],
            width: 1.5,
            colour: 0xc8c8d0,
        },
        Prim::Glyph {
            bitmap: mask,
            pos: [26, 12],
            colour: 0xe8e8ec,
            clip: all,
        },
        // Cut in half by its clip.
        Prim::Glyph {
            bitmap: mask,
            pos: [56, 14],
            colour: 0xf0c060,
            clip: [56, 14, 61, 32],
        },
        Prim::Glyph {
            bitmap: rgba,
            pos: [70, 16],
            colour: 0,
            clip: all,
        },
        Prim::Image {
            bitmap: ring,
            pos: [40, 42],
            tint: None,
            clip: all,
        },
        Prim::Image {
            bitmap: ring,
            pos: [60, 42],
            tint: Some(0x70e0a0),
            clip: all,
        },
        // Off the edge of the window.
        quad([88.0, 56.0, 120.0, 80.0], 3.0, 0xd04040, 255),
    ];
    (Scene { prims }, bitmaps, (96, 64))
}
