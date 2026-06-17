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

mod animaciones;
mod perfiles;
mod themes;

use std::path::PathBuf;
use std::sync::Arc;

use perfiles::{DesktopProfile, DesktopProfiles};

use llimphi_module_file_picker::{self as picker, PickerAction, PickerMsg, PickerState};

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

/// Índice del panel "Acerca" (último) — para el menú Ayuda (estado/about).
/// Orden: Vista=0, Atajos=1, Animaciones=2, Pata=3, Inicio=4, Sistema=5, Acerca=6.
const INFO_DIENTE: usize = 6;
/// Índice del panel "Vista" (1º) — Perfiles vive ahí; saltamos tras crear/duplicar.
const PERFILES_DIENTE: usize = 0;

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
    distro: String,
    cpu_model: String,
    cpu_cores: usize,
    swap_total_kb: u64,
    swap_free_kb: u64,
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

/// PRETTY_NAME de /etc/os-release (la distro).
fn read_distro() -> String {
    let s = read_proc_file("/etc/os-release");
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("PRETTY_NAME=") {
            return rest.trim().trim_matches('"').to_string();
        }
    }
    "Linux".into()
}

/// Modelo de CPU + cantidad de núcleos lógicos, de /proc/cpuinfo.
fn read_cpu() -> (String, usize) {
    let s = read_proc_file("/proc/cpuinfo");
    let mut model = String::new();
    let mut cores = 0;
    for line in s.lines() {
        if line.starts_with("processor") {
            cores += 1;
        } else if model.is_empty() {
            if let Some(rest) = line.strip_prefix("model name") {
                if let Some((_, v)) = rest.split_once(':') {
                    model = v.trim().to_string();
                }
            }
        }
    }
    if model.is_empty() {
        model = "—".into();
    }
    (model, cores)
}

fn refresh_host(info: &mut HostInfo) {
    info.host = read_hostname();
    info.kernel = read_kernel();
    info.uptime = parse_uptime(&read_proc_file("/proc/uptime"));
    let meminfo = read_proc_file("/proc/meminfo");
    let (total, avail) = parse_meminfo(&meminfo);
    info.mem_total_kb = total;
    info.mem_avail_kb = avail;
    info.swap_total_kb = meminfo_field(&meminfo, "SwapTotal:");
    info.swap_free_kb = meminfo_field(&meminfo, "SwapFree:");
    info.load = parse_loadavg(&read_proc_file("/proc/loadavg"));
    info.distro = read_distro();
    let (m, c) = read_cpu();
    info.cpu_model = m;
    info.cpu_cores = c;
}

/// Lee un campo `KB` de /proc/meminfo por su prefijo (ej. "SwapTotal:").
fn meminfo_field(meminfo: &str, prefix: &str) -> u64 {
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix(prefix) {
            return rest.trim().split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }
    0
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
    /// Biblioteca de **perfiles de escritorio completos** (custom, editables,
    /// creables y duplicables). Cada perfil = foto de config mirada + keymap +
    /// barra pata. El activo (`dprofiles.active`) es el que se está editando en
    /// las pestañas mirada · pata · Atajos.
    dprofiles: DesktopProfiles,
    pata: pata_core::Config,
    /// Reglas de ventana de mirada (estilo Hyprland windowrule): por clase
    /// (`app_id`) y/o título → escritorio/flotante/fullscreen/tamaño. Editables
    /// en la pestaña «Reglas» de Vista; se persisten en `rules.ron`.
    rules: mirada_brain::rules::Rules,
    rules_path: Option<PathBuf>,
    /// Biblioteca de **themes** (look reusable: apariencia+teselado+decoración).
    /// El perfil activo referencia uno; la pestaña Themes edita el referenciado.
    themes: themes::Themes,
    /// Biblioteca de **conjuntos de animación** (transición/slide/Prezi). El
    /// perfil activo referencia uno; el panel Animaciones edita el referenciado.
    animaciones: animaciones::Animations,
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
    /// Diálogo de abrir archivo (elegir wallpaper). `None` = cerrado. Cuando está
    /// abierto, `picker_paths` son los archivos candidatos y `picker_root` la base.
    picker: Option<PickerState>,
    picker_paths: Vec<PathBuf>,
    picker_root: PathBuf,
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
    /// Biblioteca de perfiles de escritorio (`perfiles-escritorio.ron`).
    dprofiles: bool,
    /// Reglas de ventana (`rules.ron`).
    rules: bool,
    /// Biblioteca de themes (`themes.ron`).
    themes: bool,
    /// Biblioteca de conjuntos de animación (`animaciones.ron`).
    animaciones: bool,
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
    /// Mensaje del diálogo de archivos (elegir wallpaper).
    Picker(PickerMsg),
    /// Editor visual 2D del Prezi: mover el escritorio `i` a la celda (col, fila).
    PreziMove(usize, i32, i32),
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

        let handle_clone = handle.clone();
        let watcher = ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::ConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("wawa-panel · watcher: {e}"))
        .ok();

        build_model(watcher)
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
            Msg::Picker(pm) => {
                // Tomamos el estado por valor para no chocar el borrow con
                // picker_paths/sync_active_profile.
                if let Some(mut st) = m.picker.take() {
                    match picker::apply(&mut st, pm, &m.picker_paths, &m.picker_root) {
                        PickerAction::Open(path) => {
                            m.mirada.wallpaper_path = path.to_string_lossy().to_string();
                            m.dirty.mirada = true;
                            sync_active_profile(&mut m);
                            m.save_in = SAVE_DELAY_TICKS;
                            m.status = format!("fondo: {}", path.display());
                            // queda cerrado (no devolvemos el estado)
                        }
                        PickerAction::Close => { /* cerrado */ }
                        PickerAction::None => m.picker = Some(st),
                    }
                }
            }
            Msg::PreziMove(i, col, row) => {
                // Asegura N celdas y mueve el escritorio i; edita el perfil activo.
                let n = mirada_brain::action::WORKSPACE_COUNT;
                if m.mirada.overview_geometry.len() < n {
                    m.mirada.overview_geometry = m.mirada.overview_geometry_for(n);
                }
                if let Some(slot) = m.mirada.overview_geometry.get_mut(i) {
                    *slot = (col.max(0), row.max(0));
                    m.dirty.mirada = true;
                    sync_active_profile(&mut m);
                    m.save_in = SAVE_DELAY_TICKS;
                    m.status = format!("escritorio {} → ({col}, {row})", i + 1);
                }
            }
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
        // Diálogo de archivos abierto → todas las teclas al picker.
        if let Some(st) = &model.picker {
            return picker::on_key(st, event).map(Msg::Picker);
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
        // El diálogo de archivos tiene prioridad: modal centrado con scrim.
        if let Some(st) = &model.picker {
            let pal = llimphi_module_file_picker::PickerPalette::from_theme(&theme);
            let panel = picker::view(st, &model.picker_paths, &model.picker_root, &pal, Msg::Picker);
            let caja = View::new(Style {
                size: Size { width: length(720.0_f32), height: length(460.0_f32) },
                ..Default::default()
            })
            .children(vec![panel]);
            let scrim = View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: length(0.0_f32),
                    top: length(0.0_f32),
                    right: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(llimphi_ui::llimphi_raster::peniko::Color::from_rgba8(0, 0, 0, 150))
            .on_click(Msg::Picker(PickerMsg::Close))
            .children(vec![caja]);
            return Some(scrim);
        }
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

/// Construye el Model leyendo todo de disco. Lo usan `init` (con watcher, vía la
/// app viva) y el modo `--shot` (headless, sin Handle ni watcher).
fn build_model(watcher: Option<ConfigWatcher>) -> Model {
    let cfg = WawaConfig::load();
    let _ = rimay_localize::set_locale(&cfg.lang);

    let mut host = HostInfo::default();
    refresh_host(&mut host);

    let mirada_path = mirada_brain::Config::default_path();
    let mirada = mirada_path
        .as_deref()
        .map(mirada_brain::Config::load_or_default)
        .unwrap_or_default();
    let pata = pata_config::load();

    let keymap_path = mirada_brain::Keymap::default_path();
    let keymap_rows = keymap_path
        .as_deref()
        .map(mirada_brain::Keymap::load_or_init)
        .unwrap_or_default()
        .to_rows();

    let profiles_path = mirada_brain::KeymapProfiles::default_path();
    let profiles = profiles_path
        .as_deref()
        .map(mirada_brain::KeymapProfiles::load_or_init)
        .unwrap_or_default();

    let dprofiles = DesktopProfiles::load_or_seed(&mirada);

    let rules_path = mirada_brain::rules::Rules::default_path();
    let rules = rules_path
        .as_deref()
        .map(mirada_brain::rules::Rules::load_or_default)
        .unwrap_or_default();

    let themes = themes::Themes::load_or_seed(&cfg.theme_variant, &cfg.accent);
    let animaciones = animaciones::Animations::load_or_seed();

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
        dprofiles,
        pata,
        rules,
        rules_path,
        themes,
        animaciones,
        allichay: AllichayState::new(),
        host,
        status: String::new(),
        dirty: SaveDirty::default(),
        save_in: 0,
        _config_watcher: watcher,
        menu_open: None,
        menu_active: usize::MAX,
        menu_anim: Tween::idle(1.0),
        picker: None,
        picker_paths: Vec::new(),
        picker_root: PathBuf::from("/"),
    }
}

fn main() {
    rimay_localize::init();
    // Modo captura headless: `wawa-panel --shot <png> [pestaña] [item]`. Arma el
    // Model, abre esa pestaña/item y renderiza la vista a PNG, sin abrir ventana.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--shot") {
        let out = args.get(pos + 1).cloned().unwrap_or_else(|| "/tmp/panel.png".into());
        let pest = args.get(pos + 2).and_then(|s| s.parse::<usize>().ok());
        let item = args.get(pos + 3).and_then(|s| s.parse::<usize>().ok());
        shot_panel(&out, pest, item);
        return;
    }
    // `--apply-active`: vuelca el perfil ACTIVO a la config viva (config.ron +
    // keymap.ron + launcher.toml + theme) y sale, sin abrir ventana. Lo corre el
    // script de sesión al arrancar para que el escritorio arranque coherente con
    // el perfil activo (antes un launcher.toml viejo —p.ej. de mac— pisaba el
    // default y «arrancaba algo random que no era el perfil activo»).
    if args.iter().any(|a| a == "--apply-active") {
        let mut m = build_model(None);
        let active = m.dprofiles.active.clone();
        if !active.is_empty() {
            activate_profile(&mut m, &active);
        }
        // Asegura un wallpaper de fábrica si el escritorio arranca sin fondo
        // (color plano feo). Idempotente.
        if ensure_default_wallpaper(&mut m) {
            eprintln!("wawa-panel: wallpaper por defecto generado");
        }
        flush_saves(&mut m);
        eprintln!("wawa-panel: perfil activo «{active}» aplicado a la config viva");
        return;
    }
    llimphi_ui::run::<Panel>();
}

