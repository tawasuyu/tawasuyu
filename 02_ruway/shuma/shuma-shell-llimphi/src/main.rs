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
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use shuma_module::{ModuleContributions, MonitorSpec, ShortcutAction, ShortcutSpec, Source};
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
    Tab(usize),
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

    // Tabs siempre visibles cuando `main` está vacío.
    tabs: Vec<Instance>,
    active_tab: usize,

    // Monitor stack en el panel derecho del área central.
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
}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Tick rápido que drena la salida del shell (~100 ms) sin tocar
    /// el muestreo de sysmon.
    ShellTick,
    /// Click en una tab.
    SelectTab(usize),
    /// Drag del splitter de monitores.
    ResizeMonitors(f32),
    /// Msg de un módulo. El chasis lo enruta a `update` según `slot`.
    Module(Slot, ModuleMsg),
    /// Click en un shortcut de la toolbar. `slot` es el módulo emisor
    /// (a quien se le enruta la `ModuleAction`).
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

        let tabs = if cfg.tabs.is_empty() {
            // Default cuando no hay `[[tabs]]`: shell + lienzo + matilda
            // locales para que el chasis sea exploratorio desde el día
            // uno sin que el usuario tenga que escribir un shumarc. El
            // lienzo se mantiene en sync con el grafo del shell cada
            // `SHELL_TICK` (~100 ms).
            vec![
                Instance::shell(rimay_localize::t("shuma-label-shell"), default_shell_source()),
                Instance::canvas(rimay_localize::t("shuma-label-canvas")),
                Instance::matilda(rimay_localize::t("shuma-label-matilda"), Source::Local),
            ]
        } else {
            cfg.tabs.iter().filter_map(resolve_tab).collect()
        };

        Model {
            theme,
            topbar,
            bottombar,
            main,
            tabs,
            active_tab: 0,
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
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
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
            Msg::SelectTab(i) => {
                if i < m.tabs.len() {
                    m.active_tab = i;
                }
            }
            Msg::ResizeMonitors(dx) => {
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
        menu::overlay(model)
    }
}

// Helpers partidos del monolito (regla dura #1, 1522 LOC): update + view.
mod menu;
mod update;
mod view;

use update::*;
use view::*;
