//! Tipos del chasis de shuma: enums, structs de estado y mensajes.
//!
//! Toda la definición de `Model`, `Msg`, `Session`, `Instance`, etc.
//! vive aquí para mantener `main.rs` como punto de entrada limpio.

use std::collections::HashMap;

use llimphi_motion::Tween;
use llimphi_theme::Theme;
use llimphi_widget_panes::{Axis, PaneId, Side};
use llimphi_widget_text_input::TextInputState;
use shuma_module::{ModuleContributions, Source};
use shuma_sysmon::{Snapshot, SystemSampler};

use crate::containers::{prepare_rootfs, rootfs_path_for, rootfs_listo};
use crate::env::{default_shell_source, engine_preferido};
use crate::hosts;
use crate::workspace::Workspace;

// ─── Tipos de módulos conocidos ────────────────────────────────────

/// Qué `Kind` puede ocupar cada slot. Una variante por módulo compilado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Launcher,
    CommandBar,
    Shell,
    Matilda,
    Minga,
    Canvas,
}

impl Kind {
    /// `id` canónico.
    #[allow(dead_code)]
    pub(crate) fn id(self) -> &'static str {
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

/// Cuál instancia-módulo de una sesión direcciona un `Slot` o un `Msg`.
/// `Shell` es el panel **con foco** del workspace tiling; `Pane(id)`
/// direcciona un panel concreto (tiled o flotante) de la tab activa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Shell,
    Canvas,
    Matilda,
    Pane(PaneId),
}

/// Dónde corre el shell de la sesión.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Isolation {
    Local,
    Remote,
}

impl Isolation {
    pub(crate) const ALL: [Isolation; 2] = [Isolation::Local, Isolation::Remote];
    #[allow(dead_code)]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Isolation::Local => "Local",
            Isolation::Remote => "Remoto",
        }
    }
}

/// Estado de conexión de la sesión.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnState {
    Pending,
    Connected,
    #[allow(dead_code)]
    Disconnected,
}

impl ConnState {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ConnState::Pending => "en espera",
            ConnState::Connected => "conectado",
            ConnState::Disconnected => "desconectado",
        }
    }
}

/// La distro del aislamiento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Distro {
    Ubuntu,
    Debian,
    Alpine,
    Arch,
}

impl Distro {
    pub(crate) const ALL: [Distro; 4] = [Distro::Ubuntu, Distro::Debian, Distro::Alpine, Distro::Arch];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Distro::Ubuntu => "Ubuntu",
            Distro::Debian => "Debian",
            Distro::Alpine => "Alpine",
            Distro::Arch => "Arch",
        }
    }
    /// Imagen OCI fully-qualified para `podman run`.
    pub(crate) fn image(self) -> &'static str {
        match self {
            Distro::Ubuntu => "docker.io/library/ubuntu:latest",
            Distro::Debian => "docker.io/library/debian:latest",
            Distro::Alpine => "docker.io/library/alpine:latest",
            Distro::Arch => "docker.io/library/archlinux:latest",
        }
    }
}

/// Distro a partir del nombre de un rootfs.
pub(crate) fn distro_from_name(name: &str) -> Option<Distro> {
    let n = name.to_lowercase();
    Distro::ALL.into_iter().find(|d| d.label().to_lowercase() == n)
}

/// Campo del form de conexión remota con foco de teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteField {
    Host,
    User,
    Port,
}

/// Campo del form de creación de sesión nueva con foco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingField {
    Mount,
}

/// Estado de un container listado en la ventana gestora.
#[derive(Debug, Clone)]
pub(crate) struct ContainerInfo {
    pub name: String,
    pub status: String,
    pub image: String,
    /// `true` = rootfs en disco (unshare/bwrap).
    pub rootfs: bool,
}

/// Una entrada del listado del Explorer (un archivo o directorio del cwd).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExplorerEntry {
    pub is_dir: bool,
    pub name: String,
}

/// Estado del listado remoto del Explorer.
#[derive(Default)]
pub(crate) enum ExplorerState {
    /// Nada pedido todavía.
    #[default]
    Idle,
    /// Listado en curso (off-thread por SSH).
    Loading,
    /// Listado listo.
    Loaded(Vec<ExplorerEntry>),
    /// El listado falló (mensaje para mostrar).
    Error(String),
}

/// Cache del listado del Explorer para sesiones **remotas** (Remote /
/// RemoteContainer). `read_dir` local no alcanza al filesystem del host
/// remoto, así que el contenido se trae off-thread por SSH y se cachea acá.
/// La `key` ata el contenido a una `(sesión, cwd)` concreta — al cambiar
/// cualquiera de los dos, el reconciliador dispara un listado nuevo.
#[derive(Default)]
pub(crate) struct ExplorerCache {
    /// `(índice de sesión, cwd)` que refleja `state`; `None` = vacío.
    pub key: Option<(usize, String)>,
    pub state: ExplorerState,
}

