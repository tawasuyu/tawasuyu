//! `dominium-notebook-kernel` — kernel de notebook que ejecuta
//! celdas sobre el simulador determinista de [`dominium_core`] +
//! [`dominium_physics`].
//!
//! El kernel mantiene un estado interno compartido entre celdas
//! (`Arc<Mutex<DominiumState>>`): un único [`World`] mutable + sus
//! [`SimParams`]. Cada celda muta ese estado y reporta el resultado;
//! re-ejecutar una celda upstream (vía `pluma_notebook_exec::run_from`)
//! re-aplica la cascada de mutaciones desde ese punto, exactamente como
//! Excel re-evalúa una columna cuando cambia una fórmula raíz.
//!
//! ## Lenguajes reconocidos
//!
//! | `language`        | Source                       | Efecto                                                        |
//! |-------------------|------------------------------|---------------------------------------------------------------|
//! | `dominium-world`  | `"W H"` (ej. `"32 24"`)      | Resetea el mundo a una grilla `W×H`, lemmings vacíos.        |
//! | `dominium-seed`   | `"N [SEED]"` (ej. `"200 42"`)| Siembra N lemmings con LCG determinista a partir de SEED.    |
//! | `dominium-tick`   | `"N"` o vacío                | Corre N ticks (default 1); output = stats post.              |
//! | `dominium-stats`  | (vacío)                      | Lee `WorldStats` sin tick.                                   |
//! | `dominium-param`  | `"NAME=VALUE"` por línea     | Setea uno o más campos `f32` de `SimParams`.                 |
//! | `dominium-render` | `"W H [SCALE]"` (px)         | Rasteriza grid + lemmings a PNG (`SCALE`≥1 multiplica la resolución), output `OutputPayload::Image`. |
//!
//! Cualquier otra `language` devuelve `KernelError::Runtime` con
//! mensaje claro.
//!
//! ## Por qué encaja en el DAG
//!
//! - Una celda `dominium-world "32 24"` resetea el mundo.
//! - Una celda `dominium-seed "200 42"` que depende de la primera
//!   siembra agentes.
//! - Una celda `dominium-tick "100"` que depende de la segunda corre
//!   100 ticks; su output es la tabla de `WorldStats`.
//! - Editar la primera (`"64 64"`) y llamar `run_from(world)` re-
//!   ejecuta la cadena entera en orden topológico, dejando un sistema
//!   reproducible que un investigador puede explorar sin tocar Rust.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use dominium_core::{SimParams, World, WorldStats};
use dominium_physics::tick as physics_tick;
use pluma_notebook_core::{CellOutput, OutputPayload};
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};

/// Estado vivo de un kernel dominium: el `World` (o `None` antes de
/// `dominium-world`) y los `SimParams` que las celdas mutan.
#[derive(Debug, Clone, Default)]
pub struct DominiumState {
    pub world: Option<World>,
    pub params: SimParams,
}

/// Kernel ECS dominium. El estado se comparte entre celdas vía
/// `Arc<Mutex<...>>` — los notebooks reactivos lo leen y escriben en
/// orden topológico garantizado por `pluma-notebook-exec`.
pub struct DominiumKernel {
    state: Arc<Mutex<DominiumState>>,
}

impl Default for DominiumKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl DominiumKernel {
    pub fn new() -> Self {
        Self::from_state(DominiumState::default())
    }

