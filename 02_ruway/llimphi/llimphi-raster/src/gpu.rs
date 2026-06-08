//! Backend GPU directo (Fases 2 + 3 del SDD §"GPU directo wgpu").
//!
//! Tres pipelines `wgpu` cacheadas en [`GpuPipelines`] (lines / tris /
//! rects) + un acumulador [`GpuBatch`] que las apps usan por frame para
//! emitir centenares de miles a millones de primitivos en una draw call
//! por tipo, sin pasar por vello.
//!
//! Diseño minimal Fase 2/3:
//!
//! - Vertex format triángulos: `[x: f32, y: f32, rgba: u32]` (12 B/vert).
//! - Instance format líneas: `[x0, y0, x1, y1, rgba]` (20 B/seg).
//! - Instance format rects:  `[x, y, w, h, rgba]` (20 B/rect).
//! - Sin texturas. Sin AA por shader — quien necesite AA fino sigue por
//!   vello. Para puntos densos el "popping" no se nota.
//! - Blending alfa habilitado: el alpha del color es respetado.
//! - El viewport `(width, height)` se pasa al flush y va en un uniform —
//!   los shaders convierten pixel → NDC ahí.
//!
//! Cache de pipelines: una sola instancia de `GpuPipelines` por
//! `(device, color_format)`. Construirla compila los 3 pipelines en
//! caliente (~ms en hardware moderno). Los callers la mantienen viva
//! entre frames (en su Model o vía `OnceLock`).
//!
//! Grow strategy: `flush` crea un buffer por tipo no vacío en el
//! mismo frame. Sin reuso entre frames — Fase 4 (`GpuSceneCanvas`)
//! introducirá el `GpuBuffers` persistente que dobla capacidad si
//! aparece la necesidad.

use llimphi_hal::wgpu;
use vello::peniko::Color;

/// Pipelines cacheadas. Crear uno por proceso (o por surface format).
///
/// Para uso típico via [`GpuBatch`] los campos no se tocan directo. La
/// API pública existe para callers avanzados que quieran montar su propio
/// buffer persistente (datos que no cambian por frame: starfield Gaia,
/// particles iniciales, viewport estático) y emitir draw calls
/// manualmente reusando estas pipelines.
///
/// Layouts:
/// - Vertex buffer triángulos: `[x: f32, y: f32, rgba: u32]` (12 B/vert).
/// - Instance buffer rects:    `[x, y, w, h, rgba]`           (20 B/inst).
/// - Instance buffer líneas:   `[x0, y0, x1, y1, rgba]`       (20 B/inst).
/// - Bind group 0 binding 0: uniform `{viewport: vec2<f32>, line_width: f32, _pad: f32}` (16 B).
pub struct GpuPipelines {
    pub lines: wgpu::RenderPipeline,
    pub tris: wgpu::RenderPipeline,
    pub rects: wgpu::RenderPipeline,
    pub bind_layout: wgpu::BindGroupLayout,
}

impl GpuPipelines {
    /// Compila los 3 pipelines apuntando al `color_format` del target
    /// que recibirán en `flush` (el de la intermediate de `WinitSurface`,
    /// normalmente `Rgba8Unorm`).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("llimphi-raster-gpu-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-raster-gpu-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-raster-gpu-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let color_targets = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];

        // Triángulos (vertex buffer plano, color per-vertex).
        let tris = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-raster-gpu-tris"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_tris"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: 8,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: tri_primitive(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview: None,
            cache: None,
        });

        // Rects (instanced quad).
        let rects = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-raster-gpu-rects"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_rects"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: 16,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            primitive: tri_primitive(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview: None,
            cache: None,
        });

        // Líneas con grosor: cada segmento es una instancia de 20 B; el
        // VS expande a un quad de 6 vértices perpendicular al segmento
        // usando un grosor uniforme en píxeles (vienen del uniform).
        let lines = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-raster-gpu-lines"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_lines"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Uint32,
                            offset: 16,
                            shader_location: 1,
                        },
                    ],
                }],
            },
            primitive: tri_primitive(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview: None,
            cache: None,
        });

        Self {
            lines,
            tris,
            rects,
            bind_layout,
        }
    }
}

fn tri_primitive() -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        strip_index_format: None,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: None,
        unclipped_depth: false,
        polygon_mode: wgpu::PolygonMode::Fill,
        conservative: false,
    }
}

/// Acumulador de primitivas por frame. Construir → `add_*` → `flush`.
pub struct GpuBatch<'a> {
    pipelines: &'a GpuPipelines,
    line_verts: Vec<u8>,
    tri_verts: Vec<u8>,
    rect_insts: Vec<u8>,
    line_width: f32,
    line_count: u32,
    tri_vert_count: u32,
    rect_count: u32,
}

