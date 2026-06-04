//! `mirada-llimphi` — la ventana del Cerebro del compositor.
//!
//! Es el "Cerebro" de la arquitectura carmen hecho app Llimphi: envuelve
//! [`mirada_brain::Desktop`] (toda la lógica de teselado y foco) y lo
//! pinta. La cadena completa:
//!
//! ```text
//!   mirada-layout ─► mirada-protocol ─► mirada-brain ─► [esta ventana]
//!                                          │
//!                                    mirada-link ─► mirada-compositor (Cuerpo)
//! ```
//!
//! Con un Cuerpo conectado (variable `MIRADA_SOCKET`) sondea sus
//! [`BodyEvent`]s y le devuelve [`BrainCommand`]s por el socket. Sin
//! Cuerpo arranca en **simulación**: las ventanas son sintéticas y el
//! teclado de esta ventana maneja el escritorio — útil para ver el
//! motor de teselado sin hardware.
//!
//! Teclas (simulación):
//!
//! ```text
//!   n / Shift+n  abre ventana / monitor    tab / espacio  cicla layout
//!   w            cierra la enfocada        t m g c r d s  layout directo
//!   f / Shift+f  flota / pantalla completa h / l          área maestra −/+
//!   j / k        foco siguiente/anterior   , / .          nmaster −/+
//!   Shift+j / k  mueve la enfocada         1..9           ir a escritorio
//!   Enter        promueve a maestra        Ctrl+1..9      enviar a escritorio
//!   o            siguiente monitor         ` / Shift+`    scratchpad ver/guardar
//! ```
//!
//! Los pips de escritorio y las ventanas del lienzo son **clicables**, y
//! `mirada-ctl` controla el escritorio desde la terminal — ambos pasan
//! por el mismo `Desktop::apply`.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, Dimension, FlexDirection, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlConn, CtlReply, CtlRequest, CtlServer, Desktop, DesktopAction,
    Keymap, KeymapWatch, LayoutMode, Rules, WindowId, WindowPlacement,
};
use mirada_link::BrainLink;

/// Pantalla virtual del modo simulación — coincide con el lienzo.
const SCREEN_W: i32 = 1280;
const SCREEN_H: i32 = 720;
/// Período del sondeo del Cuerpo, en ms (~60 Hz).
const POLL_MS: u64 = 16;

/// Nombres de app ficticios para las ventanas de simulación.
const APPS: &[&str] = &[
    "shuma", "pluma_app", "revista", "cosmobiología", "matilda", "pluma_notebook_app", "barra",
];

struct Model {
    theme: Theme,
    desktop: Desktop,
    /// Geometría vigente — lo que se pinta. Es la última `Place` emitida.
    placements: Vec<WindowPlacement>,
    /// Contador de ids para las ventanas sintéticas.
    next_id: WindowId,
    /// Cable al Cuerpo; `None` en simulación.
    link: Option<BrainLink>,
    /// Última acción, para la barra de estado.
    note: String,
    /// Ruta del keymap del usuario, para recargarlo en caliente.
    keymap_path: Option<PathBuf>,
    /// Vigía del keymap; `None` en simulación o si no hay archivo.
    keymap_watch: Option<KeymapWatch>,
    /// Socket del API de control externo (`mirada-ctl`).
    ctl: Option<CtlServer>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada dentro del dropdown abierto (`usize::MAX` = ninguna).
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal.
    menu_anim: Tween<f32>,
    /// Menú contextual sobre la ventana enfocada: ancla `(x, y)` en
    /// coordenadas de ventana. `None` cerrado. No hay edición de texto,
    /// así que el contextual sólo ofrece acciones de gestión de ventana.
    context_menu: Option<(f32, f32)>,
}