    pub fn from_state(state: DominiumState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Handle al estado compartido. Útil para que la UI lea el `World`
    /// actual y lo pinte (cosmos-canvas-llimphi / dominium-canvas-llimphi)
    /// sin que la celda tenga que serializarlo.
    pub fn state_handle(&self) -> Arc<Mutex<DominiumState>> {
        Arc::clone(&self.state)
    }

    /// Snapshot del estado actual — copia profunda. No bloquea por más
    /// de un Mutex lock breve. Sirve para tests y para serializar
    /// reportes.
    pub fn snapshot(&self) -> DominiumState {
        self.state.lock().expect("kernel state envenenado").clone()
    }
}

#[async_trait]
impl Kernel for DominiumKernel {
    async fn execute(
        &self,
        source: &str,
        language: &str,
    ) -> Result<KernelOutput, KernelError> {
        match language {
            "dominium-world" => exec_world(source, &self.state),
            "dominium-seed" => exec_seed(source, &self.state),
            "dominium-tick" => exec_tick(source, &self.state),
            "dominium-stats" => exec_stats(&self.state),
            "dominium-param" => exec_param(source, &self.state),
            "dominium-render" => exec_render(source, &self.state),
            other => Err(KernelError::Runtime(format!(
                "lenguaje no reconocido por el kernel dominium: '{other}' \
                 (esperaba: dominium-world | dominium-seed | dominium-tick | \
                 dominium-stats | dominium-param | dominium-render)"
            ))),
        }
    }
}

fn exec_world(
    source: &str,
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let mut it = source.split_whitespace();
    let w: usize = parse_required(it.next(), "WIDTH")?;
    let h: usize = parse_required(it.next(), "HEIGHT")?;
    if w == 0 || h == 0 {
        return Err(KernelError::Runtime(
            "WIDTH y HEIGHT deben ser > 0".into(),
        ));
    }
    let mut s = lock(state)?;
    s.world = Some(World::new(w, h));
    Ok(text_output(format!("world reseteado a {w}×{h}, lemmings=0")))
}

fn exec_seed(
    source: &str,
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let mut it = source.split_whitespace();
    let n: usize = parse_required(it.next(), "N")?;
    let seed: u64 = it
        .next()
        .map(|s| {
            s.parse::<u64>().map_err(|_| {
                KernelError::Runtime(format!("SEED debe ser un u64: '{s}'"))
            })
        })
        .transpose()?
        .unwrap_or(0xC05_0510_0000_0001u64);

    let mut s = lock(state)?;
    let world = s
        .world
        .as_mut()
        .ok_or_else(|| KernelError::Runtime(
            "no hay world: llamá a dominium-world WxH primero".into(),
        ))?;
    let w_max = world.grid.width as f32 - 1.0;
    let h_max = world.grid.height as f32 - 1.0;
    let mut rng = Lcg::new(seed);
    for _ in 0..n {
        let x = rng.next_unit() * w_max;
        let y = rng.next_unit() * h_max;
        let psi = [
            rng.next_unit(),
            rng.next_unit(),
            rng.next_unit(),
            rng.next_unit(),
        ];
        world.lemmings.spawn(x, y, 100.0, psi);
    }
    Ok(text_output(format!(
        "sembrados {n} lemmings con seed={seed} (total={})",
        world.lemmings.len()
    )))
}

fn exec_tick(
    source: &str,
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let n: usize = if source.trim().is_empty() {
        1
    } else {
        source
            .trim()
            .parse()
            .map_err(|_| KernelError::Runtime(format!("N debe ser un usize: '{source}'")))?
    };
    let mut s = lock(state)?;
    let params = s.params.clone();
    let world = s
        .world
        .as_mut()
        .ok_or_else(|| KernelError::Runtime(
            "no hay world: llamá a dominium-world WxH primero".into(),
        ))?;
    for _ in 0..n {
        physics_tick(world, &params);
    }
    let stats = WorldStats::from_world(world);
    Ok(stats_to_output(&stats, Some(n)))
}

fn exec_stats(
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let s = lock(state)?;
    let world = s
        .world
        .as_ref()
        .ok_or_else(|| KernelError::Runtime(
            "no hay world: llamá a dominium-world WxH primero".into(),
        ))?;
    let stats = WorldStats::from_world(world);
    Ok(stats_to_output(&stats, None))
}

fn exec_param(
    source: &str,
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let mut s = lock(state)?;
    let mut changed: Vec<String> = Vec::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(|| {
            KernelError::Runtime(format!(
                "se espera NAME=VALUE por línea, llegó: '{line}'"
            ))
        })?;
        let name = name.trim();
        let value: f32 = value.trim().parse().map_err(|_| {
            KernelError::Runtime(format!(
                "VALUE debe ser un f32 para '{name}': '{}'",
                value.trim()
            ))
        })?;
        set_param_field(&mut s.params, name, value)?;
        changed.push(format!("{name}={value}"));
    }
    if changed.is_empty() {
        return Err(KernelError::Runtime(
            "ninguna asignación NAME=VALUE encontrada en la celda".into(),
        ));
    }
    Ok(text_output(format!("params actualizados: {}", changed.join(", "))))
}

