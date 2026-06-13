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

#![forbid(unsafe_code)]

mod config;
mod containers;
mod env;
mod hosts;
mod menu;
mod persist;
mod types;
mod update;
mod view;

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::{
    App, DragPhase, Handle, KeyEvent, KeyState, Modifiers, View, WheelDelta,
};
use llimphi_widget_text_input::TextInputState;
use shuma_module::{ModuleContributions, MonitorSpec, ShortcutAction, Source};
use shuma_sysmon::SystemSampler;
use std::collections::HashMap;

// Tipos y sub-módulos re-exportados al espacio raíz para que update/view
// los puedan usar con `use super::*` sin paths explícitos.
use containers::*;
use env::*;
use persist::*;
use types::*;
use update::*;
use view::*;

pub(crate) const HISTORY: usize = 60;
const TICK: Duration = Duration::from_secs(1);
/// Cadencia rápida para drenar el output del shell (streaming de
/// `shuma-exec`). 100 ms hace la salida sentirse en vivo sin comerse CPU notable.
const SHELL_TICK: Duration = Duration::from_millis(100);
pub(crate) const MONITORS_INITIAL_WIDTH: f32 = 280.0;

/// Construye el cliente del rail hospedado si `SHUMA_DELEGATE_SIDEBAR` está set.
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

/// Dientes que shuma presta al rail de pata: uno por herramienta.
fn host_tool_teeth() -> Vec<pata_host::HostedTooth> {
    Tool::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| pata_host::HostedTooth::new(i as u32, tool_icon_name(*t), t.label().to_string()))
        .collect()
}

/// Nombre de icono para una herramienta.
fn tool_icon_name(t: Tool) -> &'static str {
    match t {
        Tool::History => "tools",
        Tool::Monitor => "system",
        Tool::Explorer => "files",
        Tool::Matilda => "settings",
    }
}

fn main() {
    rimay_localize::init();
    // Cablear el askpass para sudo + ssh.
    if let Some(path) = resolve_askpass_path() {
        if std::env::var_os("SUDO_ASKPASS").is_none() {
            std::env::set_var("SUDO_ASKPASS", &path);
        }
        if std::env::var_os("SSH_ASKPASS").is_none() {
            std::env::set_var("SSH_ASKPASS", &path);
        }
        if std::env::var_os("SSH_ASKPASS_REQUIRE").is_none() {
            std::env::set_var("SSH_ASKPASS_REQUIRE", "force");
        }
    }
    llimphi_ui::run::<Shell>();
}

