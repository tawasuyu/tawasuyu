//! `dominium-app-llimphi` — la ventana viva del simulador sobre
//! Llimphi.
//!
//! Compone la cadena agnóstica de dominium con el canvas Llimphi:
//!
//! ```text
//!   dominium-core ─► dominium-physics ─► dominium-iso ─►
//!   dominium-render-plan ─► dominium-canvas-llimphi ─► [esta ventana]
//! ```
//!
//! Un loop de fondo (~11 Hz) avanza la simulación y reentra al
//! `update` vía `Handle::dispatch(Msg::Tick)`. Cuando la población
//! colapsa, el mundo se re-siembra solo. El panel derecho muestra
//! stats y dos controles (play/pausa, re-sembrar).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use dominium_canvas_llimphi::canvas_view;
use dominium_core::{
    BehaviorHack, Concepto, Conceptos, Epoch, LayerMods, PsiMetrics, SimParams, Trigger, World,
    WorldStats,
};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_physics::tick;
use dominium_core::kmeans_psi;
use dominium_render_plan::{
    build_plan_with_overrides, Color, PlanConfig, Quad, RenderLayer, RenderMode, RenderPlan,
};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Lado de la grilla cuadrada del mundo. 240×240 = 57 600 celdas: continente
/// con varios biomas (mares, ríos, llanuras, sierras, picos). El motor sigue
/// siendo O(grid) en difusión y O(N²) en `nearest`, así que la población
/// arranca en miles pero limitada por los frenos termodinámicos de
/// `init`-time (ver `SimParams` override).
const GRID: usize = 240;
/// Población inicial de Lemmings. Miles. La densidad efectiva queda más
/// baja que en la versión 80² histórica (≈0.043 lem/celda) porque sólo
/// spawnean en tierra navegable y el motor ya no permite el crecimiento
/// exponencial sin freno.
const LEMMINGS: usize = 2500;
/// Periodo del bucle de simulación (~11 Hz).
const TICK_MS: u64 = 90;
/// Cada cuántos ticks recalculamos k-means para colorear los clusters
/// (modo PsiCluster). 30 ticks ≈ 2.7s — suficiente para ver tribus
/// emergentes sin que el costo del kmeans (O(K·N·iter)) note.
const KMEANS_REFRESH_TICKS: u64 = 30;
/// Ancho del panel de stats.
const SIDE_WIDTH: f32 = 240.0;
/// Pack JSON por defecto — iglesia / banco / comuna / laboratorio + variantes.
/// Embebido para que el binario corra sin archivos sueltos en cwd.
const DEFAULT_PACK: &str = include_str!("../conceptos.default.json");
/// Scenarios embebidos: civilizaciones-arquetipo. Cada uno es un JSON con
/// la misma forma que el `DEFAULT_PACK`; el picker del panel cicla entre
/// ellos sin necesidad de archivos sueltos.
const PACK_ANDES: &str = include_str!("../packs/andes.json");
const PACK_MESOPOTAMIA: &str = include_str!("../packs/mesopotamia.json");
const PACK_CAPITALISMO: &str = include_str!("../packs/capitalismo.json");

/// Tamaño del ring de snapshots: ~18 segundos a 11 Hz. Permite ver hacia
/// atrás un par de minutos de simulación sin pasarse en RAM (cada snapshot
/// es un `World` clonado; con grid 40×40 y ~50 lemmings, ~30 KB).
const SNAPSHOT_RING_CAP: usize = 200;
/// Largo del trail por lemming. Tradeoff: muy alto y la pantalla se llena
/// de motas; muy bajo y el rastro no cuenta nada. 24 a 11 Hz ≈ 2 s de
/// historia visible — coincide con el horizonte que el ojo integra.
const TRAIL_CAP: usize = 24;

// ---------------------------------------------------------------------
// PRNG mínimo (LCG 64) — siembra reproducible sin dependencias.
// ---------------------------------------------------------------------

struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
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

/// Paleta retocada para que mar / tierra / cumbres se lean a primera
/// vista. Reemplaza la `Palette::default()` del render-plan en la app sin
/// tocar el crate (otros consumidores siguen con el default histórico).
fn bioma_palette() -> dominium_render_plan::Palette {
    dominium_render_plan::Palette {
        // Arena oscura para celdas sin capa dominante — visualmente
        // "tierra de borde" en lugar del gris-azulado original.
        floor: [0.30, 0.25, 0.20, 1.0],
        // Pasto firme.
        materia: [0.30, 0.62, 0.32, 1.0],
        // Azul océano profundo (sustituye al cian claro del default).
        psique: [0.16, 0.34, 0.66, 1.0],
        // Siena de cumbre (sustituye al rojo bandera).
        poder: [0.78, 0.52, 0.32, 1.0],
        oro: [0.92, 0.76, 0.28, 1.0],
        // Gris-violeta de roca alta (sustituye al violeta saturado).
        degradacion: [0.46, 0.40, 0.50, 1.0],
        // Marfil suave para lemmings — destaca sobre pasto y agua.
        lemming: [0.97, 0.95, 0.88, 1.0],
        concepto_aura: [0.95, 0.86, 0.55, 0.18],
        concepto_base: [0.58, 0.45, 0.18, 1.0],
        concepto: [0.98, 0.88, 0.42, 1.0],
        shadow: [0.04, 0.04, 0.06, 0.42],
    }
}

/// Parsea el pack JSON embebido. Si el JSON está malformado el binario
/// arranca con la colección vacía — la sim corre igual.
fn default_conceptos() -> Conceptos {
    serde_json::from_str::<Conceptos>(DEFAULT_PACK).unwrap_or_default()
}

/// Listado ordenado de packs embebidos disponibles en el picker. El primero
/// es el default; el ciclo es circular. Tupla `(id legible, JSON raw)`.
fn scenario_packs() -> [(&'static str, &'static str); 4] {
    [
        ("default", DEFAULT_PACK),
        ("andes", PACK_ANDES),
        ("mesopotamia", PACK_MESOPOTAMIA),
        ("capitalismo", PACK_CAPITALISMO),
    ]
}

/// Path absoluto al pack del usuario: `$XDG_CONFIG_HOME/dominium/pack.json`
/// (típicamente `~/.config/dominium/pack.json`). `None` si la plataforma
/// no expone un config dir.
fn user_pack_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "dominium")
        .map(|d| d.config_dir().join("pack.json"))
}

/// Escribe la colección actual al pack del usuario. Crea el directorio
/// padre si no existe. Errores van a stderr (la app no muere).
fn save_user_pack(cs: &Conceptos) {
    let Some(path) = user_pack_path() else {
        eprintln!("dominium · no hay ProjectDirs en esta plataforma");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("dominium · no pude crear {}: {e}", parent.display());
            return;
        }
    }
    match serde_json::to_string_pretty(cs) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => eprintln!("dominium · pack guardado en {}", path.display()),
            Err(e) => eprintln!("dominium · error escribiendo {}: {e}", path.display()),
        },
        Err(e) => eprintln!("dominium · error serializando pack: {e}"),
    }
}

/// Accede mutable al Concepto seleccionado, si lo hay.
fn selected_mut(m: &mut Model) -> Option<&mut Concepto> {
    let i = m.selected?;
    m.world.conceptos.items.get_mut(i)
}

/// Nombre humano de la acción atómica `0..5`.
fn action_name(b: u8) -> &'static str {
    match b {
        0 => "Mover",
        1 => "Extraer",
        2 => "Sincronizar",
        3 => "Intercambiar",
        4 => "Replicar",
        5 => "Degradar",
        _ => "?",
    }
}

/// Descripción del trigger para mostrar en el panel.
fn trigger_label(t: Trigger) -> String {
    match t {
        Trigger::Always => "Always".to_string(),
        Trigger::EnergiaBajo(v) => format!("EnergíaBajo({v:.0})"),
        Trigger::EdadSobre(v) => format!("EdadSobre({v})"),
    }
}

/// Agrega un Concepto en `(x, y)` (clamp al grid), lo nombra
/// `nuevo-N` y queda seleccionado para edición inmediata.
fn spawn_concepto_at(m: &mut Model, x: f32, y: f32) {
    let max = (GRID as f32) - 1.0;
    let n = m.world.conceptos.len();
    let new = Concepto {
        id: format!("nuevo-{}", n + 1),
        sprite_id: 0,
        pos_x: x.clamp(0.0, max),
        pos_y: y.clamp(0.0, max),
        radius: 4.0,
        mods: LayerMods::default(),
        hack: None,
        persuasion: None,
    };
    let i = m.world.conceptos.add(new);
    m.selected = Some(i);
}

/// Copia `ZWeights` (relieve visual) al array `[f32; 5]` que SimParams
/// usa como relieve físico, manteniendo el orden de capas del `Grid`.
fn mirror_zweights_to_relieve(
    z: &dominium_iso::ZWeights,
    relieve: &mut [f32; 5],
) {
    relieve[dominium_core::RELIEVE_MATERIA] = z.materia;
    relieve[dominium_core::RELIEVE_PSIQUE] = z.psique;
    relieve[dominium_core::RELIEVE_PODER] = z.poder;
    relieve[dominium_core::RELIEVE_ORO] = z.oro;
    relieve[dominium_core::RELIEVE_DEGRADACION] = z.degradacion;
}

/// Carga el pack del usuario si existe. Devuelve `None` si el archivo no
/// está, o si el contenido no es un `Conceptos` válido. Errores van a stderr.
fn load_user_pack() -> Option<Conceptos> {
    let path = user_pack_path()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<Conceptos>(&raw) {
        Ok(cs) => {
            eprintln!("dominium · pack cargado desde {}", path.display());
            Some(cs)
        }
        Err(e) => {
            eprintln!("dominium · {} corrupto: {e}", path.display());
            None
        }
    }
}