/// Setea uno de los campos `f32` planos de [`SimParams`]. La lista
/// está cerrada explícitamente porque los campos no triviales
/// (`relieve` que es array, `action_policy` que es enum, `action_weights`
/// que es matriz, `trade_target` que es enum) requieren parsers
/// dedicados; ese alcance queda fuera del MVP.
fn set_param_field(p: &mut SimParams, name: &str, v: f32) -> Result<(), KernelError> {
    match name {
        "move_speed" => p.move_speed = v,
        "move_cost" => p.move_cost = v,
        "extract_rate" => p.extract_rate = v,
        "degr_per_extract" => p.degr_per_extract = v,
        "sync_rate" => p.sync_rate = v,
        "trade_amount" => p.trade_amount = v,
        "replicate_threshold" => p.replicate_threshold = v,
        "child_energy_frac" => p.child_energy_frac = v,
        "fight_damage" => p.fight_damage = v,
        "absorb_frac" => p.absorb_frac = v,
        "desperation_threshold" => p.desperation_threshold = v,
        "abundance_threshold" => p.abundance_threshold = v,
        "metabolic_cost" => p.metabolic_cost = v,
        "diffusion_rate" => p.diffusion_rate = v,
        "entropy_rate" => p.entropy_rate = v,
        "climb_cost" => p.climb_cost = v,
        "season_amplitude" => p.season_amplitude = v,
        "regrowth_rate" => p.regrowth_rate = v,
        "carrying_capacity" => p.carrying_capacity = v,
        "psi_effect_modulation" => p.psi_effect_modulation = v,
        "social_radius" => p.social_radius = v,
        "contagion_rate" => p.contagion_rate = v,
        other => {
            return Err(KernelError::Runtime(format!(
                "parámetro no soportado por este kernel: '{other}' \
                 (sólo campos escalares f32 — relieve/action_policy/etc \
                 quedan fuera del MVP)"
            )));
        }
    }
    Ok(())
}

fn exec_render(
    source: &str,
    state: &Arc<Mutex<DominiumState>>,
) -> Result<KernelOutput, KernelError> {
    let mut it = source.split_whitespace();
    let w_px: u32 = it
        .next()
        .map(|s| {
            s.parse::<u32>().map_err(|_| {
                KernelError::Runtime(format!("WIDTH inválido: '{s}'"))
            })
        })
        .transpose()?
        .unwrap_or(256);
    let h_px: u32 = it
        .next()
        .map(|s| {
            s.parse::<u32>().map_err(|_| {
                KernelError::Runtime(format!("HEIGHT inválido: '{s}'"))
            })
        })
        .transpose()?
        .unwrap_or(256);
    // SCALE (zoom ≥ 1.0): multiplica la resolución de salida. `rasterize_world`
    // mapea TODO el grid al lienzo, así que más píxeles = misma vista del mundo
    // con más detalle (cada celda ocupa más píxeles). Valor inválido/≤0 → 1.0.
    let scale: f32 = it
        .next()
        .map(|s| s.parse::<f32>().unwrap_or(1.0))
        .unwrap_or(1.0);
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };

    if w_px == 0 || h_px == 0 || w_px > 4096 || h_px > 4096 {
        return Err(KernelError::Runtime(format!(
            "WIDTH/HEIGHT debe estar en [1, 4096], llegó {w_px}x{h_px}"
        )));
    }

    // Dimensiones finales tras aplicar el zoom, clampeadas al techo de 4096
    // por lado (evita que un SCALE grande dispare una asignación enorme).
    let out_w = (((w_px as f32) * scale).round() as u32).clamp(1, 4096);
    let out_h = (((h_px as f32) * scale).round() as u32).clamp(1, 4096);

    let s = lock(state)?;
    let world = s
        .world
        .as_ref()
        .ok_or_else(|| KernelError::Runtime(
            "no hay world: llamá a dominium-world WxH primero".into(),
        ))?;
    let png = rasterize_world(world, out_w, out_h);
    Ok(CellOutput {
        stdout: format!("rasterizado {out_w}×{out_h} px ({} bytes PNG)", png.len()),
        value: Some(format!("{}x{}", out_w, out_h)),
        payload: OutputPayload::Image {
            width: out_w,
            height: out_h,
            mime: "image/png".to_string(),
            bytes: png,
        },
    })
}