/// Renderiza la vista del panel a un PNG (headless), abriendo la `pest`/`item`
/// pedidos. Misma maquinaria que los `*_shot` de pata.
fn shot_panel(out: &str, pest: Option<usize>, item: Option<usize>) {
    use llimphi_ui::llimphi_compositor::{measure_text_node, mount, paint};
    use llimphi_ui::llimphi_hal::{wgpu, Hal};
    use llimphi_ui::llimphi_layout::taffy;
    use llimphi_ui::llimphi_layout::LayoutTree;
    use llimphi_ui::llimphi_raster::peniko::Color;
    use llimphi_ui::llimphi_raster::{vello, Renderer};
    use llimphi_ui::llimphi_text::Typesetter;

    let mut model = build_model(None);
    if let Some(p) = pest {
        model.selected_pest = p;
        model.sidebar_open = true;
    }
    if let Some(i) = item {
        model.selected_item = Some(i);
        model.allichay.select(i);
    }
    // WAWA_SHOT_PICKER=1 abre el diálogo de archivos para capturarlo.
    if std::env::var_os("WAWA_SHOT_PICKER").is_some() {
        open_wallpaper_picker(&mut model);
    }
    let (w, h) = Panel::initial_size();
    // Si hay overlay (menú o diálogo), lo componemos encima de la vista base.
    let base = Panel::view(&model);
    let view = match Panel::view_overlay(&model) {
        Some(ov) => View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .children(vec![base, ov]),
        None => base,
    };

    let hal = pollster::block_on(Hal::new(None)).expect("hal");
    let mut renderer = Renderer::new(&hal).expect("renderer");
    let mut layout = LayoutTree::new();
    let mounted = mount(&mut layout, view);
    let mut ts = Typesetter::new();
    let computed = {
        let tmap = &mounted.text_measures;
        layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(&mut ts, tm, known, avail),
                    None => taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    let mut scene = vello::Scene::new();
    paint(&mut scene, &mounted, &computed, &mut ts, None, None);

    let fmt = wgpu::TextureFormat::Rgba8Unorm;
    let target = hal.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("panel-shot"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: fmt,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let tview = target.create_view(&wgpu::TextureViewDescriptor::default());
    renderer
        .render_to_view(&hal, &scene, &tview, w, h, Color::from_rgba8(20, 20, 28, 255))
        .expect("render_to_view");
    write_png(&hal, &target, w, h, out);
    eprintln!("wawa-panel: {out} ({w}x{h})");
}

fn write_png(hal: &llimphi_ui::llimphi_hal::Hal, target: &llimphi_ui::llimphi_hal::wgpu::Texture, w: u32, h: u32, path: &str) {
    use llimphi_ui::llimphi_hal::wgpu;
    use std::io::BufWriter;
    let unpadded = (w * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buf = hal.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * h as usize) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut enc = hal.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    hal.queue.submit(std::iter::once(enc.finish()));
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    hal.device.poll(wgpu::PollType::wait_indefinitely());
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h as usize {
        let s = row * padded;
        pixels.extend_from_slice(&data[s..s + unpadded]);
    }
    drop(data);
    buf.unmap();
    let file = std::fs::File::create(path).expect("png");
    let mut enc = png::Encoder::new(BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&pixels).unwrap();
}

/// Genera un wallpaper por defecto: un degradé diagonal del color `base` (el
/// acento del theme) a una versión oscurecida, con una viñeta sutil. 1920×1080
/// RGBA → PNG en `path`. Es el fondo de fábrica para que un escritorio recién
/// instalado no arranque con un color plano.
fn write_default_wallpaper(path: &std::path::Path, base: [u8; 4]) -> std::io::Result<()> {
    const W: u32 = 1920;
    const H: u32 = 1080;
    // Esquina superior-izquierda = base aclarado; inferior-derecha = base
    // oscurecido. Mezcla lineal sobre la diagonal.
    let claro = |c: u8| ((c as f32 * 0.55 + 255.0 * 0.12).min(255.0)) as f32;
    let oscuro = |c: u8| (c as f32 * 0.16) as f32;
    let (r0, g0, b0) = (claro(base[0]), claro(base[1]), claro(base[2]));
    let (r1, g1, b1) = (oscuro(base[0]), oscuro(base[1]), oscuro(base[2]));
    let mut px = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let t = (x as f32 / W as f32 + y as f32 / H as f32) * 0.5;
            let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
            px.push(lerp(r0, r1));
            px.push(lerp(g0, g1));
            px.push(lerp(b0, b1));
            px.push(255);
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, W, H);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut wr = enc.write_header().map_err(std::io::Error::other)?;
    wr.write_image_data(&px).map_err(std::io::Error::other)
}

/// Si el perfil/theme activo no tiene wallpaper (ni imagen fija ni carpeta de
/// rotación), genera uno por defecto y lo fija. Idempotente: si ya existe el
/// archivo generado, no lo reescribe. Devuelve `true` si tocó la config.
fn ensure_default_wallpaper(m: &mut Model) -> bool {
    if !m.mirada.wallpaper_path.trim().is_empty() || !m.mirada.wallpaper_dir.trim().is_empty() {
        return false; // ya hay fondo elegido (o rotación): respetarlo
    }
    let Some(cfgdir) = mirada_brain::Config::default_path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return false;
    };
    let wall = cfgdir.join("default-wall.png");
    if !wall.exists() {
        if write_default_wallpaper(&wall, m.mirada.border_focus).is_err() {
            return false;
        }
    }
    m.mirada.wallpaper_path = wall.to_string_lossy().to_string();
    m.dirty.mirada = true;
    true
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

/// Arma el rail en **cinco paneles** (la IA pedida 2026-06-17): cada panel es un
/// diente del rail; sus secciones son las pestañas del sidebar; el contenido de
/// la pestaña activa va al canvas. Las secciones conservan su prefijo de ruteo
/// (`mirada::`/`pata::`/`wawa::`/`perfiles`/`barras`), así editar cualquiera
/// aplica al destino correcto (las `mirada::`/`pata::` editan el PERFIL ACTIVO y
/// `sync_active_profile` las guarda).
///
/// - **Vista**: Perfiles · Themes (apariencia+teselado+decoración) · Wallpapers ·
///   Vistas (espacial+monitores) · Interfaz/animaciones/dientes · Terminal · Atajos.
/// - **Pata**: lista de barras + config de cada superficie.
/// - **Inicio**: arranque (arje como init, DM) · autostarts.
/// - **Sistema**: idioma/hora · módulos.
/// - **Acerca**: menú raíz (openbox) · estado del equipo · acerca.
///
/// Pendiente (siguientes iteraciones): Themes como biblioteca (add/dup/rename),
/// Reglas-hyprland, reuso del editor Prezi 2D de pluma para mapear workspaces,
/// sonido/teclado/mouse, y la relación many2many barras↔perfiles. Íconos SVG
/// a color para los dientes (hoy glifos).
fn pestanas(m: &Model) -> Vec<PanelPestana> {
    let mirada_on = m.cfg.module_enabled("mirada");
    let pata_on = m.cfg.module_enabled("pata");
    // Secciones de mirada (config del perfil activo), prefijadas; las repartimos
    // por id corto entre las pestañas de Vista.
    let mir: Vec<Section> = if mirada_on {
        prefix_schema(m.mirada.schema(), "mirada").sections
    } else {
        Vec::new()
    };
    let take = |id: &str| mir.iter().find(|s| s.id == format!("mirada::{id}")).cloned();

    // ---- Panel VISTA ----
    let mut vista = Schema::new();
    for s in perfiles_schema(m).sections {
        vista.sections.push(s); // Perfiles (lista limpia: acciones + perfiles)
    }
    // Themes: biblioteca + edición del theme del perfil activo (apariencia +
    // teselado + decoración). El perfil ya NO es dueño de teselado/decoración.
    for s in themes_schema(m).sections {
        vista.sections.push(s);
    }
    vista.sections.push(wallpaper_section(m)); // Wallpapers (imagen + automático, unificado)
    if let Some(s) = take("vista_espacial") {
        vista.sections.push(s); // Vistas: Prezi
    }
    if let Some(s) = take("monitores") {
        vista.sections.push(s); // Vistas: monitores/workspaces
    }
    vista.sections.push(interfaz_section(&m.cfg)); // Animaciones/interfaz/dientes
    if let Some(s) = take("terminal") {
        vista.sections.push(s); // Terminal dropdown
    }
    vista.sections.push(reglas_section(&m.rules)); // Reglas (hyprland windowrule)

    // ---- Panel ATAJOS (su propio diente) ----
    // Conjuntos de atajos reusables (tab 1 = lista, tab 2 = teclas), mismo patrón
    // que Themes. Antes era una sección suelta dentro de Vista.
    let atajos = if mirada_on { atajos_schema(m) } else { Schema::new() };

    // ---- Panel ANIMACIONES (su propio diente) ----
    // Conjuntos de animación (tab 1 = lista, tab 2 = parámetros), mismo patrón.
    let animaciones = animaciones_schema(m);

    // ---- Panel PATA ----
    let mut pata = Schema::new();
    if pata_on {
        pata.sections.push(barras_section(&m.pata));
        for s in prefix_schema(m.pata.schema(), "pata").sections {
            pata.sections.push(s);
        }
    }

    // ---- Panel INICIO ----
    let mut inicio = Schema::new();
    inicio.sections.push(arranque_section());
    inicio.sections.push(autostart_section());

    // ---- Panel SISTEMA ----
    let mut sistema = Schema::new();
    sistema.sections.push(sonido_section());
    sistema.sections.push(teclado_section(&m.mirada));
    sistema.sections.push(puntero_section(&m.mirada));
    sistema.sections.push(idioma_section(&m.cfg));
    sistema.sections.push(modulos_section(&m.cfg));

    // ---- Panel ACERCA ----
    let mut acerca = Schema::new();
    if let Some(s) = take("menu") {
        acerca.sections.push(s); // menú raíz (openbox)
    }
    for s in info_schema(&m.host).sections {
        acerca.sections.push(s); // estado del equipo + acerca
    }
    acerca.sections.push(monitores_section()); // pantallas conectadas (DRM)

    vec![
        PanelPestana { title: "Vista".into(), icon: "✦".into(), schema: vista },
        PanelPestana { title: "Atajos".into(), icon: "⌨".into(), schema: atajos },
        PanelPestana { title: "Animaciones".into(), icon: "✨".into(), schema: animaciones },
        PanelPestana { title: "Pata".into(), icon: "🎛".into(), schema: pata },
        PanelPestana { title: "Inicio".into(), icon: "⏻".into(), schema: inicio },
        PanelPestana { title: "Sistema".into(), icon: "⚙".into(), schema: sistema },
        PanelPestana { title: "Acerca".into(), icon: "🖥".into(), schema: acerca },
    ]
}

/// Prefija el id de cada sección con el destino de ruteo (`"mirada::teselado"`),
/// para que el `FieldPath` de cada campo identifique a qué app aplicar.
fn prefix_schema(mut schema: Schema, target: &str) -> Schema {
    for sec in &mut schema.sections {
        sec.id = format!("{target}::{}", sec.id);
    }
    schema
}

/// La pestaña **Perfiles**: una sección por **perfil de escritorio completo**
/// de la biblioteca (custom, editable, creable y duplicable; sembrada de las 8
/// vistas de fábrica). Activar un perfil vuelca TODO de inmediato (config.ron +
/// keymap.ron + launcher.toml, que el compositor y pata recargan en caliente);
/// mientras está activo, su config queda editable en mirada · pata · Atajos y
/// cada cambio se guarda DENTRO del perfil. El activo se marca con ●. Crear y
/// duplicar viven en el menú «Perfiles».
/// Un glifo distinto por perfil (según su nombre conocido), para que no se vean
/// todos iguales en el rail. Cae a un genérico para perfiles custom.
#[allow(dead_code)] // Perfiles ahora es una lista única (dropdown), sin diente por perfil.
fn icono_perfil(name: &str) -> &'static str {
    match name {
        "mirada" => "◉",
        "windows-xp" => "⊞",
        "mac" => "◆",
        "kde" => "≡",
        "solaris" => "◧",
        "hyprland" => "❖",
        "dwm" => "▦",
        _ => "★",
    }
}

