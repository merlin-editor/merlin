//! The GPU's backend, through wgpu. A scene becomes one list of instances,
//! drawn in order by two pipelines: shapes (rounded rectangles, borders,
//! shadows and lines, smoothed by their distance fields) and sprites
//! (glyphs and pictures from two atlases, coverage in one and RGBA in the
//! other). Colours blend as the CPU's do, in the display's own values.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use winit::window::Window;

use super::scene::{BitmapId, Bitmaps, Kind, Prim, Scene};

const QUAD: f32 = 0.0;
const LINE: f32 = 1.0;
const MASK: f32 = 2.0;
const RGBA: f32 = 3.0;
const PREMUL: f32 = 4.0;
const TINTED: f32 = 5.0;

/// Atlases start this wide and double when full.
const ATLAS: u32 = 512;

/// One thing drawn, as the shaders read it.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Where it's drawn: x0, y0, x1, y1 in physical pixels.
    rect: [f32; 4],
    /// A quad's edges, a line's ends, or where a sprite's top left is in
    /// its atlas.
    shape: [f32; 4],
    /// The colour, and its alpha.
    fill: [f32; 4],
    /// A border's colour, and its width.
    border: [f32; 4],
    /// The radius or a line's half width, the kind, and whether an opaque
    /// quad fills its middle rows whole, as the CPU's does.
    params: [f32; 4],
}

fn rgb(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 0xff) as f32 / 255.0,
        ((c >> 8) & 0xff) as f32 / 255.0,
        (c & 0xff) as f32 / 255.0,
    ]
}

/// A window's surface and the renderer that draws on it.
pub struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    view_format: wgpu::TextureFormat,
    renderer: Renderer,
}

impl Gpu {
    /// A renderer for `window`, or why there can't be one: no adapter, or
    /// only a software one, which draws slower than the CPU's backend.
    pub fn new(window: Arc<Window>) -> Result<Self, String> {
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(window.clone())),
        );
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return Err(format!("{} is a software adapter", adapter.get_info().name));
        }
        let (device, queue) = request_device(&adapter)?;
        let size = window.inner_size();
        let caps = surface.get_capabilities(&adapter);
        // Colours blend in the display's values, as the CPU's do: eight
        // bits a channel, drawn through a view that isn't sRGB. Float
        // formats would be read as linear.
        use wgpu::TextureFormat::{Bgra8Unorm, Bgra8UnormSrgb, Rgba8Unorm, Rgba8UnormSrgb};
        let format = [Bgra8Unorm, Rgba8Unorm, Bgra8UnormSrgb, Rgba8UnormSrgb]
            .into_iter()
            .find(|f| caps.formats.contains(f))
            .ok_or("the surface has no 8-bit format")?;
        let view_format = format.remove_srgb_suffix();
        let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            caps.alpha_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: if view_format == format {
                vec![]
            } else {
                vec![view_format]
            },
            ..surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .ok_or("the adapter can't draw on the window")?
        };
        surface.configure(&device, &config);
        let renderer = Renderer::new(device, queue, view_format);
        Ok(Self {
            surface,
            config,
            view_format,
            renderer,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.renderer.device, &self.config);
    }

    /// Draws `scene` and shows it. Returns false if the window couldn't
    /// take a frame, so it's drawn again later.
    pub fn draw(&mut self, scene: &Scene, bitmaps: &Bitmaps, window: &Window) -> bool {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.renderer.device, &self.config);
                return false;
            }
            _ => return false,
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        self.renderer.render(
            scene,
            bitmaps,
            &view,
            (self.config.width, self.config.height),
        );
        window.pre_present_notify();
        self.renderer.queue.present(frame);
        true
    }
}

fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("merlin"),
        required_features: wgpu::Features::empty(),
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))
    .map_err(|e| e.to_string())
}

/// Draws scenes on a device: the pipelines, the atlases and the buffers.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shapes: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    globals: wgpu::Buffer,
    instances: wgpu::Buffer,
    mask: Atlas,
    colour: Atlas,
    bind: Option<wgpu::BindGroup>,
    /// The bitmaps' generation the atlases hold.
    generation: u64,
    list: Vec<Instance>,
    /// Runs of the list for one pipeline: sprites or not.
    batches: Vec<(bool, Range<u32>)>,
}