/// Rasteriza el `World` a un PNG RGBA `w_px × h_px`. Mapeo:
/// - cada pixel del PNG corresponde a una posición (x, y) de la grilla
///   muestreada con vecino más cercano;
/// - el color de la celda es una combinación de las 5 capas normalizadas
///   contra su pico actual (so las escalas no colapsan): rojo = poder,
///   verde = materia, azul = psique, amarillo (R+G) = oro, marrón
///   atenuante = degradación;
/// - cada lemming se pinta como un punto blanco de 2×2 px (clamped).
fn rasterize_world(world: &dominium_core::World, w_px: u32, h_px: u32) -> Vec<u8> {
    let g = &world.grid;
    let gw = g.width.max(1) as f32;
    let gh = g.height.max(1) as f32;

    // Peaks para normalizar — evita que escenas vacías queden negras
    // por completo (un valor pequeño se vuelve visible relativamente).
    let peak = |layer: &[f32]| -> f32 {
        layer
            .iter()
            .copied()
            .fold(0.0_f32, |m, v| if v > m { v } else { m })
            .max(1e-6)
    };
    let p_mat = peak(&g.materia);
    let p_psi = peak(&g.psique);
    let p_pod = peak(&g.poder);
    let p_oro = peak(&g.oro);
    let p_deg = peak(&g.degradacion);

    let mut buf: Vec<u8> = vec![0u8; (w_px as usize) * (h_px as usize) * 4];
    for py in 0..h_px {
        let gy = ((py as f32) * gh / h_px as f32).floor() as usize;
        let gy = gy.min(g.height - 1);
        for px in 0..w_px {
            let gx = ((px as f32) * gw / w_px as f32).floor() as usize;
            let gx = gx.min(g.width - 1);
            let idx = g.idx(gx, gy);
            let mat = g.materia[idx] / p_mat;
            let psi = g.psique[idx] / p_psi;
            let pod = g.poder[idx] / p_pod;
            let oro = g.oro[idx] / p_oro;
            let deg = g.degradacion[idx] / p_deg;

            // Mezcla: R = poder + 0.6*oro; G = materia + 0.6*oro; B = psique.
            // Degradación atenúa todo (suelo quemado).
            let atten = (1.0 - 0.5 * deg).max(0.2);
            let r = ((pod + 0.6 * oro) * atten).clamp(0.0, 1.0);
            let g_c = ((mat + 0.6 * oro) * atten).clamp(0.0, 1.0);
            let b = (psi * atten).clamp(0.0, 1.0);
            let off = ((py as usize) * w_px as usize + px as usize) * 4;
            buf[off] = (r * 255.0) as u8;
            buf[off + 1] = (g_c * 255.0) as u8;
            buf[off + 2] = (b * 255.0) as u8;
            buf[off + 3] = 255;
        }
    }

    // Pinta lemmings como pixels blancos (2×2) por agente. Coords
    // físicas en (0..g.width-1, 0..g.height-1) → (0..w_px-1, 0..h_px-1).
    let inv_gw = if g.width > 1 { (w_px as f32 - 1.0) / (g.width as f32 - 1.0) } else { 0.0 };
    let inv_gh = if g.height > 1 { (h_px as f32 - 1.0) / (g.height as f32 - 1.0) } else { 0.0 };
    for i in 0..world.lemmings.len() {
        let lx = world.lemmings.pos_x[i];
        let ly = world.lemmings.pos_y[i];
        let px = (lx * inv_gw) as i32;
        let py = (ly * inv_gh) as i32;
        for dy in 0..2i32 {
            for dx in 0..2i32 {
                let x = (px + dx).clamp(0, w_px as i32 - 1) as usize;
                let y = (py + dy).clamp(0, h_px as i32 - 1) as usize;
                let off = (y * w_px as usize + x) * 4;
                buf[off] = 255;
                buf[off + 1] = 255;
                buf[off + 2] = 255;
                buf[off + 3] = 255;
            }
        }
    }

    encode_png_rgba(&buf, w_px, h_px)
}

fn encode_png_rgba(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(rgba.len() / 2);
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("encoder header");
        writer.write_image_data(rgba).expect("encoder data");
    }
    out
}

