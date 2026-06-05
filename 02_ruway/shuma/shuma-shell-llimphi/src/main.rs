//! `shuma-shell-llimphi` — chasis del shell shuma sobre Llimphi.
//!
//! Shuma es la app standalone "normal" del workspace: una ventana con
//! tabs siempre visibles, monitores a la derecha, command-bar abajo. La
//! metáfora Quake-drawer (overlay sobre el escritorio + F12 para
//! invocar) vive en `mirada-launcher-llimphi`, no acá.
//!
//! **Layout** (sin `[main]` en shumarc):
//!
//! ```text
//!  ┌──────────────────────────────────────────────────┐
//!  │ TopBar · launcher (apps + shortcuts)             │
//!  ├────────────────────────────────┬─────────────────┤
//!  │ tabs: [shell] [lienzo] [matilda]│                 │
//!  ├────────────────────────────────┤ Monitores       │
//!  │                                │  CPU + MEM +    │
//!  │  contenido del tab activo      │  los del módulo │
//!  │                                │                 │
//!  ├────────────────────────────────┴─────────────────┤
//!  │ BottomBar · command-bar  › escribí…              │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! Si el shumarc declara `[main]`, ese módulo ocupa toda el área central
//! a pantalla completa (sin tabs ni monitores) — útil para correr shuma
//! como wrapper de matilda standalone, por ejemplo.
//!
//! El chasis no conoce a sus módulos: el `Kind` estático enumera los
//! compilados. El shumarc elige cuáles activar y en qué slot.

#![forbid(unsafe_code)]

mod config;

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, PathEl, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{
    App, DragPhase, Handle, KeyEvent, KeyState, Modifiers, PaintRect, View, WheelDelta,
};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use shuma_module::{ModuleContributions, MonitorSpec, ShortcutAction, Source};
use shuma_sysmon::{Snapshot, SystemSampler};
use std::collections::HashMap;

const HISTORY: usize = 60;
const TICK: Duration = Duration::from_secs(1);
/// Cadencia rápida para drenar el output del shell (streaming de
/// `shuma-exec`). 1 Hz se siente lento al ver `for i in …; do echo $i;
/// sleep 0.1; done`; 100 ms hace la salida sentirse en vivo sin
/// comerse CPU notable.
const SHELL_TICK: Duration = Duration::from_millis(100);
const MONITORS_INITIAL_WIDTH: f32 = 280.0;

/// Construye el cliente del rail hospedado si `SHUMA_DELEGATE_SIDEBAR` está
/// set. shuma publica sus tabs como dientes (cambian de tab al activarse) +
/// un diente "Monitores" que togglea el panel derecho. Cuando shuma tiene
/// foco, esos dientes aparecen en el rail global de pata; el área central
/// queda como puro lienzo (monitores ocultos por default). `app_id` debe ser
/// el mismo que reporta el compositor (`Shell::app_id`).
fn shuma_host(handle: &Handle<Msg>) -> Option<pata_host::HostClient> {
    if std::env::var_os("SHUMA_DELEGATE_SIDEBAR").is_none() {
        return None;
    }
    let teeth = host_tool_teeth();
    let h = handle.clone();
    pata_host::HostClient::connect("shuma.shell", "shuma", teeth, move |id| {
        h.dispatch(Msg::HostActivate(id))
    })
}

/// Dientes que shuma presta al rail de pata: uno por **herramienta** de la
/// sesión activa (id = índice en `Tool::ALL`).
fn host_tool_teeth() -> Vec<pata_host::HostedTooth> {
    Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| pata_host::HostedTooth::new(i as u32, tool_icon_name(*t), t.label().to_string()))
        .collect()
}

/// Nombre de icono (vocabulario abierto de `pata`) para una herramienta.
fn tool_icon_name(t: Tool) -> &'static str {
    match t {
        Tool::History => "tools",
        Tool::Monitor => "system",
        Tool::Explorer => "files",
        Tool::Matilda => "settings",
    }
}

/// `Source` por defecto de la tab shell según las env vars del proceso —
/// para que `SHUMA_REMOTE*` enrute los comandos al daemon sin shumarc.
/// (rescate del `detect_remote_transport` del shell GPUI):
///
/// - `SHUMA_REMOTE_TCP_ADDR=host:port` + `SHUMA_REMOTE_TCP_PUB=<hex>`
///   → TCP autenticado Noise XK (`DaemonTcp`). La keypair propia la carga
///   `start_run` al conectar; acá sólo pasamos addr + pubkey del server.
/// - `SHUMA_REMOTE_SOCKET=/path` → daemon por ese Unix socket.
/// - `SHUMA_REMOTE=1` → daemon por el socket canónico (`socket: None`).
/// - sin ninguna → `Local` (ejecución directa).
fn default_shell_source() -> Source {
    let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if let (Some(addr), Some(pub_hex)) = (
        nonempty("SHUMA_REMOTE_TCP_ADDR"),
        nonempty("SHUMA_REMOTE_TCP_PUB"),
    ) {
        return Source::DaemonTcp {
            addr,
            server_pub_hex: pub_hex,
            label: None,
        };
    }
    if let Some(path) = nonempty("SHUMA_REMOTE_SOCKET") {
        return Source::Daemon {
            socket: Some(std::path::PathBuf::from(path)),
            label: None,
        };
    }
    if std::env::var("SHUMA_REMOTE").as_deref() == Ok("1") {
        return Source::Daemon {
            socket: None,
            label: None,
        };
    }
    Source::Local
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Shell>();
}

