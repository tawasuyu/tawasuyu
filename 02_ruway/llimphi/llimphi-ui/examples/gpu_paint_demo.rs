//! Demo del hook GPU directo (`View::gpu_paint_with`) — Fase 1 del SDD
//! `02_ruway/llimphi/SDD.md` §"GPU directo wgpu".
//!
//! Pinta una grilla de N puntos coloridos sobre un panel central usando
//! un pipeline `wgpu` propio (instanced quad), encima de un fondo y
//! títulos pintados por vello. Valida que:
//!
//! - El callback `gpu_paint_with` recibe `(device, queue, encoder,
//!   view, rect)` con los recursos del runtime.
//! - El `LoadOp::Load` preserva la pasada vello (el fondo no se borra).
//! - El submit del encoder ocurre antes del `surface.present` (las
//!   primitivas GPU son visibles).
//!
//! Corre con: `cargo run -p llimphi-ui --example gpu_paint_demo --release`.

use std::sync::{Arc, OnceLock};

use llimphi_ui::llimphi_hal::wgpu;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, PaintRect, View};

const POINTS: u32 = 250_000;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone)]
enum Msg {
    Bump,
}

struct GpuDemo;

impl App for GpuDemo {
    type Model = u32;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · gpu_paint_demo"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        0
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Bump => model.wrapping_add(1),
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let title = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(48.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            format!("gpu_paint_with — {POINTS} puntos GPU directo · seed {model}"),
            22.0,
            Color::from_rgba8(220, 230, 245, 255),
        );

        // Canvas central: vello pinta el fondo (fill + radius), GPU pinta
        // la grilla de puntos encima vía gpu_paint_with. El seed del
        // modelo se mete en el shader vía una rotación trivial — cada
        // click cambia el patrón. El callback se invoca ya con el
        // CommandEncoder del frame y la TextureView intermediate.
        let seed = *model;
        let canvas = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(Color::from_rgba8(14, 18, 28, 255))
        .radius(8.0)
        .gpu_paint_with(move |device, queue, encoder, view, rect, _viewport| {
            draw_points(device, queue, encoder, view, rect, seed);
        })
        .on_click(Msg::Bump);

        let footer = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(28.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(
            "click sobre el canvas → rebobinar el seed",
            14.0,
            Color::from_rgba8(150, 165, 185, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(16.0_f32),
            },
            padding: TaffyRect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(24, 28, 38, 255))
        .children(vec![title, canvas, footer])
    }
}

fn main() {
    llimphi_ui::run::<GpuDemo>();
}

// ============================================================
// Lado GPU del demo: pipeline + buffer + draw call.
// ============================================================

/// Estado compartido del demo a través de los frames. Se construye en
/// el primer `gpu_paint_with` (cuando ya tenemos device/queue) y se
/// reutiliza después. Sin esto pagaríamos creación de pipeline + write
/// del buffer por frame, que es lo que `GpuBatch` resolverá de raíz en
/// Fase 3.
struct DemoGpu {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

fn shared() -> &'static OnceLock<Arc<DemoGpu>> {
    static SLOT: OnceLock<Arc<DemoGpu>> = OnceLock::new();
    &SLOT
}

fn draw_points(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    rect: PaintRect,
    seed: u32,
) {
    let gpu = shared()
        .get_or_init(|| Arc::new(DemoGpu::new(device)))
        .clone();

    // Uniforms: rect + seed → el VS los usa para colocar y colorear.
    let uniforms = [rect.x, rect.y, rect.w, rect.h, f32::from_bits(seed), 0.0, 0.0, 0.0];
    let mut bytes = Vec::with_capacity(32);
    for v in uniforms {
        bytes.extend_from_slice(&v.to_ne_bytes());
    }
    queue.write_buffer(&gpu.uniforms, 0, &bytes);

    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gpu_paint_demo-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Load preserva el fondo vello ya pintado en este frame.
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &gpu.bind_group, &[]);
        pass.set_vertex_buffer(0, gpu.instances.slice(..));
        pass.draw(0..6, 0..POINTS);
    }
}

impl DemoGpu {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_paint_demo-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu_paint_demo-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_paint_demo-pl"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu_paint_demo-pipe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 4,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Uint32,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });

        // Instance buffer: índice 0..POINTS empaquetado como u32.
        let mut idx_bytes = Vec::with_capacity((POINTS as usize) * 4);
        for i in 0..POINTS {
            idx_bytes.extend_from_slice(&i.to_ne_bytes());
        }
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_paint_demo-inst"),
            size: idx_bytes.len() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // El buffer ya vive el resto del programa — escribimos una vez.
        // Para esto necesitamos el queue, pero `new` no lo recibe. Lo
        // mantenemos como "lazy escrito en draw_points la primera vez";
        // por simplicidad lo escribimos en el primer queue.write_buffer
        // del flujo de uniforms. Actualmente el shader no usa la
        // instancia (sólo @builtin(vertex_index) + uniforms + builtin
        // instance_index), así que el buffer es ignorado — lo dejamos
        // para que el layout del pipeline siga válido y el día que
        // queramos meter datos por instancia ya está el slot listo.
        let _ = idx_bytes; // (no se sube — ver comentario arriba)

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gpu_paint_demo-u"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_paint_demo-bg"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            instances,
            uniforms,
            bind_group,
        }
    }
}

// Hash 32-bit barato (PCG-like) implementado en WGSL para mapear
// `instance_index + seed` → posición/color sin tocar buffers. Mantiene
// el demo en una sola draw call con cero CPU work por frame (salvo
// 32 bytes de uniforms).
const WGSL: &str = r#"
struct Uniforms {
    rect:   vec4<f32>, // x, y, w, h en pixels del frame
    seed:   u32,
    _pad0:  u32,
    _pad1:  u32,
    _pad2:  u32,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct V2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

fn hash(x: u32) -> u32 {
    var v = x ^ 2747636419u;
    v = v * 2654435769u;
    v = v ^ (v >> 16u);
    v = v * 2654435769u;
    v = v ^ (v >> 16u);
    v = v * 2654435769u;
    return v;
}

// La resolución real del frame no la conoce el shader sin un uniform
// adicional. Como aproximación robusta, asumimos que el callback se
// llama sobre un viewport "default" 960×540 (tamaño inicial del demo)
// y dejamos que rect.x/y/w/h centren los puntos dentro del canvas.
// El tamaño real del frame se debería pasar por uniforms en una versión
// no-demo — Fase 2/3 del SDD lo formaliza vía `GpuBatch`.
const FRAME_W: f32 = 960.0;
const FRAME_H: f32 = 540.0;

@vertex
fn vs(@builtin(vertex_index) vid: u32, @builtin(instance_index) iid: u32) -> V2F {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    let off = corners[vid] * 1.5; // quad de 3 pixels lado

    let h1 = hash(iid ^ u.seed);
    let h2 = hash(h1);
    let h3 = hash(h2);

    let fx = f32(h1 & 0xFFFFu) / 65535.0;
    let fy = f32(h2 & 0xFFFFu) / 65535.0;

    let px = u.rect.x + fx * u.rect.z + off.x;
    let py = u.rect.y + fy * u.rect.w + off.y;

    let ndc = vec2<f32>(
        px / FRAME_W * 2.0 - 1.0,
        1.0 - py / FRAME_H * 2.0,
    );

    let r = f32( h3        & 0xFFu) / 255.0;
    let g = f32((h3 >>  8u) & 0xFFu) / 255.0;
    let b = f32((h3 >> 16u) & 0xFFu) / 255.0;

    var out: V2F;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = vec4<f32>(r, g, b, 0.85);
    return out;
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
