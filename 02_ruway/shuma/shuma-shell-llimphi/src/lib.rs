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

pub mod config;
pub mod containers;
pub mod env;
pub mod hosts;
pub mod menu;
pub mod perfiles;
pub mod persist;
pub mod types;
pub mod update;
pub mod view;
pub mod workspace;

// Superficie pública para hosts (pata) y el bin: el `Model`, el `Msg`, el `App`
// (`Shell`) y `run()` viven en este crate-lib — la lógica de dominio ya no está
// amarrada al binario (Regla 2: frontend sobre core agnóstico). pata podrá
// embeber `Model` y rutearle `Msg`.
pub use types::{Model, Msg};

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Position, Rect,
};
use llimphi_ui::{
    App, DragPhase, Handle, ImageFit, KeyEvent, KeyState, Modifiers, View, WheelDelta,
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
use workspace::*;

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

/// Arranca shuma standalone (la app de ventana). El bin sólo llama acá; toda la
/// lógica vive en este crate-lib para que también la pueda hospedar pata.
pub fn run() {
    rimay_localize::init();
    wire_askpass();
    llimphi_ui::run::<Shell>();
}

/// Cablea el askpass para sudo + ssh (compartido por ventana y dock).
fn wire_askpass() {
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
}

/// Arranca shuma **dockeada**: una barra wlr-layer-shell anclada a un borde, en
/// vez de una ventana. Mismo `Shell` App (la lógica no cambia); el modo lo lee
/// `init` del env `SHUMA_DOCK` y `view` pinta compacto. Borde por `SHUMA_DOCK_EDGE`
/// (top/bottom/left/right, default bottom). Cae con aviso si el compositor no
/// expone wlr-layer-shell.
pub fn run_dock() {
    rimay_localize::init();
    wire_askpass();
    std::env::set_var("SHUMA_DOCK", "1");
    let edge = match std::env::var("SHUMA_DOCK_EDGE").as_deref() {
        Ok("top") => llimphi_layer::Edge::Top,
        Ok("left") => llimphi_layer::Edge::Left,
        Ok("right") => llimphi_layer::Edge::Right,
        _ => llimphi_layer::Edge::Bottom,
    };
    if let Err(e) = llimphi_layer::run::<Shell>(llimphi_layer::LayerConfig {
        edge,
        thickness: 40,
        layer: llimphi_layer::LayerKind::Top,
        exclusive: true,
        keyboard: llimphi_layer::Keyboard::OnDemand,
        namespace: "shuma".to_string(),
    }) {
        eprintln!("shuma · modo dock no disponible: {e}");
    }
}

/// Re-lanza el mismo binario en el modo opuesto (ventana ↔ barra dockeada),
/// heredando el cwd. Usado por el botón «Endockar / Modo ventana» del menú y por
/// el repliegue al perder foco. No migra la sesión viva (historial/PTY): la
/// nueva instancia arranca limpia — migrarla exigiría IPC entre procesos.
pub(crate) fn respawn_mode(to_dock: bool) {
    if let Ok(exe) = std::env::current_exe() {
        let mut c = std::process::Command::new(exe);
        if to_dock {
            c.arg("--dock");
        }
        let _ = c.spawn();
    }
}

/// Construye el `Model` de shuma **sin efectos del host** (sin ticks, watcher de
/// config, cliente de rail, ni disparo de contenedores). Pieza hosteable
/// (Regla 2): el bin standalone y pata construyen el mismo Model y cada host
/// engancha sus efectos vía [`spawn_host_effects`]. Los campos de efecto
/// (`_wawa_watcher`/`_host`) quedan en `None` hasta que el host los provea.
pub fn new_model() -> Model {
    let wawa = wawa_config::WawaConfig::load();
    let theme = wawa_config_llimphi::theme_from_wawa(&wawa, &Theme::dark());
    let _ = rimay_localize::set_locale(&wawa.lang);

    // Perfiles. El de sesión (tipo Firefox) decide el directorio de datos: hay
    // que fijarlo ANTES de leer sesiones/chrome/layouts, que ahora resuelven su
    // ruta vía `perfiles::sessions::active_data_dir`.
    let session_profiles = perfiles::sessions::SessionProfiles::load_or_init(
        &perfiles::sessions::SessionProfiles::default_path().unwrap_or_default(),
    );
    perfiles::sessions::set_active(session_profiles.active());
    let shortcuts = perfiles::shortcuts::ShortcutProfiles::load_or_init(
        &perfiles::shortcuts::ShortcutProfiles::default_path().unwrap_or_default(),
    );
    let appearance = perfiles::appearance::AppearanceProfiles::load_or_init(
        &perfiles::appearance::AppearanceProfiles::default_path().unwrap_or_default(),
    );

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
                if let ModuleState::Shell(st) = &mut sess.shell_mut().state {
                    st.restore_output(snap);
                }
            }
        }
        sessions.push(sess);
    }

    // Handoff de desacople (botón Undock de pata): si `SHUMA_HANDOFF` apunta a
    // un snapshot de salida, rehidratarlo en la sesión draft y abrir directo en
    // ella. Es la otra mitad del "mover de verdad" — pata serializa la sesión
    // embebida, nos la pasa, y la cierra de su lado, así no queda duplicada.
    // El cwd ya llega por `SHUMA_CWD`/`cd` y el historial por la history
    // persistente compartida; esto suma el scrollback visible. Consumimos el
    // archivo para que no se re-aplique si reabrís shuma.
    let mut handoff_active: Option<usize> = None;
    if let Ok(path) = std::env::var("SHUMA_HANDOFF") {
        if let Some(snap) = persist::load_output_snapshot_file(&path) {
            if let ModuleState::Shell(st) = &mut sessions[0].shell_mut().state {
                st.restore_output(snap);
                handoff_active = Some(0);
            }
        }
        let _ = std::fs::remove_file(&path);
    }

    // Grupos de environment: cargar env.json (garantizando el grupo «general»,
    // destino del builtin `:env`) y aplicar los activos al proceso — los shells
    // hijos los heredan.
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

    let chrome = load_chrome();
    // Si vino un handoff de desacople, abrimos en esa sesión; si no, la última
    // activa persistida.
    let active_session = handoff_active
        .unwrap_or_else(|| chrome.active_session.min(sessions.len().saturating_sub(1)));

    let mut model = Model {
        theme,
        dock_mode: false,
        collapse_on_blur: false,
        shortcuts,
        appearance,
        session_profiles,
        pending_prefix: false,
        perfiles_modal_open: false,
        perfiles_tab: ProfKind::Shortcuts,
        prof_name: TextInputState::new(),
        prof_name_focused: false,
        wallpaper_img: None,
        wallpaper_path: None,
        wp_path: TextInputState::new(),
        wp_path_focused: false,
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
        _wawa_watcher: None,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        ctx_menu: None,
        tab_ctx: None,
        env_groups,
        env_groups_mtime,
        tick_count: 0,
        hosted_bar: false,
        _host: None,
    };
    // Aplicar la apariencia efectiva (global o de la sesión activa) sobre el
    // tema base: si la activa es «Sistema» queda el tema de wawa ya calculado.
    perfiles::apply_active_appearance(&mut model);
    model
}

