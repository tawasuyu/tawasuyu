//! `stress_1000` — stress test del camino GPU (Tier 1) a grid 1000×1000
//! (1M celdas), con culling-durante-build, a PNG + números honestos.
//!
//! Mide y reporta, a grid 1000:
//!   - build FULL (todas las celdas) — ms/frame + #primitivas (el techo de la
//!     arquitectura plan-por-frame).
//!   - build CULLED a una cámara con zoom (~1/N del mundo) — ms/frame + #prims.
//!   - raster GPU (Tier 1) de lo culled — ms/frame (hasta device.poll(wait)).
//! y escribe `/tmp/dom_1000.png`: la vista GPU de una región del mundo 1000².
//!
//! ```bash
//! cargo run -p dominium-canvas-llimphi --example stress_1000 --release -- [grid] [vp_px]
//! ```
//! Defaults: grid 1000, viewport 700 px (zoom a una región).

use std::fs::File;
use std::io::BufWriter;
use std::time::Instant;

use dominium_canvas_llimphi::bench;
use dominium_core::conceptos::{Concepto, Conceptos, LayerMods};
use dominium_core::worldgen;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{
    build_plan, build_plan_culled, PlanConfig, RenderPlan, Viewport,
};
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

fn iso() -> IsoProjector {
    IsoProjector::new(3.0, 0.55)
}
fn weights() -> ZWeights {
    ZWeights { materia: 0.02, psique: -0.075, poder: 0.40, oro: 0.0, degradacion: 1.30 }
}
fn cfg() -> PlanConfig {
    PlanConfig { tile: 3.0, lemming_size: 2.6, ..PlanConfig::default() }
}

fn make_world(grid: usize) -> dominium_core::World {
    let lemmings = grid * grid / 23;
    let mut conceptos = Conceptos::new();
    // Conceptos repartidos por TODO el mundo, así la región con zoom siempre
    // pesca alguno (ejercita el over-layer AA tras el culling de conceptos).
    for id in 1..=8u32 {
        let f = id as f32 / 9.0;
        conceptos.items.push(Concepto {
            id: format!("c{id}"),
            sprite_id: id,
            pos_x: grid as f32 * (0.10 + 0.8 * f),
            pos_y: grid as f32 * (0.45 + 0.1 * ((id as f32 * 0.37) % 1.0)),
            radius: 6.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
    }
    worldgen::seed(0xD0_31_31_07, grid, lemmings, conceptos)
}

fn ms(t0: Instant, n: u32) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0 / n as f64
}

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let vp_px: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(700.0);

    let world = make_world(grid);
    let (iso, w, c) = (iso(), weights(), cfg());

    // Viewport: cámara con zoom centrada en el centro del mundo proyectado.
    let mid = grid as f32 * 0.5;
    let (cx, cy) = iso.project(mid, mid, 0.0);
    let vp = Viewport::centered(cx, cy, vp_px, vp_px, vp_px * 0.05);

    println!("=== stress grid {grid}² ({} celdas) — Tier 1 GPU + culling ===", grid * grid);

    // ── build FULL ──────────────────────────────────────────────────────
    let full = build_plan(&world, &iso, &w, &c); // warmup
    let nf = 8;
    let t0 = Instant::now();
    let mut sink = 0usize;
    for _ in 0..nf {
        sink = sink.wrapping_add(build_plan(&world, &iso, &w, &c).polygons.len());
    }
    std::hint::black_box(sink);
    let full_ms = ms(t0, nf);
    println!(
        "\nbuild FULL    quads {:>7} polygons {:>8} (prims {:>8})  | {full_ms:>9.3} ms/frame",
        full.quads.len(),
        full.polygons.len(),
        full.quads.len() + full.polygons.len() + full.sprites.len(),
    );

    // ── build CULLED ────────────────────────────────────────────────────
    let culled = build_plan_culled(&world, &iso, &w, &c, vp); // warmup
    let nc = 60;
    let t0 = Instant::now();
    let mut sink = 0usize;
    for _ in 0..nc {
        sink = sink.wrapping_add(build_plan_culled(&world, &iso, &w, &c, vp).polygons.len());
    }
    std::hint::black_box(sink);
    let cull_ms = ms(t0, nc);
    println!(
        "build CULLED  quads {:>7} polygons {:>8} (prims {:>8})  | {cull_ms:>9.3} ms/frame  (vp {vp_px:.0}px)",
        culled.quads.len(),
        culled.polygons.len(),
        culled.quads.len() + culled.polygons.len() + culled.sprites.len(),
    );
    println!(
        "  → culling-durante-build: {:.1}× menos prims, {:.1}× más rápido que full",
        (full.quads.len() + full.polygons.len()) as f64
            / (culled.quads.len() + culled.polygons.len()).max(1) as f64,
        full_ms / cull_ms.max(1e-6),
    );

    // ── raster GPU (Tier 1) del plan culled ─────────────────────────────
    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let pipelines = bench::Pipelines::new(&hal.device, FMT);
    let overlay = OverlayCompositor::new(&hal.device);
    let rect = PaintRect { x: 0.0, y: 0.0, w: W as f32, h: H as f32 };
    let mut ts = Typesetter::new();

    let inter = make_tex(&hal.device);
    let view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    let scratch = make_tex(&hal.device);
    let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());

    let raster_once = |renderer: &mut Renderer, ts: &mut Typesetter, plan: &RenderPlan| {
        // base (fondo)
        let base = vello::Scene::new();
        renderer.render_to_view(&hal, &base, &view, W, H, BG).unwrap();
        // GPU tris (geometría opaca)
        let mut batch = GpuBatch::new(&pipelines);
        bench::emit_tris(plan, rect, &mut batch);
        let mut enc = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("s-gpu") });
        batch.flush(&hal.device, &hal.queue, &mut enc, &view, (W as f32, H as f32), wgpu::LoadOp::Load);
        hal.queue.submit(std::iter::once(enc.finish()));
        // over (sprites + glifos AA)
        let mut over = vello::Scene::new();
        bench::over_layer(plan, &mut over, ts, rect);
        renderer.render_to_view(&hal, &over, &scratch_view, W, H, Color::TRANSPARENT).unwrap();
        let mut enc2 = hal
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("s-over") });
        overlay.composite(&hal.device, &mut enc2, &view, &scratch_view);
        hal.queue.submit(std::iter::once(enc2.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
    };

    raster_once(&mut renderer, &mut ts, &culled); // warmup
    let nr = 30;
    let t0 = Instant::now();
    for _ in 0..nr {
        raster_once(&mut renderer, &mut ts, &culled);
    }
    let raster_ms = ms(t0, nr);
    println!("\nraster GPU (Tier 1) del culled  {raster_ms:>9.3} ms/frame  ({W}x{H})");
    println!(
        "build CULLED + raster GPU = {:.3} ms/frame  (≈ {:.0} fps con sim activa+zoom)",
        cull_ms + raster_ms,
        1000.0 / (cull_ms + raster_ms),
    );

    // ── PNG de la vista culled ──────────────────────────────────────────
    write_png(&hal, &inter, "/tmp/dom_1000.png");
    eprintln!("\nescrito /tmp/dom_1000.png ({W}x{H}) — vista GPU del mundo {grid}² con zoom");
}

fn make_tex(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("s-tex"),
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
