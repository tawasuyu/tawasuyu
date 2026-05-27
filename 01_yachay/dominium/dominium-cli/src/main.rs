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
    apply_event, ActionPolicy, BehaviorHack, Concepto, Conceptos, Epoch, Event, LayerMods,
    PsiMetrics, SimParams, Trigger, World, WorldStats,
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
        /// Radio de contagio social (Fase B). `0.0` (default) deshabilita
        /// el contagio. Los agentes en este radio acercan su psi al
        /// promedio local cada tick.
        #[arg(long, default_value_t = 0.0)]
        social_radius: f32,
        /// Tasa de contagio social (Fase B). `0.0` (default) deshabilita.
        /// Rango útil 0.01..0.20.
        #[arg(long, default_value_t = 0.0)]
        contagion_rate: f32,
        /// Umbral de homofilia (Fase B.2). `0.0` = contagio universal.
        /// Con valor > 0, sólo influyen vecinos con distancia psi <
        /// umbral. Rango útil 0.3..1.0 para producir tribus aisladas.
        #[arg(long, default_value_t = 0.0)]
        homophily_threshold: f32,
        /// CSV de población inicial (Fase D.1). Header opcional; columnas
        /// requeridas: `psi_orden, psi_miedo, psi_curiosidad,
        /// psi_corruptibilidad`. Columnas opcionales: `x, y, energia,
        /// accion`. Si están presentes ganan sobre los valores generados
        /// por el PRNG; si faltan, se rellenan con el PRNG sembrado por
        /// `--seed`. Cuando se usa, `--lemmings` queda ignorado.
        #[arg(long)]
        from_csv: Option<PathBuf>,
        /// Timeline JSON de eventos a inyectar (Fase D.1). Lista ordenada
        /// de `{tick, kind: Shock|PsiNudge, ...}`. Antes de cada `tick()`,
        /// los eventos cuyo `tick` coincide con el reloj global se aplican
        /// en orden de aparición en el archivo.
        #[arg(long)]
        events_json: Option<PathBuf>,
    },
    /// Monte Carlo sweep (Fase D.2): barre un parámetro en `--steps`
    /// puntos × `--reps` corridas con seeds distintos, escribe un CSV
    /// donde cada fila es una corrida con su valor de parámetro, seed y
    /// métricas finales (n, Gini, polarización, correlaciones, conteos
    /// de acción). Determinista bit-exacto: dos sweeps con los mismos
    /// argumentos producen CSV idéntico.
    ///
    /// Nombres de `--param` válidos: `psi_modulation`, `contagion_rate`,
    /// `social_radius`, `homophily_threshold`, `policy_period`.
    /// Para `policy_period`, los valores se redondean al entero más
    /// cercano (es u32). Cuando se barre `policy_period > 0` conviene
    /// pasar `--action-policy psi-argmax` para que la política se
    /// active.
    Sweep {
        /// Nombre del parámetro a barrer.
        #[arg(long)]
        param: String,
        /// Valor mínimo del rango (inclusive).
        #[arg(long)]
        min: f32,
        /// Valor máximo del rango (inclusive).
        #[arg(long)]
        max: f32,
        /// Cantidad de puntos del barrido (≥ 2). `steps=2` produce sólo
        /// `min` y `max`; `steps=N` divide en `N-1` intervalos iguales.
        #[arg(long, default_value_t = 10)]
        steps: usize,
        /// Repeticiones por punto, con seeds distintos.
        #[arg(long, default_value_t = 3)]
        reps: usize,
        /// Ticks por corrida.
        #[arg(long, default_value_t = 500)]
        ticks: u64,
        /// Seed base del sweep. Cada repetición usa `seed_base + rep`.
        #[arg(long, default_value_t = 0xD0_31_31_07_u64)]
        seed_base: u64,
        #[arg(long, default_value_t = 40)]
        grid: usize,
        #[arg(long, default_value_t = 100)]
        lemmings: usize,
        /// Pack de Conceptos a aplicar en cada corrida.
        #[arg(long)]
        conceptos: Option<PathBuf>,
        /// Población inicial desde CSV (todos los reps comparten la
        /// misma; el seed sólo modula los valores faltantes del CSV).
        /// Cuando se pasa, `--lemmings` se ignora.
        #[arg(long)]
        from_csv: Option<PathBuf>,
        /// CSV de salida (obligatorio).
        #[arg(long)]
        csv: PathBuf,
        /// Política de acción base (se mantiene fija durante el sweep).
        #[arg(long, value_parser = parse_action_policy, default_value = "fixed")]
        action_policy: ActionPolicy,
        /// Valores baseline de parámetros NO barridos. Si barrés
        /// `psi_modulation`, el resto se mantiene en estos valores.
        #[arg(long, default_value_t = 0.0)]
        psi_modulation: f32,
        #[arg(long, default_value_t = 0)]
        policy_period: u32,
        #[arg(long, default_value_t = 0.0)]
        social_radius: f32,
        #[arg(long, default_value_t = 0.0)]
        contagion_rate: f32,
        #[arg(long, default_value_t = 0.0)]
        homophily_threshold: f32,
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
            social_radius,
            contagion_rate,
            homophily_threshold,
            from_csv,
            events_json,
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
            social_radius,
            contagion_rate,
            homophily_threshold,
            from_csv.as_deref(),
            events_json.as_deref(),
        ),
        Cmd::Repl { seed, grid, lemmings, conceptos } => {
            repl(seed, grid, lemmings, conceptos.as_deref())
        }
        Cmd::Sweep {
            param,
            min,
            max,
            steps,
            reps,
            ticks,
            seed_base,
            grid,
            lemmings,
            conceptos,
            from_csv,
            csv,
            action_policy,
            psi_modulation,
            policy_period,
            social_radius,
            contagion_rate,
            homophily_threshold,
        } => run_sweep(SweepArgs {
            param,
            min,
            max,
            steps,
            reps,
            ticks,
            seed_base,
            grid,
            lemmings,
            conceptos_path: conceptos.as_deref(),
            from_csv: from_csv.as_deref(),
            csv_out: csv.as_path(),
            action_policy,
            base_psi_modulation: psi_modulation,
            base_policy_period: policy_period,
            base_social_radius: social_radius,
            base_contagion_rate: contagion_rate,
            base_homophily_threshold: homophily_threshold,
        }),
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
    social_radius: f32,
    contagion_rate: f32,
    homophily_threshold: f32,
    from_csv: Option<&std::path::Path>,
    events_json: Option<&std::path::Path>,
) -> Result<()> {
    let mut world = build_world(seed, grid, lemmings);
    if let Some(path) = from_csv {
        let n = seed_population_from_csv(&mut world, path, seed)
            .with_context(|| format!("leyendo CSV de población {}", path.display()))?;
        eprintln!("dominium-cli · población cargada desde CSV: {n} agentes");
    }
    let events: Vec<Event> = if let Some(path) = events_json {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        let evs: Vec<Event> = serde_json::from_str(&raw)
            .with_context(|| format!("parseando timeline {}", path.display()))?;
        eprintln!("dominium-cli · timeline cargada: {} eventos", evs.len());
        evs
    } else {
        Vec::new()
    };
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
    params.social_radius = social_radius;
    params.contagion_rate = contagion_rate;
    params.homophily_threshold = homophily_threshold;

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
        // Aplica eventos cuyo tick coincide con el reloj actual ANTES del
        // tick() — el shock entra en juego en este paso (la difusión lo
        // propaga). Orden lineal en `events` para determinismo: ante dos
        // eventos en el mismo tick, se aplican en orden de aparición.
        let now = world.tick_count;
        for ev in &events {
            if ev.tick == now {
                apply_event(&mut world, &ev.kind);
            }
        }
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
    println!(
        "    psi · moran_i=[{:+.3} {:+.3} {:+.3} {:+.3}]  (autocorrelación espacial, +1=segregación, 0=azar, -1=ajedrez)",
        psi.moran_i[0], psi.moran_i[1], psi.moran_i[2], psi.moran_i[3],
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
const CSV_HEADER: &str = "tick,epoca,poblacion,materia,psique,poder,oro,degradacion,energia,mean_edad,gini_e,var_psi0,var_psi1,var_psi2,var_psi3,act_mover,act_extraer,act_sync,act_trade,act_repl,act_degr,pol_psi0,pol_psi1,pol_psi2,pol_psi3,corr_corr_extraer,corr_corr_degradar,corr_orden_intercambiar,corr_orden_replicar,corr_miedo_mover,corr_curiosidad_sync,moran_psi0,moran_psi1,moran_psi2,moran_psi3";

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
        "{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
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
        p.moran_i[0],
        p.moran_i[1],
        p.moran_i[2],
        p.moran_i[3],
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
        persuasion: None,
    })
}

