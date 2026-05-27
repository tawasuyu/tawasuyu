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
use dominium_core::{
    ActionPolicy, BehaviorHack, Concepto, Conceptos, Epoch, LayerMods, PsiMetrics, SimParams,
    Trigger, World, WorldStats,
};
use dominium_physics::tick;

#[derive(Parser, Debug)]
#[command(version, about = "Headless runner for the dominium simulator")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Modo interactivo: arranca un mundo y acepta comandos línea por línea.
    /// Comandos:
    ///   step [N]                — avanza N ticks (default 1)
    ///   stats                    — imprime poblacion/materia/oro/energia
    ///   list                     — lista los Conceptos activos
    ///   add ID X Y R [HACK]     — agrega un Concepto en (x,y) con radius
    ///   del N                    — borra el Concepto con índice N
    ///   load PATH                — carga un pack JSON
    ///   save PATH                — guarda el pack actual
    ///   csv PATH                 — abre archivo CSV para los próximos step
    ///   quit                     — sale
    Repl {
        #[arg(long, default_value_t = 0xD0_31_31_07_u64)]
        seed: u64,
        #[arg(long, default_value_t = 40)]
        grid: usize,
        #[arg(long, default_value_t = 50)]
        lemmings: usize,
        #[arg(long)]
        conceptos: Option<PathBuf>,
    },
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
        /// Período del ciclo estacional en ticks. `0` = sin estaciones.
        #[arg(long, default_value_t = 0)]
        season_period: u32,
        /// Amplitud del ciclo estacional ∈ [0, 1]. `0` = sin estaciones.
        #[arg(long, default_value_t = 0.0)]
        season_amplitude: f32,
        /// Intensidad de la modulación de efectos por `vector_psi` (Fase A).
        /// `0.0` (default) = comportamiento histórico bit-exacto. Rango útil
        /// 0.0..1.0; valores mayores amplifican la heterogeneidad por psi.
        #[arg(long, default_value_t = 0.0)]
        psi_modulation: f32,
        /// Política de elección de acción. `fixed` (default) = la acción se
        /// hereda y nunca se reelige. `psi-argmax` = cada
        /// `--policy-period` ticks los lemmings reeligen su acción como
        /// `argmax(action_weights · psi)`.
        #[arg(long, value_parser = parse_action_policy, default_value = "fixed")]
        action_policy: ActionPolicy,
        /// Período de reelección para `--action-policy psi-argmax`. `0`
        /// deshabilita la reelección incluso con la política activa.
        #[arg(long, default_value_t = 0)]
        policy_period: u32,
    },
}

