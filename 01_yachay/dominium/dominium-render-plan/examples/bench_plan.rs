//! `bench_plan` — mide el costo de `build_plan` a distintos tamaños de
//! grilla, con la población/conceptos que siembra el worldgen real.
//!
//! No mide vello (eso necesita GPU/headless aparte): mide el costo CPU de
//! reconstruir el `RenderPlan` (lo que la app hacía CADA frame) y cuenta
//! las primitivas emitidas — el input directo del rasterizador.
//!
//! Uso:
//! ```bash
//! cargo run -p dominium-render-plan --example bench_plan --release
//! ```

use dominium_core::conceptos::Conceptos;
use dominium_core::worldgen;
use dominium_iso::{IsoProjector, ZWeights};
use dominium_render_plan::{build_plan, build_plan_culled, PlanConfig, RenderPlan, Viewport};
use std::time::Instant;

fn sim_iso() -> IsoProjector {
    // Misma cámara que la app real (scale 3.0, z_factor 0.55).
    IsoProjector::new(3.0, 0.55)
}

fn sim_weights() -> ZWeights {
    ZWeights { materia: 0.02, psique: -0.075, poder: 0.40, oro: 0.0, degradacion: 1.30 }
}

fn sim_cfg() -> PlanConfig {
    PlanConfig { tile: 3.0, lemming_size: 2.6, ..PlanConfig::default() }
}

/// Cronometra `f` durante `n` frames y devuelve ms/frame, con un sink para
/// que el optimizador no elimine el trabajo.
fn timed(n: u32, mut f: impl FnMut() -> RenderPlan) -> (f64, RenderPlan) {
    let mut last = f(); // warmup
    let t0 = Instant::now();
    let mut sink = 0usize;
    for _ in 0..n {
        last = f();
        sink = sink.wrapping_add(last.polygons.len());
    }
    let ms = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    std::hint::black_box(sink);
    (ms, last)
}

fn report(label: &str, ms: f64, plan: &RenderPlan) {
    let prims = plan.quads.len() + plan.polygons.len() + plan.sprites.len() + plan.glyphs.len();
    println!(
        "  {label:<22} quads {:>7}  polygons {:>8}  | TOTAL prims {:>8}  | {ms:>9.3} ms/frame",
        plan.quads.len(),
        plan.polygons.len(),
        prims,
    );
}

/// Bench FULL (todas las celdas), iso/weights/cfg de la app.
fn bench_full(grid: usize, lemmings: usize, n: u32) {
    let world = worldgen::seed(0xD0_31_31_07, grid, lemmings, Conceptos::new());
    let (iso, w, cfg) = (sim_iso(), sim_weights(), sim_cfg());
    let (ms, plan) = timed(n, || build_plan(&world, &iso, &w, &cfg));
    report(&format!("grid {grid}² FULL"), ms, &plan);
}

/// Bench CULLED: cámara con zoom mostrando ~1/`zoom_frac_denom` del mundo,
/// viewport centrado en el centro del plan (origen iso, donde cae el centro
/// del mapa cuadrado). El viewport se dimensiona como una fracción del bbox
/// del plan full para simular "acercarse" a una región.
fn bench_culled(grid: usize, lemmings: usize, n: u32, vp_px: f32) {
    let world = worldgen::seed(0xD0_31_31_07, grid, lemmings, Conceptos::new());
    let (iso, w, cfg) = (sim_iso(), sim_weights(), sim_cfg());
    // Centro del plan en coords de plan: el centro del mapa cuadrado proyecta
    // a x≈0 (la diagonal del rombo) y a una y en la mitad de la banda iso.
    let mid = grid as f32 * 0.5;
    let (cx, cy) = iso.project(mid, mid, 0.0);
    let vp = Viewport::centered(cx, cy, vp_px, vp_px, vp_px * 0.05);
    let (ms, plan) = timed(n, || build_plan_culled(&world, &iso, &w, &cfg, vp));
    report(&format!("grid {grid}² CULLED {vp_px:.0}px"), ms, &plan);
}

fn main() {
    println!("=== dominium build_plan bench (CPU only, sin rasterizado) ===\n");
    println!("-- referencia --");
    bench_full(50, 100, 60);
    bench_full(240, 2500, 60);
    println!("\n-- 1000×1000 (1M celdas): FULL vs CULLED a distintos zooms --");
    // 1M celdas: el full es el techo de la arquitectura plan-por-frame.
    bench_full(1000, 43_000, 20);
    // Cámara con zoom: 600px de canvas ≈ una región chica del mundo (~1/20).
    bench_culled(1000, 43_000, 60, 600.0);
    bench_culled(1000, 43_000, 60, 1200.0);
}