/// Un directorio del host montado dentro del contenedor.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Mount {
    pub host: String,
    pub target: String,
    #[serde(default)]
    pub readonly: bool,
}

/// Config persistida de un contenedor.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContainerCfg {
    pub name: String,
    #[serde(default = "host_local")]
    pub host: String,
    pub engine: String,
    pub distro: Distro,
    #[serde(default)]
    pub mounts: Vec<Mount>,
}

pub(crate) fn host_local() -> String {
    "local".to_string()
}

/// Columna de un mount con foco de teclado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MountCol {
    Host,
    Target,
}

/// Una fila de mount en el editor.
#[derive(Debug, Clone)]
pub(crate) struct MountDraft {
    pub host: TextInputState,
    pub target: TextInputState,
    pub readonly: bool,
}

impl MountDraft {
    pub(crate) fn new() -> Self {
        Self {
            host: TextInputState::new(),
            target: TextInputState::new(),
            readonly: false,
        }
    }
    pub(crate) fn from_mount(m: &Mount) -> Self {
        let mut host = TextInputState::new();
        host.set_text(m.host.clone());
        let mut target = TextInputState::new();
        target.set_text(m.target.clone());
        Self { host, target, readonly: m.readonly }
    }
    pub(crate) fn to_mount(&self) -> Option<Mount> {
        let host = self.host.text();
        let target = self.target.text();
        if host.trim().is_empty() || target.trim().is_empty() {
            return None;
        }
        Some(Mount { host, target, readonly: self.readonly })
    }
}

/// Editor de contenedor del gestor.
#[derive(Debug, Clone)]
pub(crate) struct ContainerDraft {
    pub editing: Option<String>,
    pub host: String,
    pub engine: String,
    pub distro: Distro,
    pub mounts: Vec<MountDraft>,
    pub focus: Option<(usize, MountCol)>,
}

impl ContainerDraft {
    pub(crate) fn new(host: String) -> Self {
        Self {
            editing: None,
            host,
            engine: engine_preferido().unwrap_or("unshare").to_string(),
            distro: Distro::Ubuntu,
            mounts: Vec::new(),
            focus: None,
        }
    }
    pub(crate) fn from_cfg(cfg: &ContainerCfg) -> Self {
        Self {
            editing: Some(cfg.name.clone()),
            host: cfg.host.clone(),
            engine: cfg.engine.clone(),
            distro: cfg.distro,
            mounts: cfg.mounts.iter().map(MountDraft::from_mount).collect(),
            focus: None,
        }
    }
    pub(crate) fn to_cfg(&self, name: String) -> ContainerCfg {
        ContainerCfg {
            name,
            host: self.host.clone(),
            engine: self.engine.clone(),
            distro: self.distro,
            mounts: self.mounts.iter().filter_map(MountDraft::to_mount).collect(),
        }
    }
}

/// Form para crear/editar un host remoto.
#[derive(Debug, Clone)]
pub(crate) struct HostDraft {
    pub name: TextInputState,
    pub host: TextInputState,
    pub user: TextInputState,
    pub port: TextInputState,
    pub use_password: bool,
    pub pem_path: TextInputState,
    pub focused: Option<HostDraftField>,
    pub editing: Option<String>,
}

impl HostDraft {
    pub(crate) fn new() -> Self {
        let mut port = TextInputState::new();
        port.set_text("22");
        Self {
            name: TextInputState::new(),
            host: TextInputState::new(),
            user: TextInputState::new(),
            port,
            use_password: true,
            pem_path: TextInputState::new(),
            focused: Some(HostDraftField::Name),
            editing: None,
        }
    }

    pub(crate) fn from_host(h: &hosts::RemoteHost) -> Self {
        let mut name = TextInputState::new();
        name.set_text(h.name.clone());
        let mut host = TextInputState::new();
        host.set_text(h.host.clone());
        let mut user = TextInputState::new();
        user.set_text(h.user.clone());
        let mut port = TextInputState::new();
        port.set_text(h.port.to_string());
        let (use_password, pem) = match &h.auth {
            hosts::HostAuth::Password => (true, String::new()),
            hosts::HostAuth::Key { path } => (false, path.clone()),
        };
        let mut pem_path = TextInputState::new();
        pem_path.set_text(pem);
        Self {
            name,
            host,
            user,
            port,
            use_password,
            pem_path,
            focused: Some(HostDraftField::Name),
            editing: Some(h.name.clone()),
        }
    }