/// Siembra un mundo: continentes de materia, vetas de oro, niebla de
/// psique y una población de Lemmings con sesgos y acciones variadas.
/// Value noise multioctava determinista. Devuelve `Vec<f32>` de tamaño
/// `w*h` con valores aproximadamente en `[-1, 1]`. Las octavas suben en
/// frecuencia y bajan en amplitud — la primera define continentes, las
/// últimas, granulado. Smoothstep `s(t) = t²(3-2t)` entre celdas coarse.
fn fbm_noise(seed: u64, w: usize, h: usize) -> Vec<f32> {
    let mut rng = Lcg::new(seed);
    let mut field = vec![0.0_f32; w * h];
    // (frecuencia, amplitud). 4 octavas: 6×6 continentes → 96×96 ruido fino.
    let octaves: [(usize, f32); 4] = [(6, 1.0), (12, 0.55), (24, 0.30), (96, 0.18)];
    let mut amp_norm = 0.0_f32;
    for (_, a) in &octaves {
        amp_norm += a;
    }
    for (n, amp) in octaves {
        // Grilla coarse (n+1)×(n+1) de valores aleatorios en [-1, 1].
        let coarse_w = n + 1;
        let mut coarse = vec![0.0_f32; coarse_w * coarse_w];
        for v in coarse.iter_mut() {
            *v = rng.next_f32() * 2.0 - 1.0;
        }
        let sx = n as f32 / w as f32;
        let sy = n as f32 / h as f32;
        for y in 0..h {
            for x in 0..w {
                let fx = x as f32 * sx;
                let fy = y as f32 * sy;
                let cx = (fx.floor() as usize).min(n - 1);
                let cy = (fy.floor() as usize).min(n - 1);
                let tx = (fx - cx as f32).clamp(0.0, 1.0);
                let ty = (fy - cy as f32).clamp(0.0, 1.0);
                let smooth = |a: f32| a * a * (3.0 - 2.0 * a);
                let u = smooth(tx);
                let v = smooth(ty);
                let a = coarse[cy * coarse_w + cx];
                let b = coarse[cy * coarse_w + cx + 1];
                let c = coarse[(cy + 1) * coarse_w + cx];
                let d = coarse[(cy + 1) * coarse_w + cx + 1];
                let p = a * (1.0 - u) + b * u;
                let q = c * (1.0 - u) + d * u;
                field[y * w + x] += amp * (p * (1.0 - v) + q * v);
            }
        }
    }
    for v in field.iter_mut() {
        *v /= amp_norm;
    }
    field
}

/// Esculpe un río senoidal entre `(x0, y0)` y el borde opuesto, pintando
/// `psique` alta y limpiando `materia` a lo largo del trazo. El río tiene
/// ancho `width` celdas y serpentea con amplitud `wiggle` perpendicular al
/// rumbo. La curva se muestrea a paso unitario.
fn carve_river(w: &mut World, rng: &mut Lcg, vertical: bool, length: usize, width: f32, wiggle: f32) {
    let g_w = w.grid.width as f32;
    let g_h = w.grid.height as f32;
    let start = rng.next_f32() * if vertical { g_w } else { g_h };
    let phase = rng.next_f32() * core::f32::consts::TAU;
    let freq = 0.06 + rng.next_f32() * 0.05;
    for s in 0..length {
        let t = s as f32;
        let bend = libm::sinf(t * freq + phase) * wiggle;
        let (cx_f, cy_f) = if vertical {
            (start + bend, t * g_h / length as f32)
        } else {
            (t * g_w / length as f32, start + bend)
        };
        let r = width.ceil() as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let x = cx_f + dx as f32;
                let y = cy_f + dy as f32;
                if x < 0.0 || y < 0.0 || x >= g_w || y >= g_h {
                    continue;
                }
                let d = libm::sqrtf((dx as f32).powi(2) + (dy as f32).powi(2));
                if d > width {
                    continue;
                }
                let intensity = 1.0 - d / width;
                let idx = w.grid.idx(x as usize, y as usize);
                // Río = mucha psique (agua azul), nada de materia, sin oro.
                w.grid.psique[idx] = (w.grid.psique[idx] + 130.0 * intensity).min(180.0);
                w.grid.materia[idx] *= 1.0 - intensity * 0.95;
                w.grid.oro[idx] *= 1.0 - intensity * 0.8;
                w.grid.poder[idx] *= 1.0 - intensity * 0.8;
                w.grid.degradacion[idx] *= 1.0 - intensity * 0.9;
            }
        }
    }
}

fn seed(seed: u64) -> World {
    let mut w = World::new(GRID, GRID);
    let mut rng = Lcg::new(seed);
    // --- Capas iniciales basadas en dos campos fbm independientes ---
    // `elev` ∈ ~[-1, 1] decide bioma; `humid` ∈ ~[-1, 1] modula fertilidad.
    let elev = fbm_noise(seed ^ 0xE1E_7A57, GRID, GRID);
    let humid = fbm_noise(seed ^ 0x4D015_7CE, GRID, GRID);
    for cy in 0..GRID {
        for cx in 0..GRID {
            let idx = w.grid.idx(cx, cy);
            let e = elev[idx];
            let h = humid[idx];
            // Bordes del mapa empujados al mar para evitar continentes que
            // se peguen al borde — atenuación cosenoidal radial.
            let nx = (cx as f32 / GRID as f32) * 2.0 - 1.0;
            let ny = (cy as f32 / GRID as f32) * 2.0 - 1.0;
            let edge_drop = (nx * nx + ny * ny).min(1.0);
            let e = e - edge_drop * 0.35;

            if e < -0.18 {
                // Mar profundo: psique alta para que el azul aguante la
                // difusión lenta (entropy=0.005, diffusion=0.02 → unos cientos
                // de ticks antes de notarse erosión visual). Pintar también
                // `degradacion` baja persistente refuerza el tono frío y
                // ancla la celda como "no fértil" para los lemmings que la
                // crucen.
                w.grid.psique[idx] = 180.0 + rng.next_f32() * 30.0;
                w.grid.degradacion[idx] = 2.0;
            } else if e < -0.05 {
                // Mar somero / lagunas: agua más clara, mínima vida acuática.
                w.grid.psique[idx] = 110.0 + rng.next_f32() * 20.0;
                w.grid.materia[idx] = rng.next_f32() * 4.0;
                w.grid.degradacion[idx] = 1.0;
            } else if e < 0.08 {
                // Costa / pantano fértil: alta materia + algo de agua.
                w.grid.materia[idx] = 45.0 + (h.max(0.0)) * 30.0 + rng.next_f32() * 6.0;
                w.grid.psique[idx] = 18.0 + rng.next_f32() * 8.0;
                if rng.next_f32() > 0.94 {
                    w.grid.oro[idx] = rng.next_f32() * 18.0;
                }
            } else if e < 0.30 {
                // Llanura: el granero del mundo. Materia muy alta cuando
                // hay humedad; menos donde el clima es seco.
                let fertility = (h * 0.5 + 0.5).clamp(0.2, 1.0);
                w.grid.materia[idx] = 50.0 + fertility * 50.0 + rng.next_f32() * 5.0;
                if rng.next_f32() > 0.92 {
                    w.grid.oro[idx] = rng.next_f32() * 24.0;
                }
            } else if e < 0.50 {
                // Colinas: materia decreciente, asoma el poder (vetas).
                let alpha = (e - 0.30) / 0.20;
                w.grid.materia[idx] = (1.0 - alpha) * 35.0 + rng.next_f32() * 4.0;
                w.grid.poder[idx] = alpha * 9.0;
                if rng.next_f32() > 0.82 {
                    w.grid.oro[idx] = rng.next_f32() * 30.0; // minas en colinas
                }
            } else {
                // Montañas / picos: poco material vivo, mucha estructura
                // bruta (poder) y, en los más altos, cicatriz rocosa. Umbral
                // bajado de 0.60 a 0.50 — más superficie es cordillera, los
                // continentes se ven más "duros" y los picos abundan.
                let alpha = ((e - 0.50) / 0.50).clamp(0.0, 1.0);
                w.grid.poder[idx] = 6.0 + alpha * 18.0;
                w.grid.degradacion[idx] = 1.5 + alpha * alpha * 14.0;
                if rng.next_f32() > 0.97 {
                    w.grid.oro[idx] = rng.next_f32() * 35.0;
                }
            }
        }
    }
    // --- Ríos: 2 cruces. Uno vertical, uno horizontal. Sin erosión real
    //     — los ríos se pintan encima del bioma sobrescribiendo. ---
    carve_river(&mut w, &mut rng, true, GRID, 2.4, GRID as f32 * 0.18);
    carve_river(&mut w, &mut rng, false, GRID, 1.8, GRID as f32 * 0.14);

    // --- Lemmings: distribuidos solo en tierra firme (e ∈ [-0.05, 0.45]).
    //     Rechaza candidatos en mar o pico. Si tras 32 intentos no encuentra
    //     un punto válido, suelta donde caiga (failsafe para no congelar el
    //     seed). ---
    let pick_land = |rng: &mut Lcg, elev: &[f32]| -> (f32, f32) {
        for _ in 0..32 {
            let x = rng.next_f32() * (GRID as f32 - 1.0);
            let y = rng.next_f32() * (GRID as f32 - 1.0);
            let nx = (x / GRID as f32) * 2.0 - 1.0;
            let ny = (y / GRID as f32) * 2.0 - 1.0;
            let edge_drop = (nx * nx + ny * ny).min(1.0);
            let e = elev[(y as usize) * GRID + (x as usize)] - edge_drop * 0.35;
            if e > -0.02 && e < 0.40 {
                return (x, y);
            }
        }
        (
            rng.next_f32() * (GRID as f32 - 1.0),
            rng.next_f32() * (GRID as f32 - 1.0),
        )
    };
    for k in 0..LEMMINGS {
        let (x, y) = pick_land(&mut rng, &elev);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 40.0 + rng.next_f32() * 40.0, psi);
        // Distribución calibrada al punto fijo del sistema con herencia
        // de acción + intercambio fuerte (trade_amount = 1.5):
        //   α_e = 0.30 (Extraer · cosecha — fuente principal de E)
        //   α_t = 0.30 (Intercambiar · redistribución — evita concentración)
        //   α_m = 0.20 (Mover · exploración)
        //   α_r = 0.15 (Replicar · natalidad)
        //   α_s = 0.05 (Sincronizar · convergencia cultural)
        //
        // Balance energético por capita en equilibrio:
        //   dE/dt = α_e · e_r - α_m · c_m - α_r · f · E_r · 1[E_r>T]
        //         = 0.30·2.5 - 0.20·0.06 - 0.15·0.45·E_r
        //         = 0.738 - 0.0675·E_r
        //   E* = 0.738 / 0.0675 ≈ 11 (cerca del threshold T=12)
        // El sistema oscila alrededor de ese E*, replicando a baja
        // frecuencia pero sostenidamente.
        w.lemmings.accion[i] = match k % 20 {
            0..=5 => 1,            // 6/20 = 0.30 Extraer
            6..=11 => 3,           // 6/20 = 0.30 Intercambiar
            12..=15 => 0,          // 4/20 = 0.20 Mover
            16..=18 => 4,          // 3/20 = 0.15 Replicar
            _ => 2,                // 1/20 = 0.05 Sincronizar
        } as u8;
    }
    // Si el usuario ya tiene un pack guardado, gana sobre el embebido —
    // así sus ediciones sobreviven al reseed/reapertura. Si no, default.
    w.conceptos = load_user_pack().unwrap_or_else(default_conceptos);
    w
}

