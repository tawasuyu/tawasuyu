//! `mirada-llimphi` — la ventana del Cerebro del compositor.
//!
//! Es el "Cerebro" de la arquitectura mirada hecho app Llimphi: envuelve
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
use llimphi_icons::Icon;
use llimphi_motion::{animate, motion, Tween};
use llimphi_widget_empty::{empty_view, EmptyPalette};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, Dimension, FlexDirection, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use mirada_brain::{
    BodyEvent, BrainCommand, CtlConn, CtlReply, CtlRequest, CtlServer, Desktop, DesktopAction,
    Keymap, KeymapProfiles, KeymapWatch, LayoutMode, Rules, Vista, WindowId, WindowPlacement,
};
use mirada_link::BrainLink;

use mirada_app_llimphi::overview::{overview_view, Camera};

/// Pantalla virtual del modo simulación — coincide con el lienzo.
const SCREEN_W: i32 = 1280;
const SCREEN_H: i32 = 720;
/// Período del sondeo del Cuerpo, en ms (~60 Hz).
const POLL_MS: u64 = 16;

/// Nombres de app ficticios para las ventanas de simulación.
const APPS: &[&str] = &[
    "shuma", "pluma_app", "revista", "cosmobiología", "matilda", "pluma_notebook_app", "barra",
];

/// Estado de la **vista espacial** (el "Prezi" de mirada): el zoom-out que
/// muestra todos los escritorios como mosaicos para saltar entre ellos. Vive
/// sólo mientras la vista está abierta (`Model::overview = Some`).
struct OverviewState {
    /// Cámara: progreso `0..1`. `1` = grilla completa visible; `0` = la celda
    /// [`focus`](Self::focus) llena la pantalla. Al **abrir** va `0 → 1`
    /// (zoom-out desde el escritorio activo); al **elegir** un destino va
    /// `valor → 0` con `focus = destino` (zoom-in que aterriza en él).
    zoom: Tween<f32>,
    /// La celda sobre la que se centra la cámara (origen del zoom).
    focus: usize,
    /// Destino elegido por click/tecla: cuando el zoom-in termina, se hace
    /// `SwitchWorkspace(destino)` y se cierra la vista. `None` mientras la
    /// vista está abierta sin elección.
    landing: Option<usize>,
}

struct Model {
    theme: Theme,
    desktop: Desktop,
    /// Vista espacial abierta, o `None` (el escritorio normal). Ver
    /// [`OverviewState`].
    overview: Option<OverviewState>,
    /// Editor de geometría del Prezi activo (tecla `g` en el overview): las
    /// flechas mueven el escritorio [`overview_sel`](Self::overview_sel).
    overview_edit: bool,
    /// Escritorio seleccionado en el editor de geometría (0-based).
    overview_sel: usize,
    /// Arrastre en curso en el editor: `(escritorio, dcol acumulado, dfila acum)`.
    overview_drag: Option<(usize, f32, f32)>,
    /// Pedido pendiente de abrir/cerrar la vista espacial vía el atajo global
    /// (`Super+e`) que el Cuerpo nos reenvía como `BodyEvent::Keybind`. Se
    /// procesa en `Msg::Tick`, que tiene el `handle` para animar el zoom.
    pending_overview_toggle: bool,
    /// Sesión de **Win+Tab en Prezi**: el escritorio destino que se resalta
    /// mientras se mantiene Super. `Some(i)` ⇒ la vista espacial está en modo
    /// switcher (abierta por Win+Tab); al soltar Super se salta a `i`. `None` ⇒
    /// no es una sesión de Win+Tab (overview por `Super+e`, o cerrado).
    overview_wintab: Option<usize>,
    /// Win+Tab reenviado: avanzar (`Some(true)`) o retroceder (`Some(false)`) el
    /// destino. Lo procesa `Msg::Tick` (necesita el `handle`).
    pending_overview_step: Option<bool>,
    /// Super se soltó durante un Win+Tab de Prezi (el Cuerpo nos lo avisa con el
    /// keybind sentinela): confirmar el destino resaltado. Lo procesa `Msg::Tick`.
    pending_overview_commit: bool,
    /// Último `(activo, cargas)` empujado al Cuerpo vía `SetWorkspaces` (para el
    /// switcher Win+Tab + slide en modo enlazado). Evita re-enviar sin cambios.
    last_ws_push: Option<(usize, Vec<usize>)>,
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
    /// Biblioteca de perfiles de atajos (dwm/i3/hyprland + propios). El menú
    /// «Atajos» conmuta/duplica/crea/borra; al cambiar el activo se vuelca a
    /// keymap.ron y se aplica al Desktop vivo.
    profiles: KeymapProfiles,
    /// Ruta de la biblioteca de perfiles (`~/.config/mirada/profiles.ron`).
    profiles_path: Option<PathBuf>,
    /// Si está activo, aplicar una vista NO toca la barra de pata (conserva el
    /// `launcher.toml` del usuario). Toggle del menú «Vista». De sesión.
    vista_keep_bar: bool,
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
    /// Ruta de la sesión persistida (`~/.local/share/mirada/session.ron`);
    /// `None` si no se pudo determinar el directorio de datos.
    session_path: Option<PathBuf>,
    /// Última sesión guardada en disco — para escribir sólo cuando la forma
    /// del escritorio cambia, no en cada tick.
    last_session: Option<mirada_brain::DesktopState>,
    /// Vigía de `.git/HEAD` del repo en `MIRADA_GIT_WORKSPACE` (si se configuró):
    /// al cambiar de rama, mirada intercambia la sesión (guarda la actual bajo la
    /// rama vieja, restaura la de la nueva). `None` = feature apagada.
    git_branch: Option<mirada_brain::GitBranchWatch>,
    /// Directorio de sesiones por rama (`…/mirada/sessions/`). `None` si no se
    /// pudo determinar el directorio de datos.
    sessions_dir: Option<PathBuf>,
}

