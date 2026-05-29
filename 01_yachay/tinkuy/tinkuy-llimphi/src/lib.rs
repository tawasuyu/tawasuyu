//! `tinkuy-llimphi` — frontend Llimphi del motor de partículas.
//!
//! Capa 4 · E1 del roadmap: el **chassis**. Panel único con tiles
//! draggables ([[feedback_panel_tiles_draggables]]):
//!
//! ```text
//!   ┌─ visor 3D ──┐ ┌─ fuerzas ───┐
//!   │             │ │ ε  σ  cutoff │
//!   │ (E3 pinta)  │ │              │
//!   └─────────────┘ └──────────────┘
//!   ┌─ observables┐ ┌─ snapshots ─┐
//!   │ step  T  KE │ │ #step  CID  │
//!   │ |p|   CID   │ │  …  ring K  │
//!   └─────────────┘ └──────────────┘
//! ```
//!
//! El motor (`tinkuy-core` + `tinkuy-forces`) corre **dentro del
//! `update`**: un `Handle::spawn_periodic` dispara `Msg::Tick` cada
//! ~33 ms, el `update` avanza `STEPS_POR_TICK` pasos de Velocity-Verlet
//! sobre el `World` del modelo, refresca observables y guarda la última
//! K snapshots en un ring buffer. El `view` lee el modelo y pinta los
//! cuatro tiles.
//!
//! El **visor 3D** queda como placeholder textual hasta E3 — el tile
//! existe, recibe el drag-to-swap y aloja un view; lo que pinta dentro
//! lo decide E3 con `View::paint_with`.

#![forbid(unsafe_code)]

use std::collections::VecDeque;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_tiled::{tiled_view_reorderable_cols, TileSpec, TiledPalette};

use tinkuy_core::{
    kinetic_energy, reflect_walls, temperature, total_momentum, velocity_verlet_step,
    Grid3D, IntegratorParams, Outbox, Snapshot, World,
};
use tinkuy_forces::{clear_accelerations, lennard_jones, LjParams};

// ─── Parámetros del demo ──────────────────────────────────────────────────────

/// Mismas unidades reducidas que `tinkuy-sim` para que la salida sea
/// directamente comparable.
const SIGMA: f32 = 1.0;
const EPSILON: f32 = 1.0;
const CUTOFF: f32 = 2.5;
const SPACING: f32 = 1.5 * SIGMA;
const DT: f32 = 0.005;
const KB: f64 = 1.0;
const TEMP_INIT: f32 = 0.5;

/// Tamaño del lattice cúbico inicial. `SIDE³` partículas — 4³ = 64 mantiene
/// el ritmo del `update` por debajo de los 33 ms del tick incluso single-thread.
const SIDE: usize = 4;

/// Pasos de simulación por `Msg::Tick`. Con `33 ms / tick × 4 steps ≈ 120 steps/s`
/// — suficiente para ver evolucionar `T` y la huella de los CIDs en vivo.
const STEPS_POR_TICK: usize = 4;

/// Período del tick periódico. ~30 Hz de UI; cualquier valor menor a `16 ms`
/// es invisible bajo el coste del compositor.
const TICK_MS: u64 = 33;

/// Profundidad del ring buffer de snapshots mostrado en el tile correspondiente.
const SNAPSHOTS_K: usize = 12;

// ─── Modelo y mensajes ────────────────────────────────────────────────────────

/// Identidad de cada tile del panel. Se reordenan vía drag de su title bar.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileId {
    Visor,
    Fuerzas,
    Observables,
    Snapshots,
}

#[derive(Clone)]
pub enum Msg {
    /// Una rebanada de simulación. Disparado por el `spawn_periodic` desde
    /// `init` y reinscrito por el propio runtime — el `update` no agenda
    /// el siguiente tick, sólo consume.
    Tick,
    /// Alterna pausa. La pausa se respeta dentro del `update`: el tick
    /// sigue llegando pero el `update` no avanza la simulación.
    TogglePause,
    /// Reinicia el `World` al estado inicial (lattice + velocidades térmicas).
    Reset,
    /// Drag-to-swap del tiled. Llega desde la title bar de cualquier tile.
    Swap { from: usize, to: usize },
}

