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
use matilda_apply::{plan_to_steps, ContainerAction, ServiceAction};
use matilda_core::{Container, Host, Inventory, RestartPolicy, VHost};
use matilda_discover::{
    discover_inventory, discover_runtime, observed_inventory, RuntimeState, ServerState,
};
use matilda_ghost::{apply, dry_run, ApplyReport};
use matilda_linker::{Linker, SshAuth, SshConfig};
use matilda_plan::{plan, Op, Plan};
use shuma_module::{ModuleContributions, MonitorSpec, Rgb, Sample, ShortcutSpec, Source};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const ID: &str = "matilda";

/// Estado de un host de la flota (M5): el runtime observado de un servidor
/// declarado, o un error si no se pudo alcanzar. `Pending` mientras el
/// fetch por SSH está en vuelo (lo corre el chasis en un thread por host).
#[derive(Debug, Clone)]
pub enum FleetEntry {
    Pending,
    Ready(RuntimeState),
    Failed(String),
}

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
    /// Path al inventario JSON, si vino del shumarc. El módulo lo
    /// expone para que el chasis sepa de dónde recargar al pulsar
    /// «Reload»; el módulo mismo no hace IO, sólo recibe `SetDesired`.
    pub inventory_path: Option<PathBuf>,
    /// Estado runtime observado (qué corre AHORA: estado, status, puertos).
    /// `None` hasta el primer discover. Es la base del monitoreo en vivo,
    /// distinto del inventario declarativo (`desired`/`current`).
    pub runtime: Option<RuntimeState>,
    /// Contenedor seleccionado en el panel — abre la barra de acciones
    /// (start/stop/restart/logs/rm). `None` = nada seleccionado.
    pub selected_container: Option<String>,
    /// Servicio systemd seleccionado — abre su barra de acciones.
    pub selected_service: Option<String>,
    /// Flota (M5): runtime por host declarado (`name` → estado). Lo llena el
    /// chasis vía SSH, un host por thread. Vacío hasta el primer Refresh.
    pub fleet: std::collections::BTreeMap<String, FleetEntry>,
    /// Host de la flota seleccionado — expande sus contenedores/servicios.
    pub selected_host: Option<String>,
    /// Hosts de la flota con un fetch SSH en vuelo — guarda anti-apilamiento
    /// del polling periódico (M5): un host colgado no debe acumular threads
    /// tick tras tick. Lo comparte el chasis con el thread de polling; el
    /// thread se borra a sí mismo al terminar. Vacío = nada en vuelo.
    pub fleet_poll_inflight: Arc<Mutex<std::collections::HashSet<String>>>,
    /// `true` mientras un fetch de runtime del Source montado remoto está en
    /// vuelo (M4) — guard anti-apilamiento del polling, igual criterio que
    /// `fleet_poll_inflight` pero para el host montado. Compartido con el
    /// thread, que lo baja al terminar.
    pub runtime_poll_inflight: Arc<std::sync::atomic::AtomicBool>,
    /// Contenedor de la flota seleccionado dentro del host expandido — abre
    /// la barra de acciones remotas (M5). Scoped al `selected_host`; se
    /// limpia al cambiar de host. `None` = nada seleccionado.
    pub selected_fleet_container: Option<String>,
    /// Servicio de la flota seleccionado dentro del host expandido — abre su
    /// barra de acciones remotas. Scoped al `selected_host`.
    pub selected_fleet_service: Option<String>,
    pending_steps: Arc<Mutex<usize>>,
    /// `(up, down)` compartido con el sampler del monitor de runtime.
    runtime_counts: Arc<Mutex<(usize, usize)>>,
}

impl State {
    pub fn new(source: Source) -> Self {
        Self::with_inventory(source, example_inventory())
    }

    /// Variante de `new` con inventario explícito — usada por el chasis
    /// cuando el `shumarc` declara `inventory = "path/to/inv.json"`.
    pub fn with_inventory(source: Source, desired: Inventory) -> Self {
        Self {
            source,
            desired,
            current: None,
            plan: None,
            log: Vec::new(),
            split_width: 380.0,
            inventory_path: None,
            runtime: None,
            selected_container: None,
            selected_service: None,
            fleet: std::collections::BTreeMap::new(),
            selected_host: None,
            fleet_poll_inflight: Arc::new(Mutex::new(std::collections::HashSet::new())),
            runtime_poll_inflight: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            selected_fleet_container: None,
            selected_fleet_service: None,
            pending_steps: Arc::new(Mutex::new(0)),
            runtime_counts: Arc::new(Mutex::new((0, 0))),
        }
    }

