//! `shuma-shell-llimphi` — chasis del shell shuma sobre Llimphi.
//!
//! **Layout normal (drawer cerrado)** — el shuma es mínimal:
//!
//! ```text
//!  ┌──────────────────────────────────────────────────┐
//!  │ TopBar · launcher (apps + shortcuts)             │
//!  ├──────────────────────────────────────────────────┤
//!  │                                                  │
//!  │  Main module (matilda, editor, etc.)             │
//!  │                                                  │
//!  ├──────────────────────────────────────────────────┤
//!  │ BottomBar · command-bar  › escribí…              │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! **Drawer Quake abierto** (F12 o click en la command bar):
//!
//! ```text
//!  ┌──────────────────────────────────────────────────┐
//!  │ TopBar                                           │
//!  ├──────────────────────────────────────────────────┤
//!  │  Main module (parcialmente tapado)               │
//!  │  ┌────────────────────────────────────────────┐  │
//!  │  │ tabs: [shell] [matilda] [logs] …           │  │
//!  │  ├──────────────────────────┬─────────────────┤  │
//!  │  │ contenido del tab activo │ monitor stack   │  │
//!  │  │ (40% de la ventana,      │ CPU/MEM + los   │  │
//!  │  │  desliza desde abajo)    │ del módulo      │  │
//!  │  └──────────────────────────┴─────────────────┘  │
//!  ├──────────────────────────────────────────────────┤
//!  │ BottomBar · $ ls _                               │
//!  └──────────────────────────────────────────────────┘
//! ```
//!
//! El chasis no conoce a sus módulos: el `Kind` estático enumera los
//! compilados (hoy: launcher / commandbar / shell). El shumarc (bloque
//! 5) elige cuáles activar y en qué slot. El drawer Quake oculta/muestra
//! con F12 (toggle), `Esc` (cerrar), o click en la command bar (abrir).
//! El triger por hover queda pendiente — necesita enter/leave events.

#![forbid(unsafe_code)]

mod config;

use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, PathEl, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{
    App, DragPhase, Handle, Key, KeyEvent, KeyState, NamedKey, PaintRect, View,
};
use llimphi_theme::Theme;
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
use shuma_module::{
    DrawerTrigger, ModuleContributions, MonitorSpec, ShortcutAction, ShortcutSpec, Source,
};
use std::collections::HashMap;
use shuma_sysmon::{Snapshot, SystemSampler};

const HISTORY: usize = 60;
const TICK: Duration = Duration::from_secs(1);
/// Cadencia rápida para drenar el output del shell (streaming de
/// `shuma-exec`). 1 Hz se siente lento al ver `for i in …; do echo $i;
/// sleep 0.1; done`; 100 ms hace la salida sentirse en vivo sin
/// comerse CPU notable.
const SHELL_TICK: Duration = Duration::from_millis(100);
const MONITORS_INITIAL_WIDTH: f32 = 280.0;

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
/// Bottombar/Main) se identifican por sí mismos; el DrawerTab lleva el
/// índice del tab para enrutar al instance correcto.
#[derive(Debug, Clone)]
enum Slot {
    TopBar,
    BottomBar,
    #[allow(dead_code)]
    Main,
    DrawerTab(usize),
}

// ─── Modelo + Msg ───────────────────────────────────────────────────

struct Model {
    theme: Theme,

    // Slots fijos (únicos):
    topbar: Option<Instance>,
    bottombar: Option<Instance>,
    main: Option<Instance>, // placeholder por ahora: None

    // Slot drawer: lista de tabs + estado del overlay.
    drawer_tabs: Vec<Instance>,
    drawer_open: bool,
    active_drawer_tab: usize,
    drawer_trigger: DrawerTrigger,

    // Monitor stack (vive dentro del drawer, panel derecho).
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
}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Tick rápido que drena la salida del shell (~100 ms) sin tocar
    /// el muestreo de sysmon.
    ShellTick,
    /// Toggle del drawer (F12 o click en la command bar).
    ToggleDrawer,
    /// Cierra el drawer (Esc).
    CloseDrawer,
    /// Click en una tab del drawer.
    SelectDrawerTab(usize),
    /// Drag del splitter de monitores en el drawer.
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
        let topbar = resolve_slot(cfg.topbar.as_ref())
            .or_else(|| Some(Instance::launcher(shuma_module_launcher::State::from_apps_dir())));
        let bottombar = resolve_slot(cfg.bottombar.as_ref()).or_else(|| {
            Some(Instance::command_bar(
                shuma_module_commandbar::State::default(),
            ))
        });
        let main = resolve_slot(cfg.main.as_ref());

        let drawer_tabs = if cfg.drawer.tabs.is_empty() {
            // Default cuando no hay `[[drawer.tabs]]`: shell + lienzo +
            // matilda locales para que el chasis sea exploratorio desde
            // el día uno sin que el usuario tenga que escribir un shumarc.
            // El lienzo se mantiene en sync con el grafo del shell cada
            // `SHELL_TICK` (~100 ms).
            vec![
                Instance::shell(rimay_localize::t("shuma-label-shell"), Source::Local),
                Instance::canvas(rimay_localize::t("shuma-label-canvas")),
                Instance::matilda(rimay_localize::t("shuma-label-matilda"), Source::Local),
            ]
        } else {
            cfg.drawer
                .tabs
                .iter()
                .filter_map(resolve_drawer_tab)
                .collect()
        };

        Model {
            theme,
            topbar,
            bottombar,
            main,
            drawer_tabs,
            drawer_open: false,
            active_drawer_tab: 0,
            drawer_trigger: cfg.drawer.trigger.unwrap_or_default(),
            sysmon: SystemSampler::new(HISTORY),
            last_snapshot: None,
            monitors_width: MONITORS_INITIAL_WIDTH,
            extra_history: HashMap::new(),
            extra_display: HashMap::new(),
            _wawa_watcher: wawa_watcher,
        }
    }

    fn on_key(model: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        // Esc: cerrar el drawer si está abierto. Sin efecto si está
        // cerrado (no consume Esc — la app del Main lo recibe).
        if let Key::Named(NamedKey::Escape) = &e.key {
            if model.drawer_open {
                return Some(Msg::CloseDrawer);
            }
        }
        // Tecla configurada para toggle (default F12). El parser
        // acepta combos `Ctrl+Shift+Space`, `Super+grave`, `Alt+F1`,
        // etc. — sin caso/orden estrictos.
        if let Some(want) = model.drawer_trigger.key.as_deref() {
            if matches_key(want, &e.key, &e.modifiers) {
                return Some(Msg::ToggleDrawer);
            }
        }
        // Reenvía teclas al módulo focado. Hoy sólo el shell consume
        // teclas (input del REPL); el resto de módulos siguen sin
        // recibirlas hasta que las necesiten.
        forward_key_to_focused_shell(model, e)
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
            Msg::ToggleDrawer => {
                m.drawer_open = !m.drawer_open;
            }
            Msg::CloseDrawer => {
                m.drawer_open = false;
            }
            Msg::SelectDrawerTab(i) => {
                if i < m.drawer_tabs.len() {
                    m.active_drawer_tab = i;
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
        }
        m
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let theme = &model.theme;

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
        .children(vec![topbar, main_area, bottombar])
    }
}