// ---------------------------------------------------------------------
// Modelo y bucle
// ---------------------------------------------------------------------

struct Model {
    world: World,
    params: SimParams,
    iso: IsoProjector,
    weights: ZWeights,
    cfg: PlanConfig,
    running: bool,
    tick: u64,
    epoch: u64,
    rng_seed: u64,
    /// Índice del Concepto seleccionado, si alguno. `None` cuando no hay
    /// selección. Si se "Limpia" la lista se resetea a `None`.
    selected: Option<usize>,
    /// Cuando está activo, editar `ZWeights` (relieve visual) también
    /// escribe a `params.relieve` (relieve físico) — lo que ves es lo
    /// que sienten los lemmings.
    sync_relieve: bool,
    /// Buffer de texto del input de renombre. `id_input_focused` decide
    /// si el panel muestra el text-input o el label estático.
    id_input: TextInputState,
    id_input_focused: bool,
    /// Índice del scenario actual en `scenario_packs()`. El picker del panel
    /// lo cicla; "Sembrar pack" instala el JSON correspondiente.
    scenario_idx: usize,
    /// Ring de snapshots del `World` — el último elemento es el más reciente
    /// ya cosechado. `rewind_offset == 0` significa "presente" y se renderiza
    /// `world`; `> 0` significa "mirar hacia atrás" y se renderiza
    /// `snapshots[len - 1 - offset]` en modo read-only.
    snapshots: VecDeque<World>,
    /// Cuántos pasos hacia atrás está mirando el usuario. `0` = vivo.
    /// Cuando `> 0`, el `Tick` deja de avanzar el mundo (la sim se
    /// auto-pausa visualmente, pero el reloj real podría seguir si se
    /// pidiera). Implementación: pausamos también el motor mientras hay
    /// rewind, así no acumula divergencia.
    rewind_offset: usize,
    /// Trails: para cada lemming vivo, las últimas `TRAIL_CAP` posiciones
    /// `(x, y)`. Como los lemmings se referencian por índice y `swap_remove`
    /// puede mover índices, el trail se reconstruye cada tick desde
    /// `lemmings.pos_x/pos_y` — sólo guardamos las posiciones, no su id.
    /// Estructura: `trails[k]` es el snapshot del frame `tick - k`.
    trails: VecDeque<Vec<(f32, f32)>>,
    /// Toggle para mostrar las trayectorias.
    show_trails: bool,
    /// Theme efectivo. Se construye en init desde `wawa-config` (con
    /// fallback a `Theme::dark()` si no hay archivo aún) y se rearma
    /// en cada `Msg::WawaConfigChanged`.
    theme: Theme,
    /// Subscripción al bus de configuración del SO. `Option` porque
    /// la creación puede fallar en plataformas sin ProjectDirs.
    /// Se mantiene viva mientras vive el `Model`.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
    /// Asignación k-means → cluster por lemming. Vacío hasta que se entre
    /// al modo PsiCluster o se ejecute el primer refresh. `assignments[i]`
    /// ∈ `0..KMEANS_K` indica el cluster del lemming `i`. Si la población
    /// cambia entre refrescos (spawn/kill), los índices nuevos caen en `0`
    /// hasta el próximo refresh.
    cluster_assignments: Vec<u8>,
    /// Tick global en el que se calculó por última vez `cluster_assignments`.
    /// Usado para gated refresh cada `KMEANS_REFRESH_TICKS`.
    cluster_last_refresh: u64,
    /// Cuál tab del panel lateral está activo. La UI muestra los grupos
    /// relevantes según esta selección — el modelo es simple, sin lazy load.
    panel_tab: PanelTab,
    /// Si el usuario ya entendió las gestures de canvas (click crea, drag
    /// mueve, segundo click selecciona). Cuando es `false` la app muestra
    /// un hint flotante sobre el canvas. Se apaga al primer click.
    onboarding_done: bool,
}

/// Pestañas del panel lateral. El orden es el orden visual en el tab bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelTab {
    Mundo,
    Conceptos,
    Psique,
    Vista,
}

impl PanelTab {
    fn label(self) -> &'static str {
        match self {
            PanelTab::Mundo => "Mundo",
            PanelTab::Conceptos => "Conceptos",
            PanelTab::Psique => "ψ",
            PanelTab::Vista => "Vista",
        }
    }

    fn all() -> [PanelTab; 4] {
        [
            PanelTab::Mundo,
            PanelTab::Conceptos,
            PanelTab::Psique,
            PanelTab::Vista,
        ]
    }
}

/// Una de las cuatro capas modificables de un `Concepto` (degradacion
/// queda fuera — es cicatriz emergente, no editable).
#[derive(Clone, Copy, Debug)]
enum Layer {
    Materia,
    Psique,
    Poder,
    Oro,
}

/// Slot de `SimParams` editable desde el panel. Los 4 más visibles más los
/// dos del ciclo estacional; los demás quedan al default.
#[derive(Clone, Copy, Debug)]
enum ParamSlot {
    ClimbCost,
    DiffusionRate,
    EntropyRate,
    MoveCost,
    SeasonPeriod,
    SeasonAmplitude,
    /// Intensidad con la que el psi modula los efectos de las acciones.
    PsiModulation,
    /// Radio social del contagio (Fase B).
    SocialRadius,
    /// Tasa de convergencia del contagio social.
    ContagionRate,
    /// Umbral de homofilia (Fase B.2) — 0 = sin filtro.
    HomophilyThreshold,
}

impl ParamSlot {
    fn range(self) -> (f32, f32) {
        match self {
            ParamSlot::ClimbCost => (0.0, 0.5),
            ParamSlot::DiffusionRate => (0.0, 0.5),
            ParamSlot::EntropyRate => (0.0, 0.05),
            ParamSlot::MoveCost => (0.0, 0.5),
            // 0 = sin estaciones; hasta 500 ticks por ciclo (≈45 s a 11 Hz).
            ParamSlot::SeasonPeriod => (0.0, 500.0),
            ParamSlot::SeasonAmplitude => (0.0, 1.0),
            // Psi modulation: rango [0, 1] de uso típico; > 1 amplifica
            // demasiado y rompe calibraciones del default.
            ParamSlot::PsiModulation => (0.0, 1.0),
            // Radio social — hasta media diagonal del grid 80×80.
            ParamSlot::SocialRadius => (0.0, 30.0),
            // Tasa de contagio: > 0.5 produce conformismo brutal en pocos
            // ticks; típicos 0.05..0.20.
            ParamSlot::ContagionRate => (0.0, 0.5),
            // Homofilia 0..2 — > sqrt(4) = 2 incluye todo el psi space.
            ParamSlot::HomophilyThreshold => (0.0, 2.0),
        }
    }
}

/// Capa de `ZWeights` editable desde el panel — define el **relieve
/// visual** (cuánto eleva cada capa el render). Independiente del
/// `relieve` físico de `SimParams`.
#[derive(Clone, Copy, Debug)]
enum ZSlot {
    Materia,
    Psique,
    Poder,
    Oro,
    Degradacion,
}