/// An atlas wasn't big enough for a frame's bitmaps.
struct Full(Kind);

impl Renderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
        ];
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let shapes = pipeline("fs_shape");
        let sprites = pipeline("fs_sprite");
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let instances = Self::instance_buffer(&device, 1024);
        let mask = Atlas::new(&device, wgpu::TextureFormat::R8Unorm, ATLAS);
        let colour = Atlas::new(&device, wgpu::TextureFormat::Rgba8Unorm, ATLAS);
        Self {
            device,
            queue,
            shapes,
            sprites,
            layout,
            globals,
            instances,
            mask,
            colour,
            bind: None,
            generation: u64::MAX,
            list: Vec::new(),
            batches: Vec::new(),
        }
    }

    fn instance_buffer(device: &wgpu::Device, count: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: count * size_of::<Instance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Draws `scene` on `target`, `size` physical pixels.
    pub fn render(
        &mut self,
        scene: &Scene,
        bitmaps: &Bitmaps,
        target: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        // New bitmaps: start the atlases again, small.
        if self.generation != bitmaps.generation() {
            self.mask.reset(&self.device, ATLAS);
            self.colour.reset(&self.device, ATLAS);
            self.bind = None;
            self.generation = bitmaps.generation();
        }
        let max = self.device.limits().max_texture_dimension_2d;
        let mut tries = 0;
        while let Err(Full(kind)) = self.build(scene, bitmaps) {
            let atlas = match kind {
                Kind::Mask => &mut self.mask,
                _ => &mut self.colour,
            };
            let bigger = (atlas.size * 2).min(max);
            atlas.reset(&self.device, bigger);
            self.bind = None;
            tries += 1;
            if tries > 8 {
                // More than fits in the biggest atlas: draw what did.
                break;
            }
        }
        if self.bind.is_none() {
            self.bind = Some(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("scene"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.mask.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.colour.view),
                    },
                ],
            }));
        }
        let globals = [size.0 as f32, size.1 as f32, 0.0, 0.0];
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::cast_slice(&globals));
        let needed = self.list.len() as u64 * size_of::<Instance>() as u64;
        if needed > self.instances.size() {
            let count = (self.list.len() as u64).next_power_of_two();
            self.instances = Self::instance_buffer(&self.device, count);
        }
        if !self.list.is_empty() {
            self.queue
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&self.list));
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_bind_group(0, self.bind.as_ref(), &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            for (sprites, range) in &self.batches {
                pass.set_pipeline(if *sprites {
                    &self.sprites
                } else {
                    &self.shapes
                });
                pass.draw(0..4, range.clone());
            }
        }
        self.queue.submit([encoder.finish()]);
    }

    /// Turns `scene` into instances and batches, putting the bitmaps it
    /// uses in the atlases.
    fn build(&mut self, scene: &Scene, bitmaps: &Bitmaps) -> Result<(), Full> {
        self.list.clear();
        self.batches.clear();
        for prim in &scene.prims {
            match *prim {
                Prim::Shadow {
                    rect,
                    radius,
                    blur,
                    light,
                } => {
                    for quad in Prim::shadow_quads(rect, radius, blur, light) {
                        self.shape(&quad);
                    }
                }
                Prim::Quad { .. } | Prim::Line { .. } => self.shape(prim),
                Prim::Glyph {
                    bitmap,
                    pos,
                    colour,
                    clip,
                } => self.sprite(bitmaps, bitmap, pos, Some(colour), clip)?,
                Prim::Image {
                    bitmap,
                    pos,
                    tint,
                    clip,
                } => self.sprite(bitmaps, bitmap, pos, tint, clip)?,
            }
        }
        Ok(())
    }

    fn push(&mut self, sprite: bool, instance: Instance) {
        let at = self.list.len() as u32;
        self.list.push(instance);
        match self.batches.last_mut() {
            Some((s, range)) if *s == sprite => range.end = at + 1,
            _ => self.batches.push((sprite, at..at + 1)),
        }
    }

    fn shape(&mut self, prim: &Prim) {
        match *prim {
            Prim::Quad {
                rect,
                radius,
                fill,
                alpha,
                border,
                border_w,
            } => {
                let [r, g, b] = rgb(fill);
                let [br, bg, bb] = rgb(border);
                let opaque = alpha == 255;
                self.push(
                    false,
                    Instance {
                        rect: rect.map(f32::round),
                        shape: rect,
                        fill: [r, g, b, alpha as f32 / 255.0],
                        border: [br, bg, bb, border_w],
                        params: [radius, QUAD, opaque as u8 as f32, 0.0],
                    },
                );
            }
            Prim::Line {
                a,
                b,
                width,
                colour,
            } => {
                let half = width / 2.0;
                let [r, g, bl] = rgb(colour);
                self.push(
                    false,
                    Instance {
                        rect: [
                            (a[0].min(b[0]) - half - 1.0).max(0.0).trunc(),
                            (a[1].min(b[1]) - half - 1.0).max(0.0).trunc(),
                            (a[0].max(b[0]) + half + 1.0).trunc(),
                            (a[1].max(b[1]) + half + 1.0).trunc(),
                        ],
                        shape: [a[0], a[1], b[0], b[1]],
                        fill: [r, g, bl, 1.0],
                        border: [0.0; 4],
                        params: [half, LINE, 0.0, 0.0],
                    },
                );
            }
            _ => {}
        }
    }

    fn sprite(
        &mut self,
        bitmaps: &Bitmaps,
        id: BitmapId,
        pos: [i32; 2],
        colour: Option<u32>,
        clip: [i32; 4],
    ) -> Result<(), Full> {
        let bitmap = bitmaps.get(id);
        let (w, h) = (bitmap.width as i32, bitmap.height as i32);
        let x0 = pos[0].max(clip[0]);
        let y0 = pos[1].max(clip[1]);
        let x1 = (pos[0] + w).min(clip[2]);
        let y1 = (pos[1] + h).min(clip[3]);
        if x0 >= x1 || y0 >= y1 {
            return Ok(());
        }
        let atlas = match bitmap.kind {
            Kind::Mask => &mut self.mask,
            _ => &mut self.colour,
        };
        let Some([ax, ay]) = atlas.place(&self.queue, id, bitmap) else {
            return Err(Full(bitmap.kind));
        };
        let kind = match (bitmap.kind, colour) {
            (Kind::Mask, _) => MASK,
            (Kind::Rgba, _) => RGBA,
            (Kind::Premul, None) => PREMUL,
            (Kind::Premul, Some(_)) => TINTED,
        };
        let [r, g, b] = rgb(colour.unwrap_or(0));
        self.push(
            true,
            Instance {
                rect: [x0 as f32, y0 as f32, x1 as f32, y1 as f32],
                shape: [
                    (ax as i32 + x0 - pos[0]) as f32,
                    (ay as i32 + y0 - pos[1]) as f32,
                    0.0,
                    0.0,
                ],
                fill: [r, g, b, 1.0],
                border: [0.0; 4],
                params: [0.0, kind, 0.0, 0.0],
            },
        );
        Ok(())
    }
}