fn perfiles_schema(m: &Model) -> Schema {
    use allichay::{EnumOption, Field};
    let activo = if m.dprofiles.active.is_empty() {
        "—".to_string()
    } else {
        m.dprofiles.active.clone()
    };
    // Una sola lista en el canvas (como Themes): un selector con TODOS los
    // perfiles (el activo con «●») + acciones. No una pestaña por perfil.
    let opts: Vec<EnumOption> = m
        .dprofiles
        .names()
        .into_iter()
        .map(|n| {
            let label = if n == m.dprofiles.active { format!("● {n}") } else { n.clone() };
            EnumOption::new(n, label)
        })
        .collect();
    Schema::new().section(
        Section::new("perfiles::acciones", "Perfiles")
            .icon("👤")
            .help(
                "Perfiles de escritorio completos (keymap + barra + theme + fondo). \
                 Elegí el activo; las demás pestañas de Vista editan ESE perfil. \
                 Crear/duplicar/renombrar/eliminar; «rescatar» re-siembra los \
                 perfiles de fábrica que falten (por si renombraste o borraste uno).",
            )
            .field(Field::radio("usar", "Perfil activo", m.dprofiles.active.clone(), opts))
            .field(Field::button("crear", "Crear perfil (desde el actual)"))
            .field(Field::button("duplicar", format!("Duplicar «{activo}»")))
            .field(Field::text("renombrar", format!("Renombrar «{activo}» a…"), ""))
            .field(Field::button("eliminar", format!("Eliminar «{activo}»")))
            .field(Field::button("rescatar", "Rescatar perfiles de fábrica faltantes")),
    )
}

/// Inyecta las secciones de config del perfil activo —teselado/decoración/fondo/
/// zonas (mirada) + atajos + barra (pata)— DEBAJO de él en el sidebar, con el
/// título indentado (▸). Conservan su prefijo `mirada::`/`pata::` para que el
/// ruteo las aplique al perfil activo (no a la pestaña «perfiles»).
#[allow(dead_code)] // la config del perfil activo ahora vive en pestañas dedicadas de Vista.
fn anidar_config_perfil(mut schema: Schema, m: &Model) -> Schema {
    let sangrar = |s: &mut Section| s.title = format!("  ▸ {}", s.title);
    let mut mir = prefix_schema(m.mirada.schema(), "mirada");
    mir.sections.iter_mut().for_each(sangrar);
    schema.sections.extend(mir.sections);
    let mut atajos = keymap_section(&m.keymap_rows);
    sangrar(&mut atajos);
    schema.sections.push(atajos);
    let mut pata = prefix_schema(m.pata.schema(), "pata");
    pata.sections.iter_mut().for_each(sangrar);
    schema.sections.extend(pata.sections);
    // Lista de barras (agregar/borrar/nombrar/prender-apagar), también anidada.
    let mut barras = barras_section(&m.pata);
    sangrar(&mut barras);
    schema.sections.push(barras);
    schema
}

fn kind_slug(k: pata_core::SurfaceKind) -> &'static str {
    use pata_core::SurfaceKind::*;
    match k {
        Bar => "bar",
        Sidebar => "sidebar",
        Dock => "dock",
        Background => "background",
        Panel => "panel",
    }
}
fn anchor_slug(a: pata_core::Anchor) -> &'static str {
    use pata_core::Anchor::*;
    match a {
        Top => "top",
        Bottom => "bottom",
        Left => "left",
        Right => "right",
    }
}
#[allow(dead_code)]
fn parse_kind(s: &str) -> pata_core::SurfaceKind {
    use pata_core::SurfaceKind::*;
    match s.trim().to_lowercase().as_str() {
        "sidebar" => Sidebar,
        "dock" => Dock,
        "background" | "fondo" => Background,
        "panel" => Panel,
        _ => Bar,
    }
}
#[allow(dead_code)]
fn parse_anchor(s: &str) -> pata_core::Anchor {
    use pata_core::Anchor::*;
    match s.trim().to_lowercase().as_str() {
        "bottom" | "abajo" => Bottom,
        "left" | "izq" | "izquierda" => Left,
        "right" | "der" | "derecha" => Right,
        _ => Top,
    }
}
#[allow(dead_code)]
fn es_activa(s: &str) -> bool {
    !matches!(s.trim().to_lowercase().as_str(), "no" | "false" | "0" | "off" | "")
}

/// Sección «Barras»: las barras de pata como **lista** (no tabla). Cada barra
/// se prende/apaga por su cuenta (varias activas a la vez), se nombra, se
/// duplica y se borra. Su posición/grosor/autohide viven en la pestaña
/// «Superficie N» de cada una. Las barras se guardan DENTRO del perfil activo
/// (por eso «aparecen» al cambiar de perfil — cada perfil trae las suyas).
fn barras_section(pata: &pata_core::Config) -> Section {
    use allichay::Field;
    let mut sec = Section::new("barras::lista", "Barras").icon("▭").help(
        "Las barras de pata, como lista: prendé varias a la vez, nombralas, \
         duplicá o borrá cada una. Posición/grosor/autohide de cada barra están \
         en su pestaña «Superficie N». Se guardan dentro del perfil activo.",
    );
    for (i, s) in pata.surfaces.iter().enumerate() {
        let nombre = if s.name.trim().is_empty() {
            format!("{} {}", kind_slug(s.kind), anchor_slug(s.anchor))
        } else {
            s.name.clone()
        };
        sec = sec
            .field(Field::toggle(format!("on_{i}"), format!("Activa · {nombre}"), s.enabled))
            .field(Field::text(format!("name_{i}"), "    Nombre", s.name.clone()))
            .field(Field::button(format!("dup_{i}"), "    Duplicar"))
            .field(Field::button(format!("del_{i}"), "    Borrar"));
    }
    sec.field(Field::button("agregar", "＋ Agregar barra"))
}

/// Sección «Reglas» (estilo Hyprland windowrule): tabla agregable/borrable de
/// reglas de ventana del perfil/sistema (`rules.ron`).
fn reglas_section(rules: &mirada_brain::rules::Rules) -> Section {
    use allichay::{Column, Field};
    let rows: Vec<Vec<String>> = rules
        .list()
        .iter()
        .map(|r| {
            vec![
                r.app_id.clone(),
                r.title.clone(),
                if r.workspace == 0 { String::new() } else { r.workspace.to_string() },
                if r.floating { "sí".into() } else { "no".into() },
                if r.fullscreen { "sí".into() } else { "no".into() },
                if r.size.0 > 0 { r.size.0.to_string() } else { String::new() },
                if r.size.1 > 0 { r.size.1.to_string() } else { String::new() },
            ]
        })
        .collect();
    Section::new("reglas::lista", "Reglas")
        .icon("▦")
        .help(
            "Reglas de ventana (estilo Hyprland windowrule). Casan por CLASE \
             (app_id) y/o TÍTULO (subcadena, sin distinguir mayúsculas; vacío = \
             cualquiera) y aplican: Escr. (1-9; vacío = no mover), Flota, Pantalla \
             (completa) y tamaño Ancho×Alto px (sólo si flota). +/− agrega/borra.",
        )
        .field(Field::table(
            "lista",
            "Reglas",
            vec![
                Column::new("clase", "Clase"),
                Column::new("titulo", "Título"),
                Column::new("escritorio", "Escr."),
                Column::new("flota", "Flota"),
                Column::new("pantalla", "Pantalla"),
                Column::new("ancho", "Ancho"),
                Column::new("alto", "Alto"),
            ],
            rows,
        ))
}

/// Reconstruye `m.rules` desde la tabla de reglas.
fn apply_reglas_table(m: &mut Model, rows: &[Vec<String>]) {
    let yes = |s: &str| {
        matches!(s.trim().to_lowercase().as_str(), "sí" | "si" | "true" | "1" | "on")
    };
    let num = |s: &str| s.trim().parse::<i32>().unwrap_or(0).max(0);
    let list: Vec<mirada_brain::rules::Rule> = rows
        .iter()
        .map(|row| {
            let g = |i: usize| row.get(i).map(String::as_str).unwrap_or("");
            mirada_brain::rules::Rule {
                app_id: g(0).trim().to_string(),
                title: g(1).trim().to_string(),
                workspace: g(2).trim().parse::<usize>().unwrap_or(0),
                floating: yes(g(3)),
                fullscreen: yes(g(4)),
                size: (num(g(5)), num(g(6))),
            }
        })
        .collect();
    m.rules = mirada_brain::rules::Rules::new(list);
    m.dirty.rules = true;
    m.save_in = SAVE_DELAY_TICKS;
}

/// El nombre del theme que usa el perfil activo (acotado a uno existente).
fn active_theme_name(m: &Model) -> String {
    let want = m.dprofiles.get(&m.dprofiles.active).map(|p| p.theme.clone()).unwrap_or_default();
    if m.themes.get(&want).is_some() {
        want
    } else {
        m.themes.names().first().cloned().unwrap_or_default()
    }
}

/// Vuelca el theme del perfil activo a la config viva (teselado+decoración a
/// mirada, apariencia a wawa) y marca para persistir. Lo llaman las ediciones
/// del theme y el cambio de theme del perfil.
fn apply_active_theme(m: &mut Model) {
    let name = active_theme_name(m);
    if let Some(t) = m.themes.get_or_default(&name).cloned() {
        t.apply_to(&mut m.mirada);
        m.cfg.theme_variant = t.theme_variant.clone();
        m.cfg.accent = t.accent.clone();
        m.dirty.mirada = true;
        m.dirty.wawa = true;
    }
}

/// La pestaña **Themes**: biblioteca (elegir/crear/duplicar/renombrar/eliminar)
/// + edición del theme que usa el perfil activo (apariencia+teselado+decoración).
fn themes_schema(m: &Model) -> Schema {
    use allichay::{EnumOption, Field};
    let t = rimay_localize::t;
    let active_profile = m.dprofiles.active.clone();
    let name = active_theme_name(m);
    let opts: Vec<EnumOption> =
        m.themes.names().into_iter().map(|n| EnumOption::new(n.clone(), n)).collect();
    let mut schema = Schema::new().section(
        Section::new("theme::acciones", "Themes")
            .icon("🎨")
            .help(
                "El look reusable (apariencia + teselado + decoración), \
                 perpendicular a los perfiles. El perfil activo USA un theme; \
                 editarlo afecta a todos los perfiles que lo referencian.",
            )
            .field(Field::radio(
                "usar",
                format!("Theme de «{active_profile}»"),
                name.clone(),
                opts,
            ))
            .field(Field::button("crear", "Crear theme (desde el look actual)"))
            .field(Field::button("duplicar", format!("Duplicar «{name}»")))
            .field(Field::text("renombrar", format!("Renombrar «{name}» a…"), ""))
            .field(Field::button("eliminar", format!("Eliminar «{name}»"))),
    );
    if let Some(theme) = m.themes.get(&name) {
        // Apariencia del theme.
        schema = schema.section(
            Section::new("theme::apariencia", t("wawa-panel-cat-appearance"))
                .icon("◐")
                .field(Field::dropdown(
                    "variant",
                    t("wawa-panel-label-variant"),
                    theme.theme_variant.clone(),
                    THEME_VARIANTS.iter().map(|(id, k)| EnumOption::new(*id, t(k))).collect(),
                ))
                .field(Field::dropdown(
                    "accent",
                    t("wawa-panel-label-accent"),
                    theme.accent.clone(),
                    ACCENTS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
                )),
        );
        // Teselado + Decoración: reusamos las secciones de mirada sobre una
        // config temporal con el theme aplicado (DRY), re-prefijadas a `theme::`.
        let mut tmp = m.mirada.clone();
        theme.apply_to(&mut tmp);
        for sec in tmp.schema().sections {
            if sec.id == "teselado" || sec.id == "decoracion" {
                let mut s = sec;
                s.id = format!("theme::{}", s.id);
                schema = schema.section(s);
            }
        }
    }
    schema
}