#[derive(Clone)]
enum Msg {
    Tick,
    TogglePlay,
    Reseed,
    LimpiarConceptos,
    SembrarConceptos,
    SelectConcepto(usize),
    DeselectConcepto,
    EditMod(Layer, f32),
    EditRadius(f32),
    DeleteSelected,
    EditParam(ParamSlot, f32),
    EditZWeight(ZSlot, f32),
    GuardarPack,
    CargarPack,
    CrearConcepto,
    /// Click sobre el canvas, en coords de mundo. Si cae sobre un
    /// Concepto existente lo selecciona; si no, crea uno nuevo ahí.
    CanvasClick(f32, f32),
    ToggleSyncRelieve,
    ToggleAndina,
    // Editor de BehaviorHack del Concepto seleccionado.
    HackToggle,         // agrega o quita el hack.
    HackCycleTrigger,   // rota Always → EnergiaBajo → EdadSobre → Always.
    HackCycleAction,    // rota la acción forzada 0..5 → 0...
    HackEditTriggerParam(f32),
    HackEditDuration(f32),
    CycleSprite,
    /// Delta de un Move dentro de un drag activo, en coords de mundo.
    /// Mueve el Concepto seleccionado si hay uno.
    CanvasDragMove(f32, f32),
    FocusIdInput,
    BlurIdInput,
    IdInputKey(KeyEvent),
    /// Cicla al siguiente scenario embebido. Sólo cambia la selección
    /// (no lo aplica hasta que se toque "Cargar scenario").
    CycleScenario,
    /// Reemplaza los conceptos del mundo con el scenario actualmente
    /// seleccionado. Limpia hack_locks vivos y deselecciona.
    LoadScenario,
    /// Cicla `cfg.render_mode`: Composite → Heatmap(Materia) → … →
    /// Heatmap(Degradacion) → Composite.
    CycleRenderMode,
    /// Toggle de visualización de trayectorias.
    ToggleTrails,
    /// Toggle de texturización procedural sobre los techos.
    ToggleTexture,
    /// Delta sobre `rewind_offset` (positivo = más atrás; negativo = hacia
    /// el presente). El slider del panel emite estos deltas; un botón
    /// "vivo" emite `RewindHome`.
    RewindBy(f32),
    /// Vuelve `rewind_offset` a 0 (presente).
    RewindHome,
    /// El bus `wawa-config` publicó una versión nueva. Aplicamos
    /// theme y locale; los demás campos no nos competen.
    WawaConfigChanged(Box<wawa_config::WawaConfig>),
    /// Alterna `big_five` en SimParams. Si la población vino sin columna
    /// `psi5` (saves Big Four), la rellenamos al pasar a Big5.
    ToggleBigFive,
    /// Cicla `ActionPolicy` entre Fixed y PsiArgmax. Con periodo 0 nunca
    /// re-elige, así que también arrancamos un período sano la primera vez.
    CyclePsiPolicy,
    /// Cambia el tab activo del panel lateral.
    SelectTab(PanelTab),
    /// Cierra el hint flotante de onboarding (se cierra solo en el primer
    /// click sobre el canvas, pero también hay una X visible).
    DismissOnboarding,
}

struct Dominium;