/// Marca el Model como **hospedado en una barra externa** (pata): el input de la
/// sesión activa lo pinta el host con [`active_input_view`], así que el canvas
/// omite su input para no duplicarlo. Llamar tras [`new_model`] en el host.
pub fn set_hosted_in_bar(model: &mut Model, on: bool) {
    model.hosted_bar = on;
}

/// Engancha los efectos que dependen del host (event loop): ticks periódicos,
/// watcher de `WawaConfig`, cliente del rail de pata, y dispara la verificación
/// de contenedores de las sesiones. El bin standalone lo llama en `App::init`;
/// un host como pata lo llama con un `Handle` lifteado ([`Handle::lift`]) para
/// que los ticks/efectos de shuma vuelvan a su loop como `pata::Msg`.
pub fn spawn_host_effects(model: &mut Model, handle: &Handle<Msg>) {
    handle.spawn_periodic(TICK, || Msg::Tick);
    handle.spawn_periodic(SHELL_TICK, || Msg::ShellTick);
    model._wawa_watcher = {
        let handle = handle.clone();
        wawa_config::ConfigWatcher::spawn(move |cfg| {
            handle.dispatch(Msg::WawaConfigChanged(Box::new(cfg)));
        })
        .ok()
    };
    for s in &model.sessions {
        if s.use_container {
            if let Some(name) = s.container.clone() {
                handle.dispatch(Msg::EnsureContainer(name));
            }
        }
    }
    model._host = shuma_host(handle);
}