/// Enruta un `ModuleMsg` al `update` del módulo correspondiente, y se
/// encarga de interceptar mensajes que el chasis quiera promocionar
/// (p. ej. el click en la command bar abre el drawer).
fn apply_module_msg(mut m: Model, slot: Slot, msg: ModuleMsg) -> Model {
    // Hook: click en la command bar (que llega como `ToggleMode`) abre
    // el drawer si está cerrado. Si ya está abierto, deja que el módulo
    // togglee su modo libremente.
    if let (Slot::BottomBar, ModuleMsg::CommandBar(shuma_module_commandbar::Msg::ToggleMode)) =
        (&slot, &msg)
    {
        if !m.drawer_open {
            m.drawer_open = true;
            // El toggle del modo NO se aplica en este caso — un click
            // "abre el drawer" y nada más; el cambio de modo lo hace
            // la tecla dedicada (bloque 5) o un click subsiguiente.
            return m;
        }
    }

    // Hook: el `shuma-module-canvas` pide insertar una referencia
    // `%cN`/`%pN` en el input del shell. Buscamos la primera instancia
    // `Shell` (en el mismo orden que `sync_canvas_from_primary_shell`)
    // y le mandamos `InsertAtCursor`. Si la shell vive en una tab del
    // drawer, abrimos el drawer y enfocamos esa tab. La variante NO
    // se propaga al canvas — el canvas solo emite la intención.
    if let ModuleMsg::Canvas(shuma_module_canvas::Msg::InsertRef(text)) = &msg {
        if let Some(target) = first_shell_slot(&m) {
            let insert_msg = ModuleMsg::Shell(shuma_module_shell::Msg::InsertAtCursor(
                text.clone(),
            ));
            if let Slot::DrawerTab(i) = &target {
                m.active_drawer_tab = *i;
                m.drawer_open = true;
            }
            return apply_module_msg(m, target, insert_msg);
        }
        // Sin shell activo: el pedido se descarta silencioso.
        return m;
    }

    match slot {
        Slot::TopBar => {
            if let Some(inst) = m.topbar.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::BottomBar => {
            if let Some(inst) = m.bottombar.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::Main => {
            if let Some(inst) = m.main.as_mut() {
                route_to_instance(inst, msg);
            }
        }
        Slot::DrawerTab(idx) => {
            if let Some(inst) = m.drawer_tabs.get_mut(idx) {
                route_to_instance(inst, msg);
            }
        }
    }
    m
}

/// Mapea una entrada genérica `SlotEntry` del shumarc a una `Instance`.
/// `None` si el `module` no matchea ningún `Kind` compilado — se
/// imprime warning en lugar de fallar para no romper el arranque.
fn resolve_slot(entry: Option<&config::SlotEntry>) -> Option<Instance> {
    let entry = entry?;
    resolve_instance(
        &entry.module,
        entry.source.clone(),
        entry.label.clone(),
        entry.inventory.as_deref(),
    )
}

fn resolve_drawer_tab(entry: &config::DrawerTabEntry) -> Option<Instance> {
    resolve_instance(
        &entry.id,
        entry.source.clone(),
        entry.label.clone(),
        entry.inventory.as_deref(),
    )
}

fn resolve_instance(
    id: &str,
    source: Source,
    label: Option<String>,
    inventory: Option<&std::path::Path>,
) -> Option<Instance> {
    let label = label.unwrap_or_else(|| source.label());
    match id {
        shuma_module_launcher::ID => {
            Some(Instance::launcher(shuma_module_launcher::State::from_apps_dir()))
        }
        shuma_module_commandbar::ID => Some(Instance::command_bar(
            shuma_module_commandbar::State::default(),
        )),
        shuma_module_shell::ID => Some(Instance::shell(label, source)),
        shuma_module_matilda::ID => Some(Instance::matilda_with_inventory(
            label, source, inventory,
        )),
        shuma_module_minga::ID => Some(Instance::minga(label, source)),
        shuma_module_canvas::ID => Some(Instance::canvas(label)),
        unknown => {
            eprintln!("shuma: módulo desconocido «{unknown}» — se ignora");
            None
        }
    }
}

/// Fallback al inventario de ejemplo cuando el path declarado falla
/// — replica el default de `State::new` sin perder el path para reloads.
fn example_inventory_fallback() -> matilda_core::Inventory {
    shuma_module_matilda::example_inventory()
}

/// Lee un inventario JSON desde un path. Errores van a stderr y la
/// función retorna `None` — el chasis cae al ejemplo en lugar de
/// fallar el arranque (mismo criterio que el config TOML malformado).
fn load_matilda_inventory(path: &std::path::Path) -> Option<matilda_core::Inventory> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "shuma: no se pudo leer inventario {} ({e}) — uso ejemplo",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_str::<matilda_core::Inventory>(&text) {
        Ok(inv) => Some(inv),
        Err(e) => {
            eprintln!(
                "shuma: inventario {} mal formado ({e}) — uso ejemplo",
                path.display()
            );
            None
        }
    }
}

/// Recolecta las `ModuleContributions` de todas las instancias vivas.
/// Devuelve un `Vec<(Slot, ModuleContributions)>` para que el caller
/// sepa de qué módulo viene cada monitor/shortcut.
fn collect_contributions(model: &Model) -> Vec<(Slot, ModuleContributions)> {
    let mut out: Vec<(Slot, ModuleContributions)> = Vec::new();

    let push = |out: &mut Vec<(Slot, ModuleContributions)>, slot: Slot, inst: &Instance| {
        let c = match &inst.state {
            ModuleState::Launcher(s) => shuma_module_launcher::contributions(s),
            ModuleState::CommandBar(s) => shuma_module_commandbar::contributions(s),
            ModuleState::Shell(s) => shuma_module_shell::contributions(s),
            ModuleState::Matilda(s) => shuma_module_matilda::contributions(s),
            ModuleState::Minga(s) => shuma_module_minga::contributions(s),
            ModuleState::Canvas(s) => shuma_module_canvas::contributions(s),
        };
        out.push((slot, c));
    };

    if let Some(inst) = &model.topbar {
        push(&mut out, Slot::TopBar, inst);
    }
    if let Some(inst) = &model.bottombar {
        push(&mut out, Slot::BottomBar, inst);
    }
    if let Some(inst) = &model.main {
        push(&mut out, Slot::Main, inst);
    }
    for (i, inst) in model.drawer_tabs.iter().enumerate() {
        push(&mut out, Slot::DrawerTab(i), inst);
    }
    out
}

/// Muestrea **todos** los monitores extra (los aporta cada módulo
/// activo) e inserta el último valor en su buffer del modelo.
/// Recorta cada buffer a `HISTORY` muestras.
fn sample_extra_monitors(m: &mut Model) {
    let contribs = collect_contributions(m);
    for (slot, c) in contribs {
        for spec in &c.monitors {
            let key = monitor_key(&slot, spec);
            let sample = (spec.sampler)();
            let entry = m.extra_history.entry(key.clone()).or_default();
            entry.push(sample.value);
            if entry.len() > HISTORY {
                let excess = entry.len() - HISTORY;
                entry.drain(0..excess);
            }
            m.extra_display.insert(key, sample.display);
        }
    }
}

/// Aplica `Msg::Tick` a cada `Instance` de tipo `Shell` activa para que
/// drene la salida streamed de `shuma-exec`. Llamado a cadencia rápida
/// (`SHELL_TICK`) sin tocar el muestreo de sysmon (`TICK`).
///
/// Después de drenar, sincroniza el `intent_graph` de la primera shell
/// encontrada hacia todas las instancias `Canvas` activas — el lienzo
/// de contexto refleja en tiempo real los `%cN`/`%pN` del shell.
fn drain_shell_instances(m: &mut Model) {
    fn tick_one(inst: &mut Instance) {
        if let ModuleState::Shell(s) = &mut inst.state {
            *s = shuma_module_shell::update(s.clone(), shuma_module_shell::Msg::Tick);
        }
    }
    if let Some(inst) = m.topbar.as_mut() {
        tick_one(inst);
    }
    if let Some(inst) = m.bottombar.as_mut() {
        tick_one(inst);
    }
    if let Some(inst) = m.main.as_mut() {
        tick_one(inst);
    }
    for inst in m.drawer_tabs.iter_mut() {
        tick_one(inst);
    }
    sync_canvas_from_primary_shell(m);
}

/// Toma el `intent_graph` de la primera instancia `Shell` encontrada
/// (en orden: topbar, bottombar, main, drawer tabs) y lo empuja a cada
/// instancia `Canvas` activa vía `Msg::SyncGraph`. Si no hay shells, el
/// canvas mantiene lo último que tenía (incluyendo su grafo de demo).
fn sync_canvas_from_primary_shell(m: &mut Model) {
    let snapshot = find_primary_shell_graph(m);
    let Some(graph) = snapshot else { return };
    let sync_one = |inst: &mut Instance| {
        if let ModuleState::Canvas(s) = &mut inst.state {
            *s = shuma_module_canvas::update(
                s.clone(),
                shuma_module_canvas::Msg::SyncGraph(graph.clone()),
            );
        }
    };
    if let Some(inst) = m.topbar.as_mut() {
        sync_one(inst);
    }
    if let Some(inst) = m.bottombar.as_mut() {
        sync_one(inst);
    }
    if let Some(inst) = m.main.as_mut() {
        sync_one(inst);
    }
    for inst in m.drawer_tabs.iter_mut() {
        sync_one(inst);
    }
}

/// Slot del primer `Shell` activo siguiendo el mismo orden que
/// `find_primary_shell_graph`. Lo usa el hook de `Msg::Canvas(InsertRef)`
/// para encontrar a quién enrutarle el `InsertAtCursor`.
fn first_shell_slot(m: &Model) -> Option<Slot> {
    if matches!(
        m.topbar.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::TopBar);
    }
    if matches!(
        m.bottombar.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::BottomBar);
    }
    if matches!(
        m.main.as_ref().map(|i| &i.state),
        Some(ModuleState::Shell(_))
    ) {
        return Some(Slot::Main);
    }
    m.drawer_tabs.iter().enumerate().find_map(|(i, inst)| {
        if matches!(inst.state, ModuleState::Shell(_)) {
            Some(Slot::DrawerTab(i))
        } else {
            None
        }
    })
}