/// A texture of bitmaps packed in shelves: rows as tall as their tallest,
/// filled left to right.
struct Atlas {
    format: wgpu::TextureFormat,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: u32,
    /// Each shelf's top, height and how far it's filled.
    shelves: Vec<(u32, u32, u32)>,
    bottom: u32,
    places: HashMap<BitmapId, [u32; 2]>,
}

impl Atlas {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> Self {
        let texture = Self::texture(device, format, size);
        Self {
            format,
            view: texture.create_view(&Default::default()),
            texture,
            size,
            shelves: Vec::new(),
            bottom: 0,
            places: HashMap::new(),
        }
    }

    fn texture(device: &wgpu::Device, format: wgpu::TextureFormat, size: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    /// Empties the atlas, at `size` wide.
    fn reset(&mut self, device: &wgpu::Device, size: u32) {
        if size != self.size {
            self.texture = Self::texture(device, self.format, size);
            self.view = self.texture.create_view(&Default::default());
            self.size = size;
        }
        self.shelves.clear();
        self.bottom = 0;
        self.places.clear();
    }

    /// Where `bitmap` is in the atlas, putting it there if it isn't yet;
    /// `None` if there's no room.
    fn place(
        &mut self,
        queue: &wgpu::Queue,
        id: BitmapId,
        bitmap: &super::scene::Bitmap,
    ) -> Option<[u32; 2]> {
        if let Some(at) = self.places.get(&id) {
            return Some(*at);
        }
        let (w, h) = (bitmap.width, bitmap.height);
        if w > self.size {
            return None;
        }
        // The first shelf it fits without wasting much of its height.
        let shelf = self
            .shelves
            .iter_mut()
            .find(|(_, sh, x)| h <= *sh && *sh <= h + h / 2 + 2 && x + w <= self.size);
        let at = match shelf {
            Some((y, _, x)) => {
                let at = [*x, *y];
                *x += w;
                at
            }
            None if self.bottom + h <= self.size => {
                let at = [0, self.bottom];
                self.shelves.push((self.bottom, h, w));
                self.bottom += h;
                at
            }
            None => return None,
        };
        let bpp = if self.format == wgpu::TextureFormat::R8Unorm {
            1
        } else {
            4
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: at[0],
                    y: at[1],
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &bitmap.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * bpp),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.places.insert(id, at);
        Some(at)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A renderer on any adapter, for tests; `None` without one.
    pub fn headless() -> Option<Renderer> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = request_device(&adapter).ok()?;
        Some(Renderer::new(
            device,
            queue,
            wgpu::TextureFormat::Rgba8Unorm,
        ))
    }

    /// Draws `scene` off screen and reads it back as 0RGB pixels.
    pub fn render(
        r: &mut Renderer,
        scene: &Scene,
        bitmaps: &Bitmaps,
        (w, h): (u32, u32),
    ) -> Vec<u32> {
        let texture = r.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        r.render(
            scene,
            bitmaps,
            &texture.create_view(&Default::default()),
            (w, h),
        );
        let row = (w * 4).next_multiple_of(256);
        let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = r.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        r.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        r.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let mut out = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * row + x * 4) as usize;
                out.push((data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32);
            }
        }
        out
    }
}

