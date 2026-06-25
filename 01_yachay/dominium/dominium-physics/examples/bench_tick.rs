//! `bench_tick` — perfila el costo del tick a escala para distinguir el cuello
//! de **población** (ya domado con índices espaciales) del de **grilla** (la
//! difusión densa, que barre todas las celdas cada tick). Headless, sin deps de
//! bench — sólo `std::time` y stats de texto (regla #8).
//!
//! `cargo run -p dominium-physics --example bench_tick --release`

use std::time::Instant;

use dominium_core::{worldgen, Conceptos, SimParams};
use dominium_physics::{diffuse_with, tick};

fn main() {
    let warmup = 5u32;
    let iters = 40u32;

    eprintln!("== difusión sola (sin agentes) — escala con la GRILLA ==");
    for &g in &[128usize, 256, 384, 512] {
        let mut world = worldgen::seed(0xB0_5EED, g, 0, Conceptos::default());
        // Sembrar campos con algo de estructura para que la difusión trabaje.
        let p = SimParams::default();
        for _ in 0..warmup {
            diffuse_with(&mut world.grid, p.diffusion_rate, p.entropy_rate);
        }
        let t0 = Instant::now();
        for _ in 0..iters {
            diffuse_with(&mut world.grid, p.diffusion_rate, p.entropy_rate);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        let cells = (g * g) as f64;
        eprintln!(
            "  grid {g:>4}² ({:>8.0} celdas): {ms:>7.3} ms/difusión · {:>5.1} ns/celda",
            cells,
            ms * 1e6 / cells
        );
    }

    eprintln!("== tick completo — grilla fija 256², población creciente ==");
    for &pop in &[500usize, 2000, 8000, 20000] {
        let mut world = worldgen::seed(0xB0_5EED, 256, pop, Conceptos::default());
        let mut p = SimParams::default();
        p.field_saturation = 150.0;
        p.max_energy = 400.0;
        p.density_block = 12;
        p.density_cap = 16;
        for _ in 0..warmup {
            tick(&mut world, &p);
        }
        let n0 = world.lemmings.len();
        let t0 = Instant::now();
        for _ in 0..iters {
            tick(&mut world, &p);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        eprintln!(
            "  pop≈{n0:>6}: {ms:>7.3} ms/tick",
        );
    }
}