impl App for Dominium {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "dominium · campo medio (llimphi)"
    }

    fn initial_size() -> (u32, u32) {
        (1120, 720)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Loop de tick a ~11 Hz; el handle ya sabe cómo dejar morir
        // el thread cuando el event loop se cierre.
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);

        // Bus de configuración del SO. Theme y locale arrancan desde
        // el archivo si existe; el watcher reentra al `update` cuando
        // cambia.
        let wawa_cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&wawa_cfg, &Theme::dark());
        let _ = rimay_localize::set_locale(&wawa_cfg.lang);
        let handle_clone = handle.clone();
        let wawa_watcher = wawa_config::ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("dominium · wawa-config watcher: {e}"))
        .ok();

        let rng_seed = 0xD0_31_31_07;
        // SimParams con overrides puntuales para grilla grande + miles de
        // lemmings. La idea: los mares aguantan minutos sin difundirse a
        // verde, y la natalidad ya no explota exponencial. Mantiene la
        // termodinámica cerrada del motor — todo lo que toco son tasas.
        let params = SimParams {
            // Difusión y entropía bajas → la psique de los mares no se
            // empuja a tierra en pocos ticks. (Defaults son 0.1 / 0.01.)
            diffusion_rate: 0.02,
            entropy_rate: 0.004,
            // Regrowth limitado a la carga base de la llanura — sin esto
            // el regrowth llena de materia incluso los mares (que tienen
            // 0 inicial pero materia → carrying_capacity).
            regrowth_rate: 0.004,
            carrying_capacity: 40.0,
            // Frenos termodinámicos al crecimiento exponencial: cada
            // lemming drena más metabolismo, replica más tarde, y los
            // hijos arrancan con menos energía → ciclo madre→hijo más
            // costoso.
            metabolic_cost: 0.35,
            replicate_threshold: 35.0,
            child_energy_frac: 0.30,
            abundance_threshold: 55.0,
            ..SimParams::default()
        };
        Model {
            world: seed(rng_seed),
            params,
            // Scale 3.0 para que la grilla 240×240 entre en pantalla:
            // ancho ≈ 240·3·cos(30°) ≈ 624 px, deja espacio al panel y al
            // border. z_factor 0.35 da volumen claro: mares ~12 px hundidos,
            // picos ~9 px elevados.
            iso: IsoProjector::new(3.0, 0.35),
            // Relieve por bioma, recalibrado para los valores nuevos de
            // las capas (psique sube a 200 en mar, degradacion a ~16 en pico):
            //   - mares  → z ≈ -12 (psique 200 × −0.06)
            //   - llanura → z ≈ +2.4 (materia 80 × 0.03)
            //   - colinas → z ≈ +6 (poder 15 × 0.4)
            //   - picos  → z ≈ +9 (degradacion 14 × 0.6 + el resto)
            weights: ZWeights {
                materia: 0.03,
                psique: -0.06,
                poder: 0.40,
                oro: 0.0,
                degradacion: 0.60,
            },
            cfg: PlanConfig {
                tile: 3.0,
                lemming_size: 2.6,
                lemming_lift: 0.6,
                concepto_size: 7.0,
                concepto_lift: 2.0,
                light_dir: (0.55, 0.35),
                andina_layers: 0,
                andina_threshold: 1.0,
                palette: bioma_palette(),
                render_mode: RenderMode::Composite,
                // Textura procedural OFF por default: con miles de celdas,
                // los micro-quads empiezan a tapar la maqueta y el render
                // pierde claridad. El usuario lo prende en el tab Vista
                // si quiere "estampa".
                texture: false,
            },
            running: true,
            tick: 0,
            epoch: 0,
            rng_seed,
            selected: None,
            sync_relieve: false,
            id_input: TextInputState::new(),
            id_input_focused: false,
            scenario_idx: 0,
            snapshots: VecDeque::with_capacity(SNAPSHOT_RING_CAP),
            rewind_offset: 0,
            trails: VecDeque::with_capacity(TRAIL_CAP),
            show_trails: false,
            theme,
            _wawa_watcher: wawa_watcher,
            cluster_assignments: Vec::new(),
            cluster_last_refresh: 0,
            panel_tab: PanelTab::Mundo,
            onboarding_done: false,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                // Si el usuario está revisando el pasado, la sim queda
                // congelada para no acumular divergencia con el ring.
                if m.running && m.rewind_offset == 0 {
                    advance(&mut m);
                }
            }
            Msg::TogglePlay => {
                m.running = !m.running;
            }
            Msg::Reseed => {
                reseed(&mut m);
            }
            Msg::LimpiarConceptos => {
                m.world.conceptos.clear();
                // Romper los hack_locks vivos: sin Concepto que los sostenga,
                // los lemmings vuelven a la lógica normal.
                for lock in m.world.lemmings.hack_lock.iter_mut() {
                    *lock = 0;
                }
                m.selected = None;
            }
            Msg::SembrarConceptos => {
                m.world.conceptos = default_conceptos();
                m.selected = None;
            }
            Msg::SelectConcepto(i) => {
                if i < m.world.conceptos.len() {
                    m.selected = Some(i);
                }
            }
            Msg::DeselectConcepto => m.selected = None,
            Msg::EditMod(layer, dv) => {
                if let Some(i) = m.selected {
                    if let Some(c) = m.world.conceptos.items.get_mut(i) {
                        let slot = match layer {
                            Layer::Materia => &mut c.mods.materia,
                            Layer::Psique => &mut c.mods.psique,
                            Layer::Poder => &mut c.mods.poder,
                            Layer::Oro => &mut c.mods.oro,
                        };
                        *slot = (*slot + dv).clamp(-1.0, 1.0);
                    }
                }
            }
            Msg::EditRadius(dv) => {
                if let Some(i) = m.selected {
                    if let Some(c) = m.world.conceptos.items.get_mut(i) {
                        c.radius = (c.radius + dv).clamp(0.5, 20.0);
                    }
                }
            }
            Msg::DeleteSelected => {
                if let Some(i) = m.selected.take() {
                    if i < m.world.conceptos.len() {
                        m.world.conceptos.remove(i);
                        for lock in m.world.lemmings.hack_lock.iter_mut() {
                            *lock = 0;
                        }
                    }
                }
            }
            Msg::EditParam(slot, dv) => {
                let (lo, hi) = slot.range();
                match slot {
                    ParamSlot::ClimbCost => {
                        m.params.climb_cost = (m.params.climb_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::DiffusionRate => {
                        m.params.diffusion_rate =
                            (m.params.diffusion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::EntropyRate => {
                        m.params.entropy_rate = (m.params.entropy_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::MoveCost => {
                        m.params.move_cost = (m.params.move_cost + dv).clamp(lo, hi)
                    }
                    ParamSlot::SeasonPeriod => {
                        let v = (m.params.season_period as f32 + dv).clamp(lo, hi);
                        m.params.season_period = v as u32;
                    }
                    ParamSlot::SeasonAmplitude => {
                        m.params.season_amplitude =
                            (m.params.season_amplitude + dv).clamp(lo, hi)
                    }
                    ParamSlot::PsiModulation => {
                        m.params.psi_effect_modulation =
                            (m.params.psi_effect_modulation + dv).clamp(lo, hi)
                    }
                    ParamSlot::SocialRadius => {
                        m.params.social_radius =
                            (m.params.social_radius + dv).clamp(lo, hi)
                    }
                    ParamSlot::ContagionRate => {
                        m.params.contagion_rate =
                            (m.params.contagion_rate + dv).clamp(lo, hi)
                    }
                    ParamSlot::HomophilyThreshold => {
                        m.params.homophily_threshold =
                            (m.params.homophily_threshold + dv).clamp(lo, hi)
                    }
                }
            }
            Msg::EditZWeight(slot, dv) => {
                let s = match slot {
                    ZSlot::Materia => &mut m.weights.materia,
                    ZSlot::Psique => &mut m.weights.psique,
                    ZSlot::Poder => &mut m.weights.poder,
                    ZSlot::Oro => &mut m.weights.oro,
                    ZSlot::Degradacion => &mut m.weights.degradacion,
                };
                *s = (*s + dv).clamp(-2.0, 2.0);
                if m.sync_relieve {
                    mirror_zweights_to_relieve(&m.weights, &mut m.params.relieve);
                }
            }
            Msg::GuardarPack => save_user_pack(&m.world.conceptos),
            Msg::CargarPack => {
                if let Some(cs) = load_user_pack() {
                    m.world.conceptos = cs;
                    for lock in m.world.lemmings.hack_lock.iter_mut() {
                        *lock = 0;
                    }
                    m.selected = None;
                }
            }
            Msg::CrearConcepto => {
                let center = (GRID as f32) * 0.5;
                spawn_concepto_at(&mut m, center, center);
            }
            Msg::CanvasClick(wx, wy) => {
                // Primer click sobre el canvas también apaga el hint de
                // onboarding — si llegó hasta acá, ya entendió que se
                // puede interactuar con el mapa.
                m.onboarding_done = true;
                // Hit-test contra Conceptos existentes (centro + radio
                // pickeable acotado). Si pega, selecciona sin crear; si
                // no, crea un Concepto nuevo ahí.
                let mut hit: Option<usize> = None;
                for (i, c) in m.world.conceptos.items.iter().enumerate() {
                    let dx = wx - c.pos_x;
                    let dy = wy - c.pos_y;
                    let pick_r = c.radius.min(3.0);
                    if dx * dx + dy * dy <= pick_r * pick_r {
                        hit = Some(i);
                        break;
                    }
                }
                match hit {
                    Some(i) => m.selected = Some(i),
                    None => spawn_concepto_at(&mut m, wx, wy),
                }
            }
            Msg::ToggleSyncRelieve => {
                m.sync_relieve = !m.sync_relieve;
                if m.sync_relieve {
                    mirror_zweights_to_relieve(&m.weights, &mut m.params.relieve);
                }
            }
            Msg::ToggleAndina => {
                // 0 ↔ 3 capas. El threshold no cambia.
                m.cfg.andina_layers = if m.cfg.andina_layers == 0 { 3 } else { 0 };
            }
            Msg::HackToggle => {
                if let Some(c) = selected_mut(&mut m) {
                    c.hack = match c.hack {
                        Some(_) => None,
                        None => Some(BehaviorHack {
                            trigger: Trigger::Always,
                            forced_action: 2, // Sincronizar — el default más visible
                            duration: 30,
                        }),
                    };
                }
            }
            Msg::HackCycleTrigger => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.trigger = match h.trigger {
                            Trigger::Always => Trigger::EnergiaBajo(15.0),
                            Trigger::EnergiaBajo(_) => Trigger::EdadSobre(100),
                            Trigger::EdadSobre(_) => Trigger::Always,
                        };
                    }
                }
            }
            Msg::HackCycleAction => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.forced_action = (h.forced_action + 1) % 6;
                    }
                }
            }
            Msg::HackEditTriggerParam(dv) => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        h.trigger = match h.trigger {
                            Trigger::Always => Trigger::Always,
                            Trigger::EnergiaBajo(v) => {
                                Trigger::EnergiaBajo((v + dv).clamp(0.0, 100.0))
                            }
                            Trigger::EdadSobre(v) => {
                                let next = (v as f32 + dv).clamp(0.0, 1000.0);
                                Trigger::EdadSobre(next as u32)
                            }
                        };
                    }
                }
            }
            Msg::HackEditDuration(dv) => {
                if let Some(c) = selected_mut(&mut m) {
                    if let Some(h) = c.hack.as_mut() {
                        let next = (h.duration as f32 + dv).clamp(1.0, 500.0);
                        h.duration = next as u32;
                    }
                }
            }
            Msg::CycleSprite => {
                if let Some(c) = selected_mut(&mut m) {
                    // 0 (sin glifo) → 1..=SPRITE_COUNT → 0 ...
                    c.sprite_id = (c.sprite_id + 1) % (dominium_render_plan::SPRITE_COUNT + 1);
                }
            }
            Msg::CanvasDragMove(dwx, dwy) => {
                if let Some(c) = selected_mut(&mut m) {
                    let max = (GRID as f32) - 1.0;
                    c.pos_x = (c.pos_x + dwx).clamp(0.0, max);
                    c.pos_y = (c.pos_y + dwy).clamp(0.0, max);
                }
            }
            Msg::FocusIdInput => {
                if let Some(c) = m.selected.and_then(|i| m.world.conceptos.items.get(i)) {
                    m.id_input.set_text(c.id.clone());
                    m.id_input_focused = true;
                }
            }
            Msg::BlurIdInput => {
                m.id_input_focused = false;
            }
            Msg::IdInputKey(ev) => {
                if m.id_input_focused && m.id_input.apply_key(&ev) {
                    let new_id = m.id_input.text().to_string();
                    if let Some(c) = selected_mut(&mut m) {
                        c.id = new_id;
                    }
                }
            }
            Msg::CycleScenario => {
                let n = scenario_packs().len();
                m.scenario_idx = (m.scenario_idx + 1) % n;
            }
            Msg::LoadScenario => {
                let packs = scenario_packs();
                let (_, json) = packs[m.scenario_idx];
                if let Ok(cs) = serde_json::from_str::<Conceptos>(json) {
                    m.world.conceptos = cs;
                    for lock in m.world.lemmings.hack_lock.iter_mut() {
                        *lock = 0;
                    }
                    m.selected = None;
                }
            }
            Msg::CycleRenderMode => {
                m.cfg.render_mode = match m.cfg.render_mode {
                    RenderMode::Composite => RenderMode::Heatmap(RenderLayer::Materia),
                    RenderMode::Heatmap(RenderLayer::Degradacion) => RenderMode::PsiCluster,
                    RenderMode::Heatmap(l) => RenderMode::Heatmap(l.next()),
                    RenderMode::PsiCluster => RenderMode::Composite,
                };
                // Forzar refresh inmediato del k-means al entrar al modo.
                if matches!(m.cfg.render_mode, RenderMode::PsiCluster) {
                    refresh_clusters(&mut m);
                }
            }
            Msg::ToggleTrails => {
                m.show_trails = !m.show_trails;
            }
            Msg::ToggleTexture => {
                m.cfg.texture = !m.cfg.texture;
            }
            Msg::RewindBy(dv) => {
                let cap = m.snapshots.len().saturating_sub(1);
                let cur = m.rewind_offset as f32;
                let next = (cur + dv).clamp(0.0, cap as f32);
                m.rewind_offset = next as usize;
            }
            Msg::RewindHome => {
                m.rewind_offset = 0;
            }
            Msg::ToggleBigFive => {
                m.params.big_five = !m.params.big_five;
                if m.params.big_five {
                    // Saves Big Four que entraron sin columna psi5 hay que
                    // rellenarlos antes de que el motor consulte
                    // `lemmings.psi5[i]`.
                    m.world.lemmings.ensure_psi5_len();
                }
            }
            Msg::CyclePsiPolicy => {
                m.params.action_policy = match m.params.action_policy {
                    dominium_core::ActionPolicy::Fixed => {
                        if m.params.policy_reeval_period == 0 {
                            m.params.policy_reeval_period = 20;
                        }
                        dominium_core::ActionPolicy::PsiArgmax
                    }
                    dominium_core::ActionPolicy::PsiArgmax => {
                        dominium_core::ActionPolicy::Fixed
                    }
                };
            }
            Msg::WawaConfigChanged(cfg) => {
                // Re-armamos el theme y el locale. El locale lo respeta
                // el próximo `view()` porque `rimay_localize::t(...)` se
                // re-llama cada frame.
                m.theme = theme_from_wawa(&cfg, &m.theme);
                if cfg.lang != rimay_localize::current_locale() {
                    let _ = rimay_localize::set_locale(&cfg.lang);
                }
            }
            Msg::SelectTab(tab) => {
                m.panel_tab = tab;
            }
            Msg::DismissOnboarding => {
                m.onboarding_done = true;
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if !model.id_input_focused {
            return None;
        }
        // Enter o Escape → cerrar la edición.
        if event.state == KeyState::Pressed {
            match &event.key {
                Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Escape) => {
                    return Some(Msg::BlurIdInput);
                }
                _ => {}
            }
        }
        Some(Msg::IdInputKey(event.clone()))
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = model.theme;
        let shown = displayed_world(model);
        let stats = WorldStats::from_world(shown);

        let status = status_bar(model, &theme);
        // PsiMetrics es O(N²) por Moran — para N≈500 son ~250k operaciones
        // por frame a 11 Hz, perfectamente costeable y nos da las métricas
        // psicológicas en vivo sin un segundo bucle de cálculo.
        let psi_metrics = PsiMetrics::from_world(shown);
        let mut plan = build_plan_with_overrides(
            shown,
            &model.iso,
            &model.weights,
            &model.cfg,
            |i| lemming_color_for(model, i),
        );
        if model.show_trails && model.rewind_offset == 0 {
            overlay_trails(&mut plan, model);
        }
        let plan_cx = (plan.min_x + plan.max_x) * 0.5;
        let plan_cy = (plan.min_y + plan.max_y) * 0.5;
        let iso = model.iso;
        let canvas = canvas_pane(plan)
            .on_click_at(move |lx, ly, rw, rh| {
                // Mapeo inverso al que aplica canvas-llimphi para centrar la maqueta:
                //   plan_pos = local - rect/2 + plan_center
                let plan_x = lx - rw * 0.5 + plan_cx;
                let plan_y = ly - rh * 0.5 + plan_cy;
                let (wx, wy) = iso.unproject_floor(plan_x, plan_y);
                let max = (GRID as f32) - 1.0;
                if wx >= 0.0 && wx <= max && wy >= 0.0 && wy <= max {
                    Some(Msg::CanvasClick(wx, wy))
                } else {
                    None
                }
            })
            .draggable_at(move |phase, dx, dy, _lx0, _ly0| match phase {
                DragPhase::Move => {
                    // La inversa iso es lineal → unproject(dx, dy) = delta de mundo.
                    let (wdx, wdy) = iso.unproject_floor(dx, dy);
                    if wdx == 0.0 && wdy == 0.0 {
                        None
                    } else {
                        Some(Msg::CanvasDragMove(wdx, wdy))
                    }
                }
                DragPhase::End => None,
            });
        let side = side_panel(model, &stats, &psi_metrics, &theme);

        let body = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            min_size: Size {
                width: length(0.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![canvas, side]);

        let mut frame: Vec<View<Msg>> = vec![status];
        if !model.onboarding_done {
            frame.push(onboarding_bar(&theme));
        }
        frame.push(body);
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(frame)
    }
}