    pub(crate) fn to_host(&self) -> Option<hosts::RemoteHost> {
        let name = self.name.text();
        let host = self.host.text();
        let user = self.user.text();
        if name.trim().is_empty() || host.trim().is_empty() || user.trim().is_empty() {
            return None;
        }
        let port: u16 = self.port.text().trim().parse().unwrap_or(22);
        let auth = if self.use_password {
            hosts::HostAuth::Password
        } else {
            let path = self.pem_path.text();
            hosts::HostAuth::Key { path }
        };
        Some(hosts::RemoteHost { name, host, user, port, auth })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostDraftField {
    Name,
    Host,
    User,
    Port,
    Pem,
}

/// Cuál dropdown de la config de sesión está abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DropKind {
    Isolation,
    Distro,
    Container,
    Engine,
    Host,
}

/// El tipo de una sesión.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionKind {
    Draft,
    Local,
    #[allow(dead_code)]
    Remote,
}

/// Las herramientas de la sesión activa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Tool {
    History,
    Monitor,
    Explorer,
    Matilda,
}

impl Tool {
    pub(crate) const ALL: [Tool; 4] = [Tool::History, Tool::Monitor, Tool::Explorer, Tool::Matilda];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Tool::History => "Historial",
            Tool::Monitor => "Monitor",
            Tool::Explorer => "Explorer",
            Tool::Matilda => "Matilda",
        }
    }
}

/// State vivo de un módulo.
pub(crate) enum ModuleState {
    Launcher(shuma_module_launcher::State),
    CommandBar(shuma_module_commandbar::State),
    Shell(shuma_module_shell::State),
    Matilda(Box<shuma_module_matilda::State>),
    Minga(shuma_module_minga::State),
    Canvas(shuma_module_canvas::State),
}

/// Una instancia activa de un módulo.
pub(crate) struct Instance {
    pub kind: Kind,
    #[allow(dead_code)]
    pub label: String,
    pub state: ModuleState,
}

impl Instance {
    pub(crate) fn launcher(state: shuma_module_launcher::State) -> Self {
        Self {
            kind: Kind::Launcher,
            label: rimay_localize::t("shuma-label-launcher"),
            state: ModuleState::Launcher(state),
        }
    }

    pub(crate) fn command_bar(state: shuma_module_commandbar::State) -> Self {
        Self {
            kind: Kind::CommandBar,
            label: rimay_localize::t("shuma-label-command"),
            state: ModuleState::CommandBar(state),
        }
    }

    pub(crate) fn shell(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Shell,
            label,
            state: ModuleState::Shell(shuma_module_shell::State::new(source)),
        }
    }

    pub(crate) fn matilda(label: String, source: Source) -> Self {
        Self::matilda_with_inventory(label, source, None)
    }

    pub(crate) fn matilda_with_inventory(
        label: String,
        source: Source,
        inventory: Option<&std::path::Path>,
    ) -> Self {
        use crate::update::{example_inventory_fallback, load_matilda_inventory};
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

    pub(crate) fn minga(label: String, source: Source) -> Self {
        Self {
            kind: Kind::Minga,
            label,
            state: ModuleState::Minga(shuma_module_minga::State::new(source)),
        }
    }

    pub(crate) fn canvas(label: String) -> Self {
        Self {
            kind: Kind::Canvas,
            label,
            state: ModuleState::Canvas(shuma_module_canvas::State::new()),
        }
    }
}

// ─── Sesión de trabajo ──────────────────────────────────────────────

pub(crate) struct Session {
    pub name: String,
    pub kind: SessionKind,
    pub number: Option<u32>,
    pub isolation: Isolation,
    pub distro: Distro,
    pub container: Option<String>,
    pub use_container: bool,
    pub container_engine: String,
    pub container_open: bool,
    pub conn: ConnState,
    pub host_label: Option<String>,
    pub host: TextInputState,
    pub user: TextInputState,
    pub port: TextInputState,
    pub pending: bool,
    pub mount: TextInputState,
    pub pending_focus: Option<PendingField>,
    /// Persistir el output del shell a disco y restaurarlo al reabrir.
    pub persist: bool,
    /// Perfil de **apariencia** propio de esta sesión (la "ventana"). `None` =
    /// usa el default global. Gana sobre el global cuando la sesión está activa.
    pub appearance: Option<String>,
    pub source: Source,
    /// El layout tipo zellij de esta sesión: tabs + tiling + flotantes. Cada
    /// panel es un shell vivo; `shell()` devuelve el panel con foco.
    pub workspace: Workspace,
    pub canvas: Instance,
    pub matilda: Instance,
}

