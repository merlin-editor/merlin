// A scene's instances: shapes smoothed by their distance fields, and
// sprites from the atlases. Each instance is a rectangle of four vertices.
// Coverage is worked out as the CPU's backend does, so both look the same.

struct Globals {
    // The target's size in physical pixels.
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var mask: texture_2d<f32>;
@group(0) @binding(2) var colour: texture_2d<f32>;

const LINE: f32 = 1.0;
const MASK: f32 = 2.0;
const RGBA: f32 = 3.0;
const TINTED: f32 = 5.0;

struct Instance {
    @location(0) rect: vec4<f32>,
    @location(1) shape: vec4<f32>,
    @location(2) fill: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) params: vec4<f32>,
};

struct Varying {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) rect: vec4<f32>,
    @location(1) @interpolate(flat) shape: vec4<f32>,
    @location(2) @interpolate(flat) fill: vec4<f32>,
    @location(3) @interpolate(flat) border: vec4<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) v: u32, i: Instance) -> Varying {
    let corner = vec2<f32>(f32(v & 1u), f32(v >> 1u));
    let p = mix(i.rect.xy, i.rect.zw, corner);
    var out: Varying;
    out.pos = vec4<f32>(p.x / globals.size.x * 2.0 - 1.0, 1.0 - p.y / globals.size.y * 2.0, 0.0, 1.0);
    out.rect = i.rect;
    out.shape = i.shape;
    out.fill = i.fill;
    out.border = i.border;
    out.params = i.params;
    return out;
}

// How much of the pixel centred on `p` a rectangle with edges `e` and
// corners rounded to `radius` covers. The CPU's backend only draws inside
// the edges rounded to whole pixels, and fills an opaque rectangle's rows
// clear of its corners whole.
fn cover(p: vec2<f32>, e: vec4<f32>, radius: f32, whole_rows: bool) -> f32 {
    let snapped = floor(e + 0.5);
    let inside = all(p >= snapped.xy) && all(p < snapped.zw);
    if !inside {
        return 0.0;
    }
    let r = max(min(radius, min((e.z - e.x) / 2.0, (e.w - e.y) / 2.0)), 0.0);
    if r <= 0.0 {
        return 1.0;
    }
    let d = max(max(e.xy + r - p, p - (e.zw - r)), vec2<f32>(0.0));
    if whole_rows && d.y == 0.0 {
        return 1.0;
    }
    return clamp(0.5 - (length(d) - r), 0.0, 1.0);
}

// An alpha in whole 255ths, cut down as the CPU's are.
fn bytes(a: f32) -> f32 {
    return floor(a * 255.0) / 255.0;
}

@fragment
fn fs_shape(v: Varying) -> @location(0) vec4<f32> {
    let p = v.pos.xy;
    if v.params.y == LINE {
        let a = v.shape.xy;
        let d = v.shape.zw - a;
        let t = clamp(dot(p - a, d) / max(dot(d, d), 1e-6), 0.0, 1.0);
        let c = bytes(clamp(0.5 - (length(a + t * d - p) - v.params.x), 0.0, 1.0));
        return vec4<f32>(v.fill.rgb * c, c);
    }
    let opaque = v.params.z > 0.5;
    var rgb = v.fill.rgb;
    let bw = v.border.w;
    if bw > 0.0 {
        let inner = vec4<f32>(v.shape.xy + bw, max(v.shape.zw - bw, v.shape.xy + bw));
        let ic = bytes(cover(p, inner, max(v.params.x - bw, 0.0), true));
        rgb = mix(v.border.rgb, v.fill.rgb, ic);
    }
    let a = bytes(cover(p, v.shape, v.params.x, opaque) * v.fill.a);
    return vec4<f32>(rgb * a, a);
}

@fragment
fn fs_sprite(v: Varying) -> @location(0) vec4<f32> {
    let at = vec2<i32>(v.shape.xy + floor(v.pos.xy) - v.rect.xy);
    let kind = v.params.y;
    if kind == MASK {
        let a = textureLoad(mask, at, 0).r;
        return vec4<f32>(v.fill.rgb * a, a);
    }
    let t = textureLoad(colour, at, 0);
    if kind == RGBA {
        return vec4<f32>(t.rgb * t.a, t.a);
    }
    if kind == TINTED {
        return vec4<f32>(v.fill.rgb * t.a, t.a);
    }
    // Premultiplied already.
    return t;
}
