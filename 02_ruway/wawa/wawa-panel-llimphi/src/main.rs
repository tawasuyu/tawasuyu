//! `wawa-panel-llimphi` — panel de control del sistema operativo wawa.
//!
//! Un panel de **configuración** navegado por un rail de **dientes**. Un diente
//! es siempre un panel: lleva su nombre y, dentro, una lista de items.
//!
//! Los dientes son de dos clases y conviven en el mismo rail:
//!
//! * **Dientes-categoría** (por tipo, transversales): Apariencia, Idioma,
//!   Módulos, Información. El SO (y a futuro cualquier app) aporta sus items.
//! * **Dientes-de-app**: una app suscrita registra su propio diente con su
//!   esquema de config (mirada, pata). Un módulo apagado oculta su diente.
//!
//! No hay lanzador de apps ni botones externos: esto configura, no abre apps.
//! El renderizador es `llimphi-module-allichay`; cada cambio se rutea a su
//! destino (`wawa` / `mirada` / `pata`) y se persiste en su formato nativo.

use std::path::PathBuf;
use std::sync::Arc;

use allichay::{Configurable, EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_module_allichay::{schema_panel, AllichayMsg, AllichayState};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};
use llimphi_ui::DragPhase;
use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use wawa_config::{ConfigWatcher, WawaConfig};

// =====================================================================
// Constantes y catálogos
// =====================================================================

/// Refresco del monitor (Información).
const TICK_MS: u64 = 1_000;
/// Ancho del rail de pestañas (la tira que sobresale).
const RAIL_W: f32 = 46.0;
/// Ancho del sidebar de items (a la izquierda).
const SIDEBAR_W: f32 = 232.0;
/// Alto del viewport del panel (para el scroll). Conservador respecto del alto
/// de ventana inicial menos menubar/header/status; si la ventana es más alta
/// queda algo de aire abajo. (Mejorable cuando el host trackee el resize.)
const VIEWPORT_H: f32 = 500.0;

/// Variantes del theme. El id casa con `Theme::by_name`; el label es key i18n.
const THEME_VARIANTS: &[(&str, &str)] = &[
    ("dark", "wawa-panel-variant-dark"),
    ("light", "wawa-panel-variant-light"),
    ("aurora", "wawa-panel-variant-aurora"),
    ("sunset", "wawa-panel-variant-sunset"),
];

/// Locales ofrecidos. El id come `rimay_localize::set_locale`.
const LANGS: &[(&str, &str)] = &[("es-PE", "Español"), ("en-US", "English"), ("qu-PE", "Runasimi")];

/// Acentos. El id persiste en `WawaConfig::accent`; el color lo resuelve
/// `wawa_config::accent_rgb`.
const ACCENTS: &[(&str, &str)] = &[
    ("default", "tawasuyu"),
    ("unanchay", "unanchay"),
    ("yachay", "yachay"),
    ("ruway", "ruway"),
    ("ukupacha", "ukupacha"),
];

/// Módulos del SO con su id estable, glyph y key i18n.
const MODULES: &[(&str, &str, &str)] = &[
    ("mirada", "◉", "wawa-panel-mod-mirada"),
    ("shuma", "✦", "wawa-panel-mod-shuma"),
    ("chasqui", "✉", "wawa-panel-mod-chasqui"),
    ("akasha", "↻", "wawa-panel-mod-akasha"),
    ("minga", "◈", "wawa-panel-mod-minga"),
    ("agora", "◯", "wawa-panel-mod-agora"),
];

/// Índice de la pestaña "Información" (2ª) — para el menú Ayuda.
const INFO_DIENTE: usize = 1;

// =====================================================================
// Información del host (Linux /proc)
// =====================================================================

#[derive(Clone, Default)]
struct HostInfo {
    host: String,
    kernel: String,
    uptime: u64,
    mem_total_kb: u64,
    mem_avail_kb: u64,
    load: (f32, f32, f32),
}

fn read_proc_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn parse_meminfo(s: &str) -> (u64, u64) {
    let (mut total, mut avail) = (0, 0);
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = rest.trim().split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail = rest.trim().split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    (total, avail)
}

fn parse_loadavg(s: &str) -> (f32, f32, f32) {
    let mut it = s.split_whitespace();
    let a = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let b = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let c = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    (a, b, c)
}

fn parse_uptime(s: &str) -> u64 {
    s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()).map(|v| v as u64).unwrap_or(0)
}

fn read_hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

fn read_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "—".into())
}

fn refresh_host(info: &mut HostInfo) {
    info.host = read_hostname();
    info.kernel = read_kernel();
    info.uptime = parse_uptime(&read_proc_file("/proc/uptime"));
    let (total, avail) = parse_meminfo(&read_proc_file("/proc/meminfo"));
    info.mem_total_kb = total;
    info.mem_avail_kb = avail;
    info.load = parse_loadavg(&read_proc_file("/proc/loadavg"));
}

fn fmt_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hrs = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hrs:02}h {mins:02}m")
    } else {
        format!("{hrs:02}h {mins:02}m")
    }
}

fn fmt_mem(used_kb: u64, total_kb: u64) -> String {
    let used_mb = used_kb as f64 / 1024.0;
    let total_mb = total_kb as f64 / 1024.0;
    if total_mb > 1024.0 {
        format!("{:.1} / {:.1} GiB", used_mb / 1024.0, total_mb / 1024.0)
    } else {
        format!("{used_mb:.0} / {total_mb:.0} MiB")
    }
}

