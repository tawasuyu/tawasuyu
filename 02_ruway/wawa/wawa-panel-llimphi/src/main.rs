//! `wawa-panel-llimphi` — panel de control del sistema operativo wawa.
//!
//! Una app Llimphi nativa que centraliza la configuración del SO en
//! seis categorías navegables desde una columna lateral:
//!
//! * **Apariencia** — variante del theme (dark/light/aurora/sunset).
//! * **Idioma** — locale del sistema (es-PE/qu-PE/en-US) + reloj 24h/12h.
//! * **Aplicaciones** — lanzadores de las apps Llimphi del SO.
//! * **Monitor** — hora, uptime, memoria, carga, host, kernel (vive).
//! * **Módulos** — toggles de las piezas del SO (mirada, shuma, …).
//! * **Acerca de** — info del sistema y suite.
//!
//! Persiste en `$XDG_CONFIG_HOME/wawa-panel/state.json` con
//! `directories::ProjectDirs`. Al arrancar carga la config previa
//! (silencioso si no existe) y aplica el locale via `rimay_localize::
//! set_locale`. El theme y los módulos no se "aplican al SO" todavía
//! — esa parte vendrá cuando exista un bus de configuración global;
//! por ahora se guarda como preferencia de usuario.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Position, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_button::{button_styled, ButtonPalette};
use llimphi_widget_context_menu::{
    context_menu_view, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use app_bus::{AppMenu, Menu, MenuItem};
use std::path::PathBuf;
use std::sync::Arc;
use wawa_config::{ConfigWatcher, WawaConfig};

use allichay::{Configurable, FieldPath, FieldValue};
use llimphi_module_allichay::{section_view, AllichayMsg, AllichayState};
use llimphi_widget_dock_rail::{dock_rail_view, DockRailItem, DockRailPalette};

/// Apps suscribibles que exponen su config como schema. El `key` casa con el
/// id de módulo en `WawaConfig.modules` (un módulo apagado oculta su diente);
/// pata no es un módulo del SO, así que `module_enabled` lo deja siempre visible.
const CONFIGURABLE_APPS: &[(&str, &str)] = &[("mirada", "mirada"), ("pata", "pata")];

/// Base del id de diente de las secciones de app (para no chocar con las
/// categorías builtin 0..N).
const APP_DIENTE_BASE: u64 = 1000;

// =====================================================================
// Constantes y catálogos
// =====================================================================

/// Refresco del monitor.
const TICK_MS: u64 = 1_000;
/// Ancho del sidebar de navegación.
const NAV_WIDTH: f32 = 60.0;
/// Alto de cada fila de control.
const ROW_HEIGHT: f32 = 36.0;

/// Categorías visibles del panel. Orden fijo — refleja el orden mental
/// del usuario: lo visual primero (apariencia, idioma), después las
/// apps y el monitor, y al final lo más infraestructural (módulos,
/// acerca).
#[derive(Copy, Clone, PartialEq, Eq)]
enum Category {
    Appearance,
    Language,
    Apps,
    Monitor,
    Modules,
    About,
}

impl Category {
    fn all() -> &'static [Category] {
        &[
            Category::Appearance,
            Category::Language,
            Category::Apps,
            Category::Monitor,
            Category::Modules,
            Category::About,
        ]
    }
    fn glyph(self) -> &'static str {
        match self {
            Category::Appearance => "◐",
            Category::Language => "✦",
            Category::Apps => "▣",
            Category::Monitor => "◉",
            Category::Modules => "≡",
            Category::About => "?",
        }
    }
    fn i18n_key(self) -> &'static str {
        match self {
            Category::Appearance => "wawa-panel-cat-appearance",
            Category::Language => "wawa-panel-cat-language",
            Category::Apps => "wawa-panel-cat-apps",
            Category::Monitor => "wawa-panel-cat-monitor",
            Category::Modules => "wawa-panel-cat-modules",
            Category::About => "wawa-panel-cat-about",
        }
    }
    fn hint_key(self) -> &'static str {
        match self {
            Category::Appearance => "wawa-panel-section-appearance-hint",
            Category::Language => "wawa-panel-section-language-hint",
            Category::Apps => "wawa-panel-section-apps-hint",
            Category::Monitor => "wawa-panel-section-monitor-hint",
            Category::Modules => "wawa-panel-section-modules-hint",
            Category::About => "wawa-panel-section-about-hint",
        }
    }
}

/// Variantes del theme conocidas. El nombre coincide con
/// `llimphi_theme::Theme::name` para `Theme::by_name`.
const THEME_VARIANTS: &[(&str, &str)] = &[
    ("dark", "wawa-panel-variant-dark"),
    ("light", "wawa-panel-variant-light"),
    ("aurora", "wawa-panel-variant-aurora"),
    ("sunset", "wawa-panel-variant-sunset"),
];

/// Locales que el panel ofrece. El id (izquierda) es el que come
/// `rimay_localize::set_locale`.
const LANGS: &[(&str, &str)] = &[
    ("es-PE", "Español"),
    ("en-US", "English"),
    ("qu-PE", "Runasimi"),
];

/// Acentos disponibles para la UI. El id (izq) es lo que persiste en
/// `WawaConfig::accent`; el label (der) es lo que ve el usuario. El
/// color real lo resuelve `wawa_config::accent_rgb(id)`.
const ACCENTS: &[(&str, &str)] = &[
    ("default", "gioser"),
    ("unanchay", "unanchay"),
    ("yachay", "yachay"),
    ("ruway", "ruway"),
    ("ukupacha", "ukupacha"),
];

