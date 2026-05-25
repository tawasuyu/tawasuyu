//! `dominium-cli` — corre la simulación sin ventana y dumpa stats.
//!
//! Útil para:
//!  - **Validar determinismo cross-platform**: corré dos veces con el
//!    mismo seed en x86 y ARM; los CSV deben ser bit-exactos.
//!  - **Experimentar con packs**: cargá un `conceptos.json` y mirá la
//!    población/materia a lo largo de N ticks sin esperar a la ventana.
//!  - **Profiling**: medir el throughput del motor (tps).
//!
//! Comandos:
//!
//! ```text
//! dominium-cli run --seed 42 --ticks 1000 --grid 40 --lemmings 50
//! dominium-cli run --conceptos pack.json --csv stats.csv
//! ```
//!
//! Cada fila del CSV: `tick,poblacion,materia_total,oro_total,energia_total,degradacion_total`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dominium_core::{Conceptos, SimParams, World};
use dominium_physics::tick;

#[derive(Parser, Debug)]
#[command(version, about = "Headless runner for the dominium simulator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Corre N ticks y opcionalmente escribe stats a CSV.
    Run {
        /// Seed del PRNG para sembrar el mundo (determinista).
        #[arg(long, default_value_t = 0xD0_31_31_07_u64)]
        seed: u64,
        /// Cantidad de ticks a correr.
        #[arg(long, default_value_t = 200)]
        ticks: u64,
        /// Lado de la grilla cuadrada.
        #[arg(long, default_value_t = 40)]
        grid: usize,
        /// Población inicial de lemmings.
        #[arg(long, default_value_t = 50)]
        lemmings: usize,
        /// Pack JSON de Conceptos a cargar. Vacío = sin Conceptos.
        #[arg(long)]
        conceptos: Option<PathBuf>,
        /// Archivo CSV destino. Vacío = imprime resumen a stdout.
        #[arg(long)]
        csv: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run { seed, ticks, grid, lemmings, conceptos, csv } => {
            run_sim(seed, ticks, grid, lemmings, conceptos.as_deref(), csv.as_deref())
        }
    }
}

fn run_sim(
    seed: u64,
    ticks: u64,
    grid: usize,
    lemmings: usize,
    conceptos_path: Option<&std::path::Path>,
    csv_path: Option<&std::path::Path>,
) -> Result<()> {
    let mut world = build_world(seed, grid, lemmings);
    if let Some(path) = conceptos_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        let cs: Conceptos = serde_json::from_str(&raw)
            .with_context(|| format!("parseando {}", path.display()))?;
        world.conceptos = cs;
    }
    let params = SimParams::default();

    let mut writer: Option<BufWriter<File>> = match csv_path {
        Some(p) => Some(BufWriter::new(
            File::create(p).with_context(|| format!("abriendo {}", p.display()))?,
        )),
        None => None,
    };
    if let Some(w) = writer.as_mut() {
        writeln!(w, "tick,poblacion,materia,oro,energia,degradacion")?;
    }

    let t0 = std::time::Instant::now();
    for t in 0..ticks {
        tick(&mut world, &params);
        if let Some(w) = writer.as_mut() {
            let row = row_of(&world, t + 1);
            writeln!(
                w,
                "{},{},{:.3},{:.3},{:.3},{:.3}",
                row.tick, row.pop, row.materia, row.oro, row.energia, row.degradacion
            )?;
        }
        if world.lemmings.is_empty() {
            eprintln!("colapso en tick {} — población vacía", t + 1);
            break;
        }
    }
    let dt = t0.elapsed();
    if let Some(w) = writer.as_mut() {
        w.flush()?;
    }
    let final_row = row_of(&world, ticks);
    let tps = (ticks as f64) / dt.as_secs_f64().max(1e-9);
    println!(
        "ok · {} ticks en {:.2?} ({:.0} tps) · seed={} grid={}×{} · poblacion={} materia={:.0} oro={:.0} energia={:.0}",
        ticks,
        dt,
        tps,
        seed,
        grid,
        grid,
        final_row.pop,
        final_row.materia,
        final_row.oro,
        final_row.energia,
    );
    Ok(())
}

struct Row {
    tick: u64,
    pop: usize,
    materia: f32,
    oro: f32,
    energia: f32,
    degradacion: f32,
}

fn row_of(w: &World, t: u64) -> Row {
    Row {
        tick: t,
        pop: w.lemmings.len(),
        materia: w.grid.materia.iter().sum(),
        oro: w.grid.oro.iter().sum(),
        energia: w.lemmings.energia.iter().sum(),
        degradacion: w.grid.degradacion.iter().sum(),
    }
}

// PRNG mínimo (mismo LCG que el app — bit-exacto).
struct Lcg(u64);
impl Lcg {
    fn new(s: u64) -> Self { Self(s) }
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn next_f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn build_world(seed: u64, grid: usize, lemmings: usize) -> World {
    let mut w = World::new(grid, grid);
    let mut rng = Lcg::new(seed);
    for cy in 0..grid {
        for cx in 0..grid {
            let idx = w.grid.idx(cx, cy);
            let m = rng.next_f32();
            w.grid.materia[idx] = m * m * 60.0;
            if rng.next_f32() > 0.92 {
                w.grid.oro[idx] = rng.next_f32() * 40.0;
            }
            w.grid.psique[idx] = rng.next_f32() * 12.0;
        }
    }
    for _ in 0..lemmings {
        let x = rng.next_f32() * (grid as f32 - 1.0);
        let y = rng.next_f32() * (grid as f32 - 1.0);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 30.0 + rng.next_f32() * 40.0, psi);
        w.lemmings.accion[i] = (rng.next_u32() % 6) as u8;
    }
    w
}