/// Lista los contenedores locales (`docker ps -a`) en un hilo y entrega los
/// nombres por `Msg::ContainersLoaded`. Vacío si docker no está o falla.
fn spawn_list_containers(handle: &Handle<Msg>) {
    handle.spawn(|| {
        let names = std::process::Command::new("docker")
            .args(["ps", "-a", "--format", "{{.Names}}"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Msg::ContainersLoaded(names)
    });
}

/// Crea un contenedor `name` de la `image` dada (corre detached con `sleep
/// infinity`) en un hilo; al volver, re-lista. No-op de UI si docker falla.
fn spawn_create_container(handle: &Handle<Msg>, image: &'static str, name: String) {
    handle.spawn(move || {
        let _ = std::process::Command::new("docker")
            .args(["run", "-d", "--name", &name, image, "sleep", "infinity"])
            .output();
        Msg::RefreshContainers
    });
}

/// Si la sesión activa es la draft y se acaba de configurar, la promueve a
/// sesión propia (número + kind según aislamiento + nombre) e inserta una draft
/// nueva al frente para que siga siendo el punto de entrada.
fn promote_if_draft(m: &mut Model) {
    let is_draft = matches!(m.sessions.get(m.active_session), Some(s) if s.kind == SessionKind::Draft);
    if !is_draft {
        return;
    }
    let n = m.sessions.iter().filter(|s| s.number.is_some()).count() as u32 + 1;
    if let Some(s) = m.sessions.get_mut(m.active_session) {
        s.number = Some(n);
        s.kind = match s.isolation {
            Isolation::Remote => SessionKind::Remote,
            Isolation::Local => SessionKind::Local,
        };
        s.name = match s.isolation {
            Isolation::Local => format!("local {n}"),
            Isolation::Remote => format!("remota {n}"),
        };
    }
    // Nace un draft nuevo al frente; la sesión promovida se corre un índice.
    m.sessions.insert(0, Session::draft());
    m.active_session += 1;
}

// ─── Tipos de módulos conocidos por este binario ───────────────────

/// Qué `Kind` puede ocupar cada slot. Una variante por módulo
/// compilado: agregar uno nuevo (p. ej. `matilda`) es una variante +
/// ramas en `update`/`view`. El static dispatch sortea la ausencia de
/// `View::map` en llimphi-ui.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Launcher,
    CommandBar,
    Shell,
    Matilda,
    Minga,
    Canvas,
}

impl Kind {
    /// `id` canónico — bloque 5 lo usa para matchear shumarc.
    #[allow(dead_code)]
    fn id(self) -> &'static str {
        match self {
            Kind::Launcher => shuma_module_launcher::ID,
            Kind::CommandBar => shuma_module_commandbar::ID,
            Kind::Shell => shuma_module_shell::ID,
            Kind::Matilda => shuma_module_matilda::ID,
            Kind::Minga => shuma_module_minga::ID,
            Kind::Canvas => shuma_module_canvas::ID,
        }
    }
}

/// Cuál de las tres instancias-módulo de una sesión direcciona un `Slot` o un
/// `Msg`. Las vistas Hosts y Vhosts comparten la instancia Matilda (mismo
/// inventario, distinto render).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    Shell,
    Canvas,
    Matilda,
}

/// Dónde corre el shell de la sesión (la base del aislamiento). El contenedor
/// NO es exclusivo: es una capa opcional **encima** de Local o Remoto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Isolation {
    /// Directo sobre esta máquina.
    Local,
    /// Sobre una máquina remota por SSH.
    Remote,
}

impl Isolation {
    const ALL: [Isolation; 2] = [Isolation::Local, Isolation::Remote];
    /// Etiqueta corta (la rica con sublabel la arma `view::iso_items`).
    #[allow(dead_code)]
    fn label(self) -> &'static str {
        match self {
            Isolation::Local => "Local",
            Isolation::Remote => "Remoto",
        }
    }
}

/// Estado de conexión de la sesión — lo refleja su panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnState {
    /// En espera (aún no conectada — remoto sin conectar / contenedor sin crear).
    Pending,
    /// Conectada y lista.
    Connected,
    /// Estuvo conectada y se cayó. (Se setea al caerse SSH/contenedor — fase B/C.)
    #[allow(dead_code)]
    Disconnected,
}

impl ConnState {
    fn label(self) -> &'static str {
        match self {
            ConnState::Pending => "en espera",
            ConnState::Connected => "conectado",
            ConnState::Disconnected => "desconectado",
        }
    }
}