fn find_primary_shell_graph(m: &Model) -> Option<shuma_intent::SessionGraph> {
    let pick = |inst: &Instance| match &inst.state {
        ModuleState::Shell(s) => Some(s.intent_graph().clone()),
        _ => None,
    };
    if let Some(inst) = m.topbar.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    if let Some(inst) = m.bottombar.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    if let Some(inst) = m.main.as_ref() {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    for inst in &m.drawer_tabs {
        if let Some(g) = pick(inst) {
            return Some(g);
        }
    }
    None
}

fn monitor_key(slot: &Slot, spec: &MonitorSpec) -> String {
    let slot_label = match slot {
        Slot::TopBar => "topbar",
        Slot::BottomBar => "bottombar",
        Slot::Main => "main",
        Slot::DrawerTab(i) => return format!("drawer:{i}/{}", spec.id),
    };
    format!("{slot_label}/{}", spec.id)
}

/// Resuelve un `ShortcutClicked` en una transición concreta del
/// modelo. Las tres variantes:
///
/// - `Command(line)` — por ahora, sólo se loguea en el log de Matilda
///   si está disponible; la ejecución real va con la integración del
///   REPL.
/// - `FocusTab(target)` — busca un `DrawerTab` con `Kind::id() == target`
///   y lo enfoca. Si el drawer está cerrado, también lo abre.
/// - `ModuleAction(action_id)` — dispatcha al módulo emisor vía su
///   `dispatch(action_id) -> Option<Msg>`.
fn handle_shortcut(
    mut m: Model,
    slot: Slot,
    action: ShortcutAction,
    handle: &Handle<Msg>,
) -> Model {
    match action {
        ShortcutAction::Command { line } => {
            // Hack temporario: lo agregamos al log del primer matilda
            // que encontremos para que el usuario vea feedback.
            if let Some(inst) = m
                .drawer_tabs
                .iter_mut()
                .find(|i| matches!(i.state, ModuleState::Matilda(_)))
            {
                if let ModuleState::Matilda(s) = &mut inst.state {
                    s.log.push(format!("? command: {line}"));
                }
            }
        }
        ShortcutAction::FocusTab { target } => {
            if let Some(i) = m
                .drawer_tabs
                .iter()
                .position(|inst| inst.kind.id() == target)
            {
                m.active_drawer_tab = i;
                m.drawer_open = true;
            }
        }
        ShortcutAction::ModuleAction { action_id } => {
            // Reload del inventario: el path lo lleva el State del
            // módulo (cargado por el chasis al construir la instancia
            // desde el shumarc). Sirve para Local y Remote por igual.
            if action_id == "matilda.reload" {
                if let Some(path) = matilda_inventory_path(&slot, &m) {
                    let mmsg = match load_matilda_inventory(&path) {
                        Some(inv) => shuma_module_matilda::Msg::SetDesired(inv),
                        None => shuma_module_matilda::Msg::LogLine(format!(
                            "✘ reload: ver stderr ({})",
                            path.display()
                        )),
                    };
                    return apply_module_msg(m, slot, ModuleMsg::Matilda(mmsg));
                } else {
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(
                            "✘ sin inventory_path: agregá `inventory = …` al shumarc".into(),
                        )),
                    );
                }
            }
            // Hooks remotos: ciertas acciones de matilda necesitan
            // SSH + tokio. Las delegamos a un thread (`Handle::spawn`)
            // que al volver dispatcha un Msg con el resultado.
            if let Some((source, desired)) = remote_matilda_inputs(&slot, &m) {
                if action_id == "matilda.discover" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ conectando a {} para discover…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg = match shuma_module_matilda::discover_remote_blocking(
                            &source, &desired,
                        ) {
                            Ok(inv) => shuma_module_matilda::Msg::SetCurrent(inv),
                            Err(e) => shuma_module_matilda::Msg::LogLine(format!(
                                "✘ discover remoto: {e}"
                            )),
                        };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
                if action_id == "matilda.dry_run" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ dry-run remoto en {} (sin tocar nada)…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg = match shuma_module_matilda::dry_run_remote_blocking(
                            &source, &desired,
                        ) {
                            Ok(lines) => shuma_module_matilda::Msg::DryRunReport(lines),
                            Err(e) => shuma_module_matilda::Msg::LogLine(format!(
                                "✘ dry-run remoto: {e}"
                            )),
                        };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
                if action_id == "matilda.apply" {
                    m = apply_module_msg(
                        m,
                        slot.clone(),
                        ModuleMsg::Matilda(shuma_module_matilda::Msg::LogLine(format!(
                            "→ apply remoto en {} por SSH…",
                            source.label()
                        ))),
                    );
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let msg = match shuma_module_matilda::apply_remote_blocking(
                            &source, &desired,
                        ) {
                            Ok((lines, new_current)) => {
                                shuma_module_matilda::Msg::ApplyReport { lines, new_current }
                            }
                            Err(e) => shuma_module_matilda::Msg::LogLine(format!(
                                "✘ apply remoto: {e}"
                            )),
                        };
                        Msg::Module(slot_back, ModuleMsg::Matilda(msg))
                    });
                    return m;
                }
            }
            // Minga refresh: el módulo es "declarativo" en update (no
            // toca sled) — el load real lo hacemos acá en un thread y
            // reenviamos el snapshot como SnapshotReady.
            if action_id == "minga.refresh" {
                if let Some(repo_path) = minga_repo_path(&slot, &m) {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let result = shuma_module_minga::load_snapshot(&repo_path);
                        Msg::Module(
                            slot_back,
                            ModuleMsg::Minga(shuma_module_minga::Msg::SnapshotReady(result)),
                        )
                    });
                    // Y también marcar el state como "refreshing".
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Minga(shuma_module_minga::Msg::Refresh),
                    );
                }
            }
            // Minga verify_all: recorre las raíces del snapshot y las
            // verifica una por una en un thread.
            if action_id == "minga.verify_all" {
                if let (Some(repo_path), Some(alphas)) = (
                    minga_repo_path(&slot, &m),
                    minga_visible_alphas(&slot, &m),
                ) {
                    let slot_back = slot.clone();
                    handle.spawn(move || {
                        let results =
                            shuma_module_minga::verify_all_blocking(&repo_path, &alphas);
                        Msg::Module(
                            slot_back,
                            ModuleMsg::Minga(shuma_module_minga::Msg::VerifyAllReady(results)),
                        )
                    });
                    return apply_module_msg(
                        m,
                        slot,
                        ModuleMsg::Minga(shuma_module_minga::Msg::VerifyAll),
                    );
                }
            }
            let msg = dispatch_to_module(&slot, &m, action_id);
            if let Some(mmsg) = msg {
                m = apply_module_msg(m, slot, mmsg);
            }
        }
    }
    m
}