// =====================================================================
// Modelo + mensajes
// =====================================================================

struct Model {
    /// Pestaña activa: índice en [`pestanas`] (la app/categoría).
    selected_pest: usize,
    /// Item activo dentro de la pestaña: índice de sección en su schema (lo que
    /// se abre en el canvas central). `None` = ninguno → el canvas muestra el
    /// resumen de la pestaña.
    selected_item: Option<usize>,
    /// Si el sidebar de items está visible (se oculta clickeando la pestaña activa).
    sidebar_open: bool,
    /// Ancho del sidebar (px), arrastrable.
    sidebar_w: f32,
    cfg: WawaConfig,
    mirada: mirada_brain::Config,
    mirada_path: Option<PathBuf>,
    /// Filas crudas `[combinación, acción]` del keymap de mirada (buffer
    /// editable; el `Keymap` válido se deriva al guardar — ver [`flush_saves`]).
    keymap_rows: Vec<Vec<String>>,
    keymap_path: Option<PathBuf>,
    /// Biblioteca de perfiles de atajos (dwm/i3/hyprland + propios). El selector
    /// del panel conmuta el activo; al cambiar, recarga el keymap visible.
    profiles: mirada_brain::KeymapProfiles,
    profiles_path: Option<PathBuf>,
    /// La **vista** (perfil de escritorio completo) activa: look + decoración +
    /// layout + atajos + barra. `None` = ninguna aplicada esta sesión.
    active_vista: Option<String>,
    pata: pata_core::Config,
    allichay: AllichayState,
    host: HostInfo,
    status: String,
    /// Qué configs tienen cambios sin persistir (se aplican en memoria al
    /// instante pero el `save()` a disco se difiere — ver [`SaveDirty`]).
    dirty: SaveDirty,
    /// Ticks que faltan para volcar lo sucio a disco. `0` = nada pendiente. Cada
    /// cambio lo rearma a [`SAVE_DELAY_TICKS`]; un drag de slider lo resetea en
    /// cada movimiento, así que sólo se guarda una vez al soltar (debounce).
    save_in: u32,
    _config_watcher: Option<ConfigWatcher>,
    menu_open: Option<usize>,
    menu_active: usize,
    menu_anim: Tween<f32>,
}

/// Ticks (de [`TICK_MS`]) que se espera tras el último cambio antes de persistir.
const SAVE_DELAY_TICKS: u32 = 1;

/// Banderas de "config sucia": cuáles tienen cambios aplicados en memoria pero
/// todavía no escritos a disco. Evita el martilleo de `save()` (y la tormenta de
/// recargas del `FileWatch` del compositor) durante el arrastre de un slider.
#[derive(Default)]
struct SaveDirty {
    wawa: bool,
    mirada: bool,
    keymap: bool,
    pata: bool,
    profiles: bool,
}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Click en una pestaña del rail (app/categoría): cambia el sidebar; si ya
    /// estaba activa, lo oculta/muestra.
    SelectPestana(u64),
    /// Click en un item del sidebar: abre su contenido en el canvas central.
    SelectItem(u64),
    /// Arrastre del divisor: delta de ancho del sidebar.
    SetSidebarWidth(f32),
    /// Mensaje del renderizador de config (foco/cambio/scroll).
    Allichay(AllichayMsg),
    /// Tecla al campo de texto en edición.
    AllichayKey(KeyEvent),
    /// Cambió la config del SO desde afuera (otro panel, edición manual).
    ConfigChanged(Box<WawaConfig>),
    MenuOpen(Option<usize>),
    MenuCommand(String),
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    CloseMenus,
}

// =====================================================================
// App
// =====================================================================

struct Panel;