/// Apps Llimphi del SO. Cada entrada es `(binario, id-i18n-o-nombre, descripcion)`.
/// El binario es lo que se `Command::spawn`-ea cuando el usuario aprieta
/// "Lanzar"; si no existe en `$PATH` o `target/debug/`, el panel lo
/// reporta en el status sin caer.
const APPS: &[(&str, &str, &str)] = &[
    ("nada",            "nada",       "Editor de texto · sesiones, LSP, theme switcher"),
    ("dominium-app-llimphi",   "dominium",           "Simulador del campo medio · lemmings y conceptos"),
    ("nakui-explorer-llimphi", "nakui-explorer",     "Explorador estelar del catálogo cosmos"),
    ("nahual-image-viewer-llimphi", "nahual-viewer", "Visor de imágenes con texto y shell"),
    ("nahual-file-explorer-llimphi", "nahual-files", "Explorador de archivos"),
    ("chasqui-explorer-llimphi", "chasqui",          "Correo y mensajería · mónadas"),
    ("pluma-editor-llimphi",     "pluma-editor",     "Editor de markdown y notebooks"),
    ("wawa-explorer-llimphi",    "wawa-explorer",    "Explorador de paquetes y release channels"),
    ("agora-app",                "agora",            "Mercado y plaza pública"),
    ("minga-explorer-llimphi",   "minga-explorer",   "Red p2p, dht y vfs distribuida"),
];

/// Módulos del SO con su id estable (lo que se guarda en config),
/// glyph y key i18n del label.
const MODULES: &[(&str, &str, &str)] = &[
    ("mirada",  "◉", "wawa-panel-mod-mirada"),
    ("shuma",   "✦", "wawa-panel-mod-shuma"),
    ("chasqui", "✉", "wawa-panel-mod-chasqui"),
    ("akasha",  "↻", "wawa-panel-mod-akasha"),
    ("minga",   "◈", "wawa-panel-mod-minga"),
    ("agora",   "◯", "wawa-panel-mod-agora"),
];

// =====================================================================
// Configuración persistida
// =====================================================================

// El modelo persistido vive en `wawa-config::WawaConfig` — un struct
// compartido con el resto del SO. Acá sólo lo consumimos: cargamos al
// arrancar, escribimos cuando el usuario apreta "guardar", y nos
// suscribimos a cambios externos vía `ConfigWatcher` (otro panel
// abierto, edición manual del JSON, futuras herramientas CLI).

// =====================================================================
// Información del host (Linux /proc)
// =====================================================================

/// Snapshot del estado del host. Se refresca en cada `Tick`.
#[derive(Clone, Default)]
struct HostInfo {
    /// `gethostname` o /etc/hostname.
    host: String,
    /// `uname -r` (kernel release).
    kernel: String,
    /// Tiempo encendido en segundos.
    uptime: u64,
    /// MemTotal kB.
    mem_total_kb: u64,
    /// MemAvailable kB.
    mem_avail_kb: u64,
    /// loadavg 1/5/15.
    load: (f32, f32, f32),
    /// Hora actual: hora, minuto, segundo (locales).
    hms: (u32, u32, u32),
}

fn read_proc_file(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn parse_meminfo(s: &str) -> (u64, u64) {
    let mut total = 0;
    let mut avail = 0;
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
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|v| v as u64)
        .unwrap_or(0)
}

fn read_hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

fn read_kernel() -> String {
    // /proc/sys/kernel/osrelease equivale a `uname -r`.
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "—".into())
}

fn local_hms() -> (u32, u32, u32) {
    // Segundos desde epoch + offset local. Para evitar dep nueva,
    // calculo el offset leyendo `date +%z` no — peor. Mejor uso
    // `SystemTime::now()` UTC y aplico offset desde TZ env-var si está.
    // Para MVP: muestro UTC + 0 si no hay forma de saber. La hora local
    // exacta no es crítica para el panel.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Heurística: usar timezone offset del sistema vía $TZ_OFFSET_SEC
    // si está; sino, tomar la fecha en UTC. Aceptable como MVP — el
    // monitor advierte "UTC" en el sub-label si no hay offset.
    let offset = std::env::var("TZ_OFFSET_SEC")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let local = (secs as i64).saturating_add(offset).rem_euclid(86400) as u32;
    let h = local / 3600;
    let m = (local % 3600) / 60;
    let s = local % 60;
    (h, m, s)
}

fn refresh_host(info: &mut HostInfo) {
    info.host = read_hostname();
    info.kernel = read_kernel();
    info.uptime = parse_uptime(&read_proc_file("/proc/uptime"));
    let (total, avail) = parse_meminfo(&read_proc_file("/proc/meminfo"));
    info.mem_total_kb = total;
    info.mem_avail_kb = avail;
    info.load = parse_loadavg(&read_proc_file("/proc/loadavg"));
    info.hms = local_hms();
}

fn fmt_uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hrs = (secs % 86_400) / 3_600;
    let mins = (secs % 3_600) / 60;
    if days > 0 {
        format!("{}d {:02}h {:02}m", days, hrs, mins)
    } else {
        format!("{:02}h {:02}m", hrs, mins)
    }
}

fn fmt_mem(used_kb: u64, total_kb: u64) -> String {
    let used_mb = used_kb as f64 / 1024.0;
    let total_mb = total_kb as f64 / 1024.0;
    if total_mb > 1024.0 {
        format!("{:.1} / {:.1} GiB", used_mb / 1024.0, total_mb / 1024.0)
    } else {
        format!("{:.0} / {:.0} MiB", used_mb, total_mb)
    }
}

