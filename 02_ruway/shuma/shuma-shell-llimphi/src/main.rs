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
const MONITORS_INITIAL_WIDTH: f32 = 280.0;

fn main() {
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
            label: "Launcher".into(),
            state: ModuleState::Launcher(state),
        }
    }

    fn command_bar(state: shuma_module_commandbar::State) -> Self {
        Self {
            kind: Kind::CommandBar,
            label: "Command".into(),
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
        Self {
            kind: Kind::Matilda,
            label,
            state: ModuleState::Matilda(Box::new(shuma_module_matilda::State::new(source))),
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
}

#[derive(Clone)]
enum Msg {
    Tick,
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

        let cfg = config::ShumaConfig::load_default();
        let topbar = resolve_slot(cfg.topbar.as_ref())
            .or_else(|| Some(Instance::launcher(shuma_module_launcher::State::demo())));
        let bottombar = resolve_slot(cfg.bottombar.as_ref()).or_else(|| {
            Some(Instance::command_bar(
                shuma_module_commandbar::State::default(),
            ))
        });
        let main = resolve_slot(cfg.main.as_ref());

        let drawer_tabs = if cfg.drawer.tabs.is_empty() {
            // Default cuando no hay `[[drawer.tabs]]`: shell + matilda
            // locales para que el chasis sea exploratorio desde el día
            // uno sin que el usuario tenga que escribir un shumarc.
            vec![
                Instance::shell("Shell".into(), Source::Local),
                Instance::matilda("Matilda".into(), Source::Local),
            ]
        } else {
            cfg.drawer
                .tabs
                .iter()
                .filter_map(resolve_drawer_tab)
                .collect()
        };

        Model {
            theme: Theme::dark(),
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
        // Tecla configurada para toggle (default F12). Se compara contra
        // el label "F12"/"F11"/... — bloque 5 traerá un parser real de
        // `Super+grave` etc.
        if let Some(want) = model.drawer_trigger.key.as_deref() {
            if matches_key(want, &e.key) {
                return Some(Msg::ToggleDrawer);
            }
        }
        None
    }

    fn update(model: Self::Model, msg: Self::Msg, handle: &Handle<Self::Msg>) -> Self::Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                m.last_snapshot = Some(m.sysmon.sample());
                sample_extra_monitors(&mut m);
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
    resolve_instance(&entry.module, entry.source.clone(), entry.label.clone())
}

fn resolve_drawer_tab(entry: &config::DrawerTabEntry) -> Option<Instance> {
    resolve_instance(&entry.id, entry.source.clone(), entry.label.clone())
}

fn resolve_instance(id: &str, source: Source, label: Option<String>) -> Option<Instance> {
    let label = label.unwrap_or_else(|| source.label());
    match id {
        shuma_module_launcher::ID => {
            Some(Instance::launcher(shuma_module_launcher::State::demo()))
        }
        shuma_module_commandbar::ID => Some(Instance::command_bar(
            shuma_module_commandbar::State::default(),
        )),
        shuma_module_shell::ID => Some(Instance::shell(label, source)),
        shuma_module_matilda::ID => Some(Instance::matilda(label, source)),
        unknown => {
            eprintln!("shuma: módulo desconocido «{unknown}» — se ignora");
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
            }
            let msg = dispatch_to_module(&slot, &m, action_id);
            if let Some(mmsg) = msg {
                m = apply_module_msg(m, slot, mmsg);
            }
        }
    }
    m
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
            _ => placeholder(theme, "Módulo Main no compatible"),
        },
        None => placeholder(
            theme,
            "Sin módulo Main configurado.\n\nF12 abre el drawer con shell + monitores.\nClick en la command bar también.",
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
        return placeholder(theme, "Sin tabs en el drawer.");
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
        // Otros Kinds (Launcher/CommandBar) no tienen sentido como tab;
        // mostramos un placeholder informativo.
        _ => placeholder(theme, "Este módulo no puede ser DrawerTab."),
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
            _ => "sin datos (¿no es Linux?)".into(),
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
            _ => "sin datos".into(),
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
                format!("muestras: {} / {}", history.len(), HISTORY),
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

/// Matcher mínimo del label de tecla configurado en shumarc contra el
/// `Key` que llega del backend. Cubre F1..F24, Escape, Enter; bloque 5
/// expande para Super/Ctrl/etc.
fn matches_key(want: &str, key: &Key) -> bool {
    match key {
        Key::Named(named) => {
            let label = named_label(*named);
            label.eq_ignore_ascii_case(want)
        }
        _ => false,
    }
}

fn named_label(n: NamedKey) -> &'static str {
    match n {
        NamedKey::F1 => "F1",
        NamedKey::F2 => "F2",
        NamedKey::F3 => "F3",
        NamedKey::F4 => "F4",
        NamedKey::F5 => "F5",
        NamedKey::F6 => "F6",
        NamedKey::F7 => "F7",
        NamedKey::F8 => "F8",
        NamedKey::F9 => "F9",
        NamedKey::F10 => "F10",
        NamedKey::F11 => "F11",
        NamedKey::F12 => "F12",
        NamedKey::Escape => "Escape",
        NamedKey::Enter => "Enter",
        _ => "",
    }
}
