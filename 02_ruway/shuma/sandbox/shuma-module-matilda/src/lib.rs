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
use matilda_discover::{discover_inventory, observed_inventory, ServerState};
use matilda_ghost::{dry_run, ApplyReport};
use matilda_linker::{Linker, SshAuth, SshConfig};
use matilda_plan::{plan, Op, Plan};
use shuma_module::{ModuleContributions, MonitorSpec, Rgb, Sample, ShortcutSpec, Source};
use std::path::PathBuf;
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
    /// Descubre el inventario actual del servidor (local; los Remote
    /// los maneja el chasis vía `discover_remote_blocking` y reenvía
    /// el resultado como `SetCurrent`).
    Discover,
    /// Recalcula el plan deseado-vs-actual.
    MakePlan,
    /// Ejecuta `dry_run` sobre los pasos del plan y vuelca al log.
    DryRun,
    /// Setter directo del inventario actual — usado para inyectar el
    /// resultado del discover remoto desde el chasis (cuando el SSH
    /// terminó en un thread aparte).
    SetCurrent(Inventory),
    /// Línea informativa para el log — útil para que el chasis avise
    /// "conectando", "fallo de SSH", etc., sin acoplarse al módulo.
    LogLine(String),
    /// Drag del splitter inventario|plan.
    ResizeSplit(f32),
}

/// Mapea el `action_id` de un `ShortcutAction::ModuleAction` al `Msg`
/// que corresponde. Retorna `None` si el action_id no pertenece a este
/// módulo — el chasis simplemente lo ignora.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "matilda.discover" => Some(Msg::Discover),
        "matilda.plan" => Some(Msg::MakePlan),
        "matilda.dry_run" => Some(Msg::DryRun),
        _ => None,
    }
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
                // El discover remoto necesita un runtime tokio y vive
                // en un thread del chasis (ver `discover_remote_blocking`).
                // Aquí sólo registramos que el módulo no puede hacerlo
                // por sí mismo desde el hilo de UI — es informativo.
                s.log.push(format!(
                    "→ discover remoto en {host} delegado al chasis"
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
        Msg::SetCurrent(inv) => {
            s.log.push(format!(
                "✔ current: {} containers, {} vhosts",
                inv.containers().count(),
                inv.vhosts().count()
            ));
            s.current = Some(inv);
        }
        Msg::LogLine(line) => {
            s.log.push(line);
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

// ─── Discover remoto ───────────────────────────────────────────────

/// Ruta default de la clave SSH del usuario; coincide con el matilda CLI.
fn default_ssh_key() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(format!("{home}/.ssh/id_ed25519"))
}

/// Descubre el inventario actual del servidor remoto. **Bloqueante**:
/// crea un runtime tokio efímero, conecta por SSH y corre
/// `docker ps -a --format '{{.Names}}'` + `ls /etc/nginx/sites-enabled`.
/// Pensado para que el chasis lo invoque dentro de `Handle::spawn`
/// (un thread aparte) — no llamar desde el hilo de UI.
///
/// Para Source::Local fallback a `discover_inventory` (no necesita
/// SSH, pero usa el mismo entrypoint para uniformidad).
pub fn discover_remote_blocking(source: &Source, desired: &Inventory) -> Result<Inventory, String> {
    match source {
        Source::Local => Ok(discover_inventory(desired)),
        Source::Remote { host, user, port, .. } => {
            let auth = SshAuth::Key {
                path: default_ssh_key(),
                passphrase: None,
            };
            let mut config = SshConfig::new(host.as_str(), user.as_str(), auth);
            config.port = *port;

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("tokio runtime: {e}"))?;

            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;

                let containers_text = linker
                    .exec("docker ps -a --format '{{.Names}}' 2>/dev/null || true")
                    .await
                    .map_err(|e| format!("docker ps: {e}"))?;
                let vhosts_text = linker
                    .exec("ls -1 /etc/nginx/sites-enabled 2>/dev/null || true")
                    .await
                    .map_err(|e| format!("ls sites-enabled: {e}"))?;

                let state = ServerState {
                    containers: matilda_discover::parse_docker_names(&containers_text),
                    vhosts: matilda_discover::parse_nginx_sites(&vhosts_text),
                };
                Ok(observed_inventory(&state, desired))
            })
        }
    }
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
    fn remote_discover_is_delegated_to_the_chassis() {
        // El módulo no abre SSH desde el update — el chasis es quien
        // spawnea el thread con `discover_remote_blocking`. Aquí
        // verificamos sólo el log informativo.
        let s = State::new(Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: None,
        });
        let s = update(s, Msg::Discover);
        assert!(s.log.iter().any(|l| l.contains("delegado al chasis")));
        assert!(s.current.is_none());
    }

    #[test]
    fn set_current_updates_state_and_logs() {
        let mut s = State::new(Source::Local);
        let mut inv = matilda_core::Inventory::new();
        inv.add_container(matilda_core::Container::new("web", "nginx"));
        s = update(s, Msg::SetCurrent(inv));
        assert!(s.current.is_some());
        assert_eq!(s.current.as_ref().unwrap().containers().count(), 1);
        assert!(s.log.iter().any(|l| l.contains("1 containers")));
    }

    #[test]
    fn log_line_appends_and_caps_at_200() {
        let mut s = State::new(Source::Local);
        for i in 0..250 {
            s = update(s, Msg::LogLine(format!("line {i}")));
        }
        assert_eq!(s.log.len(), 200);
        // Las primeras 50 líneas deben haberse descartado.
        assert!(s.log[0].contains("line 50"));
    }

    #[test]
    fn discover_remote_blocking_local_falls_back_to_local() {
        // Para `Source::Local` no abre SSH — `discover_inventory` corre
        // localmente. En CI sin docker, retorna inventory vacío sin error.
        let inv = matilda_core::Inventory::new();
        let res = discover_remote_blocking(&Source::Local, &inv);
        assert!(res.is_ok());
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
    fn dispatch_maps_action_ids() {
        assert!(matches!(dispatch("matilda.discover"), Some(Msg::Discover)));
        assert!(matches!(dispatch("matilda.plan"), Some(Msg::MakePlan)));
        assert!(matches!(dispatch("matilda.dry_run"), Some(Msg::DryRun)));
        assert!(dispatch("desconocido").is_none());
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