/// Aplica una edición de la pestaña Themes (`rel` ya viene sin el `theme::`).
fn apply_theme(m: &mut Model, rel: &FieldPath, value: FieldValue) {
    let active_profile = m.dprofiles.active.clone();
    let name = active_theme_name(m);
    let sect = rel.segments().first().cloned().unwrap_or_default();
    match sect.as_str() {
        "acciones" => match rel.leaf() {
            Some("usar") => {
                if let Some(sel) = value.as_str() {
                    if let Some(p) = m.dprofiles.profiles.get_mut(&active_profile) {
                        p.theme = sel.to_string();
                    }
                    m.dirty.dprofiles = true;
                    apply_active_theme(m);
                }
            }
            Some("crear") if value.as_bool() == Some(true) => {
                let theme = themes::Theme::from_config(&m.mirada, &m.cfg.theme_variant, &m.cfg.accent);
                let nuevo = m.themes.create(theme, "theme nuevo");
                if let Some(p) = m.dprofiles.profiles.get_mut(&active_profile) {
                    p.theme = nuevo.clone();
                }
                m.dirty.themes = true;
                m.dirty.dprofiles = true;
                m.status = format!("theme «{nuevo}» creado");
            }
            Some("duplicar") if value.as_bool() == Some(true) => {
                if let Some(nuevo) = m.themes.duplicate(&name) {
                    if let Some(p) = m.dprofiles.profiles.get_mut(&active_profile) {
                        p.theme = nuevo.clone();
                    }
                    m.dirty.themes = true;
                    m.dirty.dprofiles = true;
                    m.status = format!("theme «{nuevo}» duplicado");
                }
            }
            Some("renombrar") => {
                if let Some(to) = value.as_str() {
                    let to = to.trim().to_string();
                    if !to.is_empty() && m.themes.rename(&name, &to) {
                        for p in m.dprofiles.profiles.values_mut() {
                            if p.theme == name {
                                p.theme = to.clone();
                            }
                        }
                        m.dirty.themes = true;
                        m.dirty.dprofiles = true;
                        m.status = format!("theme renombrado a «{to}»");
                    }
                }
            }
            Some("eliminar") if value.as_bool() == Some(true) => {
                if m.themes.names().len() > 1 {
                    m.themes.remove(&name);
                    let fallback = m.themes.names().first().cloned().unwrap_or_default();
                    for p in m.dprofiles.profiles.values_mut() {
                        if p.theme == name {
                            p.theme = fallback.clone();
                        }
                    }
                    m.dirty.themes = true;
                    m.dirty.dprofiles = true;
                    apply_active_theme(m);
                    m.status = format!("theme «{name}» eliminado");
                } else {
                    m.status = "no podés eliminar el último theme".into();
                }
            }
            _ => {}
        },
        "apariencia" => {
            if let Some(theme) = m.themes.themes.get_mut(&name) {
                match (rel.leaf(), value.as_str()) {
                    (Some("variant"), Some(v)) => theme.theme_variant = v.to_string(),
                    (Some("accent"), Some(v)) => theme.accent = v.to_string(),
                    _ => {}
                }
                m.dirty.themes = true;
                apply_active_theme(m);
            }
        }
        "teselado" | "decoracion" => {
            // Reusa el `apply` de mirada sobre una config temporal con el theme
            // aplicado, y re-extrae el theme — así no duplicamos la lógica.
            if let Some(cur) = m.themes.get(&name).cloned() {
                let mut tmp = m.mirada.clone();
                cur.apply_to(&mut tmp);
                if tmp.apply(rel, value).is_ok() {
                    let nt = themes::Theme::from_config(&tmp, &cur.theme_variant, &cur.accent);
                    m.themes.set(&name, nt);
                    m.dirty.themes = true;
                    apply_active_theme(m);
                }
            }
        }
        _ => {}
    }
}

/// Aplica una acción de la lista de Barras (`leaf` = `on_<i>` / `name_<i>` /
/// `dup_<i>` / `del_<i>` / `agregar`). Las barras viven en el perfil activo.
fn apply_barras_list(m: &mut Model, leaf: &str, value: FieldValue) {
    let idx = |p: &str| leaf.strip_prefix(p).and_then(|s| s.parse::<usize>().ok());
    let mut changed = true;
    if leaf == "agregar" {
        if value.as_bool() == Some(true) {
            let mut s = pata_core::Surface::bar(pata_core::Anchor::Top);
            s.name = format!("barra {}", m.pata.surfaces.len() + 1);
            m.pata.surfaces.push(s);
        } else {
            changed = false;
        }
    } else if let Some(i) = idx("on_") {
        if let Some(s) = m.pata.surfaces.get_mut(i) {
            s.enabled = value.as_bool().unwrap_or(s.enabled);
        }
    } else if let Some(i) = idx("name_") {
        if let (Some(s), Some(v)) = (m.pata.surfaces.get_mut(i), value.as_str()) {
            s.name = v.to_string();
        }
    } else if let Some(i) = idx("dup_") {
        if value.as_bool() == Some(true) {
            if let Some(s) = m.pata.surfaces.get(i).cloned() {
                let mut c = s;
                c.name = format!("{} copia", c.name);
                m.pata.surfaces.insert((i + 1).min(m.pata.surfaces.len()), c);
            }
        } else {
            changed = false;
        }
    } else if let Some(i) = idx("del_") {
        if value.as_bool() == Some(true) && m.pata.surfaces.len() > 1 {
            m.pata.surfaces.remove(i);
        } else {
            changed = false;
        }
    } else {
        changed = false;
    }
    if changed {
        m.dirty.pata = true;
        sync_active_profile(m);
    }
}

/// Crea un perfil nuevo desde la config viva y lo activa.
fn do_create_profile(m: &mut Model) {
    let base = DesktopProfile {
        mirada: m.mirada.clone(),
        keymap: m.keymap_rows.clone(),
        pata: m.pata.clone(),
        theme: active_theme_name(m),
        keymap_set: m.profiles.active().to_string(),
        animation_set: m.animaciones.active().to_string(),
    };
    let name = m.dprofiles.create(base, "perfil nuevo");
    activate_profile(m, &name);
    m.selected_pest = PERFILES_DIENTE;
    m.sidebar_open = true;
    m.status = format!("perfil «{name}» creado y activado");
}

/// Duplica el perfil activo y activa la copia.
fn do_duplicate_profile(m: &mut Model) {
    let src = m.dprofiles.active.clone();
    if let Some(name) = m.dprofiles.duplicate(&src) {
        activate_profile(m, &name);
        m.selected_pest = PERFILES_DIENTE;
        m.sidebar_open = true;
        m.status = format!("perfil «{name}» (copia de «{src}»)");
    } else {
        m.status = "no hay perfil activo que duplicar".into();
    }
}

/// Elimina el perfil activo (si hay más de uno) y activa el siguiente.
fn do_delete_profile(m: &mut Model) {
    let cur = m.dprofiles.active.clone();
    if m.dprofiles.profiles.len() <= 1 {
        m.status = "no se puede eliminar el último perfil".into();
    } else {
        m.dprofiles.remove(&cur);
        let next = m.dprofiles.active.clone();
        if !next.is_empty() {
            activate_profile(m, &next);
        }
        m.selected_pest = PERFILES_DIENTE;
        m.sidebar_open = true;
        m.status = format!("perfil «{cur}» eliminado");
    }
}

/// Renombra el perfil activo.
fn do_rename_profile(m: &mut Model, to: &str) {
    let to = to.trim().to_string();
    let active = m.dprofiles.active.clone();
    if to.is_empty() || active.is_empty() {
        return;
    }
    if m.dprofiles.rename(&active, &to) {
        m.dirty.dprofiles = true;
        m.status = format!("perfil renombrado a «{to}»");
    } else {
        m.status = format!("no pude renombrar a «{to}» (¿ya existe?)");
    }
}

/// Re-siembra los perfiles de fábrica faltantes (rescata defaults renombrados/
/// borrados) — y sus themes homónimos.
fn do_rescue_profiles(m: &mut Model) {
    let n = m.dprofiles.ensure_defaults();
    // Asegura también los themes de fábrica que falten (mismos nombres de vista).
    let mut tn = 0;
    for name in mirada_brain::VISTA_NAMES {
        if m.themes.get(name).is_none() {
            if let Some(v) = mirada_brain::Vista::by_name(name) {
                m.themes
                    .set(name, themes::Theme::from_config(&v.config, &m.cfg.theme_variant, &m.cfg.accent));
                tn += 1;
            }
        }
    }
    m.dirty.dprofiles = true;
    if tn > 0 {
        m.dirty.themes = true;
    }
    m.status = if n == 0 && tn == 0 {
        "no faltaba ningún perfil/theme de fábrica".into()
    } else {
        format!("rescatados: {n} perfil(es), {tn} theme(s)")
    };
}

/// Activa un **perfil de escritorio** completo de la biblioteca: vuelca su foto
/// (config mirada + keymap + barra pata) a las rutas vivas — `config.ron`,
/// `keymap.ron`, `launcher.toml` — que el compositor y pata recargan en
/// caliente, y lo refleja en el panel como perfil en edición.
fn activate_profile(m: &mut Model, name: &str) {
    let Some(prof) = m.dprofiles.get(name).cloned() else {
        return;
    };
    // Reflejar en el panel + FUNDIR el theme del perfil (teselado+decoración a
    // mirada, apariencia a wawa): el perfil ya no es dueño de esos campos.
    m.mirada = prof.mirada.clone();
    m.keymap_rows = prof.keymap.clone();
    m.pata = prof.pata.clone();
    m.dprofiles.active = name.to_string();
    if let Some(t) = m.themes.get_or_default(&prof.theme).cloned() {
        t.apply_to(&mut m.mirada);
        m.cfg.theme_variant = t.theme_variant.clone();
        m.cfg.accent = t.accent.clone();
    }
    // Conjunto de atajos referenciado: si el perfil apunta a uno existente, lo
    // activamos y tomamos SUS teclas (en vez del keymap embebido). Mismo patrón
    // que el theme. Vacío o inexistente → se queda con el keymap embebido.
    if !prof.keymap_set.is_empty() && m.profiles.contains(&prof.keymap_set) {
        let _ = m.profiles.set_active(&prof.keymap_set);
        m.keymap_rows = m.profiles.active_keymap().to_rows();
    }
    // Conjunto de animación referenciado: lo activamos y volcamos sus parámetros
    // (transición/slide/Prezi) a la config mirada. Vacío/inexistente → sin cambio.
    if !prof.animation_set.is_empty() && m.animaciones.contains(&prof.animation_set) {
        m.animaciones.set_active(&prof.animation_set);
        m.animaciones.active_animation().apply_to(&mut m.mirada);
    }
    // Persistir la config viva (perfil + theme fusionados) y el resto.
    if let Some(p) = m.mirada_path.clone() {
        let _ = m.mirada.save(&p);
    }
    if let Some(kp) = m.keymap_path.clone() {
        // m.keymap_rows ya refleja el conjunto referenciado (si lo hay).
        let _ = mirada_brain::Keymap::from_rows(&m.keymap_rows).save(&kp);
    }
    let _ = pata_config::save(&prof.pata);
    let _ = m.cfg.save();
    let _ = m.dprofiles.save();
    m.status = format!("perfil «{name}» activado (en caliente)");
}