    /// Como `with_inventory`, recordando además el path para reloads.
    pub fn with_inventory_path(source: Source, desired: Inventory, path: PathBuf) -> Self {
        let mut s = Self::with_inventory(source, desired);
        s.inventory_path = Some(path);
        s
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

    /// Fija el estado runtime observado y publica `(up, down)` al sampler
    /// del monitor (el thread de polling lee el `Arc` sin tocar el UI).
    pub fn set_runtime(&mut self, rt: RuntimeState) {
        *self.runtime_counts.lock().unwrap() = (rt.up_count(), rt.down_count());
        self.runtime = Some(rt);
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
    /// Aplica el plan al servidor (local sincrónico; remoto delega al
    /// chasis, que spawnea el thread SSH y devuelve `ApplyReport`).
    Apply,
    /// Setter directo del inventario actual — usado para inyectar el
    /// resultado del discover remoto desde el chasis (cuando el SSH
    /// terminó en un thread aparte).
    SetCurrent(Inventory),
    /// Inyecta el estado runtime observado — usado para el discover remoto
    /// (el chasis corre `docker ps` por SSH en un thread y reenvía esto).
    SetRuntime(RuntimeState),
    /// Como `SetRuntime` pero **sin loguear** — para el polling periódico
    /// (M4), que no debe spamear el log cada 5 s.
    SetRuntimeQuiet(RuntimeState),
    /// Línea informativa para el log — útil para que el chasis avise
    /// "conectando", "fallo de SSH", etc., sin acoplarse al módulo.
    LogLine(String),
    /// Varias líneas de una sola vez — el chasis vuelca acá la salida de una
    /// acción remota (`container_action_remote_blocking`) que corrió en un
    /// thread. Equivale a N `LogLine` pero en un único Msg.
    LogLines(Vec<String>),
    /// Inyecta el reporte de un dry-run remoto que el chasis corrió en
    /// un thread aparte (cada `String` es una línea del log).
    DryRunReport(Vec<String>),
    /// Inyecta el reporte de un apply remoto: líneas para el log y, si
    /// la aplicación fue completa, el nuevo inventario actual (re-
    /// descubierto post-apply) para resetear plan + pendientes.
    ApplyReport {
        lines: Vec<String>,
        new_current: Option<Inventory>,
    },
    /// Reemplaza el inventario deseado — usado por el chasis tras un
    /// reload exitoso desde disco. Invalida el plan vigente.
    SetDesired(Inventory),
    /// Drag del splitter inventario|plan.
    ResizeSplit(f32),
    /// Click en un contenedor: lo selecciona (toggle) y abre su barra de
    /// acciones. Re-clickear el mismo lo deselecciona.
    SelectContainer(String),
    /// Acción de ciclo de vida sobre un contenedor (start/stop/restart/
    /// logs/rm). Local sincrónico; remoto delegado al chasis.
    ContainerActionMsg { name: String, action: ContainerAction },
    /// Click en un servicio systemd: lo selecciona (toggle).
    SelectService(String),
    /// Acción sobre un servicio systemd (start/stop/restart/enable/disable/
    /// status). Local sincrónico; remoto delegado al chasis.
    ServiceActionMsg { name: String, action: ServiceAction },
    /// M5 — refrescar la flota: marca cada host declarado como `Pending`.
    /// El chasis spawnea el fetch por SSH (uno por host) y reenvía
    /// `SetHostRuntime`/`SetHostError`.
    RefreshFleet,
    /// Resultado del fetch de un host de la flota.
    SetHostRuntime { host: String, runtime: RuntimeState },
    /// Error al alcanzar un host de la flota.
    SetHostError { host: String, error: String },
    /// Como `SetHostRuntime`/`SetHostError` pero **sin loguear** — los usa el
    /// polling periódico de la flota (M5), que refresca cada host cada ~30 s y
    /// no debe spamear el log ni parpadear el host a «consultando».
    SetHostRuntimeQuiet { host: String, runtime: RuntimeState },
    /// Variante silenciosa del error de host para el polling de la flota.
    SetHostErrorQuiet { host: String, error: String },
    /// Click en un host de la flota: lo selecciona (toggle) y expande su
    /// runtime (contenedores + servicios).
    SelectHost(String),
    /// Click en un contenedor dentro del host expandido de la flota: lo
    /// selecciona (toggle) y abre su barra de acciones remotas.
    SelectFleetContainer(String),
    /// Click en un servicio dentro del host expandido de la flota.
    SelectFleetService(String),
    /// M5 — acción de ciclo de vida sobre un contenedor de un host de la
    /// flota. Siempre remota: el módulo sólo registra la intención en el log;
    /// el chasis la toma, corre el SSH en un thread (`fleet_container_action_
    /// blocking`) y re-observa el host (`SetHostRuntime`).
    FleetContainerAction { host: String, name: String, action: ContainerAction },
    /// M5 — acción sobre un servicio systemd de un host de la flota.
    FleetServiceAction { host: String, name: String, action: ServiceAction },
    /// M5 — resultado de una acción de flota que el chasis corrió por SSH:
    /// líneas para el log y, si fue mutante y exitosa, el runtime re-observado
    /// del host para refrescar su `FleetEntry` sin volver a pulsar «Fleet».
    FleetActionDone {
        host: String,
        lines: Vec<String>,
        runtime: Option<RuntimeState>,
    },
}

/// Mapea el `action_id` de un `ShortcutAction::ModuleAction` al `Msg`
/// que corresponde. Retorna `None` si el action_id no pertenece a este
/// módulo — el chasis simplemente lo ignora.
pub fn dispatch(action_id: &str) -> Option<Msg> {
    match action_id {
        "matilda.discover" => Some(Msg::Discover),
        "matilda.plan" => Some(Msg::MakePlan),
        "matilda.dry_run" => Some(Msg::DryRun),
        "matilda.apply" => Some(Msg::Apply),
        _ => None,
    }
}

pub fn update(state: State, msg: Msg) -> State {
    let mut s = state;
    match msg {
        Msg::Discover => match &s.source {
            Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
                // Matilda no habla todavía con el daemon de shuma — corre
                // siempre sobre el FS local cuando no es SSH.
                let current = discover_inventory(&s.desired);
                s.log.push(format!(
                    "✔ discover local: {} containers, {} vhosts",
                    current.containers().count(),
                    current.vhosts().count()
                ));
                s.current = Some(current);
                // Además del inventario declarativo, capturamos el estado
                // runtime (qué corre, parado, sus puertos) para el monitoreo.
                let rt = discover_runtime();
                s.log.push(format!(
                    "  runtime: {} up · {} down",
                    rt.up_count(),
                    rt.down_count()
                ));
                s.set_runtime(rt);
            }
            Source::Remote { host, .. } | Source::RemoteContainer { host, .. } => {
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
            // El dry-run para Source::Remote vive en el chasis (necesita
            // SSH + thread); aquí sólo manejamos Local sincrónicamente.
            // Para Remote, el chasis interceptó el shortcut y dispatchó
            // el resultado vía `Msg::DryRunReport`.
            if s.source.is_remote() {
                s.log
                    .push("→ dry-run remoto delegado al chasis".into());
            } else {
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
            }
            cap_log(&mut s.log);
        }
        Msg::Apply => {
            if s.source.is_remote() {
                s.log
                    .push("→ apply remoto delegado al chasis".into());
            } else {
                let p = match &s.plan {
                    Some(p) => p.clone(),
                    None => plan(&s.current_or_empty(), &s.desired),
                };
                let steps = plan_to_steps(&p, &s.desired);
                if steps.is_empty() {
                    s.log.push("Sin pasos: nada que aplicar.".into());
                } else {
                    s.log.push(format!("— aplicando {} pasos —", steps.len()));
                    let report: ApplyReport = apply(&steps);
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
                    s.log.push(format!(
                        "{} de {} pasos aplicados.",
                        report.applied(),
                        report.results.len()
                    ));
                    if report.all_ok() {
                        // Re-discover local para resetear plan + pendientes.
                        let current = discover_inventory(&s.desired);
                        let new_plan = plan(&current, &s.desired);
                        *s.pending_steps.lock().unwrap() = new_plan.len();
                        s.current = Some(current);
                        s.plan = Some(new_plan);
                    } else {
                        s.log.push("✘ se detuvo en el primer error.".into());
                    }
                }
            }
            cap_log(&mut s.log);
        }
        Msg::DryRunReport(lines) => {
            for line in lines {
                s.log.push(line);
            }
            cap_log(&mut s.log);
        }
        Msg::ApplyReport { lines, new_current } => {
            for line in lines {
                s.log.push(line);
            }
            if let Some(inv) = new_current {
                let new_plan = plan(&inv, &s.desired);
                *s.pending_steps.lock().unwrap() = new_plan.len();
                s.current = Some(inv);
                s.plan = Some(new_plan);
            }
            cap_log(&mut s.log);
        }
        Msg::SetCurrent(inv) => {
            s.log.push(format!(
                "✔ current: {} containers, {} vhosts",
                inv.containers().count(),
                inv.vhosts().count()
            ));
            s.current = Some(inv);
        }
        Msg::SetRuntime(rt) => {
            s.log.push(format!(
                "✔ runtime: {} up · {} down",
                rt.up_count(),
                rt.down_count()
            ));
            s.set_runtime(rt);
        }
        Msg::SetRuntimeQuiet(rt) => {
            s.set_runtime(rt);
        }
        Msg::LogLine(line) => {
            s.log.push(line);
            cap_log(&mut s.log);
        }
        Msg::LogLines(lines) => {
            for l in lines {
                s.log.push(l);
            }
            cap_log(&mut s.log);
        }
        Msg::SetDesired(inv) => {
            s.log.push(format!(
                "✔ inventario recargado: {} hosts, {} containers, {} vhosts",
                inv.hosts().count(),
                inv.containers().count(),
                inv.vhosts().count()
            ));
            s.desired = inv;
            s.plan = None;
            *s.pending_steps.lock().unwrap() = 0;
        }
        Msg::ResizeSplit(dx) => {
            s.split_width = (s.split_width + dx).clamp(220.0, 720.0);
        }
        Msg::SelectContainer(name) => {
            s.selected_container = if s.selected_container.as_deref() == Some(name.as_str()) {
                None
            } else {
                Some(name)
            };
        }
        Msg::ContainerActionMsg { name, action } => {
            if s.source.is_remote() {
                // El remoto necesita SSH + thread: lo corre el chasis y
                // reenvía el log por `Msg::LogLine` (ver
                // `container_action_remote_blocking`).
                s.log.push(format!(
                    "→ {} {name} remoto delegado al chasis",
                    action.label()
                ));
            } else {
                let cmd = action.command(&name);
                s.log.push(format!("$ {cmd}"));
                let (ok, out) = run_shell_capture(&cmd);
                for line in out.into_iter().take(30) {
                    s.log.push(format!("   {line}"));
                }
                s.log.push(if ok {
                    format!("✔ {} {name}", action.label())
                } else {
                    format!("✘ {} {name} falló", action.label())
                });
                // Una acción mutante cambia el runtime: lo re-observamos para
                // que el semáforo del panel quede al día sin pulsar Discover.
                if ok && action.is_mutating() {
                    s.set_runtime(discover_runtime());
                }
            }
            cap_log(&mut s.log);
        }
        Msg::SelectService(name) => {
            s.selected_service = if s.selected_service.as_deref() == Some(name.as_str()) {
                None
            } else {
                Some(name)
            };
        }
        Msg::ServiceActionMsg { name, action } => {
            if s.source.is_remote() {
                s.log.push(format!(
                    "→ {} {name} remoto delegado al chasis",
                    action.label()
                ));
            } else {
                let cmd = action.command(&name);
                s.log.push(format!("$ {cmd}"));
                let (ok, out) = run_shell_capture(&cmd);
                for line in out.into_iter().take(30) {
                    s.log.push(format!("   {line}"));
                }
                s.log.push(if ok {
                    format!("✔ {} {name}", action.label())
                } else {
                    format!("✘ {} {name} falló (¿privilegios?)", action.label())
                });
                if ok && action.is_mutating() {
                    s.set_runtime(discover_runtime());
                }
            }
            cap_log(&mut s.log);
        }
        Msg::RefreshFleet => {
            // Marcamos cada host declarado como Pending; el chasis dispara el
            // fetch por SSH (un thread por host) y reenvía los resultados.
            s.fleet.clear();
            for h in s.desired.hosts() {
                s.fleet.insert(h.name.clone(), FleetEntry::Pending);
            }
            s.log.push(format!("→ refrescando flota ({} hosts)…", s.fleet.len()));
            cap_log(&mut s.log);
        }
        Msg::SetHostRuntime { host, runtime } => {
            s.log.push(format!(
                "✔ {host}: {} up · {} down · {} svc",
                runtime.up_count(),
                runtime.down_count(),
                runtime.services.len()
            ));
            s.fleet.insert(host, FleetEntry::Ready(runtime));
            cap_log(&mut s.log);
        }
        Msg::SetHostError { host, error } => {
            s.log.push(format!("✘ {host}: {error}"));
            s.fleet.insert(host, FleetEntry::Failed(error));
            cap_log(&mut s.log);
        }
        Msg::SetHostRuntimeQuiet { host, runtime } => {
            s.fleet.insert(host, FleetEntry::Ready(runtime));
        }
        Msg::SetHostErrorQuiet { host, error } => {
            s.fleet.insert(host, FleetEntry::Failed(error));
        }
        Msg::SelectHost(name) => {
            s.selected_host = if s.selected_host.as_deref() == Some(name.as_str()) {
                None
            } else {
                Some(name)
            };
            // Cambiar de host expandido invalida la selección de recurso de la
            // flota — sus action bars pertenecen al host anterior.
            s.selected_fleet_container = None;
            s.selected_fleet_service = None;
        }
        Msg::SelectFleetContainer(name) => {
            s.selected_fleet_container =
                if s.selected_fleet_container.as_deref() == Some(name.as_str()) {
                    None
                } else {
                    s.selected_fleet_service = None;
                    Some(name)
                };
        }
        Msg::SelectFleetService(name) => {
            s.selected_fleet_service =
                if s.selected_fleet_service.as_deref() == Some(name.as_str()) {
                    None
                } else {
                    s.selected_fleet_container = None;
                    Some(name)
                };
        }
        Msg::FleetContainerAction { host, name, action } => {
            // Siempre remota: el SSH lo corre el chasis en un thread y reenvía
            // el log + el `SetHostRuntime` re-observado.
            s.log.push(format!("→ {} {name} en {host} (flota) delegado al chasis", action.label()));
            cap_log(&mut s.log);
        }
        Msg::FleetServiceAction { host, name, action } => {
            s.log.push(format!("→ {} {name} en {host} (flota) delegado al chasis", action.label()));
            cap_log(&mut s.log);
        }
        Msg::FleetActionDone { host, lines, runtime } => {
            for l in lines {
                s.log.push(l);
            }
            cap_log(&mut s.log);
            // Si la acción re-observó el host, su `FleetEntry` queda al día.
            if let Some(rt) = runtime {
                s.fleet.insert(host, FleetEntry::Ready(rt));
            }
        }
    }
    s
}

/// M5 — fetch del runtime de un host de la flota por SSH. **Bloqueante**:
/// conecta por SSH (usuario/puerto del `Host`, clave default) y corre
/// `docker ps` + `systemctl` + `ls sites-enabled`, parseando con los mismos
/// parsers del discover local. Pensado para que el chasis lo corra en un
/// thread por host y reenvíe `Msg::SetHostRuntime`/`SetHostError`.
pub fn host_runtime_remote_blocking(host: &Host) -> Result<RuntimeState, String> {
    let config = ssh_config_for_host(host);
    let rt = blocking_runtime()?;
    rt.block_on(async move {
        let linker = Linker::connect(&config)
            .await
            .map_err(|e| format!("ssh connect: {e}"))?;
        fetch_remote_runtime(&linker).await
    })
}

/// Corre las consultas de runtime (`docker ps` + `systemctl` + `ls
/// sites-enabled`) sobre un `Linker` ya conectado y arma el `RuntimeState`
/// con los mismos parsers del discover local. Compartido por el fetch de un
/// host de la flota (M5) y el polling del Source montado remoto (M4).
async fn fetch_remote_runtime(linker: &Linker) -> Result<RuntimeState, String> {
    let ps = linker
        .exec(&format!(
            "docker ps -a --format '{}' 2>/dev/null || true",
            matilda_discover::DOCKER_PS_FORMAT
        ))
        .await
        .map_err(|e| format!("docker ps: {e}"))?;
    let svc = linker
        .exec(
            "systemctl list-units --type=service --state=running,failed \
             --no-legend --plain 2>/dev/null || true",
        )
        .await
        .map_err(|e| format!("systemctl: {e}"))?;
    let nginx = linker
        .exec("ls -1 /etc/nginx/sites-enabled 2>/dev/null || true")
        .await
        .map_err(|e| format!("ls sites-enabled: {e}"))?;
    Ok(RuntimeState {
        containers: matilda_discover::parse_docker_ps(&ps),
        services: matilda_discover::parse_systemctl_units(&svc),
        vhosts: matilda_discover::parse_nginx_sites(&nginx),
    })
}

/// M4 — re-observa el runtime del **Source montado** cuando es remoto (lo que
/// `poll_runtime` hace local). **Bloqueante**: el chasis lo corre en un thread
/// a cadencia lenta y reenvía `Msg::SetRuntimeQuiet`. Para Source local cae a
/// `discover_runtime` por uniformidad (sin abrir SSH).
pub fn source_runtime_remote_blocking(source: &Source) -> Result<RuntimeState, String> {
    match source {
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            Ok(discover_runtime())
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                fetch_remote_runtime(&linker).await
            })
        }
    }
}