/// Conmuta el **perfil de sesión** (contexto tipo Firefox): guarda el estado del
/// perfil actual, cambia el directorio de datos activo y **recarga** sesiones,
/// chrome, disposiciones y containers desde el nuevo directorio. Aislamiento
/// total entre contextos sin duplicar la lógica de persistencia.
pub(crate) fn switch_session_profile(mut m: Model, name: &str) -> Model {
    if m.session_profiles.active() == name {
        return m; // ya estamos ahí
    }
    // Guardar el estado del perfil actual antes de irnos.
    save_sessions(&m);
    save_chrome(&m);
    save_session_outputs(&m);
    // Conmutar (sólo perfiles existentes).
    if m.session_profiles.set_active(name).is_err() {
        return m;
    }
    perfiles::sessions::set_active(name);
    if let Some(p) = perfiles::sessions::SessionProfiles::default_path() {
        let _ = m.session_profiles.save(&p);
    }
    // Recargar desde el nuevo directorio.
    let mut sessions = vec![Session::draft()];
    for c in load_sessions() {
        let mut sess = Session::from_config(c);
        if sess.persist {
            if let Some(snap) = persist::load_session_output(&sess.name) {
                if let ModuleState::Shell(st) = &mut sess.shell_mut().state {
                    st.restore_output(snap);
                }
            }
        }
        sessions.push(sess);
    }
    m.sessions = sessions;
    let chrome = load_chrome();
    m.active_session = chrome.active_session.min(m.sessions.len().saturating_sub(1));
    m.active_tool = chrome.active_tool;
    m.session_panel_open = chrome.session_panel_open;
    m.session_w = chrome.session_w;
    m.monitors_width = chrome.monitors_width;
    m.layouts = load_layouts();
    m.container_cfgs = load_container_cfgs();
    m.pending_prefix = false;
    perfiles::apply_active_appearance(&mut m);
    m
}

/// Persiste a disco la biblioteca de perfiles del tipo dado.
pub(crate) fn save_profiles(m: &Model, kind: ProfKind) {
    match kind {
        ProfKind::Shortcuts => {
            if let Some(p) = perfiles::shortcuts::ShortcutProfiles::default_path() {
                let _ = m.shortcuts.save(&p);
            }
        }
        ProfKind::Appearance => {
            if let Some(p) = perfiles::appearance::AppearanceProfiles::default_path() {
                let _ = m.appearance.save(&p);
            }
        }
        ProfKind::Sessions => {
            if let Some(p) = perfiles::sessions::SessionProfiles::default_path() {
                let _ = m.session_profiles.save(&p);
            }
        }
    }
}

/// Renombra un **perfil de sesión**: mueve su directorio de datos en disco y
/// actualiza el índice. Si es el activo, reapunta el directorio global.
pub(crate) fn rename_session_profile(mut m: Model, from: &str, to: &str) -> Model {
    // Mover el directorio en disco antes de tocar el índice (si existe).
    if let (Some(old), Some(new)) = (
        perfiles::sessions::data_dir_for(from),
        perfiles::sessions::data_dir_for(to),
    ) {
        if old.exists() && !new.exists() {
            if let Some(parent) = new.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::rename(&old, &new);
        }
    }
    let was_active = m.session_profiles.active() == from;
    if m.session_profiles.rename(from, to).is_ok() {
        if was_active {
            perfiles::sessions::set_active(to);
        }
        if let Some(p) = perfiles::sessions::SessionProfiles::default_path() {
            let _ = m.session_profiles.save(&p);
        }
    }
    m
}

// ─── App impl ───────────────────────────────────────────────────────