fn parse_action_policy(s: &str) -> Result<ActionPolicy, String> {
    match s.to_ascii_lowercase().as_str() {
        "fixed" => Ok(ActionPolicy::Fixed),
        "psi-argmax" | "psiargmax" | "argmax" => Ok(ActionPolicy::PsiArgmax),
        other => Err(format!(
            "policy desconocida `{other}`; usá `fixed` o `psi-argmax`"
        )),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            seed,
            ticks,
            grid,
            lemmings,
            conceptos,
            csv,
            season_period,
            season_amplitude,
            psi_modulation,
            action_policy,
            policy_period,
        } => run_sim(
            seed,
            ticks,
            grid,
            lemmings,
            conceptos.as_deref(),
            csv.as_deref(),
            season_period,
            season_amplitude,
            psi_modulation,
            action_policy,
            policy_period,
        ),
        Cmd::Repl { seed, grid, lemmings, conceptos } => {
            repl(seed, grid, lemmings, conceptos.as_deref())
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
    season_period: u32,
    season_amplitude: f32,
    psi_modulation: f32,
    action_policy: ActionPolicy,
    policy_period: u32,
) -> Result<()> {
    let mut world = build_world(seed, grid, lemmings);
    if let Some(path) = conceptos_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        let cs: Conceptos = serde_json::from_str(&raw)
            .with_context(|| format!("parseando {}", path.display()))?;
        world.conceptos = cs;
    }
    let mut params = SimParams::default();
    params.season_period = season_period;
    params.season_amplitude = season_amplitude;
    params.psi_effect_modulation = psi_modulation;
    params.action_policy = action_policy;
    params.policy_reeval_period = policy_period;

    let mut writer: Option<BufWriter<File>> = match csv_path {
        Some(p) => Some(BufWriter::new(
            File::create(p).with_context(|| format!("abriendo {}", p.display()))?,
        )),
        None => None,
    };
    if let Some(w) = writer.as_mut() {
        writeln!(w, "{}", CSV_HEADER)?;
    }

    let t0 = std::time::Instant::now();
    for t in 0..ticks {
        tick(&mut world, &params);
        if let Some(w) = writer.as_mut() {
            write_row(w, &world, t + 1)?;
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
    let final_stats = WorldStats::from_world(&world);
    let psi = PsiMetrics::from_world(&world);
    let tps = (ticks as f64) / dt.as_secs_f64().max(1e-9);
    println!(
        "ok · {} ticks en {:.2?} ({:.0} tps) · seed={} grid={}×{} · poblacion={} materia={:.0} oro={:.0} energia={:.0} gini_e={:.3}",
        ticks,
        dt,
        tps,
        seed,
        grid,
        grid,
        final_stats.n,
        final_stats.total_materia,
        final_stats.total_oro,
        final_stats.total_energia,
        final_stats.gini_energia,
    );
    println!(
        "    psi · polariz=[{:.3} {:.3} {:.3} {:.3}] · corr(CORR↔Extraer)={:+.3} corr(CORR↔Degradar)={:+.3} corr(ORDEN↔Intercamb.)={:+.3} corr(MIEDO↔Mover)={:+.3}",
        psi.polarization[0],
        psi.polarization[1],
        psi.polarization[2],
        psi.polarization[3],
        psi.psi_action_corr[3][1],
        psi.psi_action_corr[3][5],
        psi.psi_action_corr[0][3],
        psi.psi_action_corr[1][0],
    );
    Ok(())
}

/// Encabezado CSV: orden estable usado por `write_row` y por el header del
/// REPL. Cualquier reordenamiento debe replicarse en `write_row`.
///
/// Columnas Fase C parcial (PsiMetrics):
/// - `pol_psi{0..3}`: polarización Esteban-Ray por componente del psi.
/// - `corr_{psi}_{accion}`: correlación punto-biserial entre el componente
///   del psi y el indicador binario de la acción. Seis pares canónicos
///   alineados con la matriz `action_weights` por default — los que
///   esperamos que se enciendan cuando `ActionPolicy::PsiArgmax` funciona.
const CSV_HEADER: &str = "tick,epoca,poblacion,materia,psique,poder,oro,degradacion,energia,mean_edad,gini_e,var_psi0,var_psi1,var_psi2,var_psi3,act_mover,act_extraer,act_sync,act_trade,act_repl,act_degr,pol_psi0,pol_psi1,pol_psi2,pol_psi3,corr_corr_extraer,corr_corr_degradar,corr_orden_intercambiar,corr_orden_replicar,corr_miedo_mover,corr_curiosidad_sync";

/// Escribe una fila al CSV usando `WorldStats` + `PsiMetrics` — formato
/// estable con `:.3` para floats macro y `:.6` para correlaciones (rango
/// `[-1,1]`, queremos resolución fina).
fn write_row<W: Write>(w: &mut W, world: &World, t: u64) -> std::io::Result<()> {
    let s = WorldStats::from_world(world);
    let e = Epoch::classify(&s);
    let p = PsiMetrics::from_world(world);
    // Índices semánticos para legibilidad — coinciden con `lemmings.rs`.
    const ORDEN: usize = 0;
    const MIEDO: usize = 1;
    const CURIOSIDAD: usize = 2;
    const CORR: usize = 3;
    // Y con `world::Action::from_u8`.
    const MOVER: usize = 0;
    const EXTRAER: usize = 1;
    const SYNC: usize = 2;
    const INTERCAMBIAR: usize = 3;
    const REPLICAR: usize = 4;
    const DEGRADAR: usize = 5;
    writeln!(
        w,
        "{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
        t,
        e.label(),
        s.n,
        s.total_materia,
        s.total_psique,
        s.total_poder,
        s.total_oro,
        s.total_degradacion,
        s.total_energia,
        s.mean_edad,
        s.gini_energia,
        s.var_psi[0],
        s.var_psi[1],
        s.var_psi[2],
        s.var_psi[3],
        s.action_counts[0],
        s.action_counts[1],
        s.action_counts[2],
        s.action_counts[3],
        s.action_counts[4],
        s.action_counts[5],
        p.polarization[0],
        p.polarization[1],
        p.polarization[2],
        p.polarization[3],
        p.psi_action_corr[CORR][EXTRAER],
        p.psi_action_corr[CORR][DEGRADAR],
        p.psi_action_corr[ORDEN][INTERCAMBIAR],
        p.psi_action_corr[ORDEN][REPLICAR],
        p.psi_action_corr[MIEDO][MOVER],
        p.psi_action_corr[CURIOSIDAD][SYNC],
    )
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

/// Modo interactivo. Cada línea = un comando. Errores no fatales se
/// imprimen y el loop continúa.
fn repl(
    seed: u64,
    grid: usize,
    lemmings: usize,
    conceptos_path: Option<&std::path::Path>,
) -> Result<()> {
    let mut world = build_world(seed, grid, lemmings);
    if let Some(p) = conceptos_path {
        let raw = std::fs::read_to_string(p)
            .with_context(|| format!("leyendo {}", p.display()))?;
        world.conceptos = serde_json::from_str(&raw)
            .with_context(|| format!("parseando {}", p.display()))?;
    }
    let params = SimParams::default();
    let mut tick_count: u64 = 0;
    let mut csv_writer: Option<BufWriter<File>> = None;
    println!("dominium-cli repl · seed={seed} grid={grid}×{grid} lemmings={lemmings}");
    println!("comandos: step [N] | stats | list | add ID X Y R [HACK] | del N |");
    println!("          load PATH | save PATH | csv PATH | quit");
    println!("(HACK opcional: 'hack ACTION DURATION' fuerza acción 0..5 N ticks)");

    use std::io::{BufRead, Write as _};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    loop {
        print!("dominium[{tick_count}]> ");
        stdout.flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        match cmd {
            "quit" | "q" | "exit" => break,
            "step" => {
                let n: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                for _ in 0..n {
                    tick(&mut world, &params);
                    tick_count += 1;
                    if let Some(w) = csv_writer.as_mut() {
                        write_row(w, &world, tick_count)?;
                    }
                    if world.lemmings.is_empty() {
                        println!("colapso en tick {tick_count}");
                        break;
                    }
                }
                let s = WorldStats::from_world(&world);
                println!(
                    "tick={} pop={} materia={:.0} oro={:.0} energia={:.0} gini_e={:.3}",
                    tick_count, s.n, s.total_materia, s.total_oro, s.total_energia, s.gini_energia
                );
            }
            "stats" => {
                let s = WorldStats::from_world(&world);
                println!(
                    "tick={} epoca={} pop={} materia={:.0} oro={:.0} energia={:.0} degradacion={:.0} mean_edad={:.1} gini_e={:.3}",
                    tick_count,
                    Epoch::classify(&s).label(),
                    s.n,
                    s.total_materia,
                    s.total_oro,
                    s.total_energia,
                    s.total_degradacion,
                    s.mean_edad,
                    s.gini_energia
                );
                println!(
                    "  var_psi=[O:{:.3} M:{:.3} C:{:.3} K:{:.3}]  acciones=[mover:{} extraer:{} sync:{} trade:{} repl:{} degr:{}]",
                    s.var_psi[0],
                    s.var_psi[1],
                    s.var_psi[2],
                    s.var_psi[3],
                    s.action_counts[0],
                    s.action_counts[1],
                    s.action_counts[2],
                    s.action_counts[3],
                    s.action_counts[4],
                    s.action_counts[5],
                );
            }
            "list" => {
                if world.conceptos.is_empty() {
                    println!("(sin conceptos)");
                }
                for (i, c) in world.conceptos.items.iter().enumerate() {
                    println!(
                        "  [{i}] {:<16} pos=({:.1},{:.1}) r={:.1} mods={{m:{:+.2} p:{:+.2} P:{:+.2} o:{:+.2}}} hack={}",
                        c.id, c.pos_x, c.pos_y, c.radius,
                        c.mods.materia, c.mods.psique, c.mods.poder, c.mods.oro,
                        c.hack.is_some()
                    );
                }
            }
            "add" => match parse_add(parts) {
                Ok(c) => {
                    let i = world.conceptos.add(c);
                    println!("ok · concepto[{i}] agregado");
                }
                Err(e) => println!("error: {e}"),
            },
            "del" => {
                let Some(idx_str) = parts.next() else {
                    println!("uso: del N");
                    continue;
                };
                match idx_str.parse::<usize>() {
                    Ok(i) if i < world.conceptos.len() => {
                        world.conceptos.remove(i);
                        println!("ok · concepto[{i}] borrado");
                    }
                    Ok(i) => println!("error: índice fuera de rango ({i})"),
                    Err(e) => println!("error: {e}"),
                }
            }
            "load" => {
                let Some(path) = parts.next() else {
                    println!("uso: load PATH");
                    continue;
                };
                match std::fs::read_to_string(path)
                    .and_then(|raw| {
                        serde_json::from_str::<Conceptos>(&raw)
                            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                    })
                {
                    Ok(cs) => {
                        world.conceptos = cs;
                        println!("ok · {} conceptos cargados", world.conceptos.len());
                    }
                    Err(e) => println!("error: {e}"),
                }
            }
            "save" => {
                let Some(path) = parts.next() else {
                    println!("uso: save PATH");
                    continue;
                };
                match serde_json::to_string_pretty(&world.conceptos) {
                    Ok(json) => match std::fs::write(path, json) {
                        Ok(()) => println!("ok · {} conceptos guardados en {path}", world.conceptos.len()),
                        Err(e) => println!("error: {e}"),
                    },
                    Err(e) => println!("error: {e}"),
                }
            }
            "csv" => {
                let Some(path) = parts.next() else {
                    println!("uso: csv PATH");
                    continue;
                };
                match File::create(path) {
                    Ok(f) => {
                        let mut w = BufWriter::new(f);
                        writeln!(w, "{}", CSV_HEADER)?;
                        csv_writer = Some(w);
                        println!("ok · CSV abierto en {path}");
                    }
                    Err(e) => println!("error: {e}"),
                }
            }
            _ => println!("comando desconocido: {cmd}"),
        }
    }
    if let Some(w) = csv_writer.as_mut() {
        w.flush().ok();
    }
    Ok(())
}

/// Parsea `add ID X Y R [hack ACTION DURATION]`. Si hack está, trigger
/// queda en `Always` con esa acción y duración.
fn parse_add<'a>(mut parts: impl Iterator<Item = &'a str>) -> Result<Concepto> {
    let id = parts.next().context("falta ID")?.to_string();
    let x: f32 = parts.next().context("falta X")?.parse().context("X inválido")?;
    let y: f32 = parts.next().context("falta Y")?.parse().context("Y inválido")?;
    let r: f32 = parts.next().context("falta R")?.parse().context("R inválido")?;
    let hack = match parts.next() {
        Some("hack") => {
            let action: u8 = parts
                .next()
                .context("falta ACTION para hack")?
                .parse()
                .context("ACTION inválido")?;
            let dur: u32 = parts
                .next()
                .context("falta DURATION para hack")?
                .parse()
                .context("DURATION inválido")?;
            Some(BehaviorHack {
                trigger: Trigger::Always,
                forced_action: action,
                duration: dur,
            })
        }
        Some(other) => anyhow::bail!("token inesperado: {other}"),
        None => None,
    };
    Ok(Concepto {
        id,
        sprite_id: 0,
        pos_x: x,
        pos_y: y,
        radius: r,
        mods: LayerMods::default(),
        hack,
    })
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
    for k in 0..lemmings {
        let x = rng.next_f32() * (grid as f32 - 1.0);
        let y = rng.next_f32() * (grid as f32 - 1.0);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 40.0 + rng.next_f32() * 40.0, psi);
        // Distribución calibrada al punto fijo (ver dominium-app-llimphi):
        // 30% Extraer + 30% Trade + 20% Mover + 15% Replicar + 5% Sync.
        w.lemmings.accion[i] = match k % 20 {
            0..=5 => 1,
            6..=11 => 3,
            12..=15 => 0,
            16..=18 => 4,
            _ => 2,
        } as u8;
    }
    w
}
