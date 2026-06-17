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
use dominium_render_plan::{build_plan, PlanConfig};
use std::time::Instant;

fn bench(grid: usize, lemmings: usize) {
    // Mundo realista (mismos generadores que la app).
    let world = worldgen::seed(0xD0_31_31_07, grid, lemmings, Conceptos::new());
    // Misma cámara/cfg que la app real (scale 3.0, z_factor 0.55).
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

    // Warmup.
    let plan = build_plan(&world, &iso, &weights, &cfg);
    let prims = plan.quads.len() + plan.polygons.len() + plan.sprites.len() + plan.glyphs.len();

    // N iteraciones cronometradas.
    const N: u32 = 60;
    let t0 = Instant::now();
    let mut sink = 0usize;
    for _ in 0..N {
        let p = build_plan(&world, &iso, &weights, &cfg);
        sink = sink.wrapping_add(p.polygons.len());
    }
    let dt = t0.elapsed();
    let per_frame_ms = dt.as_secs_f64() * 1000.0 / N as f64;
    std::hint::black_box(sink);

    println!(
        "grid {grid:>3}×{grid:<3}  lemmings {lemmings:>5}  | \
         quads {:>6}  polygons {:>6}  sprites {:>4}  | TOTAL prims {:>6}  | \
         build_plan {per_frame_ms:>7.3} ms/frame",
        plan.quads.len(),
        plan.polygons.len(),
        plan.sprites.len(),
        prims,
    );
}

fn main() {
    println!("=== dominium build_plan bench (CPU only, sin rasterizado) ===");
    bench(50, 100);
    bench(240, 2500);
}