pub struct Shell;

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
        let mut model = new_model();
        model.dock_mode = std::env::var_os("SHUMA_DOCK").is_some();
        model.collapse_on_blur = std::env::var_os("SHUMA_BAR_ON_BLUR").is_some();
        spawn_host_effects(&mut model, handle);
        model
    }

    fn on_resize(_model: &Self::Model, width: u32, height: u32) -> Option<Self::Msg> {
        Some(Msg::Resized(width as f32, height as f32))
    }

    fn on_window_focus(model: &Self::Model, focused: bool) -> Option<Self::Msg> {
        // En modo ventana, al perder el foco y si está configurado, repliega a la
        // barra dockeada (re-lanza en modo dock y cierra esta ventana). Opt-in.
        if !focused && !model.dock_mode && model.collapse_on_blur {
            return Some(Msg::MenuCommand("window.toggle-dock".to_string()));
        }
        None
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
        if model.perfiles_modal_open {
            if let llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) = &e.key {
                return Some(Msg::ClosePerfilesModal);
            }
            if model.wp_path_focused {
                return Some(Msg::WpPathKey(e.clone()));
            }
            return Some(Msg::ProfNameKey(e.clone()));
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
        // Atajos del workspace según el perfil de atajos activo (shuma/hyprland/
        // tmux/zellij/vim o uno propio): tabs, tiling, flotantes.
        if let Some(msg) = perfiles::shortcuts::resolve_key(model, e) {
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
                // M5 — polling de la flota a cadencia más lenta (~30 s): un
                // fetch SSH por host es caro, así que se espacia más que el
                // runtime local. Sólo corre si la flota ya fue activada.
                if m.tick_count % 30 == 0 {
                    update::poll_matilda_fleet(&m, handle);
                    // M4 — y el runtime del Source montado si es remoto.
                    update::poll_matilda_remote_runtime(&m, handle);
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
                // El tema del sistema sólo pisa el de shuma si la apariencia
                // efectiva es «Sistema» (sigue a wawa). Un perfil de apariencia
                // fijo (global o de sesión) manda sobre wawa.
                if perfiles::follows_system(&m) {
                    m.theme = wawa_config_llimphi::theme_from_wawa(&cfg, &m.theme);
                }
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
                    // La sesión activa puede fijar su propia apariencia (la
                    // "ventana"): re-aplicar al cambiar de sesión.
                    perfiles::apply_active_appearance(&mut m);
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
                // M5 — acciones sobre recursos de un host de la flota: necesitan
                // SSH en un thread. El módulo sólo dejó la intención en el log;
                // el chasis corre el `docker`/`systemctl` remoto y, si fue
                // mutante, re-observa el host para refrescar su semáforo.
                if let ModuleMsg::Matilda(mat) = &mmsg {
                    use shuma_module_matilda::Msg as MMsg;
                    // M2 — live-tail (`docker logs -f`): el módulo ya preparó el
                    // `log_stream` (buffer + bandera stop) al aplicar el Msg;
                    // acá leemos esos inputs y lanzamos un thread lector que
                    // dispatcha `LogStreamLine` por línea y `LogStreamEnded` al
                    // terminar. Un thread crudo (no `handle.spawn`) porque emite
                    // N mensajes a lo largo del tiempo, no uno solo.
                    if matches!(mat, MMsg::StartLogStream(_)) {
                        m = apply_module_msg(m, slot.clone(), mmsg);
                        if let Some((source, name, stop)) =
                            matilda_log_stream_inputs(&slot, &m)
                        {
                            let slot_back = slot.clone();
                            let h = handle.clone();
                            std::thread::spawn(move || {
                                let _ = shuma_module_matilda::stream_logs_blocking(
                                    &source,
                                    &name,
                                    200,
                                    &stop,
                                    |line| {
                                        h.dispatch(Msg::Module(
                                            slot_back.clone(),
                                            ModuleMsg::Matilda(MMsg::LogStreamLine(line)),
                                        ));
                                    },
                                );
                                h.dispatch(Msg::Module(
                                    slot_back.clone(),
                                    ModuleMsg::Matilda(MMsg::LogStreamEnded),
                                ));
                            });
                        }
                        return m;
                    }
                    if let MMsg::FleetContainerAction { host, name, action } = mat {
                        if let Some(h) = matilda_host_by_name(&slot, &m, host) {
                            let (name, action) = (name.clone(), *action);
                            let slot_back = slot.clone();
                            handle.spawn(move || {
                                let (ok, lines) = shuma_module_matilda::fleet_container_action_blocking(
                                    &h, &name, action,
                                );
                                let runtime = if ok && action.is_mutating() {
                                    shuma_module_matilda::host_runtime_remote_blocking(&h).ok()
                                } else {
                                    None
                                };
                                Msg::Module(
                                    slot_back,
                                    ModuleMsg::Matilda(MMsg::FleetActionDone {
                                        host: h.name.clone(),
                                        lines,
                                        runtime,
                                    }),
                                )
                            });
                        }
                        return apply_module_msg(m, slot, mmsg);
                    }
                    if let MMsg::FleetServiceAction { host, name, action } = mat {
                        if let Some(h) = matilda_host_by_name(&slot, &m, host) {
                            let (name, action) = (name.clone(), *action);
                            let slot_back = slot.clone();
                            handle.spawn(move || {
                                let (ok, lines) = shuma_module_matilda::fleet_service_action_blocking(
                                    &h, &name, action,
                                );
                                let runtime = if ok && action.is_mutating() {
                                    shuma_module_matilda::host_runtime_remote_blocking(&h).ok()
                                } else {
                                    None
                                };
                                Msg::Module(
                                    slot_back,
                                    ModuleMsg::Matilda(MMsg::FleetActionDone {
                                        host: h.name.clone(),
                                        lines,
                                        runtime,
                                    }),
                                )
                            });
                        }
                        return apply_module_msg(m, slot, mmsg);
                    }
                    // Acciones sobre el Source montado cuando es remoto: el
                    // módulo ya logueó "delegado al chasis"; acá corremos el
                    // `docker`/`systemctl` por SSH y volcamos la salida.
                    if let MMsg::ContainerActionMsg { name, action } = mat {
                        if let Some((source, _)) = remote_matilda_inputs(&slot, &m) {
                            let (name, action) = (name.clone(), *action);
                            let slot_back = slot.clone();
                            handle.spawn(move || {
                                let lines = shuma_module_matilda::container_action_remote_blocking(
                                    &source, &name, action,
                                )
                                .unwrap_or_else(|e| vec![format!("✘ {} {name}: {e}", action.label())]);
                                Msg::Module(slot_back, ModuleMsg::Matilda(MMsg::LogLines(lines)))
                            });
                        }
                        return apply_module_msg(m, slot, mmsg);
                    }
                    if let MMsg::ServiceActionMsg { name, action } = mat {
                        if let Some((source, _)) = remote_matilda_inputs(&slot, &m) {
                            let (name, action) = (name.clone(), *action);
                            let slot_back = slot.clone();
                            handle.spawn(move || {
                                let cmd = action.command(&name);
                                let lines = shuma_module_matilda::service_action_remote_blocking(
                                    &source, &cmd, action.label(), &name,
                                )
                                .unwrap_or_else(|e| vec![format!("✘ {} {name}: {e}", action.label())]);
                                Msg::Module(slot_back, ModuleMsg::Matilda(MMsg::LogLines(lines)))
                            });
                        }
                        return apply_module_msg(m, slot, mmsg);
                    }
                }
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
                m.tab_ctx = None;
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

            // ─── Workspace tipo zellij ──────────────────────────────
            Msg::PaneSplit(axis) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if !s.pending {
                        let inst = Instance::shell(s.name.clone(), s.source.clone());
                        s.workspace.split(axis, inst);
                    }
                }
            }
            Msg::PaneFocus(id) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.focus(id);
                }
            }
            Msg::PaneClose => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    let _ = s.workspace.close_focused();
                }
            }
            Msg::PaneCycle(fwd) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.cycle_focus(fwd);
                }
            }
            Msg::PaneResize(path, delta) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.resize(&path, delta);
                }
            }
            Msg::TabNew => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if !s.pending {
                        let inst = Instance::shell(s.name.clone(), s.source.clone());
                        s.workspace.new_tab(inst);
                    }
                }
            }
            Msg::TabSwitch(i) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.switch_tab(i);
                }
            }
            Msg::TabClose(i) => {
                m.tab_ctx = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    let _ = s.workspace.close_tab(i);
                }
            }
            Msg::TabCloseOthers(i) => {
                m.tab_ctx = None;
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    let _ = s.workspace.close_others(i);
                }
            }
            Msg::TabCtxOpen(i, x, y) => {
                m.tab_ctx = Some((i, x, y));
                m.ctx_menu = None;
                m.menu_open = None;
                m.menu_active = usize::MAX;
            }
            Msg::FloatNew => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    if !s.pending {
                        let inst = Instance::shell(s.name.clone(), s.source.clone());
                        s.workspace.new_float(inst);
                    }
                }
            }
            Msg::FloatToggle => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.toggle_floating();
                }
            }
            Msg::FloatMove(id, dx, dy) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.workspace.move_float(id, dx, dy);
                }
            }

            // ─── Perfiles ───────────────────────────────────────────
            Msg::ShortcutEnterPrefix => {
                m.pending_prefix = true;
            }
            Msg::ShortcutCancelPrefix => {
                m.pending_prefix = false;
            }
            Msg::ShortcutFire(act) => {
                m.pending_prefix = false;
                if let Some(concrete) = act.to_concrete(&m) {
                    handle.dispatch(concrete);
                }
            }
            Msg::SwitchShortcutProfile(name) => {
                if m.shortcuts.set_active(&name).is_ok() {
                    m.pending_prefix = false;
                    if let Some(p) = perfiles::shortcuts::ShortcutProfiles::default_path() {
                        let _ = m.shortcuts.save(&p);
                    }
                }
            }
            Msg::SwitchAppearanceProfile(name) => {
                if m.appearance.set_active(&name).is_ok() {
                    if let Some(p) = perfiles::appearance::AppearanceProfiles::default_path() {
                        let _ = m.appearance.save(&p);
                    }
                    perfiles::apply_active_appearance(&mut m);
                }
            }
            Msg::SetSessionAppearance(name) => {
                if let Some(s) = m.sessions.get_mut(m.active_session) {
                    s.appearance = name;
                }
                save_sessions(&m);
                perfiles::apply_active_appearance(&mut m);
            }
            Msg::SwitchSessionProfile(name) => {
                m = switch_session_profile(m, &name);
            }

            // ─── Modal de gestión de perfiles ───────────────────────
            Msg::OpenPerfilesModal => {
                m.perfiles_modal_open = true;
                m.prof_name_focused = true;
                m.menu_open = None;
            }
            Msg::ClosePerfilesModal => {
                m.perfiles_modal_open = false;
                m.prof_name_focused = false;
                m.prof_name.set_text("");
                m.wp_path_focused = false;
            }
            Msg::PerfilesTab(kind) => {
                m.perfiles_tab = kind;
            }
            Msg::ProfNameFocus => {
                m.prof_name_focused = true;
            }
            Msg::ProfNameKey(e) => match &e.key {
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                    m.prof_name_focused = false;
                }
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                    handle.dispatch(Msg::ProfCreate(m.perfiles_tab));
                }
                _ => {
                    let _ = m.prof_name.apply_key(&e);
                }
            },
            Msg::ProfUse(kind, name) => {
                let next = match kind {
                    ProfKind::Shortcuts => Msg::SwitchShortcutProfile(name),
                    ProfKind::Appearance => Msg::SwitchAppearanceProfile(name),
                    ProfKind::Sessions => Msg::SwitchSessionProfile(name),
                };
                handle.dispatch(next);
            }
            Msg::ProfDuplicate(kind, src) => {
                let typed = m.prof_name.text().trim().to_string();
                let name = if typed.is_empty() { format!("{src} copia") } else { typed };
                let ok = match kind {
                    ProfKind::Shortcuts => m.shortcuts.duplicate(&src, &name).is_ok(),
                    ProfKind::Appearance => m.appearance.duplicate(&src, &name).is_ok(),
                    ProfKind::Sessions => m.session_profiles.create(&name).is_ok(),
                };
                if ok {
                    save_profiles(&m, kind);
                    m.prof_name.set_text("");
                }
            }
            Msg::ProfRename(kind, src) => {
                let to = m.prof_name.text().trim().to_string();
                if to.is_empty() {
                    // sin nombre nuevo no hay nada que hacer
                } else {
                    let ok = match kind {
                        ProfKind::Shortcuts => m.shortcuts.rename(&src, &to).is_ok(),
                        ProfKind::Appearance => m.appearance.rename(&src, &to).is_ok(),
                        ProfKind::Sessions => {
                            m = rename_session_profile(m, &src, &to);
                            m.session_profiles.contains(&to)
                        }
                    };
                    if ok {
                        save_profiles(&m, kind);
                        m.prof_name.set_text("");
                    }
                }
            }
            Msg::ProfDelete(kind, name) => {
                let ok = match kind {
                    ProfKind::Shortcuts => m.shortcuts.remove(&name).is_ok(),
                    ProfKind::Appearance => m.appearance.remove(&name).is_ok(),
                    ProfKind::Sessions => m.session_profiles.remove(&name).is_ok(),
                };
                if ok {
                    save_profiles(&m, kind);
                    if kind == ProfKind::Appearance {
                        perfiles::apply_active_appearance(&mut m);
                    }
                }
            }
            Msg::ProfCreate(kind) => {
                let name = m.prof_name.text().trim().to_string();
                if !name.is_empty() {
                    let ok = match kind {
                        // Atajos: arranca como el nativo `shuma`.
                        ProfKind::Shortcuts => {
                            let base = perfiles::shortcuts::preset("shuma").expect("preset");
                            m.shortcuts.create(&name, base).is_ok()
                        }
                        // Apariencia: arranca como el perfil activo (o Oscuro).
                        ProfKind::Appearance => {
                            let base = m.appearance.active_appearance();
                            m.appearance.create(&name, base).is_ok()
                        }
                        // Sesión: contexto nuevo y vacío.
                        ProfKind::Sessions => m.session_profiles.create(&name).is_ok(),
                    };
                    if ok {
                        save_profiles(&m, kind);
                        m.prof_name.set_text("");
                    }
                }
            }
            Msg::WpPathFocus => {
                m.wp_path_focused = true;
                m.prof_name_focused = false;
            }
            Msg::WpPathKey(e) => match &e.key {
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Escape) => {
                    m.wp_path_focused = false;
                }
                llimphi_ui::Key::Named(llimphi_ui::NamedKey::Enter) => {
                    handle.dispatch(Msg::SetWallpaperActive);
                }
                _ => {
                    let _ = m.wp_path.apply_key(&e);
                }
            },
            Msg::SetWallpaperActive => {
                let path = m.wp_path.text().trim().to_string();
                if !path.is_empty() {
                    let active = m.appearance.active().to_string();
                    if m.appearance.set_wallpaper(&active, Some(path)).is_ok() {
                        save_profiles(&m, ProfKind::Appearance);
                        // Forzar re-decodificación aunque el path lógico no haya
                        // cambiado de nombre (p.ej. mismo perfil, archivo nuevo).
                        m.wallpaper_path = None;
                        perfiles::apply_active_appearance(&mut m);
                    }
                }
            }
            Msg::ClearWallpaperActive => {
                let active = m.appearance.active().to_string();
                if m.appearance.set_wallpaper(&active, None).is_ok() {
                    save_profiles(&m, ProfKind::Appearance);
                    m.wp_path.set_text("");
                    perfiles::apply_active_appearance(&mut m);
                }
            }
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;

        // Modo dock: vista compacta para la barra layer-shell — la command-bar a
        // todo lo ancho + un botón para volver a ventana. Sin tabs/monitores.
        if model.dock_mode {
            return dock_bar_view(model, theme);
        }

        let menubar = menu::menubar_row(model, theme);
        let topbar = render_topbar(model, theme);
        let main_area = render_main_area(model, theme);
        let bottombar = render_bottombar(model, theme);
        let content = vec![menubar, topbar, main_area, bottombar];

        // Con wallpaper: capa de imagen a tamaño completo (Cover) detrás de una
        // columna de contenido con fondo (translúcido) que la deja ver.
        if let Some(img) = &model.wallpaper_img {
            let full = Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            };
            let bg = View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(0.0_f32),
                    right: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                size: full,
                ..Default::default()
            })
            .image(img.clone())
            .image_fit(ImageFit::Cover);
            let column = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: full,
                ..Default::default()
            })
            .fill(theme.bg_app)
            .children(content);
            return View::new(Style {
                size: full,
                ..Default::default()
            })
            .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
            .children(vec![bg, column]);
        }

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
        .children(content)
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
        if model.perfiles_modal_open {
            return Some(view::perfiles_modal(model, &model.theme));
        }
        view::dropdown_overlay(model).or_else(|| menu::overlay(model))
    }
}