/// La distro del aislamiento (para contenedor/remoto).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Distro {
    Ubuntu,
    Debian,
    Alpine,
    Arch,
}

impl Distro {
    const ALL: [Distro; 4] = [Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch];
    fn label(self) -> &'static str {
        match self {
            Distro::Ubuntu => "Ubuntu",
            Distro::Debian => "Debian",
            Distro::Alpine => "Alpine",
            Distro::Arch => "Arch",
        }
    }
    /// Imagen OCI para crear el contenedor.
    fn image(self) -> &'static str {
        match self {
            Distro::Ubuntu => "ubuntu:latest",
            Distro::Debian => "debian:latest",
            Distro::Alpine => "alpine:latest",
            Distro::Arch => "archlinux:latest",
        }
    }
}

/// Cuál dropdown de la config de sesión está abierto (overlay del select).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropKind {
    Isolation,
    Distro,
    /// Suscribir a un contenedor existente / crear uno nuevo.
    Container,
}

/// El tipo de una sesión — define el icono de su diente (rail izquierdo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionKind {
    /// La sesión por defecto, local y sin aislamiento — "no toca nada". No
    /// lleva número de insignia. Es la primera y siempre está.
    Draft,
    /// Sesión local creada por el usuario (con número de insignia).
    Local,
    /// Sesión remota (SSH/daemon) — aislamiento remoto. Aún no la crea nadie
    /// (el `+` hace local); el form de aislamiento remoto es la fase 4.
    #[allow(dead_code)]
    Remote,
}

/// Las **herramientas** de la sesión activa — un diente del rail DERECHO. Cada
/// una abre su panel operando sobre la sesión activa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    History,
    Monitor,
    Explorer,
    Matilda,
}

impl Tool {
    /// Orden de los dientes en el rail derecho (debe seguir a `host_tool_teeth`).
    const ALL: [Tool; 4] = [Tool::History, Tool::Monitor, Tool::Explorer, Tool::Matilda];

    fn label(self) -> &'static str {
        match self {
            Tool::History => "Historial",
            Tool::Monitor => "Monitor",
            Tool::Explorer => "Explorer",
            Tool::Matilda => "Matilda",
        }
    }
}

/// State vivo de un módulo. Una variante por `Kind` para evitar trait
/// objects (cada módulo trae su propio `Msg` que no es object-safe).
enum ModuleState {
    Launcher(shuma_module_launcher::State),
    CommandBar(shuma_module_commandbar::State),
    Shell(shuma_module_shell::State),
    // `State` de matilda lleva el inventory entero (varios cientos
    // de bytes); boxearlo mantiene el enum ModuleState compacto.
    Matilda(Box<shuma_module_matilda::State>),
    Minga(shuma_module_minga::State),
    Canvas(shuma_module_canvas::State),
}

/// Una instancia activa de un módulo. `kind` + `state` deben coincidir
/// (lo invariante lo garantiza el constructor).
struct Instance {
    kind: Kind,
    /// Etiqueta del módulo. El título de la vista lo arma la sesión
    /// (`nombre · vista`); los constructores la setean y queda disponible.
    #[allow(dead_code)]
    label: String,
    state: ModuleState,
}

impl Instance {
    fn launcher(state: shuma_module_launcher::State) -> Self {
        Self {
            kind: Kind::Launcher,
            label: rimay_localize::t("shuma-label-launcher"),
            state: ModuleState::Launcher(state),
        }
    }

    fn command_bar(state: shuma_module_commandbar::State) -> Self {
        Self {
            kind: Kind::CommandBar,
            label: rimay_localize::t("shuma-label-command"),
            state: ModuleState::CommandBar(state),
        }
    }

    fn shell(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Shell,
            label,
            state: ModuleState::Shell(shuma_module_shell::State::new(source)),
        }
    }

    fn matilda(label: String, source: Source) -> Self {
        Self::matilda_with_inventory(label, source, None)
    }

    fn matilda_with_inventory(
        label: String,
        source: Source,
        inventory: Option<&std::path::Path>,
    ) -> Self {
        let state = match inventory {
            Some(p) => {
                let inv = load_matilda_inventory(p).unwrap_or_else(example_inventory_fallback);
                shuma_module_matilda::State::with_inventory_path(source, inv, p.to_path_buf())
            }
            None => shuma_module_matilda::State::new(source),
        };
        Self {
            kind: Kind::Matilda,
            label,
            state: ModuleState::Matilda(Box::new(state)),
        }
    }

    fn minga(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Minga,
            label,
            state: ModuleState::Minga(shuma_module_minga::State::new(source)),
        }
    }

    fn canvas(label: String) -> Self {
        Self {
            kind: Kind::Canvas,
            label,
            state: ModuleState::Canvas(shuma_module_canvas::State::new()),
        }
    }
}

