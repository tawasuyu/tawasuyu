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
//! El **visor 3D** (E3) pinta vía `View::paint_with` con proyección
//! axonométrica fija (sin cámara orbital): cada partícula es un círculo
//! coloreado por |v| (cold→hot lerp en sRGB), ordenadas back-to-front
//! por painter's algorithm. El wireframe de la caja sim da contexto
//! espacial. Ver el módulo [`visor`] para la proyección pura testeable.

#![forbid(unsafe_code)]

pub mod grafo;
pub mod visor;

use std::collections::VecDeque;
use std::sync::Arc;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Circle, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_nodegraph::{nodegraph_view, NodegraphMetrics, NodegraphPalette};
use llimphi_widget_tiled::{tiled_view_reorderable_cols, TileSpec, TiledPalette};

use tinkuy_core::{
    kinetic_energy, reflect_walls, temperature, total_momentum, velocity_verlet_step,
    Grid3D, IntegratorParams, Outbox, Snapshot, World,
};
use tinkuy_dsl::{compile, optimize};
use tinkuy_forces::{clear_accelerations, DslForce};

use grafo::{render_nodes, ForceGraph, LiftError};

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
    /// Mueve un nodo del grafo de fuerzas. Llega del `nodegraph` al arrastrar
    /// la title bar de un nodo; el handler suma el delta a la posición.
    MoveForceNode {
        id: u32,
        dx: f32,
        dy: f32,
    },
    /// Conecta dos pins del grafo de fuerzas. Política: el último cable que
    /// llega a un pin de entrada reemplaza al anterior. Tras aplicar el
    /// cable se recompila el grafo a `DslForce` (o se reporta el error).
    ConnectForcePins {
        from_node: u32,
        from_output: u16,
        to_node: u32,
        to_input: u16,
    },
    /// Rebobina al snapshot `idx` del ring. Restaura el `World` con
    /// `Snapshot::restore_into` (CID idéntico al original), retrocede
    /// `step`/`t` al instante capturado y pausa la simulación para que
    /// el usuario pueda inspeccionar el estado antes de continuar.
    LoadSnapshot {
        idx: usize,
    },
}

pub struct Model {
    world: World,
    grid: Grid3D,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
    params: IntegratorParams,
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
    /// Grafo de fuerzas editable visualmente. Pre-poblado con LJ.
    pub force_graph: ForceGraph,
    /// `DslForce` compilada del último `force_graph` válido. `None` mientras
    /// el grafo esté roto (pin desconectado, ciclo, etc.) — el `tick` salta
    /// la simulación pero todo lo demás sigue funcionando.
    force: Option<DslForce>,
    /// Mensaje de estado de la última recompilación. Rojo si error, neutro
    /// si "ok". Se pinta en la title bar lógica del tile fuerzas.
    force_status: ForceStatus,
}

#[derive(Clone, Debug)]
enum ForceStatus {
    Ok,
    /// Texto humano del error. Renderizado tal cual; no se interpreta.
    Error(String),
}

#[derive(Clone, Copy)]
struct Observables {
    ke: f64,
    temp: f64,
    p_mag: f64,
    cid_short: [u8; 8],
}