#[derive(Clone)]
enum Msg {
    /// Tick periódico: drena el Cuerpo, vigila el keymap, atiende ctl.
    Tick,
    /// Tecla recibida desde la ventana de simulación.
    Key(KeyEvent),
    /// Click en un pip de escritorio.
    SwitchWorkspace(usize),
    /// Click en una ventana del lienzo.
    FocusWindow(WindowId),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce a acciones reales.
    MenuCommand(String),
    /// Navegación por teclado dentro del dropdown: +1 baja, -1 sube.
    MenuNav(i32),
    /// Enter sobre la fila resaltada del dropdown.
    MenuActivate,
    /// Tick de la animación del menú (sólo re-render).
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` sobre la ventana enfocada. Sin ventana enfocada es no-op.
    ContextMenuOpen(f32, f32),
    /// Ejecuta una acción de escritorio (usado por el menú contextual y
    /// el principal sobre la ventana enfocada).
    Act(DesktopAction),
}

struct Mirada;

impl App for Mirada {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "brahman · mirada"
    }

    fn initial_size() -> (u32, u32) {
        (SCREEN_W as u32, (SCREEN_H + 70) as u32)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Keymap del usuario (~/.config/mirada/keymap.ron): define los
        // atajos que el Cuerpo intercepta y nos devuelve como `Keybind`.
        let keymap_path = Keymap::default_path();
        let keymap = match &keymap_path {
            Some(p) => Keymap::load_or_init(p),
            None => Keymap::default(),
        };
        let link = connect_body();
        // Vigilar el keymap sólo tiene sentido con un Cuerpo conectado;
        // en simulación, mirada usa las teclas de su propia ventana.
        let keymap_watch = if link.is_some() {
            keymap_path.as_deref().and_then(|p| Keymap::watch(p).ok())
        } else {
            None
        };
        // API de control: mirada siempre posee el Desktop, así que
        // siempre abre el socket de `mirada-ctl`.
        let ctl = match CtlServer::bind(&mirada_brain::ctl::default_socket_path()) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("mirada · sin API de control: {e}");
                None
            }
        };

        let mut desktop = Desktop::with_keymap(keymap);
        desktop.set_rules(load_user_rules());

        let mut model = Model {
            theme: Theme::dark(),
            desktop,
            placements: Vec::new(),
            next_id: 1,
            link,
            note: rimay_localize::t("success"),
            keymap_path,
            keymap_watch,
            ctl,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
        };
        if let Some(link) = model.link.as_mut() {
            let _ = link.send(&model.desktop.grab_keys());
            model.note = rimay_localize::t("mirada-status-body-connected");
        } else {
            // Simulación: una pantalla virtual y tres ventanas de muestra.
            feed(&mut model, BodyEvent::OutputAdded {
                id: 0,
                width: SCREEN_W,
                height: SCREEN_H,
            });
            for _ in 0..3 {
                open_window(&mut model);
            }
            model.note = rimay_localize::t("mirada-status-simulation");
        }

        // El sondeo corre siempre: drena el Cuerpo (si lo hay), vigila el
        // keymap y atiende `mirada-ctl`. Llega como `Msg::Tick` al update.
        handle.spawn_periodic(Duration::from_millis(POLL_MS), || Msg::Tick);

        model
    }

    fn on_key(model: &Model, e: &KeyEvent) -> Option<Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Con el menú principal abierto las flechas navegan: ←/→ cambian de
        // menú raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta y
        // Esc cierra. Consume la tecla.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        Some(Msg::Key(e.clone()))
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => tick(&mut m),
            Msg::Key(ev) => handle_key(&mut m, &ev),
            Msg::SwitchWorkspace(i) => act(&mut m, DesktopAction::SwitchWorkspace(i)),
            Msg::FocusWindow(id) => act(&mut m, DesktopAction::FocusWindow(id)),
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
                m.menu_active = usize::MAX;
                // Animación de aparición/swap del dropdown.
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu(&m);
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        handle_menu_command(&mut m, &cmd);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.context_menu = None;
                m.menu_active = usize::MAX;
            }
            Msg::ContextMenuOpen(x, y) => {
                // Sólo tiene sentido con una ventana enfocada.
                if m.desktop.focused_window().is_some() {
                    m.menu_open = None;
                    m.context_menu = Some((x, y));
                }
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                handle_menu_command(&mut m, &cmd);
            }
            Msg::Act(action) => {
                m.menu_open = None;
                m.context_menu = None;
                act(&mut m, action);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        // Barra de menú principal — primer hijo del column raíz.
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, theme));
        // Colores cromáticos heredados del original (HSL → RGB hardcoded).
        let win_bg = Color::from_rgba8(28, 32, 41, 255);
        let bar_bg = Color::from_rgba8(19, 22, 30, 255);
        let canvas_bg = Color::from_rgba8(10, 13, 19, 255);
        let on_accent = Color::from_rgba8(12, 16, 24, 255);

        let active = model.desktop.active_index();
        let mode = model.desktop.active_workspace().params().mode;
        let loads = model.desktop.workspace_loads();
        let focused = model.desktop.focused_window();

        // --- Barra superior: identidad + escritorios + modo ----------
        let bar = top_bar(model, theme, mode, &loads, active, focused, on_accent, bar_bg);

        // --- Lienzo: el escritorio teselado, a escala ----------------
        let canvas = canvas_view(model, theme, on_accent, win_bg, canvas_bg);

        // --- Pie de estado ------------------------------------------
        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(26.0_f32),
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
        .fill(bar_bg)
        .text_aligned(model.note.clone(), 11.0, theme.fg_placeholder, Alignment::Start);

        // --- Composición ---------------------------------------------
        let canvas_wrap = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![canvas]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        // El origen de la raíz es (0,0) ⇒ coords locales == coords de
        // ventana. Right-click abre el contextual sobre la ventana
        // enfocada.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, bar, canvas_wrap, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El menú contextual de la ventana enfocada tiene prioridad.
        if let Some((x, y)) = model.context_menu {
            let focused = model.desktop.focused_window();
            let label = focused
                .and_then(|id| model.desktop.window_info(id))
                .map(|i| i.title.clone())
                .unwrap_or_else(|| rimay_localize::t("mirada-win-label-fallback"));
            // Acciones reales del Desktop sobre la enfocada. Sin edición
            // de texto: el contextual es de gestión de ventana.
            let t = rimay_localize::t;
            let actions: [(String, DesktopAction); 6] = [
                (t("mirada-win-promote"), DesktopAction::PromoteToMaster),
                (t("mirada-win-float"), DesktopAction::ToggleFloat),
                (t("mirada-win-fullscreen"), DesktopAction::ToggleFullscreen),
                (t("mirada-win-scratchpad"), DesktopAction::SendToScratchpad),
                (t("mirada-output-next"), DesktopAction::FocusOutputNext),
                (t("close"), DesktopAction::CloseFocused),
            ];
            // "Cerrar" (último) se marca como destructivo.
            let last = actions.len() - 1;
            let items: Vec<ContextMenuItem> = actions
                .iter()
                .enumerate()
                .map(|(i, (l, _))| {
                    let it = ContextMenuItem::action(l.clone());
                    if i == last {
                        it.destructive()
                    } else {
                        it
                    }
                })
                .collect();
            let acts: Vec<DesktopAction> = actions.iter().map(|(_, a)| a.clone()).collect();
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> =
                Arc::new(move |i: usize| Msg::Act(acts[i].clone()));
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(model),
                header: Some(label),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&model.theme),
            }));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &model.theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

// ─── Lógica fuera del trait App ─────────────────────────────────────

/// Bucle de poll consolidado: drena Cuerpo, recarga keymap, sirve ctl.
fn tick(m: &mut Model) {
    let events: Vec<BodyEvent> = match m.link.as_ref() {
        Some(link) => link.drain(),
        None => Vec::new(),
    };
    let keymap_changed = m.keymap_watch.as_ref().is_some_and(|w| w.changed());
    if keymap_changed {
        reload_keymap(m);
    }
    let _ctl_served = poll_ctl(m);
    for ev in events {
        feed(m, ev);
    }
}

fn open_window(m: &mut Model) {
    let id = m.next_id;
    m.next_id += 1;
    let app = APPS[(id as usize) % APPS.len()];
    feed(m, BodyEvent::WindowOpened {
        id,
        app_id: format!("org.brahman.{app}"),
        title: format!("{app} · ventana {id}"),
    });
    m.note = format!("abierta ventana {id}");
}

fn feed(m: &mut Model, event: BodyEvent) {
    let cmds = m.desktop.on_event(event);
    dispatch(m, cmds);
}

fn act(m: &mut Model, action: DesktopAction) {
    let cmds = m.desktop.apply(action);
    dispatch(m, cmds);
}

fn reload_keymap(m: &mut Model) {
    let Some(path) = m.keymap_path.clone() else {
        return;
    };
    match Keymap::load(&path) {
        Ok(km) => {
            let cmd = m.desktop.set_keymap(km);
            dispatch(m, vec![cmd]);
            m.note = rimay_localize::t("mirada-status-keymap-reloaded");
        }
        Err(e) => m.note = format!("{}: {e}", rimay_localize::t("mirada-status-keymap-invalid")),
    }
}

fn poll_ctl(m: &mut Model) -> bool {
    let conns: Vec<CtlConn> = match &m.ctl {
        Some(ctl) => std::iter::from_fn(|| ctl.poll()).collect(),
        None => return false,
    };
    let mut served = false;
    for mut conn in conns {
        let reply = match conn.read_request() {
            Ok(Some(req)) => {
                served = true;
                serve_ctl(m, req)
            }
            Ok(None) => continue,
            Err(e) => CtlReply::Error(format!("{e}")),
        };
        let _ = conn.reply(&reply);
    }
    served
}

fn serve_ctl(m: &mut Model, req: CtlRequest) -> CtlReply {
    match req {
        CtlRequest::Do(action) => {
            act(m, action);
            CtlReply::Ok
        }
        CtlRequest::ListWindows => CtlReply::Windows(m.desktop.window_lines()),
        // Las zonas de arrastre son del compositor; esta app de Cerebro no las
        // gestiona.
        CtlRequest::CycleZones => CtlReply::Ok,
    }
}

fn dispatch(m: &mut Model, cmds: Vec<BrainCommand>) {
    for cmd in &cmds {
        if let BrainCommand::Place(p) = cmd {
            m.placements = p.clone();
        }
    }
    match m.link.as_mut() {
        Some(link) => {
            for cmd in &cmds {
                let _ = link.send(cmd);
            }
        }
        None => {
            for cmd in cmds {
                match cmd {
                    BrainCommand::Close(id) | BrainCommand::Kill(id) => {
                        feed(m, BodyEvent::WindowClosed { id });
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Mapea una tecla a una acción de escritorio. La firma cambia respecto
/// del original GPUI: ahora muta `Model` directamente porque el bucle
/// Llimphi es Elm puro.
fn handle_key(m: &mut Model, ev: &KeyEvent) {
    let shift = ev.modifiers.shift;
    let ctrl = ev.modifiers.ctrl;
    let connected = m.link.is_some();

    let key_str: Option<String> = match &ev.key {
        Key::Named(NamedKey::Tab) => Some("tab".into()),
        Key::Named(NamedKey::Space) => Some("space".into()),
        Key::Named(NamedKey::Enter) => Some("enter".into()),
        Key::Character(s) => Some(s.to_lowercase()),
        _ => None,
    };
    let Some(k) = key_str else { return };

    match k.as_str() {
        "n" if shift && !connected => {
            let id = m.desktop.outputs().len() as u32;
            feed(m, BodyEvent::OutputAdded {
                id,
                width: SCREEN_W,
                height: SCREEN_H,
            });
        }
        "n" if !connected => open_window(m),
        "w" => act(m, DesktopAction::CloseFocused),
        "f" if shift => act(m, DesktopAction::ToggleFullscreen),
        "f" => act(m, DesktopAction::ToggleFloat),
        "j" if shift => act(m, DesktopAction::MoveForward),
        "k" if shift => act(m, DesktopAction::MoveBackward),
        "j" => act(m, DesktopAction::FocusNext),
        "k" => act(m, DesktopAction::FocusPrev),
        "tab" | "space" => act(m, DesktopAction::CycleLayout),
        "t" => act(m, DesktopAction::SetLayout(LayoutMode::MasterStack)),
        "m" => act(m, DesktopAction::SetLayout(LayoutMode::Monocle)),
        "g" => act(m, DesktopAction::SetLayout(LayoutMode::Grid)),
        "c" => act(m, DesktopAction::SetLayout(LayoutMode::Columns)),
        "r" => act(m, DesktopAction::SetLayout(LayoutMode::Rows)),
        "d" => act(m, DesktopAction::SetLayout(LayoutMode::CenteredMaster)),
        "s" => act(m, DesktopAction::SetLayout(LayoutMode::Spiral)),
        "h" => act(m, DesktopAction::ShrinkMaster),
        "l" => act(m, DesktopAction::GrowMaster),
        "o" => act(m, DesktopAction::FocusOutputNext),
        "`" if shift => act(m, DesktopAction::SendToScratchpad),
        "`" => act(m, DesktopAction::ToggleScratchpad),
        "enter" => act(m, DesktopAction::PromoteToMaster),
        "," => act(m, DesktopAction::IncMaster),
        "." => act(m, DesktopAction::DecMaster),
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() && d != "0" => {
            let n = (d.as_bytes()[0] - b'1') as usize;
            if ctrl {
                act(m, DesktopAction::SendToWorkspace(n));
            } else {
                act(m, DesktopAction::SwitchWorkspace(n));
            }
        }
        _ => {}
    }
}

