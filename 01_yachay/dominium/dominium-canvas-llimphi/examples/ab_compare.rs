//! `ab_compare` — render headless A/B de la MISMA escena de dominium por
//! los dos caminos de rasterizado, a PNG, para validar paridad visual de
//! Tier 1 (GPU) contra el render histórico (vello).
//!
//! - **VIEJO** (`/tmp/dom_viejo.png`) — todo por vello (`bench::vello_full`),
//!   el render previo a Tier 1.
//! - **NUEVO** (`/tmp/dom_nuevo.png`) — geometría opaca por GPU
//!   (`bench::emit_tris` → `GpuBatch::flush`) + sprites/glifos AA en una
//!   pasada vello "over" compuesta encima (`bench::over_layer`). Replica el
//!   orden EXACTO del eventloop: `[vello base] → [GPU] → [vello over]`.
//!
//! Misma seed/cfg/cámara que la app (`bench_plan.rs`). El default es grid
//! 120; pasá otro como primer argumento.
//!
//! ```bash
//! cargo run -p dominium-canvas-llimphi --example ab_compare --release -- [grid]
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

fn build_scene(grid: usize) -> RenderPlan {
    let lemmings = grid * grid / 23; // ~2500 a grid 240, ~625 a 120
    // Conceptos con sprite_id 1..=8 repartidos por la grilla → ejercitan el
    // over-layer AA (íconos + auras/bases/topes). Así la comparación A/B
    // cubre también los sprites, no sólo terreno/lemmings.
    let mut conceptos = Conceptos::new();
    for id in 1..=8u32 {
        let f = id as f32 / 9.0;
        conceptos.items.push(Concepto {
            id: format!("c{id}"),
            sprite_id: id,
            pos_x: grid as f32 * (0.15 + 0.7 * f),
            pos_y: grid as f32 * (0.2 + 0.6 * ((id as f32 * 0.37) % 1.0)),
            radius: 5.0,
            mods: LayerMods::default(),
            hack: None,
            persuasion: None,
        });
    }
    let world = worldgen::seed(0xD0_31_31_07, grid, lemmings, conceptos);
    let iso = IsoProjector::new(3.0, 0.55);
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
    let grid: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);

    let plan = build_scene(grid);
    eprintln!(
        "escena grid {grid}: quads {} polygons {} sprites {} glyphs {}",
        plan.quads.len(),
        plan.polygons.len(),
        plan.sprites.len(),
        plan.glyphs.len()
    );

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let pipelines = bench::Pipelines::new(&hal.device, FMT);
    let overlay = OverlayCompositor::new(&hal.device);

    // Rect del canvas = el frame entero (el centrado usa rect.w/h).
    let rect = PaintRect { x: 0.0, y: 0.0, w: W as f32, h: H as f32 };
    let mut ts = Typesetter::new();

    // ── VIEJO: todo por vello en una pasada ────────────────────────────
    {
        let inter = make_tex(&hal.device);
        let view = inter.create_view(&wgpu::TextureViewDescriptor::default());
        let mut scene = vello::Scene::new();
        bench::vello_full(&plan, &mut scene, &mut ts, rect);
        renderer
            .render_to_view(&hal, &scene, &view, W, H, BG)
            .expect("render viejo");
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        write_png(&hal, &inter, "/tmp/dom_viejo.png");
    }

    // ── NUEVO: vello base (fondo) → GPU tris → vello over composite ─────
    {
        let inter = make_tex(&hal.device);
        let view = inter.create_view(&wgpu::TextureViewDescriptor::default());

        // (1) base: sólo el fondo (render_to_view limpia con BG).
        let base = vello::Scene::new();
        renderer
            .render_to_view(&hal, &base, &view, W, H, BG)
            .expect("render base");

        // (2) GPU: geometría opaca como tris, LoadOp::Load preserva el fondo.
        let mut batch = GpuBatch::new(&pipelines);
        bench::emit_tris(&plan, rect, &mut batch);
        let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ab-gpu"),
        });
        batch.flush(&hal.device, &hal.queue, &mut enc, &view, (W as f32, H as f32), wgpu::LoadOp::Load);
        hal.queue.submit(std::iter::once(enc.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        // (3) over: sprites + glifos AA en scratch transparente → composite.
        let scratch = make_tex(&hal.device);
        let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());
        let mut over = vello::Scene::new();
        bench::over_layer(&plan, &mut over, &mut ts, rect);
        renderer
            .render_to_view(&hal, &over, &scratch_view, W, H, Color::TRANSPARENT)
            .expect("render over");
        let mut enc2 = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ab-over"),
        });
        overlay.composite(&hal.device, &mut enc2, &view, &scratch_view);
        hal.queue.submit(std::iter::once(enc2.finish()));
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        write_png(&hal, &inter, "/tmp/dom_nuevo.png");
    }

    eprintln!("escrito /tmp/dom_viejo.png y /tmp/dom_nuevo.png ({W}x{H})");
}

fn make_tex(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ab-tex"),
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