impl App for Panel {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "wawa · panel de control"
    }

    fn initial_size() -> (u32, u32) {
        (1080, 680)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(std::time::Duration::from_millis(TICK_MS), || Msg::Tick);

        let cfg = WawaConfig::load();
        let _ = rimay_localize::set_locale(&cfg.lang);

        let handle_clone = handle.clone();
        let watcher = ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::ConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("wawa-panel · watcher: {e}"))
        .ok();

        let mut host = HostInfo::default();
        refresh_host(&mut host);

        let mirada_path = mirada_brain::Config::default_path();
        let mirada = mirada_path
            .as_deref()
            .map(mirada_brain::Config::load_or_default)
            .unwrap_or_default();
        let pata = pata_config::load();

        // Keymap de mirada: vive en su propio RON. El buffer editable son las
        // filas crudas (así no se pierde una fila a-medio-tipear); el `Keymap`
        // válido se deriva al guardar.
        let keymap_path = mirada_brain::Keymap::default_path();
        let keymap_rows = keymap_path
            .as_deref()
            .map(mirada_brain::Keymap::load_or_init)
            .unwrap_or_default()
            .to_rows();

        // Perfiles de atajos: la biblioteca conmutable (dwm/i3/hyprland + propios).
        let profiles_path = mirada_brain::KeymapProfiles::default_path();
        let profiles = profiles_path
            .as_deref()
            .map(mirada_brain::KeymapProfiles::load_or_init)
            .unwrap_or_default();

        Model {
            selected_pest: 0,
            selected_item: None,
            sidebar_open: true,
            sidebar_w: SIDEBAR_W,
            cfg,
            mirada,
            mirada_path,
            keymap_rows,
            keymap_path,
            profiles,
            profiles_path,
            active_vista: None,
            pata,
            allichay: AllichayState::new(),
            host,
            status: String::new(),
            dirty: SaveDirty::default(),
            save_in: 0,
            _config_watcher: watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                refresh_host(&mut m.host);
                // Debounce de guardado: el último cambio armó `save_in`; cuando
                // llega a 0 sin nuevos cambios, se vuelca a disco.
                if m.save_in > 0 {
                    m.save_in -= 1;
                    if m.save_in == 0 {
                        flush_saves(&mut m);
                    }
                }
            }
            Msg::SelectPestana(id) => {
                let n = pestanas(&m).len().max(1);
                let id = (id as usize).min(n - 1);
                if id == m.selected_pest {
                    // Clic en la pestaña activa: oculta/muestra su sidebar.
                    m.sidebar_open = !m.sidebar_open;
                } else {
                    m.selected_pest = id;
                    m.sidebar_open = true;
                    // Nueva pestaña: sin item → el canvas muestra su resumen.
                    m.selected_item = None;
                    m.allichay.select(0);
                }
                m.status.clear();
            }
            Msg::SelectItem(id) => {
                m.selected_item = Some(id as usize);
                m.allichay.select(id as usize);
                m.status.clear();
            }
            Msg::SetSidebarWidth(dx) => {
                m.sidebar_w = (m.sidebar_w + dx).clamp(160.0, 520.0);
            }
            Msg::Allichay(AllichayMsg::SelectSection(_)) => {}
            Msg::Allichay(AllichayMsg::Focus(path)) => {
                let seed = current_text_value(&m, &path);
                m.allichay.focus(&path, &seed);
            }
            Msg::Allichay(AllichayMsg::FocusCell(path, row, col)) => {
                if let Some(value) = current_field_value(&m, &path) {
                    m.allichay.focus_cell(&path, value, row, col);
                }
            }
            Msg::Allichay(AllichayMsg::FocusHex(path)) => {
                let seed = current_field_value(&m, &path)
                    .and_then(|v| v.as_color())
                    .map(llimphi_module_allichay::color_hex)
                    .unwrap_or_default();
                m.allichay.focus_hex(&path, &seed);
            }
            Msg::Allichay(AllichayMsg::Change(path, value)) => route_change(&mut m, &path, value),
            Msg::Allichay(AllichayMsg::ScrollTo(offset)) => m.allichay.set_scroll(offset),
            Msg::AllichayKey(event) => {
                if let Some((path, value)) = m.allichay.apply_key(&event) {
                    route_change(&mut m, &path, value);
                }
            }
            Msg::ConfigChanged(new_cfg) => {
                if *new_cfg != m.cfg {
                    let lang_changed = new_cfg.lang != m.cfg.lang;
                    m.cfg = *new_cfg;
                    if lang_changed {
                        let _ = rimay_localize::set_locale(&m.cfg.lang);
                    }
                    m.status = rimay_localize::t("wawa-panel-status-config-updated");
                }
            }
            Msg::MenuOpen(which) => {
                m.menu_open = which;
                m.menu_active = usize::MAX;
                if which.is_some() {
                    m.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(handle, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    m.menu_active = menubar_nav(&menu, mi, m.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = m.menu_open {
                    let menu = app_menu();
                    if let Some(cmd) = menubar_command_at(&menu, mi, m.menu_active) {
                        m.menu_open = None;
                        return handle_menu_command(m, &cmd);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                return handle_menu_command(m, &cmd);
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Edición de texto en curso → todas las teclas al renderizador.
        if model.allichay.is_editing() {
            return Some(Msg::AllichayKey(event.clone()));
        }
        if let Some(mi) = model.menu_open {
            let n = app_menu().menus.len().max(1);
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenus),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::MenuOpen(Some((mi + n - 1) % n))),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::MenuOpen(Some((mi + 1) % n))),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuNav(1)),
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuNav(-1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivate),
                _ => None,
            };
        }
        if let Key::Named(NamedKey::Escape) = event.key {
            if model.menu_open.is_some() {
                return Some(Msg::CloseMenus);
            }
        }
        None
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = theme_from_cfg(&model.cfg);
        let pestanas = pestanas(model);
        let pest = model.selected_pest.min(pestanas.len().saturating_sub(1));

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = build_header(&theme);
        let body = build_body(&pestanas, pest, model, &theme);
        let status = build_status(model, &theme);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![menubar, header, body, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let theme = theme_from_cfg(&model.cfg);
        let menu = app_menu();
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

fn viewport_of() -> (f32, f32) {
    let (w, h) = Panel::initial_size();
    (w as f32, h as f32)
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Panel>();
}

// =====================================================================
// Registro de pestañas (categorías) + sus items (secciones)
// =====================================================================

/// Una pestaña del rail: su nombre, su icono y el schema cuyas secciones son los
/// **items** que lista su sidebar.
struct PanelPestana {
    title: String,
    icon: String,
    schema: Schema,
}

/// Arma el rail aprovechando la triple jerarquía: pocas pestañas, cada una con
/// varios items (sin paneles de un solo item).
///
/// - **Sistema** (categoría SO): Apariencia · Idioma · Interfaz (llimphi) ·
///   Arranque (arje como init) · Módulos.
/// - **Información** (categoría SO, sólo lectura): Estado del equipo · Acerca.
/// - **mirada** (app suscrita): sus secciones (Teselado, Decoración, …).
/// - **pata** (app suscrita): sus secciones (General, Superficie N, …).
fn pestanas(m: &Model) -> Vec<PanelPestana> {
    let mut out = vec![
        PanelPestana {
            title: "Sistema".into(),
            icon: "⚙".into(),
            schema: sistema_schema(&m.cfg),
        },
        PanelPestana {
            title: "Información".into(),
            icon: "🖥".into(),
            schema: info_schema(&m.host),
        },
        PanelPestana {
            title: "Perfiles".into(),
            icon: "⌨".into(),
            schema: perfiles_schema(m),
        },
    ];
    if m.cfg.module_enabled("mirada") {
        let mut schema = prefix_schema(m.mirada.schema(), "mirada");
        // El keymap vive en su propio RON; se edita como una sección más de la
        // pestaña mirada (id ya prefijado para que el ruteo lo reconozca).
        schema.sections.push(keymap_section(&m.keymap_rows));
        out.push(PanelPestana {
            title: "mirada".into(),
            icon: "☸".into(),
            schema,
        });
    }
    if m.cfg.module_enabled("pata") {
        out.push(PanelPestana {
            title: "pata".into(),
            icon: "🎛".into(),
            schema: prefix_schema(m.pata.schema(), "pata"),
        });
    }
    out
}

/// Prefija el id de cada sección con el destino de ruteo (`"mirada::teselado"`),
/// para que el `FieldPath` de cada campo identifique a qué app aplicar.
fn prefix_schema(mut schema: Schema, target: &str) -> Schema {
    for sec in &mut schema.sections {
        sec.id = format!("{target}::{}", sec.id);
    }
    schema
}

/// La pestaña **Perfiles**: una sección por **vista** (perfil de escritorio
/// COMPLETO: look + decoración + layout + atajos + barra — mirada/windows-xp/
/// mac/kde/hyprland/dwm…) en el sidebar. Activar una vista aplica TODO de
/// inmediato (config.ron + keymap.ron + launcher.toml de pata, que el
/// compositor y pata recargan en caliente) y su config queda editable en las
/// pestañas mirada · pata · Sistema. La activa se marca con ●.
fn perfiles_schema(m: &Model) -> Schema {
    use allichay::Field;
    let mut schema = Schema::new();
    for name in mirada_brain::VISTA_NAMES {
        let label = mirada_brain::Vista::label_for(name);
        let is_active = m.active_vista.as_deref() == Some(name);
        let title = if is_active {
            format!("● {label}")
        } else {
            label.clone()
        };
        schema = schema.section(
            Section::new(name, title)
                .icon("🖥")
                .help(
                    "Vista de escritorio completa. Al activarla cambia look, \
                     decoración, layout, atajos y barra al instante. Ajustá su \
                     detalle en las pestañas mirada · pata · Sistema.",
                )
                .field(Field::toggle(
                    "activo",
                    format!("Usar la vista «{label}»"),
                    is_active,
                )),
        );
    }
    prefix_schema(schema, "perfiles")
}

/// Aplica una **vista** completa (perfil de escritorio): vuelca su config a
/// `config.ron`, su keymap como perfil activo a `keymap.ron`, y la barra de la
/// vista a `launcher.toml` — el compositor y pata recargan en caliente, así
/// toda la vista se actualiza de inmediato. Refleja todo en el panel.
fn apply_vista(m: &mut Model, name: &str) {
    let Some(v) = mirada_brain::Vista::by_name(name) else {
        return;
    };
    if let Some(p) = mirada_brain::Config::default_path() {
        let _ = v.config.save(&p);
    }
    let _ = m.profiles.set_active(v.keymap);
    if let Some(pp) = m.profiles_path.clone() {
        let _ = m.profiles.save(&pp);
    }
    if let Some(kp) = m.keymap_path.clone() {
        let _ = m.profiles.write_active_keymap(&kp);
    }
    if let Some(bar) = pata_core::Config::vista_preset(name) {
        let _ = pata_config::save(&bar);
        m.pata = bar;
    }
    // Reflejar en el panel para que las otras pestañas muestren la vista nueva.
    m.mirada = v.config.clone();
    m.keymap_rows = m.profiles.active_keymap().to_rows();
    m.active_vista = Some(name.to_string());
    m.status = format!("vista «{}» aplicada (en caliente)", v.label);
}

/// La sección "Atajos" de mirada: el keymap como tabla (combinación · acción).
/// El id va prefijado (`mirada::atajos`) para que [`route_change`] lo reconozca
/// y lo aplique al buffer del keymap (no a la `Config`).
fn keymap_section(rows: &[Vec<String>]) -> Section {
    use allichay::{Column, Field};
    Section::new("mirada::atajos", "Atajos (perfil activo)")
        .icon("⌨")
        .help("Atajos del perfil activo. Para conmutar de perfil, andá a la pestaña Perfiles.")
        .field(Field::table(
            "bindings",
            "Atajos de teclado",
            vec![
                Column::new("combo", "Combinación"),
                Column::new("action", "Acción"),
            ],
            rows.to_vec(),
        ))
}

/// La pestaña "Sistema": varios items de configuración del SO.
fn sistema_schema(cfg: &WawaConfig) -> Schema {
    Schema::new()
        .section(appearance_section(cfg))
        .section(idioma_section(cfg))
        .section(interfaz_section(cfg))
        .section(arranque_section())
        .section(modulos_section(cfg))
}

fn appearance_section(cfg: &WawaConfig) -> Section {
    let t = rimay_localize::t;
    Section::new("wawa::apariencia", t("wawa-panel-cat-appearance"))
        .icon("🎨")
        .field(Field::dropdown(
            "theme_variant",
            t("wawa-panel-label-variant"),
            cfg.theme_variant.clone(),
            THEME_VARIANTS.iter().map(|(id, k)| EnumOption::new(*id, t(k))).collect(),
        ))
        .field(Field::dropdown(
            "accent",
            t("wawa-panel-label-accent"),
            cfg.accent.clone(),
            ACCENTS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
}

fn idioma_section(cfg: &WawaConfig) -> Section {
    let t = rimay_localize::t;
    Section::new("wawa::idioma", t("language"))
        .icon("🌐")
        .field(Field::dropdown(
            "lang",
            t("wawa-panel-label-language"),
            cfg.lang.clone(),
            LANGS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
        .field(
            Field::toggle("timefmt_24h", t("wawa-panel-label-clock"), cfg.timefmt_24h)
                .help(t("wawa-panel-clock-24h")),
        )
}

/// Interfaz (llimphi): toolkit del SO. Controles reales próximamente (present
/// mode, fuente, animaciones) — por ahora informativo.
fn interfaz_section(cfg: &WawaConfig) -> Section {
    Section::new("wawa::interfaz", "Interfaz")
        .icon("🎛")
        .help("El toolkit gráfico del sistema (llimphi)")
        .field(Field::display("toolkit", "Toolkit", "llimphi"))
        // Decisión GLOBAL de dónde van los rails de dientes (sidebars). Todas
        // las apps con dientes se rigen por esto, no por app.
        .field(Field::toggle(
            "dientes_outside",
            "Dientes fuera del área de trabajo",
            cfg.dientes_outside,
        ))
        .field(Field::display(
            "proximamente",
            "Próximamente",
            "present mode (vsync) · fuente · animaciones",
        ))
}

/// Arranque: usar arje como init del sistema. Control real próximamente.
fn arranque_section() -> Section {
    Section::new("wawa::arranque", "Arranque")
        .icon("▶")
        .help("Init del sistema en Linux")
        .field(Field::display("init", "Init", "systemd (actual)"))
        .field(Field::display(
            "proximamente",
            "Próximamente",
            "elegir arje como init (PID 1)",
        ))
}

fn modulos_section(cfg: &WawaConfig) -> Section {
    let t = rimay_localize::t;
    let mut section = Section::new("wawa::modulos", t("wawa-panel-cat-modules")).icon("☸");
    for (id, _glyph, key) in MODULES {
        section = section.field(Field::toggle(*id, t(key), cfg.module_enabled(id)));
    }
    section
}

/// La pestaña "Información": estado del equipo + acerca (sólo lectura).
fn info_schema(host: &HostInfo) -> Schema {
    let t = rimay_localize::t;
    let used_kb = host.mem_total_kb.saturating_sub(host.mem_avail_kb);
    Schema::new()
        .section(
            Section::new("wawa::infohost", t("wawa-panel-cat-monitor"))
                .icon("🖥")
                .field(Field::display("host", t("wawa-panel-stat-host"), &host.host))
                .field(Field::display("kernel", t("wawa-panel-stat-kernel"), &host.kernel))
                .field(Field::display("uptime", t("wawa-panel-stat-uptime"), fmt_uptime(host.uptime)))
                .field(Field::display(
                    "mem",
                    t("wawa-panel-stat-mem"),
                    fmt_mem(used_kb, host.mem_total_kb),
                ))
                .field(Field::display(
                    "load",
                    t("wawa-panel-stat-load"),
                    format!("{:.2} · {:.2} · {:.2}", host.load.0, host.load.1, host.load.2),
                )),
        )
        .section(
            Section::new("wawa::about", t("wawa-panel-about-name"))
                .field(Field::display("name", t("wawa-panel-about-name"), "wawa"))
                .field(Field::display("version", t("wawa-panel-about-version"), env!("CARGO_PKG_VERSION")))
                .field(Field::display("toolkit", t("wawa-panel-about-toolkit"), "llimphi")),
        )
}

// =====================================================================
// Ruteo de cambios a la app destino
// =====================================================================

/// Parte un FieldPath (`["mirada::teselado", "gap"]`) en destino + ruta relativa
/// (`("mirada", ["teselado", "gap"])`).
fn split_app(path: &FieldPath) -> Option<(String, FieldPath)> {
    let segs = path.segments();
    let (key, sect) = segs.first()?.split_once("::")?;
    let mut rel = vec![sect.to_string()];
    rel.extend(segs[1..].iter().cloned());
    Some((key.to_string(), FieldPath(rel)))
}

/// Aplica un cambio a la app destino y lo persiste en su formato nativo.
/// Aplica un cambio a la config destino **en memoria** y marca su bandera de
/// sucio + arma el debounce; el `save()` a disco lo hace [`flush_saves`] cuando
/// el contador llega a cero (ver [`SaveDirty`]).
fn route_change(m: &mut Model, path: &FieldPath, value: FieldValue) {
    let Some((key, rel)) = split_app(path) else {
        m.status = format!("· ruta inválida: {path}");
        return;
    };
    match key.as_str() {
        "wawa" => {
            apply_wawa(m, rel.leaf().unwrap_or(""), value);
            m.dirty.wawa = true;
        }
        // Pestaña Perfiles: el primer segmento es el NOMBRE de la vista; el
        // toggle «activo» la aplica entera (look + atajos + barra) en caliente.
        "perfiles" => {
            let name = rel.segments().first().cloned().unwrap_or_default();
            if rel.leaf() == Some("activo") && value.as_bool() == Some(true) {
                apply_vista(m, &name);
            }
        }
        "mirada" if rel.segments().first().map(String::as_str) == Some("atajos") => {
            match rel.leaf() {
                // Conmutar el perfil activo: recarga la tabla con su keymap y
                // marca para persistir profiles.ron + keymap.ron.
                Some("profile") => {
                    if let Some(name) = value.as_str() {
                        if m.profiles.set_active(name).is_ok() {
                            m.keymap_rows = m.profiles.active_keymap().to_rows();
                            m.dirty.keymap = true;
                            m.dirty.profiles = true;
                        }
                    }
                }
                // La tabla actualiza el buffer de filas crudas (se preserva lo
                // a-medio-tipear; el Keymap válido se deriva al guardar).
                _ => {
                    if let Some(rows) = value.as_table() {
                        m.keymap_rows = rows.to_vec();
                        m.dirty.keymap = true;
                    }
                }
            }
        }
        "mirada" => {
            if let Err(e) = m.mirada.apply(&rel, value) {
                m.status = format!("· mirada: {e}");
                return;
            }
            m.dirty.mirada = true;
        }
        "pata" => {
            if let Err(e) = m.pata.apply(&rel, value) {
                m.status = format!("· pata: {e}");
                return;
            }
            m.dirty.pata = true;
        }
        _ => return,
    }
    m.save_in = SAVE_DELAY_TICKS;
}

/// Persiste a disco las configs marcadas como sucias y limpia las banderas. Lo
/// llama el `Tick` cuando el debounce expira. Cada `save()` que falla deja su
/// error en el status; si al menos uno fue OK y ninguno falló, status = "ok".
fn flush_saves(m: &mut Model) {
    let mut ok = false;
    let mut err: Option<String> = None;
    if m.dirty.wawa {
        match m.cfg.save() {
            Ok(_) => ok = true,
            Err(e) => err = Some(format!("· save: {e}")),
        }
        m.dirty.wawa = false;
    }
    if m.dirty.mirada {
        match m.mirada_path.as_deref().map(|p| m.mirada.save(p)) {
            Some(Ok(())) => ok = true,
            Some(Err(e)) => err = Some(format!("· mirada save: {e}")),
            None => err = Some("· mirada: sin ruta de config".into()),
        }
        m.dirty.mirada = false;
    }
    if m.dirty.keymap {
        // Derivamos el Keymap válido de las filas y lo escribimos a su RON.
        let km = mirada_brain::Keymap::from_rows(&m.keymap_rows);
        match m.keymap_path.as_deref().map(|p| km.save(p)) {
            Some(Ok(())) => ok = true,
            Some(Err(e)) => err = Some(format!("· keymap save: {e}")),
            None => err = Some("· keymap: sin ruta de config".into()),
        }
        m.dirty.keymap = false;
    }
    if m.dirty.profiles {
        match m.profiles_path.as_deref().map(|p| m.profiles.save(p)) {
            Some(Ok(())) => ok = true,
            Some(Err(e)) => err = Some(format!("· profiles save: {e}")),
            None => err = Some("· profiles: sin ruta".into()),
        }
        m.dirty.profiles = false;
    }
    if m.dirty.pata {
        match pata_config::save(&m.pata) {
            Ok(_) => ok = true,
            Err(e) => err = Some(format!("· pata save: {e}")),
        }
        m.dirty.pata = false;
    }
    if let Some(e) = err {
        m.status = e;
    } else if ok {
        m.status = rimay_localize::t("wawa-panel-autosave-ok");
    }
}

/// Aplica un cambio a la config del SO (`WawaConfig`) por id de campo (sin
/// persistir: el guardado lo difiere [`flush_saves`]).
fn apply_wawa(m: &mut Model, leaf: &str, value: FieldValue) {
    match leaf {
        "theme_variant" => {
            if let Some(s) = value.as_str() {
                m.cfg.theme_variant = s.to_string();
            }
        }
        "accent" => {
            if let Some(s) = value.as_str() {
                m.cfg.accent = s.to_string();
            }
        }
        "lang" => {
            if let Some(s) = value.as_str() {
                let _ = rimay_localize::set_locale(s);
                m.cfg.lang = s.to_string();
            }
        }
        "timefmt_24h" => {
            if let Some(b) = value.as_bool() {
                m.cfg.timefmt_24h = b;
            }
        }
        "dientes_outside" => {
            if let Some(b) = value.as_bool() {
                m.cfg.dientes_outside = b;
            }
        }
        // Cualquier otro id es un toggle de módulo.
        other => {
            if let Some(b) = value.as_bool() {
                if MODULES.iter().any(|(id, _, _)| *id == other) && m.cfg.module_enabled(other) != b {
                    m.cfg.toggle_module(other);
                }
            }
        }
    }
}

/// Valor de texto actual de un campo de app (para sembrar el buffer al focar).
fn current_text_value(m: &Model, path: &FieldPath) -> String {
    let Some((key, rel)) = split_app(path) else {
        return String::new();
    };
    let schema = match key.as_str() {
        "mirada" => m.mirada.schema(),
        "pata" => m.pata.schema(),
        _ => return String::new(),
    };
    schema
        .find_field(&rel)
        .and_then(|f| f.value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Valor completo de un campo de app (para sembrar el buffer de una celda de
/// lista/tabla al focarla — necesita el agregado entero, no sólo un texto).
fn current_field_value(m: &Model, path: &FieldPath) -> Option<FieldValue> {
    let (key, rel) = split_app(path)?;
    // El keymap no está en el schema de Config: su valor es el buffer de filas,
    // y el selector de perfil el nombre del activo.
    if key == "mirada" && rel.segments().first().map(String::as_str) == Some("atajos") {
        return Some(match rel.leaf() {
            Some("profile") => FieldValue::Text(m.profiles.active().to_string()),
            _ => FieldValue::Table(m.keymap_rows.clone()),
        });
    }
    // Pestaña Perfiles: el toggle «activo» = si esta vista es la aplicada.
    if key == "perfiles" {
        let name = rel.segments().first().cloned().unwrap_or_default();
        return Some(FieldValue::Bool(m.active_vista.as_deref() == Some(name.as_str())));
    }
    let schema = match key.as_str() {
        "mirada" => m.mirada.schema(),
        "pata" => m.pata.schema(),
        _ => return None,
    };
    schema.find_field(&rel).map(|f| f.value.clone())
}

// =====================================================================
// Theme
// =====================================================================

fn theme_from_cfg(cfg: &WawaConfig) -> Theme {
    let canonical = wawa_config::canonical_theme_name(&cfg.theme_variant).unwrap_or("Dark");
    let mut t = Theme::by_name(canonical).unwrap_or_else(Theme::dark);
    if let Some([r, g, b]) = wawa_config::accent_rgb(&cfg.accent) {
        let c = llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(r, g, b, 255);
        t.accent = c;
        t.border_focus = c;
    }
    t
}

// =====================================================================
// Sub-views
// =====================================================================

fn build_header(theme: &Theme) -> View<Msg> {
    let palette = AppHeaderPalette::from_theme(theme);
    app_header(rimay_localize::t("wawa-panel-title"), vec![], &palette)
}

/// El cuerpo, jerarquía de 3 niveles al modo cosmos:
/// `[ sidebar: items de la pestaña activa ] [ pestañas que sobresalen ] [ canvas: contenido del item ]`.
/// La **pestaña** (rail) elige app/categoría; su **sidebar** lista los items
/// (secciones) con su iconito; clic en un item abre su contenido en el **canvas**.
fn build_body(pestanas: &[PanelPestana], pest: usize, model: &Model, theme: &Theme) -> View<Msg> {
    let empty: &[Section] = &[];
    let sections: &[Section] = pestanas.get(pest).map(|p| p.schema.sections.as_slice()).unwrap_or(empty);
    let title = pestanas.get(pest).map(|p| p.title.as_str()).unwrap_or("");
    // Item activo, sólo si está en rango; `None` → el canvas muestra el resumen.
    let sel_item = model.selected_item.filter(|&i| i < sections.len());

    // Canvas: contenido del item activo, o resumen de la pestaña si no hay item.
    let canvas_content = match sel_item.and_then(|i| sections.get(i)) {
        Some(sec) => {
            let one = Schema {
                sections: vec![sec.clone()],
            };
            schema_panel(&one, &model.allichay, theme, VIEWPORT_H, Msg::Allichay)
        }
        None => resumen_view(title, sections, theme),
    };
    let canvas = View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(RAIL_W),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![canvas_content]);

    let rail = rail_overlay(pestanas, pest, theme);
    let center = View::new(Style {
        position: Position::Relative,
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![canvas, rail]);

    // Sidebar acoplable: visible si está abierto y la pestaña tiene items. Es
    // un pane redimensionable (divisor arrastrable); ocultable clickeando la
    // pestaña activa.
    let inner = if model.sidebar_open && !sections.is_empty() {
        let sidebar = sidebar_view(title, sections, sel_item, theme);
        splitter_two(
            Direction::Row,
            sidebar,
            PaneSize::Fixed(model.sidebar_w),
            center,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::SetSidebarWidth(dx)),
                DragPhase::End => None,
            },
            &SplitterPalette::from_theme(theme),
        )
    } else {
        center
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![inner])
}

/// El sidebar de una pestaña: su rótulo + la lista de items (secciones), cada
/// uno con su iconito. Clic en un item lo abre en el canvas.
fn sidebar_view(title: &str, sections: &[Section], sel_item: Option<usize>, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::with_capacity(sections.len() + 1);
    // Rótulo de la pestaña.
    kids.push(
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: length(34.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(12.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(title.to_string(), 13.0, theme.fg_muted, Alignment::Start),
    );
    for (i, sec) in sections.iter().enumerate() {
        kids.push(item_row(i, &sec.icon, &sec.title, sel_item == Some(i), theme));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(SIDEBAR_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
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
    .children(kids)
}

/// Una fila de item del sidebar: iconito + rótulo; el activo lleva fondo
/// resaltado + barra de acento a la izquierda. Clic → abre en el canvas.
fn item_row(i: usize, icon: &str, label: &str, active: bool, theme: &Theme) -> View<Msg> {
    let (bg, fg) = if active {
        (theme.bg_selected, theme.fg_text)
    } else {
        (theme.bg_panel, theme.fg_muted)
    };
    // Celdas con alto auto (≈ alto del texto): la fila las centra verticalmente.
    let mut cells: Vec<View<Msg>> = Vec::with_capacity(3);
    cells.push(
        View::new(Style {
            size: Size {
                width: length(22.0_f32),
                height: Dimension::auto(),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .text_aligned(
            if icon.is_empty() { "·" } else { icon }.to_string(),
            14.0,
            fg,
            Alignment::Center,
        ),
    );
    cells.push(
        View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(label.to_string(), 12.5, fg, Alignment::Start),
    );
    if active {
        cells.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    right: auto(),
                    top: length(6.0_f32),
                    bottom: length(6.0_f32),
                },
                size: Size {
                    width: length(3.0_f32),
                    height: auto(),
                },
                ..Default::default()
            })
            .fill(theme.accent)
            .radius(2.0),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(theme.bg_row_hover)
    .radius(4.0)
    .on_click(Msg::SelectItem(i as u64))
    .children(cells)
}

/// El rail de **pestañas** como overlay absoluto pegado al borde izquierdo del
/// canvas (patrón cosmos `dock_rail_overlay`): las pestañas sobresalen del
/// sidebar hacia el canvas. Clic en una pestaña cambia el sidebar.
fn rail_overlay(pestanas: &[PanelPestana], pest: usize, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = pestanas
        .iter()
        .enumerate()
        .map(|(i, _)| DockRailItem {
            id: i as u64,
            active: i == pest,
        })
        .collect();
    let icons: Vec<String> = pestanas.iter().map(|p| p.icon.clone()).collect();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| tooth_icon(icons.get(id as usize).cloned(), size, color),
        Msg::SelectPestana,
        |_| None,
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            top: length(6.0_f32),
            left: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(RAIL_W),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![rail])
}

/// Resumen del canvas cuando la pestaña no tiene items: nombre de la suite + pista.
/// El resumen del canvas al entrar a una pestaña (antes de elegir item):
/// el nombre de la pestaña + una pista + el listado de sus items.
fn resumen_view(title: &str, sections: &[Section], theme: &Theme) -> View<Msg> {
    let head = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(title.to_string(), 20.0, theme.fg_text, Alignment::Center);

    let hint = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        format!("{} opciones — elegí una a la izquierda", sections.len()),
        12.0,
        theme.fg_muted,
        Alignment::Center,
    );

    let items: Vec<View<Msg>> = sections
        .iter()
        .map(|s| {
            let icon = if s.icon.is_empty() { "·" } else { s.icon.as_str() };
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                format!("{}  {}", icon, s.title),
                12.5,
                theme.fg_muted,
                Alignment::Center,
            )
        })
        .collect();
    let mut kids = vec![head, hint];
    kids.extend(items);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(kids)
}

/// Icono de un diente (emoji que la fuente tenga), color resuelto por el rail.
fn tooth_icon(
    glyph: Option<String>,
    size: f32,
    color: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        glyph.unwrap_or_else(|| "•".to_string()),
        size * 0.9,
        color,
        Alignment::Center,
    )
}

fn build_status(model: &Model, theme: &Theme) -> View<Msg> {
    let status_msg = if model.status.is_empty() {
        rimay_localize::t("wawa-panel-status-hint")
    } else {
        model.status.clone()
    };
    let msg_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .text_aligned(status_msg, 11.5, theme.fg_muted, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(14.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![msg_v])
}

// =====================================================================
// Barra de menú
// =====================================================================

fn menubar_spec<'a>(menu: &'a AppMenu, model: &Model, theme: &'a Theme) -> MenuBarSpec<'a, Msg> {
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: viewport_of(),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

/// Menú principal: Archivo (Salir) · Idioma · Ayuda (Acerca). Sin "Ver": la
/// navegación es por dientes.
fn app_menu() -> AppMenu {
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
            Menu::new(rimay_localize::t("wawa-panel-menu-file"))
                .item(MenuItem::new(rimay_localize::t("wawa-panel-menu-quit"), "file.quit")),
        )
        .menu(
            Menu::new(rimay_localize::t("language"))
                .item(lang_item("Español", "es-PE"))
                .item(lang_item("English", "en-US"))
                .item(lang_item("Runasimi", "qu-PE")),
        )
        .menu(
            Menu::new(rimay_localize::t("wawa-panel-menu-help"))
                .item(MenuItem::new(rimay_localize::t("wawa-panel-about-name"), "help.about")),
        )
}

fn handle_menu_command(model: Model, cmd: &str) -> Model {
    let mut m = model;
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        m.cfg.lang = code.to_string();
        let _ = m.cfg.save();
        return m;
    }
    match cmd {
        "file.quit" => std::process::exit(0),
        "help.about" => {
            m.selected_pest = INFO_DIENTE;
            m.sidebar_open = true;
            m.selected_item = Some(1); // "Acerca de"
            m.allichay.select(1);
            m.status.clear();
        }
        _ => {}
    }
    m
}