pub struct Model {
    world: World,
    grid: Grid3D,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
    params: IntegratorParams,
    lj: LjParams,
    outboxes: Vec<Outbox>,
    step: usize,
    t: f64,
    paused: bool,
    /// Última observación calculada — refresca cada tick. Se cachea acá para
    /// que el `view` no recompute (`kinetic_energy`, `total_momentum`,
    /// `Snapshot::capture` no son baratos a 30 Hz sobre el `World` entero).
    obs: Observables,
    /// Ring buffer de las últimas `SNAPSHOTS_K` CIDs.
    snapshots: VecDeque<SnapshotEntry>,
    /// Orden visual de los tiles. Drag-to-swap muta este vec.
    tiles: Vec<TileId>,
}

#[derive(Clone, Copy)]
struct Observables {
    ke: f64,
    temp: f64,
    p_mag: f64,
    cid_short: [u8; 8],
}

#[derive(Clone, Copy)]
struct SnapshotEntry {
    step: usize,
    cid_short: [u8; 8],
}

// ─── PRNG determinista (igual que tinkuy-sim, sin dep externa) ─────────────────

struct SplitMix64(u64);
impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_centered(&mut self) -> f32 {
        let bits = self.next_u64();
        (bits as i64 as f64 / i64::MAX as f64) as f32
    }
}

// ─── Inicialización del mundo ─────────────────────────────────────────────────

fn init_world() -> (World, Grid3D, [f32; 3], [f32; 3]) {
    let n_actual = SIDE * SIDE * SIDE;
    let l = SIDE as f32 * SPACING + CUTOFF;

    let bounds_min = [0.0; 3];
    let bounds_max = [l, l, l];

    let mut w = World::with_capacity(n_actual);
    let mut rng = SplitMix64::new(0xC0FFEE);
    let vscale = TEMP_INIT.sqrt();
    let half = SPACING * 0.5;
    for k in 0..SIDE {
        for j in 0..SIDE {
            for i in 0..SIDE {
                let x = i as f32 * SPACING + half + (CUTOFF * 0.5);
                let y = j as f32 * SPACING + half + (CUTOFF * 0.5);
                let z = k as f32 * SPACING + half + (CUTOFF * 0.5);
                let vx = rng.next_centered() * vscale;
                let vy = rng.next_centered() * vscale;
                let vz = rng.next_centered() * vscale;
                w.spawn([x, y, z], [vx, vy, vz], 1.0, 0.0);
            }
        }
    }

    // Sustrae drift del CM — Σp = 0 al arranque, igual que tinkuy-sim.
    let [px, py, pz] = total_momentum(&w);
    let m_total = n_actual as f64;
    let dvx = (px / m_total) as f32;
    let dvy = (py / m_total) as f32;
    let dvz = (pz / m_total) as f32;
    for i in 0..n_actual {
        w.vxs.0[i] -= dvx;
        w.vys.0[i] -= dvy;
        w.vzs.0[i] -= dvz;
    }

    let dims_x = ((l / CUTOFF).ceil() as u32).max(3);
    let mut g = Grid3D::new(bounds_min, CUTOFF, [dims_x; 3], n_actual);
    g.rebuild(&w);

    (w, g, bounds_min, bounds_max)
}

fn capture_obs(world: &World) -> Observables {
    let ke = kinetic_energy(world);
    let temp = temperature(world, KB);
    let [px, py, pz] = total_momentum(world);
    let p_mag = (px * px + py * py + pz * pz).sqrt();
    let snap = Snapshot::capture(world);
    let mut cid_short = [0u8; 8];
    cid_short.copy_from_slice(&snap.cid[..8]);
    Observables {
        ke,
        temp,
        p_mag,
        cid_short,
    }
}