/// Config SSH para un `Host` de la flota: clave default del usuario +
/// usuario/puerto declarados en el inventario. Espeja la conexión de
/// `host_runtime_remote_blocking` para que acción y discovery usen el mismo
/// criterio de credenciales.
fn ssh_config_for_host(host: &Host) -> SshConfig {
    let auth = SshAuth::Key { path: default_ssh_key(), passphrase: None };
    let mut config = SshConfig::new(host.address.as_str(), host.ssh_user(), auth);
    config.port = host.ssh_port();
    config
}

/// M5 — corre un comando de acción contra un host de la flota por SSH.
/// **Bloqueante**: pensado para que el chasis lo corra en un thread y reenvíe
/// las líneas por `Msg::LogLine`. Devuelve `(éxito, líneas)` — el éxito sale
/// del exit code real del comando remoto (no de la conexión).
fn host_action_blocking(host: &Host, cmd: &str, label: &str, name: &str) -> (bool, Vec<String>) {
    let config = ssh_config_for_host(host);
    let rt = match blocking_runtime() {
        Ok(rt) => rt,
        Err(e) => return (false, vec![format!("✘ runtime: {e}")]),
    };
    // `; echo __rc:$?` deja el exit code del comando en la última línea para
    // distinguir "conectó pero el comando falló" de "no conectó".
    let probe = format!("{cmd} 2>&1; echo __rc:$?");
    let result: Result<String, String> = rt.block_on(async move {
        let linker = Linker::connect(&config)
            .await
            .map_err(|e| format!("ssh connect: {e}"))?;
        linker.exec(&probe).await.map_err(|e| format!("{cmd}: {e}"))
    });
    match result {
        Ok(text) => {
            let mut ok = true;
            let mut lines = vec![format!("$ {cmd}")];
            for l in text.lines() {
                if let Some(rc) = l.strip_prefix("__rc:") {
                    ok = rc.trim() == "0";
                } else {
                    lines.push(l.to_string());
                }
            }
            // Cap defensivo: un `docker logs` largo no debe inundar el log.
            lines.truncate(31);
            lines.push(if ok {
                format!("✔ {label} {name} en {} (remoto)", host.name)
            } else {
                format!("✘ {label} {name} en {} falló", host.name)
            });
            (ok, lines)
        }
        Err(e) => (false, vec![format!("✘ {label} {name} en {}: {e}", host.name)]),
    }
}