fn stats_to_output(stats: &WorldStats, ticks_run: Option<usize>) -> KernelOutput {
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(16);
    if let Some(t) = ticks_run {
        rows.push(vec!["ticks_aplicados".to_string(), t.to_string()]);
    }
    rows.push(vec!["n".to_string(), stats.n.to_string()]);
    rows.push(vec![
        "gini_energia".to_string(),
        format!("{:.4}", stats.gini_energia),
    ]);
    rows.push(vec![
        "total_energia".to_string(),
        format!("{:.2}", stats.total_energia),
    ]);
    rows.push(vec![
        "mean_edad".to_string(),
        format!("{:.2}", stats.mean_edad),
    ]);
    for (k, label) in ["orden", "miedo", "curiosidad", "corruptibilidad"]
        .iter()
        .enumerate()
    {
        rows.push(vec![
            format!("var_psi_{label}"),
            format!("{:.4}", stats.var_psi[k]),
        ]);
    }
    for (k, label) in
        ["mover", "extraer", "sincronizar", "intercambiar", "replicar", "pelear"]
            .iter()
            .enumerate()
    {
        rows.push(vec![
            format!("action_{label}"),
            stats.action_counts[k].to_string(),
        ]);
    }
    rows.push(vec![
        "total_materia".to_string(),
        format!("{:.2}", stats.total_materia),
    ]);
    rows.push(vec![
        "total_psique".to_string(),
        format!("{:.2}", stats.total_psique),
    ]);
    rows.push(vec![
        "total_poder".to_string(),
        format!("{:.2}", stats.total_poder),
    ]);
    rows.push(vec![
        "total_oro".to_string(),
        format!("{:.2}", stats.total_oro),
    ]);
    rows.push(vec![
        "total_degradacion".to_string(),
        format!("{:.2}", stats.total_degradacion),
    ]);
    let stdout = rows
        .iter()
        .map(|r| format!("{:<28} {}", r[0], r[1]))
        .collect::<Vec<_>>()
        .join("\n");
    CellOutput {
        stdout,
        value: Some(stats.n.to_string()),
        payload: OutputPayload::Table {
            columns: vec!["key".into(), "value".into()],
            rows,
        },
    }
}

fn text_output(msg: impl Into<String>) -> KernelOutput {
    let s = msg.into();
    CellOutput {
        stdout: s.clone(),
        value: None,
        payload: OutputPayload::Text(s),
    }
}

fn lock<'a>(
    state: &'a Arc<Mutex<DominiumState>>,
) -> Result<std::sync::MutexGuard<'a, DominiumState>, KernelError> {
    state
        .lock()
        .map_err(|_| KernelError::Runtime("kernel state envenenado".into()))
}

fn parse_required<T: std::str::FromStr>(
    raw: Option<&str>,
    name: &str,
) -> Result<T, KernelError> {
    let raw = raw.ok_or_else(|| KernelError::Runtime(format!("falta {name}")))?;
    raw.parse::<T>()
        .map_err(|_| KernelError::Runtime(format!("{name} inválido: '{raw}'")))
}

/// LCG mínimo determinista (mismos constantes que `numerical recipes`).
/// Bit-exacto cross-platform — bastante para sembrar lemmings de un
/// notebook reproducible. NO usar para criptografía.
struct Lcg {
    state: u64,
}