/// ¿Debe Esc cerrar el drawer Quake que hospeda a esta shuma? El chasis (pata)
/// lo pregunta ANTES de reenviar la tecla a `on_key`. Devuelve `false` cuando
/// shuma tiene algo propio que descartar con Esc —un modal, un dropdown, un
/// campo/draft con foco, una sesión en creación— o cuando el shell enfocado
/// corre una TUI de pantalla completa (vim/htop/less/man) que necesita el Esc.
/// En cualquier otro caso (prompt ocioso, modo líneas) Esc repliega el drawer.
pub fn escape_closes_drawer(model: &Model) -> bool {
    if model.hosts_modal_open
        || model.containers_modal_open
        || model.layouts_modal_open
        || model.perfiles_modal_open
        || model.focused_field.is_some()
        || model.dropdown_open.is_some()
    {
        return false;
    }
    if model.host_draft.as_ref().is_some_and(|d| d.focused.is_some()) {
        return false;
    }
    if model.container_draft.as_ref().is_some_and(|d| d.focus.is_some()) {
        return false;
    }
    if model.active().is_some_and(|s| s.pending) {
        return false;
    }
    let fullscreen_tui =
        |state: &ModuleState| matches!(state, ModuleState::Shell(s) if s.is_fullscreen_tui());
    if let Some(inst) = model.main.as_ref() {
        if fullscreen_tui(&inst.state) {
            return false;
        }
    }
    if let Some(s) = model.active() {
        if fullscreen_tui(&s.shell().state) {
            return false;
        }
    }
    true
}