// ─── App impl ───────────────────────────────────────────────────────

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

        let mut sessions = vec![Session::draft()];
        for c in load_sessions() {
            let mut sess = Session::from_config(c);
            // Sesión persistente: rehidratar el output guardado en el shell
            // recién construido (los bloques viejos abren plegados).
            if sess.persist {
                if let Some(snap) = persist::load_session_output(&sess.name) {
                    if let ModuleState::Shell(st) = &mut sess.shell.state {
                        st.restore_output(snap);
                    }
                }
            }
            sessions.push(sess);
        }

        // Grupos de environment: cargar env.json (garantizando el grupo
        // «general», destino del builtin `:env`) y aplicar los activos al
        // proceso — los shells hijos los heredan.
        let mut env_groups = shuma_config::load_env_groups();
        if !env_groups.iter().any(|g| g.name == "general") {
            env_groups.insert(0, shuma_config::EnvGroup::new("general"));
            let _ = shuma_config::save_env_groups(&env_groups);
        }
        for g in &env_groups {
            if g.active {
                shuma_config::apply_env_group(g, true);
            }
        }
        let env_groups_mtime = persist::env_groups_mtime();

        for s in &sessions {
            if s.use_container {
                if let Some(name) = s.container.clone() {
                    handle.dispatch(Msg::EnsureContainer(name));
                }
            }
        }

        let chrome = load_chrome();
        let active_session = chrome.active_session.min(sessions.len().saturating_sub(1));
        let host = shuma_host(handle);

        Model {
            theme,
            topbar,
            bottombar,
            main,
            sessions,
            active_session,
            hovered_session: None,
            active_tool: chrome.active_tool,
            session_panel_open: chrome.session_panel_open,
            dropdown_open: None,
            containers: Vec::new(),
            remote_containers: Vec::new(),
            remote_new_distro: Distro::Ubuntu,
            containers_full: Vec::new(),
            container_cfgs: load_container_cfgs(),
            focused_field: None,
            hosts: hosts::load_hosts(),
            host_draft: None,
            container_draft: None,
            hosts_modal_open: false,
            containers_modal_open: false,
            layouts: load_layouts(),
            layouts_modal_open: false,
            explorer: ExplorerCache::default(),
            layout_name: TextInputState::new(),
            layout_name_focused: false,
            viewport: (1280.0, 800.0),
            session_w: chrome.session_w,
            sysmon: SystemSampler::new(HISTORY),
            last_snapshot: None,
            monitors_width: chrome.monitors_width,
            extra_history: HashMap::new(),
            extra_display: HashMap::new(),
            _wawa_watcher: wawa_watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            ctx_menu: None,
            env_groups,
            env_groups_mtime,
            tick_count: 0,
            _host: host,
        }
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Los modales bloqueantes capturan TODO el teclado.
        if model.hosts_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::CloseHostsModal);
            }
            return Some(Msg::HostDraftKey(e.clone()));
        }
        if model.containers_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::CloseContainersModal);
            }
            return Some(Msg::ContainerDraftKey(e.clone()));
        }
        if model.layouts_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::CloseLayoutsModal);
            }
            return Some(Msg::LayoutNameKey(e.clone()));
        }
        if model.focused_field.is_some() {
            return Some(Msg::RemoteKey(e.clone()));
        }
        if model.dropdown_open.is_some() {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::DismissDropdown);
            }
        }
        if let Some(msg) = menu::intercept_key(model, e) {
            return Some(msg);
        }
        forward_key_to_focused_shell(model, e)
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        if modifiers.ctrl && delta.y != 0.0 {
            let factor = if delta.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            return Some(Msg::Module(
                Slot::Session(model.active_session, Which::Shell),
                ModuleMsg::Shell(shuma_module_shell::Msg::ZoomBy(factor)),
            ));
        }
        if modifiers.shift && delta.y != 0.0 {
            let dx = delta.y * 40.0;
            return Some(Msg::Module(
                Slot::Session(model.active_session, Which::Shell),
                ModuleMsg::Shell(shuma_module_shell::Msg::ScrollHoriz(dx)),
            ));
        }
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
                m.tick_count += 1;
                // Autosave del output de las sesiones persistentes (cada 5 s).
                if m.tick_count % 5 == 0 {
                    save_session_outputs(&m);
                    // M4 — polling del runtime de matilda (si hay instancia
                    // Local montada): refresca el semáforo sin pulsar Discover.
                    update::poll_matilda_runtime(&m, handle);
                }
                // env.json cambió (builtin `:env` u otra instancia) → recargar.
                let mtime = persist::env_groups_mtime();
                if mtime != m.env_groups_mtime {
                    m.env_groups_mtime = mtime;
                    m.env_groups = shuma_config::load_env_groups();
                }
            }
            Msg::ShellTick => {
                drain_shell_instances(&mut m);
                // A6 — la sesión activa no badgea (el usuario la está viendo):
                // acusá sus comandos largos en cuanto terminan, así no aparece
                // una badge stale al cambiar de diente después de verlos vivos.
                let activa = m.active_session;
                if let Some(s) = m.sessions.get_mut(activa) {
                    s.ack_long_alerts();
                }
                // E5 — despachar peticiones LLM pendientes (`:?`/`:explica`/
                // `:resume`) a un thread; el resultado vuelve por LlmResult.
                update::fulfill_llm_requests(&mut m, handle);
                // El cwd remoto pudo cambiar tras un `cd`: re-listar si hace falta.
                reconcile_explorer(&mut m, handle);
            }
            Msg::Resized(w, h) => {
                if w > 0.0 && h > 0.0 {
                    m.viewport = (w, h);
                }
            }
            Msg::WawaConfigChanged(cfg) => {
                m.theme = wawa_config_llimphi::theme_from_wawa(&cfg, &m.theme);
                let _ = rimay_localize::set_locale(&cfg.lang);
            }
            Msg::SelectSession(i) => {
                if i < m.sessions.len() {
                    if i == m.active_session {
                        m.session_panel_open = !m.session_panel_open;
                    } else {
                        m.active_session = i;
                        m.session_panel_open = true;
                    }
                    // A6 — el usuario está mirando esta sesión: limpia su badge
                    // de comando largo.
                    m.sessions[i].ack_long_alerts();
                    save_chrome(&m);
                    reconcile_explorer(&mut m, handle);
                }
            }
            Msg::HoverSession(idx) => {
                m.hovered_session = idx.filter(|&i| i < m.sessions.len());
            }
            Msg::SelectTool(t) => {
                m.active_tool = if m.active_tool == Some(t) { None } else { Some(t) };
                save_chrome(&m);
                reconcile_explorer(&mut m, handle);
            }
            Msg::RunFromHistory(cmd) => {
                let slot = Slot::Session(m.active_session, Which::Shell);
                m = apply_module_msg(
                    m,
                    slot,
                    ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(cmd)),
                );
            }
            Msg::RunFromHistoryNow(cmd) => {
                let slot = Slot::Session(m.active_session, Which::Shell);
                m = apply_module_msg(
                    m,
                    slot,
                    ModuleMsg::Shell(shuma_module_shell::Msg::RunLine(cmd)),
                );
            }
            Msg::ToggleDropdown(kind) => {
                m.dropdown_open = if m.dropdown_open == Some(kind) { None } else { Some(kind) };
                if m.dropdown_open == Some(DropKind::Container) {
                    if let Some(s) = m.sessions.get(m.active_session) {
                        if s.isolation == Isolation::Remote {
                            spawn_list_remote_containers(
                                handle,
                                s.host.text(),
                                s.user.text(),
                                s.port_num(),
                                s.container_engine.clone(),
                            );
                        }
                    }
                }
            }
            Msg::DismissDropdown => m.dropdown_open = None,
            Msg::SetIsolation(iso) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.isolation = iso;
                    if !s.pending {
                        s.apply_isolation();
                    } else {
                        s.conn = match iso {
                            Isolation::Local => ConnState::Connected,
                            Isolation::Remote => ConnState::Pending,
                        };
                    }
                }
                save_sessions(&m);
            }
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
                save_sessions(&m);
            }
            Msg::FocusField(f) => {
                m.focused_field = Some(f);
                m.dropdown_open = None;
            }
            Msg::RemoteKey(e) => {
                let Some(f) = m.focused_field else { return m };
                match &e.key {
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                        m.focused_field = None;
                    }
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                        if let Some(s) = m.sessions.get_mut(m.active_session) {
                            s.connect_remote();
                        }
                        m.focused_field = None;
                        save_sessions(&m);
                    }
                    _ => {
                        if let Some(s) = m.sessions.get_mut(m.active_session) {
                            s.remote_field_mut(f).apply_key(&e);
                        }
                    }
                }
            }
            Msg::ConnectRemote => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.connect_remote();
                }
                m.focused_field = None;
                save_sessions(&m);
            }
            Msg::ReconnectSession(idx) => {
                if let Some(s) = m.sessions.get_mut(idx) {
                    s.reconnect();
                }
                m.focused_field = None;
                save_sessions(&m);
            }
            Msg::RefreshContainers => spawn_list_containers(handle),
            Msg::ContainersLoaded(v) => m.containers = v,
            Msg::ExplorerLoaded { session, path, result } => {
                // Aceptar sólo si sigue siendo la (sesión, cwd) que pedimos —
                // si el usuario cambió de sesión o de dir, este listado es viejo.
                if m.explorer.key.as_ref().is_some_and(|(s, p)| *s == session && p == &path) {
                    m.explorer.state = match result {
                        Ok(entries) => ExplorerState::Loaded(entries),
                        Err(e) => ExplorerState::Error(e),
                    };
                }
            }
            Msg::RefreshExplorer => {
                m.explorer = ExplorerCache::default();
                reconcile_explorer(&mut m, handle);
            }
            Msg::RemoteContainersLoaded(v) => m.remote_containers = v,
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
            Msg::PickRemoteContainer(name) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get(m.active_session) {
                    spawn_remote_engine_action(
                        handle,
                        s.host.text(),
                        s.user.text(),
                        s.port_num(),
                        s.container_engine.clone(),
                        "start",
                        name.clone(),
                    );
                }
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.use_container = true;
                    s.container = Some(name);
                    if !s.pending {
                        s.connect_remote();
                    }
                }
                save_sessions(&m);
            }
            Msg::CreateContainer => {
                m.dropdown_open = None;
                if !podman_disponible() {
                    if let Some(s) = m.sessions.get_mut(m.active_session) {
                        s.conn = ConnState::Disconnected;
                        let slot = Slot::Session(m.active_session, Which::Shell);
                        m = apply_module_msg(
                            m,
                            slot,
                            ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(
                                "✘ podman no encontrado en PATH — instalá podman o desactivá 'Aislar en contenedor'".into(),
                            )),
                        );
                    }
                    return m;
                }
                let (distro, n, mount) = m
                    .sessions
                    .get(m.active_session)
                    .map(|s| (s.distro, s.number.unwrap_or(0), s.mount.text()))
                    .unwrap_or((Distro::Ubuntu, 0, String::new()));
                let name = format!("shuma-{}-{n}", distro.label().to_lowercase());
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container = Some(name.clone());
                    s.use_container = true;
                    s.conn = ConnState::Pending;
                    s.apply_isolation();
                }
                let mount_opt = if mount.trim().is_empty() { None } else { Some(mount) };
                spawn_create_container(handle, distro.image(), name, mount_opt);
                save_sessions(&m);
            }
            Msg::ToggleUseContainer => {
                let mut activado = false;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.use_container = !s.use_container;
                    if s.use_container {
                        activado = true;
                        if let Some(pref) = engine_preferido() {
                            if !binary_disponible(&s.container_engine) {
                                s.container_engine = pref.to_string();
                            }
                        }
                    }
                    if !s.pending {
                        if !s.use_container {
                            s.container = None;
                            s.apply_isolation();
                        } else if s.container.is_some() {
                            s.apply_isolation();
                        }
                    }
                }
                if activado {
                    spawn_list_containers(handle);
                }
                save_sessions(&m);
            }
            Msg::SetEngine(name) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if binary_disponible(&name)
                        || name == "unshare"
                        || name == "bwrap"
                        || name == "podman"
                    {
                        s.container_engine = name;
                    }
                }
            }
            Msg::PickRootfs(distro) => {
                m.dropdown_open = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.use_container = true;
                    s.distro = distro;
                    if !binary_disponible(&s.container_engine) {
                        if let Some(pref) = engine_preferido() {
                            s.container_engine = pref.to_string();
                        }
                    }
                    let path = rootfs_path_for(distro)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    s.container = Some(path);
                    s.apply_isolation();
                    s.conn = ConnState::Connected;
                    if s.pending {
                        s.pending = false;
                        s.pending_focus = None;
                        m.session_panel_open = true;
                    }
                }
                save_sessions(&m);
            }
            Msg::EnsureContainer(name) => {
                let engine = m
                    .sessions
                    .iter()
                    .find(|s| s.container.as_deref() == Some(name.as_str()))
                    .map(|s| (s.container_engine.clone(), s.distro))
                    .unwrap_or_else(|| ("podman".into(), Distro::Ubuntu));
                match engine.0.as_str() {
                    "unshare" | "bwrap" => {
                        if rootfs_listo(engine.1) {
                            handle.dispatch(Msg::ContainerCreated(name));
                        } else {
                            spawn_pull_rootfs_lxc(handle, engine.1, None);
                        }
                    }
                    _ => spawn_ensure_container(handle, name),
                }
            }
            Msg::OpenContainersWindow => {
                m.containers_modal_open = true;
                m.container_draft = None;
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    spawn_list_remote_containers(handle, host, user, port, engine);
                } else {
                    spawn_list_containers_full(handle);
                }
            }
            Msg::Noop => {}
            Msg::CloseContainersModal => {
                m.containers_modal_open = false;
                m.container_draft = None;
            }
            Msg::ContainersFullLoaded(v) => {
                m.containers_full = v;
            }
            Msg::RefreshContainersFull => spawn_list_containers_full(handle),
            Msg::StartContainer(name) => spawn_container_action(handle, "start", name),
            Msg::StopContainer(name) => spawn_container_action(handle, "stop", name),
            Msg::RemoveContainer(name) => spawn_container_action(handle, "rm", name),
            Msg::RemoveRootfs(name) => spawn_remove_rootfs(handle, name),
            Msg::RefreshRemoteContainers => {
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    spawn_list_remote_containers(handle, host, user, port, engine);
                }
            }
            Msg::SetRemoteNewDistro(d) => m.remote_new_distro = d,
            Msg::CreateRemoteContainer => {
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    let distro = m.remote_new_distro;
                    let n = m.active().and_then(|s| s.number).unwrap_or(0);
                    let name = format!("shuma-{}-{n}", distro.label().to_lowercase());
                    spawn_create_remote_container(handle, host, user, port, engine, distro.image(), name);
                }
            }
            Msg::RemoteStart(name) => {
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    spawn_remote_engine_action(handle, host, user, port, engine, "start", name);
                }
            }
            Msg::RemoteStop(name) => {
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    spawn_remote_engine_action(handle, host, user, port, engine, "stop", name);
                }
            }
            Msg::RemoteRemove(name) => {
                if let Some((host, user, port, engine)) = m.active_remote_target() {
                    spawn_remote_engine_action(handle, host, user, port, engine, "rm", name);
                }
            }
            Msg::OpenHostsWindow => {
                m.hosts_modal_open = true;
                m.host_draft = None;
            }
            Msg::CloseHostsModal => {
                m.hosts_modal_open = false;
                m.host_draft = None;
            }
            Msg::HostDraftStart => {
                m.host_draft = Some(HostDraft::new());
            }
            Msg::HostEdit(idx) => {
                if let Some(h) = m.hosts.get(idx).cloned() {
                    m.host_draft = Some(HostDraft::from_host(&h));
                }
            }
            Msg::HostDraftCancel => {
                m.host_draft = None;
            }
            Msg::HostDraftSave => {
                if let Some(draft) = m.host_draft.clone() {
                    if let Some(h) = draft.to_host() {
                        if let Some(old) = &draft.editing {
                            if old != &h.name {
                                m.hosts.retain(|x| &x.name != old);
                            }
                        }
                        if let Some(idx) = m.hosts.iter().position(|x| x.name == h.name) {
                            m.hosts[idx] = h.clone();
                        } else {
                            m.hosts.push(h.clone());
                        }
                        hosts::save_hosts(&m.hosts);
                        m.host_draft = Some(HostDraft::from_host(&h));
                    }
                }
            }
            Msg::HostDraftFocus(f) => {
                if let Some(d) = m.host_draft.as_mut() {
                    d.focused = Some(f);
                }
            }
            Msg::HostDraftKey(e) => {
                if let Some(d) = m.host_draft.as_mut() {
                    let Some(f) = d.focused else { return m };
                    match &e.key {
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                            d.focused = None;
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                            handle.dispatch(Msg::HostDraftSave);
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Tab) => {
                            let next = match f {
                                HostDraftField::Name => HostDraftField::Host,
                                HostDraftField::Host => HostDraftField::User,
                                HostDraftField::User => HostDraftField::Port,
                                HostDraftField::Port => {
                                    if d.use_password { HostDraftField::Name } else { HostDraftField::Pem }
                                }
                                HostDraftField::Pem => HostDraftField::Name,
                            };
                            d.focused = Some(next);
                        }
                        _ => {
                            let target = match f {
                                HostDraftField::Name => &mut d.name,
                                HostDraftField::Host => &mut d.host,
                                HostDraftField::User => &mut d.user,
                                HostDraftField::Port => &mut d.port,
                                HostDraftField::Pem => &mut d.pem_path,
                            };
                            let _ = target.apply_key(&e);
                        }
                    }
                }
            }
            Msg::HostDraftToggleAuth => {
                if let Some(d) = m.host_draft.as_mut() {
                    d.use_password = !d.use_password;
                }
            }
            Msg::HostDelete(idx) => {
                if idx < m.hosts.len() {
                    m.hosts.remove(idx);
                    hosts::save_hosts(&m.hosts);
                }
            }
            Msg::OpenLayoutsModal => {
                m.layouts_modal_open = true;
                m.layout_name_focused = true;
                m.menu_open = None;
            }
            Msg::CloseLayoutsModal => {
                m.layouts_modal_open = false;
                m.layout_name_focused = false;
            }
            Msg::LayoutNameFocus => {
                m.layout_name_focused = true;
            }
            Msg::LayoutNameKey(e) => match &e.key {
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                    m.layout_name_focused = false;
                }
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                    handle.dispatch(Msg::SaveLayout);
                }
                _ => {
                    let _ = m.layout_name.apply_key(&e);
                }
            },
            Msg::SaveLayout => {
                let name = m.layout_name.text().trim().to_string();
                if !name.is_empty() {
                    let snap = snapshot_workspace(&m, name.clone());
                    if let Some(i) = m.layouts.iter().position(|l| l.name == name) {
                        m.layouts[i] = snap;
                    } else {
                        m.layouts.push(snap);
                    }
                    save_layouts(&m.layouts);
                    m.layout_name.set_text("");
                    m.layout_name_focused = false;
                }
            }
            Msg::RestoreLayout(idx) => {
                if let Some(snap) = m.layouts.get(idx).cloned() {
                    let mut sessions = vec![Session::draft()];
                    for c in snap.sessions {
                        sessions.push(Session::from_config(c));
                    }
                    for s in &sessions {
                        if s.use_container {
                            if let Some(name) = s.container.clone() {
                                handle.dispatch(Msg::EnsureContainer(name));
                            }
                        }
                    }
                    m.sessions = sessions;
                    m.active_tool = snap.chrome.active_tool;
                    m.session_panel_open = snap.chrome.session_panel_open;
                    m.session_w = snap.chrome.session_w;
                    m.monitors_width = snap.chrome.monitors_width;
                    m.active_session = snap
                        .chrome
                        .active_session
                        .min(m.sessions.len().saturating_sub(1));
                    m.layouts_modal_open = false;
                    save_sessions(&m);
                    save_chrome(&m);
                }
            }
            Msg::DeleteLayout(idx) => {
                if idx < m.layouts.len() {
                    m.layouts.remove(idx);
                    save_layouts(&m.layouts);
                }
            }
            Msg::ContainerDraftNew => {
                let host = m
                    .sessions
                    .get(m.active_session)
                    .map(|s| s.host_key())
                    .unwrap_or_else(host_local);
                m.container_draft = Some(ContainerDraft::new(host));
            }
            Msg::ContainerDraftCancel => {
                m.container_draft = None;
            }
            Msg::ContainerEdit(idx) => {
                if let Some(info) = m.containers_full.get(idx) {
                    if info.rootfs {
                        let name = info.name.clone();
                        let host = m
                            .sessions
                            .get(m.active_session)
                            .map(|s| s.host_key())
                            .unwrap_or_else(host_local);
                        let cfg = m
                            .container_cfgs
                            .iter()
                            .find(|c| c.name == name)
                            .cloned()
                            .unwrap_or_else(|| ContainerCfg {
                                name: name.clone(),
                                host,
                                engine: engine_preferido().unwrap_or("unshare").to_string(),
                                distro: distro_from_name(&name).unwrap_or(Distro::Ubuntu),
                                mounts: Vec::new(),
                            });
                        m.container_draft = Some(ContainerDraft::from_cfg(&cfg));
                    }
                }
            }
            Msg::ContainerDraftSetEngine(name) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if d.editing.is_none() {
                        d.engine = name;
                    }
                }
            }
            Msg::ContainerDraftSetDistro(distro) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if d.editing.is_none() {
                        d.distro = distro;
                    }
                }
            }
            Msg::ContainerDraftAddMount => {
                if let Some(d) = m.container_draft.as_mut() {
                    d.mounts.push(MountDraft::new());
                    d.focus = Some((d.mounts.len() - 1, MountCol::Host));
                }
            }
            Msg::ContainerDraftRemoveMount(i) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if i < d.mounts.len() {
                        d.mounts.remove(i);
                        d.focus = None;
                    }
                }
            }
            Msg::ContainerDraftToggleMountRo(i) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if let Some(md) = d.mounts.get_mut(i) {
                        md.readonly = !md.readonly;
                    }
                }
            }
            Msg::ContainerDraftFocusMount(i, col) => {
                if let Some(d) = m.container_draft.as_mut() {
                    if i < d.mounts.len() {
                        d.focus = Some((i, col));
                    }
                }
            }
            Msg::ContainerDraftSave => {
                if let Some(d) = m.container_draft.clone() {
                    let nuevo = d.editing.is_none();
                    let name = d.editing.clone().unwrap_or_else(|| {
                        if matches!(d.engine.as_str(), "unshare" | "bwrap") {
                            d.distro.label().to_lowercase()
                        } else {
                            (1..1000)
                                .map(|n| format!("shuma-{}-{n}", d.distro.label().to_lowercase()))
                                .find(|cand| !m.container_cfgs.iter().any(|c| &c.name == cand))
                                .unwrap_or_else(|| format!("shuma-{}", d.distro.label().to_lowercase()))
                        }
                    });
                    let cfg = d.to_cfg(name.clone());
                    if let Some(slot) = m.container_cfgs.iter_mut().find(|c| c.name == name) {
                        *slot = cfg.clone();
                    } else {
                        m.container_cfgs.push(cfg.clone());
                    }
                    save_container_cfgs(&m.container_cfgs);
                    if nuevo {
                        match d.engine.as_str() {
                            "unshare" | "bwrap" => {
                                if !rootfs_listo(d.distro) {
                                    spawn_pull_rootfs_lxc(handle, d.distro, None);
                                }
                            }
                            _ => {
                                spawn_create_container(handle, d.distro.image(), name.clone(), None);
                            }
                        }
                    }
                    m.container_draft = Some(ContainerDraft::from_cfg(&cfg));
                    spawn_list_containers_full(handle);
                }
            }
            Msg::ContainerDraftKey(e) => {
                if let Some(d) = m.container_draft.as_mut() {
                    let Some((idx, col)) = d.focus else { return m };
                    match &e.key {
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                            d.focus = None;
                        }
                        llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                            handle.dispatch(Msg::ContainerDraftSave);
                        }
                        _ => {
                            if let Some(md) = d.mounts.get_mut(idx) {
                                let input = match col {
                                    MountCol::Host => &mut md.host,
                                    MountCol::Target => &mut md.target,
                                };
                                let _ = input.apply_key(&e);
                            }
                        }
                    }
                }
            }
            Msg::PickHost(choice) => {
                m.dropdown_open = None;
                let host = choice.and_then(|i| m.hosts.get(i).cloned());
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.container = None;
                    match host {
                        None => {
                            s.isolation = Isolation::Local;
                            s.host_label = None;
                            if !s.pending {
                                s.apply_isolation();
                            } else {
                                s.conn = ConnState::Connected;
                            }
                        }
                        Some(h) => {
                            s.isolation = Isolation::Remote;
                            s.host_label = Some(h.name.clone());
                            s.host.set_text(h.host);
                            s.user.set_text(h.user);
                            s.port.set_text(h.port.to_string());
                            if !s.pending {
                                s.connect_remote();
                            } else {
                                s.conn = ConnState::Pending;
                            }
                        }
                    }
                }
                save_sessions(&m);
            }
            Msg::HostApply(idx) => {
                m.dropdown_open = None;
                let h = match m.hosts.get(idx).cloned() {
                    Some(h) => h,
                    None => return m,
                };
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.isolation = Isolation::Remote;
                    s.host_label = Some(h.name.clone());
                    s.host.set_text(h.host);
                    s.user.set_text(h.user);
                    s.port.set_text(h.port.to_string());
                    if !s.pending {
                        s.connect_remote();
                    }
                }
                save_sessions(&m);
            }
            Msg::ContainerCreated(name) => {
                let idx = m
                    .sessions
                    .iter()
                    .position(|s| s.container.as_deref() == Some(name.as_str()));
                if let Some(i) = idx {
                    if let Some(s) = m.sessions.get_mut(i) {
                        s.conn = ConnState::Connected;
                        if s.use_container && !s.pending {
                            s.apply_isolation();
                        }
                    }
                }
                spawn_list_containers(handle);
                save_sessions(&m);
            }
            Msg::ContainerFailed { name, reason } => {
                let idx = m
                    .sessions
                    .iter()
                    .position(|s| s.container.as_deref() == Some(name.as_str()));
                if let Some(i) = idx {
                    let engine = m
                        .sessions
                        .get(i)
                        .map(|s| s.container_engine.clone())
                        .unwrap_or_default();
                    let accion = match engine.as_str() {
                        "unshare" | "bwrap" => "la descarga del rootfs",
                        other if !other.is_empty() => "el arranque del contenedor",
                        _ => "el contenedor",
                    };
                    if let Some(s) = m.sessions.get_mut(i) {
                        s.conn = ConnState::Disconnected;
                        s.container = None;
                        s.use_container = false;
                        s.apply_isolation();
                    }
                    let slot = Slot::Session(i, Which::Shell);
                    m = apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(format!(
                            "✘ {accion} ({engine}) falló: {reason} — caí a shell local."
                        ))),
                    );
                }
                save_sessions(&m);
            }
            Msg::CloseSession(idx) => {
                if idx > 0 && idx < m.sessions.len() {
                    let s = m.sessions.remove(idx);
                    if s.persist {
                        persist::remove_session_output(&s.name);
                    }
                    m.active_session = m.active_session.min(m.sessions.len() - 1);
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::ToggleSessionPersist(idx) => {
                if let Some(s) = m.sessions.get_mut(idx) {
                    s.persist = !s.persist;
                    let (persist, name) = (s.persist, s.name.clone());
                    save_sessions(&m);
                    if persist {
                        // Snapshot inmediato: el flag queda respaldado ya.
                        save_session_outputs(&m);
                    } else {
                        persist::remove_session_output(&name);
                    }
                }
            }
            Msg::ToggleEnvGroup(i) => {
                if let Some(g) = m.env_groups.get_mut(i) {
                    g.active = !g.active;
                    let encendido = g.active;
                    shuma_config::apply_env_group(g, encendido);
                    if !encendido {
                        // Re-aplicar los grupos que siguen activos: si una
                        // variable vivía en dos grupos, recupera el valor del
                        // que queda encendido.
                        for og in m.env_groups.iter().filter(|og| og.active) {
                            shuma_config::apply_env_group(og, true);
                        }
                    }
                    let _ = shuma_config::save_env_groups(&m.env_groups);
                    m.env_groups_mtime = persist::env_groups_mtime();
                }
            }
            Msg::OpenNewSessionForm => {
                let n = m.sessions.iter().filter(|s| s.number.is_some()).count() as u32 + 1;
                let mut s = Session::new_pending(n);
                s.pending_focus = Some(PendingField::Mount);
                m.sessions.push(s);
                m.active_session = m.sessions.len() - 1;
                m.session_panel_open = false;
                save_chrome(&m);
            }
            Msg::ConfirmNewSession => {
                enum CreatePlan {
                    Rootfs { distro: Distro, mount: Option<String> },
                    Podman { image: &'static str, name: String, mount: Option<String> },
                    PodmanEnsure { name: String },
                }
                let mut plan: Option<CreatePlan> = None;
                let mut notice: Option<String> = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if s.pending {
                        s.pending = false;
                        s.pending_focus = None;
                        if s.use_container {
                            let chosen: Option<String> = if binary_disponible(&s.container_engine) {
                                Some(s.container_engine.clone())
                            } else {
                                engine_preferido().map(|e| e.to_string())
                            };
                            match chosen.as_deref() {
                                None => {
                                    s.use_container = false;
                                    s.container = None;
                                    notice = Some(
                                        "✘ ningún engine de aislamiento está disponible (faltan `unshare`/`bwrap`/`podman`). Arrancó como shell local.".into(),
                                    );
                                }
                                Some("unshare") | Some("bwrap") => {
                                    let engine = chosen.unwrap();
                                    s.container_engine = engine.clone();
                                    let mount = s.mount.text();
                                    let mount_opt = if mount.trim().is_empty() { None } else { Some(mount) };
                                    let via_modal = s.container.is_some();
                                    if s.container.is_none() {
                                        let path = rootfs_path_for(s.distro)
                                            .map(|p| p.display().to_string())
                                            .unwrap_or_default();
                                        s.container = Some(path);
                                    }
                                    if rootfs_listo(s.distro) {
                                        s.conn = ConnState::Connected;
                                    } else {
                                        s.conn = ConnState::Pending;
                                        if !via_modal {
                                            plan = Some(CreatePlan::Rootfs {
                                                distro: s.distro,
                                                mount: mount_opt,
                                            });
                                        }
                                    }
                                }
                                Some(_) => {
                                    s.container_engine = "podman".into();
                                    s.conn = ConnState::Pending;
                                    let mount = s.mount.text();
                                    let mount_opt = if mount.trim().is_empty() { None } else { Some(mount) };
                                    match s.container.clone() {
                                        Some(name) => {
                                            plan = Some(CreatePlan::PodmanEnsure { name });
                                        }
                                        None => {
                                            let n = s.number.unwrap_or(0);
                                            let name = format!(
                                                "shuma-{}-{n}",
                                                s.distro.label().to_lowercase()
                                            );
                                            s.container = Some(name.clone());
                                            plan = Some(CreatePlan::Podman {
                                                image: s.distro.image(),
                                                name,
                                                mount: mount_opt,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        s.apply_isolation();
                        if s.use_container
                            && matches!(s.container_engine.as_str(), "unshare" | "bwrap")
                            && rootfs_listo(s.distro)
                        {
                            s.conn = ConnState::Connected;
                        }
                        if s.isolation == Isolation::Remote {
                            if !s.host.text().trim().is_empty() && !s.user.text().trim().is_empty() {
                                s.connect_remote();
                            }
                        }
                        m.session_panel_open = true;
                    }
                }
                if let Some(text) = notice {
                    let slot = Slot::Session(m.active_session, Which::Shell);
                    m = apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(text)),
                    );
                }
                match plan {
                    Some(CreatePlan::Rootfs { distro, mount }) => {
                        let slot = Slot::Session(m.active_session, Which::Shell);
                        m = apply_module_msg(
                            m,
                            slot,
                            ModuleMsg::Shell(shuma_module_shell::Msg::PushNotice(format!(
                                "⬇ descargando rootfs LXC ({}) — ~50 MB, esto tarda unos segundos…",
                                distro.label()
                            ))),
                        );
                        spawn_pull_rootfs_lxc(handle, distro, mount);
                    }
                    Some(CreatePlan::Podman { image, name, mount }) => {
                        spawn_create_container(handle, image, name, mount);
                    }
                    Some(CreatePlan::PodmanEnsure { name }) => {
                        spawn_ensure_container(handle, name);
                    }
                    None => {}
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::CancelNewSession => {
                if let Some(s) = m.sessions.get(m.active_session) {
                    if s.pending {
                        let idx = m.active_session;
                        m.sessions.remove(idx);
                        m.active_session = m.active_session.min(m.sessions.len().saturating_sub(1));
                    }
                }
                save_chrome(&m);
            }
            Msg::FocusPendingField(f) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.pending_focus = Some(f);
                }
                m.dropdown_open = None;
            }
            Msg::PendingKey(e) => {
                let Some(s) = m.sessions.get_mut(m.active_session) else {
                    return m;
                };
                let Some(f) = s.pending_focus else { return m };
                match &e.key {
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                        s.pending_focus = None;
                    }
                    llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                        handle.dispatch(Msg::ConfirmNewSession);
                    }
                    _ => match f {
                        PendingField::Mount => {
                            let _ = s.mount.apply_key(&e);
                        }
                    },
                }
            }
            Msg::ReorderSession(from, to) => {
                let len = m.sessions.len();
                if from > 0 && from < len && to > 0 && to < len && from != to {
                    let s = m.sessions.remove(from);
                    m.sessions.insert(to, s);
                    m.active_session = to;
                }
                save_sessions(&m);
                save_chrome(&m);
            }
            Msg::SetSessionWidth(dx) => {
                m.session_w = (m.session_w + dx).clamp(180.0, 480.0);
                save_chrome(&m);
            }
            Msg::SetToolWidth(dx) => {
                m.monitors_width = (m.monitors_width - dx).clamp(180.0, 480.0);
                save_chrome(&m);
            }
            Msg::Module(slot, mmsg) => {
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
                m.ctx_menu = None;
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
                if let Some(t) = Tool::ALL.get(id as usize) {
                    m.active_tool = if m.active_tool == Some(*t) { None } else { Some(*t) };
                    save_chrome(&m);
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
        if model.hosts_modal_open {
            return Some(view::hosts_modal(model, &model.theme));
        }
        if model.containers_modal_open {
            return Some(view::containers_modal(model, &model.theme));
        }
        if model.layouts_modal_open {
            return Some(view::layouts_modal(model, &model.theme));
        }
        view::dropdown_overlay(model).or_else(|| menu::overlay(model))
    }
}
