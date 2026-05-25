//! `shuma-shell-llimphi` — chasis del shell shuma sobre Llimphi.
//!
//! **El chasis no asume qué módulos existen.** Su trabajo es:
//!
//! 1. Conocer un **registry estático** de los módulos compilados en
//!    este binario (hoy: `shell`; bloque 3: `matilda`; eventual:
//!    `launcher`).
//! 2. Leer el shumarc para saber **cuáles activar** y con qué `Source`
//!    (local/remoto) — bloque 5.
//! 3. Componer la ventana a partir de los módulos activos: tabs
//!    principales, monitores en el panel derecho, shortcuts en la
//!    toolbar. (Drawer/desplegable llega en su propio bloque.)
//!
//! El shell interactivo (REPL) es un módulo más, no está hardcodeado.
//! Por eso `shuma-module-shell` (placeholder por ahora) entra al
//! registry como cualquier otro. Cuando se migre el REPL desde la
//! versión GPUI, sólo cambia `shuma-module-shell` — el chasis ni se
//! entera.
//!
//! Layout:
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────┐
//!  │ app-header · "shuma · {módulo activo}"   [toolbar]        │
//!  ├──────────────────────────────────────────────────────────┤
//!  │ tabs: [shell] [matilda] …                                 │
//!  ├──────────────────────────────────┬───────────────────────┤
//!  │                                  │ stack de monitores    │
//!  │   contenido del módulo activo    │ (CPU/MEM builtin del  │
//!  │                                  │  chasis + los que     │
//!  │                                  │  aporten los módulos) │
//!  └──────────────────────────────────┴───────────────────────┘
//! ```

#![forbid(unsafe_code)]

use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, PathEl, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, DragPhase, Handle, PaintRect, View};
use llimphi_theme::Theme;
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use shuma_module::Source;
use shuma_sysmon::{Snapshot, SystemSampler};

/// Cuántas muestras guarda la curva de cada monitor.
const HISTORY: usize = 60;
const TICK: Duration = Duration::from_secs(1);
const MONITORS_INITIAL_WIDTH: f32 = 280.0;

fn main() {
    llimphi_ui::run::<Shell>();
}

// ─── Registry estático ───────────────────────────────────────────────
//
// Cada módulo compilado en este binario entra como una variante de
// `Kind`. Agregar un módulo (p. ej. `Launcher`) es una variante nueva
// + tres ramas en el match (`label`, `view`, `Msg`). El shumarc
// referencia los módulos por `id` (`shuma_module_shell::ID`, etc.) y
// el chasis los resuelve aquí — un módulo activado pero no compilado
// se ignora con warning (bloque 5).

/// Tipos de módulos conocidos por *este* binario. Estático en lugar de
/// `dyn Module` porque `llimphi-ui::View` no tiene `map` y cada módulo
/// trae su propio `Msg` — el enum lo encapsula sin trait object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Shell,
}

impl Kind {
    /// `id` canónico — bloque 5 lo usa para enrutar entradas del shumarc.
    #[allow(dead_code)]
    fn id(self) -> &'static str {
        match self {
            Kind::Shell => shuma_module_shell::ID,
        }
    }
}

/// Una instancia activa de un módulo en la sesión actual. Una misma
/// `Kind` puede aparecer dos veces (p. ej. `shell` local + `shell`
/// remoto), con `Source` distinto.
struct Instance {
    kind: Kind,
    label: String,
    state: ModuleState,
}

/// El state vivo de cada módulo. El chasis no lo interpreta; sólo lo
/// pasa al `view`/`update` correspondiente del módulo.
enum ModuleState {
    Shell(shuma_module_shell::State),
}

impl Instance {
    fn shell(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Shell,
            label,
            state: ModuleState::Shell(shuma_module_shell::State::new(source)),
        }
    }
}

/// Mensajes opacos del módulo. Una variante por `Kind`. El chasis los
/// enruta a `update` del módulo correspondiente; no los interpreta.
#[derive(Debug, Clone)]
enum ModuleMsg {
    // Variant para shell — actualmente Msg está vacío así que no se
    // usa, pero la variante deja el cableado listo para cuando llegue
    // el REPL real.
    #[allow(dead_code)]
    Shell(shuma_module_shell::Msg),
}

// ─── Modelo + Msg del chasis ────────────────────────────────────────

struct Model {
    theme: Theme,
    /// Lista de módulos activos en esta sesión. El orden define el
    /// orden de los tabs. Por ahora se hardcodea en `init()`; bloque
    /// 5 lo lee del shumarc.
    instances: Vec<Instance>,
    /// Índice del módulo activo (la tab visible).
    active: usize,
    sysmon: SystemSampler,
    last_snapshot: Option<Snapshot>,
    monitors_width: f32,
}

#[derive(Clone)]
enum Msg {
    Tick,
    SelectTab(usize),
    ResizeMonitors(f32),
    /// Mensaje enrutado a un módulo específico (por índice en
    /// `instances`). El chasis los entrega al `update` apropiado.
    #[allow(dead_code)]
    Module(usize, ModuleMsg),
}

struct Shell;