fn fmt_clock(hms: (u32, u32, u32), is_24h: bool) -> String {
    let (h, m, s) = hms;
    if is_24h {
        format!("{:02}:{:02}:{:02}", h, m, s)
    } else {
        let ampm = if h >= 12 { "pm" } else { "am" };
        let h12 = match h % 12 {
            0 => 12,
            x => x,
        };
        format!("{:02}:{:02}:{:02} {}", h12, m, s, ampm)
    }
}

// =====================================================================
// Modelo + mensajes
// =====================================================================

struct Model {
    category: Category,
    /// Diente de app activo: índice en [`app_sections`]. `None` = hay una
    /// categoría builtin activa (la del campo `category`).
    app_sel: Option<usize>,
    cfg: WawaConfig,
    /// Config viva del compositor (mirada) y su ruta en disco.
    mirada: mirada_brain::Config,
    mirada_path: Option<PathBuf>,
    /// Config viva del marco (pata).
    pata: pata_core::Config,
    /// Estado del renderizador de config (buffers de texto + foco).
    allichay: AllichayState,
    host: HostInfo,
    status: String,
    /// Subscripción al bus: mantiene vivo el watcher que reentra al
    /// `update` cuando otro proceso (panel duplicado, edición manual,
    /// futuras CLIs) modifica el archivo. `Option` porque la creación
    /// puede fallar en plataformas sin ProjectDirs.
    _config_watcher: Option<ConfigWatcher>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado).
    menu_open: Option<usize>,
    /// Fila resaltada por teclado dentro del dropdown del menú principal
    /// (`usize::MAX` = ninguna). La mueven las flechas ↑/↓.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    menu_anim: Tween<f32>,
    /// Menú contextual sobre la categoría activa: ancla `(x, y)` en
    /// ventana. `None` cerrado.
    context_menu: Option<(f32, f32)>,
}

#[derive(Clone)]
enum Msg {
    Tick,
    /// Click en un diente del rail: id < APP_DIENTE_BASE = categoría builtin;
    /// id >= APP_DIENTE_BASE = sección de app (índice = id - base).
    NavSelect(u64),
    /// Mensaje del renderizador de config (foco/cambio de un campo de app).
    Allichay(AllichayMsg),
    /// Tecla a enrutar al campo de texto en edición del renderizador.
    AllichayKey(KeyEvent),
    SelectCategory(Category),
    SetThemeVariant(String),
    SetAccent(String),
    SetLang(String),
    SetTimeFmt(bool),
    ToggleModule(String),
    LaunchApp(String),
    Save,
    Reset,
    /// Cambió la config desde afuera (otro panel, herramienta, edición
    /// manual). El `WawaConfig` ya viene parseado por el watcher.
    ConfigChanged(Box<WawaConfig>),
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Navegación por teclado en el dropdown del menú principal (↑/↓).
    MenuNav(i32),
    /// Ejecuta el comando de la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de la animación de aparición/swap del menú principal (re-render).
    MenuTick,
    /// Cierra cualquier menú abierto (click-fuera / Esc).
    CloseMenus,
    /// Right-click en la raíz → abre el menú contextual anclado en
    /// `(x, y)` de ventana sobre la categoría activa.
    ContextMenuOpen(f32, f32),
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
        // Refresco vivo del monitor cada segundo.
        handle.spawn_periodic(Duration::from_millis(TICK_MS), || Msg::Tick);

        let cfg = WawaConfig::load();
        // Aplicar locale al arrancar para que el resto de t() use el
        // idioma que el usuario eligió la última vez.
        let _ = rimay_localize::set_locale(&cfg.lang);

        // Subscripción al bus de configuración: si otro proceso edita
        // el archivo, recibimos un Msg con la versión nueva. El propio
        // panel también escribe el archivo en `Msg::Save`; el watcher
        // ignora esos cambios porque la comparación es contra el
        // estado actual del modelo (no disparamos `ConfigChanged` si
        // ya coincide).
        let handle_clone = handle.clone();
        let watcher = ConfigWatcher::spawn(move |new_cfg| {
            handle_clone.dispatch(Msg::ConfigChanged(Box::new(new_cfg)));
        })
        .map_err(|e| eprintln!("wawa-panel · watcher: {e}"))
        .ok();

        let mut host = HostInfo::default();
        refresh_host(&mut host);

        // Configs vivas de las apps suscritas: se cargan de su archivo (mirada
        // RON, pata TOML) y se editan en memoria; cada cambio se persiste y la
        // app lo recarga (mirada vía su FileWatch).
        let mirada_path = mirada_brain::Config::default_path();
        let mirada = mirada_path
            .as_deref()
            .map(mirada_brain::Config::load_or_default)
            .unwrap_or_default();
        let pata = pata_config::load();