#[derive(Clone)]
enum Msg {
    /// Tick periódico: drena el Cuerpo, vigila el keymap, atiende ctl.
    Tick,
    /// Tecla recibida desde la ventana de simulación.
    Key(KeyEvent),
    /// Click en un pip de escritorio.
    SwitchWorkspace(usize),
    /// Abre/cierra la vista espacial (tecla `e`, menú Ver, o cerrar con Esc).
    ToggleOverview,
    /// Re-render durante el vuelo de cámara de la vista espacial; al terminar
    /// el aterrizaje, salta al escritorio destino y cierra la vista.
    OverviewTick,
    /// Elige un escritorio en la vista espacial: arranca el zoom-in hacia él.
    OverviewPick(usize),
    /// Entra/sale del editor de geometría del Prezi (tecla `g`).
    OverviewEditToggle,
    /// Selecciona qué escritorio mueven las flechas en el editor (0-based).
    OverviewSelect(usize),
    /// Mueve el escritorio seleccionado `(dcol, dfila)` en la geometría 2D y
    /// guarda la config (el compositor la hot-reloadea).
    OverviewMove(i32, i32),
    /// Arrastre de una celda en el editor: `(escritorio, fase, dcol, dfila)`
    /// con el delta EN CELDAS. Acumula durante el drag y aplica al soltar.
    OverviewDrag(usize, llimphi_ui::DragPhase, f32, f32),
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
        // Biblioteca de perfiles de atajos: el activo siembra keymap.ron.
        let profiles_path = KeymapProfiles::default_path();
        let profiles = match &profiles_path {
            Some(p) => KeymapProfiles::load_or_init(p),
            None => KeymapProfiles::default(),
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
        // Carga la config del usuario (~/.config/mirada/config.ron) para que
        // los ajustes del panel de control —incl. la vista espacial y el tema
        // del chrome— manden. El tema del panel sale de `config.theme`.
        let user_config = mirada_brain::Config::default_path()
            .map(|p| mirada_brain::Config::load_or_default(&p))
            .unwrap_or_default();
        let chrome_theme = Theme::by_name(&user_config.theme).unwrap_or_default();
        desktop.set_config(user_config);
        // Restaura la última sesión (modos/ratio/nmaster por escritorio + qué
        // escritorio mostraba cada salida). Después de `set_config`: la sesión
        // guardada manda sobre los parámetros que la config siembra.
        let session_path = mirada_brain::DesktopState::default_path();
        if let Some(p) = &session_path {
            if let Some(state) = mirada_brain::DesktopState::load_if_present(p) {
                desktop.restore(&state);
            }
        }

        // Workspaces por rama de Git: si `MIRADA_GIT_WORKSPACE` apunta a un repo,
        // vigila su `.git/HEAD` para intercambiar la sesión al cambiar de rama.
        // Las sesiones por rama viven junto a la sesión global (`…/sessions/`).
        let sessions_dir = session_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.join("sessions"));
        let git_branch = std::env::var_os("MIRADA_GIT_WORKSPACE")
            .map(PathBuf::from)
            .and_then(|repo| mirada_brain::GitBranchWatch::new(&repo));
        if let Some(w) = &git_branch {
            println!(
                "mirada · workspaces por rama de Git activos (rama actual: {})",
                w.current().unwrap_or("—")
            );
        }

