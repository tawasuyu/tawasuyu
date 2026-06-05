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
use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, Rect,
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
/// Ancho del rail de dientes (pestañitas con icono).
const NAV_WIDTH: f32 = 52.0;
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
    ("default", "gioser"),
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

/// Apps suscribibles que traen su propio diente. El `key` casa con un id de
/// módulo en `WawaConfig.modules` (módulo apagado = diente oculto); pata no es
/// módulo del SO, así que `module_enabled` lo deja siempre visible.
const CONFIGURABLE_APPS: &[(&str, &str, &str)] =
    &[("mirada", "mirada", "⚙"), ("pata", "pata", "🎛")];

/// Índice del diente "Información" (4ª categoría) — para el menú Ayuda.
const INFO_DIENTE: usize = 3;

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
    /// Diente activo: índice en [`dientes`].
    selected: usize,
    cfg: WawaConfig,
    mirada: mirada_brain::Config,
    mirada_path: Option<PathBuf>,
    pata: pata_core::Config,
    allichay: AllichayState,
    host: HostInfo,
    status: String,
    _config_watcher: Option<ConfigWatcher>,
    menu_open: Option<usize>,
    menu_active: usize,
    menu_anim: Tween<f32>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Click en un diente del rail (índice).
    NavSelect(u64),
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

        Model {
            selected: 0,
            cfg,
            mirada,
            mirada_path,
            pata,
            allichay: AllichayState::new(),
            host,
            status: String::new(),
            _config_watcher: watcher,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => refresh_host(&mut m.host),
            Msg::NavSelect(id) => {
                let n = dientes(&m).len().max(1);
                m.selected = (id as usize).min(n - 1);
                m.allichay.select(m.selected);
                m.status.clear();
            }
            Msg::Allichay(AllichayMsg::SelectSection(_)) => {}
            Msg::Allichay(AllichayMsg::Focus(path)) => {
                let seed = current_text_value(&m, &path);
                m.allichay.focus(&path, &seed);
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
        let dientes = dientes(model);
        let sel = model.selected.min(dientes.len().saturating_sub(1));

        let menu = app_menu();
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = build_header(&theme);
        let nav = build_nav(&dientes, sel, &theme);
        let content = build_content(&dientes, sel, model, &theme);
        let status = build_status(model, &theme);

        let body = View::new(Style {
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
        .children(vec![nav, content]);

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
// Registro de dientes (categorías + apps suscritas)
// =====================================================================

/// Un diente del panel: su nombre, su icono y el esquema que lista en su panel.
struct PanelDiente {
    title: String,
    icon: String,
    schema: Schema,
}

/// Arma el rail completo: dientes-categoría del SO + dientes-de-app suscritas.
fn dientes(m: &Model) -> Vec<PanelDiente> {
    let t = rimay_localize::t;
    let mut out = vec![
        PanelDiente {
            title: t("wawa-panel-cat-appearance"),
            icon: "🎨".into(),
            schema: wawa_appearance_schema(&m.cfg),
        },
        PanelDiente {
            title: t("wawa-panel-cat-language"),
            icon: "🌐".into(),
            schema: wawa_language_schema(&m.cfg),
        },
        PanelDiente {
            title: t("wawa-panel-cat-modules"),
            icon: "☸".into(),
            schema: wawa_modules_schema(&m.cfg),
        },
        PanelDiente {
            title: "Información".into(),
            icon: "🖥".into(),
            schema: wawa_info_schema(&m.host),
        },
    ];
    // Dientes-de-app: cada app suscrita (módulo activo) trae su propio esquema.
    for (key, label, icon) in CONFIGURABLE_APPS {
        if !m.cfg.module_enabled(key) {
            continue;
        }
        let schema = match *key {
            "mirada" => prefix_schema(m.mirada.schema(), "mirada"),
            "pata" => prefix_schema(m.pata.schema(), "pata"),
            _ => continue,
        };
        out.push(PanelDiente {
            title: (*label).to_string(),
            icon: (*icon).to_string(),
            schema,
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

fn wawa_appearance_schema(cfg: &WawaConfig) -> Schema {
    let t = rimay_localize::t;
    Schema::new().section(
        Section::new("wawa::apariencia", t("wawa-panel-cat-appearance"))
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
            )),
    )
}

fn wawa_language_schema(cfg: &WawaConfig) -> Schema {
    let t = rimay_localize::t;
    Schema::new().section(
        Section::new("wawa::idioma", t("language"))
            .field(Field::dropdown(
                "lang",
                t("wawa-panel-label-language"),
                cfg.lang.clone(),
                LANGS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
            ))
            .field(
                Field::toggle("timefmt_24h", t("wawa-panel-label-clock"), cfg.timefmt_24h)
                    .help(t("wawa-panel-clock-24h")),
            ),
    )
}

fn wawa_modules_schema(cfg: &WawaConfig) -> Schema {
    let t = rimay_localize::t;
    let mut section = Section::new("wawa::modulos", t("wawa-panel-cat-modules"));
    for (id, _glyph, key) in MODULES {
        section = section.field(Field::toggle(*id, t(key), cfg.module_enabled(id)));
    }
    Schema::new().section(section)
}

fn wawa_info_schema(host: &HostInfo) -> Schema {
    let t = rimay_localize::t;
    let used_kb = host.mem_total_kb.saturating_sub(host.mem_avail_kb);
    Schema::new()
        .section(
            Section::new("wawa::infohost", t("wawa-panel-cat-monitor"))
                .field(Field::display("host", t("wawa-panel-stat-host"), &host.host))
                .field(Field::display("kernel", t("wawa-panel-stat-kernel"), &host.kernel))
                .field(Field::display(
                    "uptime",
                    t("wawa-panel-stat-uptime"),
                    fmt_uptime(host.uptime),
                ))
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
                .field(Field::display(
                    "version",
                    t("wawa-panel-about-version"),
                    env!("CARGO_PKG_VERSION"),
                ))
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
fn route_change(m: &mut Model, path: &FieldPath, value: FieldValue) {
    let Some((key, rel)) = split_app(path) else {
        m.status = format!("· ruta inválida: {path}");
        return;
    };
    match key.as_str() {
        "wawa" => apply_wawa(m, rel.leaf().unwrap_or(""), value),
        "mirada" => {
            if let Err(e) = m.mirada.apply(&rel, value) {
                m.status = format!("· mirada: {e}");
                return;
            }
            match m.mirada_path.as_deref().map(|p| m.mirada.save(p)) {
                Some(Ok(())) => m.status = rimay_localize::t("wawa-panel-autosave-ok"),
                Some(Err(e)) => m.status = format!("· mirada save: {e}"),
                None => m.status = "· mirada: sin ruta de config".into(),
            }
        }
        "pata" => {
            if let Err(e) = m.pata.apply(&rel, value) {
                m.status = format!("· pata: {e}");
                return;
            }
            match pata_config::save(&m.pata) {
                Ok(_) => m.status = rimay_localize::t("wawa-panel-autosave-ok"),
                Err(e) => m.status = format!("· pata save: {e}"),
            }
        }
        _ => {}
    }
}

/// Aplica un cambio a la config del SO (`WawaConfig`) por id de campo y persiste.
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
        // Cualquier otro id es un toggle de módulo.
        other => {
            if let Some(b) = value.as_bool() {
                if MODULES.iter().any(|(id, _, _)| *id == other) && m.cfg.module_enabled(other) != b {
                    m.cfg.toggle_module(other);
                }
            }
        }
    }
    match m.cfg.save() {
        Ok(_) => m.status = rimay_localize::t("wawa-panel-autosave-ok"),
        Err(e) => m.status = format!("· save: {e}"),
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

fn build_nav(dientes: &[PanelDiente], sel: usize, theme: &Theme) -> View<Msg> {
    let items: Vec<DockRailItem> = dientes
        .iter()
        .enumerate()
        .map(|(i, _)| DockRailItem {
            id: i as u64,
            active: i == sel,
        })
        .collect();
    let icons: Vec<String> = dientes.iter().map(|d| d.icon.clone()).collect();
    dock_rail_view(
        &items,
        NAV_WIDTH,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| tooth_icon(icons.get(id as usize).cloned(), size, color),
        Msg::NavSelect,
        |_| None,
    )
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

fn build_content(dientes: &[PanelDiente], sel: usize, model: &Model, theme: &Theme) -> View<Msg> {
    match dientes.get(sel) {
        Some(d) => schema_panel(&d.schema, &model.allichay, theme, VIEWPORT_H, Msg::Allichay),
        None => View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        }),
    }
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
            m.selected = INFO_DIENTE;
            m.allichay.select(INFO_DIENTE);
            m.status.clear();
        }
        _ => {}
    }
    m
}