impl<'a> GpuBatch<'a> {
    pub fn new(pipelines: &'a GpuPipelines) -> Self {
        Self {
            pipelines,
            line_verts: Vec::new(),
            tri_verts: Vec::new(),
            rect_insts: Vec::new(),
            line_width: 1.0,
            line_count: 0,
            tri_vert_count: 0,
            rect_count: 0,
        }
    }

    /// Grosor de las próximas líneas (en pixels del frame, sin AA).
    /// Se aplica a todas las líneas del batch — el lado bueno de una
    /// sola draw call es que sólo hay un grosor "vivo" por flush.
    pub fn line_width(&mut self, w: f32) {
        self.line_width = w;
    }

    /// Añade un segmento de línea como instancia.
    pub fn add_line(&mut self, p0: (f32, f32), p1: (f32, f32), color: Color) {
        let rgba = pack_rgba(color);
        self.line_verts.extend_from_slice(&p0.0.to_ne_bytes());
        self.line_verts.extend_from_slice(&p0.1.to_ne_bytes());
        self.line_verts.extend_from_slice(&p1.0.to_ne_bytes());
        self.line_verts.extend_from_slice(&p1.1.to_ne_bytes());
        self.line_verts.extend_from_slice(&rgba.to_ne_bytes());
        self.line_count += 1;
    }

    /// Añade una polilínea como secuencia de segmentos individuales
    /// (line-list). Para N puntos emite N-1 instancias.
    pub fn add_polyline(&mut self, points: &[(f32, f32)], color: Color) {
        if points.len() < 2 {
            return;
        }
        for w in points.windows(2) {
            self.add_line(w[0], w[1], color);
        }
    }

    /// Añade un triángulo con color por vértice.
    pub fn add_tri(
        &mut self,
        a: (f32, f32),
        b: (f32, f32),
        c: (f32, f32),
        ca: Color,
        cb: Color,
        cc: Color,
    ) {
        self.push_tri_vert(a, ca);
        self.push_tri_vert(b, cb);
        self.push_tri_vert(c, cc);
    }

    fn push_tri_vert(&mut self, p: (f32, f32), color: Color) {
        let rgba = pack_rgba(color);
        self.tri_verts.extend_from_slice(&p.0.to_ne_bytes());
        self.tri_verts.extend_from_slice(&p.1.to_ne_bytes());
        self.tri_verts.extend_from_slice(&rgba.to_ne_bytes());
        self.tri_vert_count += 1;
    }

    /// Añade un triangle list crudo `[(x, y); 3*N]` con un mismo color
    /// uniforme por vértice. Útil para teselaciones precomputadas
    /// (contornos, polígonos rellenos).
    pub fn add_tri_list(&mut self, verts: &[(f32, f32)], color: Color) {
        for &p in verts {
            self.push_tri_vert(p, color);
        }
    }

    /// Añade un rectángulo lleno como instancia (sin radio — para
    /// rounded rects sigue por vello).
    pub fn add_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let rgba = pack_rgba(color);
        self.rect_insts.extend_from_slice(&x.to_ne_bytes());
        self.rect_insts.extend_from_slice(&y.to_ne_bytes());
        self.rect_insts.extend_from_slice(&w.to_ne_bytes());
        self.rect_insts.extend_from_slice(&h.to_ne_bytes());
        self.rect_insts.extend_from_slice(&rgba.to_ne_bytes());
        self.rect_count += 1;
    }

    /// Cuenta total de primitivas pendientes (útil para benches).
    pub fn primitive_count(&self) -> u32 {
        self.line_count + self.rect_count + self.tri_vert_count / 3
    }

    /// Despacha las primitivas acumuladas como 1 draw call por tipo
    /// no vacío contra `view`. `viewport` es el tamaño en pixels del
    /// target (lo usa el VS para mapear pixel → NDC).
    ///
    /// `load_op` decide si la pasada conserva el contenido previo
    /// (`Load`, lo normal cuando vello ya pintó algo) o limpia
    /// (`Clear(color)`). Apps que llamen a `GpuBatch` desde
    /// `gpu_paint_with` quieren `Load`.
    pub fn flush(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        viewport: (f32, f32),
        load_op: wgpu::LoadOp<wgpu::Color>,
    ) {
        let total = self.line_count + self.tri_vert_count + self.rect_count;
        if total == 0 {
            return;
        }

        // Uniforms: [viewport.w, viewport.h, line_width, _pad].
        let u_data = [viewport.0, viewport.1, self.line_width, 0.0];
        let mut u_bytes = Vec::with_capacity(16);
        for v in u_data {
            u_bytes.extend_from_slice(&v.to_ne_bytes());
        }
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("llimphi-raster-gpu-u"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniforms, 0, &u_bytes);

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-raster-gpu-bg"),
            layout: &self.pipelines.bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        // Buffers por tipo (sólo si hay datos).
        let lines_buf = (!self.line_verts.is_empty()).then(|| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llimphi-raster-gpu-lines-buf"),
                size: self.line_verts.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, &self.line_verts);
            b
        });
        let tris_buf = (!self.tri_verts.is_empty()).then(|| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llimphi-raster-gpu-tris-buf"),
                size: self.tri_verts.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, &self.tri_verts);
            b
        });
        let rects_buf = (!self.rect_insts.is_empty()).then(|| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llimphi-raster-gpu-rects-buf"),
                size: self.rect_insts.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, &self.rect_insts);
            b
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-raster-gpu-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);

        // Orden de draws: rects (fondo) → tris → lines (encima). Match
        // de la convención usual "fill abajo, stroke arriba".
        if let Some(buf) = rects_buf.as_ref() {
            pass.set_pipeline(&self.pipelines.rects);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..6, 0..self.rect_count);
        }
        if let Some(buf) = tris_buf.as_ref() {
            pass.set_pipeline(&self.pipelines.tris);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..self.tri_vert_count, 0..1);
        }
        if let Some(buf) = lines_buf.as_ref() {
            pass.set_pipeline(&self.pipelines.lines);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..6, 0..self.line_count);
        }
    }
}

