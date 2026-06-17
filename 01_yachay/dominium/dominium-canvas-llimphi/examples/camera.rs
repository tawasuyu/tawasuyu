//! `camera` — verificación headless del control de cámara (pan + zoom).
//!
//! Renderiza el MISMO mundo tres veces por el camino GPU (geometría opaca
//! como tris + sprites/glifos AA "over"), variando SÓLO la cámara, y escribe
//! tres PNG. Si pan/zoom están bien cableados, los tres difieren:
//!
//! - `/tmp/cam_base.png` — pan (0,0), scale default → maqueta centrada.
//! - `/tmp/cam_pan.png`  — pan (200,-120), mismo scale → maqueta corrida.
//! - `/tmp/cam_zoom.png` — scale ×2, pan (0,0) → maqueta más grande.
//!
//! El gesto de mouse en sí no se prueba sin pantalla; esto valida el
//! *plumbing* (que pan y scale llegan al render y producen pixeles distintos).
//!
//! ```bash
//! cargo run -p dominium-canvas-llimphi --example camera --release
//! ```

use std::fs::File;
use std::io::BufWriter;

use dominium_canvas_llimphi::bench;
use dominium_core::conceptos::{Concepto, Conceptos, LayerMods};
use dominium_core::worldgen;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan, PlanConfig, RenderPlan};
use llimphi_ui::llimphi_hal::{wgpu, Hal, OverlayCompositor};
use llimphi_ui::llimphi_raster::gpu::GpuBatch;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const W: u32 = 1280;
const H: u32 = 900;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BG: Color = Color::from_rgba8(11, 13, 18, 255);
const GRID: usize = 120;
const SCALE_DEFAULT: f32 = 3.0;

/// Construye el plan del mundo con una `scale` de cámara dada. El pan NO va
/// acá (es offset de pantalla aplicado en el render, no en el `RenderPlan`).
fn build_scene(scale: f32) -> RenderPlan {
    let lemmings = GRID * GRID / 23;
    let mut conceptos = Conceptos::new();
    for id in 1..=8u32 {
        let f = id as f32 / 9.0;
        conceptos.items.push(Concepto {
            id: format!("c{id}"),
            sprite_id: id,
            pos_x: GRID as f32 * (0.15 + 0.7 * f),
            pos_y: GRID as f32 * (0.2 + 0.6 * ((id as f32 * 0.37) % 1.0)),
            radius: 5.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
    }
    let world = worldgen::seed(0xD0_31_31_07, GRID, lemmings, conceptos);
    let iso = IsoProjector::new(scale, 0.55);
    let weights = ZWeights {
        materia: 0.02,
        psique: -0.075,
        poder: 0.40,
        oro: 0.0,
        degradacion: 1.30,
    };
    let cfg = PlanConfig {
        tile: 3.0,
        lemming_size: 2.6,
        ..PlanConfig::default()
    };
    build_plan(&world, &iso, &weights, &cfg)
}

fn main() {
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let pipelines = bench::Pipelines::new(&hal.device, FMT);
    let overlay = OverlayCompositor::new(&hal.device);
    let rect = PaintRect { x: 0.0, y: 0.0, w: W as f32, h: H as f32 };
    let mut ts = Typesetter::new();

    // El zoom escala el plan (más píxeles por celda); el pan es un offset de
    // pantalla. La maqueta a scale ×2 no entra centrada en el rect, pero eso
    // es exactamente lo que queremos ver (es "más grande").
    let plan_base = build_scene(SCALE_DEFAULT);
    let plan_zoom = build_scene(SCALE_DEFAULT * 2.0);

    render(&hal, &mut renderer, &pipelines, &overlay, &mut ts, rect, &plan_base, (0.0, 0.0), "/tmp/cam_base.png");
    render(&hal, &mut renderer, &pipelines, &overlay, &mut ts, rect, &plan_base, (200.0, -120.0), "/tmp/cam_pan.png");
    render(&hal, &mut renderer, &pipelines, &overlay, &mut ts, rect, &plan_zoom, (0.0, 0.0), "/tmp/cam_zoom.png");

    eprintln!("escritos /tmp/cam_base.png, /tmp/cam_pan.png, /tmp/cam_zoom.png ({W}x{H})");
}

/// Pipeline de render del camino GPU para una cámara dada (replica el orden
/// del eventloop: base vello → GPU tris → over vello composite).
#[allow(clippy::too_many_arguments)]
fn render(
    hal: &Hal,
    renderer: &mut Renderer,
    pipelines: &bench::Pipelines,
    overlay: &OverlayCompositor,
    ts: &mut Typesetter,
    rect: PaintRect,
    plan: &RenderPlan,
    pan: (f32, f32),
    path: &str,
) {
    let inter = make_tex(&hal.device);
    let view = inter.create_view(&wgpu::TextureViewDescriptor::default());

    // (1) base: el fondo.
    let base = vello::Scene::new();
    renderer.render_to_view(hal, &base, &view, W, H, BG).expect("base");

    // (2) GPU tris (geometría opaca), LoadOp::Load preserva el fondo.
    let mut batch = GpuBatch::new(pipelines);
    bench::emit_tris(plan, rect, &mut batch, pan);
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cam-gpu") });
    batch.flush(&hal.device, &hal.queue, &mut enc, &view, (W as f32, H as f32), wgpu::LoadOp::Load);
    hal.queue.submit(std::iter::once(enc.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    // (3) over: sprites + glifos AA en scratch transparente → composite.
    let scratch = make_tex(&hal.device);
    let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());
    let mut over = vello::Scene::new();
    bench::over_layer(plan, &mut over, ts, rect, pan);
    renderer.render_to_view(hal, &over, &scratch_view, W, H, Color::TRANSPARENT).expect("over");
    let mut enc2 = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("cam-over") });
    overlay.composite(&hal.device, &mut enc2, &view, &scratch_view);
    hal.queue.submit(std::iter::once(enc2.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

    write_png(hal, &inter, path);
}

fn make_tex(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cam-tex"),
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FMT,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn write_png(hal: &Hal, target: &wgpu::Texture, path: &str) {
    let unpadded = (W * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * H as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(H),
            },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((W * H * 4) as usize);
    for row in 0..H as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().unwrap();
    w.write_image_data(&pixels).unwrap();
}