impl Session {
    pub(crate) fn build(name: String, kind: SessionKind, number: Option<u32>, source: Source) -> Self {
        Self {
            workspace: Workspace::single(Instance::shell(name.clone(), source.clone())),
            canvas: Instance::canvas(rimay_localize::t("shuma-label-canvas")),
            matilda: Instance::matilda(name.clone(), source.clone()),
            name,
            kind,
            number,
            isolation: Isolation::Local,
            distro: Distro::Ubuntu,
            container: None,
            use_container: false,
            container_engine: engine_preferido().unwrap_or("bwrap").to_string(),
            container_open: false,
            pending: false,
            mount: TextInputState::new(),
            pending_focus: None,
            persist: false,
            appearance: None,
            conn: ConnState::Connected,
            host_label: None,
            host: TextInputState::new(),
            user: TextInputState::new(),
            port: {
                let mut p = TextInputState::new();
                p.set_text("22");
                p
            },
            source,
        }
    }

    pub(crate) fn draft() -> Self {
        Self::build("draft".to_string(), SessionKind::Draft, None, default_shell_source())
    }

    pub(crate) fn new_pending(n: u32) -> Self {
        let mut s = Self::build(
            format!("local {n}"),
            SessionKind::Local,
            Some(n),
            Source::Local,
        );
        s.pending = true;
        s
    }

    pub(crate) fn host_key(&self) -> String {
        self.host_label.clone().unwrap_or_else(|| "local".to_string())
    }

    /// El shell con foco del workspace tiling — el que recibe el teclado y el
    /// que el chasis trata como "el shell de la sesión".
    pub(crate) fn shell(&self) -> &Instance {
        self.workspace.focused_instance()
    }

    pub(crate) fn shell_mut(&mut self) -> &mut Instance {
        self.workspace.focused_instance_mut()
    }

    pub(crate) fn active_data(&self) -> bool {
        matches!(&self.shell().state, ModuleState::Shell(s) if s.is_running())
    }

    /// Estado de actividad del shell con foco — alimenta el color del LED del
    /// diente (quieto / movimiento / claude).
    pub(crate) fn activity(&self) -> shuma_module_shell::Activity {
        match &self.shell().state {
            ModuleState::Shell(s) => s.activity(),
            _ => shuma_module_shell::Activity::Idle,
        }
    }

    /// A6 — comandos largos terminados pendientes de acuse en esta sesión (la
    /// badge del diente). `0` si no es un shell o no hay nada pendiente.
    pub(crate) fn long_alerts(&self) -> usize {
        match &self.shell().state {
            ModuleState::Shell(s) => s.long_alerts(),
            _ => 0,
        }
    }

    /// A6 — el usuario miró esta sesión: limpia la badge de comando largo.
    pub(crate) fn ack_long_alerts(&mut self) {
        if let ModuleState::Shell(s) = &mut self.shell_mut().state {
            s.ack_long_alerts();
        }
    }

    pub(crate) fn port_num(&self) -> u16 {
        self.port.text().trim().parse().unwrap_or(22)
    }

    pub(crate) fn resolve_source(&self) -> Source {
        match self.isolation {
            Isolation::Local => match (self.use_container, self.container.clone()) {
                (true, Some(name)) => Source::Container {
                    engine: self.container_engine.clone(),
                    name,
                    label: None,
                },
                _ => Source::Local,
            },
            Isolation::Remote => {
                let host = self.host.text();
                let user = self.user.text();
                match (self.use_container, self.container.clone()) {
                    (true, Some(name)) => Source::RemoteContainer {
                        host,
                        user,
                        port: self.port_num(),
                        engine: self.container_engine.clone(),
                        name,
                        label: None,
                    },
                    _ => Source::Remote {
                        host,
                        user,
                        port: self.port_num(),
                        label: None,
                    },
                }
            }
        }
    }

    pub(crate) fn apply_isolation(&mut self) {
        if self.isolation == Isolation::Local && self.use_container {
            if let Some(name) = self.container.clone() {
                if matches!(self.container_engine.as_str(), "unshare" | "bwrap") {
                    prepare_rootfs(std::path::Path::new(&name));
                }
            }
        }
        let source = self.resolve_source();
        self.conn = if self.use_container {
            ConnState::Pending
        } else {
            match self.isolation {
                Isolation::Local => ConnState::Connected,
                Isolation::Remote => ConnState::Pending,
            }
        };
        *self.shell_mut() = Instance::shell(self.name.clone(), source.clone());
        self.matilda = Instance::matilda(self.name.clone(), source.clone());
        self.source = source;
    }

    pub(crate) fn instance(&self, w: Which) -> &Instance {
        match w {
            Which::Shell => self.shell(),
            Which::Canvas => &self.canvas,
            Which::Matilda => &self.matilda,
            Which::Pane(id) => self.workspace.pane(id).unwrap_or_else(|| self.shell()),
        }
    }