impl App for Shell {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "shuma"
    }

    fn app_id() -> Option<&'static str> {
        Some("shuma.shell")
    }

    fn initial_size() -> (u32, u32) {
        (1280, 800)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(TICK, || Msg::Tick);

        // Default cuando no hay shumarc: una sola instancia del shell
        // local. Cuando llegue el config (bloque 5), `instances` se
        // construye a partir de `[[modules]]`.
        let instances = vec![Instance::shell("Shell".into(), Source::Local)];

        Model {
            theme: Theme::dark(),
            instances,
            active: 0,
            sysmon: SystemSampler::new(HISTORY),
            last_snapshot: None,
            monitors_width: MONITORS_INITIAL_WIDTH,
        }
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                m.last_snapshot = Some(m.sysmon.sample());
            }
            Msg::SelectTab(i) => {
                if i < m.instances.len() {
                    m.active = i;
                }
            }
            Msg::ResizeMonitors(dx) => {
                m.monitors_width = (m.monitors_width - dx).clamp(180.0, 480.0);
            }
            Msg::Module(idx, mmsg) => {
                if let Some(inst) = m.instances.get_mut(idx) {
                    match (&mut inst.state, mmsg) {
                        (ModuleState::Shell(s), ModuleMsg::Shell(msg)) => {
                            let new_state = shuma_module_shell::update(s.clone(), msg);
                            *s = new_state;
                        }
                    }
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;
        let header = header_with_toolbar(model);
        let body = main_body(model, theme);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

fn header_with_toolbar(model: &Model) -> View<Msg> {
    let palette = AppHeaderPalette::from_theme(&model.theme);
    let label = match model.instances.get(model.active) {
        Some(inst) => format!("shuma · {}", inst.label),
        None => "shuma".into(),
    };
    // Toolbar de shortcuts: vacía hasta el bloque 4 (slot real).
    let actions: Vec<View<Msg>> = Vec::new();
    app_header(label, actions, &palette)
}

fn main_body(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = TabsPalette::from_theme(theme);
    let splitter_palette = SplitterPalette::from_theme(theme);

    let content = active_module_view(model, theme);
    let monitors = monitor_stack(model, theme);

    let labels: Vec<String> = model
        .instances
        .iter()
        .map(|inst| inst.label.clone())
        .collect();

    tabs_view(TabsSpec {
        labels,
        active: model.active,
        on_select: Msg::SelectTab,
        content: splitter_two(
            Direction::Row,
            content,
            PaneSize::Flex,
            monitors,
            PaneSize::Fixed(model.monitors_width),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeMonitors(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        ),
        tab_height: 32.0,
        palette,
        tab_width: None,
    })
}

/// Renderiza el contenido del módulo activo. Cada `Kind` se enruta a
/// su `view` con un `lift` que envuelve su `Msg` en `Msg::Module(idx, …)`.
fn active_module_view(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(inst) = model.instances.get(model.active) else {
        return placeholder(theme, "Sin módulos activos.");
    };
    let idx = model.active;
    match (inst.kind, &inst.state) {
        (Kind::Shell, ModuleState::Shell(state)) => shuma_module_shell::view::<Msg>(
            state,
            theme,
            move |m| Msg::Module(idx, ModuleMsg::Shell(m)),
        ),
    }
}

fn monitor_stack(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = StatCardPalette::from_theme(theme);

    let (cpu_value, mem_value) = match model.last_snapshot {
        Some(s) if s.valid => (s.cpu_percent, s.mem_percent),
        _ => (0.0, 0.0),
    };

    let cpu_history = model.sysmon.cpu_history().values();
    let mem_history = model.sysmon.mem_history().values();

    let cpu_card = monitor_card(
        "CPU",
        format!("{cpu_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!(
                "{} de {} muestras",
                model.sysmon.cpu_history().len(),
                HISTORY
            ),
            _ => "sin datos (¿no es Linux?)".into(),
        },
        Color::from_rgb8(0x82, 0xCF, 0xF2),
        cpu_history,
        &palette,
    );

    let mem_card = monitor_card(
        "MEM",
        format!("{mem_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!("{} MB de {} MB", s.mem_used_mb, s.mem_total_mb),
            _ => "sin datos".into(),
        },
        Color::from_rgb8(0xF7, 0xC8, 0x7A),
        mem_history,
        &palette,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(vec![cpu_card, mem_card])
}

fn monitor_card(
    label: &str,
    value: String,
    description: String,
    accent: Color,
    history: Vec<f32>,
    palette: &StatCardPalette,
) -> View<Msg> {
    let card = stat_card_view::<Msg>(label, value, description.as_str(), accent, &[], palette);
    let curve = curve_view(history, accent);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![card, curve])
}

fn curve_view(history: Vec<f32>, accent: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if history.len() < 2 {
            return;
        }
        let n = history.len() as f32;
        let dx = if n > 1.0 { rect.w / (n - 1.0) } else { rect.w };
        let mut path = BezPath::new();
        for (i, v) in history.iter().enumerate() {
            let x = rect.x + dx * i as f32;
            let y = rect.y + rect.h - (v.clamp(0.0, 100.0) / 100.0) * rect.h;
            let p = Point::new(x as f64, y as f64);
            if i == 0 {
                path.push(PathEl::MoveTo(p));
            } else {
                path.push(PathEl::LineTo(p));
            }
        }
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, accent, None, &path);
    })
}

fn placeholder(theme: &Theme, text: &str) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .text_aligned(text.to_string(), 13.0, theme.fg_muted, Alignment::Start)
}