/// Path del repo Minga de un slot que aloje el módulo minga.
fn minga_repo_path(slot: &Slot, model: &Model) -> Option<std::path::PathBuf> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::DrawerTab(i) => model.drawer_tabs.get(*i)?,
    };
    match &inst.state {
        ModuleState::Minga(s) => Some(s.repo_path.clone()),
        _ => None,
    }
}

/// Lista de α-hashes de las raíces actualmente visibles en el snapshot
/// del módulo minga. `None` si el slot no es minga o no tiene snapshot
/// cargado todavía.
fn minga_visible_alphas(
    slot: &Slot,
    model: &Model,
) -> Option<Vec<minga_core::ContentHash>> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::DrawerTab(i) => model.drawer_tabs.get(*i)?,
    };
    match &inst.state {
        ModuleState::Minga(s) => s.snapshot.as_ref().map(|snap| {
            snap.recent.iter().map(|r| r.alpha).collect()
        }),
        _ => None,
    }
}

/// Si la instancia focada (drawer tab activo, o Main) es un shell,
/// genera el `Msg::Module` que reenvía la tecla. El módulo shell
/// distingue Enter (submit) de inserción de texto internamente.
fn forward_key_to_focused_shell(model: &Model, e: &KeyEvent) -> Option<Msg> {
    // 1) Drawer tab activo, si el drawer está abierto.
    if model.drawer_open {
        if let Some(inst) = model.drawer_tabs.get(model.active_drawer_tab) {
            if matches!(inst.state, ModuleState::Shell(_)) {
                return Some(Msg::Module(
                    Slot::DrawerTab(model.active_drawer_tab),
                    ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
                ));
            }
        }
    }
    // 2) Slot Main, si tiene un shell activo. Permite al usuario poner
    //    el shell como módulo principal de la ventana sin drawer.
    if let Some(inst) = model.main.as_ref() {
        if matches!(inst.state, ModuleState::Shell(_)) {
            return Some(Msg::Module(
                Slot::Main,
                ModuleMsg::Shell(shuma_module_shell::Msg::Key(e.clone())),
            ));
        }
    }
    None
}