    pub(crate) fn instance_mut(&mut self, w: Which) -> &mut Instance {
        match w {
            Which::Shell => self.shell_mut(),
            Which::Canvas => &mut self.canvas,
            Which::Matilda => &mut self.matilda,
            Which::Pane(id) => {
                if self.workspace.pane(id).is_some() {
                    self.workspace.pane_mut(id).unwrap()
                } else {
                    self.shell_mut()
                }
            }
        }
    }

    pub(crate) fn to_config(&self) -> SessionConfig {
        SessionConfig {
            name: self.name.clone(),
            number: self.number,
            isolation: self.isolation,
            distro: self.distro,
            container: self.container.clone(),
            use_container: self.use_container,
            container_engine: self.container_engine.clone(),
            mount: self.mount.text(),
            host_label: self.host_label.clone(),
            host: self.host.text(),
            user: self.user.text(),
            port: self.port.text(),
            persist: self.persist,
            appearance: self.appearance.clone(),
        }
    }

    pub(crate) fn from_config(c: SessionConfig) -> Self {
        use crate::env::binary_disponible;
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
        s.use_container = c.use_container;
        if !c.container_engine.is_empty() && binary_disponible(&c.container_engine) {
            s.container_engine = c.container_engine;
        } else if let Some(pref) = engine_preferido() {
            s.container_engine = pref.to_string();
        }
        s.mount.set_text(c.mount);
        s.host_label = c.host_label;
        s.host.set_text(c.host);
        s.user.set_text(c.user);
        if !c.port.is_empty() {
            s.port.set_text(c.port);
        }
        s.persist = c.persist;
        s.appearance = c.appearance;
        s.apply_isolation();
        s
    }

    pub(crate) fn remote_field_mut(&mut self, f: RemoteField) -> &mut TextInputState {
        match f {
            RemoteField::Host => &mut self.host,
            RemoteField::User => &mut self.user,
            RemoteField::Port => &mut self.port,
        }
    }

    pub(crate) fn connect_remote(&mut self) {
        if self.host.text().trim().is_empty() || self.user.text().trim().is_empty() {
            return;
        }
        let source = self.resolve_source();
        *self.shell_mut() = Instance::shell(self.name.clone(), source.clone());
        self.matilda = Instance::matilda(self.name.clone(), source.clone());
        self.source = source;
        self.conn = ConnState::Connected;
    }

    pub(crate) fn reconnect(&mut self) {
        if self.host_label.is_some() {
            self.connect_remote();
        } else {
            self.apply_isolation();
        }
    }
}

// ─── Config persistible ─────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct SessionConfig {
    pub name: String,
    #[serde(default)]
    pub number: Option<u32>,
    pub isolation: Isolation,
    pub distro: Distro,
    #[serde(default)]
    pub container: Option<String>,
    #[serde(default)]
    pub use_container: bool,
    #[serde(default)]
    pub container_engine: String,
    #[serde(default)]
    pub mount: String,
    #[serde(default)]
    pub host_label: Option<String>,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub port: String,
    #[serde(default)]
    pub persist: bool,
    /// Perfil de apariencia propio de la sesión (la "ventana"). `None` = global.
    #[serde(default)]
    pub appearance: Option<String>,
}

/// Estado de chrome persistible.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct ChromeState {
    #[serde(default)]
    pub active_tool: Option<Tool>,
    #[serde(default = "yes")]
    pub session_panel_open: bool,
    #[serde(default)]
    pub active_session: usize,
    #[serde(default = "default_session_w")]
    pub session_w: f32,
    #[serde(default = "default_monitors_width")]
    pub monitors_width: f32,
}

fn yes() -> bool { true }
fn default_session_w() -> f32 { 240.0 }
fn default_monitors_width() -> f32 { crate::MONITORS_INITIAL_WIDTH }

impl Default for ChromeState {
    fn default() -> Self {
        Self {
            active_tool: None,
            session_panel_open: true,
            active_session: 0,
            session_w: default_session_w(),
            monitors_width: default_monitors_width(),
        }
    }
}

/// Una **disposición guardada** (estilo "sesión de tmux").
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct LayoutSnapshot {
    pub name: String,
    pub sessions: Vec<SessionConfig>,
    pub chrome: ChromeState,
}

// ─── Mensajes del chasis ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum ModuleMsg {
    Launcher(shuma_module_launcher::Msg),
    CommandBar(shuma_module_commandbar::Msg),
    #[allow(dead_code)]
    Shell(shuma_module_shell::Msg),
    Matilda(shuma_module_matilda::Msg),
    Minga(shuma_module_minga::Msg),
    Canvas(shuma_module_canvas::Msg),
}

