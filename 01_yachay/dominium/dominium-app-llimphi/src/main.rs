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

use std::time::Duration;

use dominium_canvas_llimphi::canvas_view;
use dominium_core::{Conceptos, SimParams, World};
use dominium_iso::{IsoProjector, ZWeights};
use dominium_physics::tick;
use dominium_render_plan::{build_plan, PlanConfig};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};

/// Lado de la grilla cuadrada del mundo.
const GRID: usize = 40;
/// Población inicial de Lemmings.
const LEMMINGS: usize = 50;
/// Periodo del bucle de simulación (~11 Hz).
const TICK_MS: u64 = 90;
/// Ancho del panel de stats.
const SIDE_WIDTH: f32 = 240.0;
/// Pack JSON por defecto — iglesia / banco / comuna / laboratorio.
/// Embebido para que el binario corra sin archivos sueltos en cwd.
const DEFAULT_PACK: &str = include_str!("../conceptos.default.json");

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

/// Parsea el pack JSON embebido. Si el JSON está malformado el binario
/// arranca con la colección vacía — la sim corre igual.
fn default_conceptos() -> Conceptos {
    serde_json::from_str::<Conceptos>(DEFAULT_PACK).unwrap_or_default()
}

/// Siembra un mundo: continentes de materia, vetas de oro, niebla de
/// psique y una población de Lemmings con sesgos y acciones variadas.
fn seed(seed: u64) -> World {
    let mut w = World::new(GRID, GRID);
    let mut rng = Lcg::new(seed);
    for cy in 0..GRID {
        for cx in 0..GRID {
            let idx = w.grid.idx(cx, cy);
            let m = rng.next_f32();
            w.grid.materia[idx] = m * m * 60.0;
            if rng.next_f32() > 0.92 {
                w.grid.oro[idx] = rng.next_f32() * 40.0;
            }
            w.grid.psique[idx] = rng.next_f32() * 12.0;
        }
    }
    for _ in 0..LEMMINGS {
        let x = rng.next_f32() * (GRID as f32 - 1.0);
        let y = rng.next_f32() * (GRID as f32 - 1.0);
        let psi = [
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
            rng.next_f32(),
        ];
        let i = w.lemmings.spawn(x, y, 30.0 + rng.next_f32() * 40.0, psi);
        w.lemmings.accion[i] = (rng.next_u32() % 6) as u8;
    }
    w.conceptos = default_conceptos();
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

struct Stats {
    poblacion: usize,
    materia: f32,
    oro: f32,
    energia: f32,
}

impl Model {
    fn stats(&self) -> Stats {
        let g = &self.world.grid;
        Stats {
            poblacion: self.world.lemmings.len(),
            materia: g.materia.iter().sum(),
            oro: g.oro.iter().sum(),
            energia: self.world.lemmings.energia.iter().sum(),
        }
    }
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

        let rng_seed = 0xD0_31_31_07;
        Model {
            world: seed(rng_seed),
            params: SimParams::default(),
            iso: IsoProjector::new(12.0, 0.05),
            weights: ZWeights::default(),
            cfg: PlanConfig {
                tile: 15.0,
                lemming_size: 8.0,
                lemming_lift: 0.7,
                concepto_size: 12.0,
                concepto_lift: 1.6,
                light_dir: (0.55, 0.35),
                palette: Default::default(),
            },
            running: true,
            tick: 0,
            epoch: 0,
            rng_seed,
            selected: None,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                if m.running {
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
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let stats = model.stats();

        let status = status_bar(model, &theme);
        let plan = build_plan(&model.world, &model.iso, &model.weights, &model.cfg);
        let canvas = canvas_pane(plan);
        let side = side_panel(model, &stats, &theme);

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

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![status, body])
    }
}

fn main() {
    llimphi_ui::run::<Dominium>();
}

// ---------------------------------------------------------------------
// Transiciones
// ---------------------------------------------------------------------

/// Un paso de simulación; re-siembra si la población colapsa.
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
    }
}