/// Path del inventario JSON de un slot de matilda, si lo tiene cargado.
fn matilda_inventory_path(slot: &Slot, model: &Model) -> Option<std::path::PathBuf> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::DrawerTab(i) => model.drawer_tabs.get(*i)?,
    };
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    state.inventory_path.clone()
}

/// Si `slot` contiene una instancia de `matilda` y su `source` es
/// `Remote`, retorna `(source, desired)` clonados para que el thread
/// SSH los consuma sin tomar prestado del modelo.
fn remote_matilda_inputs(
    slot: &Slot,
    model: &Model,
) -> Option<(Source, matilda_core::Inventory)> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::DrawerTab(i) => model.drawer_tabs.get(*i)?,
    };
    let state = match &inst.state {
        ModuleState::Matilda(s) => s.as_ref(),
        _ => return None,
    };
    if state.source.is_remote() {
        Some((state.source.clone(), state.desired.clone()))
    } else {
        None
    }
}

fn dispatch_to_module(slot: &Slot, model: &Model, action_id: &str) -> Option<ModuleMsg> {
    let inst = match slot {
        Slot::TopBar => model.topbar.as_ref()?,
        Slot::BottomBar => model.bottombar.as_ref()?,
        Slot::Main => model.main.as_ref()?,
        Slot::DrawerTab(i) => model.drawer_tabs.get(*i)?,
    };
    match inst.kind {
        Kind::Launcher => shuma_module_launcher::dispatch(action_id).map(ModuleMsg::Launcher),
        Kind::CommandBar => {
            shuma_module_commandbar::dispatch(action_id).map(ModuleMsg::CommandBar)
        }
        Kind::Shell => shuma_module_shell::dispatch(action_id).map(ModuleMsg::Shell),
        Kind::Matilda => shuma_module_matilda::dispatch(action_id).map(ModuleMsg::Matilda),
        Kind::Minga => shuma_module_minga::dispatch(action_id).map(ModuleMsg::Minga),
        Kind::Canvas => shuma_module_canvas::dispatch(action_id).map(ModuleMsg::Canvas),
    }
}

fn route_to_instance(inst: &mut Instance, msg: ModuleMsg) {
    match (&mut inst.state, msg) {
        (ModuleState::Launcher(s), ModuleMsg::Launcher(m)) => {
            *s = shuma_module_launcher::update(s.clone(), m);
        }
        (ModuleState::CommandBar(s), ModuleMsg::CommandBar(m)) => {
            *s = shuma_module_commandbar::update(s.clone(), m);
        }
        (ModuleState::Shell(s), ModuleMsg::Shell(m)) => {
            *s = shuma_module_shell::update(s.clone(), m);
        }
        (ModuleState::Matilda(s), ModuleMsg::Matilda(m)) => {
            **s = shuma_module_matilda::update((**s).clone(), m);
        }
        (ModuleState::Minga(s), ModuleMsg::Minga(m)) => {
            *s = shuma_module_minga::update(s.clone(), m);
        }
        (ModuleState::Canvas(s), ModuleMsg::Canvas(m)) => {
            *s = shuma_module_canvas::update(s.clone(), m);
        }
        // Combinación inconsistente (state ≠ msg kind): no hace nada.
        // El registry no debería emitirlos; si pasa es un bug del chasis.
        _ => {}
    }
}

// ─── Render de cada slot ────────────────────────────────────────────