/// Banda informativa que cubre el ancho de la app y explica las tres
/// gestures básicas del canvas. Se muestra hasta que el usuario haga el
/// primer click (que también es la gesture más obvia). Tiene una X a la
/// derecha para cerrarla manualmente sin tocar el canvas.
fn onboarding_bar(theme: &Theme) -> View<Msg> {
    let hint_text = "Click vacío → crea concepto · Click sobre uno → selecciona · Drag → mover · Tabs arriba a la derecha";
    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(hint_text, 11.5, theme.accent, Alignment::Start);
    let close_btn = View::new(Style {
        size: Size {
            width: length(28.0_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![llimphi_widget_button::button_view::<Msg>(
        "✕",
        &ButtonPalette::from_theme(theme),
        Msg::DismissOnboarding,
    )]);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(14.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![label, close_btn])
}

use wawa_config_llimphi::theme_from_wawa;

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Dominium>();
}

// ---------------------------------------------------------------------
// Transiciones
// ---------------------------------------------------------------------

/// Un paso de simulación; re-siembra si la población colapsa. Captura
/// también el snapshot del estado y el frame de trails (después de avanzar,
/// así el "presente" siempre coincide con `world`).
fn advance(m: &mut Model) {
    tick(&mut m.world, &m.params);
    m.tick += 1;
    if m.world.lemmings.is_empty() {
        m.epoch += 1;
        m.rng_seed = m
            .rng_seed
            .wrapping_mul(2862933555777941757)
            .wrapping_add(1);
        m.world = seed(m.rng_seed);
        m.tick = 0;
        m.snapshots.clear();
        m.trails.clear();
        m.cluster_assignments.clear();
    }
    push_snapshot(m);
    push_trail_frame(m);
    // K-means de psi: sólo cuando el render lo necesita. Si el usuario
    // está en otro modo, no pagamos el costo.
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster)
        && m.tick.saturating_sub(m.cluster_last_refresh) >= KMEANS_REFRESH_TICKS
    {
        refresh_clusters(m);
    }
}

/// Tres colores fijos del paleta de clusters — orden de aparición en el
/// resultado de `kmeans_psi`. Magenta / cian / amarillo: los más fáciles de
/// distinguir sobre cualquier fondo de bioma.
const CLUSTER_COLORS: [Color; 3] = [
    [0.96, 0.30, 0.72, 1.0], // magenta
    [0.30, 0.90, 0.90, 1.0], // cian
    [0.96, 0.92, 0.30, 1.0], // amarillo
];

/// Recalcula `cluster_assignments` desde el `World` actual y deja el tick
/// como timestamp del refresh. Si `kmeans_psi` devuelve `None` (pob < K),
/// limpia las asignaciones para que los lemmings caigan al color default.
fn refresh_clusters(m: &mut Model) {
    if let Some(km) = kmeans_psi(&m.world) {
        m.cluster_assignments = km.assignments;
    } else {
        m.cluster_assignments.clear();
    }
    m.cluster_last_refresh = m.tick;
}

/// Color para el lemming `i` según el `RenderMode` actual y las
/// asignaciones de cluster vigentes. Se usa como override de
/// `build_plan_with_overrides`.
fn lemming_color_for(m: &Model, i: usize) -> Color {
    if matches!(m.cfg.render_mode, RenderMode::PsiCluster)
        && i < m.cluster_assignments.len()
    {
        let c = m.cluster_assignments[i] as usize;
        if c < CLUSTER_COLORS.len() {
            return CLUSTER_COLORS[c];
        }
    }
    m.cfg.palette.lemming
}

fn reseed(m: &mut Model) {
    m.rng_seed = m.rng_seed.wrapping_add(0x9E37_79B9);
    m.world = seed(m.rng_seed);
    m.tick = 0;
    m.epoch += 1;
    m.snapshots.clear();
    m.trails.clear();
    m.rewind_offset = 0;
}

/// Empuja el `World` actual al ring (clone barato: SoA + Vec). Drop del más
/// viejo al exceder la capacidad.
fn push_snapshot(m: &mut Model) {
    if m.snapshots.len() == SNAPSHOT_RING_CAP {
        m.snapshots.pop_front();
    }
    m.snapshots.push_back(m.world.clone());
}

/// Empuja el frame de posiciones de todos los lemmings vivos al ring.
fn push_trail_frame(m: &mut Model) {
    if m.trails.len() == TRAIL_CAP {
        m.trails.pop_front();
    }
    let lem = &m.world.lemmings;
    let frame: Vec<(f32, f32)> = (0..lem.len())
        .map(|i| (lem.pos_x[i], lem.pos_y[i]))
        .collect();
    m.trails.push_back(frame);
}

/// Devuelve el `World` que actualmente se está mostrando — el presente
/// (`world`) si no hay rewind, o el snapshot apropiado si lo hay.
fn displayed_world(m: &Model) -> &World {
    if m.rewind_offset == 0 || m.snapshots.is_empty() {
        &m.world
    } else {
        let len = m.snapshots.len();
        let idx = len.saturating_sub(1 + m.rewind_offset);
        &m.snapshots[idx]
    }
}

/// Pinta las posiciones históricas de los lemmings como quads diminutos
/// con alpha decreciente — los más viejos casi transparentes. Va después
/// del `build_plan` para que los trails queden por encima del suelo pero
/// por debajo del HUD; depth pequeño constante negativo para no romper el
/// orden de pintor de las celdas.
///
/// Se llama sólo en vivo (no en rewind), porque en rewind el `World` que
/// se renderiza no necesariamente tiene los mismos índices de lemming que
/// el frame de trails — y mezclarlos confundiría al ojo más que ayudar.
fn overlay_trails(plan: &mut RenderPlan, m: &Model) {
    let n_frames = m.trails.len();
    if n_frames == 0 {
        return;
    }
    let lemming_color = m.cfg.palette.lemming;
    // Tamaño de la moteta: la mitad del marker del lemming, así no compite
    // visualmente con la posición actual.
    let size = m.cfg.lemming_size * 0.45;
    for (k, frame) in m.trails.iter().enumerate() {
        // k=0 es el más viejo → alpha bajo; k=n-1 el más nuevo → alpha alto.
        // No incluyo el último frame: ya está pintado por el lemming actual.
        if k + 1 == n_frames {
            break;
        }
        let t = (k + 1) as f32 / n_frames as f32; // ∈ (0, 1)
        let alpha = 0.10 + 0.40 * t;
        let color: Color = [
            lemming_color[0],
            lemming_color[1],
            lemming_color[2],
            alpha,
        ];
        for &(x, y) in frame {
            let (sx, sy) = m.iso.project(x, y, m.cfg.lemming_lift * 0.5);
            plan.quads.push(Quad {
                x: sx - size * 0.5,
                y: sy - size * 0.5,
                w: size,
                h: size,
                color,
                // Detrás de los Lemmings vivos (que pintan a depth ≈ x+y+0.5)
                // pero delante de la celda (depth x+y).
                depth: x + y + 0.25,
            });
        }
    }
    // Mantengo el plan ordenado: insert al final desordena. Re-ordeno por
    // depth — coste O(N log N) pero N es del orden de 50·24 = 1200 quads.
    plan.quads.sort_by(|a, b| {
        a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal)
    });
    // Re-extender la bounding box por si los trails caen fuera.
    for q in &plan.quads {
        plan.min_x = plan.min_x.min(q.x);
        plan.min_y = plan.min_y.min(q.y);
        plan.max_x = plan.max_x.max(q.x + q.w);
        plan.max_y = plan.max_y.max(q.y + q.h);
    }
}

// ---------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------

fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let estado = rimay_localize::t(if model.running {
        "dominium-status-running"
    } else {
        "dominium-status-paused"
    });
    // Texto principal: tamaño · población · epoch · tick. El usuario lo
    // ve siempre, sin importar el tab del panel.
    let line = format!(
        "{}×{}  ·  pob {}  ·  epoch {}  ·  tick {}",
        GRID,
        GRID,
        model.world.lemmings.len(),
        model.epoch,
        model.tick,
    );
    let label_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(line, 12.0, theme.fg_text, Alignment::Start);
    let estado_view = View::new(Style {
        size: Size {
            width: length(120.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(estado, 12.0, theme.accent, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![label_view, estado_view])
}

fn canvas_pane(plan: dominium_render_plan::RenderPlan) -> View<Msg> {
    let canvas_bg = llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(11, 13, 18, 255);
    let canvas = canvas_view::<Msg>(plan, Some(canvas_bg));
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .clip(true)
    .children(vec![canvas])
}

fn side_panel(
    model: &Model,
    stats: &WorldStats,
    psi_metrics: &PsiMetrics,
    theme: &Theme,
) -> View<Msg> {
    let btn_palette = ButtonPalette::from_theme(theme);
    let mut slider_palette = SliderPalette::from_theme(theme);
    // Comprimimos los slots para que entren en el sidebar de 240 px.
    slider_palette.label_width = 56.0;
    slider_palette.track_width = 90.0;
    slider_palette.value_width = 44.0;

    let header = label_view(&rimay_localize::t("dominium-header-sim"), 11.0, theme.fg_muted);

    let play_label = rimay_localize::t(if model.running {
        "dominium-btn-pause"
    } else {
        "dominium-btn-resume"
    });
    let play_btn = sized_button(&play_label, &btn_palette, Msg::TogglePlay);
    let reset_btn = sized_button(
        &rimay_localize::t("dominium-btn-reseed"),
        &btn_palette,
        Msg::Reseed,
    );

    // --- Tab bar: 4 pestañas chiquitas en fila ---
    let tab_bar = tab_bar_view(model, &btn_palette, theme);

    // Header siempre visible: play/pause + reseed (los controles más usados,
    // independientes del tab).
    let mut children: Vec<View<Msg>> = vec![
        header,
        tab_bar,
        play_btn,
        reset_btn,
        separator(theme),
    ];

    // Contenido específico del tab actual.
    match model.panel_tab {
        PanelTab::Mundo => append_mundo_tab(&mut children, model, stats, theme, &btn_palette, &slider_palette),
        PanelTab::Conceptos => append_conceptos_tab(&mut children, model, theme, &btn_palette, &slider_palette),
        PanelTab::Psique => append_psique_tab(&mut children, model, stats, psi_metrics, theme, &btn_palette, &slider_palette),
        PanelTab::Vista => append_vista_tab(&mut children, model, theme, &btn_palette, &slider_palette),
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(SIDE_WIDTH),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(14.0_f32),
            bottom: length(14.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

/// Línea horizontal de 1 px usada como separator entre secciones del panel.
fn separator(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
}

/// Barra horizontal con un botón por cada `PanelTab`. El tab activo se
/// resalta cambiando el `accent` del label (botón) — la palette de Llimphi
/// no expone "tab pill", así que usamos la convención de marcar el activo
/// con `▸`.
fn tab_bar_view(model: &Model, btn_palette: &ButtonPalette, _theme: &Theme) -> View<Msg> {
    let buttons: Vec<View<Msg>> = PanelTab::all()
        .into_iter()
        .map(|tab| {
            let active = tab == model.panel_tab;
            let label = if active {
                format!("▸ {}", tab.label())
            } else {
                tab.label().to_string()
            };
            let mut bp = btn_palette.clone();
            if active {
                bp.bg = btn_palette.bg_hover;
            }
            View::new(Style {
                size: Size {
                    width: Dimension::auto(),
                    height: length(26.0_f32),
                },
                flex_grow: 1.0,
                flex_basis: length(0.0_f32),
                ..Default::default()
            })
            .children(vec![llimphi_widget_button::button_view::<Msg>(
                &label,
                &bp,
                Msg::SelectTab(tab),
            )])
        })
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(buttons)
}

/// Tab "Mundo" — estado macro + sliders de motor + scenario picker.
fn append_mundo_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    stats: &WorldStats,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view(
        &rimay_localize::t("dominium-header-metricas"),
        11.0,
        theme.fg_muted,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-population"),
        &stats.n.to_string(),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-epoca"),
        Epoch::classify(stats).label(),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-materia"),
        &format!("{:.0}", stats.total_materia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-oro"),
        &format!("{:.0}", stats.total_oro),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-energia"),
        &format!("{:.0}", stats.total_energia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-gini-energia"),
        &format!("{:.3}", stats.gini_energia),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-edad-media"),
        &format!("{:.1}", stats.mean_edad),
        theme,
    ));
    children.push(stat_row(
        "season×",
        &format!("{:.2}", model.params.season_factor(model.world.tick_count)),
        theme,
    ));

    children.push(separator(theme));
    children.push(label_view("[ ACCIONES ACTUALES ]", 11.0, theme.fg_muted));
    let action_labels: [(&str, usize); 6] = [
        ("dominium-action-mover", 0),
        ("dominium-action-extraer", 1),
        ("dominium-action-sincronizar", 2),
        ("dominium-action-intercambiar", 3),
        ("dominium-action-replicar", 4),
        ("dominium-action-degradar", 5),
    ];
    for (key, ai) in action_labels {
        children.push(stat_row(
            &rimay_localize::t(key),
            &stats.action_counts[ai].to_string(),
            theme,
        ));
    }

    children.push(separator(theme));
    children.push(label_view("[ MOTOR ]", 11.0, theme.fg_muted));
    children.push(param_slider("climb", model.params.climb_cost, ParamSlot::ClimbCost, slider_palette));
    children.push(param_slider("move", model.params.move_cost, ParamSlot::MoveCost, slider_palette));
    children.push(param_slider("diffuse", model.params.diffusion_rate, ParamSlot::DiffusionRate, slider_palette));
    children.push(param_slider("entropy", model.params.entropy_rate, ParamSlot::EntropyRate, slider_palette));
    children.push(param_slider("season T", model.params.season_period as f32, ParamSlot::SeasonPeriod, slider_palette));
    children.push(param_slider("season A", model.params.season_amplitude, ParamSlot::SeasonAmplitude, slider_palette));

    children.push(separator(theme));
    children.push(label_view("[ SCENARIO ]", 11.0, theme.fg_muted));
    let packs = scenario_packs();
    let (current_id, _) = packs[model.scenario_idx];
    children.push(sized_button(
        &format!("pack: {} (▸ ciclar)", current_id),
        btn_palette,
        Msg::CycleScenario,
    ));
    children.push(sized_button(
        &rimay_localize::t_args("dominium-btn-load-named", &[("name", current_id.into())]),
        btn_palette,
        Msg::LoadScenario,
    ));
    children.push(separator(theme));
    children.push(label_view(&format!("grilla {GRID}×{GRID}"), 11.0, theme.fg_muted));
}

/// Tab "Conceptos" — lista de conceptos, crear/cargar/guardar/limpiar,
/// y el editor del Concepto seleccionado (radius, sprite, 4 mods, hack).
fn append_conceptos_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view(
        &rimay_localize::t("dominium-header-conceptos"),
        11.0,
        theme.fg_muted,
    ));
    children.push(label_view(
        &rimay_localize::t_args(
            "dominium-active-count",
            &[("count", model.world.conceptos.len().to_string().into())],
        ),
        12.0,
        theme.fg_text,
    ));

    // Hint contextual: si no hay conceptos, le decimos cómo crear uno.
    if model.world.conceptos.items.is_empty() {
        children.push(label_view(
            "Click sobre el mapa para crear",
            11.0,
            theme.fg_muted,
        ));
    }

    for (i, c) in model.world.conceptos.items.iter().enumerate() {
        children.push(concepto_row(i, &c.id, model.selected == Some(i), theme));
    }
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-create-concept"),
        btn_palette,
        Msg::CrearConcepto,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-seed-pack"),
        btn_palette,
        Msg::SembrarConceptos,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-clear"),
        btn_palette,
        Msg::LimpiarConceptos,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-save"),
        btn_palette,
        Msg::GuardarPack,
    ));
    children.push(sized_button(
        &rimay_localize::t("dominium-btn-load-saved"),
        btn_palette,
        Msg::CargarPack,
    ));

    // Editor del seleccionado.
    let Some(i) = model.selected else { return };
    let Some(c) = model.world.conceptos.items.get(i) else { return };
    children.push(separator(theme));
    children.push(label_view(
        &rimay_localize::t("dominium-header-editar"),
        11.0,
        theme.fg_muted,
    ));
    if model.id_input_focused {
        children.push(text_input_view(
            &model.id_input,
            &rimay_localize::t("dominium-slider-nombre"),
            true,
            &TextInputPalette::from_theme(theme),
            Msg::FocusIdInput,
        ));
    } else {
        children.push(sized_button(
            &format!("• {}  (✎ renombrar)", c.id),
            btn_palette,
            Msg::FocusIdInput,
        ));
    }
    children.push(slider_view(
        &rimay_localize::t("dominium-slider-radius"),
        c.radius,
        0.5,
        20.0,
        slider_palette,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditRadius(dv)),
            DragPhase::End => None,
        },
    ));
    let sprite_glyph = dominium_render_plan::glyph_for_sprite(c.sprite_id)
        .map(|c| c.to_string())
        .unwrap_or_else(|| "—".to_string());
    children.push(sized_button(
        &format!("sprite: {} ({})", c.sprite_id, sprite_glyph),
        btn_palette,
        Msg::CycleSprite,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-materia"),
        c.mods.materia,
        Layer::Materia,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-psique"),
        c.mods.psique,
        Layer::Psique,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-poder"),
        c.mods.poder,
        Layer::Poder,
        slider_palette,
    ));
    children.push(mod_slider(
        &rimay_localize::t("dominium-slider-oro"),
        c.mods.oro,
        Layer::Oro,
        slider_palette,
    ));

    children.push(label_view(
        &rimay_localize::t("dominium-label-hack"),
        11.0,
        theme.fg_muted,
    ));
    match c.hack {
        None => {
            children.push(sized_button(
                "+ Agregar hack",
                btn_palette,
                Msg::HackToggle,
            ));
        }
        Some(h) => {
            children.push(sized_button(
                &format!("trigger: {}", trigger_label(h.trigger)),
                btn_palette,
                Msg::HackCycleTrigger,
            ));
            match h.trigger {
                Trigger::Always => {}
                Trigger::EnergiaBajo(v) => {
                    children.push(slider_view(
                        "umbral",
                        v,
                        0.0,
                        100.0,
                        slider_palette,
                        |phase, dv| match phase {
                            DragPhase::Move => Some(Msg::HackEditTriggerParam(dv)),
                            DragPhase::End => None,
                        },
                    ));
                }
                Trigger::EdadSobre(v) => {
                    children.push(slider_view(
                        "edad",
                        v as f32,
                        0.0,
                        1000.0,
                        slider_palette,
                        |phase, dv| match phase {
                            DragPhase::Move => Some(Msg::HackEditTriggerParam(dv)),
                            DragPhase::End => None,
                        },
                    ));
                }
            }
            children.push(sized_button(
                &format!("acción: {} ({})", h.forced_action, action_name(h.forced_action)),
                btn_palette,
                Msg::HackCycleAction,
            ));
            children.push(slider_view(
                "duración",
                h.duration as f32,
                1.0,
                500.0,
                slider_palette,
                |phase, dv| match phase {
                    DragPhase::Move => Some(Msg::HackEditDuration(dv)),
                    DragPhase::End => None,
                },
            ));
            children.push(sized_button("− Quitar hack", btn_palette, Msg::HackToggle));
        }
    }
    children.push(sized_button("🗑  Borrar", btn_palette, Msg::DeleteSelected));
    children.push(sized_button("◌  Deseleccionar", btn_palette, Msg::DeselectConcepto));
}