        Model {
            category: Category::Appearance,
            app_sel: None,
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
            context_menu: None,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                refresh_host(&mut m.host);
            }
            Msg::SelectCategory(c) => {
                m.category = c;
                m.app_sel = None;
                m.allichay.blur();
                m.status.clear();
            }
            Msg::NavSelect(id) => {
                if id >= APP_DIENTE_BASE {
                    let idx = (id - APP_DIENTE_BASE) as usize;
                    m.app_sel = Some(idx);
                    m.allichay.select(idx);
                    m.status.clear();
                } else if let Some(cat) = Category::all().get(id as usize) {
                    m.category = *cat;
                    m.app_sel = None;
                    m.allichay.blur();
                    m.status.clear();
                }
            }
            Msg::Allichay(AllichayMsg::SelectSection(_)) => {
                // El rail lo maneja el panel (NavSelect); el renderizador de
                // sección no emite selección. Sin efecto.
            }
            Msg::Allichay(AllichayMsg::Focus(path)) => {
                let seed = current_text_value(&m, &path);
                m.allichay.focus(&path, &seed);
            }
            Msg::Allichay(AllichayMsg::Change(path, value)) => {
                route_change(&mut m, &path, value);
            }
            Msg::AllichayKey(event) => {
                if let Some((path, value)) = m.allichay.apply_key(&event) {
                    route_change(&mut m, &path, value);
                }
            }
            Msg::SetThemeVariant(v) => {
                m.cfg.theme_variant = v;
                autosave(&mut m);
            }
            Msg::SetAccent(a) => {
                m.cfg.accent = a;
                autosave(&mut m);
            }
            Msg::SetLang(l) => {
                let _ = rimay_localize::set_locale(&l);
                m.cfg.lang = l;
                autosave(&mut m);
            }
            Msg::SetTimeFmt(is_24h) => {
                m.cfg.timefmt_24h = is_24h;
                autosave(&mut m);
            }
            Msg::ToggleModule(id) => {
                m.cfg.toggle_module(&id);
                autosave(&mut m);
            }
            Msg::LaunchApp(bin) => {
                match std::process::Command::new(&bin).spawn() {
                    Ok(_) => m.status = format!("→ {}", bin),
                    Err(e) => m.status = format!("· {}: {}", bin, e),
                }
            }
            Msg::Save => match m.cfg.save() {
                Ok(path) => {
                    m.status = rimay_localize::t_args(
                        "wawa-panel-saved",
                        &[("path", path.display().to_string().into())],
                    );
                }
                Err(e) => {
                    m.status = format!("· save: {}", e);
                }
            },
            Msg::Reset => {
                m.cfg = WawaConfig::default();
                let _ = rimay_localize::set_locale(&m.cfg.lang);
                autosave(&mut m);
                // El status del autosave queda como "↻ aplicado"; lo
                // reemplazamos por el mensaje específico de reset para
                // que el usuario sepa qué pasó.
                m.status = rimay_localize::t("wawa-panel-reset");
            }
            Msg::ConfigChanged(new_cfg) => {
                // Cambio desde afuera. Si difiere del nuestro, lo
                // adoptamos sin perder la categoría visible ni el
                // status actual (avisar pero no resetear UX).
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
                // Abrir un menú raíz cierra cualquier contextual.
                m.context_menu = None;
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
                        return handle_menu_command(m, &cmd);
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::CloseMenus => {
                m.menu_open = None;
                m.menu_active = usize::MAX;
                m.context_menu = None;
            }
            Msg::MenuCommand(cmd) => {
                m.menu_open = None;
                return handle_menu_command(m, &cmd);
            }
            Msg::ContextMenuOpen(x, y) => {
                m.menu_open = None;
                m.context_menu = Some((x, y));
            }
        }
        m
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // Si hay un campo de texto de app en edición, todas las teclas van al
        // renderizador (clickear otro diente o categoría lo desenfoca).
        if model.allichay.is_editing() {
            return Some(Msg::AllichayKey(event.clone()));
        }
        // Menú principal abierto: las flechas navegan. ←/→ cambian de menú
        // raíz (con wrap), ↑/↓ mueven la fila activa, Enter ejecuta, Esc cierra.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
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
        // Esc cierra cualquier menú abierto antes que nada.
        if let Key::Named(NamedKey::Escape) = event.key {
            if model.menu_open.is_some() || model.context_menu.is_some() {
                return Some(Msg::CloseMenus);
            }
        }
        if event.modifiers.ctrl {
            if let Key::Character(s) = &event.key {
                if s == "s" || s == "S" {
                    return Some(Msg::Save);
                }
                if s == "r" || s == "R" {
                    return Some(Msg::Reset);
                }
            }
        }
        // Navegación rápida 1..6
        if let Key::Character(s) = &event.key {
            if let Some(idx) = s.chars().next().and_then(|c| c.to_digit(10)) {
                if (1..=Category::all().len() as u32).contains(&idx) {
                    return Some(Msg::SelectCategory(Category::all()[(idx - 1) as usize]));
                }
            }
        }
        if let Key::Named(NamedKey::Escape) = event.key {
            // Esc: limpiar status (no cerrar — eso lo hace el WM).
            return None;
        }
        None
    }

    fn view(model: &Model) -> View<Msg> {
        // `theme_from_cfg` ya incorpora el acento override (si lo hay).
        // El parámetro separado `accent` se conserva para los chips y
        // marcadores donde queremos el acento "puro" aunque el theme
        // ya lo tenga aplicado (p. ej. para no perder visibilidad si
        // un futuro variant lo pisa con otra cosa).
        let theme = theme_from_cfg(&model.cfg);
        let accent = theme.accent;

        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, &theme));
        let header = build_header(&theme);
        let nav = build_nav(model, &theme, accent);
        let content = build_content(model, &theme, accent);
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
        // Right-click en la raíz (origen 0,0 ⇒ local == ventana) abre el
        // menú contextual sobre la categoría activa.
        .on_right_click_at(|x, y, _w, _h| Some(Msg::ContextMenuOpen(x, y)))
        .children(vec![menubar, header, body, status])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        let theme = theme_from_cfg(&model.cfg);
        // El menú contextual de la categoría tiene prioridad si está abierto.
        if let Some((x, y)) = model.context_menu {
            let header = rimay_localize::t(model.category.i18n_key());
            // Acciones reales del panel: aplicar (guardar), restablecer,
            // y refrescar el monitor del host. El panel no edita texto:
            // no inventamos cortar/pegar.
            let items = vec![
                ContextMenuItem::action(rimay_localize::t("wawa-panel-action-save")),
                ContextMenuItem::action(rimay_localize::t("wawa-panel-action-reset")),
                ContextMenuItem::action(rimay_localize::t("wawa-panel-ctx-refresh-monitor")),
            ];
            let on_pick: Arc<dyn Fn(usize) -> Msg + Send + Sync> = Arc::new(|i: usize| match i {
                0 => Msg::Save,
                1 => Msg::Reset,
                _ => Msg::Tick,
            });
            return Some(context_menu_view(ContextMenuSpec {
                anchor: (x, y),
                viewport: viewport_of(),
                header: Some(header),
                items,
                active: usize::MAX,
                on_pick,
                on_dismiss: Msg::CloseMenus,
                palette: ContextMenuPalette::from_theme(&theme),
            }));
        }
        // Si no, el dropdown del menú principal.
        let menu = app_menu(model);
        menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        )
    }
}

