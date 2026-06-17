//! Backend GPU directo (Fases 2 + 3 del SDD §"GPU directo wgpu").
//!
//! Cuatro pipelines `wgpu` cacheadas en [`GpuPipelines`] (lines / tris /
//! rects / discs) + un acumulador [`GpuBatch`] que las apps usan por frame para
//! emitir centenares de miles a millones de primitivos en una draw call
//! por tipo, sin pasar por vello.
//!
//! Diseño minimal Fase 2/3:
//!
//! - Vertex format triángulos: `[x: f32, y: f32, rgba: u32]` (12 B/vert).
//! - Instance format líneas: `[x0, y0, x1, y1, rgba]` (20 B/seg).
//! - Instance format rects:  `[x, y, w, h, rgba]` (20 B/rect).
//! - Instance format discos: `[cx, cy, r, stroke, rgba]` (20 B/disco).
//! - Sin texturas. Rects/líneas/tris obtienen AA de **bordes** vía MSAA 4×
//!   (ver más abajo); los discos SÍ traen AA por SDF en el fragment
//!   (smoothstep sobre `fwidth`), que MSAA respeta. Así rects/tris/líneas
//!   instanciados salen con bordes suaves sin que el caller toque nada.
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
//!
//! ## MSAA 4× (antialiasing de bordes)
//!
//! El pase no dibuja directo sobre el `view` que recibe `flush`. En su
//! lugar rasteriza todos los primitivos a una textura **multisample 4×**
//! (cleared a transparente), la *resuelve* a una textura single-sample
//! scratch y **compone con alpha** ese resultado sobre el `view`. Así:
//!
//! - Los bordes de rects/tris/líneas quedan suaves (4 muestras/pixel),
//!   no escalonados.
//! - El contenido previo del `view` (lo que vello pintó) se preserva,
//!   porque el composite es alpha-over con `LoadOp::Load` — exactamente
//!   la semántica que tenía el viejo render pass directo con `LoadOp::Load`.
//! - `LoadOp::Clear(c)` se respeta: el `view` se limpia a `c` antes del
//!   composite (equivalente a la pasada directa anterior).
//!
//! Backward-compat: la firma pública de `flush` / `GpuPipelines::new` no
//! cambia. Las texturas MSAA + scratch se crean por-flush dimensionadas
//! al `viewport` (mismo patrón que los buffers por-frame), así el resize
//! "sale gratis" — cada frame usa el tamaño que se le pasa, sin estado
//! persistente que recrear. El pipeline de composite se compila una vez
//! y se cachea en `GpuPipelines` (es `Sync`, vive en `OnceLock`).

use llimphi_hal::wgpu;
use vello::peniko::Color;

/// Número de muestras del MSAA del pase GPU. 4× es el punto dulce
/// universal (soportado por todo hardware moderno, coste moderado).
const MSAA_SAMPLES: u32 = 4;

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
    /// Discos/anillos rellenos con AA por SDF en el fragment. Instance
    /// format: `[cx, cy, r, stroke, rgba]` (20 B/disco). `stroke <= 0`
    /// → disco lleno; `stroke > 0` → anillo de ese grosor (px). Ver
    /// [`GpuBatch::add_disc`] / [`GpuBatch::add_ring`].
    pub discs: wgpu::RenderPipeline,
    pub bind_layout: wgpu::BindGroupLayout,
    /// Pipeline de pantalla completa que compone (alpha-over) la textura
    /// scratch resuelta del MSAA sobre el `view` del `flush`. Single-sample.
    /// El formato del target es el `color_format` con el que se construyó.
    composite: wgpu::RenderPipeline,
    composite_bgl: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    /// Formato de color del target — necesario para crear las texturas
    /// MSAA/scratch del `flush` con el mismo formato que el `view`.
    color_format: wgpu::TextureFormat,
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
            multisample: wgpu::MultisampleState {
                count: MSAA_SAMPLES,
                ..Default::default()
            },
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
            multisample: wgpu::MultisampleState {
                count: MSAA_SAMPLES,
                ..Default::default()
            },
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
            multisample: wgpu::MultisampleState {
                count: MSAA_SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview: None,
            cache: None,
        });

        // Discos/anillos (instanced quad + SDF AA en el fragment). Cada
        // disco es una instancia de 24 B: `[cx, cy, r, stroke, rgba]`. El
        // VS expande un quad que cubre el disco (con 1 px de margen para
        // que el smoothstep del borde no se recorte) y pasa al FS la
        // posición local en px; el FS calcula la distancia al centro y
        // hace smoothstep sobre ~1 px (`fwidth`) → borde antialiased.
        let discs = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-raster-gpu-discs"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_discs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 20,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        // cx, cy
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        // r, stroke
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
                        // rgba
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
            multisample: wgpu::MultisampleState {
                count: MSAA_SAMPLES,
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_disc"),
                compilation_options: Default::default(),
                targets: &color_targets,
            }),
            multiview: None,
            cache: None,
        });

        // Pipeline de composite (alpha-over) de la scratch resuelta del
        // MSAA sobre el `view`. Single-sample (count = 1), pase fullscreen
        // de un triángulo. Asume alpha **premultiplicado** — el MSAA + el
        // blending de los primitivos producen color premultiplicado, así
        // que el over correcto es `src.rgb*1 + dst.rgb*(1-src.a)`.
        let composite_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("llimphi-raster-gpu-composite-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("llimphi-raster-gpu-composite-pl"),
            bind_group_layouts: &[&composite_bgl],
            push_constant_ranges: &[],
        });
        let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("llimphi-raster-gpu-composite"),
            layout: Some(&composite_pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_composite"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_composite"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("llimphi-raster-gpu-composite-sampler"),
            ..Default::default()
        });

        Self {
            lines,
            tris,
            rects,
            discs,
            bind_layout,
            composite,
            composite_bgl,
            composite_sampler,
            color_format,
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
    disc_insts: Vec<u8>,
    line_width: f32,
    line_count: u32,
    tri_vert_count: u32,
    rect_count: u32,
    disc_count: u32,
}

