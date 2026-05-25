//! `shuma-module-matilda` — administración declarativa como módulo.
//!
//! Adapta el CLI `matilda` para que viva como tab dentro de `shuma-shell-llimphi`:
//! visualiza el inventario, calcula el plan de reconciliación contra el
//! estado actual y previsualiza los pasos en seco (`dry_run`). Apply
//! real local también; apply remoto vía `matilda-linker` llega cuando
//! el chasis cablee `Source::Remote` (bloque de conectividad).
//!
//! Diseño del tab:
//!
//! ```text
//!  Matilda · local · 1 host · 2 containers · 1 vhost
//!  ┌──────────────────────────┬──────────────────────────────┐
//!  │ Inventario               │ Plan (4 acciones)            │
//!  │                          │  1. crear contenedor «web»   │
//!  │ HOSTS (1)                │  2. crear contenedor «api»   │
//!  │   edge-1   10.0.0.1      │  3. crear vhost «sitio.com»  │
//!  │                          │  …                            │
//!  │ CONTAINERS (2)           │                              │
//!  │   web      nginx:1.27    │ Log                          │
//!  │   api      ejemplo/api   │  $ docker pull nginx:1.27    │
//!  │                          │  …                            │
//!  │ VHOSTS (1)               │                              │
//!  │   sitio.com → web:80     │                              │
//!  └──────────────────────────┴──────────────────────────────┘
//! ```
//!
//! Contribuciones declarativas:
//!
//! - **Monitor "matilda · pasos"**: count del plan vigente (0 cuando el
//!   inventario actual coincide con el deseado).
//! - **Shortcuts**: `Discover`, `Plan`, `Dry-run`. El chasis los pinta
//!   en la toolbar de la app-header.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_theme::Theme;
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use matilda_apply::plan_to_steps;
use matilda_core::{Container, Host, Inventory, RestartPolicy, VHost};
use matilda_discover::discover_inventory;
use matilda_ghost::{dry_run, ApplyReport};
use matilda_plan::{plan, Op, Plan};
use shuma_module::{ModuleContributions, MonitorSpec, Rgb, Sample, ShortcutSpec, Source};
use std::sync::{Arc, Mutex};

pub const ID: &str = "matilda";

/// Estado del módulo. El `desired` se llena con un ejemplo arrancable
/// hasta que el bloque 5 cablee `--inventory` desde el shumarc. El
/// `pending_steps` se comparte por `Arc<Mutex<>>` para que el sampler
/// del monitor lo lea desde el thread de polling sin pelear con el UI.
#[derive(Debug, Clone)]
pub struct State {
    pub source: Source,
    pub desired: Inventory,
    pub current: Option<Inventory>,
    pub plan: Option<Plan>,
    pub log: Vec<String>,
    pub split_width: f32,
    pending_steps: Arc<Mutex<usize>>,
}

impl State {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            desired: example_inventory(),
            current: None,
            plan: None,
            log: Vec::new(),
            split_width: 380.0,
            pending_steps: Arc::new(Mutex::new(0)),
        }
    }

    /// Inventario actual contra el cual reconciliar — si no se ha
    /// hecho discover, asume "vacío" (todo es creación). Equivale al
    /// modo CLI `matilda plan inv.json` sin `--discover`.
    pub fn current_or_empty(&self) -> Inventory {
        self.current.clone().unwrap_or_default()
    }

    /// Cuenta de pasos pendientes — alimenta el monitor.
    pub fn pending_count(&self) -> usize {
        self.plan.as_ref().map(|p| p.len()).unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Descubre el inventario actual del servidor (local por ahora).
    Discover,
    /// Recalcula el plan deseado-vs-actual.
    MakePlan,
    /// Ejecuta `dry_run` sobre los pasos del plan y vuelca al log.
    DryRun,
    /// Drag del splitter inventario|plan.
    ResizeSplit(f32),
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Discover => match &s.source {
            Source::Local => {
                let current = discover_inventory(&s.desired);
                s.log.push(format!(
                    "✔ discover local: {} containers, {} vhosts",
                    current.containers().count(),
                    current.vhosts().count()
                ));
                s.current = Some(current);
            }
            Source::Remote { host, .. } => {
                s.log.push(format!(
                    "✘ discover remoto en {host} no implementado todavía"
                ));
            }
        },
        Msg::MakePlan => {
            let p = plan(&s.current_or_empty(), &s.desired);
            s.log.push(format!(
                "✔ plan: {} acciones ({} crear, {} actualizar, {} eliminar)",
                p.len(),
                p.count(Op::Create),
                p.count(Op::Update),
                p.count(Op::Remove)
            ));
            *s.pending_steps.lock().unwrap() = p.len();
            s.plan = Some(p);
        }
        Msg::DryRun => {
            let p = match &s.plan {
                Some(p) => p.clone(),
                None => plan(&s.current_or_empty(), &s.desired),
            };
            let steps = plan_to_steps(&p, &s.desired);
            if steps.is_empty() {
                s.log.push("Sin pasos: nada que aplicar.".into());
            } else {
                s.log.push(format!("— dry-run de {} pasos —", steps.len()));
                let report: ApplyReport = dry_run(&steps);
                for r in &report.results {
                    s.log.push(format!(
                        "{} {}",
                        if r.ok { "✔" } else { "✘" },
                        r.describe
                    ));
                    for line in &r.log {
                        s.log.push(format!("   {line}"));
                    }
                }
            }
            // Recorta el log a las últimas 200 líneas para no crecer
            // sin tope durante una sesión larga.
            let len = s.log.len();
            if len > 200 {
                s.log.drain(0..len - 200);
            }
        }
        Msg::ResizeSplit(dx) => {
            s.split_width = (s.split_width + dx).clamp(220.0, 720.0);
        }
    }
    s
}