        let mut model = Model {
            theme: chrome_theme,
            desktop,
            overview: None,
            overview_edit: false,
            overview_sel: 0,
            overview_drag: None,
            pending_overview_toggle: false,
            overview_wintab: None,
            pending_overview_step: None,
            pending_overview_commit: false,
            last_ws_push: None,
            placements: Vec::new(),
            next_id: 1,
            link,
            note: rimay_localize::t("success"),
            keymap_path,
            keymap_watch,
            profiles,
            profiles_path,
            vista_keep_bar: false,
            ctl,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            context_menu: None,
            session_path,
            last_session: None,
            git_branch,
            sessions_dir,
        };
        if let Some(link) = model.link.as_mut() {
            let _ = link.send(&with_overview_grab(model.desktop.grab_keys()));
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
        // Con la vista espacial abierta, es modal: Esc/`e` la cierran, un
        // dígito 1..9 aterriza en ese escritorio, y el resto se traga.
        if model.overview.is_some() {
            let editing = model.overview_edit;
            return match &e.key {
                Key::Named(NamedKey::Escape) => Some(Msg::ToggleOverview),
                // En modo editor las flechas MUEVEN el escritorio seleccionado.
                Key::Named(NamedKey::ArrowLeft) if editing => Some(Msg::OverviewMove(-1, 0)),
                Key::Named(NamedKey::ArrowRight) if editing => Some(Msg::OverviewMove(1, 0)),
                Key::Named(NamedKey::ArrowUp) if editing => Some(Msg::OverviewMove(0, -1)),
                Key::Named(NamedKey::ArrowDown) if editing => Some(Msg::OverviewMove(0, 1)),
                Key::Character(s) => {
                    let s = s.to_lowercase();
                    if s == "e" {
                        return Some(Msg::ToggleOverview);
                    }
                    if s == "g" {
                        return Some(Msg::OverviewEditToggle);
                    }
                    match s.bytes().next() {
                        Some(c) if c.is_ascii_digit() && c != b'0' => {
                            let d = (c - b'1') as usize;
                            // Editor: el dígito ELIGE qué escritorio mover;
                            // vista normal: aterriza en él.
                            Some(if editing {
                                Msg::OverviewSelect(d)
                            } else {
                                Msg::OverviewPick(d)
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
        }
        // `e` (fuera de un menú) abre la vista espacial.
        if model.menu_open.is_none() {
            if let Key::Character(s) = &e.key {
                if s.eq_ignore_ascii_case("e") {
                    return Some(Msg::ToggleOverview);
                }
            }
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
            Msg::Tick => {
                tick(&mut m);
                // El atajo global del overview llega como `Keybind` y `feed` lo
                // marca pendiente; lo ejecutamos acá, con `handle` para el zoom.
                if m.pending_overview_toggle {
                    m.pending_overview_toggle = false;
                    toggle_overview(&mut m, handle);
                }
                // Win+Tab en Prezi: el Cuerpo reenvía el paso (al pulsar) y el
                // commit (al soltar Super). Se procesan acá por el `handle`.
                if let Some(forward) = m.pending_overview_step.take() {
                    overview_step(&mut m, forward, handle);
                }
                if m.pending_overview_commit {
                    m.pending_overview_commit = false;
                    overview_commit(&mut m, handle);
                }
            }
            Msg::Key(ev) => handle_key(&mut m, &ev),
            Msg::SwitchWorkspace(i) => act(&mut m, DesktopAction::SwitchWorkspace(i)),
            Msg::FocusWindow(id) => act(&mut m, DesktopAction::FocusWindow(id)),
            Msg::ToggleOverview => toggle_overview(&mut m, handle),
            Msg::OverviewPick(target) => overview_pick(&mut m, target, handle),
            Msg::OverviewEditToggle => {
                m.overview_edit = !m.overview_edit;
                if m.overview_edit {
                    m.overview_sel = m.desktop.active_index();
                    m.note = "editor de geometría del Prezi: flechas mueven".into();
                }
            }
            Msg::OverviewSelect(d) => {
                let count = m.desktop.workspace_loads().len().max(1);
                m.overview_sel = d.min(count - 1);
            }
            Msg::OverviewMove(dx, dy) => {
                let count = m.desktop.workspace_loads().len().max(1);
                let sel = m.overview_sel.min(count - 1);
                apply_geometry_move(&mut m, sel, dx, dy);
            }
            Msg::OverviewDrag(i, phase, dcol, drow) => {
                use llimphi_ui::DragPhase;
                match phase {
                    DragPhase::Move => {
                        let (ax, ay) = match m.overview_drag {
                            Some((d, ax, ay)) if d == i => (ax + dcol, ay + drow),
                            _ => (dcol, drow),
                        };
                        m.overview_drag = Some((i, ax, ay));
                        m.overview_sel = i;
                    }
                    DragPhase::End => {
                        if let Some((d, ax, ay)) = m.overview_drag.take() {
                            apply_geometry_move(&mut m, d, ax.round() as i32, ay.round() as i32);
                        }
                    }
                }
            }
            Msg::OverviewTick => {
                // Si el aterrizaje terminó, salta al destino y cierra la vista.
                let landing = m.overview.as_ref().and_then(|ov| {
                    ov.landing.filter(|_| ov.zoom.done())
                });
                if let Some(target) = landing {
                    m.overview = None;
                    act(&mut m, DesktopAction::SwitchWorkspace(target));
                }
            }
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
                        dispatch_menu_cmd(&mut m, &cmd, handle);
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
                dispatch_menu_cmd(&mut m, &cmd, handle);
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

        // --- Lienzo: el escritorio teselado, o la vista espacial -----
        let canvas = match &model.overview {
            Some(ov) => overview_view(
                &model.desktop,
                theme,
                on_accent,
                win_bg,
                canvas_bg,
                Camera { zoom: ov.zoom.value(), focus: ov.focus },
                (SCREEN_W, SCREEN_H),
                Msg::OverviewPick,
                model.overview_edit.then_some(model.overview_sel),
                model.overview_wintab,
                |i, phase, dcol, drow| Msg::OverviewDrag(i, phase, dcol, drow),
            ),
            None => canvas_view(model, theme, on_accent, win_bg, canvas_bg),
        };

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
    swap_session_on_branch_change(m);
    push_workspaces_if_changed(m);
    save_session_if_changed(m);
}

/// Workspaces por rama de Git: si la rama del repo vigilado cambió, guarda la
/// sesión actual bajo la rama que se deja y restaura la de la rama nueva (cada
/// rama es un escritorio guardado: modos/ratio por workspace + `home` por
/// `app_id`). Sólo con Cuerpo conectado (no pisar la sesión real desde una
/// simulación). El respawn de las ventanas concretas de cada rama es Fase 1.bis;
/// aquí se intercambia la **forma** del escritorio, no los procesos.
fn swap_session_on_branch_change(m: &mut Model) {
    if m.link.is_none() {
        return;
    }
    let Some(dir) = m.sessions_dir.clone() else {
        return;
    };
    // Acota el préstamo del vigía a la consulta; después se opera sobre `desktop`.
    let switch = {
        let Some(watch) = m.git_branch.as_mut() else {
            return;
        };
        match watch.poll() {
            Some(s) => s,
            None => return,
        }
    };
    // Guarda la sesión actual bajo la rama que se deja.
    if let Some(from) = &switch.from {
        let _ = std::fs::create_dir_all(&dir);
        let p = mirada_brain::git_branch::branch_session_path(&dir, from);
        if let Err(e) = m.desktop.snapshot().save(&p) {
            eprintln!("mirada · no pude guardar la sesión de «{from}»: {e}");
        }
    }
    // Restaura la sesión de la rama nueva (si existe; si no, el escritorio queda
    // como está y se guardará al primer cambio bajo esa rama).
    let p = mirada_brain::git_branch::branch_session_path(&dir, &switch.to);
    if let Some(state) = mirada_brain::DesktopState::load_if_present(&p) {
        m.desktop.restore(&state);
        // No dejar que `save_session_if_changed` pise lo recién restaurado.
        m.last_session = Some(m.desktop.snapshot());
    }
    println!(
        "mirada · rama {:?} → «{}»: sesión intercambiada",
        switch.from, switch.to
    );
}

/// Empuja al Cuerpo el estado de escritorios (`SetWorkspaces`) para que su
/// switcher Win+Tab —HUD + slide de transición— funcione en modo enlazado,
/// donde el compositor no tiene el `Desktop` local. Sólo con Cuerpo conectado y
/// sólo si cambió (activo/cargas). El `slide_ms` traduce el modo: `0` en
/// `Direct` (salto seco), la duración configurada en `Hyprland`/`Prezi`.
fn push_workspaces_if_changed(m: &mut Model) {
    if m.link.is_none() {
        return;
    }
    let active = m.desktop.active_index();
    let loads = m.desktop.workspace_loads();
    if m.last_ws_push.as_ref() == Some(&(active, loads.clone())) {
        return;
    }
    let (slide_ms, switch_mode) = {
        let cfg = m.desktop.config();
        let mode = cfg.workspace_switch_mode;
        // Direct = salto seco (sin duración); el resto anima con la duración
        // configurada. El slug viaja para que el Cuerpo sepa el modo REAL
        // (Cube/Prezi/Hyprland), no sólo si hay slide.
        let ms = if mode == mirada_brain::WorkspaceSwitchMode::Direct {
            0
        } else {
            cfg.slide_ms
        };
        (ms, mode.slug().to_string())
    };
    let cmd = BrainCommand::SetWorkspaces {
        active: active as u32,
        loads: loads.iter().map(|&n| n as u32).collect(),
        slide_ms,
        switch_mode,
    };
    if let Some(link) = m.link.as_mut() {
        let _ = link.send(&cmd);
    }
    m.last_ws_push = Some((active, loads));
}

/// Persiste la forma del escritorio cuando cambia. Sólo escribe si el snapshot
/// difiere del último guardado (los cambios de layout son escasos, no por tick)
/// y sólo con un Cuerpo conectado — en simulación no queremos pisar la sesión
/// real con la pantalla de muestra.
fn save_session_if_changed(m: &mut Model) {
    if m.link.is_none() {
        return;
    }
    let Some(path) = m.session_path.clone() else {
        return;
    };
    let snap = m.desktop.snapshot();
    if m.last_session.as_ref() == Some(&snap) {
        return;
    }
    if let Err(e) = snap.save(&path) {
        eprintln!("mirada · no pude guardar la sesión: {e}");
    }
    m.last_session = Some(snap);
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

/// Atajo global que abre/cierra la vista espacial (Prezi). El overview es estado
/// de la **app**, no del escritorio, así que no es un `DesktopAction`: lo
/// agregamos a los grabs (ver [`with_overview_grab`]) para que el Cuerpo nos lo
/// reenvíe como `BodyEvent::Keybind` y lo atendemos en [`feed`].
const OVERVIEW_KEYBIND: &str = "Super+e";

/// En modo **Prezi** el Win+Tab (y su reverso) también abren la vista espacial,
/// en vez del switcher+slide. El Cuerpo nos reenvía estos combos como `Keybind`
/// SÓLO cuando el modo configurado es Prezi en sesión enlazada (ver el
/// compositor, `input.rs` arm «Super+Tab»); en los demás modos los maneja él
/// (switcher de celdas + slide). Así Win+Tab hace Prezi cuando Prezi está puesto.
const OVERVIEW_WINTAB: &[&str] = &["Super+Tab", "Super+Shift+Tab"];

/// Combo **sentinela** (no es una tecla real) que el Cuerpo nos reenvía cuando
/// detecta que se soltó Super durante un Win+Tab de Prezi: confirma el destino
/// resaltado. El Cuerpo lo ve el release (la app sólo recibe combos discretos),
/// así que el compositor lo sintetiza. Ver `input.rs`/`sesion.rs` del compositor.
pub const OVERVIEW_WINTAB_COMMIT: &str = "PreziWintabCommit";

/// Aumenta un `BrainCommand::GrabKeys` con los atajos del overview (idempotente):
/// `Super+e` (toggle directo) y `Super+Tab`/`Super+Shift+Tab` (Win+Tab en Prezi).
/// Se aplica en cada envío de grabs al Cuerpo para que sobreviva a recargas de
/// keymap y cambios de perfil.
fn with_overview_grab(cmd: BrainCommand) -> BrainCommand {
    match cmd {
        BrainCommand::GrabKeys(mut keys) => {
            for k in std::iter::once(OVERVIEW_KEYBIND).chain(OVERVIEW_WINTAB.iter().copied()) {
                if !keys.iter().any(|x| x == k) {
                    keys.push(k.to_string());
                }
            }
            BrainCommand::GrabKeys(keys)
        }
        other => other,
    }
}

fn feed(m: &mut Model, event: BodyEvent) {
    match event {
        // El overview (Prezi) es estado de la app, no del escritorio: el Cuerpo
        // nos reenvía su atajo global como `Keybind` (grabeado en
        // `with_overview_grab`). Lo marcamos pendiente y lo procesa `Msg::Tick`
        // (que tiene el `handle` para animar el zoom).
        // `Super+e`: toggle directo de la vista espacial.
        BodyEvent::Keybind(ref combo) if combo == OVERVIEW_KEYBIND => {
            m.pending_overview_toggle = true;
        }
        // Win+Tab en Prezi (reenviado sólo en ese modo): abre/cicla el destino.
        BodyEvent::Keybind(ref combo) if combo == "Super+Tab" => {
            m.pending_overview_step = Some(true);
        }
        BodyEvent::Keybind(ref combo) if combo == "Super+Shift+Tab" => {
            m.pending_overview_step = Some(false);
        }
        // Super se soltó durante el Win+Tab: confirmar el destino.
        BodyEvent::Keybind(ref combo) if combo == OVERVIEW_WINTAB_COMMIT => {
            m.pending_overview_commit = true;
        }
        // Win+Tab confirmó un salto de escritorio en el Cuerpo (modo enlazado):
        // lo aplicamos como una acción de escritorio normal.
        BodyEvent::SwitchWorkspace(i) => act(m, DesktopAction::SwitchWorkspace(i as usize)),
        other => {
            let cmds = m.desktop.on_event(other);
            dispatch(m, cmds);
        }
    }
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
            let cmd = with_overview_grab(m.desktop.set_keymap(km));
            dispatch(m, vec![cmd]);
            m.note = rimay_localize::t("mirada-status-keymap-reloaded");
        }
        Err(e) => m.note = format!("{}: {e}", rimay_localize::t("mirada-status-keymap-invalid")),
    }
}

// --- Perfiles de atajos (menú «Atajos») --------------------------------

/// Aplica el perfil activo: lo persiste (profiles.ron + keymap.ron) e instala
/// su keymap en el Desktop vivo, que reemite el `GrabKeys` al Cuerpo.
fn apply_active_profile(m: &mut Model) {
    if let Some(p) = m.profiles_path.clone() {
        if let Err(e) = m.profiles.save(&p) {
            m.note = format!("perfiles: {e}");
            return;
        }
    }
    if let Some(kp) = m.keymap_path.clone() {
        let _ = m.profiles.write_active_keymap(&kp);
    }
    let km = m.profiles.active_keymap();
    let cmd = with_overview_grab(m.desktop.set_keymap(km));
    dispatch(m, vec![cmd]);
}

/// Conmuta el perfil activo y lo aplica.
fn switch_profile(m: &mut Model, name: &str) {
    match m.profiles.set_active(name) {
        Ok(()) => {
            apply_active_profile(m);
            m.note = format!("perfil de atajos: {name}");
        }
        Err(e) => m.note = e.to_string(),
    }
}

/// Duplica el perfil activo en una copia editable y la deja activa.
fn dup_active_profile(m: &mut Model) {
    let src = m.profiles.active().to_string();
    let name = unique_profile_name(&m.profiles, &format!("{src} copia"));
    let r = m
        .profiles
        .duplicate(&src, &name)
        .and_then(|()| m.profiles.set_active(&name));
    match r {
        Ok(()) => {
            apply_active_profile(m);
            m.note = format!("perfil duplicado: {name}");
        }
        Err(e) => m.note = e.to_string(),
    }
}

/// Crea un perfil nuevo desde un preset de fábrica y lo deja activo.
fn new_profile_from(m: &mut Model, preset: &str) {
    let name = unique_profile_name(&m.profiles, preset);
    let r = m
        .profiles
        .create_from_preset(&name, preset)
        .and_then(|()| m.profiles.set_active(&name));
    match r {
        Ok(()) => {
            apply_active_profile(m);
            m.note = format!("perfil nuevo: {name}");
        }
        Err(e) => m.note = e.to_string(),
    }
}

/// Borra el perfil activo (los de fábrica están protegidos) y cae a `dwm`.
fn delete_active_profile(m: &mut Model) {
    let name = m.profiles.active().to_string();
    match m.profiles.remove(&name) {
        Ok(()) => {
            apply_active_profile(m);
            m.note = format!("perfil borrado: {name} → {}", m.profiles.active());
        }
        Err(e) => m.note = e.to_string(),
    }
}

// --- Vistas (menú «Vista») ---------------------------------------------

/// Aplica una vista por slug; no-op con aviso si no existe.
fn apply_vista_by_name(m: &mut Model, name: &str) {
    match Vista::by_name(name) {
        Some(v) => apply_vista(m, &v),
        None => m.note = format!("vista desconocida: {name}"),
    }
}

/// Aplica un preset de escritorio completo: decoraciones + layout + tema +
/// teclas. Lo instala en vivo (re-decora, re-tesela, re-graba teclas), repinta
/// el chrome del panel con el tema de la vista, y persiste config.ron +
/// keymap.ron (el compositor recarga la config por su FileWatch).
fn apply_vista(m: &mut Model, v: &Vista) {
    // 1. Decoraciones + parámetros de teselado de la vista, y re-tesela con su
    //    layout (reload_config siembra params; SetLayout fuerza el relayout).
    let mut cmds = m.desktop.reload_config(v.config.clone());
    cmds.extend(m.desktop.apply(DesktopAction::SetLayout(v.config.layout)));
    dispatch(m, cmds);
    // 2. Keymap de la vista (un preset de fábrica) como perfil activo — esto ya
    //    persiste profiles.ron + keymap.ron y lo instala en el Desktop vivo.
    let _ = m.profiles.set_active(v.keymap);
    apply_active_profile(m);
    // 3. Tema del chrome del panel.
    m.theme = Theme::by_name(&v.config.theme).unwrap_or_default();
    // 4. Persistir config.ron (el compositor lo recarga por su FileWatch).
    if let Some(p) = mirada_brain::Config::default_path() {
        if let Err(e) = v.config.save(&p) {
            m.note = format!("config: {e}");
            return;
        }
    }
    // 5. Reconfigurar la barra (pata): escribimos su launcher.toml con el preset
    //    de barra de la vista; pata lo recarga en caliente por mtime. El slug de
    //    la vista casa 1:1 con el preset de barra de pata-core. Si el usuario
    //    pidió conservar su barra, no la tocamos.
    if !m.vista_keep_bar {
        if let Some(bar) = pata_core::Config::vista_preset(v.name) {
            if let Err(e) = pata_config::save(&bar) {
                m.note = format!("barra: {e}");
                return;
            }
        }
    }
    m.note = format!("vista: {}", v.label);
}

/// Un nombre de perfil libre: `base`, o `base 2`, `base 3`… si ya existe.
fn unique_profile_name(p: &KeymapProfiles, base: &str) -> String {
    if !p.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|c| !p.contains(c))
        .expect("siempre hay un sufijo libre")
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
        CtlRequest::Workspaces => CtlReply::Workspaces(mirada_brain::WorkspacesState {
            active: m.desktop.active_index() + 1,
            loads: m.desktop.workspace_loads(),
            layout: mirada_brain::layout_slug(m.desktop.active_workspace().params().mode)
                .to_string(),
            on_other_outputs: m.desktop.workspaces_on_other_outputs(),
        }),
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

// ─── Vista espacial (Prezi) ─────────────────────────────────────────

/// Duración del vuelo de cámara de la vista espacial, según la config.
fn overview_anim(m: &Model) -> Duration {
    Duration::from_millis(m.desktop.config().overview_anim_ms as u64)
}

/// Aplica un movimiento `(dx, dy)` celdas al escritorio `desktop` en la
/// geometría 2D del Prezi, lo guarda en config.ron (el compositor la
/// hot-reloadea) y lo aplica en vivo. Lo usan las flechas y el arrastre.
fn apply_geometry_move(m: &mut Model, desktop: usize, dx: i32, dy: i32) {
    if dx == 0 && dy == 0 {
        return;
    }
    let count = m.desktop.workspace_loads().len().max(1);
    let mut cfg = m.desktop.config().clone();
    cfg.overview_geometry = cfg.overview_geometry_moved(count, desktop, dx, dy);
    if let Some(p) = mirada_brain::Config::default_path() {
        if let Err(e) = cfg.save(&p) {
            m.note = format!("no pude guardar la geometría: {e}");
        }
    }
    m.desktop.set_config(cfg);
}

/// Abre la vista espacial (zoom-out desde el escritorio activo) o la cierra si
/// ya estaba abierta. No hace nada si la config la deshabilita.
fn toggle_overview(m: &mut Model, handle: &Handle<Msg>) {
    if m.overview.is_some() {
        m.overview = None;
        m.overview_wintab = None; // cerrar cancela cualquier Win+Tab en curso
        return;
    }
    open_overview(m, handle);
}

/// Abre la vista espacial (sin togglear): zoom-out desde el escritorio activo.
/// No-op si ya está abierta o el overview está deshabilitado en la config.
fn open_overview(m: &mut Model, handle: &Handle<Msg>) {
    if m.overview.is_some() || !m.desktop.config().overview_enabled {
        return;
    }
    let dur = overview_anim(m);
    m.overview = Some(OverviewState {
        zoom: Tween::new(0.0, 1.0, dur, motion::ease_out_cubic),
        focus: m.desktop.active_index(),
        landing: None,
    });
    animate(handle, dur, || Msg::OverviewTick);
}

/// Win+Tab en Prezi: abre la vista espacial (si hace falta) y mueve el destino
/// resaltado al siguiente escritorio OCUPADO (`forward`) o al anterior. El salto
/// real ocurre al soltar Super ([`overview_commit`]). Como cualquier alt-tab, el
/// primer paso ya apunta al SIGUIENTE escritorio (no al actual).
fn overview_step(m: &mut Model, forward: bool, handle: &Handle<Msg>) {
    let loads = m.desktop.workspace_loads();
    let occ: Vec<usize> = (0..loads.len()).filter(|&i| loads[i] > 0).collect();
    if occ.is_empty() {
        return;
    }
    let just_opened = m.overview.is_none();
    open_overview(m, handle);
    if m.overview.is_none() {
        return; // overview deshabilitado en config
    }
    // Desde dónde avanzar: el destino actual del Win+Tab, o el activo al abrir.
    let from = match m.overview_wintab {
        Some(t) if !just_opened => t,
        _ => m.desktop.active_index(),
    };
    m.overview_wintab = Some(paso_ocupado(&occ, from, forward));
}

/// El siguiente escritorio **ocupado** a partir de `from`, ciclando en la
/// dirección dada. `occ` = índices ocupados (no vacío, ascendente). Si `from`
/// no está en `occ`, arranca del primero. Pura → testeable sin `Model`.
fn paso_ocupado(occ: &[usize], from: usize, forward: bool) -> usize {
    let pos = occ.iter().position(|&i| i == from).unwrap_or(0);
    let n = occ.len();
    let next = if forward { (pos + 1) % n } else { (pos + n - 1) % n };
    occ[next]
}

/// Confirma el destino del Win+Tab: vuela y salta a él, cerrando la vista.
/// No-op si no hay un Win+Tab en curso (`overview_wintab == None`).
fn overview_commit(m: &mut Model, handle: &Handle<Msg>) {
    if let Some(target) = m.overview_wintab.take() {
        overview_pick(m, target, handle);
    }
}

/// Elige un escritorio en la vista espacial: arranca el zoom-in hacia su celda.
/// Al terminar (en [`Msg::OverviewTick`]) salta a ese escritorio y cierra.
fn overview_pick(m: &mut Model, target: usize, handle: &Handle<Msg>) {
    if target >= m.desktop.workspace_loads().len() {
        return;
    }
    let dur = overview_anim(m);
    if let Some(ov) = m.overview.as_mut() {
        let cur = ov.zoom.value();
        ov.zoom = Tween::new(cur, 0.0, dur, motion::ease_in_out_cubic);
        ov.focus = target;
        ov.landing = Some(target);
    }
    animate(handle, dur, || Msg::OverviewTick);
}

/// Despacha un comando del menú principal, interceptando los que necesitan el
/// `Handle` (animaciones) antes de delegar en [`handle_menu_command`].
fn dispatch_menu_cmd(m: &mut Model, cmd: &str, handle: &Handle<Msg>) {
    match cmd {
        "view.overview" => toggle_overview(m, handle),
        _ => handle_menu_command(m, cmd),
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

    // Menú «Vista»: presets de escritorio completo (look + teclas).
    let vista = vistas_menu(
        model.profiles.active(),
        model.desktop.config(),
        model.vista_keep_bar,
    );
    // Menú «Atajos»: la biblioteca de perfiles de teclas.
    let atajos = profiles_menu(&model.profiles);

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
                .item(MenuItem::new(t("mirada-output-next"), "view.output_next").shortcut("o").separated())
                .item(MenuItem::new(t("mirada-view-overview"), "view.overview").shortcut("e")),
        )
        .menu(
            Menu::new(t("mirada-menu-window"))
                .item(promover)
                .item(flotar)
                .item(fullscreen)
                .item(scratch),
        )
        .menu(vista)
        .menu(atajos)
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

/// El menú «Atajos»: la biblioteca de perfiles de teclas. El perfil activo
/// lleva ✔ y su item conmuta (`profile.use.<nombre>`); abajo, las acciones de
/// gestión. Función pura sobre la biblioteca — verificable sin GPU. Etiquetas
/// literales (los nombres de perfil no se localizan).
fn profiles_menu(profiles: &KeymapProfiles) -> Menu {
    let active = profiles.active();
    let mut menu = Menu::new("Atajos");
    for name in profiles.names() {
        let mut it = MenuItem::new(name.clone(), format!("profile.use.{name}"));
        if name == active {
            it = it.icon("\u{2714}");
        }
        menu = menu.item(it);
    }
    let mut borrar = MenuItem::new("Borrar el activo", "profile.delete").separated();
    if Keymap::is_builtin_name(active) {
        // Los presets de fábrica no se borran: se duplican.
        borrar = borrar.disabled();
    }
    menu.item(MenuItem::new("Duplicar el activo", "profile.dup").separated())
        .item(MenuItem::new("Nuevo desde dwm", "profile.new.dwm"))
        .item(MenuItem::new("Nuevo desde i3", "profile.new.i3"))
        .item(MenuItem::new("Nuevo desde Hyprland", "profile.new.hyprland"))
        .item(borrar)
}

/// El menú «Vista»: presets de escritorio completo (look + decoraciones +
/// layout + teclas). Lleva ✔ la vista cuyo `config` y keymap coinciden EXACTO
/// con el estado actual (si el usuario tuneó algo a mano, ninguna marca).
/// Función pura — verificable sin GPU.
fn vistas_menu(active_keymap: &str, current: &mirada_brain::Config, keep_bar: bool) -> Menu {
    let mut menu = Menu::new("Vista");
    for v in Vista::all() {
        let matches = v.keymap == active_keymap && &v.config == current;
        let mut it = MenuItem::new(v.label, format!("vista.use.{}", v.name));
        if matches {
            it = it.icon("\u{2714}");
        }
        menu = menu.item(it);
    }
    // Toggle: conservar la barra de pata al cambiar de vista (no pisar el TOML).
    let mut keep = MenuItem::new("Conservar mi barra", "vista.keep_bar").separated();
    if keep_bar {
        keep = keep.icon("\u{2714}");
    }
    menu.item(keep)
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
    // Perfiles de atajos: conmutar, crear desde preset, duplicar, borrar.
    if let Some(name) = cmd.strip_prefix("profile.use.") {
        switch_profile(m, &name.to_string());
        return;
    }
    if let Some(preset) = cmd.strip_prefix("profile.new.") {
        new_profile_from(m, &preset.to_string());
        return;
    }
    if cmd == "profile.dup" {
        dup_active_profile(m);
        return;
    }
    if cmd == "profile.delete" {
        delete_active_profile(m);
        return;
    }
    // Vistas: aplicar un preset de escritorio completo.
    if let Some(name) = cmd.strip_prefix("vista.use.") {
        apply_vista_by_name(m, &name.to_string());
        return;
    }
    if cmd == "vista.keep_bar" {
        m.vista_keep_bar = !m.vista_keep_bar;
        m.note = if m.vista_keep_bar {
            "vistas: conservaré tu barra de pata".into()
        } else {
            "vistas: la barra seguirá a la vista".into()
        };
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
    // Chip de zoom-Z: visible sólo con agrupación. El glifo ⧉ marca el árbol
    // fractal; el número es la profundidad de zoom (0 = se ve el espacio
    // entero). Lenguaje-neutro: no toca el catálogo de localización.
    let ws = model.desktop.active_workspace();
    let zoom_chip = ws.is_grouped().then(|| {
        let depth = ws.zoom_depth();
        let (fg, bg) = if depth > 0 {
            (on_accent, theme.accent)
        } else {
            (theme.fg_muted, theme.bg_row_hover)
        };
        View::new(Style {
            size: Size {
                width: length(46.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(bg)
        .radius(4.0)
        .text_aligned(format!("⧉ {depth}"), 12.0, fg, Alignment::Start)
    });

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
    .children({
        let mut kids = vec![mirada_tag, sep_a, pips_row, sep_b, layout_label];
        if let Some(chip) = zoom_chip {
            kids.push(chip);
        }
        kids.push(spacer);
        kids.push(focus_label_node);
        kids
    })
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

    // Estado vacío: sin ventanas visibles, un empty-state con ícono y
    // orientación en vez de un renglón suelto. El hint localizado trae
    // «título — descripción»; lo partimos para las dos líneas del widget.
    let visible = model.placements.iter().filter(|p| p.visible).count();
    if visible == 0 {
        let hint = rimay_localize::t("mirada-canvas-empty-hint");
        let (titulo, desc) = match hint.split_once(" — ") {
            Some((t, d)) => (t.to_string(), Some(d.to_string())),
            None => (hint.clone(), None),
        };
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
            .children(vec![empty_view(
                Icon::Grid,
                titulo,
                desc.as_deref(),
                &EmptyPalette::from_theme(theme),
            )]),
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
            // Pop-in al aparecer la ventana (apertura o salto de escritorio):
            // la baldosa entra con un suave fade+escala en vez de saltar. Key
            // estable por id ⇒ no re-anima en cada relayout/foco.
            .animated_enter(p.id as u64, motion::NORMAL)
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
    bitacora::abrir("mirada");
    rimay_localize::init();
    let _ = rimay_localize::set_locale(&wawa_config::WawaConfig::load().lang);
    llimphi_ui::run::<Mirada>();
}

#[cfg(test)]
mod tests {
    use super::{
        paso_ocupado, profiles_menu, vistas_menu, with_overview_grab, KeymapProfiles, Vista,
        OVERVIEW_KEYBIND, OVERVIEW_WINTAB,
    };
    use mirada_brain::BrainCommand;

    /// El Win+Tab cicla sólo por escritorios ocupados, con wrap, en ambos
    /// sentidos; arrancando del activo apunta al SIGUIENTE (como alt-tab).
    #[test]
    fn paso_ocupado_cicla_con_wrap() {
        let occ = [0usize, 2, 3]; // el 1 está vacío
        // Adelante desde el activo (0) → 2 (saltea el vacío); luego 3; luego wrap a 0.
        assert_eq!(paso_ocupado(&occ, 0, true), 2);
        assert_eq!(paso_ocupado(&occ, 2, true), 3);
        assert_eq!(paso_ocupado(&occ, 3, true), 0);
        // Atrás: 0 → 3 (wrap), 3 → 2, 2 → 0.
        assert_eq!(paso_ocupado(&occ, 0, false), 3);
        assert_eq!(paso_ocupado(&occ, 3, false), 2);
        // `from` en un escritorio vacío (no listado) arranca del primer ocupado.
        assert_eq!(paso_ocupado(&occ, 1, true), 2);
        // Dos escritorios: Win+Tab alterna entre ambos.
        let dos = [0usize, 1];
        assert_eq!(paso_ocupado(&dos, 0, true), 1);
        assert_eq!(paso_ocupado(&dos, 1, true), 0);
    }

    /// `with_overview_grab` agrega Super+e + Win+Tab a los grabs (idempotente) y
    /// preserva los que ya estaban — si no, el Cuerpo no reenviaría Win+Tab y la
    /// vista espacial sería inalcanzable en sesión enlazada.
    #[test]
    fn with_overview_grab_agrega_wintab_idempotente() {
        let base = BrainCommand::GrabKeys(vec!["Super+q".into(), "Super+Tab".into()]);
        let BrainCommand::GrabKeys(keys) = with_overview_grab(base) else {
            panic!("debe seguir siendo GrabKeys");
        };
        assert!(keys.iter().any(|k| k == "Super+q"), "preserva los previos");
        assert!(keys.iter().any(|k| k == OVERVIEW_KEYBIND));
        for k in OVERVIEW_WINTAB {
            assert!(keys.iter().any(|x| x == k), "falta {k}");
        }
        // Idempotente: "Super+Tab" ya estaba, no se duplica.
        assert_eq!(keys.iter().filter(|k| *k == "Super+Tab").count(), 1);
    }

    /// El menú «Vista» lista las 6 vistas y marca con ✔ la que coincide EXACTO
    /// con el estado actual (config + keymap). Con el default nativo, es `mirada`.
    #[test]
    fn el_menu_de_vistas_marca_la_activa() {
        let default_cfg = mirada_brain::Config::default();
        let menu = vistas_menu("mirada", &default_cfg, false);
        // Una entrada por vista + el toggle «Conservar mi barra».
        assert_eq!(menu.items.len(), Vista::all().len() + 1);
        let keep = menu
            .items
            .iter()
            .find(|it| it.command == "vista.keep_bar")
            .unwrap();
        assert!(keep.icon.is_none(), "keep_bar arranca apagado");
        assert!(
            vistas_menu("mirada", &default_cfg, true)
                .items
                .iter()
                .any(|it| it.command == "vista.keep_bar" && it.icon.is_some()),
            "con keep_bar=true el toggle lleva ✔"
        );
        let mirada = menu
            .items
            .iter()
            .find(|it| it.command == "vista.use.mirada")
            .unwrap();
        assert!(mirada.icon.is_some(), "la nativa debe estar marcada con el default");
        // dwm no coincide con el default (sin barra, sin margen).
        let dwm = menu
            .items
            .iter()
            .find(|it| it.command == "vista.use.dwm")
            .unwrap();
        assert!(dwm.icon.is_none());
    }

    /// Con un keymap distinto al de la vista, no se marca aunque la config calce.
    #[test]
    fn la_vista_no_se_marca_si_el_keymap_difiere() {
        let default_cfg = mirada_brain::Config::default();
        let menu = vistas_menu("hyprland", &default_cfg, false); // keymap ≠ "mirada"
        let mirada = menu
            .items
            .iter()
            .find(|it| it.command == "vista.use.mirada")
            .unwrap();
        assert!(mirada.icon.is_none());
    }

    /// El menú «Atajos» lista cada perfil con su comando `profile.use.<n>`,
    /// marca el activo con ✔ y trae las acciones de gestión.
    #[test]
    fn el_menu_de_atajos_refleja_la_biblioteca() {
        let mut profs = KeymapProfiles::default();
        profs.set_active("i3").unwrap();
        let menu = profiles_menu(&profs);

        // Un item por perfil de fábrica, con el comando de conmutación.
        for name in ["dwm", "i3", "hyprland"] {
            let cmd = format!("profile.use.{name}");
            assert!(
                menu.items.iter().any(|it| it.command == cmd),
                "falta el item de conmutación para {name}"
            );
        }
        // El activo (i3) lleva ✔; los demás no.
        let i3 = menu
            .items
            .iter()
            .find(|it| it.command == "profile.use.i3")
            .unwrap();
        assert!(i3.icon.is_some(), "el perfil activo debe llevar ✔");
        let dwm = menu
            .items
            .iter()
            .find(|it| it.command == "profile.use.dwm")
            .unwrap();
        assert!(dwm.icon.is_none(), "un perfil inactivo no lleva ✔");

        // Acciones de gestión presentes.
        for cmd in [
            "profile.dup",
            "profile.new.dwm",
            "profile.new.i3",
            "profile.new.hyprland",
            "profile.delete",
        ] {
            assert!(
                menu.items.iter().any(|it| it.command == cmd),
                "falta la acción {cmd}"
            );
        }
        // Con un preset de fábrica activo, «Borrar» está deshabilitado.
        let borrar = menu
            .items
            .iter()
            .find(|it| it.command == "profile.delete")
            .unwrap();
        assert!(!borrar.enabled, "borrar un preset de fábrica debe estar vedado");
    }

    /// Con un perfil propio activo, «Borrar el activo» se habilita.
    #[test]
    fn borrar_se_habilita_con_un_perfil_propio() {
        let mut profs = KeymapProfiles::default();
        profs.duplicate("hyprland", "mío").unwrap();
        profs.set_active("mío").unwrap();
        let menu = profiles_menu(&profs);
        let borrar = menu
            .items
            .iter()
            .find(|it| it.command == "profile.delete")
            .unwrap();
        assert!(borrar.enabled, "un perfil propio sí se puede borrar");
        // Y aparece en la lista con su ✔.
        let mio = menu
            .items
            .iter()
            .find(|it| it.command == "profile.use.mío")
            .unwrap();
        assert!(mio.icon.is_some());
    }
}