fn connect_body() -> Option<BrainLink> {
    let path = std::env::var("MIRADA_SOCKET").ok()?;
    BrainLink::connect(&path).ok()
}

fn load_user_rules() -> Rules {
    match Rules::default_path() {
        Some(p) => Rules::load_or_default(&p),
        None => Rules::default(),
    }
}

fn mode_name(m: LayoutMode) -> String {
    match m {
        LayoutMode::MasterStack => rimay_localize::t("mirada-layout-master-stack"),
        LayoutMode::Monocle => rimay_localize::t("mirada-layout-monocle"),
        LayoutMode::Grid => rimay_localize::t("mirada-layout-grid"),
        LayoutMode::Columns => rimay_localize::t("mirada-layout-columns"),
        LayoutMode::Rows => rimay_localize::t("mirada-layout-rows"),
        LayoutMode::CenteredMaster => rimay_localize::t("mirada-layout-centered"),
        LayoutMode::Spiral => rimay_localize::t("mirada-layout-spiral"),
    }
}

// ─── Menú principal y contextual ────────────────────────────────────

/// Viewport para clampear overlays. El Model no trackea el tamaño de
/// ventana, así que usamos `initial_size()`.
fn viewport_of(_model: &Model) -> (f32, f32) {
    let (w, h) = Mirada::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(model),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// El menú principal de mirada. Archivo / Ver / Ayuda — sólo comandos que
/// mapean a acciones reales del `Desktop`. Sin "Editar": no hay campos de
/// texto editables. Los items que actúan sobre la enfocada se inhabilitan
/// cuando no hay ventana enfocada. Abrir ventana/monitor sólo tiene
/// sentido en simulación (sin Cuerpo conectado).
fn app_menu(model: &Model) -> AppMenu {
    let has_focus = model.desktop.focused_window().is_some();
    let sim = model.link.is_none();
    let mode = model.desktop.active_workspace().params().mode;

    // Etiquetas de UI localizadas: IDs genéricos del catálogo o prefijados
    // con `mirada-`. Los segundos argumentos de MenuItem (ids de comando)
    // son estables y NO se localizan.
    let t = rimay_localize::t;

    let mut abrir = MenuItem::new(t("mirada-menu-open-window"), "file.new_window").shortcut("n");
    let mut abrir_mon = MenuItem::new(t("mirada-menu-open-output"), "file.new_output").shortcut("Shift+n");
    if !sim {
        // Con Cuerpo conectado, las ventanas las crea el compositor real.
        abrir = abrir.disabled();
        abrir_mon = abrir_mon.disabled();
    }
    let mut cerrar = MenuItem::new(t("mirada-menu-close-focused"), "win.close").shortcut("w").separated();
    if !has_focus {
        cerrar = cerrar.disabled();
    }

    // Submenú de layouts: el modo vigente queda en gris (ya aplicado).
    let layout_item = |label: String, cmd: &str, m: LayoutMode| {
        let it = MenuItem::new(label, cmd);
        if mode == m {
            it.disabled()
        } else {
            it
        }
    };

    let mut promover = MenuItem::new(t("mirada-win-promote"), "win.promote").shortcut("Enter");
    let mut flotar = MenuItem::new(t("mirada-win-float"), "win.float").shortcut("f");
    let mut fullscreen = MenuItem::new(t("mirada-win-fullscreen"), "win.fullscreen").shortcut("Shift+f");
    let mut scratch = MenuItem::new(t("mirada-win-scratchpad"), "win.scratchpad").shortcut("Shift+`");
    if !has_focus {
        promover = promover.disabled();
        flotar = flotar.disabled();
        fullscreen = fullscreen.disabled();
        scratch = scratch.disabled();
    }

    // Menú de idioma: autónimos sin traducir (convención del SO). El item
    // activo lleva ✔. El comando `lang.<code>` lo resuelve `handle_menu_command`.
    let cur = rimay_localize::current_locale();
    let lang_item = |label: &str, code: &str| {
        let mut it = MenuItem::new(label, format!("lang.{code}"));
        if cur == code {
            it = it.icon("\u{2714}");
        }
        it
    };

    AppMenu::new()
        .menu(
            Menu::new(t("file"))
                .item(abrir)
                .item(abrir_mon)
                .item(cerrar)
                .item(MenuItem::new(t("exit"), "file.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new(t("view"))
                .item(MenuItem::new(t("mirada-layout-cycle"), "view.cycle").shortcut("Tab"))
                .item(layout_item(t("mirada-layout-master-stack"), "layout.master", LayoutMode::MasterStack).separated())
                .item(layout_item(t("mirada-layout-monocle"), "layout.monocle", LayoutMode::Monocle))
                .item(layout_item(t("mirada-layout-grid"), "layout.grid", LayoutMode::Grid))
                .item(layout_item(t("mirada-layout-columns"), "layout.columns", LayoutMode::Columns))
                .item(layout_item(t("mirada-layout-rows"), "layout.rows", LayoutMode::Rows))
                .item(layout_item(t("mirada-layout-centered"), "layout.centered", LayoutMode::CenteredMaster))
                .item(layout_item(t("mirada-layout-spiral"), "layout.spiral", LayoutMode::Spiral))
                .item(MenuItem::new(t("mirada-layout-shrink"), "view.shrink").shortcut("h").separated())
                .item(MenuItem::new(t("mirada-layout-grow"), "view.grow").shortcut("l"))
                .item(MenuItem::new(t("mirada-output-next"), "view.output_next").shortcut("o").separated()),
        )
        .menu(
            Menu::new(t("mirada-menu-window"))
                .item(promover)
                .item(flotar)
                .item(fullscreen)
                .item(scratch),
        )
        .menu(
            Menu::new(t("help"))
                .item(MenuItem::new(t("about"), "help.about")),
        )
        .menu(
            Menu::new(t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
}

/// Traduce un command id del menú principal a la acción real del Desktop.
fn handle_menu_command(m: &mut Model, cmd: &str) {
    // Cambio de idioma: aplica el locale en caliente y lo persiste en wawa-config.
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        let mut cfg = wawa_config::WawaConfig::load();
        cfg.lang = code.to_string();
        let _ = cfg.save();
        return;
    }
    match cmd {
        "file.new_window" if m.link.is_none() => open_window(m),
        "file.new_output" if m.link.is_none() => {
            let id = m.desktop.outputs().len() as u32;
            feed(m, BodyEvent::OutputAdded {
                id,
                width: SCREEN_W,
                height: SCREEN_H,
            });
        }
        "win.close" => act(m, DesktopAction::CloseFocused),
        "file.quit" => std::process::exit(0),
        "view.cycle" => act(m, DesktopAction::CycleLayout),
        "layout.master" => act(m, DesktopAction::SetLayout(LayoutMode::MasterStack)),
        "layout.monocle" => act(m, DesktopAction::SetLayout(LayoutMode::Monocle)),
        "layout.grid" => act(m, DesktopAction::SetLayout(LayoutMode::Grid)),
        "layout.columns" => act(m, DesktopAction::SetLayout(LayoutMode::Columns)),
        "layout.rows" => act(m, DesktopAction::SetLayout(LayoutMode::Rows)),
        "layout.centered" => act(m, DesktopAction::SetLayout(LayoutMode::CenteredMaster)),
        "layout.spiral" => act(m, DesktopAction::SetLayout(LayoutMode::Spiral)),
        "view.shrink" => act(m, DesktopAction::ShrinkMaster),
        "view.grow" => act(m, DesktopAction::GrowMaster),
        "view.output_next" => act(m, DesktopAction::FocusOutputNext),
        "win.promote" => act(m, DesktopAction::PromoteToMaster),
        "win.float" => act(m, DesktopAction::ToggleFloat),
        "win.fullscreen" => act(m, DesktopAction::ToggleFullscreen),
        "win.scratchpad" => act(m, DesktopAction::SendToScratchpad),
        // "help.about" y desconocidos: no-op (sin diálogo todavía).
        _ => {}
    }
}

// ─── Subviews ───────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn top_bar(
    model: &Model,
    theme: &Theme,
    mode: LayoutMode,
    loads: &[usize],
    active: usize,
    focused: Option<WindowId>,
    on_accent: Color,
    bar_bg: Color,
) -> View<Msg> {
    let mut pips: Vec<View<Msg>> = Vec::with_capacity(loads.len());
    for (i, &load) in loads.iter().enumerate() {
        let is_active = i == active;
        let fg = if is_active {
            on_accent
        } else if load > 0 {
            theme.fg_text
        } else {
            theme.fg_placeholder
        };
        let bg = if is_active {
            theme.accent
        } else if load > 0 {
            theme.bg_row_hover
        } else {
            bar_bg
        };
        pips.push(
            View::new(Style {
                size: Size {
                    width: length(24.0_f32),
                    height: length(22.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(bg)
            .radius(4.0)
            .text_aligned(format!("{}", i + 1), 12.0, fg, Alignment::Start)
            .on_click(Msg::SwitchWorkspace(i)),
        );
    }

    let focus_label = match focused.and_then(|id| model.desktop.window_info(id)) {
        Some(info) => info.title.clone(),
        None => "—".to_string(),
    };

    let pips_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: Dimension::auto(),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(pips);

    let label_node = |text: String, color: Color, size: f32, width: f32| {
        View::new(Style {
            size: Size {
                width: length(width),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(text, size, color, Alignment::Start)
    };

    let mirada_tag = label_node("mirada".into(), theme.accent, 13.0, 70.0);
    let sep_a = label_node("·".into(), theme.fg_placeholder, 12.0, 12.0);
    let sep_b = label_node("·".into(), theme.fg_placeholder, 12.0, 12.0);
    let layout_label = label_node(
        format!("{}: {}", rimay_localize::t("mirada-label-layout"), mode_name(mode)),
        theme.fg_muted,
        12.0,
        180.0,
    );
    let spacer = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(22.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    });
    let focus_label_node = label_node(
        format!("{}: {focus_label}", rimay_localize::t("mirada-label-focus")),
        theme.fg_muted,
        12.0,
        320.0,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(12.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bar_bg)
    .children(vec![
        mirada_tag,
        sep_a,
        pips_row,
        sep_b,
        layout_label,
        spacer,
        focus_label_node,
    ])
}

fn canvas_view(
    model: &Model,
    theme: &Theme,
    on_accent: Color,
    win_bg: Color,
    canvas_bg: Color,
) -> View<Msg> {
    let outs = model.desktop.outputs();
    let (bb_w, bb_h) = if outs.is_empty() {
        (SCREEN_W as f32, SCREEN_H as f32)
    } else {
        let w = outs.iter().map(|o| o.rect.x + o.rect.w).max().unwrap_or(SCREEN_W);
        let h = outs.iter().map(|o| o.rect.y + o.rect.h).max().unwrap_or(SCREEN_H);
        (w as f32, h as f32)
    };
    let scale = (SCREEN_W as f32 / bb_w)
        .min(SCREEN_H as f32 / bb_h)
        .min(1.0);

    let mut children: Vec<View<Msg>> = Vec::new();

    // Marcos de cada salida.
    for (i, o) in outs.iter().enumerate() {
        let is_focused_out = i == model.desktop.focused_output();
        let border = if is_focused_out { theme.accent } else { theme.border };
        let label = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(16.0_f32),
            },
            padding: Rect {
                left: length(4.0_f32),
                right: length(4.0_f32),
                top: length(2.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            format!(
                "{} {} · {} {}",
                rimay_localize::t("mirada-label-output"),
                i + 1,
                rimay_localize::t("mirada-label-workspace"),
                o.workspace + 1,
            ),
            10.0,
            theme.fg_placeholder,
            Alignment::Start,
        );
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(o.rect.x as f32 * scale),
                    top: length(o.rect.y as f32 * scale),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(o.rect.w as f32 * scale),
                    height: length(o.rect.h as f32 * scale),
                },
                ..Default::default()
            })
            // Llimphi no tiene "border 1px" como propiedad — emulamos
            // con un fill del color del border que pinta una franja
            // alrededor del contenido vía padding (cheap edge).
            .fill(border)
            .children(vec![label]),
        );
    }

    // Mensaje vacío.
    let visible = model.placements.iter().filter(|p| p.visible).count();
    if visible == 0 {
        children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(0.0_f32),
                    right: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text_aligned(
                rimay_localize::t("mirada-canvas-empty-hint"),
                13.0,
                theme.fg_placeholder,
                Alignment::Center,
            ),
        );
    }

    // Ventanas.
    for p in model.placements.iter().filter(|p| p.visible) {
        let info = model.desktop.window_info(p.id);
        let title = info
            .map(|i| i.title.clone())
            .unwrap_or_else(|| format!("ventana {}", p.id));
        let app_id = info.map(|i| i.app_id.clone()).unwrap_or_default();
        let border = if p.focused { theme.accent } else { theme.border };
        let tb_bg = if p.focused { theme.accent } else { theme.bg_row_hover };
        let tb_fg = if p.focused { on_accent } else { theme.fg_muted };
        let kind_label = if p.fullscreen {
            rimay_localize::t("mirada-win-kind-fullscreen")
        } else if p.floating {
            rimay_localize::t("mirada-win-kind-floating")
        } else {
            rimay_localize::t("mirada-win-kind-surface")
        };

        let titlebar = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(tb_bg)
        .text_aligned(title, 11.0, tb_fg, Alignment::Start);

        let interior = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        })
        .fill(win_bg)
        .children(vec![
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(app_id, 11.0, theme.fg_placeholder, Alignment::Center),
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(kind_label, 11.0, theme.fg_placeholder, Alignment::Center),
        ]);

        children.push(
            View::new(Style {
                flex_direction: FlexDirection::Column,
                position: Position::Absolute,
                inset: Rect {
                    left: length(p.rect.x as f32 * scale),
                    top: length(p.rect.y as f32 * scale),
                    right: auto(),
                    bottom: auto(),
                },
                size: Size {
                    width: length(p.rect.w as f32 * scale),
                    height: length(p.rect.h as f32 * scale),
                },
                padding: Rect {
                    left: length(2.0_f32),
                    right: length(2.0_f32),
                    top: length(2.0_f32),
                    bottom: length(2.0_f32),
                },
                ..Default::default()
            })
            .fill(border)
            .radius(5.0)
            .on_click(Msg::FocusWindow(p.id))
            .children(vec![titlebar, interior]),
        );
    }

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: length(SCREEN_W as f32),
            height: length(SCREEN_H as f32),
        },
        ..Default::default()
    })
    .fill(canvas_bg)
    .children(children)
}

fn main() {
    rimay_localize::init();
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    llimphi_ui::run::<Mirada>();
}