/// Sincroniza la foto del perfil **activo** con la config viva del panel (tras
/// editar mirada/pata/atajos) y marca la biblioteca para persistir. Así cada
/// perfil conserva sus propios ajustes en vez de compartir un único `config.ron`.
fn sync_active_profile(m: &mut Model) {
    let active = m.dprofiles.active.clone();
    if active.is_empty() {
        return;
    }
    // Preservamos las referencias del perfil (theme + conjunto de atajos): el
    // perfil no es dueño de su teselado/decoración (theme) ni de sus teclas
    // (conjunto de atajos); las referencia por nombre.
    let (theme, keymap_set, animation_set) = m
        .dprofiles
        .get(&active)
        .map(|p| (p.theme.clone(), p.keymap_set.clone(), p.animation_set.clone()))
        .unwrap_or_default();
    m.dprofiles.set(
        &active,
        DesktopProfile {
            mirada: m.mirada.clone(),
            keymap: m.keymap_rows.clone(),
            pata: m.pata.clone(),
            theme,
            keymap_set,
            animation_set,
        },
    );
    m.dirty.dprofiles = true;
}

/// La sección "Atajos" de mirada: el keymap como tabla (combinación · acción).
/// El id va prefijado (`mirada::atajos`) para que [`route_change`] lo reconozca
/// y lo aplique al buffer del keymap (no a la `Config`).
fn keymap_section(rows: &[Vec<String>]) -> Section {
    use allichay::{Column, Field};
    Section::new("atajos::teclas", "Teclas")
        .icon("⌨")
        .help("Las teclas del conjunto seleccionado. Editar acá afecta a todos los perfiles que usan este conjunto. +/− agrega/borra.")
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

/// Esquema del panel **Atajos** (su propio diente). Tab 1 = la biblioteca de
/// conjuntos de atajos (lista reusable: elegir cuál usa el perfil activo +
/// crear/duplicar/renombrar/eliminar). Tab 2 = las teclas del conjunto. Mismo
/// patrón que la pestaña Themes.
fn atajos_schema(m: &Model) -> Schema {
    use allichay::Field;
    let active_profile = m.dprofiles.active.clone();
    let set = m.profiles.active().to_string();
    let opts: Vec<EnumOption> =
        m.profiles.names().into_iter().map(|n| EnumOption::new(n.clone(), n)).collect();
    Schema::new()
        .section(
            Section::new("atajos::conjuntos", "Atajos")
                .icon("⌨")
                .help(
                    "Conjuntos de atajos reusables (dwm/i3/hyprland o propios), \
                     perpendiculares a los perfiles. El perfil activo USA un \
                     conjunto; editarlo afecta a todos los perfiles que lo \
                     referencian.",
                )
                .field(Field::radio(
                    "usar",
                    format!("Atajos de «{active_profile}»"),
                    set.clone(),
                    opts,
                ))
                .field(Field::button("crear", "Crear conjunto (desde el actual)"))
                .field(Field::button("duplicar", format!("Duplicar «{set}»")))
                .field(Field::text("renombrar", format!("Renombrar «{set}» a…"), ""))
                .field(Field::button("eliminar", format!("Eliminar «{set}»"))),
        )
        .section(keymap_section(&m.keymap_rows))
}

/// Nombre único para un conjunto de atajos nuevo a partir de `hint`.
fn unique_keymap_name(m: &Model, hint: &str) -> String {
    let base = if hint.trim().is_empty() { "atajos" } else { hint.trim() };
    if !m.profiles.contains(base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base} {n}")).find(|c| !m.profiles.contains(c)).unwrap()
}

/// Aplica una edición del panel Atajos (`rel` sin el prefijo `atajos::`).
fn apply_atajos(m: &mut Model, rel: &FieldPath, value: FieldValue) {
    let active_profile = m.dprofiles.active.clone();
    // Relaciona el conjunto activo con el perfil global (como hace theme).
    let relacionar = |m: &mut Model| {
        let set = m.profiles.active().to_string();
        if let Some(p) = m.dprofiles.profiles.get_mut(&active_profile) {
            p.keymap_set = set;
        }
        m.dirty.dprofiles = true;
    };
    match rel.segments().first().map(String::as_str) {
        Some("conjuntos") => match rel.leaf() {
            Some("usar") => {
                if let Some(name) = value.as_str() {
                    if m.profiles.set_active(name).is_ok() {
                        m.keymap_rows = m.profiles.active_keymap().to_rows();
                        m.dirty.keymap = true;
                        m.dirty.profiles = true;
                        relacionar(m);
                        m.status = format!("usando atajos «{name}»");
                    }
                }
            }
            Some("crear") if value.as_bool() == Some(true) => {
                let nombre = unique_keymap_name(m, "atajos nuevo");
                let km = mirada_brain::Keymap::from_rows(&m.keymap_rows);
                if m.profiles.create(&nombre, km).is_ok() {
                    let _ = m.profiles.set_active(&nombre);
                    m.dirty.profiles = true;
                    relacionar(m);
                    m.status = format!("conjunto «{nombre}» creado");
                }
            }
            Some("duplicar") if value.as_bool() == Some(true) => {
                let src = m.profiles.active().to_string();
                let nombre = unique_keymap_name(m, &format!("{src} copia"));
                if m.profiles.duplicate(&src, &nombre).is_ok() {
                    let _ = m.profiles.set_active(&nombre);
                    m.dirty.profiles = true;
                    relacionar(m);
                    m.status = format!("conjunto «{nombre}» (copia de «{src}»)");
                }
            }
            Some("renombrar") => {
                if let Some(to) = value.as_str() {
                    let (to, from) = (to.trim().to_string(), m.profiles.active().to_string());
                    if !to.is_empty() && m.profiles.rename(&from, &to).is_ok() {
                        let _ = m.profiles.set_active(&to);
                        // Re-apunta los perfiles que usaban el viejo nombre.
                        for p in m.dprofiles.profiles.values_mut() {
                            if p.keymap_set == from {
                                p.keymap_set = to.clone();
                            }
                        }
                        m.dirty.profiles = true;
                        m.dirty.dprofiles = true;
                        m.status = format!("conjunto renombrado a «{to}»");
                    }
                }
            }
            Some("eliminar") if value.as_bool() == Some(true) => {
                let cur = m.profiles.active().to_string();
                if m.profiles.len() > 1 && m.profiles.remove(&cur).is_ok() {
                    m.keymap_rows = m.profiles.active_keymap().to_rows();
                    m.dirty.keymap = true;
                    m.dirty.profiles = true;
                    relacionar(m);
                    m.status = format!("conjunto «{cur}» eliminado");
                } else {
                    m.status = "no podés eliminar el último conjunto".into();
                }
            }
            _ => {}
        },
        // Tab «Teclas»: la tabla edita el buffer + lo vuelca al conjunto activo.
        Some("teclas") => {
            if let Some(rows) = value.as_table() {
                m.keymap_rows = rows.to_vec();
                let km = mirada_brain::Keymap::from_rows(&m.keymap_rows);
                let active = m.profiles.active().to_string();
                let _ = m.profiles.set_keymap(&active, km);
                m.dirty.keymap = true;
                m.dirty.profiles = true;
            }
        }
        _ => {}
    }
}

/// Esquema del panel **Animaciones** (su propio diente). Tab 1 = biblioteca de
/// conjuntos de animación (relacionable con el perfil global + CRUD). Tab 2 =
/// los parámetros del conjunto (transición Win+Tab, slide, vuelo Prezi). Mismo
/// patrón que Themes/Atajos.
fn animaciones_schema(m: &Model) -> Schema {
    use allichay::Field;
    let active_profile = m.dprofiles.active.clone();
    let set = m.animaciones.active().to_string();
    let a = m.animaciones.active_animation();
    let opts: Vec<EnumOption> =
        m.animaciones.names().into_iter().map(|n| EnumOption::new(n.clone(), n)).collect();
    Schema::new()
        .section(
            Section::new("animaciones::conjuntos", "Animaciones")
                .icon("✨")
                .help(
                    "Conjuntos de animación reusables, perpendiculares a los \
                     perfiles. El perfil activo USA uno; editarlo afecta a todos \
                     los perfiles que lo referencian.",
                )
                .field(Field::radio(
                    "usar",
                    format!("Animación de «{active_profile}»"),
                    set.clone(),
                    opts,
                ))
                .field(Field::button("crear", "Crear conjunto (desde el actual)"))
                .field(Field::button("duplicar", format!("Duplicar «{set}»")))
                .field(Field::text("renombrar", format!("Renombrar «{set}» a…"), ""))
                .field(Field::button("eliminar", format!("Eliminar «{set}»"))),
        )
        .section(
            Section::new("animaciones::parametros", "Parámetros")
                .icon("✨")
                .help("Cómo anima el escritorio. Editar afecta a los perfiles que usan este conjunto.")
                .field(Field::dropdown(
                    "switch_mode",
                    "Transición de escritorio (Win+Tab)",
                    a.switch_mode.slug().to_string(),
                    vec![
                        EnumOption::new("direct", "Directo (sin animación)"),
                        EnumOption::new("hyprland", "Deslizar (estilo Hyprland)"),
                        EnumOption::new("prezi", "Zoom a vista espacial (Prezi)"),
                    ],
                ))
                .field(Field::slider_int("slide_ms", "Duración del slide (ms)", a.slide_ms as i64, 0, 600))
                .field(Field::slider_int(
                    "overview_anim_ms",
                    "Vuelo de cámara Prezi (ms)",
                    a.overview_anim_ms as i64,
                    0,
                    800,
                )),
        )
}

/// Aplica una edición del panel Animaciones (`rel` sin el prefijo `animaciones::`).
fn apply_animaciones(m: &mut Model, rel: &FieldPath, value: FieldValue) {
    let active_profile = m.dprofiles.active.clone();
    let relacionar = |m: &mut Model| {
        let set = m.animaciones.active().to_string();
        if let Some(p) = m.dprofiles.profiles.get_mut(&active_profile) {
            p.animation_set = set;
        }
        m.dirty.dprofiles = true;
    };
    // Vuelca el conjunto activo a la config viva (para que el compositor lo
    // recargue) y marca ambos sucios.
    let aplicar_vivo = |m: &mut Model| {
        m.animaciones.active_animation().apply_to(&mut m.mirada);
        m.dirty.animaciones = true;
        m.dirty.mirada = true;
    };
    match rel.segments().first().map(String::as_str) {
        Some("conjuntos") => match rel.leaf() {
            Some("usar") => {
                if let Some(name) = value.as_str() {
                    if m.animaciones.set_active(name) {
                        relacionar(m);
                        aplicar_vivo(m);
                        m.status = format!("usando animación «{name}»");
                    }
                }
            }
            Some("crear") if value.as_bool() == Some(true) => {
                let base = animaciones::Animation::from_config(&m.mirada);
                let nombre = m.animaciones.create(base, "animación nueva");
                m.animaciones.set_active(&nombre);
                relacionar(m);
                m.dirty.animaciones = true;
                m.status = format!("conjunto «{nombre}» creado");
            }
            Some("duplicar") if value.as_bool() == Some(true) => {
                let src = m.animaciones.active().to_string();
                if let Some(nombre) = m.animaciones.duplicate(&src) {
                    m.animaciones.set_active(&nombre);
                    relacionar(m);
                    m.dirty.animaciones = true;
                    m.status = format!("conjunto «{nombre}» (copia de «{src}»)");
                }
            }
            Some("renombrar") => {
                if let Some(to) = value.as_str() {
                    let (to, from) = (to.trim().to_string(), m.animaciones.active().to_string());
                    if !to.is_empty() && m.animaciones.rename(&from, &to) {
                        for p in m.dprofiles.profiles.values_mut() {
                            if p.animation_set == from {
                                p.animation_set = to.clone();
                            }
                        }
                        m.dirty.animaciones = true;
                        m.dirty.dprofiles = true;
                        m.status = format!("conjunto renombrado a «{to}»");
                    }
                }
            }
            Some("eliminar") if value.as_bool() == Some(true) => {
                let cur = m.animaciones.active().to_string();
                if m.animaciones.len() > 1 {
                    m.animaciones.remove(&cur);
                    relacionar(m);
                    aplicar_vivo(m);
                    m.status = format!("conjunto «{cur}» eliminado");
                } else {
                    m.status = "no podés eliminar el último conjunto".into();
                }
            }
            _ => {}
        },
        Some("parametros") => {
            let mut a = m.animaciones.active_animation();
            match rel.leaf() {
                Some("switch_mode") => {
                    if let Some(mode) = value.as_str().and_then(mirada_brain::WorkspaceSwitchMode::from_slug)
                    {
                        a.switch_mode = mode;
                    }
                }
                Some("slide_ms") => {
                    if let Some(v) = value.as_int() {
                        a.slide_ms = v.max(0) as u32;
                    }
                }
                Some("overview_anim_ms") => {
                    if let Some(v) = value.as_int() {
                        a.overview_anim_ms = v.max(0) as u32;
                    }
                }
                _ => {}
            }
            let active = m.animaciones.active().to_string();
            m.animaciones.set(&active, a.clone());
            a.apply_to(&mut m.mirada);
            m.dirty.animaciones = true;
            m.dirty.mirada = true;
        }
        _ => {}
    }
}

/// La pestaña "Sistema": varios items de configuración del SO.
#[allow(dead_code)] // reemplazado por el reparto en 5 paneles de `pestanas`.
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

/// Lee el volumen (0-100) y el estado de silencio del sink de audio por
/// defecto vía `wpctl` (PipeWire/WirePlumber). Sin wpctl o sin sink devuelve
/// `(50, false)` para que la UI muestre algo coherente.
fn query_sound() -> (i64, bool) {
    let out = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output();
    if let Ok(o) = out {
        if o.status.success() {
            let s = String::from_utf8_lossy(&o.stdout);
            let muted = s.contains("MUTED");
            let vol = s
                .split_whitespace()
                .nth(1)
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.5);
            return (((vol * 100.0).round() as i64).clamp(0, 150), muted);
        }
    }
    (50, false)
}