/// Una **sesión de trabajo**: un ambiente con su aislamiento (local o remoto)
/// y sus tres vistas (shell, lienzo, inventario matilda). Cambiar de sesión
/// (tab superior) cambia todo el ambiente; el rail derecho elige la vista.
struct Session {
    name: String,
    kind: SessionKind,
    /// Número de insignia del diente (None para la draft).
    number: Option<u32>,
    /// Base del aislamiento (Local/Remoto).
    isolation: Isolation,
    /// Capa de contenedor OPCIONAL (encima de Local o Remoto). El colapsable
    /// del panel la crea/conecta.
    distro: Distro,
    /// Contenedor suscrito (`None` = sin contenedor). El exec real del shell
    /// dentro de él se cablea con `Source::Container` (shuma-module).
    container: Option<String>,
    /// Si el colapsable de contenedor está abierto en el panel.
    container_open: bool,
    /// Estado de conexión de la sesión (lo refleja el panel).
    conn: ConnState,
    /// El origen de ejecución del shell + matilda (Local / Daemon / Remote).
    /// (El enforcement real del aislamiento contenedor/remoto es deuda; hoy el
    /// shell corre con este `source`.)
    #[allow(dead_code)]
    source: Source,
    shell: Instance,
    canvas: Instance,
    matilda: Instance,
}

impl Session {
    fn build(name: String, kind: SessionKind, number: Option<u32>, source: Source) -> Self {
        Self {
            shell: Instance::shell(name.clone(), source.clone()),
            canvas: Instance::canvas(rimay_localize::t("shuma-label-canvas")),
            matilda: Instance::matilda(name.clone(), source.clone()),
            name,
            kind,
            number,
            isolation: Isolation::Local,
            distro: Distro::Ubuntu,
            container: None,
            container_open: false,
            // Local arranca conectado; remoto en espera hasta conectar.
            conn: ConnState::Connected,
            source,
        }
    }

    /// La sesión por defecto: local, sin configurar, sin número. No toca nada.
    /// Tiene los campos de config a sus defaults; al tocarlos pasa a sesión propia.
    fn draft() -> Self {
        Self::build("draft".to_string(), SessionKind::Draft, None, default_shell_source())
    }

    /// `true` si la sesión está moviendo datos ahora (comando corriendo) — para
    /// el puntito LED del diente.
    fn active_data(&self) -> bool {
        matches!(&self.shell.state, ModuleState::Shell(s) if s.is_running())
    }

    /// Reconstruye el shell + matilda con el `source` que dicta el aislamiento
    /// elegido. Pierde el shell anterior a propósito: reconfigurar el aislamiento
    /// = ambiente nuevo. (Container aún corre local — el exec en contenedor es
    /// deuda; Remote usa el daemon de `default_shell_source`.)
    fn apply_isolation(&mut self) {
        let source = match self.isolation {
            Isolation::Local => Source::Local,
            Isolation::Remote => default_shell_source(),
        };
        // Local está listo; remoto queda en espera hasta conectar.
        self.conn = match self.isolation {
            Isolation::Local => ConnState::Connected,
            Isolation::Remote => ConnState::Pending,
        };
        self.shell = Instance::shell(self.name.clone(), source.clone());
        self.matilda = Instance::matilda(self.name.clone(), source.clone());
        self.source = source;
    }

    fn instance(&self, w: Which) -> &Instance {
        match w {
            Which::Shell => &self.shell,
            Which::Canvas => &self.canvas,
            Which::Matilda => &self.matilda,
        }
    }

    fn instance_mut(&mut self, w: Which) -> &mut Instance {
        match w {
            Which::Shell => &mut self.shell,
            Which::Canvas => &mut self.canvas,
            Which::Matilda => &mut self.matilda,
        }
    }

    /// Config persistible de la sesión (sin las instancias-módulo vivas).
    fn to_config(&self) -> SessionConfig {
        SessionConfig {
            name: self.name.clone(),
            number: self.number,
            isolation: self.isolation,
            distro: self.distro,
            container: self.container.clone(),
        }
    }

    /// Reconstruye una sesión desde su config persistida.
    fn from_config(c: SessionConfig) -> Self {
        let kind = match c.isolation {
            Isolation::Remote => SessionKind::Remote,
            Isolation::Local => SessionKind::Local,
        };
        let source = match c.isolation {
            Isolation::Local => Source::Local,
            Isolation::Remote => default_shell_source(),
        };
        let mut s = Session::build(c.name, kind, c.number, source);
        s.isolation = c.isolation;
        s.distro = c.distro;
        s.container = c.container;
        s.conn = match c.isolation {
            Isolation::Local => ConnState::Connected,
            Isolation::Remote => ConnState::Pending,
        };
        s
    }
}

/// Config persistible de una sesión (lo que sobrevive a reiniciar shuma).
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionConfig {
    name: String,
    #[serde(default)]
    number: Option<u32>,
    isolation: Isolation,
    distro: Distro,
    #[serde(default)]
    container: Option<String>,
}

/// `$XDG_CONFIG_HOME/shuma/sessions.json`.
fn sessions_path() -> Option<std::path::PathBuf> {
    directories::BaseDirs::new().map(|b| b.config_dir().join("shuma").join("sessions.json"))
}