#[cfg(test)]
mod compare {
    use super::super::{cpu, scene};

    /// The GPU draws the sample scene as the CPU does, give or take a
    /// step of rounding. Skipped without an adapter.
    #[test]
    fn draws_as_the_cpu_does() {
        let Some(mut renderer) = super::tests::headless() else {
            eprintln!("no adapter; skipped");
            return;
        };
        let (scene, bitmaps, (w, h)) = scene::sample();
        let mut cpu_pixels = vec![0u32; w * h];
        cpu::draw(&scene, &bitmaps, &mut cpu_pixels, w, h);
        let gpu_pixels =
            super::tests::render(&mut renderer, &scene, &bitmaps, (w as u32, h as u32));
        let mut worst = (0, 0);
        for (i, (a, b)) in cpu_pixels.iter().zip(&gpu_pixels).enumerate() {
            let diff = [16, 8, 0]
                .iter()
                .map(|s| ((a >> s) & 0xff).abs_diff((b >> s) & 0xff))
                .max()
                .unwrap();
            if diff > worst.0 {
                worst = (diff, i);
            }
        }
        let (diff, i) = worst;
        let mut counts = [0; 256];
        for (a, b) in cpu_pixels.iter().zip(&gpu_pixels) {
            let d = [16, 8, 0]
                .iter()
                .map(|s| ((a >> s) & 0xff).abs_diff((b >> s) & 0xff))
                .max()
                .unwrap();
            counts[d as usize] += 1;
        }
        eprintln!("pixels by difference: {:?}", &counts[..=diff as usize]);
        assert!(
            diff <= 3,
            "pixel ({}, {}) is {:06x} on the CPU and {:06x} on the GPU",
            i % w,
            i / w,
            cpu_pixels[i],
            gpu_pixels[i]
        );
    }
}