/// Sonido: salida de audio por defecto. Controles REALES sobre `wpctl`
/// (PipeWire/WirePlumber) — no necesitan driver propio, hablan al servidor de
/// audio ya corriendo. Volumen 0-150 % y silencio.
fn sonido_section() -> Section {
    let (vol, muted) = query_sound();
    Section::new("sonido::salida", "Sonido")
        .icon("🔊")
        .help("Salida de audio por defecto · PipeWire/WirePlumber (wpctl)")
        .field(
            Field::slider_int("volumen", "Volumen", vol, 0, 150)
                .help("Porcentaje del sink @DEFAULT_AUDIO_SINK@"),
        )
        .field(Field::toggle("mudo", "Silenciar", muted))
}

/// Distribuciones de teclado XKB ofrecidas (id XKB → etiqueta). Lista corta de
/// las más comunes; el id se escribe tal cual al config de mirada.
const XKB_LAYOUTS: &[(&str, &str)] = &[
    ("", "Sistema (por defecto)"),
    ("us", "Inglés (EE. UU.)"),
    ("es", "Español (España)"),
    ("latam", "Español (Latinoamérica)"),
    ("fr", "Francés"),
    ("de", "Alemán"),
    ("it", "Italiano"),
    ("pt", "Portugués"),
    ("br", "Portugués (Brasil)"),
    ("ru", "Ruso"),
    ("gb", "Inglés (Reino Unido)"),
];

/// Teclado: distribución XKB del compositor. REAL: la aplica mirada al crear el
/// teclado (cambia al reiniciar la sesión). Ruteado a la config de mirada.
fn teclado_section(mir: &mirada_brain::Config) -> Section {
    Section::new("mirada::teclado", "Teclado")
        .icon("⌨")
        .help("Distribución del teclado (XKB). Se aplica al reiniciar la sesión.")
        .field(Field::dropdown(
            "xkb_layout",
            "Distribución",
            mir.xkb_layout.clone(),
            XKB_LAYOUTS
                .iter()
                .map(|(id, l)| EnumOption::new(*id, *l))
                .collect(),
        ))
        .field(
            Field::text("xkb_variant", "Variante (opcional)", mir.xkb_variant.clone())
                .help("p. ej. dvorak, nodeadkeys — vacío = ninguna"),
        )
}

/// Puntero/touchpad: preferencias de libinput. REAL: mirada las aplica a cada
/// dispositivo (al conectarlo / al reiniciar sesión). Ruteado a mirada.
fn puntero_section(mir: &mirada_brain::Config) -> Section {
    Section::new("mirada::puntero", "Ratón y touchpad")
        .icon("🖱")
        .help("Preferencias de libinput. Se aplican al (re)conectar el dispositivo.")
        .field(Field::toggle(
            "natural_scroll",
            "Scroll natural",
            mir.natural_scroll,
        ))
        .field(Field::toggle(
            "tap_to_click",
            "Tocar para clickear (touchpad)",
            mir.tap_to_click,
        ))
        .field(
            Field::slider(
                "pointer_speed",
                "Velocidad del puntero",
                mir.pointer_speed,
                -1.0,
                1.0,
                0.1,
            )
            .help("−1 lento · 0 neutro · 1 rápido"),
        )
        .field(Field::toggle(
            "focus_follows_mouse",
            "El foco sigue al puntero",
            mir.focus_follows_mouse,
        ))
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

/// El init real del sistema (PID 1), leído de `/proc/1/comm`. En Artix será
/// `openrc-init`/`runit`/`s6-svscan`/`init`/`dinit`, NO systemd.
fn detectar_init() -> String {
    std::fs::read_to_string("/proc/1/comm")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "desconocido".into())
}

/// Arranque: usar arje como init del sistema. Control real próximamente.
fn arranque_section() -> Section {
    Section::new("wawa::arranque", "Arranque")
        .icon("▶")
        .help("Init del sistema en Linux")
        .field(Field::display("init", "Init actual", format!("{} (PID 1)", detectar_init())))
        .field(Field::display(
            "proximamente",
            "Próximamente",
            "elegir arje como init (PID 1)",
        ))
}

/// Ruta del archivo de autoarranque que lee mirada al iniciar la sesión
/// (`~/.config/mirada/autostart`, un comando por línea).
fn autostart_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    std::path::Path::new(&home).join(".config/mirada/autostart")
}

/// Lee los comandos de autoarranque (descarta líneas vacías y comentarios `#`).
fn read_autostart() -> Vec<String> {
    std::fs::read_to_string(autostart_path())
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// Reescribe el archivo de autoarranque (uno por línea). Crea el directorio si
/// falta. Deja el error en el status si algo sale mal.
fn write_autostart(m: &mut Model, items: &[String]) {
    let path = autostart_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    match std::fs::write(&path, format!("{body}\n")) {
        Ok(()) => m.status = format!("autoarranque guardado ({} programas)", items.iter().filter(|s| !s.trim().is_empty()).count()),
        Err(e) => m.status = format!("· autoarranque: {e}"),
    }
}

/// Autoarranque: programas que mirada lanza al iniciar la sesión. Lista
/// editable (+/− agrega/borra) sobre `~/.config/mirada/autostart`. Real: mirada
/// lee ese archivo al arrancar.
fn autostart_section() -> Section {
    Section::new("autostart::lista", "Autoarranque")
        .icon("⟳")
        .help(
            "Programas que el escritorio lanza al iniciar sesión (uno por línea). \
             mirada lee ~/.config/mirada/autostart al arrancar. +/− agrega/borra.",
        )
        .field(Field::list(
            "lista",
            "Programas al inicio",
            read_autostart(),
            "comando",
        ))
}

/// Fondo de escritorio: muestra la imagen actual del perfil activo y abre el
/// diálogo de archivos para elegir otra (toggle de acción).
#[allow(dead_code)] // Wallpapers usa la sección `fondo` de mirada (perfil activo).
fn fondo_section(wallpaper: &str) -> Section {
    let actual = if wallpaper.trim().is_empty() {
        "(ninguna)".to_string()
    } else {
        wallpaper.to_string()
    };
    Section::new("wawa::fondo", "Fondo")
        .icon("▦")
        .help("Imagen de fondo del escritorio (del perfil activo).")
        .field(Field::display("imagen", "Imagen actual", actual))
        .field(Field::toggle("elegir", "Elegir imagen de fondo…", false))
}

/// **Wallpaper** unificado: imagen fija (elegir) + automático (proveedor +
/// intervalo + activar). Antes eran dos secciones separadas («Fondo» y «Fondo
/// automático») que se veían duplicadas; el usuario las pidió juntas.
fn wallpaper_section(m: &Model) -> Section {
    use allichay::Field;
    let wp = m.mirada.wallpaper_path.trim();
    let actual = if wp.is_empty() { "(color sólido)".to_string() } else { wp.to_string() };
    let prov = if m.cfg.wallpaper_provider.is_empty() {
        "bing".to_string()
    } else {
        m.cfg.wallpaper_provider.clone()
    };
    Section::new("wawa::wallpaper", "Wallpaper")
        .icon("▦")
        .help(
            "El fondo del escritorio, todo junto: una imagen fija (Elegir…) o \
             automático por proveedor (foto del día, etc.). La imagen es del \
             perfil activo.",
        )
        .field(Field::display("imagen", "Imagen actual", actual))
        .field(Field::button("elegir", "Elegir imagen de fondo…"))
        .field(Field::dropdown(
            "wallpaper_provider",
            "Automático · proveedor",
            prov,
            vec![
                EnumOption::new("bing", "Bing — foto del día"),
                EnumOption::new("nasa", "NASA — imagen astronómica del día"),
                EnumOption::new("folder", "Carpeta local (rota)"),
                EnumOption::new("solar", "Solar — según la hora del día"),
            ],
        ))
        .field(Field::slider(
            "wallpaper_hours",
            "Automático · refrescar cada (horas)",
            m.cfg.wallpaper_interval_hours.max(1) as f64,
            1.0,
            48.0,
            1.0,
        ))
        .field(Field::button("aplicar_fondo", "Aplicar automático ahora"))
        .field(Field::toggle(
            "activar_rotacion",
            "Activar rotación automática (cada sesión)",
            wallpaper_autostart_enabled(),
        ))
}

/// Fondo automático: proveedor (Bing/NASA/Carpeta) + intervalo + activar/aplicar.
/// Escribe `~/.config/mirada/wallpaper.ron` y lanza el daemon `mirada-wallpaper`.
#[allow(dead_code)] // fundido en `wallpaper_section`.
fn fondo_auto_section(cfg: &WawaConfig) -> Section {
    let prov = if cfg.wallpaper_provider.is_empty() {
        "bing".to_string()
    } else {
        cfg.wallpaper_provider.clone()
    };
    Section::new("wawa::fondoauto", "Fondo automático")
        .icon("▦")
        .help("Descarga y rota el fondo desde un proveedor (foto del día, etc.).")
        .field(Field::dropdown(
            "wallpaper_provider",
            "Proveedor",
            prov,
            vec![
                EnumOption::new("bing", "Bing — foto del día"),
                EnumOption::new("nasa", "NASA — imagen astronómica del día"),
                EnumOption::new("folder", "Carpeta local (rota)"),
                EnumOption::new("solar", "Solar — según la hora del día"),
            ],
        ))
        .field(Field::slider(
            "wallpaper_hours",
            "Refrescar cada (horas)",
            cfg.wallpaper_interval_hours.max(1) as f64,
            1.0,
            48.0,
            1.0,
        ))
        .field(Field::toggle("aplicar_fondo", "Aplicar fondo ahora", false))
        // Refleja el estado real del autostart (toggle persistente).
        .field(Field::toggle(
            "activar_rotacion",
            "Activar rotación automática (arranca sola cada sesión)",
            wallpaper_autostart_enabled(),
        ))
}

/// Escribe `~/.config/mirada/wallpaper.ron` para el daemon, derivado del
/// proveedor + intervalo elegidos en el panel (y la carpeta de wallpapers para
/// Folder). RON construido a mano para no arrastrar las deps del daemon.
fn write_wallpaper_ron(provider: &str, hours: u32, folder_dir: &str) {
    let Some(path) = mirada_brain::Config::default_path()
        .and_then(|p| p.parent().map(|d| d.join("wallpaper.ron")))
    else {
        return;
    };
    let secs = (hours.max(1) as u64) * 3600;
    let dir = if folder_dir.trim().is_empty() {
        std::env::var("HOME").map(|h| format!("{h}/Pictures")).unwrap_or_default()
    } else {
        folder_dir.to_string()
    };
    let source = match provider {
        "nasa" => "Nasa(api_key: \"DEMO_KEY\")".to_string(),
        "folder" => format!("Folder(dir: {dir:?})"),
        "solar" => {
            // Sin imágenes por fase no hace mucho; dejamos placeholders que el
            // usuario completa. Igual escribe la estructura válida.
            format!(
                "Solar(lat: 0.0, lon: 0.0, night: {0:?}, dawn: {0:?}, day: {0:?}, dusk: {0:?})",
                ""
            )
        }
        _ => "Bing(market: \"en-US\", resolution: \"1920x1080\")".to_string(),
    };
    let ron = format!(
        "(source: {source}, interval_secs: {secs}, output: \"\", keep: 8)\n"
    );
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, ron);
}

