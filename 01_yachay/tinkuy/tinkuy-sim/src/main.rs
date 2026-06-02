//! `tinkuy-sim` — demo end-to-end del motor.
//!
//! Lanza N partículas en una caja cúbica de Lennard-Jones (unidades reducidas
//! ε=σ=m=1) con velocidades térmicas iniciales tipo Maxwell-Boltzmann (RNG
//! determinista vía splitmix64, ningún `rand` dep). Cada `--report-every`
//! steps imprime: step, t, KE, T, |Σp|, CID BLAKE3 (primeros 8 bytes hex).
//!
//! Sirve a tres propósitos:
//!   1. Validar el stack completo (ECS + Grid + Verlet + LJ + walls + obs).
//!   2. Demostrar la narrativa Wawa: cada snapshot es content-addressable.
//!   3. Establecer baseline de coste para futuros benchmarks (rough fps).

use std::time::Instant;

use tinkuy_core::{
    kinetic_energy, lattice_cubica, reflect_walls, temperature, total_momentum,
    velocity_verlet_step, Grid3D, IntegratorParams, Outbox, Snapshot, World,
};
use tinkuy_forces::{clear_accelerations, lennard_jones, LjParams};

// ─── CLI parsing (sin clap) ───────────────────────────────────────────────────

struct Cfg {
    n: usize,
    steps: usize,
    dt: f32,
    report_every: usize,
    seed: u64,
    temperature_init: f32,
}

impl Cfg {
    fn from_args() -> Self {
        let mut cfg = Cfg {
            n: 256,
            steps: 200,
            dt: 0.005,
            report_every: 20,
            seed: 0xC0FFEE,
            temperature_init: 0.5,
        };
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            let (key, val) = if let Some((k, v)) = a.split_once('=') {
                i += 1;
                (k.to_string(), v.to_string())
            } else {
                let v = args.get(i + 1).cloned().unwrap_or_default();
                i += 2;
                (a.clone(), v)
            };
            match key.as_str() {
                "--n"            => cfg.n = val.parse().unwrap_or(cfg.n),
                "--steps"        => cfg.steps = val.parse().unwrap_or(cfg.steps),
                "--dt"           => cfg.dt = val.parse().unwrap_or(cfg.dt),
                "--report-every" => cfg.report_every = val.parse().unwrap_or(cfg.report_every),
                "--seed"         => cfg.seed = val.parse().unwrap_or(cfg.seed),
                "--temp"         => cfg.temperature_init = val.parse().unwrap_or(cfg.temperature_init),
                "--help" | "-h"  => {
                    print_help();
                    std::process::exit(0);
                }
                _ => eprintln!("warn: argumento desconocido: {key}"),
            }
        }
        cfg
    }
}

fn print_help() {
    println!("tinkuy-sim — demo de simulación LJ con reporte BLAKE3");
    println!();
    println!("USO: tinkuy-sim [opciones]");
    println!();
    println!("OPCIONES:");
    println!("  --n N                Nº de partículas (se redondea a cube)  [256]");
    println!("  --steps S            Pasos de simulación                     [200]");
    println!("  --dt DT              Paso temporal                           [0.005]");
    println!("  --report-every K     Imprime stats cada K steps              [20]");
    println!("  --seed U64           Semilla PRNG (xorshift)                 [0xC0FFEE]");
    println!("  --temp T0            Temperatura inicial (reduced units)     [0.5]");
}

// ─── Inicialización del estado ────────────────────────────────────────────────

const SIGMA:   f32 = 1.0;
const EPSILON: f32 = 1.0;
const CUTOFF:  f32 = 2.5;
// 1.5σ: por encima del mínimo de LJ (r_min ≈ 1.122σ) y suficientemente lejos
// del cutoff (2.5σ) para que la PE inicial sea pequeña; así KE no crece de
// golpe por relajación del lattice.
const SPACING: f32 = 1.5 * SIGMA;
const KB:      f64 = 1.0; // unidades reducidas

fn init_world(cfg: &Cfg) -> (World, Grid3D, [f32; 3], [f32; 3]) {
    // Cubic lattice: ⌈N^(1/3)⌉ por eje. n_actual puede exceder cfg.n; ajustamos.
    // El setup canónico (lattice + drift CM + grilla) vive en tinkuy-core.
    let side = (cfg.n as f32).cbrt().ceil() as usize;
    lattice_cubica(side, SPACING, CUTOFF, cfg.seed, cfg.temperature_init)
}

// ─── Reporte ──────────────────────────────────────────────────────────────────

fn print_header() {
    println!(
        "{:>6} {:>10} {:>14} {:>10} {:>14} {:>20}",
        "step", "t", "KE", "T", "|p_total|", "CID[..8]"
    );
}

fn print_row(step: usize, t: f64, world: &World) {
    let ke = kinetic_energy(world);
    let tk = temperature(world, KB);
    let [px, py, pz] = total_momentum(world);
    let pmag = (px * px + py * py + pz * pz).sqrt();
    let snap = Snapshot::capture(world);
    let cid_hex: String = snap.cid[..8].iter().map(|b| format!("{:02x}", b)).collect();
    println!(
        "{:>6} {:>10.3} {:>14.6} {:>10.4} {:>14.3e} {:>20}",
        step, t, ke, tk, pmag, cid_hex
    );
}

// ─── Loop principal ───────────────────────────────────────────────────────────

fn main() {
    let cfg = Cfg::from_args();
    let (mut w, mut g, bmin, bmax) = init_world(&cfg);
    let n = w.len();

    let params = IntegratorParams {
        dt: cfg.dt, bounds_min: bmin, bounds_max: bmax,
    };
    let lj = LjParams { epsilon: EPSILON, sigma: SIGMA, cutoff: CUTOFF };
    let n_workers = rayon::current_num_threads().max(1);
    let mut outboxes: Vec<Outbox> = (0..n_workers).map(|_| Outbox::default()).collect();

    eprintln!(
        "tinkuy-sim · N={} (lattice {}³) · dt={} · steps={} · workers={} · seed={:#x}",
        n,
        (n as f32).cbrt() as u32,
        cfg.dt, cfg.steps, n_workers, cfg.seed
    );
    eprintln!(
        "dominio: [{:.2}, {:.2}]³ · grilla: {:?} celdas · cutoff: {}",
        bmin[0], bmax[0], g.dims, CUTOFF
    );
    println!();
    print_header();
    print_row(0, 0.0, &w);

    let t_start = Instant::now();
    for step in 1..=cfg.steps {
        velocity_verlet_step(&mut w, &mut g, &params, &mut outboxes, |world, grid| {
            clear_accelerations(world);
            lennard_jones(world, grid, &lj);
        });
        reflect_walls(&mut w, bmin, bmax);
        if step % cfg.report_every == 0 || step == cfg.steps {
            print_row(step, step as f64 * cfg.dt as f64, &w);
        }
    }
    let elapsed = t_start.elapsed();
    let total_steps = cfg.steps as f64;
    let particle_steps = total_steps * n as f64;
    eprintln!();
    eprintln!(
        "completado en {:.3}s · {:.0} steps/s · {:.2e} particle-steps/s",
        elapsed.as_secs_f64(),
        total_steps / elapsed.as_secs_f64(),
        particle_steps / elapsed.as_secs_f64(),
    );
}