/// Inventario de ejemplo — equivale al `matilda example`. Permite
/// arrancar el módulo sin un archivo de inventario y demostrar el
/// flujo plan/dry-run sin tocar nada del servidor.
pub fn example_inventory() -> Inventory {
    let mut inv = Inventory::new();
    inv.add_host(Host::new("edge-1", "10.0.0.1").with_tag("prod"));
    inv.add_container(
        Container::new("web", "nginx:1.27")
            .with_port(8080, 80)
            .with_volume("/srv/site", "/usr/share/nginx/html")
            .with_restart(RestartPolicy::Always),
    );
    inv.add_container(
        Container::new("api", "ghcr.io/ejemplo/api:1.0")
            .with_port(9000, 9000)
            .with_env("DATABASE_URL", "postgres://db/app")
            .with_restart(RestartPolicy::UnlessStopped),
    );
    inv.add_vhost(
        VHost::to_container("sitio.com", "web", 80)
            .with_alias("www.sitio.com")
            .with_tls(),
    );
    inv
}

// ─── view ──────────────────────────────────────────────────────────

pub fn view<HostMsg: Clone + Send + Sync + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = matilda_header(state, theme);

    let inv_pane = inventory_pane(state, theme);
    let plan_pane = plan_and_log_pane(state, theme);

    let splitter_palette = SplitterPalette::from_theme(theme);
    let lift_resize = lift.clone();
    let body = splitter_two(
        Direction::Row,
        inv_pane,
        PaneSize::Fixed(state.split_width),
        plan_pane,
        PaneSize::Flex,
        move |phase, dx| match phase {
            DragPhase::Move => Some(lift_resize(Msg::ResizeSplit(dx))),
            DragPhase::End => None,
        },
        &splitter_palette,
    );

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

fn matilda_header<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    let label = format!(
        "Matilda · {} · {} hosts · {} containers · {} vhosts",
        state.source.label(),
        state.desired.hosts().count(),
        state.desired.containers().count(),
        state.desired.vhosts().count(),
    );
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(label, 12.0, theme.fg_text, Alignment::Start)
}

/// Panel izquierdo: el inventario deseado en 3 secciones (hosts /
/// containers / vhosts). Compuesto como Views planos — el
/// `llimphi-widget-list` exigiría un `on_click` por fila, y en este
/// tab las filas son informativas (no se seleccionan todavía).
fn inventory_pane<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    let mut children: Vec<View<HostMsg>> = Vec::new();

    children.push(section_label(
        &format!("HOSTS ({})", state.desired.hosts().count()),
        theme,
    ));
    for h in state.desired.hosts() {
        children.push(inv_row(&format!("  {}   {}", h.name, h.address), theme));
    }

    children.push(section_label(
        &format!("CONTAINERS ({})", state.desired.containers().count()),
        theme,
    ));
    for c in state.desired.containers() {
        children.push(inv_row(&format!("  {}   {}", c.name, c.image), theme));
    }

    children.push(section_label(
        &format!("VHOSTS ({})", state.desired.vhosts().count()),
        theme,
    ));
    for v in state.desired.vhosts() {
        children.push(inv_row(
            &format!("  {} → {}", v.domain, describe_upstream(&v.upstream)),
            theme,
        ));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(children)
}

fn describe_upstream(u: &matilda_core::Upstream) -> String {
    use matilda_core::Upstream::*;
    match u {
        Container { name, port } => format!("{name}:{port}"),
        Address(addr) => addr.clone(),
    }
}

fn inv_row<HostMsg: Clone + 'static>(text: &str, theme: &Theme) -> View<HostMsg> {
    text_row(text, theme.fg_text, theme)
}