/// M5 — acción de ciclo de vida sobre un contenedor de un host de la flota.
pub fn fleet_container_action_blocking(
    host: &Host,
    name: &str,
    action: ContainerAction,
) -> (bool, Vec<String>) {
    host_action_blocking(host, &action.command(name), action.label(), name)
}

/// M5 — acción sobre un servicio systemd de un host de la flota.
pub fn fleet_service_action_blocking(
    host: &Host,
    name: &str,
    action: ServiceAction,
) -> (bool, Vec<String>) {
    host_action_blocking(host, &action.command(name), action.label(), name)
}

/// Ejecuta un comando de shell local y captura stdout+stderr como líneas.
/// Devuelve `(éxito, líneas)`. Usado por las acciones de ciclo de vida de
/// contenedores (`docker start/stop/…`); el remoto va por `Linker`.
fn run_shell_capture(cmd: &str) -> (bool, Vec<String>) {
    match std::process::Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(out) => {
            let mut lines: Vec<String> = Vec::new();
            for l in String::from_utf8_lossy(&out.stdout).lines() {
                lines.push(l.to_string());
            }
            for l in String::from_utf8_lossy(&out.stderr).lines() {
                lines.push(l.to_string());
            }
            (out.status.success(), lines)
        }
        Err(e) => (false, vec![format!("no se pudo ejecutar: {e}")]),
    }
}

/// Ejecuta una acción de ciclo de vida en el servidor remoto por SSH.
/// **Bloqueante** — pensado para que el chasis lo corra en un thread y
/// reenvíe las líneas por `Msg::LogLine`. Devuelve las líneas del log.
pub fn container_action_remote_blocking(
    source: &Source,
    name: &str,
    action: ContainerAction,
) -> Result<Vec<String>, String> {
    let cmd = action.command(name);
    match source {
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            let (ok, mut out) = run_shell_capture(&cmd);
            out.insert(0, format!("$ {cmd}"));
            out.push(if ok { format!("✔ {} {name}", action.label()) } else { format!("✘ {} {name} falló", action.label()) });
            Ok(out)
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                let text = linker
                    .exec(&format!("{cmd} 2>&1"))
                    .await
                    .map_err(|e| format!("{cmd}: {e}"))?;
                let mut lines = vec![format!("$ {cmd}")];
                lines.extend(text.lines().take(30).map(str::to_string));
                lines.push(format!("✔ {} {name} (remoto)", action.label()));
                Ok(lines)
            })
        }
    }
}

/// Ejecuta una acción sobre el Source montado, dado el comando ya armado y
/// su etiqueta. **Bloqueante** — el chasis lo corre en un thread y vuelca las
/// líneas por `Msg::LogLines`. Generaliza el path de servicios (que no tiene
/// un enum-con-`command()` propio para el Source remoto como contenedores).
pub fn service_action_remote_blocking(
    source: &Source,
    cmd: &str,
    label: &str,
    name: &str,
) -> Result<Vec<String>, String> {
    match source {
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            let (ok, mut out) = run_shell_capture(cmd);
            out.insert(0, format!("$ {cmd}"));
            out.push(if ok { format!("✔ {label} {name}") } else { format!("✘ {label} {name} falló") });
            Ok(out)
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            let cmd = cmd.to_string();
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                let text = linker
                    .exec(&format!("{cmd} 2>&1"))
                    .await
                    .map_err(|e| format!("{cmd}: {e}"))?;
                let mut lines = vec![format!("$ {cmd}")];
                lines.extend(text.lines().take(30).map(str::to_string));
                lines.push(format!("✔ {label} {name} (remoto)"));
                Ok(lines)
            })
        }
    }
}

fn cap_log(log: &mut Vec<String>) {
    const MAX: usize = 200;
    let len = log.len();
    if len > MAX {
        log.drain(0..len - MAX);
    }
}

// ─── Discover y dry-run remotos ─────────────────────────────────────

/// Re-observa el estado runtime local (`docker ps` + `systemctl`). El
/// chasis lo llama en un thread a cadencia lenta (M4 — polling) y reenvía
/// el resultado por `Msg::SetRuntime`. Es lo más barato del discover (no
/// corre `docker inspect` por contenedor como `discover_inventory`).
pub fn poll_runtime() -> RuntimeState {
    discover_runtime()
}

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
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            Ok(discover_inventory(desired))
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                fetch_remote_inventory(&linker, desired).await
            })
        }
    }
}

/// Equivalente remoto de `Msg::DryRun`: conecta por SSH, descubre el
/// inventory actual, calcula el plan deseado-vs-actual y enumera los
/// pasos que SE EJECUTARÍAN — sin invocar ninguno. Útil para validar
/// que el `Source::Remote` está bien configurado y previsualizar el
/// cambio antes de un eventual Apply real (fuera de scope aquí).
///
/// Devuelve un `Vec<String>` con líneas listas para insertar al log
/// (incluyendo el reporte de dry-run de cada paso). El chasis las
/// envuelve en `Msg::DryRunReport`.
pub fn dry_run_remote_blocking(
    source: &Source,
    desired: &Inventory,
) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();

    let current = match source {
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            discover_inventory(desired)
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                fetch_remote_inventory(&linker, desired).await
            })?
        }
    };
    lines.push(format!(
        "✔ current: {} containers, {} vhosts",
        current.containers().count(),
        current.vhosts().count()
    ));

    let p = plan(&current, desired);
    if p.is_empty() {
        lines.push("Sin cambios: el servidor ya está al día.".into());
        return Ok(lines);
    }
    lines.push(format!(
        "plan: {} acciones ({} crear, {} actualizar, {} eliminar)",
        p.len(),
        p.count(Op::Create),
        p.count(Op::Update),
        p.count(Op::Remove)
    ));

    let steps = plan_to_steps(&p, desired);
    let report: ApplyReport = dry_run(&steps);
    for r in &report.results {
        lines.push(format!(
            "{} {}",
            if r.ok { "✔" } else { "✘" },
            r.describe
        ));
        for line in &r.log {
            lines.push(format!("   {line}"));
        }
    }
    Ok(lines)
}

/// Aplica el plan deseado-vs-actual en el servidor remoto: conecta por
/// SSH, descubre el inventario, calcula el plan, ejecuta los pasos en
/// orden y re-descubre el estado final. **Bloqueante** — pensado para
/// que el chasis lo invoque dentro de `Handle::spawn` y reenvíe el
/// resultado por `Msg::ApplyReport`.
///
/// Devuelve `(lines, new_current)`: el log textual y, si todos los
/// pasos completaron, el inventario re-observado (para resetear el
/// plan/pendientes del módulo). Si algún paso falla, `new_current` es
/// `None` — la UI conserva el plan vigente para que el operador vea
/// dónde se rompió.
pub fn apply_remote_blocking(
    source: &Source,
    desired: &Inventory,
) -> Result<(Vec<String>, Option<Inventory>), String> {
    let mut lines = Vec::new();

    match source {
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            // Local lo maneja `Msg::Apply` sincrónicamente. Para
            // uniformidad damos un fallback síncrono sin tocar el UI.
            let current = discover_inventory(desired);
            let p = plan(&current, desired);
            if p.is_empty() {
                lines.push("Sin cambios: nada que aplicar.".into());
                return Ok((lines, Some(current)));
            }
            let steps = plan_to_steps(&p, desired);
            let report: ApplyReport = apply(&steps);
            push_apply_log(&mut lines, &report);
            let new_current = if report.all_ok() {
                Some(discover_inventory(desired))
            } else {
                None
            };
            Ok((lines, new_current))
        }
        Source::Remote { .. } | Source::RemoteContainer { .. } => {
            let config = ssh_config_for(source)?;
            let rt = blocking_runtime()?;
            rt.block_on(async move {
                let linker = Linker::connect(&config)
                    .await
                    .map_err(|e| format!("ssh connect: {e}"))?;
                let current = fetch_remote_inventory(&linker, desired).await?;
                lines.push(format!(
                    "✔ current: {} containers, {} vhosts",
                    current.containers().count(),
                    current.vhosts().count()
                ));
                let p = plan(&current, desired);
                if p.is_empty() {
                    lines.push("Sin cambios: el servidor ya está al día.".into());
                    return Ok((lines, Some(current)));
                }
                lines.push(format!(
                    "plan: {} acciones ({} crear, {} actualizar, {} eliminar)",
                    p.len(),
                    p.count(Op::Create),
                    p.count(Op::Update),
                    p.count(Op::Remove)
                ));
                let steps = plan_to_steps(&p, desired);
                lines.push(format!("— aplicando {} pasos por SSH —", steps.len()));
                let report = linker.apply(&steps).await;
                push_apply_log(&mut lines, &report);
                let new_current = if report.all_ok() {
                    Some(fetch_remote_inventory(&linker, desired).await?)
                } else {
                    None
                };
                Ok((lines, new_current))
            })
        }
    }
}