/// La línea de autostart del daemon de wallpaper (la lee el compositor de
/// `~/.config/mirada/autostart`, una por línea).
const WP_AUTOSTART_LINE: &str = "mirada-wallpaper daemon";

fn wallpaper_autostart_path() -> Option<PathBuf> {
    mirada_brain::Config::default_path().and_then(|p| p.parent().map(|d| d.join("autostart")))
}

/// `true` si el daemon de wallpaper está en el autostart de mirada.
fn wallpaper_autostart_enabled() -> bool {
    wallpaper_autostart_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| t.lines().any(|l| l.trim() == WP_AUTOSTART_LINE))
        .unwrap_or(false)
}

/// Agrega/quita la línea del daemon en el autostart de mirada (persistente: el
/// compositor lo lanza en cada sesión).
fn set_wallpaper_autostart(on: bool) {
    let Some(path) = wallpaper_autostart_path() else { return };
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = cur
        .lines()
        .map(|l| l.to_string())
        .filter(|l| l.trim() != WP_AUTOSTART_LINE)
        .collect();
    if on {
        lines.push(WP_AUTOSTART_LINE.to_string());
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    let _ = std::fs::write(&path, out);
}

/// Escanea `dir` (y un nivel de subcarpetas) por imágenes y las agrega a `out`.
fn scan_images(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let es_imagen = |p: &std::path::Path| {
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp")
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_file() {
            if es_imagen(&p) {
                out.push(p);
            }
        } else if p.is_dir() {
            if let Ok(rd2) = std::fs::read_dir(&p) {
                for e2 in rd2.flatten() {
                    let p2 = e2.path();
                    if p2.is_file() && es_imagen(&p2) {
                        out.push(p2);
                    }
                }
            }
        }
    }
}

/// Abre el diálogo de archivos poblado con imágenes de las carpetas habituales
/// de wallpapers + la carpeta configurada.
fn open_wallpaper_picker(m: &mut Model) {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(h) = &home {
        dirs.push(h.join("Pictures"));
        dirs.push(h.join("Imágenes"));
        dirs.push(h.join("Wallpapers"));
        dirs.push(h.join("Fondos"));
        dirs.push(h.join(".config/mirada/wallpapers"));
    }
    dirs.push(PathBuf::from("/usr/share/backgrounds"));
    if !m.mirada.wallpaper_dir.trim().is_empty() {
        dirs.push(PathBuf::from(&m.mirada.wallpaper_dir));
    }
    let mut paths: Vec<PathBuf> = Vec::new();
    for d in &dirs {
        scan_images(d, &mut paths);
    }
    paths.sort();
    paths.dedup();
    let root = home.unwrap_or_else(|| PathBuf::from("/"));
    m.picker = Some(PickerState::new(&paths, &root));
    m.picker_paths = paths;
    m.picker_root = root;
    m.status = format!("elegí un fondo ({} imágenes)", m.picker_paths.len());
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
/// Lee los monitores conectados desde `/sys/class/drm` (conector → modo
/// preferido). Es lo que ve el kernel DRM; no necesita hablar con el
/// compositor. Devuelve `(nombre, modo)` por conector conectado.
fn read_monitors() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir("/sys/class/drm") else {
        return out;
    };
    let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        // Conectores: cardN-<NOMBRE>. Saltamos el nodo de la tarjeta a secas.
        let Some((_, conector)) = name.split_once('-') else {
            continue;
        };
        let status = std::fs::read_to_string(path.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let modo = std::fs::read_to_string(path.join("modes"))
            .ok()
            .and_then(|m| m.lines().next().map(str::to_string))
            .unwrap_or_else(|| "—".into());
        out.push((conector.to_string(), modo));
    }
    out
}

/// Monitores conectados (DRM) — un campo de sólo-lectura por conector.
fn monitores_section() -> Section {
    let mut s = Section::new("wawa::monitores", "Monitores")
        .icon("🖵")
        .help("Pantallas conectadas (DRM · /sys/class/drm)");
    let mons = read_monitors();
    if mons.is_empty() {
        s = s.field(Field::display("ninguno", "Estado", "sin monitores detectados"));
    } else {
        for (i, (name, modo)) in mons.iter().enumerate() {
            s = s.field(Field::display(format!("mon{i}"), name.clone(), modo.clone()));
        }
    }
    s
}

fn info_schema(host: &HostInfo) -> Schema {
    let t = rimay_localize::t;
    let used_kb = host.mem_total_kb.saturating_sub(host.mem_avail_kb);
    let swap_used = host.swap_total_kb.saturating_sub(host.swap_free_kb);
    Schema::new()
        .section(
            Section::new("wawa::infohost", t("wawa-panel-cat-monitor"))
                .icon("🖥")
                .field(Field::display("host", t("wawa-panel-stat-host"), &host.host))
                .field(Field::display("distro", "Distribución", &host.distro))
                .field(Field::display("kernel", t("wawa-panel-stat-kernel"), &host.kernel))
                .field(Field::display("arch", "Arquitectura", std::env::consts::ARCH))
                .field(Field::display("init", "Init", detectar_init()))
                .field(Field::display("uptime", t("wawa-panel-stat-uptime"), fmt_uptime(host.uptime)))
                .field(Field::display(
                    "cpu",
                    "CPU",
                    format!("{} · {} núcleos", host.cpu_model, host.cpu_cores),
                ))
                .field(Field::display(
                    "mem",
                    t("wawa-panel-stat-mem"),
                    fmt_mem(used_kb, host.mem_total_kb),
                ))
                .field(Field::display(
                    "swap",
                    "Swap",
                    if host.swap_total_kb == 0 {
                        "—".to_string()
                    } else {
                        fmt_mem(swap_used, host.swap_total_kb)
                    },
                ))
                .field(Field::display(
                    "load",
                    t("wawa-panel-stat-load"),
                    format!("{:.2} · {:.2} · {:.2}", host.load.0, host.load.1, host.load.2),
                ))
                .field(Field::toggle("monitor", "Abrir monitor de procesos…", false)),
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
            // Wallpaper: el botón «elegir» abre el diálogo de archivos.
            if rel.leaf() == Some("elegir") && value.as_bool() == Some(true) {
                open_wallpaper_picker(m);
                return;
            }
            // Información: el toggle «monitor» abre el monitor de procesos.
            if rel.leaf() == Some("monitor") && value.as_bool() == Some(true) {
                let _ = std::process::Command::new("sandokan-monitor").spawn();
                m.status = "abriendo monitor de procesos…".into();
                return;
            }
            // Fondo automático · «Aplicar ahora» (one-shot).
            if rel.leaf() == Some("aplicar_fondo") && value.as_bool() == Some(true) {
                let prov = if m.cfg.wallpaper_provider.is_empty() { "bing" } else { &m.cfg.wallpaper_provider };
                write_wallpaper_ron(prov, m.cfg.wallpaper_interval_hours, &m.mirada.wallpaper_dir);
                let _ = std::process::Command::new("mirada-wallpaper").spawn();
                m.status = format!("aplicando fondo de {prov}…");
                return;
            }
            // Fondo automático · «Activar rotación»: toggle PERSISTENTE — lo mete
            // en el autostart de mirada (arranca solo cada sesión) y lo
            // lanza/mata ahora.
            if rel.leaf() == Some("activar_rotacion") {
                let on = value.as_bool().unwrap_or(false);
                set_wallpaper_autostart(on);
                if on {
                    let prov = if m.cfg.wallpaper_provider.is_empty() { "bing" } else { &m.cfg.wallpaper_provider };
                    write_wallpaper_ron(prov, m.cfg.wallpaper_interval_hours, &m.mirada.wallpaper_dir);
                    let _ = std::process::Command::new("mirada-wallpaper").arg("daemon").spawn();
                    m.status = "rotación activada — arranca sola cada sesión".into();
                } else {
                    let _ = std::process::Command::new("pkill").args(["-f", "mirada-wallpaper"]).spawn();
                    m.status = "rotación automática desactivada".into();
                }
                return;
            }
            apply_wawa(m, rel.leaf().unwrap_or(""), value);
            m.dirty.wawa = true;
        }
        // Pestaña Perfiles: el primer segmento es el NOMBRE del perfil; el
        // toggle «activo» lo aplica entero (look + atajos + barra) en caliente.
        "perfiles" => {
            match rel.leaf() {
                // Selector «usar»: activa el perfil elegido (en caliente).
                Some("usar") => {
                    if let Some(sel) = value.as_str() {
                        activate_profile(m, sel);
                        m.dirty.dprofiles = true;
                    }
                }
                Some("crear") if value.as_bool() == Some(true) => do_create_profile(m),
                Some("duplicar") if value.as_bool() == Some(true) => do_duplicate_profile(m),
                Some("eliminar") if value.as_bool() == Some(true) => do_delete_profile(m),
                Some("renombrar") => {
                    if let Some(to) = value.as_str() {
                        do_rename_profile(m, to);
                    }
                }
                Some("rescatar") if value.as_bool() == Some(true) => do_rescue_profiles(m),
                _ => {}
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
        // Lista de barras: la tabla reconstruye m.pata.surfaces (agregar/borrar/
        // renombrar/prender-apagar). sync_active_profile lo guarda en el perfil.
        "barras" => {
            apply_barras_list(m, rel.leaf().unwrap_or(""), value);
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        "reglas" => {
            if let Some(rows) = value.as_table() {
                apply_reglas_table(m, &rows.to_vec());
            }
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        "theme" => {
            apply_theme(m, &rel, value);
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        "atajos" => {
            apply_atajos(m, &rel, value);
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        "animaciones" => {
            apply_animaciones(m, &rel, value);
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        // Autoarranque: la lista reescribe ~/.config/mirada/autostart al toque.
        "autostart" => {
            if let Some(items) = value.as_list() {
                let items = items.to_vec();
                write_autostart(m, &items);
            }
            return;
        }
        // Sonido: aplica EN CALIENTE sobre wpctl (sin persistir nada nuestro —
        // el estado vive en WirePlumber). Volumen 0-150 % / silencio.
        "sonido" => {
            match rel.leaf() {
                Some("volumen") => {
                    if let Some(v) = value.as_int() {
                        let frac = (v as f64 / 100.0).clamp(0.0, 1.5);
                        let _ = std::process::Command::new("wpctl")
                            .args([
                                "set-volume",
                                "@DEFAULT_AUDIO_SINK@",
                                &format!("{frac:.2}"),
                            ])
                            .spawn();
                        m.status = format!("volumen → {v} %");
                    }
                }
                Some("mudo") => {
                    let on = value.as_bool().unwrap_or(false);
                    let _ = std::process::Command::new("wpctl")
                        .args([
                            "set-mute",
                            "@DEFAULT_AUDIO_SINK@",
                            if on { "1" } else { "0" },
                        ])
                        .spawn();
                    m.status = if on { "audio silenciado".into() } else { "audio activo".into() };
                }
                _ => {}
            }
            return;
        }
        _ => return,
    }
    // Editar mirada/pata/atajos modifica el perfil ACTIVO: vuelca la config viva
    // dentro de su entrada en la biblioteca (cada perfil conserva lo suyo).
    if matches!(key.as_str(), "mirada" | "pata") {
        sync_active_profile(m);
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
    if m.dirty.dprofiles {
        match m.dprofiles.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· perfiles save: {e}")),
        }
        m.dirty.dprofiles = false;
    }
    if m.dirty.rules {
        match m.rules_path.as_deref().map(|p| m.rules.save(p)) {
            Some(Ok(())) => ok = true,
            Some(Err(e)) => err = Some(format!("· reglas save: {e}")),
            None => err = Some("· reglas: sin ruta".into()),
        }
        m.dirty.rules = false;
    }
    if m.dirty.themes {
        match m.themes.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· themes save: {e}")),
        }
        m.dirty.themes = false;
    }
    if m.dirty.animaciones {
        match m.animaciones.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· animaciones save: {e}")),
        }
        m.dirty.animaciones = false;
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
        "wallpaper_provider" => {
            if let Some(s) = value.as_str() {
                m.cfg.wallpaper_provider = s.to_string();
            }
        }
        "wallpaper_hours" => {
            if let Some(v) = value.as_float() {
                m.cfg.wallpaper_interval_hours = (v as u32).max(1);
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
    // Panel Atajos. Tab «conjuntos»: «usar» = conjunto activo; renombrar = texto;
    // botones = false. Tab «teclas»: la tabla = el buffer de filas del keymap.
    if key == "atajos" {
        return Some(match rel.segments().first().map(String::as_str) {
            Some("conjuntos") => match rel.leaf() {
                Some("usar") => FieldValue::Text(m.profiles.active().to_string()),
                Some("renombrar") => FieldValue::Text(String::new()),
                _ => FieldValue::Bool(false),
            },
            _ => FieldValue::Table(m.keymap_rows.clone()),
        });
    }
    // Panel Animaciones, tab «conjuntos»: «usar» = conjunto activo; renombrar =
    // texto; botones = false. (Los parámetros del tab 2 salen del schema.)
    if key == "animaciones" && rel.segments().first().map(String::as_str) == Some("conjuntos") {
        return Some(match rel.leaf() {
            Some("usar") => FieldValue::Text(m.animaciones.active().to_string()),
            Some("renombrar") => FieldValue::Text(String::new()),
            _ => FieldValue::Bool(false),
        });
    }
    // Pestaña Perfiles (lista única): el selector «usar» = perfil activo; los
    // botones (crear/duplicar/eliminar/rescatar) leen false; renombrar = texto.
    if key == "perfiles" {
        return Some(match rel.leaf() {
            Some("usar") => FieldValue::Text(m.dprofiles.active.clone()),
            Some("renombrar") => FieldValue::Text(String::new()),
            _ => FieldValue::Bool(false),
        });
    }
    // Pestaña Themes: «usar» = theme del perfil activo; renombrar = texto;
    // botones = false. (apariencia/teselado/decoración salen del schema abajo.)
    if key == "theme" && rel.segments().first().map(String::as_str) == Some("acciones") {
        return Some(match rel.leaf() {
            Some("usar") => FieldValue::Text(active_theme_name(m)),
            Some("renombrar") => FieldValue::Text(String::new()),
            _ => FieldValue::Bool(false),
        });
    }
    // Lista de barras: on_<i> = enabled; name_<i> = nombre; botones = false.
    if key == "barras" {
        let leaf = rel.leaf().unwrap_or("");
        let idx = |p: &str| leaf.strip_prefix(p).and_then(|s| s.parse::<usize>().ok());
        if let Some(i) = idx("on_") {
            return Some(FieldValue::Bool(m.pata.surfaces.get(i).map(|s| s.enabled).unwrap_or(false)));
        }
        if let Some(i) = idx("name_") {
            return Some(FieldValue::Text(m.pata.surfaces.get(i).map(|s| s.name.clone()).unwrap_or_default()));
        }
        return Some(FieldValue::Bool(false));
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

/// Editor **visual 2D del Prezi**: una grilla donde cada escritorio es un tile
/// arrastrable. Al soltar, el tile snapea a la celda más cercana y se guarda en
/// `overview_geometry` del perfil activo (la vista espacial lo respeta). Es la
/// versión visual del que antes era una tabla col/fila.
fn prezi_editor_view(model: &Model, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    const CELL: f32 = 78.0;
    let n = mirada_brain::action::WORKSPACE_COUNT;
    let geo = model.mirada.overview_geometry_for(n);
    let max_c = geo.iter().map(|g| g.0).max().unwrap_or(0);
    let max_r = geo.iter().map(|g| g.1).max().unwrap_or(0);
    let cols = (max_c + 2).max(4) as usize;
    let rows = (max_r + 2).max(3) as usize;
    let cw = cols as f32 * CELL;
    let ch = rows as f32 * CELL;
    let linea = {
        let k = theme.border.components;
        Color::from_rgba8(
            (k[0] * 255.0) as u8,
            (k[1] * 255.0) as u8,
            (k[2] * 255.0) as u8,
            90,
        )
    };

    let mut kids: Vec<View<Msg>> = Vec::with_capacity(n);
    for (i, &(c, r)) in geo.iter().enumerate() {
        let x = c as f32 * CELL + 5.0;
        let y = r as f32 * CELL + 5.0;
        let tile = View::new(Style {
            position: Position::Absolute,
            inset: Rect { left: length(x), top: length(y), right: auto(), bottom: auto() },
            size: Size { width: length(CELL - 10.0), height: length(CELL - 10.0) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(8.0)
        .text_aligned(format!("{}", i + 1), 20.0, theme.bg_panel, Alignment::Center)
        .draggable(move |phase, dx, dy| match phase {
            DragPhase::End => {
                let nc = (c + (dx / CELL).round() as i32).max(0);
                let nr = (r + (dy / CELL).round() as i32).max(0);
                Some(Msg::PreziMove(i, nc, nr))
            }
            _ => None,
        });
        kids.push(tile);
    }

    let canvas = View::new(Style {
        position: Position::Relative,
        size: Size { width: length(cw), height: length(ch) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(8.0)
    .paint_with(move |scene, _ts, rect| {
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KRect};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        let (x0, y0) = (rect.x as f64, rect.y as f64);
        for col in 1..cols {
            let gx = x0 + col as f64 * CELL as f64;
            scene.fill(Fill::NonZero, Affine::IDENTITY, &linea, None,
                &KRect::new(gx, y0, gx + 1.0, y0 + rect.h as f64));
        }
        for row in 1..rows {
            let gy = y0 + row as f64 * CELL as f64;
            scene.fill(Fill::NonZero, Affine::IDENTITY, &linea, None,
                &KRect::new(x0, gy, x0 + rect.w as f64, gy + 1.0));
        }
    })
    .children(kids);

    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "Plano 2D del Prezi · arrastrá cada escritorio a su celda".to_string(),
        13.0,
        theme.fg_text,
        Alignment::Start,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(vec![titulo, canvas])
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
            let panel = schema_panel(&one, &model.allichay, theme, VIEWPORT_H, Msg::Allichay);
            // La sección «Vista espacial» suma arriba el editor visual 2D del
            // Prezi (canvas con tiles arrastrables); los campos van debajo.
            if sec.id.contains("vista_espacial") {
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![prezi_editor_view(model, theme), panel])
            } else {
                panel
            }
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
/// Color vívido y estable para un icono de sección (por glifo), para que el rail
/// de items no sea blanco y negro. Glifos conocidos llevan su color temático; el
/// resto cae a una paleta indexada por el glifo.
fn icon_color(icon: &str) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let rgb = match icon {
        "🎨" => (236, 107, 118), // apariencia
        "🌐" => (97, 150, 236),  // idioma
        "🎛" => (84, 196, 194),  // interfaz / pata
        "▶" => (138, 201, 108),  // arranque
        "☸" => (244, 162, 97),   // módulos
        "🖥" => (96, 200, 220),   // info
        "✚" => (138, 201, 108),  // acciones
        "▦" | "▭" => (167, 139, 250), // fondo / barras
        "⌨" => (233, 196, 106),  // atajos
        _ => {
            const PAL: &[(u8, u8, u8)] = &[
                (236, 107, 118),
                (244, 162, 97),
                (233, 196, 106),
                (138, 201, 108),
                (84, 196, 194),
                (97, 150, 236),
                (167, 139, 250),
                (240, 138, 201),
            ];
            let sum: u32 = icon.chars().map(|c| c as u32).sum();
            PAL[(sum as usize) % PAL.len()]
        }
    };
    Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255)
}

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
        // Icono en COLOR (no B&N): color vívido estable por glifo.
        .text_aligned(
            if icon.is_empty() { "·" } else { icon }.to_string(),
            14.0,
            icon_color(icon),
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
            Menu::new("Perfiles")
                .item(MenuItem::new("Crear perfil (desde el actual)", "perfil.create"))
                .item(MenuItem::new("Duplicar perfil activo", "perfil.duplicate"))
                .item(MenuItem::new("Eliminar perfil activo", "perfil.delete")),
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
    // Crear/duplicar/eliminar perfiles de escritorio (mismos helpers que los
    // botones de la pestaña Perfiles).
    match cmd {
        "perfil.create" => {
            do_create_profile(&mut m);
            return m;
        }
        "perfil.duplicate" => {
            do_duplicate_profile(&mut m);
            return m;
        }
        "perfil.delete" => {
            do_delete_profile(&mut m);
            return m;
        }
        _ => {}
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