/// Vista compacta para el **modo dock** (barra layer-shell): la command-bar a
/// todo lo ancho + un botón «ventana» que vuelve al modo ventana. La barra es
/// fina (la fija `llimphi-layer`), así que no caben tabs/monitores.
fn dock_bar_view(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::{auto, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    let bar = View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        size: Size {
            width: auto(),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![render_bottombar(model, theme)]);

    let btn = View::new(Style {
        flex_shrink: 0.0,
        size: Size {
            width: length(74.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .on_click(Msg::MenuCommand("window.toggle-dock".to_string()))
    .text_aligned("ventana".to_string(), 12.0, theme.fg_muted, Alignment::Center);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![bar, btn])
}

// ─── Superficie hosteable (para pata) ────────────────────────────────
//
// Funciones libres que **delegan** a los métodos del `App` `Shell`. El
// standalone queda idéntico (el App impl no se toca); un host como pata
// construye el `Model` con `new_model()`, lo tickea con `spawn_host_effects`
// (handle lifteado), le rutea input/Msg con `update`/`on_key`/`on_wheel`/
// `on_resize`, y pinta `view(model).map(...)` + `view_overlay(model).map(...)`.

/// Aplica un `Msg` al `Model` de shuma (delegado a `App::update`).
pub fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
    <Shell as App>::update(model, msg, handle)
}

/// Vista principal de shuma para `model` (delegado a `App::view`).
pub fn view(model: &Model) -> View<Msg> {
    <Shell as App>::view(model)
}

/// Overlay (modales/menús/dropdowns) de shuma, si hay (delegado a `App::view_overlay`).
pub fn view_overlay(model: &Model) -> Option<View<Msg>> {
    <Shell as App>::view_overlay(model)
}

/// Traduce una tecla a un `Msg` de shuma según el foco actual (delegado a `App::on_key`).
pub fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
    <Shell as App>::on_key(model, e)
}

/// Traduce la rueda a un `Msg` de shuma (delegado a `App::on_wheel`).
pub fn on_wheel(
    model: &Model,
    delta: WheelDelta,
    cursor: (f32, f32),
    modifiers: Modifiers,
) -> Option<Msg> {
    <Shell as App>::on_wheel(model, delta, cursor, modifiers)
}

/// Reacciona a un resize del área hospedada (delegado a `App::on_resize`).
pub fn on_resize(model: &Model, width: u32, height: u32) -> Option<Msg> {
    <Shell as App>::on_resize(model, width, height)
}

/// Vista del **input vivo de la sesión activa**, aislado del resto del chrome,
/// para hospedarlo en una barra externa (el cabezal de pata): es el mismísimo
/// `shell_input_view` que pinta el canvas, ruteado por el `lift` de la sesión
/// activa, así que tipear ahí ejecuta en esa sesión. `None` si la activa no es
/// un shell (form de nueva sesión / sin sesiones) — en ese caso el host muestra
/// un fallback. Espeja `shuma_module_shell::input_view` a nivel de la app
/// completa (la sesión activa ES un `shuma-module-shell`).
pub fn active_input_view(model: &Model, theme: &Theme) -> Option<View<Msg>> {
    let session = model.active()?;
    if session.pending {
        return None;
    }
    let idx = model.active_session;
    match &session.shell().state {
        ModuleState::Shell(state) => Some(shuma_module_shell::input_view(state, theme, move |m| {
            Msg::Module(Slot::Session(idx, Which::Shell), ModuleMsg::Shell(m))
        })),
        _ => None,
    }
}

/// `true` si el `Msg` es el "focalizar el input" de un shell de sesión (click
/// sobre el input vivo). El host (pata) lo usa para abrir su drawer cuando se
/// clickea el cabezal de la barra —espeja el auto-open de FocusInput del path
/// bare—.
pub fn msg_is_focus_input(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::Module(_, ModuleMsg::Shell(shuma_module_shell::Msg::FocusInput))
    )
}