fn push_apply_log(lines: &mut Vec<String>, report: &ApplyReport) {
    for r in &report.results {
        lines.push(format!(
            "{} {}",
            if r.ok { "✔" } else { "✘" },
            r.describe
        ));
        for line in &r.log {
            lines.push(format!("   {line}"));
        }
    }
    lines.push(format!(
        "{} de {} pasos aplicados.",
        report.applied(),
        report.results.len()
    ));
    if !report.all_ok() {
        lines.push("✘ se detuvo en el primer error.".into());
    }
}

fn ssh_config_for(source: &Source) -> Result<SshConfig, String> {
    match source {
        Source::Remote { host, user, port, .. }
        | Source::RemoteContainer { host, user, port, .. } => {
            let auth = SshAuth::Key {
                path: default_ssh_key(),
                passphrase: None,
            };
            let mut config = SshConfig::new(host.as_str(), user.as_str(), auth);
            config.port = *port;
            Ok(config)
        }
        Source::Local | Source::Daemon { .. } | Source::DaemonTcp { .. } | Source::Container { .. } => {
            Err("ssh_config_for esperaba Source::Remote".into())
        }
    }
}

fn blocking_runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime: {e}"))
}

async fn fetch_remote_inventory(
    linker: &Linker,
    desired: &Inventory,
) -> Result<Inventory, String> {
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
        // Servicios declarativos por SSH: v1 no los consulta (cada uno sería
        // un round-trip). El plan los verá como Create → `enable --now`, que
        // es idempotente. Consultar el estado remoto queda pendiente.
        services: Vec::new(),
    };
    Ok(observed_inventory(&state, desired))
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
    inv.add_service(matilda_core::Service::new("nginx"));
    inv
}

// ─── view ──────────────────────────────────────────────────────────

pub fn view<HostMsg: Clone + Send + Sync + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let header = matilda_header(state, theme);

    let inv_pane = inventory_pane(state, theme, lift.clone());
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

/// Panel izquierdo: el inventario en 3 secciones (hosts / containers /
/// vhosts). Las filas de contenedor son **clickeables**: seleccionan el
/// contenedor y abren la barra de acciones (start/stop/restart/logs/rm).
fn inventory_pane<HostMsg: Clone + Send + Sync + 'static>(
    state: &State,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    let mut children: Vec<View<HostMsg>> = Vec::new();

    // FLEET (M5) — los hosts declarados con su runtime por SSH. Cada host es
    // clickeable: lo selecciona y expande sus contenedores/servicios.
    children.push(section_label(
        &format!("FLEET ({} hosts)", state.desired.hosts().count()),
        theme,
    ));
    for h in state.desired.hosts() {
        let entry = state.fleet.get(&h.name);
        let sel = state.selected_host.as_deref() == Some(h.name.as_str());
        children.push(host_row(h, entry, sel, theme, lift.clone()));
        // Expandido: el runtime del host (contenedores + servicios). Cada
        // recurso es clickeable → abre su barra de acciones REMOTAS (M5): el
        // chasis corre `docker`/`systemctl` por SSH contra ESTE host.
        if sel {
            if let Some(FleetEntry::Ready(rt)) = entry {
                for c in &rt.containers {
                    let csel = state.selected_fleet_container.as_deref() == Some(c.name.as_str());
                    children.push(fleet_resource_row(
                        c.state.glyph(), &c.name, &c.status, c.state.is_up(), csel,
                        Msg::SelectFleetContainer(c.name.clone()), theme, lift.clone(),
                    ));
                    if csel {
                        children.push(fleet_container_action_bar(&h.name, &c.name, theme, lift.clone()));
                    }
                }
                for svc in &rt.services {
                    use matilda_discover::ServiceState;
                    let ok = svc.state == ServiceState::Active;
                    let ssel = state.selected_fleet_service.as_deref() == Some(svc.name.as_str());
                    children.push(fleet_resource_row(
                        svc.state.glyph(), &svc.name, &svc.sub, ok, ssel,
                        Msg::SelectFleetService(svc.name.clone()), theme, lift.clone(),
                    ));
                    if ssel {
                        children.push(fleet_service_action_bar(&h.name, &svc.name, theme, lift.clone()));
                    }
                }
            }
        }
    }

    // CONTAINERS — con estado runtime (●/○ + status) cuando hay discover.
    let cont_label = match &state.runtime {
        Some(rt) => format!(
            "CONTAINERS ({}) · {} up · {} down",
            state.desired.containers().count(),
            rt.up_count(),
            rt.down_count()
        ),
        None => format!(
            "CONTAINERS ({}) · sin discover",
            state.desired.containers().count()
        ),
    };
    children.push(section_label(&cont_label, theme));
    for c in state.desired.containers() {
        let status = state.runtime.as_ref().and_then(|rt| rt.container(&c.name));
        // M6 — drift visible: el discover marca el contenedor desviado con
        // imagen "(desviado)" en `current`. Lo mostramos como chip.
        let drift = matches!(
            state.current.as_ref().and_then(|inv| inv.container(&c.name)),
            Some(cur) if cur.image == "(desviado)"
        );
        children.push(container_row(
            &c.name,
            &c.image,
            status,
            drift,
            state.selected_container.as_deref() == Some(c.name.as_str()),
            theme,
            lift.clone(),
        ));
        // Barra de acciones bajo el contenedor seleccionado.
        if state.selected_container.as_deref() == Some(c.name.as_str()) {
            children.push(container_action_bar(&c.name, theme, lift.clone()));
        }
    }
    // Huérfanos: contenedores que corren pero no están en el inventario
    // deseado — el operador los ve y los opera sin ir a la terminal.
    if let Some(rt) = &state.runtime {
        for cs in &rt.containers {
            if state.desired.container(&cs.name).is_none() {
                let sel = state.selected_container.as_deref() == Some(cs.name.as_str());
                children.push(container_row(
                    &cs.name, &cs.image, Some(cs), false, sel, theme, lift.clone(),
                ));
                if sel {
                    children.push(container_action_bar(&cs.name, theme, lift.clone()));
                }
            }
        }
    }

    // SERVICES — systemd (running/failed), runtime puro + acciones.
    if let Some(rt) = &state.runtime {
        if !rt.services.is_empty() {
            children.push(section_label(
                &format!(
                    "SERVICES ({}) · {} activos · {} fallados",
                    rt.services.len(),
                    rt.services_active(),
                    rt.services_failed()
                ),
                theme,
            ));
            for svc in &rt.services {
                let sel = state.selected_service.as_deref() == Some(svc.name.as_str());
                children.push(service_row(svc, sel, theme, lift.clone()));
                if sel {
                    children.push(service_action_bar(&svc.name, theme, lift.clone()));
                }
            }
        }
    }

    // SERVICES declarados — los del inventario (deseados), con sus flags
    // enable/active y si están corriendo ahora (cross-ref con el runtime).
    // Es la paridad con contenedores/vhosts: el deseo se ve en el panel.
    if state.desired.services().count() > 0 {
        children.push(section_label(
            &format!("SERVICES declarados ({})", state.desired.services().count()),
            theme,
        ));
        for svc in state.desired.services() {
            let corriendo = state
                .runtime
                .as_ref()
                .map(|rt| rt.services.iter().any(|s| s.name == svc.unit && s.state.is_active()))
                .unwrap_or(false);
            let glyph = if corriendo { '●' } else { '◌' };
            let flags = match (svc.enabled, svc.active) {
                (true, true) => "enable+start",
                (true, false) => "enable",
                (false, true) => "start",
                (false, false) => "disable+stop",
            };
            children.push(inv_row(&format!("  {glyph} {}   [{flags}]", svc.unit), theme));
        }
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

/// Fila de un host de la flota (M5): semáforo (● alcanzable / ◐ consultando
/// / ✖ error / ◌ sin consultar) + nombre + dirección + resumen up/down/svc
/// o el error. Clickeable → selecciona y expande su runtime.
fn host_row<HostMsg: Clone + Send + Sync + 'static>(
    host: &Host,
    entry: Option<&FleetEntry>,
    selected: bool,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let green = Color::from_rgba8(0x82, 0xCD, 0x8C, 0xFF);
    let red = Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF);
    let (glyph, color, summary) = match entry {
        None => ('◌', theme.fg_muted, "· sin consultar (pulsá «Fleet»)".to_string()),
        Some(FleetEntry::Pending) => ('◐', theme.fg_muted, "· consultando…".to_string()),
        Some(FleetEntry::Ready(rt)) => {
            let c = if rt.down_count() == 0 && rt.services_failed() == 0 { green } else { red };
            (
                '●',
                c,
                format!(
                    "· {} up · {} down · {} svc",
                    rt.up_count(),
                    rt.down_count(),
                    rt.services.len()
                ),
            )
        }
        Some(FleetEntry::Failed(e)) => {
            let short: String = e.chars().take(40).collect();
            ('✖', red, format!("· ✘ {short}"))
        }
    };
    let prefix = if selected { "▸ " } else { "  " };
    let mut row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::SelectHost(host.name.clone())))
    .text_aligned(
        format!("{prefix}{glyph} {}  {}  {summary}", host.name, host.address),
        11.0,
        color,
        Alignment::Start,
    );
    if selected {
        row = row.fill(theme.bg_row_hover);
    }
    row
}