#[derive(Clone)]
struct SnapshotEntry {
    step: usize,
    cid_short: [u8; 8],
    /// Payload del snapshot — el mismo `Snapshot::bytes` que generó la CID.
    /// `Arc<[u8]>` para que el clon en el ring buffer y el Msg sean baratos
    /// (~16 B por copia en vez de `n*44 + 8` por click).
    bytes: Arc<[u8]>,
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

/// Captura observables y devuelve también el `Snapshot` — `bytes` viajan al
/// ring para que un click en E4 pueda rebobinar. Los call sites que sólo
/// necesitan observables descartan el `Snapshot` con `_`.
fn capture_obs_and_snap(world: &World) -> (Observables, Snapshot) {
    let ke = kinetic_energy(world);
    let temp = temperature(world, KB);
    let [px, py, pz] = total_momentum(world);
    let p_mag = (px * px + py * py + pz * pz).sqrt();
    let snap = Snapshot::capture(world);
    let mut cid_short = [0u8; 8];
    cid_short.copy_from_slice(&snap.cid[..8]);
    (
        Observables {
            ke,
            temp,
            p_mag,
            cid_short,
        },
        snap,
    )
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
        // Driver de simulación: un Tick cada TICK_MS ms. El periodic vive
        // hasta que el event loop se cierra; ver `Handle::spawn_periodic`.
        handle.spawn_periodic(std::time::Duration::from_millis(TICK_MS), || Msg::Tick);

        let force_graph = ForceGraph::lennard_jones_default();
        let (force, force_status) = recompile_force(&force_graph);
        let (obs, _) = capture_obs_and_snap(&world);
        Model {
            world,
            grid,
            bounds_min,
            bounds_max,
            params,
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
            force_graph,
            force,
            force_status,
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                if !model.paused {
                    // Tomamos la fuerza por `&mut Option<DslForce>` — si está
                    // `None` (grafo roto), `clear_accelerations` corre pero
                    // ninguna fuerza se aplica: las partículas inerciales
                    // siguen su trayectoria. Es un fallback visual útil.
                    let force_opt = &mut model.force;
                    for _ in 0..STEPS_POR_TICK {
                        velocity_verlet_step(
                            &mut model.world,
                            &mut model.grid,
                            &model.params,
                            &mut model.outboxes,
                            |world, grid| {
                                clear_accelerations(world);
                                if let Some(f) = force_opt.as_mut() {
                                    f.apply(world, grid);
                                }
                            },
                        );
                        reflect_walls(&mut model.world, model.bounds_min, model.bounds_max);
                        model.step += 1;
                        model.t += DT as f64;
                    }
                    let (obs, snap) = capture_obs_and_snap(&model.world);
                    // Empuja al ring: drop más viejo si llena. `snap.bytes`
                    // ya está alocado por `Snapshot::capture` — lo movemos
                    // a un `Arc<[u8]>` directamente.
                    if model.snapshots.len() == SNAPSHOTS_K {
                        model.snapshots.pop_front();
                    }
                    model.snapshots.push_back(SnapshotEntry {
                        step: model.step,
                        cid_short: obs.cid_short,
                        bytes: Arc::from(snap.bytes.into_boxed_slice()),
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
                let (obs, _) = capture_obs_and_snap(&model.world);
                model.obs = obs;
                model.snapshots.clear();
            }
            Msg::Swap { from, to } => {
                if from != to && from < model.tiles.len() && to < model.tiles.len() {
                    model.tiles.swap(from, to);
                }
            }
            Msg::MoveForceNode { id, dx, dy } => {
                model.force_graph.move_node(id, dx, dy);
            }
            Msg::ConnectForcePins {
                from_node,
                from_output,
                to_node,
                to_input,
            } => {
                model
                    .force_graph
                    .rewire_input(from_node, from_output, to_node, to_input);
                let (force, status) = recompile_force(&model.force_graph);
                model.force = force;
                model.force_status = status;
            }
            Msg::LoadSnapshot { idx } => {
                if let Some(entry) = model.snapshots.get(idx).cloned() {
                    // `restore_into` repuebla las SoA y zera ax_prev; el
                    // CID tras restaurar coincide con `entry.cid_short` por
                    // construcción (round-trip de Snapshot, ver tinkuy-core).
                    if Snapshot::restore_into(&entry.bytes, &mut model.world).is_ok() {
                        // Rebuild de la grilla espacial para que el siguiente
                        // tick de fuerzas vea las partículas en sus celdas
                        // restauradas (las posiciones cambiaron).
                        model.grid.rebuild(&model.world);
                        model.step = entry.step;
                        model.t = entry.step as f64 * DT as f64;
                        // Recapturamos observables sobre el estado restaurado.
                        let (obs, _) = capture_obs_and_snap(&model.world);
                        model.obs = obs;
                        // Pausa: el usuario pidió ver este estado; respetar
                        // su mirada antes de reanudar (Space para retomar).
                        model.paused = true;
                    }
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
                    label: "visor 3D (axonométrico · |v|→color)".into(),
                    content: visor_body(model, &theme),
                },
                TileId::Fuerzas => TileSpec {
                    label: fuerzas_label(model),
                    content: fuerzas_body(model, &theme),
                },
                TileId::Observables => TileSpec {
                    label: "observables".into(),
                    content: observables_body(model, &theme),
                },
                TileId::Snapshots => TileSpec {
                    label: "snapshots · click rebobina".into(),
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

/// Tile del visor 3D (E3). Proyección axonométrica fija (ver
/// [`visor::project`]); partículas como circulitos coloreados por |v|
/// (cold→hot, lerp en sRGB premultiplicado). Sin cámara orbital — el
/// MVP confía en que el lattice 4³ con la inclinación de `z` ya se lee
/// como caja. Wireframe de la caja sim como contexto visual.
fn visor_body(model: &Model, theme: &Theme) -> View<Msg> {
    // Capturamos las SoA de posiciones/velocidades por valor — el painter
    // es `Arc<dyn Fn ... + 'static + Send + Sync>`. Con N=64 el clone son
    // ~1.5 KiB por frame; el coste del compositor lo eclipsa.
    let n = model.world.len();
    let xs = model.world.xs.0[..n].to_vec();
    let ys = model.world.ys.0[..n].to_vec();
    let zs = model.world.zs.0[..n].to_vec();
    let vxs = model.world.vxs.0[..n].to_vec();
    let vys = model.world.vys.0[..n].to_vec();
    let vzs = model.world.vzs.0[..n].to_vec();
    let bmin = model.bounds_min;
    let bmax = model.bounds_max;

    let bg = theme.bg_panel_alt;
    let edge_color = theme.border;
    // Cold→hot: azul-cian (frío) → naranja-rojo (caliente). Se interpolan
    // con `lerp_rect` por par; la gradiente en sRGB es suficiente sin
    // pisarse con el azul del tema oscuro.
    let cold = Color::from_rgba8(80, 160, 240, 255);
    let hot = Color::from_rgba8(240, 110, 60, 255);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(bg)
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 4.0 || rect.h <= 4.0 {
            return;
        }
        let pad: f32 = 18.0;
        let avail_w = (rect.w - 2.0 * pad).max(1.0);
        let avail_h = (rect.h - 2.0 * pad).max(1.0);

        let (umin, umax, vmin, vmax_box) = visor::project_bbox(bmin, bmax);
        let span_u = (umax - umin).max(1e-6);
        let span_v = (vmax_box - vmin).max(1e-6);
        let scale = (avail_w / span_u).min(avail_h / span_v);
        let proj_w = span_u * scale;
        let proj_h = span_v * scale;
        let off_x = rect.x + pad + (avail_w - proj_w) * 0.5;
        let off_y = rect.y + pad + (avail_h - proj_h) * 0.5;

        // Mapeo (u, v) sim → canvas; flip de v porque canvas crece hacia abajo.
        let map_uv = |u: f32, v: f32| -> (f64, f64) {
            let cx = off_x + (u - umin) * scale;
            let cy = off_y + (vmax_box - v) * scale;
            (cx as f64, cy as f64)
        };

        // 1) Wireframe de la caja sim (contexto espacial).
        let corners = visor::box_corners(bmin, bmax);
        let mut canvas_corners = [(0.0_f64, 0.0_f64); 8];
        for (i, &(cx, cy, cz)) in corners.iter().enumerate() {
            let (u, v) = visor::project(cx, cy, cz);
            canvas_corners[i] = map_uv(u, v);
        }
        let edge_stroke = Stroke::new(1.0);
        for &(a, b) in &visor::BOX_EDGES {
            let (x1, y1) = canvas_corners[a];
            let (x2, y2) = canvas_corners[b];
            let mut path = BezPath::new();
            path.move_to((x1, y1));
            path.line_to((x2, y2));
            scene.stroke(&edge_stroke, Affine::IDENTITY, edge_color, None, &path);
        }

        if n == 0 {
            return;
        }

        // 2) |v|_max para normalizar el color. Un solo paso, en el espacio
        // de velocidades; sqrt al final del bucle (más barato que por par).
        let mut vmax_sq = 0.0_f32;
        for i in 0..n {
            let v2 = vxs[i] * vxs[i] + vys[i] * vys[i] + vzs[i] * vzs[i];
            if v2 > vmax_sq {
                vmax_sq = v2;
            }
        }
        let v_max = vmax_sq.sqrt().max(1e-6);

        // 3) Painter's algorithm: ordenar back-to-front por depth_key.
        // Los más al fondo (depth_key alto) se pintan primero.
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_by(|&a, &b| {
            let da = visor::depth_key(xs[a as usize], zs[a as usize]);
            let db = visor::depth_key(xs[b as usize], zs[b as usize]);
            db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
        });

        // 4) Pintar partículas. Radio base 3 px; sin profundidad-radio en MVP.
        let radius: f64 = 3.0;
        for &idx in &order {
            let i = idx as usize;
            let (u, v) = visor::project(xs[i], ys[i], zs[i]);
            let (cx, cy) = map_uv(u, v);
            let spd = (vxs[i] * vxs[i] + vys[i] * vys[i] + vzs[i] * vzs[i]).sqrt();
            let t = (spd / v_max).clamp(0.0, 1.0);
            let col = cold.lerp_rect(hot, t);
            let circle = Circle::new((cx, cy), radius);
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &circle);
        }
    })
}

/// El label de la title bar del tile "fuerzas" lleva el estado de la última
/// recompilación: si está "ok" sirve sólo de identificación; si hay error,
/// el detalle viaja ahí para que el usuario no tenga que abrir otro panel.
fn fuerzas_label(model: &Model) -> String {
    match &model.force_status {
        ForceStatus::Ok => "fuerzas · grafo → bytecode (ok)".into(),
        ForceStatus::Error(msg) => format!("fuerzas · ERROR: {}", msg),
    }
}

fn fuerzas_body(model: &Model, theme: &Theme) -> View<Msg> {
    // Lienzo del grafo de fuerzas. Drag de title bar → mueve el nodo;
    // drag desde un pin de salida hacia un pin de entrada → conecta y
    // dispara recompilación a `DslForce`.
    let palette = NodegraphPalette::from_theme(theme);
    let metrics = NodegraphMetrics::default();
    let nodes = render_nodes(&model.force_graph);
    let wires = model.force_graph.wires.clone();
    nodegraph_view(
        &nodes,
        &wires,
        &palette,
        &metrics,
        // Por convención del widget: `Move` lleva el delta acumulado por
        // evento; `End` cierra el drag con un último Msg `(dx=0,dy=0)`
        // que no estorba porque `move_node` es aditivo.
        |id, phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::MoveForceNode { id, dx, dy }),
            DragPhase::End => None,
        },
        |from_node, from_output, to_node, to_input| {
            Some(Msg::ConnectForcePins {
                from_node,
                from_output,
                to_node,
                to_input,
            })
        },
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
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(SNAPSHOTS_K + 2);
    rows.push(text_row(
        format!("últimas {} CIDs (click → rebobinar)", SNAPSHOTS_K),
        11.0,
        theme.fg_muted,
    ));
    if model.snapshots.is_empty() {
        rows.push(text_row("(esperando primer tick…)".into(), 12.0, theme.fg_muted));
    } else {
        // El ring guarda `step` ascendente; iteramos en reverso para que el
        // más reciente quede arriba, manteniendo el `idx` original — el Msg
        // ::LoadSnapshot lo usa para indexar la VecDeque sin reverse.
        let total = model.snapshots.len();
        for (i, entry) in model.snapshots.iter().enumerate().rev() {
            let marker = if entry.step == model.step { "▶ " } else { "  " };
            let txt = format!(
                "{}step {:>6}   {}",
                marker,
                entry.step,
                cid_to_hex(&entry.cid_short)
            );
            rows.push(snapshot_row(txt, i, total, theme));
        }
    }
    padded_col(rows, None)
}

/// Una fila de snapshot — clickeable, con hover para sugerir que se puede
/// rebobinar a este estado. El `idx` es el índice en `model.snapshots`
/// (VecDeque) y viaja directo al `Msg::LoadSnapshot`.
fn snapshot_row(text: String, idx: usize, _total: usize, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text, 12.0, theme.fg_text, Alignment::Start)
    .hover_fill(theme.bg_row_hover)
    .on_click(Msg::LoadSnapshot { idx })
}

// ─── Recompilación grafo → DslForce ───────────────────────────────────────────

/// Lifta el grafo a `Expr`, optimiza, compila a `Bytecode` y arma un
/// `DslForce` listo para `apply`. Devuelve `(None, Error(msg))` si algo
/// falla — el caller deja `force = None` y la simulación corre sin fuerzas.
fn recompile_force(graph: &ForceGraph) -> (Option<DslForce>, ForceStatus) {
    let expr = match graph.lift_to_expr() {
        Ok(e) => e,
        Err(err) => return (None, ForceStatus::Error(lift_error_to_string(err))),
    };
    let expr_opt = optimize(expr);
    let bc = match compile(&expr_opt) {
        Ok(b) => b,
        Err(err) => {
            return (
                None,
                ForceStatus::Error(format!("compile: {:?}", err)),
            )
        }
    };
    let force = DslForce::from_bytecode(bc, EPSILON, SIGMA, CUTOFF).with_label("grafo");
    (Some(force), ForceStatus::Ok)
}

fn lift_error_to_string(err: LiftError) -> String {
    match err {
        LiftError::SinSalida => "grafo sin nodo F/r (Output)".into(),
        LiftError::SalidaDuplicada => "más de un nodo F/r — debe haber exactamente uno".into(),
        LiftError::PinDesconectado { node, pin } => {
            format!("pin {} del nodo #{} sin cablear", pin, node)
        }
        LiftError::Ciclo => "ciclo detectado en el grafo".into(),
    }
}

// ─── Entrypoint del demo ──────────────────────────────────────────────────────

/// Atajo: corre el frontend con su `App` por defecto. Equivalente a
/// `llimphi_ui::run::<TinkuyApp>()`. Mantenido para que el `examples/`
/// del crate quepa en una línea.
pub fn run() {
    llimphi_ui::run::<TinkuyApp>();
}

#[cfg(test)]
mod rewind_tests {
    //! Tests del round-trip de E4 — sin tocar la UI: ejercitamos el camino
    //! "capture → mutate → restore" tal como lo hace `Msg::LoadSnapshot`,
    //! para garantizar que la CID se conserva bit a bit y que la grilla
    //! puede repoblarse con el estado restaurado.
    use super::*;

    fn step_once(
        world: &mut World,
        grid: &mut Grid3D,
        params: &IntegratorParams,
        outboxes: &mut Vec<Outbox>,
        force: &mut DslForce,
    ) {
        velocity_verlet_step(world, grid, params, outboxes, |w, _g| {
            clear_accelerations(w);
            // No aplicamos fuerza acá — sólo queremos verificar el rewind
            // sobre un mundo "vivo" con posiciones/velocidades distintas
            // entre dos instantes. La fuerza importa para el round-trip
            // del CID sólo en la medida en que cambia el estado.
            let _ = force;
        });
    }

    #[test]
    fn rewind_restaura_cid_bit_a_bit() {
        let (mut world, mut grid, bmin, bmax) = init_world();
        let params = IntegratorParams { dt: DT, bounds_min: bmin, bounds_max: bmax };
        let mut outboxes = vec![Outbox::default()];
        let graph = ForceGraph::lennard_jones_default();
        let (force_opt, _) = recompile_force(&graph);
        let mut force = force_opt.expect("LJ default debe compilar");

        // Snapshot en t=0.
        let snap_a = Snapshot::capture(&world);

        // Avanza 16 steps con paredes (cualquier mutación sirve).
        for _ in 0..16 {
            step_once(&mut world, &mut grid, &params, &mut outboxes, &mut force);
            reflect_walls(&mut world, bmin, bmax);
        }
        // Snapshot en t=16.
        let snap_b = Snapshot::capture(&world);
        assert_ne!(snap_a.cid, snap_b.cid, "16 steps deben cambiar el estado");

        // Rewind a t=0 vía restore_into y comparamos CID byte a byte.
        Snapshot::restore_into(&snap_a.bytes, &mut world).unwrap();
        grid.rebuild(&world);
        let snap_a_again = Snapshot::capture(&world);
        assert_eq!(snap_a.cid, snap_a_again.cid, "rewind debe devolver bit-exacto");
    }

    #[test]
    fn rewind_dos_veces_a_dos_estados_distintos() {
        let (mut world, mut grid, bmin, bmax) = init_world();
        let params = IntegratorParams { dt: DT, bounds_min: bmin, bounds_max: bmax };
        let mut outboxes = vec![Outbox::default()];
        let graph = ForceGraph::lennard_jones_default();
        let mut force = recompile_force(&graph).0.unwrap();

        for _ in 0..4 {
            step_once(&mut world, &mut grid, &params, &mut outboxes, &mut force);
            reflect_walls(&mut world, bmin, bmax);
        }
        let snap_4 = Snapshot::capture(&world);

        for _ in 0..4 {
            step_once(&mut world, &mut grid, &params, &mut outboxes, &mut force);
            reflect_walls(&mut world, bmin, bmax);
        }
        let snap_8 = Snapshot::capture(&world);

        // Saltamos al 8, después al 4, después de vuelta al 8.
        Snapshot::restore_into(&snap_8.bytes, &mut world).unwrap();
        assert_eq!(Snapshot::capture(&world).cid, snap_8.cid);
        Snapshot::restore_into(&snap_4.bytes, &mut world).unwrap();
        assert_eq!(Snapshot::capture(&world).cid, snap_4.cid);
        Snapshot::restore_into(&snap_8.bytes, &mut world).unwrap();
        assert_eq!(Snapshot::capture(&world).cid, snap_8.cid);
    }
}