fn cid_to_hex(cid: &[u8; 8]) -> String {
    let mut s = String::with_capacity(16);
    for b in cid {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ─── App ──────────────────────────────────────────────────────────────────────

pub struct TinkuyApp;

impl App for TinkuyApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "tinkuy · LJ 4³ partículas (espacio: pausa · r: reset · drag titles: swap)"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 760)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let (world, grid, bounds_min, bounds_max) = init_world();
        let params = IntegratorParams {
            dt: DT,
            bounds_min,
            bounds_max,
        };
        let lj = LjParams {
            epsilon: EPSILON,
            sigma: SIGMA,
            cutoff: CUTOFF,
        };
        // Driver de simulación: un Tick cada TICK_MS ms. El periodic vive
        // hasta que el event loop se cierra; ver `Handle::spawn_periodic`.
        handle.spawn_periodic(std::time::Duration::from_millis(TICK_MS), || Msg::Tick);

        let obs = capture_obs(&world);
        Model {
            world,
            grid,
            bounds_min,
            bounds_max,
            params,
            lj,
            outboxes: vec![Outbox::default()],
            step: 0,
            t: 0.0,
            paused: false,
            obs,
            snapshots: VecDeque::with_capacity(SNAPSHOTS_K),
            tiles: vec![
                TileId::Visor,
                TileId::Fuerzas,
                TileId::Observables,
                TileId::Snapshots,
            ],
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                if !model.paused {
                    // `LjParams` no es `Copy`; tomamos referencia para evitar
                    // mover el campo en cada iteración del closure.
                    let lj_ref = &model.lj;
                    for _ in 0..STEPS_POR_TICK {
                        velocity_verlet_step(
                            &mut model.world,
                            &mut model.grid,
                            &model.params,
                            &mut model.outboxes,
                            |world, grid| {
                                clear_accelerations(world);
                                lennard_jones(world, grid, lj_ref);
                            },
                        );
                        reflect_walls(&mut model.world, model.bounds_min, model.bounds_max);
                        model.step += 1;
                        model.t += DT as f64;
                    }
                    let obs = capture_obs(&model.world);
                    // Empuja al ring: drop más viejo si llena.
                    if model.snapshots.len() == SNAPSHOTS_K {
                        model.snapshots.pop_front();
                    }
                    model.snapshots.push_back(SnapshotEntry {
                        step: model.step,
                        cid_short: obs.cid_short,
                    });
                    model.obs = obs;
                }
            }
            Msg::TogglePause => {
                model.paused = !model.paused;
            }
            Msg::Reset => {
                let (w, g, bmin, bmax) = init_world();
                model.world = w;
                model.grid = g;
                model.bounds_min = bmin;
                model.bounds_max = bmax;
                model.params = IntegratorParams {
                    dt: DT,
                    bounds_min: bmin,
                    bounds_max: bmax,
                };
                model.step = 0;
                model.t = 0.0;
                model.obs = capture_obs(&model.world);
                model.snapshots.clear();
            }
            Msg::Swap { from, to } => {
                if from != to && from < model.tiles.len() && to < model.tiles.len() {
                    model.tiles.swap(from, to);
                }
            }
        }
        model
    }

    fn on_key(_model: &Model, event: &llimphi_ui::KeyEvent) -> Option<Msg> {
        use llimphi_ui::{Key, KeyState, NamedKey};
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Space) => Some(Msg::TogglePause),
            Key::Character(s) if s.eq_ignore_ascii_case("r") => Some(Msg::Reset),
            _ => None,
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = TiledPalette::from_theme(&theme);

        let tiles: Vec<TileSpec<Msg>> = model
            .tiles
            .iter()
            .map(|id| match id {
                TileId::Visor => TileSpec {
                    label: "visor 3D (E3 pendiente)".into(),
                    content: visor_placeholder(&theme),
                },
                TileId::Fuerzas => TileSpec {
                    label: "fuerzas (LJ)".into(),
                    content: fuerzas_body(model, &theme),
                },
                TileId::Observables => TileSpec {
                    label: "observables".into(),
                    content: observables_body(model, &theme),
                },
                TileId::Snapshots => TileSpec {
                    label: "snapshots (CID[..8])".into(),
                    content: snapshots_body(model, &theme),
                },
            })
            .collect();

        // 2 columnas fijas → grilla 2×2 estable. El auto-sqrt también daría
        // 2 para n=4 pero la promesa "2 cols" sobrevive a tiles vacíos.
        tiled_view_reorderable_cols(
            tiles,
            2,
            |from, to| Some(Msg::Swap { from, to }),
            &palette,
        )
    }
}

