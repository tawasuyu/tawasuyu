//! Spike Fase 0 — GPU directo vs vello.
//!
//! Compara el tiempo total CPU+GPU por frame para pintar N puntos en una
//! textura `Rgba8Unorm` 1024×1024 con dos estrategias:
//!
//! - **Vello**: una llamada `Scene::fill(Rect 1×1)` por punto, luego
//!   `vello::Renderer::render_to_texture`.
//! - **GPU directo**: un pipeline `wgpu` con instanced quad. Cada punto es
//!   una instancia `[x:f32, y:f32, rgba:u32]`. Una sola draw call.
//!
//! Tamaños: 100K, 500K, 1M puntos. 10 frames de warmup + 20 medidos por
//! tamaño. Reporta mediana y factor de aceleración.
//!
//! Criterio de aceptación del SDD (`llimphi/SDD.md` §"GPU directo wgpu"):
//! factor ≥ 5× a 500K → seguir con Fase 1. Si no, abortar.
//!
//! Corre con: `cargo run -p llimphi-raster --example spike_gpu_directo --release`.

use std::io::Write;
use std::time::Instant;

use llimphi_hal::{wgpu, Hal};
use llimphi_raster::{
    kurbo::{Affine, Rect},
    peniko::{color::palette, Color, Fill},
    vello,
};

const W: u32 = 1024;
const H: u32 = 1024;
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const WARMUP_FRAMES: usize = 5;
const MEASURED_FRAMES: usize = 15;
// Vello revienta (SIGSEGV en `vello_encoding::path::flatten`) cuando la
// escena pasa de ~200K paths con los `Limits::default()` que pide el HAL.
// Es exactamente el techo del SDD §"GPU directo wgpu". Lo medimos hasta
// donde vello aguanta; el lado directo se mide a sizes mucho mayores para
// confirmar el régimen post-techo.
const VELLO_SIZES: &[usize] = &[25_000, 50_000, 100_000, 200_000];
const DIRECTO_SIZES: &[usize] = &[100_000, 500_000, 1_000_000, 5_000_000];

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");

    // Textura destino compartida por ambos backends. STORAGE_BINDING para
    // vello (compute), RENDER_ATTACHMENT para el pipeline directo. Idéntica
    // al `intermediate` de `WinitSurface` (HAL real).
    let (target, target_view) = create_target(&hal.device);

    let mut vello_renderer = vello::Renderer::new(
        &hal.device,
        vello::RendererOptions {
            use_cpu: false,
            antialiasing_support: vello::AaSupport {
                area: true,
                msaa8: false,
                msaa16: false,
            },
            num_init_threads: None,
            pipeline_cache: None,
        },
    )
    .expect("vello renderer");

    let directo = DirectoPipeline::new(&hal.device);

    println!();
    println!("spike GPU directo — target {W}×{H} Rgba8Unorm, headless");
    println!("warmup {WARMUP_FRAMES}, measured {MEASURED_FRAMES}");
    println!();
    println!("vello (scene.fill por punto):");
    println!("  {:>10} | {:>14}", "N", "ms / frame");
    println!("  {:->10} + {:->14}", "", "");
    let mut vello_100k_ms: Option<f64> = None;
    for &n in VELLO_SIZES {
        let points = gen_points(n);
        let ms = bench_vello(&hal, &mut vello_renderer, &target_view, &points);
        println!("  {:>10} | {:>14.3}", n, ms);
        let _ = std::io::stdout().flush();
        if n == 100_000 {
            vello_100k_ms = Some(ms);
        }
    }
    println!();
    println!("GPU directo (instanced quad, 1 draw call):");
    println!("  {:>10} | {:>14}", "N", "ms / frame");
    println!("  {:->10} + {:->14}", "", "");
    let mut directo_100k_ms: Option<f64> = None;
    for &n in DIRECTO_SIZES {
        let points = gen_points(n);
        let ms = bench_directo(&hal, &directo, &target_view, &points);
        println!("  {:>10} | {:>14.3}", n, ms);
        let _ = std::io::stdout().flush();
        if n == 100_000 {
            directo_100k_ms = Some(ms);
        }
    }
    println!();
    if let (Some(v), Some(d)) = (vello_100k_ms, directo_100k_ms) {
        let factor = v / d;
        let verdict = if factor >= 5.0 { "PASA" } else { "ABORTAR" };
        println!(
            "veredicto Fase 0 @ 100K: vello {:.2} ms / directo {:.2} ms = {:.2}× → {}",
            v, d, factor, verdict
        );
        println!("(SDD pide ≥5× a 500K, pero vello no llega a 500K — techo medido <300K)");
    }
    println!();
    // Mantener vivo el texture para evitar warnings.
    drop(target);
}

fn create_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("spike-target"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TARGET_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// LCG numerical recipes — determinista, sin dependencias.
fn gen_points(n: usize) -> Vec<(f32, f32, u32)> {
    let mut state: u32 = 0x1234_5678;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x = (state % W) as f32;
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let y = (state % H) as f32;
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        // RGBA packed little-endian: R en byte bajo (queda igual a como lo
        // lee el shader: `rgba & 0xFF` → R).
        let rgba = (state & 0x00FF_FFFF) | 0xFF00_0000;
        out.push((x, y, rgba));
    }
    out
}