impl<'a> GpuBatch<'a> {
    pub fn new(pipelines: &'a GpuPipelines) -> Self {
        Self {
            pipelines,
            line_verts: Vec::new(),
            tri_verts: Vec::new(),
            rect_insts: Vec::new(),
            disc_insts: Vec::new(),
            line_width: 1.0,
            line_count: 0,
            tri_vert_count: 0,
            rect_count: 0,
            disc_count: 0,
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

    /// Añade un disco (círculo relleno) con AA por shader como instancia.
    /// `(cx, cy)` es el centro y `r` el radio, ambos en pixels del frame.
    /// El borde queda antialiased vía un SDF + `smoothstep` de ~1 px en
    /// el fragment — no escalonado, sin MSAA. El alpha del color se
    /// respeta (blending alfa activo).
    pub fn add_disc(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        self.push_disc(cx, cy, r, 0.0, color);
    }

    /// Añade un anillo (círculo hueco / stroke circular) con AA por
    /// shader. `r` es el radio exterior; `stroke` el grosor del trazo en
    /// px (el agujero interior tiene radio `r - stroke`). `stroke <= 0`
    /// degenera en un disco lleno. Ambos bordes (externo e interno)
    /// quedan antialiased.
    pub fn add_ring(&mut self, cx: f32, cy: f32, r: f32, stroke: f32, color: Color) {
        self.push_disc(cx, cy, r, stroke.max(0.0), color);
    }

    fn push_disc(&mut self, cx: f32, cy: f32, r: f32, stroke: f32, color: Color) {
        let rgba = pack_rgba(color);
        self.disc_insts.extend_from_slice(&cx.to_ne_bytes());
        self.disc_insts.extend_from_slice(&cy.to_ne_bytes());
        self.disc_insts.extend_from_slice(&r.to_ne_bytes());
        self.disc_insts.extend_from_slice(&stroke.to_ne_bytes());
        self.disc_insts.extend_from_slice(&rgba.to_ne_bytes());
        self.disc_count += 1;
    }

    /// Cuenta total de primitivas pendientes (útil para benches).
    pub fn primitive_count(&self) -> u32 {
        self.line_count + self.rect_count + self.disc_count + self.tri_vert_count / 3
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
        let total =
            self.line_count + self.tri_vert_count + self.rect_count + self.disc_count;
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
        let discs_buf = (!self.disc_insts.is_empty()).then(|| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("llimphi-raster-gpu-discs-buf"),
                size: self.disc_insts.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&b, 0, &self.disc_insts);
            b
        });

        // ── MSAA 4× ──────────────────────────────────────────────────
        // Texturas por-flush dimensionadas al viewport (mismo patrón que
        // los buffers de arriba; el resize "sale gratis"). `tex_w/h` se
        // clampean a ≥1 para evitar Extent3d de 0 (un viewport degenerado
        // no debería llegar acá, pero defensivo).
        let tex_w = (viewport.0.round() as u32).max(1);
        let tex_h = (viewport.1.round() as u32).max(1);
        let extent = wgpu::Extent3d {
            width: tex_w,
            height: tex_h,
            depth_or_array_layers: 1,
        };
        let fmt = self.pipelines.color_format;
        // Color attachment multisample: lo rasterizan los 4 pipelines.
        let msaa_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-raster-gpu-msaa"),
            size: extent,
            mip_level_count: 1,
            sample_count: MSAA_SAMPLES,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let msaa_view = msaa_tex.create_view(&wgpu::TextureViewDescriptor::default());
        // Scratch single-sample: recibe el resolve del MSAA y luego se
        // samplea en el composite sobre el `view`.
        let resolve_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("llimphi-raster-gpu-resolve"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: fmt,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let resolve_view =
            resolve_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // Pase de primitivos: MSAA cleared a TRANSPARENT, resuelto al
        // scratch single-sample. El scratch queda con alpha
        // **premultiplicado** (el blending alfa de los pipelines sobre
        // fondo transparente produce `rgb = color*alpha`, `a = alpha`).
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("llimphi-raster-gpu-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &msaa_view,
                    resolve_target: Some(&resolve_view),
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &bind_group, &[]);

            // Orden de draws: rects (fondo) → discos → tris → lines (encima).
            // Match de la convención usual "fill abajo, stroke arriba".
            if let Some(buf) = rects_buf.as_ref() {
                pass.set_pipeline(&self.pipelines.rects);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..self.rect_count);
            }
            if let Some(buf) = discs_buf.as_ref() {
                pass.set_pipeline(&self.pipelines.discs);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..self.disc_count);
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

        // Composite del scratch resuelto sobre el `view`. Respeta el
        // `load_op` recibido:
        //  - `Load`  → alpha-over: preserva lo que ya está en `view`
        //              (vello), exactamente como el viejo pase directo.
        //  - `Clear` → limpia `view` al color pedido y luego compone
        //              el scratch encima (mismo resultado que limpiar y
        //              dibujar directo).
        let composite_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("llimphi-raster-gpu-composite-bg"),
            layout: &self.pipelines.composite_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&resolve_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(
                        &self.pipelines.composite_sampler,
                    ),
                },
            ],
        });
        let mut cpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("llimphi-raster-gpu-composite-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    // El blend del pipeline ya hace el alpha-over; con
                    // Load conserva el fondo, con Clear lo borra primero.
                    load: load_op,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        cpass.set_pipeline(&self.pipelines.composite);
        cpass.set_bind_group(0, &composite_bg, &[]);
        cpass.draw(0..3, 0..1);
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

// -------- discos/anillos: 1 instancia = (cxcy, r/stroke, rgba) --------
//
// Quad que cubre el disco con 1.5 px de margen (para que el smoothstep
// del borde no se recorte). El VS pasa al FS la posición local en px
// relativa al centro; el FS evalúa el SDF del círculo y hace smoothstep
// sobre `fwidth` → borde antialiased. `stroke > 0` recorta un anillo.

struct DiscV2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,  // px relativos al centro
    @location(2) params: vec2<f32>, // r, stroke (px)
};