/// Guarda las sesiones reales (no la draft) para reiniciarlas en el próximo
/// arranque. Silencioso ante errores de IO.
fn save_sessions(m: &Model) {
    let Some(path) = sessions_path() else {
        return;
    };
    let cfgs: Vec<SessionConfig> = m
        .sessions
        .iter()
        .filter(|s| s.kind != SessionKind::Draft)
        .map(|s| s.to_config())
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&cfgs) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, json);
    }
}

/// Lee las sesiones persistidas (vacío si no hay archivo o no parsea).
fn load_sessions() -> Vec<SessionConfig> {
    sessions_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<SessionConfig>>(&s).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone)]
enum ModuleMsg {
    Launcher(shuma_module_launcher::Msg),
    CommandBar(shuma_module_commandbar::Msg),
    #[allow(dead_code)]
    Shell(shuma_module_shell::Msg),
    Matilda(shuma_module_matilda::Msg),
    Minga(shuma_module_minga::Msg),
    Canvas(shuma_module_canvas::Msg),
}

// ─── Slot del chasis al que va un Msg de módulo ────────────────────

/// Identifica de dónde viene un `ModuleMsg`. Los slots únicos (TopBar/
/// Bottombar/Main) se identifican por sí mismos; el Tab lleva el
/// índice del tab para enrutar al instance correcto.
#[derive(Debug, Clone)]
enum Slot {
    TopBar,
    BottomBar,
    #[allow(dead_code)]
    Main,
    /// Una instancia-módulo de la sesión `idx` (cuál, lo dice `Which`).
    Session(usize, Which),
}

// ─── Modelo + Msg ───────────────────────────────────────────────────

struct Model {
    theme: Theme,

    // Slots fijos (únicos):
    topbar: Option<Instance>,
    bottombar: Option<Instance>,
    /// Si está set, ocupa toda el área central (sin tabs). Útil para
    /// configurar shuma como wrapper de una sola app (matilda standalone,
    /// editor, etc.) vía shumarc.
    main: Option<Instance>,

    // Sesiones de trabajo (tabs superiores cuando `main` está vacío). Cambiar
    // de sesión cambia todo el ambiente; `active_view` (rail derecho) elige la
    // vista de la sesión activa.
    sessions: Vec<Session>,
    active_session: usize,
    /// Herramienta abierta a la derecha (`None` = sin panel de herramienta).
    active_tool: Option<Tool>,
    /// Si el panel de la sesión activa (su configuración, a la izquierda) está
    /// desplegado. Cada diente de sesión ES su panel: al seleccionarlo se abre;
    /// re-clickear el activo lo cierra.
    session_panel_open: bool,
    /// Dropdown de config abierto (overlay del select), o `None`.
    dropdown_open: Option<DropKind>,
    /// Contenedores locales descubiertos (`docker ps -a`) — para suscribir.
    containers: Vec<String>,

    // Anchos resizables de los paneles laterales (px).
    session_w: f32,
    sysmon: SystemSampler,
    last_snapshot: Option<Snapshot>,
    monitors_width: f32,
    /// Historial por monitor extra (los que aportan los módulos vía
    /// `contributions()`). La clave es `"<slot>/<spec.id>"`. El chasis
    /// los muestrea en cada `Tick` y los acumula como `f32`.
    extra_history: HashMap<String, Vec<f32>>,
    /// Último `Sample::display` por monitor — se pinta como subtítulo
    /// de la stat-card.
    extra_display: HashMap<String, String>,
    /// Watcher del bus de config wawa. Vive lo que vive el modelo —
    /// al dropear se cierran los notify::RecommendedWatcher y el thread
    /// de debounce sale silenciosamente. Ningún read directo desde
    /// el código de update — sólo recibe callbacks que se traducen a
    /// `Msg::WawaConfigChanged`.
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,

    /// Menú principal: índice del menú raíz abierto (`None` = cerrado).
    menu_open: Option<usize>,
    /// Fila activa (resaltada por teclado) del dropdown del menú principal.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú contextual de terminal: ancla `(x, y)` en ventana (`None` =
    /// cerrado). Se abre con right-click sobre el área de trabajo.
    ctx_menu: Option<(f32, f32)>,

    /// Cliente del rail hospedado: con `SHUMA_DELEGATE_SIDEBAR`, shuma presta
    /// sus tabs + el toggle de monitores al rail de pata. Kept-alive (las
    /// activaciones llegan por callback → `Msg::HostActivate`); el `_` evita
    /// el lint de campo sin leer, como `_wawa_watcher`.
    _host: Option<pata_host::HostClient>,
}

impl Model {
    /// La sesión activa (la primera si el índice quedó fuera de rango).
    fn active(&self) -> Option<&Session> {
        self.sessions.get(self.active_session).or_else(|| self.sessions.first())
    }

    /// Instancia-módulo `w` de la sesión `idx`, si existe.
    fn session_instance(&self, idx: usize, w: Which) -> Option<&Instance> {
        self.sessions.get(idx).map(|s| s.instance(w))
    }