/// Empaqueta un `peniko::Color` a u32 little-endian RGBA8.
/// El shader lo lee como `inst.rgba` y separa bytes — debe coincidir
/// con la convención del WGSL (`r = rgba & 0xFF`, etc.).
fn pack_rgba(c: Color) -> u32 {
    let [r, g, b, a] = c.to_rgba8().to_u8_array();
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | ((a as u32) << 24)
}

const WGSL: &str = r#"
struct Uniforms {
    viewport:   vec2<f32>,
    line_width: f32,
    _pad:       f32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct V2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

fn unpack_rgba(c: u32) -> vec4<f32> {
    let r = f32( c        & 0xFFu) / 255.0;
    let g = f32((c >>  8u) & 0xFFu) / 255.0;
    let b = f32((c >> 16u) & 0xFFu) / 255.0;
    let a = f32((c >> 24u) & 0xFFu) / 255.0;
    return vec4<f32>(r, g, b, a);
}

fn px_to_ndc(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(p.x / u.viewport.x * 2.0 - 1.0, 1.0 - p.y / u.viewport.y * 2.0);
}

// -------- triángulos: 1 vértice = (xy, rgba) --------

@vertex
fn vs_tris(@location(0) xy: vec2<f32>, @location(1) rgba: u32) -> V2F {
    var out: V2F;
    out.pos = vec4<f32>(px_to_ndc(xy), 0.0, 1.0);
    out.color = unpack_rgba(rgba);
    return out;
}

// -------- rects: 1 instancia = (xy, wh, rgba), 6 vértices/quad --------

@vertex
fn vs_rects(
    @builtin(vertex_index) vid: u32,
    @location(0) inst_xy: vec2<f32>,
    @location(1) inst_wh: vec2<f32>,
    @location(2) inst_rgba: u32,
) -> V2F {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 1.0),
    );
    let local = corners[vid];
    let px = inst_xy + local * inst_wh;
    var out: V2F;
    out.pos = vec4<f32>(px_to_ndc(px), 0.0, 1.0);
    out.color = unpack_rgba(inst_rgba);
    return out;
}

// -------- líneas: 1 instancia = (p0xy, p1xy, rgba), expandida a quad ----

@vertex
fn vs_lines(
    @builtin(vertex_index) vid: u32,
    @location(0) seg: vec4<f32>,
    @location(1) rgba: u32,
) -> V2F {
    // Quad perpendicular al segmento, grosor uniforme `u.line_width` px.
    // vid 0..5 mapea a los 6 vértices del quad (2 tris).
    let p0 = seg.xy;
    let p1 = seg.zw;
    let dir = normalize(p1 - p0);
    let n = vec2<f32>(-dir.y, dir.x);
    let half_w = u.line_width * 0.5;
    let offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -half_w),  // p0 -n
        vec2<f32>(0.0,  half_w),  // p0 +n
        vec2<f32>(1.0,  half_w),  // p1 +n
        vec2<f32>(0.0, -half_w),  // p0 -n
        vec2<f32>(1.0,  half_w),  // p1 +n
        vec2<f32>(1.0, -half_w),  // p1 -n
    );
    let o = offsets[vid];
    let along = mix(p0, p1, o.x);
    let across = n * o.y;
    let px = along + across;
    var out: V2F;
    out.pos = vec4<f32>(px_to_ndc(px), 0.0, 1.0);
    out.color = unpack_rgba(rgba);
    return out;
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