fn render_topbar(model: &Model, theme: &Theme) -> View<Msg> {
    match &model.topbar {
        Some(inst) => match (inst.kind, &inst.state) {
            (Kind::Launcher, ModuleState::Launcher(state)) => {
                shuma_module_launcher::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::TopBar, ModuleMsg::Launcher(m))
                })
            }
            _ => empty_bar(theme, 40.0),
        },
        None => empty_bar(theme, 40.0),
    }
}

fn render_bottombar(model: &Model, theme: &Theme) -> View<Msg> {
    match &model.bottombar {
        Some(inst) => match (inst.kind, &inst.state) {
            (Kind::CommandBar, ModuleState::CommandBar(state)) => {
                shuma_module_commandbar::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::BottomBar, ModuleMsg::CommandBar(m))
                })
            }
            _ => empty_bar(theme, 28.0),
        },
        None => empty_bar(theme, 28.0),
    }
}

fn empty_bar(theme: &Theme, height: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
}

/// Área central: `position: Relative` para que el drawer pueda
/// posicionarse absoluto desde el bottom. El main module va de fondo,
/// el drawer overlay encima (orden de hijos = orden de pintado).
fn render_main_area(model: &Model, theme: &Theme) -> View<Msg> {
    let main_layer = render_main_layer(model, theme);
    let mut children = vec![main_layer];
    if model.drawer_open {
        children.push(render_drawer_overlay(model, theme));
    }

    View::new(Style {
        position: Position::Relative,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

fn render_main_layer(model: &Model, theme: &Theme) -> View<Msg> {
    let body = match &model.main {
        Some(inst) => match (inst.kind, &inst.state) {
            // Bloque 5 enrutará cada Kind activable como Main desde el
            // shumarc. Shell y Matilda son los compatibles hoy; el
            // resto cae al placeholder.
            (Kind::Shell, ModuleState::Shell(state)) => shuma_module_shell::view::<Msg>(
                state,
                theme,
                |m| Msg::Module(Slot::Main, ModuleMsg::Shell(m)),
            ),
            (Kind::Matilda, ModuleState::Matilda(state)) => {
                shuma_module_matilda::view::<Msg>(state.as_ref(), theme, |m| {
                    Msg::Module(Slot::Main, ModuleMsg::Matilda(m))
                })
            }
            (Kind::Minga, ModuleState::Minga(state)) => {
                shuma_module_minga::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::Main, ModuleMsg::Minga(m))
                })
            }
            (Kind::Canvas, ModuleState::Canvas(state)) => {
                shuma_module_canvas::view::<Msg>(state, theme, |m| {
                    Msg::Module(Slot::Main, ModuleMsg::Canvas(m))
                })
            }
            _ => placeholder(theme, &rimay_localize::t("shuma-empty-main-incompat")),
        },
        None => placeholder(
            theme,
            &format!(
                "{}\n\n{}",
                rimay_localize::t("shuma-empty-no-main"),
                rimay_localize::t("shuma-empty-no-main-hint"),
            ),
        ),
    };

    // Wrap `body` en un View posicionado absoluto para que conviva en
    // el `MainArea` (que es `Position::Relative`) sin chocar contra el
    // drawer overlay.
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![body])
}

/// Drawer Quake: tira de tabs arriba, splitter row con (contenido del
/// tab activo | monitor stack). Posicionado absoluto desde el bottom
/// para deslizar desde abajo. El alto se calcula como
/// `height_fraction * altura del MainArea` — taffy no nos da la altura
/// del padre acá, así que usamos `percent(height_fraction)` sobre el
/// rect del MainArea (el ancestor `Position::Relative`).
fn render_drawer_overlay(model: &Model, theme: &Theme) -> View<Msg> {
    let tabs_palette = TabsPalette::from_theme(theme);
    let splitter_palette = SplitterPalette::from_theme(theme);

    let toolbar = drawer_toolbar(model, theme);
    let content = drawer_tab_content(model, theme);
    let monitors = monitor_stack(model, theme);

    let labels: Vec<String> = model
        .drawer_tabs
        .iter()
        .map(|inst| inst.label.clone())
        .collect();

    let tabs = tabs_view(TabsSpec {
        labels,
        active: model.active_drawer_tab,
        on_select: Msg::SelectDrawerTab,
        content: splitter_two(
            Direction::Row,
            content,
            PaneSize::Flex,
            monitors,
            PaneSize::Fixed(model.monitors_width),
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeMonitors(dx)),
                DragPhase::End => None,
            },
            &splitter_palette,
        ),
        tab_height: 32.0,
        palette: tabs_palette,
        tab_width: None,
    });

    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![toolbar, tabs]);

    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: auto(),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(model.drawer_trigger.height_fraction.clamp(0.1, 0.95)),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![body])
}