@vertex
fn vs_discs(
    @builtin(vertex_index) vid: u32,
    @location(0) inst_c: vec2<f32>,
    @location(1) inst_rs: vec2<f32>,
    @location(2) inst_rgba: u32,
) -> DiscV2F {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    let r = inst_rs.x;
    let margin = r + 1.5;          // 1.5 px de aire para el AA del borde
    let local = corners[vid] * margin;
    let px = inst_c + local;
    var out: DiscV2F;
    out.pos = vec4<f32>(px_to_ndc(px), 0.0, 1.0);
    out.color = unpack_rgba(inst_rgba);
    out.local = local;
    out.params = inst_rs;
    return out;
}

@fragment
fn fs_disc(in: DiscV2F) -> @location(0) vec4<f32> {
    let r = in.params.x;
    let stroke = in.params.y;
    let dist = length(in.local);     // distancia al centro en px
    // Ancho del filtro AA en px (≈ 1 px en pantalla).
    let aa = fwidth(dist);
    // Borde exterior: cobertura 1 dentro de r, 0 fuera de r+aa.
    var cov = 1.0 - smoothstep(r - aa, r + aa, dist);
    // Anillo: si hay stroke, recortamos el agujero interior con AA.
    if (stroke > 0.0) {
        let inner = max(r - stroke, 0.0);
        cov = cov * smoothstep(inner - aa, inner + aa, dist);
    }
    if (cov <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * cov);
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    return in.color;
}

// -------- composite: blit fullscreen de la scratch resuelta del MSAA ----
//
// Triángulo de pantalla completa (3 vértices, sin vertex buffer). Samplea
// la scratch (alpha **premultiplicado**) y la emite tal cual; el alpha-over
// real lo hace el BlendState del pipeline (`One, OneMinusSrcAlpha`).

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;

struct CompV2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_composite(@builtin(vertex_index) vid: u32) -> CompV2F {
    // Triángulo gigante que cubre el viewport (técnica estándar).
    var uvs = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(2.0, 0.0),
        vec2<f32>(0.0, 2.0),
    );
    let uv = uvs[vid];
    var out: CompV2F;
    out.pos = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    // El framebuffer tiene Y hacia abajo; la textura, hacia arriba en UV.
    out.uv = vec2<f32>(uv.x, 1.0 - uv.y);
    return out;
}

@fragment
fn fs_composite(in: CompV2F) -> @location(0) vec4<f32> {
    return textureSample(src_tex, src_samp, in.uv);
}
"#;