/// Carga una población inicial desde CSV (Fase D.1). Reemplaza COMPLETAMENTE
/// la población actual del `world` (la grilla se preserva — el sustrato
/// sigue siendo el sembrado por el PRNG). Devuelve la cantidad de agentes
/// cargados.
///
/// Header opcional. Columnas reconocidas:
/// - `psi_orden, psi_miedo, psi_curiosidad, psi_corruptibilidad`
///   (requeridas si hay header; si no, las primeras 4 columnas se asumen
///   en este orden).
/// - `x, y` (opcionales): posición; si faltan, se generan con el PRNG.
/// - `energia` (opcional): energía inicial; default 40 + rng·40.
/// - `accion` (opcional): byte de acción (0..=5); default reusa el bucket
///   módulo k del `build_world`.
fn seed_population_from_csv(world: &mut World, path: &std::path::Path, seed: u64) -> Result<usize> {
    let raw = std::fs::read_to_string(path).context("leyendo CSV")?;
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let first = lines.next().context("CSV vacío")?;
    // Detecta header: si tiene letras alfabéticas (no sólo dígitos/puntos/comas/menos)
    // asume header. Si no, primera fila es ya un agente.
    let has_header = first
        .chars()
        .any(|c| c.is_alphabetic() && c != 'e' && c != 'E');
    // Mapeo de columna → posición. Sin header, asumimos PSI_ORDEN,
    // PSI_MIEDO, PSI_CURIOSIDAD, PSI_CORRUPTIBILIDAD en columnas 0..3.
    let mut col_psi_o = Some(0usize);
    let mut col_psi_m = Some(1usize);
    let mut col_psi_c = Some(2usize);
    let mut col_psi_k = Some(3usize);
    let mut col_x: Option<usize> = None;
    let mut col_y: Option<usize> = None;
    let mut col_energia: Option<usize> = None;
    let mut col_accion: Option<usize> = None;
    if has_header {
        col_psi_o = None;
        col_psi_m = None;
        col_psi_c = None;
        col_psi_k = None;
        for (i, name) in first.split(',').map(str::trim).enumerate() {
            match name {
                "psi_orden" => col_psi_o = Some(i),
                "psi_miedo" => col_psi_m = Some(i),
                "psi_curiosidad" => col_psi_c = Some(i),
                "psi_corruptibilidad" => col_psi_k = Some(i),
                "x" => col_x = Some(i),
                "y" => col_y = Some(i),
                "energia" => col_energia = Some(i),
                "accion" => col_accion = Some(i),
                _ => {} // columna desconocida — la ignoramos
            }
        }
    }
    let col_psi_o = col_psi_o.context("falta columna psi_orden")?;
    let col_psi_m = col_psi_m.context("falta columna psi_miedo")?;
    let col_psi_c = col_psi_c.context("falta columna psi_curiosidad")?;
    let col_psi_k = col_psi_k.context("falta columna psi_corruptibilidad")?;
    // Reemplazo total de población. La grilla se preserva.
    world.lemmings = dominium_core::Lemmings::new();
    // PRNG sembrado por --seed: alimenta x/y/energia faltantes. Determinista.
    let mut rng = Lcg::new(seed);
    let mut rows_iter: Box<dyn Iterator<Item = &str>> = if has_header {
        Box::new(lines)
    } else {
        Box::new(std::iter::once(first).chain(lines))
    };
    let max = (world.grid.width.max(world.grid.height) as f32) - 1.0;
    let mut k = 0usize;
    while let Some(line) = rows_iter.next() {
        let fields: Vec<&str> = line.split(',').map(str::trim).collect();
        let get = |c: usize| -> Result<f32> {
            fields
                .get(c)
                .with_context(|| format!("fila {k} faltan columnas (idx {c})"))?
                .parse::<f32>()
                .with_context(|| format!("fila {k} col {c} no parsea como f32"))
        };
        let psi = [
            get(col_psi_o)?,
            get(col_psi_m)?,
            get(col_psi_c)?,
            get(col_psi_k)?,
        ];
        let x = match col_x {
            Some(c) => get(c)?.clamp(0.0, max),
            None => rng.next_f32() * max,
        };
        let y = match col_y {
            Some(c) => get(c)?.clamp(0.0, max),
            None => rng.next_f32() * max,
        };
        let energia = match col_energia {
            Some(c) => get(c)?.max(0.0),
            None => 40.0 + rng.next_f32() * 40.0,
        };
        let i = world.lemmings.spawn(x, y, energia, psi);
        let accion = match col_accion {
            Some(c) => {
                let raw = fields
                    .get(c)
                    .with_context(|| format!("fila {k} falta accion (idx {c})"))?
                    .trim();
                raw.parse::<u8>()
                    .with_context(|| format!("fila {k} accion no parsea como u8"))?
                    .min(5)
            }
            None => match k % 20 {
                0..=5 => 1,
                6..=11 => 3,
                12..=15 => 0,
                16..=18 => 4,
                _ => 2,
            },
        };
        world.lemmings.accion[i] = accion;
        k += 1;
    }
    Ok(k)
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

// ---------------------------------------------------------------------
// D.2 — Monte Carlo sweep
// ---------------------------------------------------------------------

/// Métricas finales de una corrida — todo lo que el sweep escribe en CSV.
struct SweepRowMetrics {
    n: usize,
    gini_e: f32,
    mean_edad: f32,
    polariz: [f32; 4],
    psi_action_corr: [[f32; 6]; 4],
    moran_i: [f32; 4],
    action_counts: [u32; 6],
}

impl SweepRowMetrics {
    fn from_world(world: &World) -> Self {
        let s = WorldStats::from_world(world);
        let p = PsiMetrics::from_world(world);
        Self {
            n: s.n,
            gini_e: s.gini_energia,
            mean_edad: s.mean_edad,
            polariz: p.polarization,
            psi_action_corr: p.psi_action_corr,
            moran_i: p.moran_i,
            action_counts: s.action_counts,
        }
    }
}

/// Corre N ticks sobre el `world` con los `params` dados, devolviendo las
/// métricas finales. Pura — sin I/O — para que el sweep pueda
/// paralelizar trivialmente si más adelante hace falta.
fn simulate_one(mut world: World, params: &SimParams, ticks: u64) -> SweepRowMetrics {
    for _ in 0..ticks {
        tick(&mut world, params);
        if world.lemmings.is_empty() {
            break;
        }
    }
    SweepRowMetrics::from_world(&world)
}

/// Parámetros del sweep — los empaquetamos para no pasar 15 args sueltos
/// al `run_sweep`.
struct SweepArgs<'a> {
    param: String,
    min: f32,
    max: f32,
    steps: usize,
    reps: usize,
    ticks: u64,
    seed_base: u64,
    grid: usize,
    lemmings: usize,
    conceptos_path: Option<&'a std::path::Path>,
    from_csv: Option<&'a std::path::Path>,
    csv_out: &'a std::path::Path,
    action_policy: ActionPolicy,
    base_psi_modulation: f32,
    base_policy_period: u32,
    base_social_radius: f32,
    base_contagion_rate: f32,
    base_homophily_threshold: f32,
}