/// Identifica de dónde viene un `ModuleMsg`.
#[derive(Debug, Clone)]
pub(crate) enum Slot {
    TopBar,
    BottomBar,
    #[allow(dead_code)]
    Main,
    Session(usize, Which),
}

// ─── Perfiles ───────────────────────────────────────────────────────

/// Cuál de las tres bibliotecas de perfiles está mirando/gestionando el modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfKind {
    /// Atajos del workspace (globales).
    Shortcuts,
    /// Apariencia (global + por sesión).
    Appearance,
    /// Perfiles de sesión (contextos tipo Firefox).
    Sessions,
}

// ─── Modelo ─────────────────────────────────────────────────────────

pub struct Model {
    pub theme: Theme,

    /// `true` cuando shuma corre como **barra dockeada** (superficie
    /// wlr-layer-shell vía `llimphi-layer`), no como ventana. La fija `init`
    /// según el env `SHUMA_DOCK`. En modo dock la vista es compacta (la
    /// command-bar). Cuando shuma se hospeda en pata (módulo), queda `false`.
    pub dock_mode: bool,
    /// `true` si, en modo ventana, al perder el foco shuma debe replegarse a la
    /// barra dockeada (env `SHUMA_BAR_ON_BLUR`). Opt-in: por defecto `false`.
    pub collapse_on_blur: bool,

    /// Perfiles de **atajos** del workspace (globales, conmutables con un clic).
    pub shortcuts: crate::perfiles::shortcuts::ShortcutProfiles,
    /// Perfiles de **apariencia** (default global; cada sesión puede fijar el suyo).
    pub appearance: crate::perfiles::appearance::AppearanceProfiles,
    /// Índice de **perfiles de sesión** (contextos tipo Firefox).
    pub session_profiles: crate::perfiles::sessions::SessionProfiles,
    /// `true` mientras se esperó el prefijo de un keymap con prefijo (tmux/vim).
    /// Transitorio, no se persiste.
    pub pending_prefix: bool,
    /// Modal de gestión de perfiles abierto.
    pub perfiles_modal_open: bool,
    /// Pestaña activa del modal de perfiles.
    pub perfiles_tab: ProfKind,
    /// Campo de nombre del modal de perfiles (crear/duplicar/renombrar).
    pub prof_name: TextInputState,
    /// `true` si el campo de nombre del modal de perfiles tiene foco.
    pub prof_name_focused: bool,
    /// Wallpaper decodificado de la apariencia efectiva (cacheado; clon barato
    /// por frame). `None` = sin wallpaper. Lo refresca `apply_active_appearance`.
    pub wallpaper_img: Option<llimphi_image::Image>,
    /// Path del wallpaper cacheado — para no re-decodificar si no cambió.
    pub wallpaper_path: Option<String>,
    /// Campo del modal de perfiles para escribir el path del wallpaper.
    pub wp_path: TextInputState,
    /// `true` si el campo de wallpaper tiene foco.
    pub wp_path_focused: bool,

    pub topbar: Option<Instance>,
    pub bottombar: Option<Instance>,
    pub main: Option<Instance>,

    pub sessions: Vec<Session>,
    pub active_session: usize,
    pub hovered_session: Option<usize>,
    pub active_tool: Option<Tool>,
    pub session_panel_open: bool,
    pub dropdown_open: Option<DropKind>,
    pub containers: Vec<String>,
    pub remote_containers: Vec<String>,
    pub remote_new_distro: Distro,
    pub containers_full: Vec<ContainerInfo>,
    pub container_cfgs: Vec<ContainerCfg>,
    pub focused_field: Option<RemoteField>,
    pub hosts: Vec<hosts::RemoteHost>,
    pub host_draft: Option<HostDraft>,
    pub container_draft: Option<ContainerDraft>,
    pub hosts_modal_open: bool,
    pub containers_modal_open: bool,
    pub layouts: Vec<LayoutSnapshot>,
    pub layouts_modal_open: bool,
    /// Listado del Explorer para sesiones remotas (off-thread por SSH).
    pub explorer: ExplorerCache,
    pub layout_name: TextInputState,
    pub layout_name_focused: bool,
    pub viewport: (f32, f32),

    pub session_w: f32,
    pub sysmon: SystemSampler,
    pub last_snapshot: Option<Snapshot>,
    pub monitors_width: f32,
    pub extra_history: HashMap<String, Vec<f32>>,
    pub extra_display: HashMap<String, String>,
    pub _wawa_watcher: Option<wawa_config::ConfigWatcher>,

    pub menu_open: Option<usize>,
    pub menu_active: usize,
    pub menu_anim: Tween<f32>,
    pub ctx_menu: Option<(f32, f32)>,
    /// Menú contextual de una tab abierto: (índice de tab, x, y).
    pub tab_ctx: Option<(usize, f32, f32)>,