/// Toolbar del drawer: pinta los `ShortcutSpec` del tab activo como
/// botones que disparan `Msg::ShortcutClicked`. Si el tab activo no
/// aporta shortcuts, la barra queda vacía (alto 0 — colapsa).
fn drawer_toolbar(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Dimension;
    use llimphi_ui::llimphi_text::Alignment;

    let Some(inst) = model.drawer_tabs.get(model.active_drawer_tab) else {
        return empty_bar(theme, 0.0);
    };
    let slot = Slot::DrawerTab(model.active_drawer_tab);
    let contribs = match &inst.state {
        ModuleState::Launcher(s) => shuma_module_launcher::contributions(s),
        ModuleState::CommandBar(s) => shuma_module_commandbar::contributions(s),
        ModuleState::Shell(s) => shuma_module_shell::contributions(s),
        ModuleState::Matilda(s) => shuma_module_matilda::contributions(s),
        ModuleState::Minga(s) => shuma_module_minga::contributions(s),
        ModuleState::Canvas(s) => shuma_module_canvas::contributions(s),
    };

    if contribs.shortcuts.is_empty() {
        return empty_bar(theme, 0.0);
    }

    let mut buttons: Vec<View<Msg>> = contribs
        .shortcuts
        .into_iter()
        .map(|spec| shortcut_button(slot.clone(), spec, theme))
        .collect();

    // Label izquierdo: el nombre del tab activo.
    let label = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(inst.label.clone(), 12.0, theme.fg_text, Alignment::Start);

    let mut row = vec![label];
    row.append(&mut buttons);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(llimphi_ui::llimphi_layout::taffy::AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(row)
}

fn shortcut_button(slot: Slot, spec: ShortcutSpec, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{prelude::Dimension, AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;

    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(4.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(spec.label.clone(), 12.0, theme.fg_text, Alignment::Center)
    .on_click(Msg::ShortcutClicked(slot, spec.action))
}

fn drawer_tab_content(model: &Model, theme: &Theme) -> View<Msg> {
    let Some(inst) = model.drawer_tabs.get(model.active_drawer_tab) else {
        return placeholder(theme, &rimay_localize::t("shuma-empty-no-drawer-tabs"));
    };
    let idx = model.active_drawer_tab;
    match (inst.kind, &inst.state) {
        (Kind::Shell, ModuleState::Shell(state)) => {
            shuma_module_shell::view::<Msg>(state, theme, move |m| {
                Msg::Module(Slot::DrawerTab(idx), ModuleMsg::Shell(m))
            })
        }
        (Kind::Matilda, ModuleState::Matilda(state)) => {
            shuma_module_matilda::view::<Msg>(state.as_ref(), theme, move |m| {
                Msg::Module(Slot::DrawerTab(idx), ModuleMsg::Matilda(m))
            })
        }
        (Kind::Minga, ModuleState::Minga(state)) => {
            shuma_module_minga::view::<Msg>(state, theme, move |m| {
                Msg::Module(Slot::DrawerTab(idx), ModuleMsg::Minga(m))
            })
        }
        (Kind::Canvas, ModuleState::Canvas(state)) => {
            shuma_module_canvas::view::<Msg>(state, theme, move |m| {
                Msg::Module(Slot::DrawerTab(idx), ModuleMsg::Canvas(m))
            })
        }
        // Otros Kinds (Launcher/CommandBar) no tienen sentido como tab;
        // mostramos un placeholder informativo.
        _ => placeholder(theme, &rimay_localize::t("shuma-empty-no-drawer-compat")),
    }
}

// ─── Monitor stack ─────────────────────────────────────────────────

fn monitor_stack(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = StatCardPalette::from_theme(theme);

    let (cpu_value, mem_value) = match model.last_snapshot {
        Some(s) if s.valid => (s.cpu_percent, s.mem_percent),
        _ => (0.0, 0.0),
    };

    let cpu_card = monitor_card(
        "CPU",
        format!("{cpu_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!(
                "{} de {} muestras",
                model.sysmon.cpu_history().len(),
                HISTORY
            ),
            _ => rimay_localize::t("shuma-empty-no-data-linux"),
        },
        Color::from_rgb8(0x82, 0xCF, 0xF2),
        model.sysmon.cpu_history().values(),
        &palette,
    );

    let mem_card = monitor_card(
        "MEM",
        format!("{mem_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!("{} MB de {} MB", s.mem_used_mb, s.mem_total_mb),
            _ => rimay_localize::t("shuma-empty-no-data"),
        },
        Color::from_rgb8(0xF7, 0xC8, 0x7A),
        model.sysmon.mem_history().values(),
        &palette,
    );

    let mut children = vec![cpu_card, mem_card];

    // Stat-cards extra: una por cada `MonitorSpec` aportado por los
    // módulos vivos. El historial vive en `model.extra_history`.
    for (slot, contribs) in collect_contributions(model) {
        for spec in &contribs.monitors {
            let key = monitor_key(&slot, spec);
            let history = model
                .extra_history
                .get(&key)
                .cloned()
                .unwrap_or_default();
            let display = model
                .extra_display
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "—".into());
            let accent = Color::from_rgb8(spec.accent.r, spec.accent.g, spec.accent.b);
            children.push(monitor_card(
                spec.label.as_str(),
                display,
                rimay_localize::t_args(
                    "shuma-stat-samples",
                    &[
                        ("have", history.len().to_string().into()),
                        ("total", HISTORY.to_string().into()),
                    ],
                ),
                accent,
                history,
                &palette,
            ));
        }
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
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

fn monitor_card(
    label: &str,
    value: String,
    description: String,
    accent: Color,
    history: Vec<f32>,
    palette: &StatCardPalette,
) -> View<Msg> {
    let card = stat_card_view::<Msg>(label, value, description.as_str(), accent, &[], palette);
    let curve = curve_view(history, accent);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(6.0_f32),
        },
        ..Default::default()
    })
    .children(vec![card, curve])
}

fn curve_view(history: Vec<f32>, accent: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if history.len() < 2 {
            return;
        }
        let n = history.len() as f32;
        let dx = if n > 1.0 { rect.w / (n - 1.0) } else { rect.w };
        let mut path = BezPath::new();
        for (i, v) in history.iter().enumerate() {
            let x = rect.x + dx * i as f32;
            let y = rect.y + rect.h - (v.clamp(0.0, 100.0) / 100.0) * rect.h;
            let p = Point::new(x as f64, y as f64);
            if i == 0 {
                path.push(PathEl::MoveTo(p));
            } else {
                path.push(PathEl::LineTo(p));
            }
        }
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, accent, None, &path);
    })
}

fn placeholder(theme: &Theme, text: &str) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .text_aligned(text.to_string(), 13.0, theme.fg_muted, Alignment::Start)
}