fn run_sweep(a: SweepArgs) -> Result<()> {
    if a.steps < 2 {
        anyhow::bail!("--steps debe ser ≥ 2 (recibí {})", a.steps);
    }
    // Carga única del pack de Conceptos — todos los reps usan la misma
    // lista. La determinismo se mantiene porque la lista no se permuta.
    let conceptos = if let Some(path) = a.conceptos_path {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("leyendo {}", path.display()))?;
        Some(serde_json::from_str::<Conceptos>(&raw)
            .with_context(|| format!("parseando {}", path.display()))?)
    } else {
        None
    };
    let mut writer = BufWriter::new(
        File::create(a.csv_out)
            .with_context(|| format!("abriendo CSV salida {}", a.csv_out.display()))?,
    );
    writeln!(
        writer,
        "param_name,param_value,seed,rep,n,gini_e,mean_edad,\
        pol_psi0,pol_psi1,pol_psi2,pol_psi3,\
        corr_corr_extraer,corr_corr_degradar,corr_orden_intercambiar,corr_orden_replicar,corr_miedo_mover,corr_curiosidad_sync,\
        moran_psi0,moran_psi1,moran_psi2,moran_psi3,\
        act_mover,act_extraer,act_sync,act_trade,act_repl,act_degr"
    )?;
    let t0 = std::time::Instant::now();
    let total_runs = a.steps * a.reps;
    let mut completed = 0usize;
    for step in 0..a.steps {
        // Linspace inclusivo entre min y max.
        let value = if a.steps == 1 {
            a.min
        } else {
            a.min + (a.max - a.min) * (step as f32 / (a.steps as f32 - 1.0))
        };
        for rep in 0..a.reps {
            let seed = a.seed_base.wrapping_add(rep as u64);
            // Construir el mundo: PRNG sembrado por `seed`; si hay CSV de
            // población, sobrescribe los agentes.
            let mut world = build_world(seed, a.grid, a.lemmings);
            if let Some(path) = a.from_csv {
                seed_population_from_csv(&mut world, path, seed)
                    .with_context(|| format!("CSV pop {}", path.display()))?;
            }
            if let Some(cs) = &conceptos {
                world.conceptos = cs.clone();
            }
            // Aplicar el valor al parámetro elegido sobre la baseline.
            let mut params = SimParams::default();
            params.action_policy = a.action_policy;
            params.psi_effect_modulation = a.base_psi_modulation;
            params.policy_reeval_period = a.base_policy_period;
            params.social_radius = a.base_social_radius;
            params.contagion_rate = a.base_contagion_rate;
            params.homophily_threshold = a.base_homophily_threshold;
            apply_param_override(&mut params, &a.param, value)
                .with_context(|| format!("--param {}", a.param))?;
            let m = simulate_one(world, &params, a.ticks);
            const ORDEN: usize = 0;
            const MIEDO: usize = 1;
            const CURIOSIDAD: usize = 2;
            const CORR: usize = 3;
            const MOVER: usize = 0;
            const EXTRAER: usize = 1;
            const SYNC: usize = 2;
            const INTERCAMBIAR: usize = 3;
            const REPLICAR: usize = 4;
            const DEGRADAR: usize = 5;
            writeln!(
                writer,
                "{},{:.6},{},{},{},{:.6},{:.3},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{},{},{},{},{}",
                a.param,
                value,
                seed,
                rep,
                m.n,
                m.gini_e,
                m.mean_edad,
                m.polariz[0],
                m.polariz[1],
                m.polariz[2],
                m.polariz[3],
                m.psi_action_corr[CORR][EXTRAER],
                m.psi_action_corr[CORR][DEGRADAR],
                m.psi_action_corr[ORDEN][INTERCAMBIAR],
                m.psi_action_corr[ORDEN][REPLICAR],
                m.psi_action_corr[MIEDO][MOVER],
                m.psi_action_corr[CURIOSIDAD][SYNC],
                m.moran_i[0],
                m.moran_i[1],
                m.moran_i[2],
                m.moran_i[3],
                m.action_counts[0],
                m.action_counts[1],
                m.action_counts[2],
                m.action_counts[3],
                m.action_counts[4],
                m.action_counts[5],
            )?;
            completed += 1;
        }
        eprintln!(
            "sweep · step {}/{}: param={} value={:.4} · ({} corridas)",
            step + 1,
            a.steps,
            a.param,
            value,
            a.reps,
        );
    }
    writer.flush()?;
    let dt = t0.elapsed();
    println!(
        "ok · sweep `{}` [{:.3}..{:.3}] · {} steps × {} reps = {} corridas en {:.2?}",
        a.param,
        a.min,
        a.max,
        a.steps,
        a.reps,
        total_runs,
        dt,
    );
    let _ = completed;
    Ok(())
}

/// Modifica el `SimParams` para el parámetro indicado por nombre.
fn apply_param_override(params: &mut SimParams, name: &str, value: f32) -> Result<()> {
    match name {
        "psi_modulation" => params.psi_effect_modulation = value,
        "contagion_rate" => params.contagion_rate = value,
        "social_radius" => params.social_radius = value,
        "homophily_threshold" => params.homophily_threshold = value,
        // policy_period es u32 — redondeamos al entero más cercano y
        // clampeamos a 0 para que valores negativos no overflowen.
        "policy_period" => {
            params.policy_reeval_period = value.max(0.0).round() as u32;
        }
        other => anyhow::bail!(
            "param desconocido `{other}`; opciones: psi_modulation, contagion_rate, \
             social_radius, homophily_threshold, policy_period"
        ),
    }
    Ok(())
}