    /// Grupos de environment (env.json) — el panel del sidebar los lista
    /// y activa/desactiva en bloque; `:env` los alimenta desde el teclado.
    pub env_groups: Vec<shuma_config::EnvGroup>,
    /// mtime de env.json al último load — para recargar si el builtin
    /// (u otra instancia) lo tocó.
    pub env_groups_mtime: Option<std::time::SystemTime>,
    /// Contador de Msg::Tick (1 s) — debounce del autosave de outputs.
    pub tick_count: u64,

    /// `true` cuando un host (pata) muestra el **input de la sesión activa en su
    /// propia barra** (vía `active_input_view`): el canvas entonces pinta el
    /// cuerpo del shell SIN su input, para no duplicarlo. Default `false`
    /// (standalone: input dentro del canvas, como siempre).
    pub hosted_bar: bool,

    pub _host: Option<pata_host::HostClient>,
}

impl Model {
    pub(crate) fn active(&self) -> Option<&Session> {
        self.sessions.get(self.active_session).or_else(|| self.sessions.first())
    }

    pub(crate) fn active_remote_target(&self) -> Option<(String, String, u16, String)> {
        let s = self.active()?;
        if s.isolation != Isolation::Remote {
            return None;
        }
        let engine = if matches!(s.container_engine.as_str(), "podman" | "docker") {
            s.container_engine.clone()
        } else {
            "podman".to_string()
        };
        Some((s.host.text(), s.user.text(), s.port_num(), engine))
    }

    pub(crate) fn session_instance(&self, idx: usize, w: Which) -> Option<&Instance> {
        self.sessions.get(idx).map(|s| s.instance(w))
    }

    pub(crate) fn session_instance_mut(&mut self, idx: usize, w: Which) -> Option<&mut Instance> {
        self.sessions.get_mut(idx).map(|s| s.instance_mut(w))
    }
}

// ─── Enum de mensajes de la app ─────────────────────────────────────

#[derive(Clone)]
pub enum Msg {
    Tick,
    ShellTick,
    Resized(f32, f32),
    SelectSession(usize),
    HoverSession(Option<usize>),
    SelectTool(Tool),
    ToggleDropdown(DropKind),
    DismissDropdown,
    SetIsolation(Isolation),
    SetDistro(Distro),
    Noop,
    ToggleContainer,
    FocusField(RemoteField),
    RemoteKey(llimphi_ui::KeyEvent),
    ConnectRemote,
    ReconnectSession(usize),
    CloseSession(usize),
    /// Flag «Persistir sesión» del panel: guarda/restaura el output.
    ToggleSessionPersist(usize),
    /// Activa/desactiva un grupo de environment (índice en `env_groups`).
    ToggleEnvGroup(usize),
    OpenNewSessionForm,
    ConfirmNewSession,
    CancelNewSession,
    FocusPendingField(PendingField),
    PendingKey(llimphi_ui::KeyEvent),
    RefreshContainers,
    ContainersLoaded(Vec<String>),
    /// Resultado de listar el cwd de una sesión remota (off-thread por SSH).
    ExplorerLoaded {
        session: usize,
        path: String,
        result: Result<Vec<ExplorerEntry>, String>,
    },
    /// Fuerza re-listar el cwd remoto del Explorer (botón ↻).
    RefreshExplorer,
    RemoteContainersLoaded(Vec<String>),
    SubscribeContainer(usize),
    PickRemoteContainer(String),
    CreateContainer,
    ToggleUseContainer,
    SetEngine(String),
    PickRootfs(Distro),
    ContainerCreated(String),
    ContainerFailed { name: String, reason: String },
    EnsureContainer(String),

    OpenContainersWindow,
    CloseContainersModal,
    ContainersFullLoaded(Vec<ContainerInfo>),
    RefreshContainersFull,
    StartContainer(String),
    StopContainer(String),
    RemoveContainer(String),
    RemoveRootfs(String),

    RefreshRemoteContainers,
    SetRemoteNewDistro(Distro),
    CreateRemoteContainer,
    RemoteStart(String),
    RemoteStop(String),
    RemoteRemove(String),

    OpenHostsWindow,
    CloseHostsModal,
    HostDraftStart,
    HostEdit(usize),
    HostDraftCancel,
    HostDraftSave,
    HostDraftFocus(HostDraftField),
    HostDraftKey(llimphi_ui::KeyEvent),
    HostDraftToggleAuth,
    HostDelete(usize),

    OpenLayoutsModal,
    CloseLayoutsModal,
    LayoutNameFocus,
    LayoutNameKey(llimphi_ui::KeyEvent),
    SaveLayout,
    RestoreLayout(usize),
    DeleteLayout(usize),