/// Viewport para clampear overlays: el panel no trackea el tamaño de
/// ventana, así que usamos `initial_size()`.
fn viewport_of() -> (f32, f32) {
    let (w, h) = Panel::initial_size();
    (w as f32, h as f32)
}

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
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

/// El menú principal del panel. Archivo / Ver / Idioma / Ayuda — sólo comandos
/// que mapean a acciones reales (guardar, restablecer, navegar
/// categorías, cambiar idioma). Sin "Editar": el panel no tiene campos de texto
/// editables (sus controles son chips, toggles y botones).
fn app_menu(model: &Model) -> AppMenu {
    // Submenú "Ver": saltar a cada categoría. La activa va con check
    // textual al frente para que el estado sea legible.
    let mut ver = Menu::new(rimay_localize::t("wawa-panel-menu-view"));
    for (idx, cat) in Category::all().iter().enumerate() {
        let active = model.category == *cat;
        let mark = if active { "● " } else { "  " };
        let label = format!("{}{}  {}", mark, cat.glyph(), rimay_localize::t(cat.i18n_key()));
        let cmd = format!("view.cat.{idx}");
        let mut item = MenuItem::new(label, cmd).shortcut(format!("{}", idx + 1));
        if active {
            // La categoría activa no necesita re-navegar: la deshabilitamos.
            item = item.disabled();
        }
        ver = ver.item(item);
    }

    // Menú de idioma: autónimos sin traducir (convención del SO). El item
    // activo lleva ✔. El comando `lang.<code>` lo resuelve
    // `handle_menu_command` → set_locale + persiste en wawa-config.
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
                .item(
                    MenuItem::new(rimay_localize::t("wawa-panel-action-save"), "file.save")
                        .shortcut("Ctrl+S"),
                )
                .item(
                    MenuItem::new(rimay_localize::t("wawa-panel-action-reset"), "file.reset")
                        .shortcut("Ctrl+R"),
                )
                .item(MenuItem::new(rimay_localize::t("wawa-panel-menu-quit"), "file.quit").separated()),
        )
        .menu(ver)
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

/// Traduce un command id del menú principal al efecto real sobre el
/// modelo. Mapea sólo a acciones que el panel ya implementa.
fn handle_menu_command(model: Model, cmd: &str) -> Model {
    let mut m = model;
    // Cambio de idioma desde el menú "Idioma": aplica el locale en caliente
    // y lo persiste en wawa-config. El watcher reentra con `ConfigChanged`,
    // propagando el cambio a otras apps abiertas.
    if let Some(code) = cmd.strip_prefix("lang.") {
        let _ = rimay_localize::set_locale(code);
        m.cfg.lang = code.to_string();
        let _ = m.cfg.save();
        return m;
    }
    match cmd {
        "file.save" => match m.cfg.save() {
            Ok(path) => {
                m.status = rimay_localize::t_args(
                    "wawa-panel-saved",
                    &[("path", path.display().to_string().into())],
                );
            }
            Err(e) => m.status = format!("· save: {e}"),
        },
        "file.reset" => {
            m.cfg = WawaConfig::default();
            let _ = rimay_localize::set_locale(&m.cfg.lang);
            autosave(&mut m);
            m.status = rimay_localize::t("wawa-panel-reset");
        }
        "file.quit" => std::process::exit(0),
        "help.about" => {
            m.category = Category::About;
            m.status.clear();
        }
        other => {
            // Navegación de categorías: "view.cat.<idx>".
            if let Some(idx) = other.strip_prefix("view.cat.").and_then(|s| s.parse::<usize>().ok()) {
                if let Some(cat) = Category::all().get(idx) {
                    m.category = *cat;
                    m.status.clear();
                }
            }
        }
    }
    m
}

fn main() {
    rimay_localize::init();
    llimphi_ui::run::<Panel>();
}

/// Persiste la config y actualiza el status. Llamada después de cada
/// mutación del Model::cfg para reflejar el cambio en disco (y por
/// ende en el bus) sin requerir Save explícito.
fn autosave(m: &mut Model) {
    match m.cfg.save() {
        Ok(_) => m.status = rimay_localize::t("wawa-panel-autosave-ok"),
        Err(e) => m.status = format!("· save: {e}"),
    }
}

// =====================================================================
// Apps suscritas: schemas montados como dientes
// =====================================================================