/// Matcher del binding del shumarc contra la tecla recibida. Sintaxis:
///
/// ```text
/// <mods>+<tecla>
/// ```
///
/// donde `<mods>` es una secuencia (en cualquier orden, separadas por
/// `+`) de `Ctrl`, `Alt`, `Shift`, `Super` (alias `Meta`/`Cmd`/`Win`)
/// — case-insensitive. `<tecla>` puede ser un named key (`F1..F24`,
/// `Escape`, `Enter`, `Space`, `Tab`, `Backspace`, `Delete`, `Home`,
/// `End`, `PageUp`, `PageDown`, `ArrowLeft/Right/Up/Down`) o un
/// carácter literal (`a`, `1`, `grave`). El parsing es tolerante:
/// espacios alrededor de `+` se ignoran.
fn matches_key(want: &str, key: &Key, mods: &llimphi_ui::Modifiers) -> bool {
    let parsed = match parse_binding(want) {
        Some(p) => p,
        None => return false,
    };
    if parsed.ctrl != mods.ctrl
        || parsed.alt != mods.alt
        || parsed.shift != mods.shift
        || parsed.meta != mods.meta
    {
        return false;
    }
    match (parsed.target, key) {
        (KeyTarget::Named(want), Key::Named(got)) => want == *got,
        (KeyTarget::Char(want), Key::Character(s)) => {
            s.chars().next().map(|c| c.to_ascii_lowercase() == want).unwrap_or(false)
        }
        _ => false,
    }
}

#[derive(Debug)]
struct ParsedBinding {
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
    target: KeyTarget,
}

#[derive(Debug, PartialEq, Eq)]
enum KeyTarget {
    Named(NamedKey),
    Char(char),
}

fn parse_binding(s: &str) -> Option<ParsedBinding> {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut meta = false;
    let mut target: Option<KeyTarget> = None;
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return None;
    }
    // El último token es la tecla; los previos son modifiers.
    let (last, mods) = parts.split_last()?;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => ctrl = true,
            "alt" | "option" => alt = true,
            "shift" => shift = true,
            "super" | "meta" | "cmd" | "win" => meta = true,
            "" => continue, // tolerancia con strings sucios
            _ => return None,
        }
    }
    target = Some(match last.to_ascii_lowercase().as_str() {
        "escape" | "esc" => KeyTarget::Named(NamedKey::Escape),
        "enter" | "return" => KeyTarget::Named(NamedKey::Enter),
        "tab" => KeyTarget::Named(NamedKey::Tab),
        "backspace" => KeyTarget::Named(NamedKey::Backspace),
        "delete" | "del" => KeyTarget::Named(NamedKey::Delete),
        "space" => KeyTarget::Named(NamedKey::Space),
        "home" => KeyTarget::Named(NamedKey::Home),
        "end" => KeyTarget::Named(NamedKey::End),
        "pageup" | "pgup" => KeyTarget::Named(NamedKey::PageUp),
        "pagedown" | "pgdn" => KeyTarget::Named(NamedKey::PageDown),
        "left" | "arrowleft" => KeyTarget::Named(NamedKey::ArrowLeft),
        "right" | "arrowright" => KeyTarget::Named(NamedKey::ArrowRight),
        "up" | "arrowup" => KeyTarget::Named(NamedKey::ArrowUp),
        "down" | "arrowdown" => KeyTarget::Named(NamedKey::ArrowDown),
        "insert" | "ins" => KeyTarget::Named(NamedKey::Insert),
        "grave" | "backtick" | "`" => KeyTarget::Char('`'),
        "minus" | "-" => KeyTarget::Char('-'),
        "equal" | "=" => KeyTarget::Char('='),
        "slash" | "/" => KeyTarget::Char('/'),
        f if f.starts_with('f') && f.len() <= 3 => {
            let n: u32 = f[1..].parse().ok()?;
            let named = match n {
                1 => NamedKey::F1,
                2 => NamedKey::F2,
                3 => NamedKey::F3,
                4 => NamedKey::F4,
                5 => NamedKey::F5,
                6 => NamedKey::F6,
                7 => NamedKey::F7,
                8 => NamedKey::F8,
                9 => NamedKey::F9,
                10 => NamedKey::F10,
                11 => NamedKey::F11,
                12 => NamedKey::F12,
                13 => NamedKey::F13,
                14 => NamedKey::F14,
                15 => NamedKey::F15,
                16 => NamedKey::F16,
                17 => NamedKey::F17,
                18 => NamedKey::F18,
                19 => NamedKey::F19,
                20 => NamedKey::F20,
                21 => NamedKey::F21,
                22 => NamedKey::F22,
                23 => NamedKey::F23,
                24 => NamedKey::F24,
                _ => return None,
            };
            KeyTarget::Named(named)
        }
        c if c.chars().count() == 1 => KeyTarget::Char(c.chars().next().unwrap()),
        _ => return None,
    });
    Some(ParsedBinding {
        ctrl,
        alt,
        shift,
        meta,
        target: target.unwrap(),
    })
}

#[cfg(test)]
mod tests_bindings {
    use super::*;
    use llimphi_ui::Modifiers;

    #[test]
    fn f12_matches_named_no_modifiers() {
        let p = parse_binding("F12").unwrap();
        assert!(matches!(p.target, KeyTarget::Named(NamedKey::F12)));
        assert!(!p.ctrl && !p.alt && !p.shift && !p.meta);
    }

    #[test]
    fn ctrl_grave_parses_and_matches() {
        let want = "Ctrl+grave";
        let key = Key::Character("`".into());
        let mods = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert!(matches_key(want, &key, &mods));
        // Sin Ctrl no matchea.
        let mods_no = Modifiers::default();
        assert!(!matches_key(want, &key, &mods_no));
    }

    #[test]
    fn super_space_alias() {
        let p = parse_binding("Super+Space").unwrap();
        assert!(p.meta);
        assert!(matches!(p.target, KeyTarget::Named(NamedKey::Space)));
        let p2 = parse_binding("Meta+Space").unwrap();
        assert!(p2.meta);
    }

    #[test]
    fn ctrl_shift_letter_combo() {
        let want = "Ctrl+Shift+a";
        let key = Key::Character("a".into());
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert!(matches_key(want, &key, &mods));
        // Solo Ctrl, sin Shift → no matchea.
        let mods_no = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        assert!(!matches_key(want, &key, &mods_no));
    }

    #[test]
    fn unknown_token_returns_none() {
        assert!(parse_binding("Hyper+x").is_none());
        assert!(parse_binding("F99").is_none());
    }
}