/// Tab "ψ" — sliders de psicología social + métricas ψ.
fn append_psique_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    stats: &WorldStats,
    psi_metrics: &PsiMetrics,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view("[ DIVERSIDAD ψ ]", 11.0, theme.fg_muted));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-orden"),
        &format!("{:.3}", stats.var_psi[0]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-miedo"),
        &format!("{:.3}", stats.var_psi[1]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-curiosidad"),
        &format!("{:.3}", stats.var_psi[2]),
        theme,
    ));
    children.push(stat_row(
        &rimay_localize::t("dominium-stat-var-psi-corruptib"),
        &format!("{:.3}", stats.var_psi[3]),
        theme,
    ));

    children.push(separator(theme));
    children.push(label_view("[ CONTAGIO SOCIAL ]", 11.0, theme.fg_muted));
    children.push(param_slider(
        "psi mod",
        model.params.psi_effect_modulation,
        ParamSlot::PsiModulation,
        slider_palette,
    ));
    children.push(param_slider(
        "radio soc",
        model.params.social_radius,
        ParamSlot::SocialRadius,
        slider_palette,
    ));
    children.push(param_slider(
        "contagio",
        model.params.contagion_rate,
        ParamSlot::ContagionRate,
        slider_palette,
    ));
    children.push(param_slider(
        "homofilia",
        model.params.homophily_threshold,
        ParamSlot::HomophilyThreshold,
        slider_palette,
    ));
    let big5_label = if model.params.big_five {
        "✓  Big Five: ON (5D)"
    } else {
        "○  Big Five: OFF (4D)"
    };
    children.push(sized_button(big5_label, btn_palette, Msg::ToggleBigFive));
    let policy_label = match model.params.action_policy {
        dominium_core::ActionPolicy::Fixed => "○  Política: Fixed".to_string(),
        dominium_core::ActionPolicy::PsiArgmax => format!(
            "✓  Política: PsiArgmax (T={})",
            model.params.policy_reeval_period
        ),
    };
    children.push(sized_button(&policy_label, btn_palette, Msg::CyclePsiPolicy));

    children.push(separator(theme));
    children.push(label_view("[ POLARIZACIÓN Esteban-Ray ]", 11.0, theme.fg_muted));
    let psi_labels = ["ORDEN", "MIEDO", "CURIO", "CORR"];
    for (i, lab) in psi_labels.iter().enumerate() {
        children.push(stat_row(
            &format!("polar {lab}"),
            &format!("{:.4}", psi_metrics.polarization[i]),
            theme,
        ));
    }
    children.push(label_view("[ Moran's I (autocorr.) ]", 11.0, theme.fg_muted));
    for (i, lab) in psi_labels.iter().enumerate() {
        children.push(stat_row(
            &format!("Moran {lab}"),
            &format!("{:+.3}", psi_metrics.moran_i[i]),
            theme,
        ));
    }
    if model.params.big_five {
        children.push(stat_row(
            "polar EXTRA",
            &format!("{:.4}", psi_metrics.polarization_ext),
            theme,
        ));
        children.push(stat_row(
            "Moran EXTRA",
            &format!("{:+.3}", psi_metrics.moran_i_ext),
            theme,
        ));
    }

    // Legend de clusters cuando el render está mostrando tribus.
    if matches!(model.cfg.render_mode, RenderMode::PsiCluster) {
        children.push(separator(theme));
        children.push(label_view("[ TRIBUS k-means ]", 11.0, theme.fg_muted));
        for (k, c) in CLUSTER_COLORS.iter().enumerate() {
            let n_in = model
                .cluster_assignments
                .iter()
                .filter(|&&a| a as usize == k)
                .count();
            children.push(stat_row(
                &format!("cluster {k}  ({})", color_swatch(*c)),
                &n_in.to_string(),
                theme,
            ));
        }
    }
}

/// Tab "Vista" — render mode + trails + andina + ZWeights + rewind.
fn append_vista_tab(
    children: &mut Vec<View<Msg>>,
    model: &Model,
    theme: &Theme,
    btn_palette: &ButtonPalette,
    slider_palette: &SliderPalette,
) {
    children.push(label_view("[ MODO RENDER ]", 11.0, theme.fg_muted));
    let render_label = match model.cfg.render_mode {
        RenderMode::Composite => "Render: compuesto".to_string(),
        RenderMode::Heatmap(l) => format!("Render: heatmap {}", l.label()),
        RenderMode::PsiCluster => "Render: tribus ψ (k-means)".to_string(),
    };
    children.push(sized_button(&render_label, btn_palette, Msg::CycleRenderMode));
    let trails_label = if model.show_trails {
        "✓  Trayectorias: ON"
    } else {
        "○  Trayectorias: OFF"
    };
    children.push(sized_button(trails_label, btn_palette, Msg::ToggleTrails));
    let texture_label = if model.cfg.texture {
        "✓  Textura: ON"
    } else {
        "○  Textura: OFF"
    };
    children.push(sized_button(texture_label, btn_palette, Msg::ToggleTexture));
    let andina_label = if model.cfg.andina_layers > 0 {
        "✓  Estampa andina: ON"
    } else {
        "○  Estampa andina: OFF"
    };
    children.push(sized_button(andina_label, btn_palette, Msg::ToggleAndina));

    children.push(separator(theme));
    children.push(label_view("[ RELIEVE VISUAL ]", 11.0, theme.fg_muted));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-materia"),
        model.weights.materia,
        ZSlot::Materia,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-psique"),
        model.weights.psique,
        ZSlot::Psique,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-poder"),
        model.weights.poder,
        ZSlot::Poder,
        slider_palette,
    ));
    children.push(z_slider(
        &rimay_localize::t("dominium-slider-oro"),
        model.weights.oro,
        ZSlot::Oro,
        slider_palette,
    ));
    children.push(z_slider(
        "degrad.",
        model.weights.degradacion,
        ZSlot::Degradacion,
        slider_palette,
    ));
    let sync_label = if model.sync_relieve {
        "✓  Sync físico: ON"
    } else {
        "○  Sync físico: OFF"
    };
    children.push(sized_button(sync_label, btn_palette, Msg::ToggleSyncRelieve));

    children.push(separator(theme));
    children.push(label_view("[ REWIND ]", 11.0, theme.fg_muted));
    let max_rewind = model.snapshots.len().saturating_sub(1).max(1);
    children.push(slider_view(
        "rewind",
        model.rewind_offset as f32,
        0.0,
        max_rewind as f32,
        slider_palette,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::RewindBy(dv)),
            DragPhase::End => None,
        },
    ));
    if model.rewind_offset > 0 {
        children.push(sized_button(
            &format!("▶  Vivo (estabas {} atrás)", model.rewind_offset),
            btn_palette,
            Msg::RewindHome,
        ));
    }
}

/// Glifo simple para indicar el color de un cluster en una fila de stat.
/// El texto es monoespaciado pero los colores van en el panel — usamos
/// emojis círculos para que el matching visual sea inmediato sin tocar el
/// renderer del label.
fn color_swatch(c: Color) -> &'static str {
    let r = c[0] > 0.6;
    let g = c[1] > 0.6;
    let b = c[2] > 0.6;
    match (r, g, b) {
        (true, false, true) => "magenta",
        (false, true, true) => "cian",
        (true, true, false) => "amarillo",
        _ => "·",
    }
}

fn label_view(text: &str, size_px: f32, color: llimphi_ui::llimphi_raster::peniko::Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), size_px, color, Alignment::Start)
}

fn stat_row(label: &str, value: &str, theme: &Theme) -> View<Msg> {
    let label_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_muted, Alignment::Start);
    let value_v = View::new(Style {
        size: Size {
            width: length(90.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(value.to_string(), 12.0, theme.fg_text, Alignment::End);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![label_v, value_v])
}

fn sized_button(label: &str, palette: &ButtonPalette, msg: Msg) -> View<Msg> {
    let mut btn = button_view(label, palette, msg);
    btn.style.size = Size {
        width: percent(1.0_f32),
        height: length(30.0_f32),
    };
    btn
}

/// Fila clicable con el nombre de un Concepto. La fila seleccionada
/// queda resaltada con `bg_selected`; las demás reaccionan al hover.
fn concepto_row(i: usize, id: &str, selected: bool, theme: &Theme) -> View<Msg> {
    let bg = if selected { theme.bg_selected } else { theme.bg_panel };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .text_aligned(
        format!("·  {id}"),
        12.0,
        if selected { theme.accent } else { theme.fg_text },
        Alignment::Start,
    )
    .on_click(Msg::SelectConcepto(i))
}

/// Slider para una capa de `LayerMods`. Rango fijo `[-1, 1]` — encaja con
/// el patrón típico (emisión positiva, drenaje negativo).
fn mod_slider(label: &str, value: f32, layer: Layer, palette: &SliderPalette) -> View<Msg> {
    slider_view(
        label,
        value,
        -1.0,
        1.0,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditMod(layer, dv)),
            DragPhase::End => None,
        },
    )
}

/// Slider para un slot de `SimParams`. El rango lo decide el slot.
fn param_slider(
    label: &str,
    value: f32,
    slot: ParamSlot,
    palette: &SliderPalette,
) -> View<Msg> {
    let (min, max) = slot.range();
    slider_view(
        label,
        value,
        min,
        max,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditParam(slot, dv)),
            DragPhase::End => None,
        },
    )
}

/// Slider para un slot de `ZWeights` (relieve visual del render).
/// Rango simétrico [-2, 2]: negativo = la capa cava valles, positivo = eleva.
fn z_slider(
    label: &str,
    value: f32,
    slot: ZSlot,
    palette: &SliderPalette,
) -> View<Msg> {
    slider_view(
        label,
        value,
        -2.0,
        2.0,
        palette,
        move |phase, dv| match phase {
            DragPhase::Move => Some(Msg::EditZWeight(slot, dv)),
            DragPhase::End => None,
        },
    )
}