/// Fila de un recurso dentro de un host expandido de la flota: glifo +
/// nombre + detalle, indentada. **Clickeable** (M5) → emite `select` para
/// abrir la barra de acciones remotas. El prefijo `▸` y el fondo marcan la
/// selección, igual que las filas del Source montado.
#[allow(clippy::too_many_arguments)]
fn fleet_resource_row<HostMsg: Clone + Send + Sync + 'static>(
    glyph: char,
    name: &str,
    detail: &str,
    ok: bool,
    selected: bool,
    select: Msg,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let color = if ok {
        Color::from_rgba8(0x82, 0xCD, 0x8C, 0xFF)
    } else {
        theme.fg_muted
    };
    let tail = if detail.is_empty() {
        String::new()
    } else {
        format!("  · {detail}")
    };
    let prefix = if selected { "    ▸ " } else { "      " };
    let mut row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(select))
    .text_aligned(
        format!("{prefix}{glyph} {name}{tail}"),
        11.0,
        color,
        Alignment::Start,
    );
    if selected {
        row = row.fill(theme.bg_row_hover);
    }
    row
}

/// Barra de acciones remotas para un contenedor de un host de la flota (M5).
/// Idéntica a `container_action_bar` salvo que el click emite
/// `FleetContainerAction { host, … }` — el chasis corre el `docker` por SSH
/// contra `host` y re-observa su runtime.
fn fleet_container_action_bar<HostMsg: Clone + Send + Sync + 'static>(
    host: &str,
    name: &str,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let mut buttons: Vec<View<HostMsg>> = Vec::new();
    for action in ContainerAction::all() {
        let color = if matches!(action, ContainerAction::Remove) {
            Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF)
        } else {
            theme.accent
        };
        buttons.push(
            View::new(Style {
                size: Size { width: length(54.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::FleetContainerAction {
                host: host.to_string(),
                name: name.to_string(),
                action,
            }))
            .text_aligned(action.label().to_string(), 11.0, color, Alignment::Start),
        );
    }
    fleet_action_bar_frame(buttons)
}

/// Barra de acciones remotas para un servicio systemd de un host de la flota.
fn fleet_service_action_bar<HostMsg: Clone + Send + Sync + 'static>(
    host: &str,
    name: &str,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let mut buttons: Vec<View<HostMsg>> = Vec::new();
    for action in ServiceAction::all() {
        let color = if matches!(action, ServiceAction::Stop | ServiceAction::Disable) {
            Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF)
        } else {
            theme.accent
        };
        buttons.push(
            View::new(Style {
                size: Size { width: length(60.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::FleetServiceAction {
                host: host.to_string(),
                name: name.to_string(),
                action,
            }))
            .text_aligned(action.label().to_string(), 11.0, color, Alignment::Start),
        );
    }
    fleet_action_bar_frame(buttons)
}

/// Marco común de las barras de acción de la flota: fila con sangría extra
/// (los recursos de flota ya van indentados) y el mismo gap/padding que las
/// barras del Source montado.
fn fleet_action_bar_frame<HostMsg: Clone + 'static>(
    buttons: Vec<View<HostMsg>>,
) -> View<HostMsg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(40.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(buttons)
}

/// Fila de contenedor con semáforo runtime, clickeable. Sin estado
/// observado pinta `◌` tenue; con estado, el glifo coloreado (verde vivo /
/// tenue parado) + el `status` de Docker. `drift` agrega un chip ⚠; el
/// click selecciona el contenedor (abre la barra de acciones).
#[allow(clippy::too_many_arguments)]
fn container_row<HostMsg: Clone + Send + Sync + 'static>(
    name: &str,
    image: &str,
    status: Option<&matilda_discover::ContainerStatus>,
    drift: bool,
    selected: bool,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let (glyph, color, tail) = match status {
        Some(cs) => {
            let color = if cs.state.is_up() {
                Color::from_rgba8(0x82, 0xCD, 0x8C, 0xFF) // verde vivo
            } else {
                theme.fg_muted
            };
            (cs.state.glyph(), color, format!("  · {}", cs.status))
        }
        None => ('◌', theme.fg_muted, String::new()),
    };
    let prefix = if selected { "▸ " } else { "  " };
    let drift_chip = if drift { "  ⚠ drift" } else { "" };
    let mut row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::SelectContainer(name.to_string())))
    .text_aligned(
        format!("{prefix}{glyph} {name}   {image}{tail}{drift_chip}"),
        11.0,
        color,
        Alignment::Start,
    );
    if selected {
        row = row.fill(theme.bg_row_hover);
    }
    row
}

/// Barra de acciones para el contenedor seleccionado: un botón por
/// `ContainerAction` (Start/Stop/Restart/Logs/Remove).
fn container_action_bar<HostMsg: Clone + Send + Sync + 'static>(
    name: &str,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let mut buttons: Vec<View<HostMsg>> = Vec::new();
    for action in ContainerAction::all() {
        // Remove en rojo tenue (es destructivo); el resto en accent.
        let color = if matches!(action, ContainerAction::Remove) {
            Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF)
        } else {
            theme.accent
        };
        buttons.push(
            View::new(Style {
                size: Size { width: length(54.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::ContainerActionMsg {
                name: name.to_string(),
                action,
            }))
            .text_aligned(action.label().to_string(), 11.0, color, Alignment::Start),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(buttons)
}

/// Fila de servicio systemd con semáforo (●/✖/○) + `sub` + descripción,
/// clickeable para abrir su barra de acciones.
fn service_row<HostMsg: Clone + Send + Sync + 'static>(
    svc: &matilda_discover::ServiceStatus,
    selected: bool,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    use matilda_discover::ServiceState;
    let color = match svc.state {
        ServiceState::Active | ServiceState::Activating => Color::from_rgba8(0x82, 0xCD, 0x8C, 0xFF),
        ServiceState::Failed => Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF),
        _ => theme.fg_muted,
    };
    let prefix = if selected { "▸ " } else { "  " };
    let desc = if svc.description.is_empty() {
        String::new()
    } else {
        format!("  · {}", svc.description)
    };
    let mut row = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .on_click(lift(Msg::SelectService(svc.name.clone())))
    .text_aligned(
        format!("{prefix}{} {}  ({}){desc}", svc.state.glyph(), svc.name, svc.sub),
        11.0,
        color,
        Alignment::Start,
    );
    if selected {
        row = row.fill(theme.bg_row_hover);
    }
    row
}

