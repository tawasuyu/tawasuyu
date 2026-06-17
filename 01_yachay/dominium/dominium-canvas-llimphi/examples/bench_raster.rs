//! `bench_raster` — mide el COSTO DE RASTERIZADO de un frame de dominium en
//! sim ACTIVA (plan reconstruido cada tick), VIEJO (vello puro) vs NUEVO
//! (GPU tris + over-layer vello), a grid 240.
//!
//! `bench_plan` mide sólo el build CPU; éste mide lo que Tier 1 ataca: las
//! ~115 k primitivas → rasterizado. Para aislar el rasterizado del present
//! medimos hasta `device.poll(wait)` tras el submit (la GPU terminó el
//! frame). El build se mide aparte para poder separar build vs raster.
//!
//! ```bash
//! cargo run -p dominium-canvas-llimphi --example bench_raster --release -- [grid] [iters]
//! ```

use std::time::Instant;

use dominium_canvas_llimphi::bench;
use dominium_core::conceptos::{Concepto, Conceptos, LayerMods};
use dominium_core::worldgen;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan_with_overrides, PlanConfig, RenderPlan};
use llimphi_ui::llimphi_hal::{wgpu, Hal, OverlayCompositor};
use llimphi_ui::llimphi_raster::gpu::GpuBatch;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_raster::{vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;
use llimphi_ui::PaintRect;

const W: u32 = 1600;
const H: u32 = 1000;
const FMT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const BG: Color = Color::from_rgba8(11, 13, 18, 255);

struct Sim {
    world: dominium_core::World,
    iso: IsoProjector,
    weights: ZWeights,
    cfg: PlanConfig,
}

impl Sim {
    fn new(grid: usize) -> Self {
        let lemmings = grid * grid / 23;
        let mut conceptos = Conceptos::new();
        for id in 1..=8u32 {
            conceptos.items.push(Concepto {
                id: format!("c{id}"),
                sprite_id: id,
                pos_x: grid as f32 * (0.15 + 0.08 * id as f32),
                pos_y: grid as f32 * 0.5,
                radius: 5.0,
                mods: LayerMods::default(),
                hack: None,
                persuasion: None,
            });
        }
        Sim {
            world: worldgen::seed(0xD0_31_31_07, grid, lemmings, conceptos),
            iso: IsoProjector::new(3.0, 0.55),
            weights: ZWeights { materia: 0.02, psique: -0.075, poder: 0.40, oro: 0.0, degradacion: 1.30 },
            cfg: PlanConfig { tile: 3.0, lemming_size: 2.6, ..PlanConfig::default() },
        }
    }
    fn build(&self) -> RenderPlan {
        build_plan_with_overrides(&self.world, &self.iso, &self.weights, &self.cfg, |_| self.cfg.palette.lemming)
    }
}

fn main() {
    let grid: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(240);
    let iters: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let sim = Sim::new(grid);
    let plan0 = sim.build();
    println!("=== bench_raster grid {grid} (sim ACTIVA: plan reconstruido cada frame) ===");
    println!(
        "primitivas/frame: quads {} + polygons {} + sprites {} = {} (≈ fills vello viejos)",
        plan0.quads.len(),
        plan0.polygons.len(),
        plan0.sprites.len(),
        plan0.quads.len() + plan0.polygons.len() + plan0.sprites.len()
    );

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let pipelines = bench::Pipelines::new(&hal.device, FMT);
    let overlay = OverlayCompositor::new(&hal.device);
    let inter = make_tex(&hal.device);
    let view = inter.create_view(&wgpu::TextureViewDescriptor::default());
    let scratch = make_tex(&hal.device);
    let scratch_view = scratch.create_view(&wgpu::TextureViewDescriptor::default());
    let rect = PaintRect { x: 0.0, y: 0.0, w: W as f32, h: H as f32 };
    let mut ts = Typesetter::new();

    // ── BUILD (común a ambos) ──────────────────────────────────────────
    {
        let _ = sim.build(); // warmup
        let t0 = Instant::now();
        let mut sink = 0usize;
        for _ in 0..iters {
            sink = sink.wrapping_add(sim.build().polygons.len());
        }
        std::hint::black_box(sink);
        println!("\nbuild_plan          {:>8.3} ms/frame", ms(t0, iters));
    }

    // ── VIEJO: raster vello completo (build + raster por frame) ────────
    {
        // warmup
        let p = sim.build();
        let mut sc = vello::Scene::new();
        bench::vello_full(&p, &mut sc, &mut ts, rect, (0.0, 0.0));
        renderer.render_to_view(&hal, &sc, &view, W, H, BG).unwrap();
        let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());

        let t0 = Instant::now();
        for _ in 0..iters {
            let p = sim.build();
            let mut sc = vello::Scene::new();
            bench::vello_full(&p, &mut sc, &mut ts, rect, (0.0, 0.0));
            renderer.render_to_view(&hal, &sc, &view, W, H, BG).unwrap();
            let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let total = ms(t0, iters);
        println!("\nVIEJO (vello puro)");
        println!("  build+raster total {:>7.3} ms/frame", total);
    }

    // ── NUEVO: GPU tris + over composite (build + raster por frame) ────
    {
        // warmup
        let p = sim.build();
        raster_gpu(&hal, &mut renderer, &pipelines, &overlay, &view, &scratch_view, &mut ts, &p, rect);

        let t0 = Instant::now();
        for _ in 0..iters {
            let p = sim.build();
            raster_gpu(&hal, &mut renderer, &pipelines, &overlay, &view, &scratch_view, &mut ts, &p, rect);
        }
        let total = ms(t0, iters);
        println!("\nNUEVO (GPU tris + over)");
        println!("  build+raster total {:>7.3} ms/frame", total);
        println!("  (1 draw-call de triángulos para todo el terreno+lemmings+conceptos;");
        println!("   sprites/glifos AA en 1 pasada vello over)");
    }
}

#[allow(clippy::too_many_arguments)]
fn raster_gpu(
    hal: &Hal,
    renderer: &mut Renderer,
    pipelines: &bench::Pipelines,
    overlay: &OverlayCompositor,
    view: &wgpu::TextureView,
    scratch_view: &wgpu::TextureView,
    ts: &mut Typesetter,
    plan: &RenderPlan,
    rect: PaintRect,
) {
    // base (fondo)
    let base = vello::Scene::new();
    renderer.render_to_view(hal, &base, view, W, H, BG).unwrap();
    // GPU tris
    let mut batch = GpuBatch::new(pipelines);
    bench::emit_tris(plan, rect, &mut batch, (0.0, 0.0));
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("br-gpu") });
    batch.flush(&hal.device, &hal.queue, &mut enc, view, (W as f32, H as f32), wgpu::LoadOp::Load);
    hal.queue.submit(std::iter::once(enc.finish()));
    // over
    let mut over = vello::Scene::new();
    bench::over_layer(plan, &mut over, ts, rect, (0.0, 0.0));
    renderer.render_to_view(hal, &over, scratch_view, W, H, Color::TRANSPARENT).unwrap();
    let mut enc2 = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("br-over") });
    overlay.composite(&hal.device, &mut enc2, view, scratch_view);
    hal.queue.submit(std::iter::once(enc2.finish()));
    let _ = hal.device.poll(wgpu::PollType::wait_indefinitely());
}

fn ms(t0: Instant, iters: u32) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

fn make_tex(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("br-tex"),
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