// ─── Cuerpos de tiles ─────────────────────────────────────────────────────────

fn padded_col<Msg2: Clone + Send + Sync + 'static>(
    children: Vec<View<Msg2>>,
    bg: Option<Color>,
) -> View<Msg2> {
    let mut v = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children);
    if let Some(c) = bg {
        v = v.fill(c);
    }
    v
}

fn text_row<Msg2: Clone + Send + Sync + 'static>(
    text: String,
    size: f32,
    color: Color,
) -> View<Msg2> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size + 6.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

fn visor_placeholder(theme: &Theme) -> View<Msg> {
    // Marcado deliberadamente como "placeholder" — E3 lo reemplaza con un
    // `View::paint_with` que pinta partículas como puntos coloreados por |v|.
    padded_col(
        vec![
            text_row("(visor 3D — E3)".into(), 13.0, theme.fg_muted),
            text_row(
                "el tile existe y recibe drag-to-swap.".into(),
                11.0,
                theme.fg_muted,
            ),
            text_row(
                "E3 enchufa paint_with(Scene) para".into(),
                11.0,
                theme.fg_muted,
            ),
            text_row(
                "pintar partículas con proyección ortográfica.".into(),
                11.0,
                theme.fg_muted,
            ),
        ],
        None,
    )
}

fn fuerzas_body(model: &Model, theme: &Theme) -> View<Msg> {
    let pausa = if model.paused { " · PAUSA" } else { "" };
    padded_col(
        vec![
            text_row(format!("ε      = {}", model.lj.epsilon), 13.0, theme.fg_text),
            text_row(format!("σ      = {}", model.lj.sigma), 13.0, theme.fg_text),
            text_row(format!("cutoff = {}", model.lj.cutoff), 13.0, theme.fg_text),
            text_row(format!("dt     = {}", DT), 13.0, theme.fg_text),
            text_row(format!("N      = {}", model.world.len()), 13.0, theme.fg_text),
            text_row(format!("steps/tick = {}", STEPS_POR_TICK), 13.0, theme.fg_muted),
            text_row(format!("modo: Lennard-Jones{}", pausa), 12.0, theme.accent),
            text_row(
                "[espacio] pausa · [r] reset".into(),
                11.0,
                theme.fg_muted,
            ),
        ],
        None,
    )
}

fn observables_body(model: &Model, theme: &Theme) -> View<Msg> {
    let cid = cid_to_hex(&model.obs.cid_short);
    padded_col(
        vec![
            text_row(format!("step = {}", model.step), 14.0, theme.fg_text),
            text_row(format!("t    = {:.3}", model.t), 14.0, theme.fg_text),
            text_row(format!("KE   = {:.6}", model.obs.ke), 14.0, theme.fg_text),
            text_row(format!("T    = {:.4}", model.obs.temp), 14.0, theme.accent),
            text_row(format!("|p|  = {:.3e}", model.obs.p_mag), 13.0, theme.fg_muted),
            text_row(format!("CID  = {}", cid), 12.0, theme.fg_muted),
        ],
        None,
    )
}

fn snapshots_body(model: &Model, theme: &Theme) -> View<Msg> {
    // Más reciente arriba — más legible que el orden natural del VecDeque.
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(SNAPSHOTS_K + 1);
    rows.push(text_row(
        format!("últimas {} CIDs (ring)", SNAPSHOTS_K),
        11.0,
        theme.fg_muted,
    ));
    if model.snapshots.is_empty() {
        rows.push(text_row("(esperando primer tick…)".into(), 12.0, theme.fg_muted));
    } else {
        for entry in model.snapshots.iter().rev() {
            let txt = format!("step {:>6}   {}", entry.step, cid_to_hex(&entry.cid_short));
            rows.push(text_row(txt, 12.0, theme.fg_text));
        }
    }
    padded_col(rows, None)
}

// ─── Entrypoint del demo ──────────────────────────────────────────────────────

/// Atajo: corre el frontend con su `App` por defecto. Equivalente a
/// `llimphi_ui::run::<TinkuyApp>()`. Mantenido para que el `examples/`
/// del crate quepa en una línea.
pub fn run() {
    llimphi_ui::run::<TinkuyApp>();
}