fn plan_and_log_pane<HostMsg: Clone + 'static>(state: &State, theme: &Theme) -> View<HostMsg> {
    let plan_label = match &state.plan {
        Some(p) if p.is_empty() => "Plan · sin cambios".to_string(),
        Some(p) => format!("Plan · {} acciones", p.len()),
        None => "Plan · sin calcular (pulsá «Plan» en la toolbar)".to_string(),
    };

    let plan_header = section_label(&plan_label, theme);

    let mut plan_children: Vec<View<HostMsg>> = vec![plan_header];
    if let Some(p) = &state.plan {
        for (i, action) in p.actions.iter().enumerate() {
            plan_children.push(text_row(
                &format!("{:>2}. {}", i + 1, action.describe()),
                theme.fg_text,
                theme,
            ));
        }
    }

    plan_children.push(section_label("Log", theme));
    for line in state.log.iter().rev().take(40).rev() {
        plan_children.push(text_row(line, theme.fg_muted, theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(plan_children)
}

fn section_label<HostMsg: Clone + 'static>(text: &str, theme: &Theme) -> View<HostMsg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, theme.accent, Alignment::Start)
}

fn text_row<HostMsg: Clone + 'static>(
    text: &str,
    color: llimphi_ui::llimphi_raster::peniko::Color,
    _theme: &Theme,
) -> View<HostMsg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(text.to_string(), 11.0, color, Alignment::Start)
}

// ─── contributions ──────────────────────────────────────────────────

pub fn contributions(state: &State) -> ModuleContributions {
    let pending = state.pending_steps.clone();
    let monitor = MonitorSpec {
        id: "matilda.pending",
        label: format!("matilda · {}", state.source.label()),
        accent: Rgb::new(0xE5, 0xC0, 0x7B),
        history_capacity: 60,
        period_secs: 5.0,
        sampler: Box::new(move || {
            let n = *pending.lock().unwrap();
            Sample::new(n as f32, format!("{n} pendientes"))
        }),
    };

    ModuleContributions {
        monitors: vec![monitor],
        shortcuts: vec![
            ShortcutSpec::module_action("Discover", "matilda.discover")
                .with_hint("Lee el estado actual del servidor"),
            ShortcutSpec::module_action("Plan", "matilda.plan")
                .with_hint("Calcula la reconciliación deseado-vs-actual"),
            ShortcutSpec::module_action("Dry-run", "matilda.dry_run")
                .with_hint("Previsualiza los pasos sin aplicar"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_stable() {
        assert_eq!(ID, "matilda");
    }

    #[test]
    fn example_inventory_has_expected_shape() {
        let inv = example_inventory();
        assert_eq!(inv.hosts().count(), 1);
        assert_eq!(inv.containers().count(), 2);
        assert_eq!(inv.vhosts().count(), 1);
    }

    #[test]
    fn fresh_state_has_no_plan_no_current() {
        let s = State::new(Source::Local);
        assert!(s.plan.is_none());
        assert!(s.current.is_none());
        assert_eq!(s.pending_count(), 0);
    }

    #[test]
    fn make_plan_against_empty_current_creates_all() {
        let s = State::new(Source::Local);
        let s = update(s, Msg::MakePlan);
        let plan = s.plan.as_ref().expect("plan se debe haber calculado");
        // 2 containers + 1 vhost (los hosts no producen acción si no hay
        // current, pero el example_inventory tiene 1 → cuenta como create).
        assert_eq!(plan.count(Op::Create), 4);
        assert_eq!(s.pending_count(), 4);
    }

    #[test]
    fn dry_run_appends_log_lines() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::MakePlan);
        let log_before = s.log.len();
        s = update(s, Msg::DryRun);
        assert!(s.log.len() > log_before, "dry-run debe agregar líneas al log");
    }

    #[test]
    fn dry_run_with_empty_plan_says_nothing_to_apply() {
        let mut s = State::new(Source::Local);
        // Force plan vacío: igualamos current al desired.
        s.current = Some(s.desired.clone());
        s = update(s, Msg::MakePlan);
        assert_eq!(s.plan.as_ref().unwrap().len(), 0);
        s = update(s, Msg::DryRun);
        assert!(s
            .log
            .iter()
            .any(|l| l.contains("nada que aplicar")));
    }

    #[test]
    fn remote_discover_is_unimplemented_for_now() {
        let s = State::new(Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: None,
        });
        let s = update(s, Msg::Discover);
        assert!(s.log.iter().any(|l| l.contains("no implementado")));
        assert!(s.current.is_none());
    }

    #[test]
    fn resize_split_clamps_to_range() {
        let s = State::new(Source::Local);
        let s = update(s, Msg::ResizeSplit(-10000.0));
        assert!(s.split_width >= 220.0);
        let s = update(s, Msg::ResizeSplit(10000.0));
        assert!(s.split_width <= 720.0);
    }

    #[test]
    fn contributions_expose_monitor_and_three_shortcuts() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        assert_eq!(c.monitors.len(), 1);
        assert_eq!(c.shortcuts.len(), 3);
        assert_eq!(c.shortcuts[0].label, "Discover");
        assert_eq!(c.shortcuts[1].label, "Plan");
        assert_eq!(c.shortcuts[2].label, "Dry-run");
    }

    #[test]
    fn monitor_sampler_reflects_pending_steps() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::MakePlan); // 4 pendientes
        let c = contributions(&s);
        let sample = (c.monitors[0].sampler)();
        assert_eq!(sample.value, 4.0);
        assert_eq!(sample.display, "4 pendientes");
    }
}