impl Lcg {
    fn new(seed: u64) -> Self {
        // Evita el estado 0 absorbente — si el caller pasó 0,
        // arrancamos en una semilla impar conocida.
        let state = if seed == 0 { 0xDEADBEEF_CAFEBABEu64 } else { seed };
        Self { state }
    }
    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.state >> 32) as u32
    }
    /// Float en [0, 1).
    fn next_unit(&mut self) -> f32 {
        // 24 bits altos del u32 → mantisa de f32 — distribución
        // uniforme correcta sin sesgos por shift.
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_notebook_core::{CellKind, Notebook};
    use pluma_notebook_exec::run_all;

    fn kernel() -> DominiumKernel {
        DominiumKernel::new()
    }

    #[tokio::test]
    async fn world_resetea_grilla() {
        let k = kernel();
        let out = k.execute("16 8", "dominium-world").await.unwrap();
        let s = k.snapshot();
        let w = s.world.unwrap();
        assert_eq!(w.grid.width, 16);
        assert_eq!(w.grid.height, 8);
        assert_eq!(w.lemmings.len(), 0);
        assert!(out.stdout.contains("16×8"));
    }

    #[tokio::test]
    async fn seed_sin_world_falla() {
        let k = kernel();
        let r = k.execute("100", "dominium-seed").await;
        assert!(matches!(r, Err(KernelError::Runtime(ref m)) if m.contains("dominium-world")));
    }

    #[tokio::test]
    async fn seed_determinista_misma_seed_misma_poblacion() {
        let k1 = kernel();
        k1.execute("16 16", "dominium-world").await.unwrap();
        k1.execute("50 42", "dominium-seed").await.unwrap();
        let pop1 = k1.snapshot().world.unwrap().lemmings.pos_x.clone();

        let k2 = kernel();
        k2.execute("16 16", "dominium-world").await.unwrap();
        k2.execute("50 42", "dominium-seed").await.unwrap();
        let pop2 = k2.snapshot().world.unwrap().lemmings.pos_x.clone();

        assert_eq!(pop1, pop2, "misma seed debe producir misma población");
    }

    #[tokio::test]
    async fn tick_avanza_reloj() {
        let k = kernel();
        k.execute("16 16", "dominium-world").await.unwrap();
        k.execute("50 1", "dominium-seed").await.unwrap();
        let t0 = k.snapshot().world.unwrap().tick_count;
        k.execute("10", "dominium-tick").await.unwrap();
        let t1 = k.snapshot().world.unwrap().tick_count;
        assert_eq!(t1 - t0, 10);
    }

    #[tokio::test]
    async fn tick_vacio_es_uno() {
        let k = kernel();
        k.execute("8 8", "dominium-world").await.unwrap();
        k.execute("5 1", "dominium-seed").await.unwrap();
        let t0 = k.snapshot().world.unwrap().tick_count;
        k.execute("", "dominium-tick").await.unwrap();
        let t1 = k.snapshot().world.unwrap().tick_count;
        assert_eq!(t1 - t0, 1);
    }

    #[tokio::test]
    async fn stats_devuelve_tabla() {
        let k = kernel();
        k.execute("8 8", "dominium-world").await.unwrap();
        k.execute("3 1", "dominium-seed").await.unwrap();
        let out = k.execute("", "dominium-stats").await.unwrap();
        match out.payload {
            OutputPayload::Table { columns, rows } => {
                assert_eq!(columns, vec!["key".to_string(), "value".to_string()]);
                let n_row = rows.iter().find(|r| r[0] == "n").unwrap();
                assert_eq!(n_row[1], "3");
            }
            other => panic!("se esperaba Table, llegó {other:?}"),
        }
    }

    #[tokio::test]
    async fn param_setea_campo_conocido() {
        let k = kernel();
        k.execute("move_speed=0.75", "dominium-param").await.unwrap();
        assert!((k.snapshot().params.move_speed - 0.75).abs() < 1e-6);
    }

    #[tokio::test]
    async fn param_multiline_setea_varios() {
        let k = kernel();
        k.execute("move_speed=0.5\nsync_rate=0.1", "dominium-param")
            .await
            .unwrap();
        let p = k.snapshot().params;
        assert!((p.move_speed - 0.5).abs() < 1e-6);
        assert!((p.sync_rate - 0.1).abs() < 1e-6);
    }

    #[tokio::test]
    async fn param_desconocido_falla() {
        let k = kernel();
        let r = k.execute("relieve=0.5", "dominium-param").await;
        assert!(matches!(r, Err(KernelError::Runtime(_))));
    }

    #[tokio::test]
    async fn lenguaje_no_dominium_falla() {
        let k = kernel();
        let r = k.execute("hola", "python").await;
        assert!(matches!(r, Err(KernelError::Runtime(ref m)) if m.contains("no reconocido")));
    }

    #[tokio::test]
    async fn render_sin_world_falla() {
        let k = kernel();
        let r = k.execute("64 64", "dominium-render").await;
        assert!(matches!(r, Err(KernelError::Runtime(ref m)) if m.contains("dominium-world")));
    }

    #[tokio::test]
    async fn render_produce_png_payload() {
        let k = kernel();
        k.execute("16 16", "dominium-world").await.unwrap();
        k.execute("20 1", "dominium-seed").await.unwrap();
        let out = k.execute("64 64", "dominium-render").await.unwrap();
        match out.payload {
            OutputPayload::Image {
                width,
                height,
                mime,
                bytes,
            } => {
                assert_eq!(width, 64);
                assert_eq!(height, 64);
                assert_eq!(mime, "image/png");
                // Header PNG: 89 50 4E 47 0D 0A 1A 0A
                assert_eq!(
                    &bytes[..8],
                    &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]
                );
            }
            other => panic!("se esperaba Image, llegó {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_scale_multiplica_la_resolucion() {
        let k = kernel();
        k.execute("16 16", "dominium-world").await.unwrap();
        // 64×64 base con SCALE 2.0 → 128×128 de salida.
        let out = k.execute("64 64 2.0", "dominium-render").await.unwrap();
        match out.payload {
            OutputPayload::Image { width, height, .. } => {
                assert_eq!(width, 128);
                assert_eq!(height, 128);
            }
            other => panic!("se esperaba Image, llegó {other:?}"),
        }
        assert_eq!(out.value.as_deref(), Some("128x128"));
    }

    #[tokio::test]
    async fn render_scale_clampea_al_techo() {
        let k = kernel();
        k.execute("8 8", "dominium-world").await.unwrap();
        // 4096 base × 2 → clamp a 4096 (no 8192).
        let out = k.execute("4096 4096 2", "dominium-render").await.unwrap();
        if let OutputPayload::Image { width, height, .. } = out.payload {
            assert_eq!((width, height), (4096, 4096));
        } else {
            panic!("se esperaba Image");
        }
    }

    #[tokio::test]
    async fn render_defaults_256x256() {
        let k = kernel();
        k.execute("8 8", "dominium-world").await.unwrap();
        let out = k.execute("", "dominium-render").await.unwrap();
        if let OutputPayload::Image { width, height, .. } = out.payload {
            assert_eq!(width, 256);
            assert_eq!(height, 256);
        } else {
            panic!("se esperaba Image");
        }
    }

    #[tokio::test]
    async fn render_dimensiones_invalidas_falla() {
        let k = kernel();
        k.execute("8 8", "dominium-world").await.unwrap();
        let r = k.execute("0 100", "dominium-render").await;
        assert!(matches!(r, Err(KernelError::Runtime(_))));
        let r2 = k.execute("100 8000", "dominium-render").await;
        assert!(matches!(r2, Err(KernelError::Runtime(_))));
    }

    #[tokio::test]
    async fn notebook_completo_ejecuta_en_topo_order() {
        // Notebook con cadena world → seed → param → tick. Una sola
        // corrida con run_all debe dejar el world con lemmings vivos +
        // tick_count > 0.
        let k = kernel();
        let mut nb = Notebook::new();
        let w = nb.push(
            CellKind::Code { language: "dominium-world".into() },
            "32 24",
        );
        let s = nb.push(
            CellKind::Code { language: "dominium-seed".into() },
            "100 7",
        );
        let p = nb.push(
            CellKind::Code { language: "dominium-param".into() },
            "move_speed=0.4\nsync_rate=0.05",
        );
        let t = nb.push(
            CellKind::Code { language: "dominium-tick".into() },
            "20",
        );
        nb.add_dependency(s, w);
        nb.add_dependency(p, w);
        nb.add_dependency(t, s);
        nb.add_dependency(t, p);

        let report = run_all(&mut nb, &k).await.unwrap();
        assert_eq!(report.executed.len(), 4);
        assert!(report.failed.is_empty());

        let snap = k.snapshot();
        let w = snap.world.as_ref().unwrap();
        // El sim puede ganar o perder lemmings durante el tick
        // (Replicar/Pelear cambian la población). Sólo verificamos
        // que hubo siembra y que el reloj corrió N ticks.
        assert!(w.lemmings.len() > 0, "el seed sembró población");
        assert_eq!(w.tick_count, 20, "tick avanzó el reloj N pasos");
        assert!((snap.params.move_speed - 0.4).abs() < 1e-6);
    }
}