/// Una sección de config de una app, lista para pintar como diente.
struct AppSec {
    /// Id completo de la sección (`"mirada::teselado"`) — prefijo del FieldPath.
    full_id: String,
    /// Glifo del diente.
    icon: String,
    /// La sección con el título ya prefijado por la app.
    section: allichay::Section,
}

/// Las secciones de config de las apps suscritas (módulo activo), en orden
/// estable. El índice en esta lista es el id de diente menos [`APP_DIENTE_BASE`].
fn app_sections(m: &Model) -> Vec<AppSec> {
    let mut out = Vec::new();
    for (key, label) in CONFIGURABLE_APPS {
        if !m.cfg.module_enabled(key) {
            continue;
        }
        let schema = match *key {
            "mirada" => m.mirada.schema(),
            "pata" => m.pata.schema(),
            _ => continue,
        };
        for mut sec in schema.sections {
            let full_id = format!("{key}::{}", sec.id);
            let icon = if sec.icon.is_empty() {
                "•".to_string()
            } else {
                sec.icon.clone()
            };
            sec.title = format!("{label} · {}", sec.title);
            out.push(AppSec {
                full_id,
                icon,
                section: sec,
            });
        }
    }
    out
}

/// Parte un FieldPath combinado (`["mirada::teselado", "gap"]`) en la clave de
/// app y la ruta relativa a esa app (`("mirada", ["teselado", "gap"])`).
fn split_app(path: &FieldPath) -> Option<(String, FieldPath)> {
    let segs = path.segments();
    let (key, sect) = segs.first()?.split_once("::")?;
    let mut rel = vec![sect.to_string()];
    rel.extend(segs[1..].iter().cloned());
    Some((key.to_string(), FieldPath(rel)))
}

/// Aplica un cambio a la config de la app destino y la persiste en su formato
/// nativo (mirada RON, pata TOML). El status refleja el resultado.
fn route_change(m: &mut Model, path: &FieldPath, value: FieldValue) {
    let Some((key, rel)) = split_app(path) else {
        m.status = format!("· ruta inválida: {path}");
        return;
    };
    match key.as_str() {
        "mirada" => {
            if let Err(e) = m.mirada.apply(&rel, value) {
                m.status = format!("· mirada: {e}");
                return;
            }
            match m.mirada_path.as_deref().map(|p| m.mirada.save(p)) {
                Some(Ok(())) => m.status = rimay_localize::t("wawa-panel-autosave-ok"),
                Some(Err(e)) => m.status = format!("· mirada save: {e}"),
                None => m.status = "· mirada: sin ruta de config".to_string(),
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

/// El valor de texto actual de un campo de app (para sembrar el buffer al
/// enfocarlo). Vacío si la ruta no resuelve o el campo no es texto.
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
// Resolución del theme + acento
// =====================================================================

/// Construye el Theme efectivo a partir de la config: variant + accent
/// override. Si `variant` no se reconoce, cae a Dark. Si `accent` es
/// `"default"` o desconocido, deja el accent del preset base.
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

fn build_nav(model: &Model, theme: &Theme, _accent: llimphi_ui::llimphi_raster::peniko::Color) -> View<Msg> {
    // Las categorías builtin (0..N) + las secciones de las apps suscritas
    // (APP_DIENTE_BASE+i) como un único rail de dientes.
    let apps = app_sections(model);
    let mut items: Vec<DockRailItem> = Category::all()
        .iter()
        .enumerate()
        .map(|(i, cat)| DockRailItem {
            id: i as u64,
            active: model.app_sel.is_none() && model.category == *cat,
        })
        .collect();
    for i in 0..apps.len() {
        items.push(DockRailItem {
            id: APP_DIENTE_BASE + i as u64,
            active: model.app_sel == Some(i),
        });
    }
    let cat_glyphs: Vec<String> = Category::all().iter().map(|c| c.glyph().to_string()).collect();
    let app_glyphs: Vec<String> = apps.iter().map(|a| a.icon.clone()).collect();

    let rail = dock_rail_view(
        &items,
        44.0,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| {
            let g = if id >= APP_DIENTE_BASE {
                app_glyphs.get((id - APP_DIENTE_BASE) as usize).cloned()
            } else {
                cat_glyphs.get(id as usize).cloned()
            };
            glyph_icon(g, size, color)
        },
        Msg::NavSelect,
        |_| None,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(NAV_WIDTH),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![rail])
}

/// Pinta el glifo de un diente centrado, con el color ya resuelto por el rail.
fn glyph_icon(
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
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        glyph.unwrap_or_else(|| "•".to_string()),
        size * 0.85,
        color,
        Alignment::Center,
    )
}

fn build_content(
    model: &Model,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    // Diente de app activo: pinto su sección de schema con el renderizador.
    if let Some(i) = model.app_sel {
        let apps = app_sections(model);
        if let Some(app) = apps.get(i) {
            let base = FieldPath::empty().push(app.full_id.clone());
            return section_view(&app.section, &base, &model.allichay, theme, Msg::Allichay);
        }
    }
    let head = section_head(model.category, theme);
    let body = match model.category {
        Category::Appearance => section_appearance(model, theme, accent),
        Category::Language => section_language(model, theme, accent),
        Category::Apps => section_apps(theme),
        Category::Monitor => section_monitor(model, theme),
        Category::Modules => section_modules(model, theme, accent),
        Category::About => section_about(model, theme),
    };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(18.0_f32),
            bottom: length(18.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(12.0_f32),
        },
        ..Default::default()
    })
    .children(vec![head, body])
}

fn section_head(cat: Category, theme: &Theme) -> View<Msg> {
    let title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t(cat.i18n_key()),
        16.0,
        theme.fg_text,
        Alignment::Start,
    );
    let hint = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t(cat.hint_key()),
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );
    let underline = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![title, hint, underline])
}