fn bench_vello(
    hal: &Hal,
    renderer: &mut vello::Renderer,
    target: &wgpu::TextureView,
    points: &[(f32, f32, u32)],
) -> f64 {
    let mut scene = vello::Scene::new();
    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED_FRAMES);
    for frame in 0..(WARMUP_FRAMES + MEASURED_FRAMES) {
        let t0 = Instant::now();
        scene.reset();
        for &(x, y, rgba) in points {
            let r = (rgba & 0xFF) as u8;
            let g = ((rgba >> 8) & 0xFF) as u8;
            let b = ((rgba >> 16) & 0xFF) as u8;
            let a = ((rgba >> 24) & 0xFF) as u8;
            let xf = x as f64;
            let yf = y as f64;
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                Color::from_rgba8(r, g, b, a),
                None,
                &Rect::new(xf, yf, xf + 1.0, yf + 1.0),
            );
        }
        renderer
            .render_to_texture(
                &hal.device,
                &hal.queue,
                &scene,
                target,
                &vello::RenderParams {
                    base_color: palette::css::BLACK,
                    width: W,
                    height: H,
                    antialiasing_method: vello::AaConfig::Area,
                },
            )
            .expect("vello render");
        // Bloquear hasta que la GPU termine este frame. Sin esto medimos
        // sólo el submit + queue building, no el trabajo real.
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP_FRAMES {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

fn bench_directo(
    hal: &Hal,
    pipe: &DirectoPipeline,
    target: &wgpu::TextureView,
    points: &[(f32, f32, u32)],
) -> f64 {
    // Buffer de instancias dimensionado para el peor caso.
    let bytes_per_inst = std::mem::size_of::<[u32; 3]>(); // [x:f32, y:f32, rgba:u32] = 12B
    let inst_buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spike-directo-inst"),
        size: (points.len() * bytes_per_inst) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut samples: Vec<f64> = Vec::with_capacity(MEASURED_FRAMES);
    for frame in 0..(WARMUP_FRAMES + MEASURED_FRAMES) {
        let t0 = Instant::now();
        // Empaquetar instancias: igual a la "scene build" del lado vello,
        // para que la comparación sea fair (ambos parten de los mismos
        // puntos crudos).
        let bytes = pack_instances(points);
        hal.queue.write_buffer(&inst_buf, 0, &bytes);

        let mut encoder = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spike-directo-enc"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spike-directo-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipe.pipeline);
            pass.set_vertex_buffer(0, inst_buf.slice(..));
            // 6 vértices por instancia (2 tris = quad), N instancias.
            pass.draw(0..6, 0..points.len() as u32);
        }
        hal.queue.submit(std::iter::once(encoder.finish()));
        hal.device.poll(wgpu::Maintain::Wait);
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if frame >= WARMUP_FRAMES {
            samples.push(dt);
        }
    }
    median(&mut samples)
}

fn pack_instances(points: &[(f32, f32, u32)]) -> Vec<u8> {
    let mut v = Vec::with_capacity(points.len() * 12);
    for &(x, y, rgba) in points {
        v.extend_from_slice(&x.to_ne_bytes());
        v.extend_from_slice(&y.to_ne_bytes());
        v.extend_from_slice(&rgba.to_ne_bytes());
    }
    v
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

/// Pipeline trivial para el bench: instanced quad sin texturas, color
/// per-instance. No es código de producción — es el "mock GPU directo"
/// que pide la Fase 0 del SDD para medir el techo alcanzable.
struct DirectoPipeline {
    pipeline: wgpu::RenderPipeline,
}

impl DirectoPipeline {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spike-directo-shader"),
            source: wgpu::ShaderSource::Wgsl(WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spike-directo-layout"),
            bind_group_layouts: &[],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spike-directo-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Instance,
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview: None,
            cache: None,
        });
        Self { pipeline }
    }
}

const WGSL: &str = r#"
struct Inst {
    @location(0) xy: vec2<f32>,
    @location(1) rgba: u32,
};

struct V2F {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

const W: f32 = 1024.0;
const H: f32 = 1024.0;

@vertex
fn vs(@builtin(vertex_index) vid: u32, inst: Inst) -> V2F {
    // Quad 1.5px alrededor de (inst.xy + 0.5). Pixel-centered.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.75, -0.75),
        vec2<f32>( 0.75, -0.75),
        vec2<f32>( 0.75,  0.75),
        vec2<f32>(-0.75, -0.75),
        vec2<f32>( 0.75,  0.75),
        vec2<f32>(-0.75,  0.75),
    );
    let off = corners[vid];
    let px = inst.xy + vec2<f32>(0.5, 0.5) + off;
    // pixel → NDC, Y invertido (vello / textura framebuffer).
    let ndc = vec2<f32>(px.x / W * 2.0 - 1.0, 1.0 - px.y / H * 2.0);

    let r = f32( inst.rgba        & 0xFFu) / 255.0;
    let g = f32((inst.rgba >>  8u) & 0xFFu) / 255.0;
    let b = f32((inst.rgba >> 16u) & 0xFFu) / 255.0;
    let a = f32((inst.rgba >> 24u) & 0xFFu) / 255.0;

    var out: V2F;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.color = vec4<f32>(r, g, b, a);
    return out;
}

@fragment
fn fs(in: V2F) -> @location(0) vec4<f32> {
    return in.color;
}
"#;