/// Barra de acciones del servicio seleccionado.
fn service_action_bar<HostMsg: Clone + Send + Sync + 'static>(
    name: &str,
    theme: &Theme,
    lift: impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static,
) -> View<HostMsg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let mut buttons: Vec<View<HostMsg>> = Vec::new();
    for action in ServiceAction::all() {
        let color = if matches!(action, ServiceAction::Stop | ServiceAction::Disable) {
            Color::from_rgba8(0xE0, 0x6C, 0x6C, 0xFF)
        } else {
            theme.accent
        };
        buttons.push(
            View::new(Style {
                size: Size { width: length(60.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .hover_fill(theme.bg_row_hover)
            .on_click(lift(Msg::ServiceActionMsg {
                name: name.to_string(),
                action,
            }))
            .text_aligned(action.label().to_string(), 11.0, color, Alignment::Start),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(buttons)
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

    // Monitor de runtime: contenedores vivos (la serie es el #up; el
    // detalle lleva up/down). Es el monitoreo en vivo del servidor.
    let counts = state.runtime_counts.clone();
    let runtime_monitor = MonitorSpec {
        id: "matilda.runtime",
        label: format!("matilda · {} · up", state.source.label()),
        accent: Rgb::new(0x82, 0xCD, 0x8C),
        history_capacity: 60,
        period_secs: 5.0,
        sampler: Box::new(move || {
            let (up, down) = *counts.lock().unwrap();
            Sample::new(up as f32, format!("{up} up · {down} down"))
        }),
    };

    ModuleContributions {
        monitors: vec![monitor, runtime_monitor],
        shortcuts: vec![
            ShortcutSpec::module_action("Discover", "matilda.discover")
                .with_hint("Lee el estado actual del servidor"),
            ShortcutSpec::module_action("Plan", "matilda.plan")
                .with_hint("Calcula la reconciliación deseado-vs-actual"),
            ShortcutSpec::module_action("Dry-run", "matilda.dry_run")
                .with_hint("Previsualiza los pasos sin aplicar"),
            ShortcutSpec::module_action("Apply", "matilda.apply")
                .with_hint("Reconcilia el servidor con el inventario deseado"),
            ShortcutSpec::module_action("Fleet", "matilda.fleet")
                .with_hint("Consulta el runtime de todos los hosts por SSH"),
            ShortcutSpec::module_action("Reload", "matilda.reload")
                .with_hint("Relee el inventario JSON desde disco"),
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
    fn with_inventory_uses_provided_desired() {
        let mut inv = matilda_core::Inventory::new();
        inv.add_container(matilda_core::Container::new("only", "alpine:3"));
        let s = State::with_inventory(Source::Local, inv);
        assert_eq!(s.desired.containers().count(), 1);
        assert_eq!(s.desired.hosts().count(), 0);
        assert!(s.plan.is_none());
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
        // 1 host + 2 containers + 1 vhost + 1 service = 5 creates.
        assert_eq!(plan.count(Op::Create), 5);
        assert_eq!(s.pending_count(), 5);
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
    fn dry_run_report_appends_lines_to_log() {
        let mut s = State::new(Source::Local);
        let lines = vec!["línea 1".into(), "línea 2".into(), "línea 3".into()];
        s = update(s, Msg::DryRunReport(lines));
        assert!(s.log.iter().any(|l| l == "línea 1"));
        assert!(s.log.iter().any(|l| l == "línea 3"));
    }

    #[test]
    fn dry_run_with_remote_source_defers_to_chassis() {
        let s = State::new(Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: None,
        });
        let s = update(s, Msg::DryRun);
        assert!(s
            .log
            .iter()
            .any(|l| l.contains("delegado al chasis")));
    }

    #[test]
    fn dry_run_remote_blocking_local_returns_lines() {
        let inv = matilda_core::Inventory::new();
        let res = dry_run_remote_blocking(&Source::Local, &inv);
        assert!(res.is_ok());
        let lines = res.unwrap();
        assert!(!lines.is_empty());
        // El primer reporte siempre incluye el current.
        assert!(lines[0].contains("current"));
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
        assert!(matches!(dispatch("matilda.apply"), Some(Msg::Apply)));
        assert!(dispatch("desconocido").is_none());
    }

    #[test]
    fn contributions_expose_monitor_and_five_shortcuts() {
        let s = State::new(Source::Local);
        let c = contributions(&s);
        // pending + runtime.
        assert_eq!(c.monitors.len(), 2);
        assert_eq!(c.monitors[1].id, "matilda.runtime");
        assert_eq!(c.shortcuts.len(), 6);
        assert_eq!(c.shortcuts[0].label, "Discover");
        assert_eq!(c.shortcuts[1].label, "Plan");
        assert_eq!(c.shortcuts[2].label, "Dry-run");
        assert_eq!(c.shortcuts[3].label, "Apply");
        assert_eq!(c.shortcuts[4].label, "Fleet");
        assert_eq!(c.shortcuts[5].label, "Reload");
    }

    #[test]
    fn set_runtime_actualiza_estado_y_contadores() {
        use matilda_discover::{ContainerStatus, RunState, RuntimeState};
        let mut s = State::new(Source::Local);
        assert!(s.runtime.is_none());
        let rt = RuntimeState {
            containers: vec![
                ContainerStatus {
                    name: "web".into(),
                    image: "nginx:1.27".into(),
                    state: RunState::Running,
                    status: "Up 2 hours".into(),
                    ports: "0.0.0.0:80->80/tcp".into(),
                },
                ContainerStatus {
                    name: "viejo".into(),
                    image: "img".into(),
                    state: RunState::Exited,
                    status: "Exited (0)".into(),
                    ports: String::new(),
                },
            ],
            services: vec![],
            vhosts: vec![],
        };
        s = update(s, Msg::SetRuntime(rt));
        let rt = s.runtime.as_ref().expect("runtime fijado");
        assert_eq!(rt.up_count(), 1);
        assert_eq!(rt.down_count(), 1);
        // `viejo` no está en el inventario deseado → es huérfano observado.
        assert!(s.desired.container("viejo").is_none());
        assert!(s.log.iter().any(|l| l.contains("1 up") && l.contains("1 down")));
    }

    #[test]
    fn select_container_es_toggle() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SelectContainer("web".into()));
        assert_eq!(s.selected_container.as_deref(), Some("web"));
        // Re-click deselecciona.
        s = update(s, Msg::SelectContainer("web".into()));
        assert!(s.selected_container.is_none());
        // Otro contenedor reemplaza.
        s = update(s, Msg::SelectContainer("a".into()));
        s = update(s, Msg::SelectContainer("b".into()));
        assert_eq!(s.selected_container.as_deref(), Some("b"));
    }

    #[test]
    fn container_action_local_loguea_comando() {
        // Sin docker en el entorno de test, el comando falla — pero el log
        // debe contener la línea del comando y un cierre. (No depende de
        // que docker exista; sólo del path de ejecución local.)
        let mut s = State::new(Source::Local);
        s = update(
            s,
            Msg::ContainerActionMsg {
                name: "web".into(),
                action: ContainerAction::Start,
            },
        );
        assert!(s.log.iter().any(|l| l.contains("docker start web")));
        assert!(s.log.iter().any(|l| l.contains("Start web")));
    }

    #[test]
    fn service_action_local_loguea_comando() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SelectService("sshd.service".into()));
        assert_eq!(s.selected_service.as_deref(), Some("sshd.service"));
        s = update(
            s,
            Msg::ServiceActionMsg {
                name: "sshd.service".into(),
                action: ServiceAction::Restart,
            },
        );
        assert!(s.log.iter().any(|l| l.contains("systemctl restart sshd.service")));
    }

    #[test]
    fn fleet_refresh_marca_pending_y_resultados_aterrizan() {
        use matilda_discover::RuntimeState;
        let mut s = State::new(Source::Local); // example tiene 1 host: edge-1
        s = update(s, Msg::RefreshFleet);
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Pending)));
        // Aterriza el runtime de ese host.
        let rt = RuntimeState {
            containers: vec![matilda_discover::ContainerStatus {
                name: "web".into(),
                image: "nginx".into(),
                state: matilda_discover::RunState::Running,
                status: "Up".into(),
                ports: String::new(),
            }],
            services: vec![],
            vhosts: vec![],
        };
        s = update(s, Msg::SetHostRuntime { host: "edge-1".into(), runtime: rt });
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Ready(_))));
        // Y un error en otro.
        s = update(s, Msg::SetHostError { host: "edge-1".into(), error: "timeout".into() });
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Failed(_))));
        // Selección toggle.
        s = update(s, Msg::SelectHost("edge-1".into()));
        assert_eq!(s.selected_host.as_deref(), Some("edge-1"));
        s = update(s, Msg::SelectHost("edge-1".into()));
        assert!(s.selected_host.is_none());
    }

    #[test]
    fn fleet_container_action_se_delega_al_chasis() {
        // El módulo no abre SSH: sólo deja la intención en el log; el chasis
        // corre `fleet_container_action_blocking` en un thread.
        let mut s = State::new(Source::Local);
        s = update(s, Msg::FleetContainerAction {
            host: "edge-1".into(),
            name: "web".into(),
            action: ContainerAction::Restart,
        });
        assert!(s.log.iter().any(|l| l.contains("Restart")
            && l.contains("web")
            && l.contains("edge-1")
            && l.contains("delegado al chasis")));
    }

    #[test]
    fn fleet_service_action_se_delega_al_chasis() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::FleetServiceAction {
            host: "edge-1".into(),
            name: "nginx.service".into(),
            action: ServiceAction::Stop,
        });
        assert!(s.log.iter().any(|l| l.contains("nginx.service")
            && l.contains("edge-1")
            && l.contains("delegado al chasis")));
    }

    #[test]
    fn select_fleet_resource_es_toggle_y_excluyente() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SelectFleetContainer("web".into()));
        assert_eq!(s.selected_fleet_container.as_deref(), Some("web"));
        // Seleccionar un servicio limpia el contenedor (mutuamente excluyentes).
        s = update(s, Msg::SelectFleetService("sshd.service".into()));
        assert!(s.selected_fleet_container.is_none());
        assert_eq!(s.selected_fleet_service.as_deref(), Some("sshd.service"));
        // Re-click deselecciona.
        s = update(s, Msg::SelectFleetService("sshd.service".into()));
        assert!(s.selected_fleet_service.is_none());
    }

    #[test]
    fn cambiar_de_host_limpia_la_seleccion_de_recurso() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::SelectHost("edge-1".into()));
        s = update(s, Msg::SelectFleetContainer("web".into()));
        assert_eq!(s.selected_fleet_container.as_deref(), Some("web"));
        // Expandir otro host abandona la selección anterior.
        s = update(s, Msg::SelectHost("edge-2".into()));
        assert!(s.selected_fleet_container.is_none());
        assert!(s.selected_fleet_service.is_none());
    }

    #[test]
    fn source_runtime_remote_blocking_local_no_abre_ssh() {
        // Para Source::Local cae a `discover_runtime` (sin SSH). En CI sin
        // docker retorna un runtime vacío, pero Ok.
        let res = source_runtime_remote_blocking(&Source::Local);
        assert!(res.is_ok());
    }

    #[test]
    fn fleet_quiet_actualiza_sin_loguear() {
        use matilda_discover::RuntimeState;
        let mut s = State::new(Source::Local);
        let log_before = s.log.len();
        let rt = RuntimeState { containers: vec![], services: vec![], vhosts: vec![] };
        s = update(s, Msg::SetHostRuntimeQuiet { host: "edge-1".into(), runtime: rt });
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Ready(_))));
        // El polling no debe agregar nada al log.
        assert_eq!(s.log.len(), log_before);
        s = update(s, Msg::SetHostErrorQuiet { host: "edge-1".into(), error: "timeout".into() });
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Failed(_))));
        assert_eq!(s.log.len(), log_before);
    }

    #[test]
    fn fleet_action_done_loguea_y_refresca_el_host() {
        use matilda_discover::RuntimeState;
        let mut s = State::new(Source::Local);
        // Sin runtime → sólo loguea, no toca el FleetEntry.
        s = update(s, Msg::FleetActionDone {
            host: "edge-1".into(),
            lines: vec!["$ docker logs web".into(), "✔ Logs web en edge-1 (remoto)".into()],
            runtime: None,
        });
        assert!(s.log.iter().any(|l| l.contains("Logs web en edge-1")));
        assert!(s.fleet.get("edge-1").is_none());
        // Con runtime (acción mutante re-observada) → el host queda Ready.
        let rt = RuntimeState { containers: vec![], services: vec![], vhosts: vec![] };
        s = update(s, Msg::FleetActionDone {
            host: "edge-1".into(),
            lines: vec!["✔ Restart web en edge-1 (remoto)".into()],
            runtime: Some(rt),
        });
        assert!(matches!(s.fleet.get("edge-1"), Some(FleetEntry::Ready(_))));
    }

    #[test]
    fn with_inventory_path_records_the_path() {
        let inv = matilda_core::Inventory::new();
        let p = PathBuf::from("/etc/matilda/inv.json");
        let s = State::with_inventory_path(Source::Local, inv, p.clone());
        assert_eq!(s.inventory_path.as_deref(), Some(p.as_path()));
    }

    #[test]
    fn set_desired_replaces_inventory_and_invalidates_plan() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::MakePlan);
        assert!(s.pending_count() > 0);

        let mut new_inv = matilda_core::Inventory::new();
        new_inv.add_container(matilda_core::Container::new("alone", "alpine"));
        s = update(s, Msg::SetDesired(new_inv));

        assert_eq!(s.desired.containers().count(), 1);
        assert_eq!(s.desired.hosts().count(), 0);
        assert!(s.plan.is_none());
        assert_eq!(s.pending_count(), 0);
        assert!(s.log.iter().any(|l| l.contains("recargado")));
    }

    #[test]
    fn apply_with_remote_source_defers_to_chassis() {
        let s = State::new(Source::Remote {
            host: "srv".into(),
            user: "ops".into(),
            port: 22,
            label: None,
        });
        let s = update(s, Msg::Apply);
        assert!(s
            .log
            .iter()
            .any(|l| l.contains("delegado al chasis")));
        assert!(s.plan.is_none());
    }

    #[test]
    fn apply_report_with_new_current_resets_plan() {
        let mut s = State::new(Source::Local);
        // Forzamos un plan vigente con pendientes.
        s = update(s, Msg::MakePlan);
        assert!(s.pending_count() > 0);

        // Ahora simulamos que el chasis aplicó remoto y re-descubrió:
        // el nuevo current coincide con el desired → plan vacío.
        let new_current = s.desired.clone();
        s = update(
            s,
            Msg::ApplyReport {
                lines: vec!["✔ aplicado".into()],
                new_current: Some(new_current),
            },
        );
        assert_eq!(s.pending_count(), 0);
        assert_eq!(s.plan.as_ref().unwrap().len(), 0);
        assert!(s.log.iter().any(|l| l == "✔ aplicado"));
    }

    #[test]
    fn apply_report_without_new_current_keeps_plan() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::MakePlan);
        let pending_before = s.pending_count();
        s = update(
            s,
            Msg::ApplyReport {
                lines: vec!["✘ falló paso 2".into()],
                new_current: None,
            },
        );
        // El plan vigente sobrevive — el operador inspecciona dónde se rompió.
        assert_eq!(s.pending_count(), pending_before);
        assert!(s.log.iter().any(|l| l.contains("falló paso 2")));
    }

    #[test]
    fn apply_remote_blocking_local_returns_lines() {
        let inv = matilda_core::Inventory::new();
        let res = apply_remote_blocking(&Source::Local, &inv);
        assert!(res.is_ok());
        let (lines, new_current) = res.unwrap();
        // Inventory vacío → "nada que aplicar"; new_current refleja el local.
        assert!(!lines.is_empty());
        assert!(new_current.is_some());
    }

    #[test]
    fn monitor_sampler_reflects_pending_steps() {
        let mut s = State::new(Source::Local);
        s = update(s, Msg::MakePlan); // 5 pendientes (host+2 cont+vhost+service)
        let c = contributions(&s);
        let sample = (c.monitors[0].sampler)();
        assert_eq!(sample.value, 5.0);
        assert_eq!(sample.display, "5 pendientes");
    }
}