fn section_appearance(
    model: &Model,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let variant_row = labelled_row(
        rimay_localize::t("wawa-panel-label-variant"),
        segmented(
            THEME_VARIANTS
                .iter()
                .map(|(id, key)| {
                    (
                        rimay_localize::t(key),
                        model.cfg.theme_variant == *id,
                        Msg::SetThemeVariant((*id).to_string()),
                    )
                })
                .collect(),
            theme,
            accent,
        ),
        theme,
    );
    let accent_row = labelled_row(
        rimay_localize::t("wawa-panel-label-accent"),
        segmented(
            ACCENTS
                .iter()
                .map(|(id, label)| {
                    (
                        (*label).to_string(),
                        model.cfg.accent == *id,
                        Msg::SetAccent((*id).to_string()),
                    )
                })
                .collect(),
            theme,
            accent,
        ),
        theme,
    );
    column(vec![variant_row, accent_row])
}

fn section_language(
    model: &Model,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let lang_row = labelled_row(
        rimay_localize::t("wawa-panel-label-language"),
        segmented(
            LANGS
                .iter()
                .map(|(id, label)| {
                    (
                        (*label).to_string(),
                        model.cfg.lang == *id,
                        Msg::SetLang((*id).to_string()),
                    )
                })
                .collect(),
            theme,
            accent,
        ),
        theme,
    );
    let clock_row = labelled_row(
        rimay_localize::t("wawa-panel-label-clock"),
        segmented(
            vec![
                (
                    rimay_localize::t("wawa-panel-clock-24h"),
                    model.cfg.timefmt_24h,
                    Msg::SetTimeFmt(true),
                ),
                (
                    rimay_localize::t("wawa-panel-clock-12h"),
                    !model.cfg.timefmt_24h,
                    Msg::SetTimeFmt(false),
                ),
            ],
            theme,
            accent,
        ),
        theme,
    );
    column(vec![lang_row, clock_row])
}

fn section_apps(theme: &Theme) -> View<Msg> {
    let palette = ButtonPalette::from_theme(theme);
    let rows: Vec<View<Msg>> = APPS
        .iter()
        .map(|(bin, name, desc)| app_row(bin, name, desc, &palette, theme))
        .collect();
    column_padded(rows)
}