    fn session_instance_mut(&mut self, idx: usize, w: Which) -> Option<&mut Instance> {
        self.sessions.get_mut(idx).map(|s| s.instance_mut(w))
    }

}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Tick rápido que drena la salida del shell (~100 ms) sin tocar
    /// el muestreo de sysmon.
    ShellTick,
    /// Click en un diente de sesión (rail izquierdo): cambia el ambiente.
    SelectSession(usize),
    /// Click en un diente de herramienta (rail derecho): abre/cierra su panel.
    SelectTool(Tool),
    /// Abrir/cerrar un dropdown de config (aislamiento o distro).
    ToggleDropdown(DropKind),
    /// Cerrar el dropdown (scrim / Esc).
    DismissDropdown,
    /// Elegir el aislamiento en el panel de config. Sobre la draft, configurar
    /// la promueve a sesión propia (y nace un draft nuevo); sobre una sesión
    /// real, edita su config.
    SetIsolation(Isolation),
    /// Elegir la distro del aislamiento (idem promoción del draft).
    SetDistro(Distro),
    /// Abrir/cerrar el colapsable de contenedor (capa opcional).
    ToggleContainer,
    /// Cerrar (descartar) la sesión `idx`. La draft (0) no se cierra.
    CloseSession(usize),
    /// Re-listar los contenedores locales (`docker ps -a`).
    RefreshContainers,
    /// Resultado del listado de contenedores.
    ContainersLoaded(Vec<String>),
    /// Suscribir la sesión activa al contenedor `idx` de la lista.
    SubscribeContainer(usize),
    /// Crear un contenedor nuevo con la distro de la sesión y suscribirla.
    CreateContainer,
    /// Reordenar dientes por drag: mover la sesión `from` a la posición `to`.
    /// La draft (0) queda fija.
    ReorderSession(usize, usize),
    /// Resize del panel de sesión (izq) / de herramienta (der), por drag del
    /// divisor del `splitter`.
    SetSessionWidth(f32),
    SetToolWidth(f32),
    /// Click en una línea del historial: carga ese comando en el input del
    /// shell de la sesión activa.
    RunFromHistory(String),
    /// Msg de un módulo. El chasis lo enruta a `update` según `slot`.
    Module(Slot, ModuleMsg),
    /// Click en un botón de acción (matilda: discover/dry-run/apply/reload).
    /// `slot` es el módulo emisor; lo resuelve `handle_shortcut`.
    ShortcutClicked(Slot, ShortcutAction),
    /// La config de wawa (`$XDG_CONFIG_HOME/wawa/config.json`) cambió;
    /// rearmamos el theme, accent y locale sin reiniciar. Boxed por
    /// tamaño (la config tiene un BTreeMap de módulos).
    WawaConfigChanged(Box<wawa_config::WawaConfig>),

    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Navegación de teclado en el dropdown del menú principal (±1 fila).
    MenuNav(i32),
    /// Enter sobre la fila activa del menú principal.
    MenuActivate,
    /// Tick de re-render para la animación de aparición del dropdown.
    MenuTick,
    /// Comando elegido en el menú principal o contextual — se traduce al
    /// `Msg`/acción real del chasis o del módulo shell focado.
    MenuCommand(String),
    /// Right-click sobre el área de trabajo → abre el menú contextual de
    /// terminal en `(x, y)` de ventana.
    ContextMenuOpen(f32, f32),
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,

    /// Rail hospedado de pata: el usuario activó un diente. `id < tabs.len()`
    /// selecciona esa tab; `MONITORS_TOOTH` togglea el panel de monitores.
    HostActivate(u32),
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
        handle.spawn_periodic(SHELL_TICK, || Msg::ShellTick);

        // wawa-config (bus de preferencias del SO) — theme/accent/lang.
        // Lo cargamos antes de armar las instancias para que el primer
        // render ya tenga el theme correcto. El watcher avisa cambios
        // posteriores con `Msg::WawaConfigChanged`.
        let wawa = wawa_config::WawaConfig::load();
        let theme = wawa_config_llimphi::theme_from_wawa(&wawa, &Theme::dark());
        let _ = rimay_localize::set_locale(&wawa.lang);
        let wawa_watcher = {
            let handle = handle.clone();
            wawa_config::ConfigWatcher::spawn(move |cfg| {
                handle.dispatch(Msg::WawaConfigChanged(Box::new(cfg)));
            })
            .ok()
        };

        let cfg = config::ShumaConfig::load_default();
        let topbar = resolve_slot(cfg.topbar.as_ref()).or_else(|| {
            Some(Instance::launcher(
                shuma_module_launcher::State::from_apps_dir(),
            ))
        });
        let bottombar = resolve_slot(cfg.bottombar.as_ref()).or_else(|| {
            Some(Instance::command_bar(
                shuma_module_commandbar::State::default(),
            ))
        });
        let main = resolve_slot(cfg.main.as_ref());

        // La draft (índice 0) + las sesiones persistidas del último arranque.
        let mut sessions = vec![Session::draft()];
        for c in load_sessions() {
            sessions.push(Session::from_config(c));
        }

        // Rail hospedado: si `SHUMA_DELEGATE_SIDEBAR` está set, prestamos las
        // HERRAMIENTAS de la sesión activa al rail de pata.
        let host = shuma_host(handle);

        Model {
            theme,
            topbar,
            bottombar,
            main,
            sessions,
            active_session: 0,
            // Arranca con el Historial abierto a la derecha.
            active_tool: Some(Tool::History),
            // Y el panel de la draft abierto a la izquierda (su config).
            session_panel_open: true,
            dropdown_open: None,
            containers: Vec::new(),
            session_w: 240.0,
            sysmon: SystemSampler::new(HISTORY),
            last_snapshot: None,
            monitors_width: MONITORS_INITIAL_WIDTH,
            extra_history: HashMap::new(),
            extra_display: HashMap::new(),
            _wawa_watcher: wawa_watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            ctx_menu: None,
            _host: host,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Con un dropdown de config abierto, Esc lo cierra (no va al shell).
        if model.dropdown_open.is_some() {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::DismissDropdown);
            }
        }
        // Con un menú abierto, Esc lo cierra y se come la tecla (no va al
        // shell). El resto de teclas siguen su curso normal.
        if let Some(msg) = menu::intercept_key(model, e) {
            return Some(msg);
        }
        // Reenvía teclas al módulo focado. Hoy sólo el shell consume
        // teclas (input del REPL); el resto de módulos siguen sin
        // recibirlas hasta que las necesiten.
        forward_key_to_focused_shell(model, e)
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        // `delta.y` viene en líneas (positivo = hacia abajo). El scroll
        // del shell mide px desde el fondo, donde positivo = ver
        // historial, así que invertimos y escalamos a ~40 px por línea.
        let dpx = -delta.y * 40.0;
        if dpx == 0.0 {
            return None;
        }
        forward_wheel_to_focused_shell(model, dpx)
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                m.last_snapshot = Some(m.sysmon.sample());
                sample_extra_monitors(&mut m);
            }
            Msg::ShellTick => {
                drain_shell_instances(&mut m);
            }
            Msg::WawaConfigChanged(cfg) => {
                // Re-armar el theme con el nuevo variant + accent. El
                // fallback es el theme actual — si la nueva config tiene
                // un variant raro, conservamos lo de antes.
                m.theme = wawa_config_llimphi::theme_from_wawa(&cfg, &m.theme);
                // Locale activo — `set_locale` es no-op si el lang no
                // está en el catálogo; los próximos `t(...)` ya devuelven
                // strings en el nuevo idioma sin necesidad de reiniciar
                // (los labels in-memory siguen siendo viejos hasta que
                // el módulo correspondiente vuelva a rehidratarlos,
                // pero todo lo que se calcula en cada `view()` se
                // refresca al instante).
                let _ = rimay_localize::set_locale(&cfg.lang);
            }
            // Click en un diente de sesión: lo selecciona y abre su panel.
            // Re-clickear el activo cierra/abre su panel (cada diente ES su panel).
            Msg::SelectSession(i) => {
                if i < m.sessions.len() {
                    if i == m.active_session {
                        m.session_panel_open = !m.session_panel_open;
                    } else {
                        m.active_session = i;
                        m.session_panel_open = true;
                    }
                }
            }
            // Click en una herramienta: toggle de su panel (re-click cierra).
            Msg::SelectTool(t) => {
                m.active_tool = if m.active_tool == Some(t) { None } else { Some(t) };
            }
            Msg::RunFromHistory(cmd) => {
                let slot = Slot::Session(m.active_session, Which::Shell);
                m = apply_module_msg(
                    m,
                    slot,
                    ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(cmd)),
                );
            }
            Msg::ToggleDropdown(kind) => {
                m.dropdown_open = if m.dropdown_open == Some(kind) { None } else { Some(kind) };
            }
            Msg::DismissDropdown => m.dropdown_open = None,
            // Config del aislamiento. Sobre la draft, configurarla la promueve
            // a sesión propia (y nace un draft nuevo); sobre una real, edita.
            Msg::SetIsolation(iso) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.isolation = iso;
                }
                promote_if_draft(&mut m);
                // El cambio de aislamiento reconstruye el shell con su source.
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.apply_isolation();
                }
                save_sessions(&m);
            }
            // Abrir/cerrar el colapsable de contenedor (capa opcional). Al abrir,
            // listamos los contenedores locales.
            Msg::ToggleContainer => {
                let mut opening = false;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container_open = !s.container_open;
                    opening = s.container_open;
                }
                if opening {
                    spawn_list_containers(handle);
                }
            }
            Msg::SetDistro(d) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.distro = d;
                }
                promote_if_draft(&mut m);
                save_sessions(&m);
            }
            Msg::RefreshContainers => spawn_list_containers(handle),
            Msg::ContainersLoaded(v) => m.containers = v,
            Msg::SubscribeContainer(i) => {
                m.dropdown_open = None;
                if let Some(name) = m.containers.get(i).cloned() {
                    if let Some(s) = m.sessions.get_mut(m.active_session) {
                        s.container = Some(name);
                        s.conn = ConnState::Connected;
                    }
                }
                save_sessions(&m);
            }
            Msg::CreateContainer => {
                m.dropdown_open = None;
                // Crea un contenedor de la distro de la sesión y re-lista. (El
                // exec del shell dentro de él es deuda — falta Source::Container.)
                let (distro, n) = m
                    .sessions
                    .get(m.active_session)
                    .map(|s| (s.distro, s.number.unwrap_or(0)))
                    .unwrap_or((Distro::Ubuntu, 0));
                let name = format!("shuma-{}-{n}", distro.label().to_lowercase());
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container = Some(name.clone());
                    s.conn = ConnState::Connected;
                }
                spawn_create_container(handle, distro.image(), name);
                save_sessions(&m);
            }
            Msg::CloseSession(idx) => {
                // La draft (0) no se cierra; las demás se descartan.
                if idx > 0 && idx < m.sessions.len() {
                    m.sessions.remove(idx);
                    m.active_session = m.active_session.min(m.sessions.len() - 1);
                }
                save_sessions(&m);
            }
            Msg::ReorderSession(from, to) => {
                // La draft (0) queda fija; el resto se reordena.
                let len = m.sessions.len();
                if from > 0 && from < len && to > 0 && to < len && from != to {
                    let s = m.sessions.remove(from);
                    m.sessions.insert(to, s);
                    m.active_session = to;
                }
                save_sessions(&m);
            }
            Msg::SetSessionWidth(dx) => {
                m.session_w = (m.session_w + dx).clamp(180.0, 480.0);
            }
            Msg::SetToolWidth(dx) => {
                m.monitors_width = (m.monitors_width - dx).clamp(180.0, 480.0);
            }
            Msg::Module(slot, mmsg) => {
                // Hook: SelectRoot del módulo minga dispara la carga
                // de la fuente reconstruida en un thread aparte. El
                // mensaje se sigue propagando para que el state marque
                // `selected = Some(alpha)` y `selected_source = None`
                // mientras carga.
                if let ModuleMsg::Minga(shuma_module_minga::Msg::SelectRoot(alpha)) = &mmsg {
                    if let Some(repo_path) = minga_repo_path(&slot, &m) {
                        let alpha = *alpha;
                        let slot_back = slot.clone();
                        handle.spawn(move || {
                            let result = shuma_module_minga::load_root_source(&repo_path, alpha);
                            Msg::Module(
                                slot_back,
                                ModuleMsg::Minga(shuma_module_minga::Msg::SourceLoaded {
                                    alpha,
                                    result,
                                }),
                            )
                        });
                    }
                }
                m = apply_module_msg(m, slot, mmsg);
            }
            Msg::ShortcutClicked(slot, action) => {
                m = handle_shortcut(m, slot, action, handle);
            }
            Msg::MenuOpen(idx) => {
                m.menu_open = idx;
                m.menu_active = usize::MAX;
                // Abrir el menú principal cierra el contextual (y viceversa).
                m.ctx_menu = None;
                // Animación de aparición/swap: cada vez que se abre (o se
                // cambia de) menú, el dropdown se funde+desliza de nuevo.
                if idx.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = menu::app_menu(&m);
                    m.menu_active =
                        llimphi_widget_menubar::menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = menu::app_menu(&m);
                    if let Some(cmd) =
                        llimphi_widget_menubar::menubar_command_at(&menu, mi, m.menu_active)
                    {
                        m = menu::handle_command(m, &cmd);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::ContextMenuOpen(x, y) => {
                m.ctx_menu = Some((x, y));
                m.menu_open = None;
                m.menu_active = usize::MAX;
            }
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.ctx_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m = menu::handle_command(m, &cmd);
            }
            Msg::HostActivate(id) => {
                // Rail hospedado: un diente de herramienta abre/cierra su panel.
                if let Some(t) = Tool::ALL.get(id as usize) {
                    m.active_tool = if m.active_tool == Some(*t) { None } else { Some(*t) };
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;

        let menubar = menu::menubar_row(model, theme);
        let topbar = render_topbar(model, theme);
        let main_area = render_main_area(model, theme);
        let bottombar = render_bottombar(model, theme);

        // El right-click se engancha en la raíz (origen 0,0 → las coords
        // locales que llegan al handler ya son de ventana) y abre el menú
        // contextual de terminal. Un nodo hijo con su propio handler de
        // right-click ganaría; hoy ninguno lo pone, así que la raíz es el
        // catch-all.
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, topbar, main_area, bottombar])
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // El dropdown de config (select) tiene prioridad sobre el menú.
        view::dropdown_overlay(model).or_else(|| menu::overlay(model))
    }
}

// Helpers partidos del monolito (regla dura #1, 1522 LOC): update + view.
mod menu;
mod update;
mod view;

use update::*;
use view::*;