    ContainerDraftNew,
    ContainerDraftCancel,
    ContainerEdit(usize),
    ContainerDraftSetEngine(String),
    ContainerDraftSetDistro(Distro),
    ContainerDraftAddMount,
    ContainerDraftRemoveMount(usize),
    ContainerDraftToggleMountRo(usize),
    ContainerDraftFocusMount(usize, MountCol),
    ContainerDraftSave,
    ContainerDraftKey(llimphi_ui::KeyEvent),
    PickHost(Option<usize>),
    HostApply(usize),

    ReorderSession(usize, usize),
    SetSessionWidth(f32),
    SetToolWidth(f32),
    RunFromHistory(String),
    RunFromHistoryNow(String),
    Module(Slot, ModuleMsg),
    ShortcutClicked(Slot, shuma_module::ShortcutAction),
    WawaConfigChanged(Box<wawa_config::WawaConfig>),

    MenuOpen(Option<usize>),
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    MenuCommand(String),
    ContextMenuOpen(f32, f32),
    CloseMenus,

    HostActivate(u32),

    // ─── Perfiles (atajos · apariencia · sesión) ────────────────────
    /// Una acción de atajo resuelta por el keymap activo (directa o tras prefijo).
    ShortcutFire(crate::perfiles::shortcuts::ShortcutAction),
    /// Se pulsó el prefijo de un keymap con prefijo (tmux/vim): entra en pendiente.
    ShortcutEnterPrefix,
    /// Tecla suelta tras el prefijo (o cancelación): sale de pendiente.
    ShortcutCancelPrefix,
    /// Conmuta el perfil de atajos activo (global).
    SwitchShortcutProfile(String),
    /// Conmuta el perfil de apariencia global (default de toda ventana).
    SwitchAppearanceProfile(String),
    /// Fija la apariencia de la sesión activa (`None` = como el global).
    SetSessionAppearance(Option<String>),
    /// Conmuta el perfil de sesión activo (contexto tipo Firefox).
    SwitchSessionProfile(String),
    /// Abre el modal de gestión de perfiles.
    OpenPerfilesModal,
    /// Cierra el modal de gestión de perfiles.
    ClosePerfilesModal,
    /// Cambia la pestaña del modal de perfiles.
    PerfilesTab(ProfKind),
    /// Foca el campo de nombre del modal de perfiles.
    ProfNameFocus,
    /// Tecla en el campo de nombre del modal de perfiles.
    ProfNameKey(llimphi_ui::KeyEvent),
    /// Activa un perfil (lo mismo que conmutarlo) desde el modal.
    ProfUse(ProfKind, String),
    /// Duplica un perfil con el nombre del campo (o `<src> copia` si vacío).
    ProfDuplicate(ProfKind, String),
    /// Renombra un perfil al nombre del campo (sólo perfiles propios).
    ProfRename(ProfKind, String),
    /// Borra un perfil propio.
    ProfDelete(ProfKind, String),
    /// Crea un perfil nuevo con el nombre del campo (desde una base sensata).
    ProfCreate(ProfKind),
    /// Foca el campo de path del wallpaper.
    WpPathFocus,
    /// Tecla en el campo de path del wallpaper.
    WpPathKey(llimphi_ui::KeyEvent),
    /// Fija el wallpaper del perfil de apariencia activo al path del campo.
    SetWallpaperActive,
    /// Quita el wallpaper del perfil de apariencia activo.
    ClearWallpaperActive,

    // ─── Workspace tipo zellij (tabs · tiling · flotantes) ──────────
    /// Parte el panel con foco (Horizontal = lado a lado · Vertical = apilado).
    PaneSplit(Axis),
    /// Pone el foco en un panel concreto (click en el panel).
    PaneFocus(PaneId),
    /// Cierra el panel con foco.
    PaneClose,
    /// Cicla el foco entre paneles tiled (true = siguiente).
    PaneCycle(bool),
    /// Arrastra un divisor del tiling: ajusta el ratio del split por `path`.
    PaneResize(Vec<Side>, f32),
    /// Tab nueva (con un shell fresco).
    TabNew,
    /// Activa la tab `i`.
    TabSwitch(usize),
    /// Cierra la tab `i`.
    TabClose(usize),
    /// Cierra todas las tabs menos la `i`.
    TabCloseOthers(usize),
    /// Abre el menú contextual de la tab `i` en (x, y).
    TabCtxOpen(usize, f32, f32),
    /// Agrega un panel flotante nuevo.
    FloatNew,
    /// Enciende/apaga la capa de paneles flotantes.
    FloatToggle,
    /// Mueve un panel flotante por (dx, dy) px.
    FloatMove(PaneId, f32, f32),
}