fn app_row(bin: &str, name: &str, desc: &str, palette: &ButtonPalette, theme: &Theme) -> View<Msg> {
    let label_col = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: length(36.0_f32),
        },
        flex_grow: 1.0,
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(16.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(name.to_string(), 13.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(14.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(desc.to_string(), 11.0, theme.fg_muted, Alignment::Start),
    ]);

    let launch = button_styled(
        rimay_localize::t("wawa-panel-action-launch"),
        Style {
            size: Size {
                width: length(90.0_f32),
                height: length(28.0_f32),
            },
            ..Default::default()
        },
        Alignment::Center,
        palette,
        Msg::LaunchApp(bin.to_string()),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(44.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(10.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .children(vec![label_col, launch])
}

fn section_monitor(model: &Model, theme: &Theme) -> View<Msg> {
    let used_kb = model
        .host
        .mem_total_kb
        .saturating_sub(model.host.mem_avail_kb);
    let stats = vec![
        (
            rimay_localize::t("wawa-panel-stat-time"),
            fmt_clock(model.host.hms, model.cfg.timefmt_24h),
            if std::env::var("TZ_OFFSET_SEC").is_ok() {
                "local".to_string()
            } else {
                "utc".to_string()
            },
        ),
        (
            rimay_localize::t("wawa-panel-stat-uptime"),
            fmt_uptime(model.host.uptime),
            String::new(),
        ),
        (
            rimay_localize::t("wawa-panel-stat-mem"),
            fmt_mem(used_kb, model.host.mem_total_kb),
            if model.host.mem_total_kb > 0 {
                format!(
                    "{:.0}%",
                    100.0 * used_kb as f64 / model.host.mem_total_kb as f64
                )
            } else {
                String::new()
            },
        ),
        (
            rimay_localize::t("wawa-panel-stat-load"),
            format!(
                "{:.2} · {:.2} · {:.2}",
                model.host.load.0, model.host.load.1, model.host.load.2
            ),
            "1m · 5m · 15m".to_string(),
        ),
        (
            rimay_localize::t("wawa-panel-stat-host"),
            model.host.host.clone(),
            String::new(),
        ),
        (
            rimay_localize::t("wawa-panel-stat-kernel"),
            model.host.kernel.clone(),
            String::new(),
        ),
    ];

    let cells: Vec<View<Msg>> = stats
        .into_iter()
        .map(|(label, value, sub)| stat_cell(&label, &value, &sub, theme))
        .collect();
    // 3 columnas en flex-wrap; los cells tienen width:30% para llenar.
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(cells)
}

fn stat_cell(label: &str, value: &str, sub: &str, theme: &Theme) -> View<Msg> {
    let label_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(14.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 10.5, theme.fg_muted, Alignment::Start);
    let value_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(value.to_string(), 14.0, theme.fg_text, Alignment::Start);
    let sub_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(13.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(sub.to_string(), 10.0, theme.fg_placeholder, Alignment::Start);

    // Aproximadamente 1/3 del ancho disponible con un poco de gap.
    let mut children = vec![label_v, value_v];
    if !sub.is_empty() {
        children.push(sub_v);
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.32_f32),
            height: length(74.0_f32),
        },
        flex_grow: 0.0,
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(3.0)
    .children(children)
}

fn section_modules(
    model: &Model,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let rows: Vec<View<Msg>> = MODULES
        .iter()
        .map(|(id, glyph, key)| {
            let label = format!("{}  {}", glyph, rimay_localize::t(key));
            let on = model.cfg.module_enabled(id);
            module_row(id, label, on, theme, accent)
        })
        .collect();
    column_padded(rows)
}

fn module_row(
    id: &str,
    label: String,
    on: bool,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let label_v = View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(36.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label, 13.0, theme.fg_text, Alignment::Start);

    let toggle = toggle_chip(on, theme, accent, Msg::ToggleModule(id.to_string()));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(40.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(0.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .children(vec![label_v, toggle])
}

fn toggle_chip(
    on: bool,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
    msg: Msg,
) -> View<Msg> {
    // Pill 60x22 con bolita a izq/der. La construyo como container fill
    // de fondo + child posicionado.
    let (bg, knob_offset) = if on {
        (accent, 38.0_f32)
    } else {
        (theme.bg_input, 2.0_f32)
    };
    let knob = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(knob_offset),
            right: auto(),
            top: length(2.0_f32),
            bottom: auto(),
        },
        size: Size {
            width: length(18.0_f32),
            height: length(18.0_f32),
        },
        ..Default::default()
    })
    .fill(if on { theme.fg_text } else { theme.fg_muted })
    .radius(9.0);

    View::new(Style {
        size: Size {
            width: length(60.0_f32),
            height: length(22.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(11.0)
    .children(vec![knob])
    .on_click(msg)
}

fn section_about(model: &Model, theme: &Theme) -> View<Msg> {
    let blurb = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(60.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t("wawa-panel-about-blurb"),
        13.0,
        theme.fg_text,
        Alignment::Start,
    );

    let kv = |k: &str, v: &str| -> View<Msg> {
        let key = View::new(Style {
            size: Size {
                width: length(130.0_f32),
                height: length(22.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(k.to_string(), 12.0, theme.fg_muted, Alignment::Start);
        let val = View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: length(22.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(v.to_string(), 12.0, theme.fg_text, Alignment::Start);
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![key, val])
    };

    let rows = vec![
        kv(
            &rimay_localize::t("wawa-panel-about-name"),
            "wawa",
        ),
        kv(
            &rimay_localize::t("wawa-panel-about-version"),
            env!("CARGO_PKG_VERSION"),
        ),
        kv(
            &rimay_localize::t("wawa-panel-about-kernel"),
            if model.host.kernel.is_empty() { "—" } else { &model.host.kernel },
        ),
        kv(&rimay_localize::t("wawa-panel-about-toolkit"), "llimphi"),
        kv(
            &rimay_localize::t("wawa-panel-stat-host"),
            if model.host.host.is_empty() { "—" } else { &model.host.host },
        ),
    ];

    let mut children: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 1);
    children.push(blurb);
    children.extend(rows);
    column(children)
}

// =====================================================================
// Status bar (pie de página)
// =====================================================================

fn build_status(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = ButtonPalette::from_theme(theme);
    let save_btn = button_styled(
        rimay_localize::t("wawa-panel-action-save"),
        Style {
            size: Size {
                width: length(110.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        },
        Alignment::Center,
        &palette,
        Msg::Save,
    );
    let reset_btn = button_styled(
        rimay_localize::t("wawa-panel-action-reset"),
        Style {
            size: Size {
                width: length(100.0_f32),
                height: length(24.0_f32),
            },
            ..Default::default()
        },
        Alignment::Center,
        &palette,
        Msg::Reset,
    );

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
            height: length(34.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: Rect {
            left: length(14.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![msg_v, save_btn, reset_btn])
}

// =====================================================================
// Helpers de composición
// =====================================================================

fn labelled_row(label: String, control: View<Msg>, theme: &Theme) -> View<Msg> {
    let label_v = View::new(Style {
        size: Size {
            width: length(140.0_f32),
            height: length(ROW_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label, 12.0, theme.fg_muted, Alignment::Start);
    let control_box = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: Dimension::auto(),
            height: length(ROW_HEIGHT),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![control]);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(ROW_HEIGHT),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![label_v, control_box])
}

fn segmented(
    options: Vec<(String, bool, Msg)>,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let chips: Vec<View<Msg>> = options
        .into_iter()
        .map(|(label, active, msg)| seg_chip(label, active, msg, theme, accent))
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        padding: Rect {
            left: length(2.0_f32),
            right: length(2.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        gap: Size {
            width: length(2.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_input)
    .radius(4.0)
    .children(chips)
}

fn seg_chip(
    label: String,
    active: bool,
    msg: Msg,
    theme: &Theme,
    accent: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let (bg, fg) = if active {
        (theme.bg_button, theme.fg_text)
    } else {
        (theme.bg_input, theme.fg_muted)
    };
    let style = Style {
        size: Size {
            width: Dimension::auto(),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    };
    let mut v = View::new(style)
        .fill(bg)
        .hover_fill(theme.bg_button_hover)
        .radius(3.0)
        .text_aligned(label, 11.5, fg, Alignment::Center)
        .on_click(msg);
    if active {
        // Pequeña marca superior con el acento: una barra de 2px arriba.
        let bar = View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: auto(),
            },
            size: Size {
                width: percent(1.0_f32),
                height: length(2.0_f32),
            },
            ..Default::default()
        })
        .fill(accent);
        v = v.children(vec![bar]);
    }
    v
}

fn column(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

fn column_padded(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}