fn reseed(m: &mut Model) {
    m.rng_seed = m.rng_seed.wrapping_add(0x9E37_79B9);
    m.world = seed(m.rng_seed);
    m.tick = 0;
    m.epoch += 1;
}

// ---------------------------------------------------------------------
// Vistas
// ---------------------------------------------------------------------

fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let estado = if model.running { "● corriendo" } else { "‖ en pausa" };
    let label_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(
        format!(
            "dominium · campo medio   ·   época {}   ·   tick {}",
            model.epoch, model.tick
        ),
        12.0,
        theme.fg_text,
        Alignment::Start,
    );
    let estado_view = View::new(Style {
        size: Size {
            width: length(120.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(estado.to_string(), 12.0, theme.accent, Alignment::End);

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

fn side_panel(model: &Model, stats: &Stats, theme: &Theme) -> View<Msg> {
    let btn_palette = ButtonPalette::from_theme(theme);
    let mut slider_palette = SliderPalette::from_theme(theme);
    // Comprimimos los slots para que entren en el sidebar de 240 px.
    slider_palette.label_width = 56.0;
    slider_palette.track_width = 90.0;
    slider_palette.value_width = 44.0;

    let header = label_view("[ SIM ]", 11.0, theme.fg_muted);

    let play_label = if model.running { "‖  Pausar" } else { "▶  Reanudar" };
    let play_btn = sized_button(play_label, &btn_palette, Msg::TogglePlay);
    let reset_btn = sized_button("↺  Re-sembrar", &btn_palette, Msg::Reseed);

    let separator = || -> View<Msg> {
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.border)
    };

    let conceptos_header = label_view("[ CONCEPTOS ]", 11.0, theme.fg_muted);
    let conceptos_count = label_view(
        &format!("{} activos", model.world.conceptos.len()),
        12.0,
        theme.fg_text,
    );
    let mut children: Vec<View<Msg>> = vec![
        header,
        play_btn,
        reset_btn,
        separator(),
        stat_row("Población", &stats.poblacion.to_string(), theme),
        stat_row("Materia", &format!("{:.0}", stats.materia), theme),
        stat_row("Oro", &format!("{:.0}", stats.oro), theme),
        stat_row("Energía", &format!("{:.0}", stats.energia), theme),
        separator(),
        conceptos_header,
        conceptos_count,
    ];
    for (i, c) in model.world.conceptos.items.iter().enumerate() {
        children.push(concepto_row(i, &c.id, model.selected == Some(i), theme));
    }
    children.push(sized_button("✚  Sembrar pack", &btn_palette, Msg::SembrarConceptos));
    children.push(sized_button("✖  Limpiar", &btn_palette, Msg::LimpiarConceptos));

    // Editor del concepto seleccionado: sliders en vivo sobre radius + 4 mods.
    if let Some(i) = model.selected {
        if let Some(c) = model.world.conceptos.items.get(i) {
            children.push(separator());
            children.push(label_view("[ EDITAR ]", 11.0, theme.fg_muted));
            children.push(label_view(&format!("• {}", c.id), 12.0, theme.fg_text));
            children.push(slider_view(
                "radius",
                c.radius,
                0.5,
                20.0,
                &slider_palette,
                |phase, dv| match phase {
                    DragPhase::Move => Some(Msg::EditRadius(dv)),
                    DragPhase::End => None,
                },
            ));
            children.push(mod_slider("materia", c.mods.materia, Layer::Materia, &slider_palette));
            children.push(mod_slider("psique", c.mods.psique, Layer::Psique, &slider_palette));
            children.push(mod_slider("poder", c.mods.poder, Layer::Poder, &slider_palette));
            children.push(mod_slider("oro", c.mods.oro, Layer::Oro, &slider_palette));
            children.push(sized_button("🗑  Borrar", &btn_palette, Msg::DeleteSelected));
            children.push(sized_button("◌  Deseleccionar", &btn_palette, Msg::DeselectConcepto));
        }
    }

    children.push(separator());
    children.push(label_view(&format!("grilla {GRID}×{GRID}"), 11.0, theme.fg_muted));
    children.push(label_view("relieve = materia (Z)", 11.0, theme.fg_muted));

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
