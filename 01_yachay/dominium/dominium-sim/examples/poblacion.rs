//! Benchmark de población de dominium — evidencia dura del fix de la
//! explosión demográfica + cuelgue O(N²).
//!
//! Reproduce la **siembra y los params EXACTOS de la app**
//! (`dominium-app-llimphi/src/main.rs` y `consts.rs`): grilla 240², 2500
//! lemmings, pack de Conceptos default embebido, y la sintonía termodinámica
//! de `init()`. Corre ≥3000 ticks logueando cada 100 ticks población, materia
//! total y tiempo de tick (ms).
//!
//! Dos regímenes en una sola corrida:
//!
//!   ANTES  — frenos de población desactivados (`max_population = 0`,
//!            `density_block = 0`): la réplica mira sólo energía individual.
//!            Es el motor que explota. Para no congelar la máquina, esta
//!            corrida se ABORTA si la población cruza `DANGER` (la divergencia
//!            ya quedó demostrada por los tiempos de tick crecientes).
//!
//!   DESPUÉS — frenos activos (densidad-dependencia + tope duro). La
//!            población debe asentarse en un `N*` acotado y el tick-ms
//!            mantenerse plano.
//!
//! Correr:
//!   cargo run -p dominium-sim --example poblacion --release

use std::time::Instant;

use dominium_core::{Conceptos, SimParams, TradeTarget, World};
use dominium_physics::tick;

// ── Siembra exacta de la app ──────────────────────────────────────────────
const GRID: usize = 240; // dominium-app-llimphi/src/consts.rs
const LEMMINGS: usize = 2500; // idem
const RNG_SEED: u64 = 0xD0_31_31_07; // dominium-app-llimphi/src/main.rs

// Pack de Conceptos default de la app, embebido para reproducir la siembra
// idéntica (los Conceptos pintan campos iniciales / emiten por tick).
const DEFAULT_PACK: &str =
    include_str!("../../dominium-app-llimphi/conceptos.default.json");

fn default_conceptos() -> Conceptos {
    serde_json::from_str::<Conceptos>(DEFAULT_PACK).unwrap_or_default()
}

fn seed_world() -> World {
    dominium_core::worldgen::seed(RNG_SEED, GRID, LEMMINGS, default_conceptos())
}

/// Params de la app (`dominium-app-llimphi/src/main.rs` init), sin frenos.
fn params_app_base() -> SimParams {
    SimParams {
        diffusion_rate: 0.02,
        entropy_rate: 0.004,
        regrowth_rate: 0.004,
        carrying_capacity: 40.0,
        metabolic_cost: 0.05,
        replicate_threshold: 28.0,
        child_energy_frac: 0.45,
        abundance_threshold: 50.0,
        trade_target: TradeTarget::Poorest,
        ..SimParams::default()
    }
}

fn materia_total(w: &World) -> f64 {
    w.grid.materia.iter().map(|&v| v as f64).sum()
}

/// Máximo de cada capa de la grilla — la evidencia del fix del "edificio
/// cáncer". Sin `field_saturation` un Concepto emisor bombea su capa a +∞
/// y este máximo crece monótono; con cap, se estabiliza en el techo.
fn grid_max(w: &World) -> [f32; 5] {
    let m = |l: &[f32]| l.iter().copied().fold(0.0f32, f32::max);
    [
        m(&w.grid.materia),
        m(&w.grid.psique),
        m(&w.grid.poder),
        m(&w.grid.oro),
        m(&w.grid.degradacion),
    ]
}

/// Corre `max_ticks` y loguea cada `every`. Si `danger > 0` y la población lo
/// cruza, aborta (protege contra el cuelgue del régimen "antes").
fn run_bench(name: &str, mut w: World, p: &SimParams, max_ticks: u64, every: u64, danger: usize) {
    println!("\n=== {name} ===");
    println!(
        "{:>6} | {:>8} | {:>12} | {:>8} | maxGrid[mat psi pod oro degr]",
        "tick", "pob", "materia", "tick_ms"
    );
    println!("{}", "-".repeat(78));
    let mut peak = w.lemmings.len();
    for t in 0..max_ticks {
        let t0 = Instant::now();
        tick(&mut w, p);
        let dt_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let n = w.lemmings.len();
        peak = peak.max(n);
        if t % every == 0 || t == max_ticks - 1 {
            let g = grid_max(&w);
            println!(
                "{:>6} | {:>8} | {:>12.0} | {:>8.3} | [{:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7.1}]",
                t, n, materia_total(&w), dt_ms, g[0], g[1], g[2], g[3], g[4]
            );
        }
        if danger > 0 && n >= danger {
            println!(
                "  ⚠ ABORTADO en tick {t}: población {n} ≥ {danger} (régimen divergente — \
                 el O(N²) ya está congelando el tick; ver el tick_ms creciente arriba)"
            );
            break;
        }
    }
    println!("  pico de población: {peak}");
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let only = argv.get(1).map(|s| s.as_str());
    let max_ticks: u64 = argv
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3000);

    // Modo dedicado al BUG 1 ("edificio cáncer"): población acotada por el
    // tope duro de la app, y `field_saturation` toggleable por env (SAT, 0 =
    // sin cap). Aísla la divergencia de las CAPAS de la grilla del ruido
    // demográfico. Correr:
    //   SAT=0   cargo run -p dominium-sim --example poblacion --release -- field
    //   SAT=150 cargo run -p dominium-sim --example poblacion --release -- field
    if only == Some("field") {
        let mut p = params_app_base();
        p.max_population = 6000; // como la app, para que el max-grid no sea ruido de pob
        p.field_saturation = std::env::var("SAT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(150.0);
        eprintln!("  field_saturation = {}", p.field_saturation);
        let label = if p.field_saturation <= 0.0 {
            "BUG 1 — SIN cap (campos divergen)"
        } else {
            "BUG 1 — CON cap (campos estables)"
        };
        run_bench(label, seed_world(), &p, max_ticks, 100, 0);
        return;
    }

    if only != Some("after") {
        // ANTES: frenos off → motor que explota. Tope DANGER para no colgar.
        run_bench("ANTES (frenos off — explosión)", seed_world(), &params_app_base(), max_ticks, 100, 60_000);
    }
    if only != Some("before") {
        // DESPUÉS: densidad-dependencia + tope duro (lo que la app va a usar).
        let mut p = params_app_base();
        // Permite override de tuning por env para barrer rápido.
        let getf = |k: &str, d: f32| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        let getu = |k: &str, d: u32| std::env::var(k).ok().and_then(|s| s.parse().ok()).unwrap_or(d);
        p.regrowth_rate = getf("REGROW", p.regrowth_rate);
        p.carrying_capacity = getf("CAP", p.carrying_capacity);
        p.metabolic_cost = getf("META", p.metabolic_cost);
        p.density_block = getu("DBLOCK", 12);
        p.density_cap = getu("DCAP", 14);
        p.max_population = getu("MAXPOP", 30_000);
        eprintln!(
            "  tuning: regrow={} cap={} meta={} dblock={} dcap={} maxpop={}",
            p.regrowth_rate, p.carrying_capacity, p.metabolic_cost,
            p.density_block, p.density_cap, p.max_population
        );
        run_bench("DESPUÉS (densidad-dependencia + tope)", seed_world(), &p, max_ticks, 100, 0);
    }
}
