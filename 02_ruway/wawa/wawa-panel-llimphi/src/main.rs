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
mod greeter;
mod iconos;
mod autologin;
mod pacha;
mod paloma;
mod plugins;
mod remote;
mod splash;
mod perfiles;
mod shuma_shortcuts;
mod themes;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

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
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_toast::{toast_stack_view, Toast};
// Editor de recorrido (Prezi) de la vista espacial — lienzo libre + rotación.
use pluma_deck_core::{Camara, ContenidoMarco, Marco, Recorrido, RecorridoState, Rect as DeckRect};
use pluma_deck_recorrido_llimphi::{panel_actual, recorrido_view_editor};
use llimphi_widget_app_header::{app_header_iconed, AppHeaderPalette, AppIcon};
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
/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: std::time::Duration = std::time::Duration::from_secs(4);

/// Hash estable de una cadena → `key` para animaciones implícitas (la misma
/// escena/item produce siempre la misma key entre rebuilds).
fn key_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}
/// Ancho del rail de pestañas (la tira que sobresale).
const RAIL_W: f32 = 46.0;
/// Ancho del sidebar de items (a la izquierda).
const SIDEBAR_W: f32 = 232.0;
/// Alto del viewport del panel (para el scroll). Conservador respecto del alto
/// de ventana inicial menos menubar/header/status; si la ventana es más alta
/// queda algo de aire abajo. (Mejorable cuando el host trackee el resize.)
const VIEWPORT_H: f32 = 500.0;
/// Alto reservado para el editor visual del Prezi en «Vista espacial» (lienzo +
/// título + padding). Se le resta a [`VIEWPORT_H`] para el viewport de los
/// campos, así editor y campos reparten el alto en lugar de aplastarse.
const PREZI_EDITOR_H: f32 = 280.0;

/// Alto reservado para el editor visual de disposición de monitores en la
/// sección «Monitores» (mismo criterio que [`PREZI_EDITOR_H`]).
const MONITOR_EDITOR_H: f32 = 280.0;

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
/// Orden: Vista=0, Themes=1, Atajos=2, Animaciones=3, Pata=4, Inicio=5,
/// Sistema=6, Acerca=7.
const INFO_DIENTE: usize = 7;
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
    /// Último item abierto por diente (índice de pestaña → índice de sección).
    /// Al entrar a un diente aterrizamos en su último tab (o el 1º), sin pasar
    /// por el canvas-resumen — ahorra un clic.
    last_item: std::collections::HashMap<usize, usize>,
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
    /// Atajos de **terminal de shuma** (perfil activo + nombres disponibles),
    /// leídos de `~/.config/shuma/shortcuts.ron`. El selector del panel conmuta
    /// el activo; toma efecto en el próximo arranque de shuma.
    shuma_atajos_active: String,
    shuma_atajos_names: Vec<String>,
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
    /// Config de **cuentas de correo** (paloma): varias cuentas IMAP/SMTP con
    /// método de auth (contraseña u OAuth2). Se persiste en `cuentas.json`; el
    /// diente «Correo» la edita y paloma la lee al arrancar.
    paloma: paloma::PalomaState,
    /// Catálogo de **contextos** (pacha): modos de uso con nombre. El diente
    /// «Contextos» edita `pachas.ron` y muestra el estado del cifrado de dotfiles.
    pacha: pacha::PachaState,
    /// Política de **autologin** (entrar sin contraseña). El diente «Inicio» la
    /// edita; el greeter la lee. Incluye el tradeoff de secretos.
    autologin: autologin::AutologinState,
    /// Config del **greeter** (DM): fondo animado + paleta. Se persiste en
    /// `greeter.conf`; el greeter la lee en el próximo login.
    greeter: greeter::GreeterCfg,
    /// Config del **splash de arranque** (`arje-splash`): fuente (logo/imagen/
    /// animación) + colores + panel de logs. Se persiste en `arje/splash.conf`;
    /// el instalador la hornea en el initramfs (ver [`splash`]).
    splash: splash::SplashCfg,
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
    /// Editor de recorrido (Prezi) de la sección «Vista espacial»: lienzo libre
    /// con un marco por escritorio (mover/zoom/rotar). Se sincroniza a
    /// `mirada.overview_places` en cada edición.
    prezi: PreziEdit,
    /// Editor visual de **disposición de monitores** de la sección «Monitores»:
    /// cajas (una por salida conectada) arrastrables en un lienzo libre — como
    /// el Prezi pero SIN giro y con tamaños propios (la resolución de cada
    /// pantalla). Vuelca el orden/disposición a `mirada.outputs`.
    monitor: MonitorEdit,
    /// Editor de **sesión remota** (waypipe) abierto en una subventana, o `None`
    /// si está cerrado. Edita un borrador de `StartupApp`; al guardar se vuelca a
    /// `mirada.startup` (ver [`remote`]).
    remote_edit: Option<remote::RemoteEdit>,
    /// Estado allichay propio del overlay de sesión remota (buffers de texto y
    /// foco), separado del [`Model::allichay`] del panel de fondo.
    remote_allichay: AllichayState,
    /// Plugins de mirada leídos de `~/.config/mirada/plugins` (para la lista del
    /// diente Inicio). Se relee al abrir/guardar el editor.
    mirada_plugins: Vec<plugins::PluginInfo>,
    /// Editor de **reglas de un plugin** (el asignador) en una subventana, o
    /// `None` si está cerrado (ver [`plugins`]).
    plugin_edit: Option<plugins::PluginEdit>,
    /// Estado allichay propio del overlay de plugins (buffers de texto y foco).
    plugin_allichay: AllichayState,
    /// Toasts vivos (confirmaciones de guardado, errores de persistencia). Se
    /// purgan por TTL en cada [`Msg::Tick`] y al clickearlos ([`Msg::ToastExpire`]).
    toasts: Vec<Toast>,
    /// Id incremental para correlacionar toast ↔ dismiss.
    next_toast: u64,
}

/// Px de mundo por celda de la grilla del Prezi dentro del editor de recorrido.
/// Sólo es la escala de autoría; lo persistido va en **unidades de celda**.
const PREZI_CELL: f64 = 240.0;

/// Estado del editor de recorrido (Prezi) embebido en «Vista espacial». Mapea
/// 1:1 escritorio↔marco — el id del marco es `i+1` (escritorio `i`).
struct PreziEdit {
    rec: Recorrido,
    state: RecorridoState,
    /// Marco seleccionado (objetivo de rotar). `None` = nada elegido.
    sel: Option<u64>,
    /// Estado de arrastre fijado en el primer Move: `None` = sin arrastre,
    /// `Some(None)` = paneando el lienzo, `Some(Some(id))` = moviendo ese marco.
    grip: Option<Option<u64>>,
}

impl PreziEdit {
    /// Construye el editor desde el plano rico del config (o la grilla derivada).
    fn from_config(cfg: &mirada_brain::Config) -> Self {
        let n = mirada_brain::action::WORKSPACE_COUNT;
        let places = cfg.overview_places_for(n);
        let mut rec = Recorrido::new();
        for (i, p) in places.iter().enumerate() {
            let id = (i + 1) as u64;
            let rect = DeckRect::new(
                p.x as f64 * PREZI_CELL,
                p.y as f64 * PREZI_CELL,
                (p.w as f64 * PREZI_CELL).max(40.0),
                (p.h as f64 * PREZI_CELL).max(40.0),
            );
            rec.agregar_marco(
                Marco::new(
                    id,
                    rect,
                    ContenidoMarco::Croquis {
                        titulo: Some(format!("{id}")),
                        cajas: croquis_escritorio(id),
                    },
                )
                .con_giro(p.rot as f64),
            );
            rec.pasos.push(id);
        }
        // Encuadre inicial centrado en el plano, sin depender del tamaño del panel
        // (se reajusta con la rueda). El zoom busca que el ancho del plano entre
        // en ~440 px del canvas del panel.
        let (centro, span_w) = rec
            .bbox()
            .map(|b| (b.centro(), b.w.max(1.0)))
            .unwrap_or(((0.0, 0.0), PREZI_CELL));
        let mut state = RecorridoState::new();
        state.camara = Camara::new(centro, (440.0 / span_w).clamp(0.08, 1.5), 0.0);
        Self { rec, state, sel: None, grip: None }
    }

    /// Vuelca los marcos a `overview_places` (unidades de celda + giro en rad),
    /// en orden de escritorio (id ascendente).
    fn to_places(&self) -> Vec<mirada_brain::OverviewPlace> {
        let mut marcos: Vec<&Marco> = self.rec.marcos.iter().collect();
        marcos.sort_by_key(|m| m.id);
        marcos
            .into_iter()
            .map(|m| {
                mirada_brain::OverviewPlace::new(
                    (m.rect.x / PREZI_CELL) as f32,
                    (m.rect.y / PREZI_CELL) as f32,
                    (m.rect.w / PREZI_CELL) as f32,
                    (m.rect.h / PREZI_CELL) as f32,
                    m.rot_rad as f32,
                )
            })
            .collect()
    }
}

/// Miniatura esquemática del escritorio `id` para el croquis del editor: un
/// patrón de teselado **representativo** (no las ventanas vivas — el panel edita
/// config, no observa el compositor), variado por `id` para que el plano se vea
/// poblado y deje de parecer un numpad. Cajas `[x,y,w,h]` normalizadas a `0..1`.
/// Algunos escritorios quedan vacíos a propósito (como en un escritorio real).
fn croquis_escritorio(id: u64) -> Vec<[f32; 4]> {
    match (id - 1) % 6 {
        // Monocle: una ventana grande.
        0 => vec![[0.07, 0.10, 0.86, 0.80]],
        // Master + 2 apiladas a la derecha.
        1 => vec![
            [0.06, 0.10, 0.50, 0.80],
            [0.60, 0.10, 0.34, 0.37],
            [0.60, 0.53, 0.34, 0.37],
        ],
        // Grilla 2×2.
        2 => vec![
            [0.06, 0.09, 0.41, 0.39],
            [0.53, 0.09, 0.41, 0.39],
            [0.06, 0.52, 0.41, 0.39],
            [0.53, 0.52, 0.41, 0.39],
        ],
        // Tres columnas.
        3 => vec![
            [0.06, 0.10, 0.27, 0.80],
            [0.37, 0.10, 0.26, 0.80],
            [0.67, 0.10, 0.27, 0.80],
        ],
        // Master + 3 apiladas.
        4 => vec![
            [0.06, 0.10, 0.50, 0.80],
            [0.60, 0.10, 0.34, 0.24],
            [0.60, 0.38, 0.34, 0.24],
            [0.60, 0.66, 0.34, 0.24],
        ],
        // Escritorio vacío.
        _ => Vec::new(),
    }
}

/// Sensibilidad de la manija de giro: rad por px de arrastre horizontal.
const PREZI_ROT_SENS: f64 = 0.01;

/// Sincroniza el plano del editor → `mirada.overview_places` del perfil activo y
/// arma el guardado diferido.
fn prezi_sync(m: &mut Model) {
    m.mirada.overview_places = m.prezi.to_places();
    m.dirty.mirada = true;
    sync_active_profile(m);
    m.save_in = SAVE_DELAY_TICKS;
}

/// Px de mundo por píxel **lógico** de monitor en el lienzo del editor de
/// disposición. 2560 px lógicos → 640 de mundo: cajas del orden de un marco
/// Prezi, que la cámara encuadra sola.
const MON_SCALE: f64 = 0.25;
/// Separación (px de mundo) entre cajas de monitor al disponerlas en línea.
const MON_GAP: f64 = 80.0;

/// Estado del editor visual de **disposición de monitores** embebido en la
/// sección «Monitores». Cada salida conectada es un marco (caja) arrastrable en
/// un lienzo libre — como el Prezi pero SIN giro y con tamaños propios (la
/// resolución lógica de cada pantalla). El orden/forma se vuelca a
/// `mirada.outputs` (campo `order`) + `output_direction`.
struct MonitorEdit {
    rec: Recorrido,
    state: RecorridoState,
    /// Marco (monitor) seleccionado. `None` = nada elegido.
    sel: Option<u64>,
    /// Presa fijada en el primer Move: `None` sin arrastre, `Some(None)` paneo,
    /// `Some(Some(id))` moviendo ese marco.
    grip: Option<Option<u64>>,
    /// Nombre de conector DRM por marco (índice = `id - 1`).
    nombres: Vec<String>,
}

/// Parsea un modo DRM (`"2560x1440"`, `"1920x1080@60"`) a `(ancho, alto)` en px.
fn parse_modo(modo: &str) -> Option<(f64, f64)> {
    let core = modo.split(['@', ' ']).next()?;
    let (w, h) = core.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

impl MonitorEdit {
    /// Construye el editor desde los monitores DRM conectados + los overrides de
    /// mirada (orden, escala, disposición). Los dispone en línea según
    /// `output_direction`, cada caja con su tamaño **lógico** (resolución / escala).
    fn from_config(cfg: &mirada_brain::Config) -> Self {
        // (nombre, modo nativo, ancho lógico, alto lógico).
        let mut mons: Vec<(String, String, f64, f64)> = read_monitors()
            .into_iter()
            .map(|(name, modo)| {
                let (pw, ph) = parse_modo(&modo).unwrap_or((1920.0, 1080.0));
                let escala = (cfg.output_scale_120_for(&name) as f64 / 120.0).max(0.1);
                (name, modo, pw / escala, ph / escala)
            })
            .collect();
        // Orden = el configurado en mirada `(order, name)`, estable y reproducible.
        mons.sort_by(|a, b| {
            cfg.output_order_for(&a.0)
                .cmp(&cfg.output_order_for(&b.0))
                .then_with(|| a.0.cmp(&b.0))
        });

        let horizontal = cfg.output_direction != "vertical";
        let mut rec = Recorrido::new();
        let mut nombres = Vec::new();
        let mut avance = 0.0;
        for (i, (name, modo, lw, lh)) in mons.iter().enumerate() {
            let (w, h) = (lw * MON_SCALE, lh * MON_SCALE);
            let rect = if horizontal {
                DeckRect::new(avance, 0.0, w, h)
            } else {
                DeckRect::new(0.0, avance, w, h)
            };
            avance += if horizontal { w } else { h } + MON_GAP;
            let id = (i + 1) as u64;
            // Sin giro (default rot=0): los monitores no rotan en este editor.
            rec.agregar_marco(Marco::new(
                id,
                rect,
                ContenidoMarco::Texto {
                    titulo: Some(name.clone()),
                    parrafos: vec![modo.clone()],
                },
            ));
            // Sin `pasos`: sin ruta narrativa ni HUD «paso X/N» en el lienzo.
            nombres.push(name.clone());
        }

        let (centro, span) = rec
            .bbox()
            .map(|b| (b.centro(), b.w.max(b.h).max(1.0)))
            .unwrap_or(((0.0, 0.0), 480.0));
        let mut state = RecorridoState::new();
        state.camara = Camara::new(centro, (520.0 / span).clamp(0.05, 2.0), 0.0);
        Self { rec, state, sel: None, grip: None, nombres }
    }

    /// Vuelca la disposición del editor a `(outputs, output_direction)`:
    /// - la **dirección** sale de la forma de la nube de cajas (más ancha que
    ///   alta → horizontal; si no, vertical);
    /// - el **orden** sale de la posición de cada caja sobre el eje dominante.
    /// Preserva los demás campos del override (wallpaper/escala/transform) y los
    /// overrides de monitores ahora desconectados (no presentes en el editor).
    fn to_outputs(
        &self,
        prev: &[mirada_brain::OutputOverride],
    ) -> (Vec<mirada_brain::OutputOverride>, String) {
        let horizontal = self.rec.bbox().map(|b| b.w >= b.h).unwrap_or(true);
        let mut marcos: Vec<&Marco> = self.rec.marcos.iter().collect();
        marcos.sort_by(|a, b| {
            let (ka, kb) = if horizontal {
                (a.rect.x, b.rect.x)
            } else {
                (a.rect.y, b.rect.y)
            };
            ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut outs: Vec<mirada_brain::OutputOverride> = Vec::new();
        for (order, m) in marcos.iter().enumerate() {
            let Some(name) = self.nombres.get((m.id - 1) as usize).cloned() else {
                continue;
            };
            let base = prev.iter().find(|o| o.name == name);
            outs.push(mirada_brain::OutputOverride {
                name: name.clone(),
                wallpaper_path: base.map(|b| b.wallpaper_path.clone()).unwrap_or_default(),
                wallpaper_fit: base.map(|b| b.wallpaper_fit.clone()).unwrap_or_default(),
                order: order as i32,
                scale_120: base.map(|b| b.scale_120).unwrap_or(0),
                transform: base.map(|b| b.transform.clone()).unwrap_or_default(),
            });
        }
        // Conservamos overrides de salidas desconectadas (no las pisamos al guardar).
        for o in prev {
            if !outs.iter().any(|x| x.name == o.name) {
                outs.push(o.clone());
            }
        }
        let dir = if horizontal { "horizontal" } else { "vertical" }.to_string();
        (outs, dir)
    }
}

/// Sincroniza la disposición del editor → `mirada.outputs` + `output_direction`
/// del perfil activo y arma el guardado diferido.
fn monitor_sync(m: &mut Model) {
    let (outs, dir) = m.monitor.to_outputs(&m.mirada.outputs);
    m.mirada.outputs = outs;
    m.mirada.output_direction = dir;
    m.dirty.mirada = true;
    sync_active_profile(m);
    m.save_in = SAVE_DELAY_TICKS;
}

/// `true` si la sección abierta en el canvas es «Monitores» de mirada (donde
/// vive el editor de disposición) — gatea el ruteo de rueda a su lienzo.
fn monitor_section_active(m: &Model) -> bool {
    let pestanas = pestanas(m);
    let pest = m.selected_pest.min(pestanas.len().saturating_sub(1));
    let Some(secs) = pestanas.get(pest).map(|p| p.schema.sections.as_slice()) else {
        return false;
    };
    m.selected_item
        .filter(|&i| i < secs.len())
        .and_then(|i| secs.get(i))
        .is_some_and(|s| s.id.contains("mirada::monitores"))
}

/// `true` si la sección abierta en el canvas es «Vista espacial» (donde vive el
/// editor de recorrido) — gatea el ruteo de rueda/teclado a su lienzo.
fn prezi_section_active(m: &Model) -> bool {
    let pestanas = pestanas(m);
    let pest = m.selected_pest.min(pestanas.len().saturating_sub(1));
    let Some(secs) = pestanas.get(pest).map(|p| p.schema.sections.as_slice()) else {
        return false;
    };
    m.selected_item
        .filter(|&i| i < secs.len())
        .and_then(|i| secs.get(i))
        .is_some_and(|s| s.id.contains("vista_espacial"))
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
    /// Config del greeter (`greeter.conf`).
    greeter: bool,
    /// Config del splash de arranque (`arje/splash.conf`).
    splash: bool,
    /// Config de cuentas de correo de paloma (`cuentas.json`).
    paloma: bool,
    /// Catálogo de contextos de pacha (`pachas.ron`).
    pacha: bool,
    /// Política de autologin (`autologin.conf`).
    autologin: bool,
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
    /// Editor de recorrido (Prezi): arrastre sobre el lienzo — `(dx,dy)` delta +
    /// `(lx,ly)` posición del press (decide en el 1er Move si agarra marco o vacío).
    PreziDrag { dx: f32, dy: f32, lx: f32, ly: f32 },
    /// Fin del arrastre del lienzo (suelta la presa).
    PreziDragEnd,
    /// Zoom-a-cursor del lienzo del Prezi (rueda).
    PreziZoom { mult: f64, cursor: (f32, f32) },
    /// Rota el marco seleccionado `delta` rad (teclado `[` / `]`).
    PreziRotate(f64),
    /// Rota el marco seleccionado arrastrando la manija: `dx` px de pantalla.
    PreziRotateHandle(f32),
    /// Editor de disposición de monitores: arrastre sobre el lienzo — `(dx,dy)`
    /// delta + `(lx,ly)` posición del press (decide en el 1er Move si agarra una
    /// caja-monitor o el vacío).
    MonitorDrag { dx: f32, dy: f32, lx: f32, ly: f32 },
    /// Fin del arrastre del lienzo de monitores (suelta la presa).
    MonitorDragEnd,
    /// Zoom-a-cursor del lienzo de monitores (rueda).
    MonitorZoom { mult: f64, cursor: (f32, f32) },
    /// Cambió la config del SO desde afuera (otro panel, edición manual).
    ConfigChanged(Box<WawaConfig>),
    MenuOpen(Option<usize>),
    MenuCommand(String),
    MenuNav(i32),
    MenuActivate,
    MenuTick,
    CloseMenus,
    /// Compositor de widgets de barra: agregar `kind` al slot `slot` (0=inicio,
    /// 1=centro, 2=fin) de la superficie `surf`.
    BarWidgetAdd(usize, u8, String),
    /// Quitar el widget `idx` del slot `slot` de la superficie `surf`.
    BarWidgetRemove(usize, u8, usize),
    /// Compositor de dientes de sidebar: agregar un diente con contenido `kind`
    /// a la superficie `surf`.
    SidebarTabAdd(usize, String),
    /// Quitar el diente `idx` de la superficie `surf`.
    SidebarTabRemove(usize, usize),
    /// Subventana de sesión remota: cambio/foco/scroll del formulario (allichay).
    RemoteEditMsg(AllichayMsg),
    /// Tecla al campo de texto en edición de la subventana de sesión remota.
    RemoteEditKey(KeyEvent),
    /// Cerrar la subventana de sesión remota sin guardar (click en el scrim).
    RemoteEditCancel,
    /// Subventana de reglas de plugin: cambio/foco/scroll del formulario (allichay).
    PluginEditMsg(AllichayMsg),
    /// Tecla al campo de texto en edición de la subventana de plugin.
    PluginEditKey(KeyEvent),
    /// Cerrar la subventana de plugin sin guardar (click en el scrim).
    PluginEditCancel,
    /// Un toast fue descartado a mano (click): se quita del stack.
    ToastExpire(u64),
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
                // Purga de toasts expirados (TTL).
                if !m.toasts.is_empty() {
                    let now = Instant::now();
                    m.toasts.retain(|t| t.is_alive(now));
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
                    // Aterriza en el último tab abierto de este diente (o el 1º),
                    // sin pasar por el canvas-resumen — ahorra un clic.
                    let secs = pestanas(&m).get(id).map(|p| p.schema.sections.len()).unwrap_or(0);
                    let item = m.last_item.get(&id).copied().unwrap_or(0).min(secs.saturating_sub(1));
                    m.selected_item = Some(item);
                    m.allichay.select(item);
                }
                m.status.clear();
            }
            Msg::SelectItem(id) => {
                m.selected_item = Some(id as usize);
                m.last_item.insert(m.selected_pest, id as usize);
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
            Msg::BarWidgetAdd(surf, slot, kind) => {
                if let Some(s) = m.pata.surfaces.get_mut(surf) {
                    let dst = match slot {
                        0 => &mut s.start,
                        1 => &mut s.center,
                        _ => &mut s.end,
                    };
                    dst.push(pata_core::WidgetSpec::new(kind));
                    m.dirty.pata = true;
                    sync_active_profile(&mut m);
                }
            }
            Msg::BarWidgetRemove(surf, slot, idx) => {
                if let Some(s) = m.pata.surfaces.get_mut(surf) {
                    let dst = match slot {
                        0 => &mut s.start,
                        1 => &mut s.center,
                        _ => &mut s.end,
                    };
                    if idx < dst.len() {
                        dst.remove(idx);
                        m.dirty.pata = true;
                        sync_active_profile(&mut m);
                    }
                }
            }
            Msg::SidebarTabAdd(surf, kind) => {
                // Icono/rótulo derivados del catálogo (best-effort).
                let (icon, label) = pata_core::widget_catalog()
                    .iter()
                    .find(|w| w.kind == kind)
                    .map(|w| (w.icon.to_string(), w.label.to_string()))
                    .unwrap_or_else(|| ("▫".to_string(), kind.clone()));
                if let Some(s) = m.pata.surfaces.get_mut(surf) {
                    s.tabs.push(pata_core::SidebarTab::new(
                        icon,
                        label,
                        pata_core::WidgetSpec::new(kind),
                    ));
                    m.dirty.pata = true;
                    sync_active_profile(&mut m);
                }
            }
            Msg::SidebarTabRemove(surf, idx) => {
                if let Some(s) = m.pata.surfaces.get_mut(surf) {
                    if idx < s.tabs.len() {
                        s.tabs.remove(idx);
                        m.dirty.pata = true;
                        sync_active_profile(&mut m);
                    }
                }
            }
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
            Msg::PreziDrag { dx, dy, lx, ly } => {
                let panel = panel_actual().unwrap_or(DeckRect::new(0.0, 0.0, 1.0, 1.0));
                // En el primer Move fijamos qué se agarró (marco o vacío) hasta
                // soltar — así no cambia de presa a mitad del arrastre.
                let grip = match m.prezi.grip {
                    Some(g) => g,
                    None => {
                        // `lx,ly` vienen LOCALES al lienzo (origen = esquina del
                        // nodo); `screen_to_world` espera coords ABSOLUTAS de
                        // ventana (resta `panel.centro()`, que es absoluto). Sin
                        // sumar el origen del panel, el hit-test caía corrido por
                        // la posición del editor en la pantalla y NUNCA agarraba un
                        // marco → el editor se sentía read-only (sólo paneaba).
                        let abs = (lx as f64 + panel.x, ly as f64 + panel.y);
                        let world = m.prezi.state.camara.screen_to_world(abs, panel);
                        let hit = m.prezi.rec.marco_en_punto(world);
                        m.prezi.grip = Some(hit);
                        if hit.is_some() {
                            m.prezi.sel = hit; // agarrar un marco lo selecciona
                        }
                        hit
                    }
                };
                match grip {
                    Some(id) => {
                        let (wdx, wdy) =
                            m.prezi.state.camara.delta_pantalla_a_mundo(dx as f64, dy as f64);
                        m.prezi.rec.mover_marco(id, wdx, wdy);
                        prezi_sync(&mut m);
                        m.status = format!("escritorio {id} reubicado");
                    }
                    None => m.prezi.state.arrastrar_delta(dx as f64, dy as f64),
                }
            }
            Msg::PreziDragEnd => m.prezi.grip = None,
            Msg::PreziZoom { mult, cursor } => {
                if let Some(panel) = panel_actual() {
                    m.prezi.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
                }
            }
            Msg::PreziRotate(delta) => {
                if let Some(id) = m.prezi.sel {
                    m.prezi.rec.rotar_marco(id, delta);
                    prezi_sync(&mut m);
                    let deg = m.prezi.rec.marco(id).map(|mr| mr.rot_rad.to_degrees()).unwrap_or(0.0);
                    m.status = format!("escritorio {id} · giro {deg:.0}°");
                }
            }
            Msg::PreziRotateHandle(dx) => {
                if let Some(id) = m.prezi.sel {
                    // Manija de scrub: arrastre horizontal → giro proporcional.
                    m.prezi.rec.rotar_marco(id, dx as f64 * PREZI_ROT_SENS);
                    prezi_sync(&mut m);
                    let deg = m.prezi.rec.marco(id).map(|mr| mr.rot_rad.to_degrees()).unwrap_or(0.0);
                    m.status = format!("escritorio {id} · giro {deg:.0}°");
                }
            }
            Msg::MonitorDrag { dx, dy, lx, ly } => {
                let panel = panel_actual().unwrap_or(DeckRect::new(0.0, 0.0, 1.0, 1.0));
                // En el 1er Move fijamos la presa (caja-monitor o vacío) hasta soltar.
                let grip = match m.monitor.grip {
                    Some(g) => g,
                    None => {
                        // `lx,ly` son LOCALES al lienzo; `screen_to_world` espera
                        // coords ABSOLUTAS de ventana (igual que el Prezi).
                        let abs = (lx as f64 + panel.x, ly as f64 + panel.y);
                        let world = m.monitor.state.camara.screen_to_world(abs, panel);
                        let hit = m.monitor.rec.marco_en_punto(world);
                        m.monitor.grip = Some(hit);
                        if hit.is_some() {
                            m.monitor.sel = hit; // agarrar una caja la selecciona
                        }
                        hit
                    }
                };
                match grip {
                    Some(id) => {
                        let (wdx, wdy) =
                            m.monitor.state.camara.delta_pantalla_a_mundo(dx as f64, dy as f64);
                        m.monitor.rec.mover_marco(id, wdx, wdy);
                        monitor_sync(&mut m);
                        let nombre = m
                            .monitor
                            .nombres
                            .get((id - 1) as usize)
                            .cloned()
                            .unwrap_or_default();
                        m.status = format!("monitor {nombre} reubicado");
                    }
                    None => m.monitor.state.arrastrar_delta(dx as f64, dy as f64),
                }
            }
            Msg::MonitorDragEnd => m.monitor.grip = None,
            Msg::MonitorZoom { mult, cursor } => {
                if let Some(panel) = panel_actual() {
                    m.monitor.state.wheel(mult, (cursor.0 as f64, cursor.1 as f64), panel);
                }
            }
            Msg::AllichayKey(event) => {
                if let Some((path, value)) = m.allichay.apply_key(&event) {
                    route_change(&mut m, &path, value);
                }
            }
            // Subventana de sesión remota: su propio estado allichay y borrador.
            Msg::RemoteEditMsg(am) => match am {
                AllichayMsg::Focus(path) => {
                    if let Some(edit) = &m.remote_edit {
                        let seed = path.leaf().map(|l| edit.text_value(l)).unwrap_or_default();
                        m.remote_allichay.focus(&path, &seed);
                    }
                }
                AllichayMsg::Change(path, value) => {
                    let leaf = path.leaf().unwrap_or("").to_string();
                    match leaf.as_str() {
                        "guardar" if value.as_bool() == Some(true) => remote_edit_save(&mut m),
                        "borrar" if value.as_bool() == Some(true) => remote_edit_delete(&mut m),
                        "cancelar" if value.as_bool() == Some(true) => remote_edit_close(&mut m),
                        _ => {
                            if let Some(edit) = m.remote_edit.as_mut() {
                                edit.apply(&leaf, value);
                            }
                        }
                    }
                }
                AllichayMsg::ScrollTo(off) => m.remote_allichay.set_scroll(off),
                AllichayMsg::SelectSection(_)
                | AllichayMsg::FocusCell(..)
                | AllichayMsg::FocusHex(..) => {}
            },
            Msg::RemoteEditKey(event) => {
                if let Some((path, value)) = m.remote_allichay.apply_key(&event) {
                    if let Some(edit) = m.remote_edit.as_mut() {
                        edit.apply(path.leaf().unwrap_or(""), value);
                    }
                }
            }
            Msg::RemoteEditCancel => remote_edit_close(&mut m),
            // Subventana de reglas de plugin (asignador): mismo patrón que la remota.
            Msg::PluginEditMsg(am) => match am {
                AllichayMsg::Focus(path) => {
                    if let Some(edit) = &m.plugin_edit {
                        let seed = path.leaf().map(|l| edit.text_value(l)).unwrap_or_default();
                        m.plugin_allichay.focus(&path, &seed);
                    }
                }
                AllichayMsg::Change(path, value) => {
                    let leaf = path.leaf().unwrap_or("").to_string();
                    match leaf.as_str() {
                        "guardar" if value.as_bool() == Some(true) => plugin_edit_save(&mut m),
                        "cancelar" if value.as_bool() == Some(true) => plugin_edit_close(&mut m),
                        "add" if value.as_bool() == Some(true) => {
                            if let Some(edit) = m.plugin_edit.as_mut() {
                                edit.add_rule();
                            }
                        }
                        // Quitar entrada: `rule:{i}:del` (asignador) o
                        // `line:{i}:del` (editor de líneas). El índice es el token
                        // antes de «del».
                        _ if leaf.ends_with(":del") && value.as_bool() == Some(true) => {
                            if let Some(i) =
                                leaf.rsplit(':').nth(1).and_then(|s| s.parse::<usize>().ok())
                            {
                                if let Some(edit) = m.plugin_edit.as_mut() {
                                    edit.del_rule(i);
                                }
                            }
                        }
                        _ => {
                            if let Some(edit) = m.plugin_edit.as_mut() {
                                edit.apply(&leaf, value);
                            }
                        }
                    }
                }
                AllichayMsg::ScrollTo(off) => m.plugin_allichay.set_scroll(off),
                AllichayMsg::SelectSection(_)
                | AllichayMsg::FocusCell(..)
                | AllichayMsg::FocusHex(..) => {}
            },
            Msg::PluginEditKey(event) => {
                if let Some((path, value)) = m.plugin_allichay.apply_key(&event) {
                    if let Some(edit) = m.plugin_edit.as_mut() {
                        edit.apply(path.leaf().unwrap_or(""), value);
                    }
                }
            }
            Msg::PluginEditCancel => plugin_edit_close(&mut m),
            Msg::ToastExpire(id) => m.toasts.retain(|t| t.id != id),
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
        // Subventana de sesión remota abierta → sus teclas van a su editor; Esc
        // la cierra. Tiene prioridad (es el modal más arriba).
        if model.remote_edit.is_some() {
            if matches!(event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::RemoteEditCancel);
            }
            if model.remote_allichay.is_editing() {
                return Some(Msg::RemoteEditKey(event.clone()));
            }
            return None;
        }
        // Subventana de reglas de plugin: mismo trato que la remota.
        if model.plugin_edit.is_some() {
            if matches!(event.key, Key::Named(NamedKey::Escape)) {
                return Some(Msg::PluginEditCancel);
            }
            if model.plugin_allichay.is_editing() {
                return Some(Msg::PluginEditKey(event.clone()));
            }
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
        // Editor de recorrido (Prezi) abierto: `[` / `]` rotan el marco elegido.
        if prezi_section_active(model) && model.prezi.sel.is_some() {
            if let Key::Character(c) = &event.key {
                match c.as_str() {
                    "[" => return Some(Msg::PreziRotate(-0.08)),
                    "]" => return Some(Msg::PreziRotate(0.08)),
                    _ => {}
                }
            }
        }
        None
    }

    fn on_wheel(
        model: &Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Msg> {
        // Sólo capturamos la rueda cuando un lienzo (Prezi o monitores) está
        // abierto y el cursor cae dentro de su rect (registrado por el último
        // paint); fuera de ahí devolvemos `None` para no robarle el scroll a los
        // campos de config. Ambos editores comparten el side-channel del rect.
        let prezi = prezi_section_active(model);
        let monitor = monitor_section_active(model);
        if !prezi && !monitor {
            return None;
        }
        let panel = panel_actual()?;
        if !pluma_deck_recorrido_llimphi::dentro(panel, cursor.0, cursor.1) {
            return None;
        }
        let mult = pluma_deck_recorrido_llimphi::ZOOM_BASE.powf(-delta.y as f64);
        if monitor {
            Some(Msg::MonitorZoom { mult, cursor })
        } else {
            Some(Msg::PreziZoom { mult, cursor })
        }
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
        // La subventana de sesión remota es el modal más arriba: scrim + caja con
        // el formulario (renderizado con el mismo schema_panel del panel de fondo).
        if let Some(edit) = &model.remote_edit {
            let schema = edit.schema();
            let box_w = 520.0_f32;
            let box_h = 600.0_f32;
            let form = schema_panel(
                &schema,
                &model.remote_allichay,
                &theme,
                box_h - 24.0,
                Msg::RemoteEditMsg,
            );
            let caja = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: length(box_w), height: length(box_h) },
                padding: Rect {
                    left: length(16.0_f32),
                    right: length(16.0_f32),
                    top: length(12.0_f32),
                    bottom: length(12.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .children(vec![form]);
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
            .on_click(Msg::RemoteEditCancel)
            .children(vec![caja]);
            return Some(scrim);
        }
        // La subventana de reglas de plugin: mismo armado (scrim + caja).
        if let Some(edit) = &model.plugin_edit {
            let schema = edit.schema();
            let box_w = 520.0_f32;
            let box_h = 600.0_f32;
            let form = schema_panel(
                &schema,
                &model.plugin_allichay,
                &theme,
                box_h - 24.0,
                Msg::PluginEditMsg,
            );
            let caja = View::new(Style {
                flex_direction: FlexDirection::Column,
                size: Size { width: length(box_w), height: length(box_h) },
                padding: Rect {
                    left: length(16.0_f32),
                    right: length(16.0_f32),
                    top: length(12.0_f32),
                    bottom: length(12.0_f32),
                },
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .children(vec![form]);
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
            .on_click(Msg::PluginEditCancel)
            .children(vec![caja]);
            return Some(scrim);
        }
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
        let menu_overlay = menubar_overlay_animated(
            &menubar_spec(&menu, model, &theme),
            model.menu_active,
            model.menu_anim.value(),
        );
        // Toasts efímeros (confirmaciones de guardado / errores de persistencia),
        // apilados abajo-derecha. Se componen por encima del overlay de menú.
        let now = Instant::now();
        let alive: Vec<Toast> = model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        let toasts = (!alive.is_empty())
            .then(|| toast_stack_view(&alive, viewport_of(), Msg::ToastExpire));
        match (menu_overlay, toasts) {
            (None, None) => None,
            (Some(o), None) => Some(o),
            (None, Some(t)) => Some(t),
            (Some(o), Some(t)) => Some(
                View::new(Style {
                    position: Position::Absolute,
                    inset: Rect {
                        left: length(0.0_f32),
                        top: length(0.0_f32),
                        right: length(0.0_f32),
                        bottom: length(0.0_f32),
                    },
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![o, t]),
            ),
        }
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

    let (shuma_atajos_active, shuma_atajos_names) = shuma_shortcuts::load();

    let dprofiles = DesktopProfiles::load_or_seed(&mirada);

    let rules_path = mirada_brain::rules::Rules::default_path();
    let rules = rules_path
        .as_deref()
        .map(mirada_brain::rules::Rules::load_or_default)
        .unwrap_or_default();

    let themes = themes::Themes::load_or_seed(&cfg.theme_variant, &cfg.accent);
    let animaciones = animaciones::Animations::load_or_seed();
    let greeter_cfg = greeter::GreeterCfg::load();
    let splash_cfg = splash::SplashCfg::load();
    let paloma = paloma::PalomaState::load();
    let pacha = pacha::PachaState::load();
    let autologin = autologin::AutologinState::load();

    let prezi = PreziEdit::from_config(&mirada);
    let monitor = MonitorEdit::from_config(&mirada);

    Model {
        selected_pest: 0,
        // Arranca en el 1er tab de Vista (no en el canvas-resumen).
        selected_item: Some(0),
        last_item: std::collections::HashMap::new(),
        sidebar_open: true,
        sidebar_w: SIDEBAR_W,
        cfg,
        mirada,
        mirada_path,
        keymap_rows,
        keymap_path,
        profiles,
        profiles_path,
        shuma_atajos_active,
        shuma_atajos_names,
        dprofiles,
        pata,
        rules,
        rules_path,
        themes,
        animaciones,
        paloma,
        pacha,
        autologin,
        greeter: greeter_cfg,
        splash: splash_cfg,
        monitor,
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
        prezi,
        remote_edit: None,
        remote_allichay: AllichayState::new(),
        mirada_plugins: plugins::list_plugins(),
        plugin_edit: None,
        plugin_allichay: AllichayState::new(),
        toasts: Vec::new(),
        next_toast: 0,
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
    vista.sections.push(wallpaper_section(m)); // Wallpapers (imagen + automático, unificado)
    vista.sections.push(greeter_section(&m.greeter)); // Fondo del greeter (pantalla de login)
    vista.sections.push(splash_section(&m.splash)); // Splash del arranque (arje-splash)
    if let Some(s) = take("vista_espacial") {
        vista.sections.push(s); // Vistas: Prezi
    }
    if let Some(s) = take("monitores") {
        vista.sections.push(s); // Vistas: monitores/workspaces
    }
    vista.sections.push(interfaz_section(&m.cfg)); // Animaciones/interfaz/dientes
    if let Some(s) = take("movimiento") {
        vista.sections.push(s); // Movimiento: fade/pop al abrir, glow de foco,
                                // fade al cerrar, slide, atenuar sin foco, reduce-motion
    }
    if let Some(s) = take("efectos") {
        vista.sections.push(s); // Efectos: esquinas redondeadas + glass (blur)
    }
    if let Some(s) = take("terminal") {
        vista.sections.push(s); // Terminal dropdown
    }
    vista.sections.push(reglas_section(&m.rules)); // Reglas (hyprland windowrule)

    // ---- Panel THEMES (su propio diente) ----
    // Biblioteca de themes (tab 1 «Themes» = lista radio + CRUD, tabs siguientes
    // = apariencia/teselado/decoración del theme). Antes vivía dentro de Vista.
    let themes = themes_schema(m);

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
    inicio.sections.push(autologin::section(&m.autologin));
    inicio.sections.push(autostart_section());
    inicio.sections.push(remote::sessions_section(&m.mirada.startup));
    inicio.sections.push(plugins::plugins_section(&m.mirada_plugins));

    // ---- Panel SISTEMA ----
    let mut sistema = Schema::new();
    sistema.sections.push(sonido_section());
    sistema.sections.push(teclado_section(&m.mirada));
    sistema.sections.push(puntero_section(&m.mirada));
    sistema.sections.push(idioma_section(&m.cfg));
    sistema.sections.push(wawa_ai_section(m));
    sistema.sections.push(wawa_voz_section(m));
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
        PanelPestana { title: "Themes".into(), icon: "🎨".into(), schema: themes },
        PanelPestana { title: "Atajos".into(), icon: "⌨".into(), schema: atajos },
        PanelPestana { title: "Animaciones".into(), icon: "✨".into(), schema: animaciones },
        PanelPestana { title: "Pata".into(), icon: "🎛".into(), schema: pata },
        PanelPestana { title: "Inicio".into(), icon: "⏻".into(), schema: inicio },
        PanelPestana { title: "Sistema".into(), icon: "⚙".into(), schema: sistema },
        PanelPestana { title: "Acerca".into(), icon: "🖥".into(), schema: acerca },
        // Diente-de-app: configura las cuentas de correo de paloma.
        PanelPestana { title: "Correo".into(), icon: "✉".into(), schema: paloma::schema(&m.paloma) },
        // Diente-de-app: contextos (pacha) + estado del cifrado de dotfiles.
        PanelPestana { title: "Contextos".into(), icon: "◴".into(), schema: pacha::schema(&m.pacha) },
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

/// Sección «Greeter»: el fondo animado de la pantalla de login (DM). Escribe
/// `greeter.conf`; el greeter lo lee en el próximo arranque. El catálogo de
/// animaciones/paletas sale de [`greeter::ANIMS`] / [`greeter::COLORS`].
fn greeter_section(g: &greeter::GreeterCfg) -> Section {
    use allichay::{EnumOption, Field};
    Section::new("greeter::fondo", "Greeter (login)")
        .icon("🔐")
        .help(
            "El fondo animado de la pantalla de inicio (greeter). En \
             multi-monitor se pinta en todos los monitores y la tarjeta de \
             login viaja al que tiene el ratón. Los cambios entran en el \
             próximo login.",
        )
        .field(Field::toggle("rain", "Fondo animado", g.rain_enabled))
        .field(Field::dropdown(
            "anim",
            "Animación",
            g.anim.clone(),
            greeter::ANIMS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
        .field(Field::dropdown(
            "rain_color",
            "Color",
            g.rain_color.clone(),
            greeter::COLORS.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
        .field(Field::text(
            "lottie",
            "Animación Lottie de fondo (.json) — vacío = procedural",
            g.lottie.clone(),
        ))
        .field(Field::text(
            "rive",
            "Proyecto rive de fondo (.ron del studio) — vacío = sin rive",
            g.rive.clone(),
        ))
}

/// Sección «Arranque»: el splash sin parpadeo (`arje-splash`). Escribe
/// `arje/splash.conf`; el instalador lo hornea en el initramfs/ESP. El catálogo
/// de fuentes/logs sale de [`splash::SOURCES`] / [`splash::LOG_MODES`].
/// Dispara `fondo-bake <kind> <ruta>` en segundo plano para pre-renderizar un
/// Lottie/rive a la cache de frames (la usan el splash y el wallpaper, que no
/// rasterizan vello). Best-effort: si el binario no está, sólo loguea.
fn spawn_fondo_bake(kind: &str, path: &str) {
    match std::process::Command::new("fondo-bake").arg(kind).arg(path).spawn() {
        Ok(_) => eprintln!("wawa-panel: fondo-bake {kind} «{path}» disparado"),
        Err(e) => eprintln!("wawa-panel: no pude lanzar fondo-bake ({e})"),
    }
}

fn splash_section(s: &splash::SplashCfg) -> Section {
    use allichay::{EnumOption, Field};
    let mut sec = Section::new("splash::arranque", "Arranque (splash)")
        .icon("🌅")
        .help(
            "El splash sin parpadeo que se ve desde el encendido hasta el login. \
             Elegí el logo nativo, una imagen PNG o una animación (carpeta de \
             PNG). Los cambios se guardan en ~/.config/arje/splash.conf; corré \
             el instalador (scripts/install-arje-splash.sh) para que el próximo \
             arranque los tome.",
        )
        .field(Field::dropdown(
            "source",
            "Fuente",
            s.source.clone(),
            splash::SOURCES.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ));
    // Mostramos el campo de ruta según la fuente elegida.
    if s.source == "image" {
        sec = sec.field(Field::text("image", "Ruta del PNG", s.image.clone()));
    } else if s.source == "frames" {
        sec = sec.field(Field::text("frames", "Carpeta de PNG (animación)", s.frames.clone()));
    } else if s.source == "lottie" {
        sec = sec.field(Field::text("lottie", "Ruta del Lottie (.json)", s.lottie.clone()));
    } else if s.source == "rive" {
        sec = sec.field(Field::text("rive", "Ruta del rive (.ron del studio)", s.rive.clone()));
    }
    sec.field(Field::text("fps", "Cuadros por segundo", s.fps.to_string()))
        .field(Field::text("bg", "Color de fondo (#rrggbb)", s.bg.clone()))
        .field(Field::text("accent", "Color de acento (#rrggbb)", s.accent.clone()))
        .field(Field::dropdown(
            "logs",
            "Logs de arranque",
            s.logs.clone(),
            splash::LOG_MODES.iter().map(|(id, l)| EnumOption::new(*id, *l)).collect(),
        ))
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
    .section({
        // Composición del perfil activo: qué theme / atajos / animaciones USA,
        // eligiendo por NOMBRE entre las bibliotecas. (También se relacionan al
        // elegirlos en cada diente; acá se ve y arma todo junto.)
        let prof = m.dprofiles.get(&m.dprofiles.active);
        let theme_actual = prof.map(|p| p.theme.clone()).unwrap_or_default();
        let atajos_actual = prof.map(|p| p.keymap_set.clone()).unwrap_or_default();
        let anim_actual = prof.map(|p| p.animation_set.clone()).unwrap_or_default();
        let opt = |names: Vec<String>| -> Vec<EnumOption> {
            names.into_iter().map(|n| EnumOption::new(n.clone(), n)).collect()
        };
        Section::new("perfiles::composicion", "Composición")
            .icon("🧩")
            .help("Qué usa este perfil. Elegí por nombre el theme, los atajos y las animaciones.")
            .field(Field::radio("set_theme", "Theme", theme_actual, opt(m.themes.names())))
            .field(Field::radio("set_atajos", "Atajos", atajos_actual, opt(m.profiles.names())))
            .field(Field::radio("set_anim", "Animaciones", anim_actual, opt(m.animaciones.names())))
    })
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
        "Cada barra en UNA fila: elegí su posición (o «Inactiva» para apagarla). \
         ＋ agrega otra. Grosor/autohide de cada barra están en su pestaña \
         «Superficie N». Se guardan dentro del perfil activo.",
    );
    for (i, s) in pata.surfaces.iter().enumerate() {
        let nombre = if s.name.trim().is_empty() {
            format!("{} {}", kind_slug(s.kind), anchor_slug(s.anchor))
        } else {
            s.name.clone()
        };
        // Una sola fila por barra: el rótulo es su nombre; el control es un
        // select de posición que INCLUYE «Inactiva» (reemplaza al check activo).
        let pos = if !s.enabled { "inactivo".to_string() } else { anchor_slug(s.anchor).to_string() };
        sec = sec.field(Field::dropdown(
            format!("pos_{i}"),
            nombre,
            pos,
            vec![
                EnumOption::new("inactivo", "Inactiva"),
                EnumOption::new("top", "Arriba"),
                EnumOption::new("bottom", "Abajo"),
                EnumOption::new("left", "Izq."),
                EnumOption::new("right", "Der."),
            ],
        ));
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
            // `glass` también es del theme (cristal del look): se edita acá, no
            // en Vista. Encendido en «mirada», apagado en el resto.
            if sec.id == "teselado" || sec.id == "decoracion" || sec.id == "glass" {
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
            Some("duplicar") => {
                let target = value.as_str().map(String::from).unwrap_or_else(|| name.clone());
                if let Some(nuevo) = m.themes.duplicate(&target) {
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
            Some("eliminar") => {
                let target = value.as_str().map(String::from).unwrap_or_else(|| name.clone());
                if m.themes.names().len() > 1 {
                    m.themes.remove(&target);
                    let fallback = m.themes.names().first().cloned().unwrap_or_default();
                    for p in m.dprofiles.profiles.values_mut() {
                        if p.theme == target {
                            p.theme = fallback.clone();
                        }
                    }
                    m.dirty.themes = true;
                    m.dirty.dprofiles = true;
                    apply_active_theme(m);
                    m.status = format!("theme «{target}» eliminado");
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
        "teselado" | "decoracion" | "glass" => {
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
    } else if let Some(i) = idx("pos_") {
        // Select de posición: «inactivo» apaga la barra; un borde la prende y la
        // ancla a ese lado.
        if let (Some(v), Some(s)) = (value.as_str(), m.pata.surfaces.get_mut(i)) {
            if v == "inactivo" {
                s.enabled = false;
            } else {
                s.enabled = true;
                s.anchor = parse_anchor(v);
            }
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
        .section(shuma_atajos_section(m))
}

/// Sección «Terminal (shuma)» del panel Atajos: conmuta el perfil de atajos del
/// workspace de shuma (tabs/tiling/flotantes: nativo `shuma`, `terminal` con los
/// acordes acostumbrados, hyprland/tmux/zellij/vim o propios). Toma efecto en el
/// próximo arranque de shuma.
fn shuma_atajos_section(m: &Model) -> Section {
    use allichay::Field;
    let opts: Vec<EnumOption> = m
        .shuma_atajos_names
        .iter()
        .map(|n| EnumOption::new(n.clone(), n.clone()))
        .collect();
    Section::new("atajos::terminal", "Terminal (shuma)")
        .icon("▭")
        .help(
            "Atajos del workspace de la terminal shuma (tabs, tiling y flotantes). \
             «terminal» trae los acordes acostumbrados de emuladores \
             (Ctrl+Shift+T tab nueva, Ctrl+Tab siguiente, Ctrl+Shift+W cerrar). \
             El cambio toma efecto la próxima vez que abrís shuma.",
        )
        .field(Field::radio(
            "usar",
            "Perfil de atajos de shuma",
            m.shuma_atajos_active.clone(),
            opts,
        ))
}

/// Sección **«IA y semántica»** del panel Sistema (config GLOBAL del SO, en
/// `WawaConfig::ai`): el backend del LLM (instrumento de asistentes como `:?` de
/// shuma o el RAG de paloma) y la búsqueda semántica por embeddings. Una sola
/// fuente de verdad para todas las apps; toma efecto en vivo (las apps leen
/// `wawa.ai`).
fn wawa_ai_section(m: &Model) -> Section {
    use allichay::Field;
    let backends = [
        ("", "Automático (por entorno)"),
        ("anthropic", "Anthropic (Claude)"),
        ("gemini", "Gemini"),
        ("deepseek", "DeepSeek"),
        ("cohere", "Cohere"),
        ("ollama", "Ollama (local)"),
        ("mock", "Mock (sin red)"),
    ];
    let opts: Vec<EnumOption> = backends
        .iter()
        .map(|(v, l)| EnumOption::new((*v).to_string(), (*l).to_string()))
        .collect();
    Section::new("wawa_ai::ia", "IA y semántica")
        .icon("🜲")
        .help(
            "Config GLOBAL del SO: el backend del LLM (asistentes como `:?` de shuma \
             o el RAG de correo) y la búsqueda semántica por significado (`:buscar`). \
             Lo usan todas las apps. La API key, vacía, se lee del entorno \
             (recomendado, no se guarda en claro).",
        )
        .field(Field::radio(
            "backend",
            "Backend del LLM",
            m.cfg.ai.llm.backend.clone(),
            opts,
        ))
        .field(Field::text(
            "model",
            "Modelo (vacío = default del backend)",
            m.cfg.ai.llm.model.clone(),
        ))
        .field(Field::toggle(
            "sem_enabled",
            "Búsqueda semántica por significado",
            m.cfg.ai.semantic.enabled,
        ))
        .field(Field::text(
            "sem_socket",
            "Socket del daemon de embeddings (vacío = por defecto)",
            m.cfg.ai.semantic.socket.clone(),
        ))
}

/// Aplica una edición de la sección «IA y semántica»: muta la config GLOBAL en
/// memoria (`m.cfg.ai`) y marca para persistir (la escribe [`flush_saves`] a la
/// capa de usuario de `WawaConfig`). Toma efecto en vivo (las apps releen `wawa.ai`).
fn apply_wawa_ai(m: &mut Model, field: &str, value: FieldValue) {
    match field {
        "backend" => m.cfg.ai.llm.backend = value.as_str().unwrap_or("").to_string(),
        "model" => m.cfg.ai.llm.model = value.as_str().unwrap_or("").to_string(),
        "sem_enabled" => m.cfg.ai.semantic.enabled = value.as_bool().unwrap_or(false),
        "sem_socket" => m.cfg.ai.semantic.socket = value.as_str().unwrap_or("").to_string(),
        _ => return,
    }
    m.dirty.wawa = true;
    m.save_in = SAVE_DELAY_TICKS;
    m.status = "ajuste de IA/semántica global guardado".into();
}

/// Sección «Voz (manos libres)»: motores de STT/TTS (híbrido mock/local/nube),
/// palabra de llamada y compuerta wake-word. Config GLOBAL del SO en
/// `WawaConfig::ai.voz`; la leen los hosts de voz (shuma, mirada…) para armar
/// `rimay_voz::VozConfig` + `OpcionesEscucha`.
fn wawa_voz_section(m: &Model) -> Section {
    use allichay::Field;
    // Presets del híbrido. El valor crudo es el de `rimay_voz::Backend::parse`.
    let stt_opts: Vec<EnumOption> = [
        ("", "Mock (sin modelo)"),
        ("local", "Local (daemon)"),
        ("nube:openai:whisper-1", "Nube (OpenAI Whisper)"),
    ]
    .iter()
    .map(|(v, l)| EnumOption::new((*v).to_string(), (*l).to_string()))
    .collect();
    let tts_opts: Vec<EnumOption> = [
        ("", "Mock (sin modelo)"),
        ("local", "Local (daemon)"),
        ("nube:openai:tts-1", "Nube (OpenAI)"),
    ]
    .iter()
    .map(|(v, l)| EnumOption::new((*v).to_string(), (*l).to_string()))
    .collect();
    let v = &m.cfg.ai.voz;
    Section::new("wawa_voz::voz", "Voz (manos libres)")
        .icon("🎙")
        .help(
            "Voz del SO: el reconocimiento (dictado) y la síntesis (lectura) — \
             cada uno mock, local (daemon) o nube, por separado. La palabra de \
             llamada despierta la escucha; con el wake-word activo, dormido sólo \
             se transcribe lo que suena al llamado (el resto no llega al STT ni a \
             la nube).",
        )
        .field(Field::radio("stt", "Reconocimiento (STT)", v.stt.clone(), stt_opts))
        .field(Field::radio("tts", "Síntesis (TTS)", v.tts.clone(), tts_opts))
        .field(Field::text(
            "llamado",
            "Palabra de llamada (vacío = «shuma»)",
            v.llamado.clone(),
        ))
        .field(Field::toggle(
            "wake",
            "Wake-word: sólo transcribir tras el llamado",
            v.wake,
        ))
}

/// Aplica una edición de «Voz»: muta `WawaConfig::ai.voz` y marca para persistir.
fn apply_wawa_voz(m: &mut Model, field: &str, value: FieldValue) {
    match field {
        "stt" => m.cfg.ai.voz.stt = value.as_str().unwrap_or("").to_string(),
        "tts" => m.cfg.ai.voz.tts = value.as_str().unwrap_or("").to_string(),
        "llamado" => m.cfg.ai.voz.llamado = value.as_str().unwrap_or("").to_string(),
        "wake" => m.cfg.ai.voz.wake = value.as_bool().unwrap_or(false),
        _ => return,
    }
    m.dirty.wawa = true;
    m.save_in = SAVE_DELAY_TICKS;
    m.status = "ajuste de voz guardado".into();
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
            Some("duplicar") => {
                // El iconito de fila manda el NOMBRE; el botón viejo, Bool → activo.
                let src = value.as_str().map(String::from).unwrap_or_else(|| m.profiles.active().to_string());
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
            Some("eliminar") => {
                let cur = value.as_str().map(String::from).unwrap_or_else(|| m.profiles.active().to_string());
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
        // Sección «Terminal (shuma)»: conmuta el perfil de atajos de shuma y lo
        // persiste directo a su RON (archivo ajeno, sin bandera de sucio nuestra).
        Some("terminal") => {
            if rel.leaf() == Some("usar") {
                if let Some(name) = value.as_str() {
                    match shuma_shortcuts::set_active(name) {
                        Ok(()) => {
                            m.shuma_atajos_active = name.to_string();
                            m.status =
                                format!("atajos de shuma → «{name}» (al reabrir shuma)");
                        }
                        Err(e) => m.status = format!("· no pude guardar atajos de shuma: {e}"),
                    }
                }
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
                        EnumOption::new("cube", "Cubo 3D (estilo Compiz)"),
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
            Some("duplicar") => {
                let src = value.as_str().map(String::from).unwrap_or_else(|| m.animaciones.active().to_string());
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
            Some("eliminar") => {
                let cur = value.as_str().map(String::from).unwrap_or_else(|| m.animaciones.active().to_string());
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

/// Opciones XKB de **cambio de distribución** (`grp:*`) ofrecidas: la tecla que
/// rota entre varias distribuciones. El id se escribe tal cual a `xkb_options`.
const XKB_GRP_TOGGLES: &[(&str, &str)] = &[
    ("", "Sin cambio rápido"),
    ("grp:alt_shift_toggle", "Alt + Shift"),
    ("grp:ctrl_shift_toggle", "Ctrl + Shift"),
    ("grp:win_space_toggle", "Super + Espacio"),
    ("grp:caps_toggle", "Bloq Mayús"),
    ("grp:alts_toggle", "Ambos Alt juntos"),
    ("grp:shifts_toggle", "Ambos Shift juntos"),
    ("grp:lalt_toggle", "Alt izquierdo"),
    ("grp:rwin_toggle", "Super derecho"),
];

/// Teclado: distribución(es) XKB del compositor. REAL: mirada las aplica **en
/// caliente** al guardar (ya no pide reiniciar la sesión). Soporta **varias**
/// distribuciones separadas por coma + una tecla `grp:*` para rotarlas; la barra
/// `pata` pinta la distribución activa. Ruteado a la config de mirada.
fn teclado_section(mir: &mirada_brain::Config) -> Section {
    // Catálogo de distribuciones comunes para la ayuda (código → nombre).
    let lista: String = XKB_LAYOUTS
        .iter()
        .filter(|(id, _)| !id.is_empty())
        .map(|(id, l)| format!("{id}={l}"))
        .collect::<Vec<_>>()
        .join(", ");
    Section::new("mirada::teclado", "Teclado")
        .icon("⌨")
        .help("Distribución(es) de teclado (XKB). Se aplica al guardar, sin reiniciar.")
        .field(
            Field::text("xkb_layout", "Distribución(es)", mir.xkb_layout.clone()).help(format!(
                "una o varias separadas por coma (p. ej. «es» o «us,es,ru»). \
                 Vacío = la del sistema. Comunes: {lista}"
            )),
        )
        .field(
            Field::text("xkb_variant", "Variante(s)", mir.xkb_variant.clone()).help(
                "opcional, una por distribución separada por coma \
                 (p. ej. «,dvorak» = 1.ª normal, 2.ª dvorak). Vacío = ninguna",
            ),
        )
        .field(
            Field::dropdown(
                "xkb_options",
                "Cambiar distribución con",
                mir.xkb_options.clone(),
                XKB_GRP_TOGGLES
                    .iter()
                    .map(|(id, l)| EnumOption::new(*id, *l))
                    .collect(),
            )
            .help("la tecla que rota entre las distribuciones de arriba"),
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
/// Cierra la subventana de sesión remota descartando el borrador.
fn remote_edit_close(m: &mut Model) {
    m.remote_edit = None;
    m.remote_allichay = AllichayState::new();
}

/// Guarda el borrador de la subventana en `mirada.startup` (reemplaza la entrada
/// editada o agrega la nueva) y marca la config sucia — `flush_saves` la persiste
/// a `config.ron` y el compositor la recarga al próximo arranque.
fn remote_edit_save(m: &mut Model) {
    if let Some(edit) = m.remote_edit.take() {
        let remote::RemoteEdit { idx, draft } = edit;
        match idx {
            Some(i) => {
                if let Some(slot) = m.mirada.startup.get_mut(i) {
                    *slot = draft;
                }
            }
            None => m.mirada.startup.push(draft),
        }
        m.dirty.mirada = true;
        m.save_in = SAVE_DELAY_TICKS;
        m.status = "sesión remota guardada".into();
    }
    m.remote_allichay = AllichayState::new();
}

/// Borra del `startup` la sesión que la subventana estaba editando.
fn remote_edit_delete(m: &mut Model) {
    if let Some(edit) = m.remote_edit.take() {
        if let Some(i) = edit.idx {
            if i < m.mirada.startup.len() {
                m.mirada.startup.remove(i);
            }
        }
        m.dirty.mirada = true;
        m.save_in = SAVE_DELAY_TICKS;
        m.status = "sesión remota borrada".into();
    }
    m.remote_allichay = AllichayState::new();
}

/// Cierra la subventana de reglas de plugin descartando el borrador.
fn plugin_edit_close(m: &mut Model) {
    m.plugin_edit = None;
    m.plugin_allichay = AllichayState::new();
}

/// Guarda las reglas del editor en el `.ron` del plugin (reescribe sólo su
/// `config`, fuera de la firma) y relee la lista. El host de plugins lo recarga
/// en caliente — no pasa por `flush_saves`/config.ron, es el archivo del plugin.
fn plugin_edit_save(m: &mut Model) {
    if let Some(edit) = m.plugin_edit.take() {
        match edit.save() {
            Ok(()) => m.status = format!("reglas de «{}» guardadas (recarga en caliente)", edit.name),
            Err(e) => m.status = format!("no se pudo guardar {}: {e}", edit.name),
        }
        m.mirada_plugins = plugins::list_plugins();
    }
    m.plugin_allichay = AllichayState::new();
}

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
                // El iconito de fila manda el NOMBRE; el botón viejo, Bool → activo.
                Some("duplicar") => {
                    let src = value.as_str().map(String::from).unwrap_or_else(|| m.dprofiles.active.clone());
                    if let Some(name) = m.dprofiles.duplicate(&src) {
                        activate_profile(m, &name);
                        m.status = format!("perfil «{name}» (copia de «{src}»)");
                    }
                }
                Some("eliminar") => {
                    let cur = value.as_str().map(String::from).unwrap_or_else(|| m.dprofiles.active.clone());
                    if m.dprofiles.profiles.len() <= 1 {
                        m.status = "no se puede eliminar el último perfil".into();
                    } else {
                        m.dprofiles.remove(&cur);
                        let next = m.dprofiles.active.clone();
                        if !next.is_empty() {
                            activate_profile(m, &next);
                        }
                        m.status = format!("perfil «{cur}» eliminado");
                    }
                }
                Some("renombrar") => {
                    if let Some(to) = value.as_str() {
                        do_rename_profile(m, to);
                    }
                }
                Some("rescatar") if value.as_bool() == Some(true) => do_rescue_profiles(m),
                // Composición: el perfil activo elige qué theme/atajos/animación
                // USA (referencia por nombre) y se aplica en caliente.
                Some("set_theme") => {
                    if let Some(sel) = value.as_str() {
                        let active = m.dprofiles.active.clone();
                        if let Some(p) = m.dprofiles.profiles.get_mut(&active) {
                            p.theme = sel.to_string();
                        }
                        m.dirty.dprofiles = true;
                        apply_active_theme(m);
                    }
                }
                Some("set_atajos") => {
                    if let Some(sel) = value.as_str() {
                        let active = m.dprofiles.active.clone();
                        if let Some(p) = m.dprofiles.profiles.get_mut(&active) {
                            p.keymap_set = sel.to_string();
                        }
                        if m.profiles.set_active(sel).is_ok() {
                            m.keymap_rows = m.profiles.active_keymap().to_rows();
                            m.dirty.keymap = true;
                        }
                        m.dirty.profiles = true;
                        m.dirty.dprofiles = true;
                    }
                }
                Some("set_anim") => {
                    if let Some(sel) = value.as_str() {
                        let active = m.dprofiles.active.clone();
                        if let Some(p) = m.dprofiles.profiles.get_mut(&active) {
                            p.animation_set = sel.to_string();
                        }
                        if m.animaciones.set_active(sel) {
                            m.animaciones.active_animation().apply_to(&mut m.mirada);
                            m.dirty.mirada = true;
                        }
                        m.dirty.animaciones = true;
                        m.dirty.dprofiles = true;
                    }
                }
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
        // IA + semántica GLOBAL → WawaConfig::ai (capa de usuario).
        "wawa_ai" => {
            apply_wawa_ai(m, rel.leaf().unwrap_or(""), value);
            return;
        }
        // Voz GLOBAL → WawaConfig::ai.voz (capa de usuario).
        "wawa_voz" => {
            apply_wawa_voz(m, rel.leaf().unwrap_or(""), value);
            return;
        }
        // Greeter (DM): fondo animado + paleta → greeter.conf (próximo login).
        "greeter" => {
            match rel.leaf() {
                Some("rain") => m.greeter.rain_enabled = value.as_bool().unwrap_or(false),
                Some("anim") => {
                    if let Some(v) = value.as_str() {
                        m.greeter.anim = v.to_string();
                        m.greeter.rain_enabled = true; // elegir animación la enciende
                        // Paleta por defecto para fondos con tono propio (fuego
                        // en verde parece pasto; plasma luce en cian).
                        match v {
                            "fire" => m.greeter.rain_color = "amber".into(),
                            "plasma" => m.greeter.rain_color = "cyan".into(),
                            "aurora" => m.greeter.rain_color = "green".into(),
                            "lightning" => m.greeter.rain_color = "cyan".into(),
                            _ => {}
                        }
                    }
                }
                Some("rain_color") => {
                    if let Some(v) = value.as_str() {
                        m.greeter.rain_color = v.to_string();
                    }
                }
                Some("lottie") => {
                    if let Some(v) = value.as_str() {
                        m.greeter.lottie = v.to_string();
                    }
                }
                Some("rive") => {
                    if let Some(v) = value.as_str() {
                        m.greeter.rive = v.to_string();
                    }
                }
                _ => {}
            }
            m.dirty.greeter = true;
            m.save_in = SAVE_DELAY_TICKS;
            return;
        }
        // Splash del arranque (arje-splash): fuente + colores + logs →
        // arje/splash.conf (lo hornea el instalador en el próximo build).
        "splash" => {
            match rel.leaf() {
                Some("source") => if let Some(v) = value.as_str() { m.splash.source = v.to_string() },
                Some("image") => if let Some(v) = value.as_str() {
                    m.splash.image = v.to_string();
                    if !v.is_empty() { m.splash.source = "image".into(); }
                },
                Some("frames") => if let Some(v) = value.as_str() {
                    m.splash.frames = v.to_string();
                    if !v.is_empty() { m.splash.source = "frames".into(); }
                },
                Some("lottie") => if let Some(v) = value.as_str() {
                    m.splash.lottie = v.to_string();
                    if !v.is_empty() { m.splash.source = "lottie".into(); }
                },
                Some("rive") => if let Some(v) = value.as_str() {
                    m.splash.rive = v.to_string();
                    if !v.is_empty() { m.splash.source = "rive".into(); }
                },
                Some("fps") => if let Some(v) = value.as_str() {
                    if let Ok(n) = v.trim().parse() { m.splash.fps = n; }
                },
                Some("bg") => if let Some(v) = value.as_str() { m.splash.bg = v.to_string() },
                Some("accent") => if let Some(v) = value.as_str() { m.splash.accent = v.to_string() },
                Some("logs") => if let Some(v) = value.as_str() { m.splash.logs = v.to_string() },
                _ => {}
            }
            m.dirty.splash = true;
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
        // Sesiones remotas (waypipe): los botones abren la subventana de edición.
        // `nueva` crea un borrador; `editar:N` carga la N-ésima sesión.
        "remote" => {
            if value.as_bool() != Some(true) {
                return;
            }
            match rel.leaf() {
                Some("nueva") => {
                    m.remote_edit = Some(remote::RemoteEdit::nueva());
                    m.remote_allichay = AllichayState::new();
                }
                Some(l) => {
                    if let Some(n) = l.strip_prefix("editar:").and_then(|s| s.parse::<usize>().ok()) {
                        if let Some(app) = m.mirada.startup.get(n) {
                            m.remote_edit = Some(remote::RemoteEdit::editar(n, app));
                            m.remote_allichay = AllichayState::new();
                        }
                    }
                }
                None => {}
            }
            return;
        }
        // Plugins de mirada: `plugin:N` abre el editor de reglas del N-ésimo
        // (sólo los editables — el asignador). Los `info:N` son display, no llegan.
        "plugins" => {
            if value.as_bool() != Some(true) {
                return;
            }
            if let Some(n) = rel
                .leaf()
                .and_then(|l| l.strip_prefix("plugin:"))
                .and_then(|s| s.parse::<usize>().ok())
            {
                if let Some(info) = m.mirada_plugins.get(n) {
                    if info.editable() {
                        m.plugin_edit = Some(plugins::PluginEdit::open(info));
                        m.plugin_allichay = AllichayState::new();
                    }
                }
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
        // Diente «Correo»: edita las cuentas de paloma (`cuentas.json`).
        "paloma" => {
            let action = paloma::route(&mut m.paloma, &rel, value);
            if action.dirty {
                m.dirty.paloma = true;
                m.save_in = SAVE_DELAY_TICKS;
            }
            if !action.status.is_empty() {
                m.status = action.status;
            }
            return;
        }
        "pacha" => {
            let action = pacha::route(&mut m.pacha, &rel, value);
            if action.dirty {
                m.dirty.pacha = true;
                m.save_in = SAVE_DELAY_TICKS;
            }
            if !action.status.is_empty() {
                m.status = action.status;
            }
            return;
        }
        "autologin" => {
            let action = autologin::route(&mut m.autologin, &rel, value);
            if action.dirty {
                m.dirty.autologin = true;
                m.save_in = SAVE_DELAY_TICKS;
            }
            if !action.status.is_empty() {
                m.status = action.status;
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
            Some(Ok(())) => {
                ok = true;
                // Wallpaper Lottie/rive: pre-bakeamos el asset a frames (el
                // compositor no rasteriza vello; reproduce la cache). El compositor
                // igual cae a la chakana mientras el baker trabaja.
                let wp = m.mirada.wallpaper_path.trim();
                if !wp.is_empty() {
                    match m.mirada.wallpaper_source.as_str() {
                        "lottie" => spawn_fondo_bake("lottie", wp),
                        "rive" => spawn_fondo_bake("rive", wp),
                        _ => {}
                    }
                }
            }
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
    if m.dirty.greeter {
        match m.greeter.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· greeter save: {e}")),
        }
        m.dirty.greeter = false;
    }
    if m.dirty.paloma {
        match m.paloma.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· paloma save: {e}")),
        }
        m.dirty.paloma = false;
    }
    if m.dirty.pacha {
        match m.pacha.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· pacha save: {e}")),
        }
        m.dirty.pacha = false;
    }
    if m.dirty.autologin {
        match m.autologin.save() {
            Ok(()) => ok = true,
            Err(e) => err = Some(format!("· autologin save: {e}")),
        }
        m.dirty.autologin = false;
    }
    if m.dirty.splash {
        match m.splash.save() {
            Ok(()) => {
                ok = true;
                // El splash no tiene GPU al boot: hay que pre-bakear el Lottie/rive
                // a frames para que pueda blitearlos. Lo disparamos en segundo plano.
                match m.splash.source.as_str() {
                    "lottie" if !m.splash.lottie.is_empty() => {
                        spawn_fondo_bake("lottie", &m.splash.lottie)
                    }
                    "rive" if !m.splash.rive.is_empty() => spawn_fondo_bake("rive", &m.splash.rive),
                    _ => {}
                }
            }
            Err(e) => err = Some(format!("· splash save: {e}")),
        }
        m.dirty.splash = false;
    }
    if let Some(e) = err {
        m.status = e.clone();
        let id = m.next_toast;
        m.next_toast += 1;
        m.toasts.push(Toast::error(id, format!("No se pudo guardar {e}"), TOAST_TTL));
    } else if ok {
        let msg = rimay_localize::t("wawa-panel-autosave-ok");
        m.status = msg.clone();
        let id = m.next_toast;
        m.next_toast += 1;
        m.toasts.push(Toast::success(id, msg, TOAST_TTL));
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
    if key == "wawa_ai" {
        return match rel.leaf() {
            Some("model") => m.cfg.ai.llm.model.clone(),
            Some("sem_socket") => m.cfg.ai.semantic.socket.clone(),
            _ => String::new(),
        };
    }
    if key == "wawa_voz" {
        return match rel.leaf() {
            Some("llamado") => m.cfg.ai.voz.llamado.clone(),
            _ => String::new(),
        };
    }
    if key == "paloma" {
        return paloma::text_value(&m.paloma, &rel).unwrap_or_default();
    }
    if key == "pacha" {
        return pacha::text_value(&m.pacha, &rel).unwrap_or_default();
    }
    if key == "autologin" {
        return autologin::text_value(&m.autologin, &rel).unwrap_or_default();
    }
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
            // Sección «Terminal (shuma)»: el selector «usar» = perfil shuma activo.
            Some("terminal") => FieldValue::Text(m.shuma_atajos_active.clone()),
            _ => FieldValue::Table(m.keymap_rows.clone()),
        });
    }
    // Sección «IA y semántica (shuma)»: valores vivos del shumarc.
    if key == "wawa_ai" {
        return Some(match rel.leaf() {
            Some("backend") => FieldValue::Text(m.cfg.ai.llm.backend.clone()),
            Some("model") => FieldValue::Text(m.cfg.ai.llm.model.clone()),
            Some("sem_enabled") => FieldValue::Bool(m.cfg.ai.semantic.enabled),
            Some("sem_socket") => FieldValue::Text(m.cfg.ai.semantic.socket.clone()),
            _ => FieldValue::Text(String::new()),
        });
    }
    // Sección «Voz»: valores vivos de WawaConfig::ai.voz.
    if key == "wawa_voz" {
        return Some(match rel.leaf() {
            Some("stt") => FieldValue::Text(m.cfg.ai.voz.stt.clone()),
            Some("tts") => FieldValue::Text(m.cfg.ai.voz.tts.clone()),
            Some("llamado") => FieldValue::Text(m.cfg.ai.voz.llamado.clone()),
            Some("wake") => FieldValue::Bool(m.cfg.ai.voz.wake),
            _ => FieldValue::Text(String::new()),
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
        let prof = m.dprofiles.get(&m.dprofiles.active);
        return Some(match rel.leaf() {
            Some("usar") => FieldValue::Text(m.dprofiles.active.clone()),
            Some("renombrar") => FieldValue::Text(String::new()),
            // Composición: referencias del perfil activo (por nombre).
            Some("set_theme") => FieldValue::Enum(prof.map(|p| p.theme.clone()).unwrap_or_default()),
            Some("set_atajos") => {
                FieldValue::Enum(prof.map(|p| p.keymap_set.clone()).unwrap_or_default())
            }
            Some("set_anim") => {
                FieldValue::Enum(prof.map(|p| p.animation_set.clone()).unwrap_or_default())
            }
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
        if let Some(i) = idx("pos_") {
            let v = m
                .pata
                .surfaces
                .get(i)
                .map(|s| if !s.enabled { "inactivo".to_string() } else { anchor_slug(s.anchor).to_string() })
                .unwrap_or_else(|| "inactivo".to_string());
            return Some(FieldValue::Enum(v));
        }
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
    app_header_iconed(AppIcon::Wawa, rimay_localize::t("wawa-panel-title"), vec![], &palette)
}

/// Editor de **recorrido** del Prezi (la vista espacial): un lienzo libre tipo
/// Prezi con un marco por escritorio. Arrastrar un marco lo mueve; arrastrar el
/// vacío panea; la rueda hace zoom-a-cursor; `[` / `]` o la **manija ⟳** rotan
/// el marco elegido. Cada edición se vuelca a `overview_places` del perfil activo
/// (posición libre + giro), que la vista espacial respeta. Reemplaza la grilla
/// col/fila por el mismo lienzo que `recorrido_editor_demo`.
fn prezi_editor_view(model: &Model, theme: &Theme) -> View<Msg> {
    /// Lado de la manija de giro, px.
    const HANDLE: f32 = 16.0;
    /// Alto del lienzo del editor, px. El bloque entero (título + lienzo +
    /// padding) debe caber en [`PREZI_EDITOR_H`] para no comerse los campos.
    const CANVAS_H: f32 = PREZI_EDITOR_H - 60.0;

    // Lienzo: el render del recorrido (free canvas + rotación) con el arrastre
    // cableado al `update` del panel (move = mover marco / panear; end = soltar).
    let lienzo = recorrido_view_editor(&model.prezi.rec, &model.prezi.state, model.prezi.sel)
        .draggable_at(|phase, dx, dy, lx, ly| match phase {
            DragPhase::Move => Some(Msg::PreziDrag { dx, dy, lx, ly }),
            DragPhase::End => Some(Msg::PreziDragEnd),
        });

    let mut canvas_kids: Vec<View<Msg>> = vec![lienzo];

    // Manija de giro «sobre el marco»: anclada al borde superior (girado) del
    // marco elegido, en coordenadas del contenedor (= rect del último paint).
    if let Some(handle) = prezi_rotate_handle(model, theme, HANDLE) {
        canvas_kids.push(handle);
    }

    let canvas = View::new(Style {
        position: Position::Relative,
        size: Size { width: percent(1.0_f32), height: length(CANVAS_H) },
        ..Default::default()
    })
    .radius(8.0)
    .children(canvas_kids);

    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "Plano Prezi · arrastrá un escritorio para moverlo · rueda: zoom · manija ⟳ o [ ] : rotar"
            .to_string(),
        12.5,
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

/// La **manija de giro** del marco seleccionado: un disco ámbar pegado al borde
/// superior (girado) del marco, arrastrable horizontalmente para rotar (scrub).
/// `None` si no hay marco elegido o todavía no se pintó el lienzo (no hay rect).
fn prezi_rotate_handle(model: &Model, theme: &Theme, side: f32) -> Option<View<Msg>> {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let id = model.prezi.sel?;
    let panel = panel_actual()?;
    let m = model.prezi.rec.marco(id)?;
    let (cx, cy) = m.rect.centro();
    let hh = m.rect.h * 0.5;
    let (s, c) = m.rot_rad.sin_cos();
    // Centro del borde superior del marco, girado alrededor de su centro.
    let anchor = (cx + hh * s, cy - hh * c);
    let (sx, sy) = model.prezi.state.camara.world_to_screen(anchor, panel);
    // A coordenadas del contenedor (origen = panel.x/panel.y), un pelín afuera,
    // y acotado al lienzo para que no se escape si el marco quedó fuera de vista.
    let hx = ((sx - panel.x) as f32 - side * 0.5).clamp(0.0, (panel.w as f32 - side).max(0.0));
    let hy = ((sy - panel.y) as f32 - side * 0.5 - 14.0)
        .clamp(0.0, (panel.h as f32 - side).max(0.0));
    let ambar = Color::from_rgba8(245, 180, 50, 255);
    Some(
        View::new(Style {
            position: Position::Absolute,
            inset: Rect { left: length(hx), top: length(hy), right: auto(), bottom: auto() },
            size: Size { width: length(side), height: length(side) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(ambar)
        .radius((side * 0.5) as f64)
        .text_aligned("⟳".to_string(), 11.0, theme.bg_panel, Alignment::Center)
        .draggable(|phase, dx, _dy| match phase {
            DragPhase::Move => Some(Msg::PreziRotateHandle(dx)),
            DragPhase::End => Some(Msg::PreziDragEnd),
        }),
    )
}

/// Editor visual de **disposición de monitores**: el lienzo del recorrido
/// (mismo look que el Prezi) con las cajas-monitor arrastrables, SIN manija de
/// giro ni rotación. Se inyecta arriba de los campos de la sección «Monitores».
fn monitor_editor_view(model: &Model, theme: &Theme) -> View<Msg> {
    /// Alto del lienzo del editor, px. Mismo criterio que el Prezi.
    const CANVAS_H: f32 = MONITOR_EDITOR_H - 60.0;

    // Lienzo: el render del recorrido (free canvas, sin giro porque ningún marco
    // tiene `rot_rad` ni hay manija) con el arrastre cableado al `update`.
    let lienzo =
        recorrido_view_editor(&model.monitor.rec, &model.monitor.state, model.monitor.sel)
            .draggable_at(|phase, dx, dy, lx, ly| match phase {
                DragPhase::Move => Some(Msg::MonitorDrag { dx, dy, lx, ly }),
                DragPhase::End => Some(Msg::MonitorDragEnd),
            });

    let canvas = View::new(Style {
        position: Position::Relative,
        size: Size { width: percent(1.0_f32), height: length(CANVAS_H) },
        ..Default::default()
    })
    .radius(8.0)
    .children(vec![lienzo]);

    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        "Disposición · arrastrá un monitor para reubicarlo · rueda: zoom · el orden y la dirección se derivan de la posición"
            .to_string(),
        12.5,
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

/// Vista custom de la **lista de barras**: una fila por barra con [nombre]
/// [select de posición segmentado] [⧉ duplicar] [✕ borrar], y «＋ Agregar» al
/// final. Inyectada en el canvas (allichay no compone varios controles por
/// fila). Emite los mismos `Change` que la sección `barras::lista`
/// (`pos_{i}`/`dup_{i}`/`del_{i}`/`agregar`) → los maneja `apply_barras_list`.
fn barras_editor_view(model: &Model, theme: &Theme) -> View<Msg> {
    let cambio = |leaf: String, value: FieldValue| -> Msg {
        Msg::Allichay(AllichayMsg::Change(
            FieldPath(vec!["barras::lista".into(), leaf]),
            value,
        ))
    };
    const POS: &[(&str, &str)] = &[
        ("inactivo", "Inactiva"),
        ("top", "Arriba"),
        ("bottom", "Abajo"),
        ("left", "Izq."),
        ("right", "Der."),
    ];

    let mut filas: Vec<View<Msg>> = Vec::new();
    for (i, s) in model.pata.surfaces.iter().enumerate() {
        let nombre = if s.name.trim().is_empty() {
            format!("{} {}", kind_slug(s.kind), anchor_slug(s.anchor))
        } else {
            s.name.clone()
        };
        let cur = if !s.enabled { "inactivo" } else { anchor_slug(s.anchor) };

        // Rótulo (nombre de la barra).
        let label = View::new(Style {
            size: Size { width: length(150.0_f32), height: percent(1.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(nombre, 13.0, theme.fg_text, Alignment::Start)
        .ellipsis(1);

        // Chips de posición (segmentado).
        let chips: Vec<View<Msg>> = POS
            .iter()
            .map(|(slug, txt)| {
                let sel = *slug == cur;
                let (fill, fg) = if sel { (theme.accent, theme.bg_panel) } else { (theme.bg_panel_alt, theme.fg_muted) };
                View::new(Style {
                    flex_grow: 1.0,
                    size: Size { width: percent(0.0_f32), height: length(26.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                })
                .fill(fill)
                .hover_fill(theme.bg_button_hover)
                .on_click(cambio(format!("pos_{i}"), FieldValue::Enum((*slug).to_string())))
                .text_aligned((*txt).to_string(), 12.0, fg, Alignment::Center)
            })
            .collect();
        let segmento = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size { width: percent(0.0_f32), height: length(26.0_f32) },
            gap: Size { width: length(2.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .radius(5.0)
        .children(chips);

        // Iconitos de duplicar / borrar al lado del segmento.
        let icono = |glifo: &str, tip: &str, msg: Msg, col| {
            View::new(Style {
                size: Size { width: length(28.0_f32), height: length(26.0_f32) },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .radius(5.0)
            .fill(theme.bg_panel_alt)
            .hover_fill(theme.bg_button_hover)
            .tooltip(tip.to_string())
            .on_click(msg)
            .text_aligned(glifo.to_string(), 14.0, col, Alignment::Center)
        };
        let dup = icono("⧉", "Duplicar", cambio(format!("dup_{i}"), FieldValue::Bool(true)), theme.fg_muted);
        let del = icono("✕", "Borrar", cambio(format!("del_{i}"), FieldValue::Bool(true)), theme.accent);

        let fila = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![label, segmento, dup, del]);
        filas.push(fila);
    }

    // ＋ Agregar barra.
    let agregar = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(6.0)
    .fill(theme.bg_panel_alt)
    .hover_fill(theme.bg_button_hover)
    .on_click(cambio("agregar".into(), FieldValue::Bool(true)))
    .text_aligned("＋ Agregar barra".to_string(), 13.0, theme.fg_text, Alignment::Center);
    filas.push(agregar);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(filas)
}

/// Compositor de la **fila de widgets** de una barra/dock: tres slots
/// (Inicio/Centro/Fin), cada uno con sus widgets (chip [icono nombre ✕]) y una
/// **paleta** del catálogo (filtrado por el tipo de superficie) para agregar.
/// `surf` es el índice de la superficie en `m.pata.surfaces`.
fn bar_widgets_view(surf: usize, s: &pata_core::Surface, theme: &Theme) -> View<Msg> {
    let palette = pata_core::widgets_for_surface(s.kind);
    let label_de = |kind: &str| -> (String, String) {
        palette
            .iter()
            .find(|w| w.kind == kind)
            .map(|w| (w.icon.to_string(), w.label.to_string()))
            .unwrap_or_else(|| ("▫".to_string(), kind.to_string()))
    };

    let slot_view = |slot_idx: u8, titulo: &str, widgets: &[pata_core::WidgetSpec]| -> View<Msg> {
        // Chips de los widgets actuales del slot.
        let mut chips: Vec<View<Msg>> = widgets
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let (icon, label) = label_de(&w.kind);
                let x = View::new(Style {
                    size: Size { width: length(18.0_f32), height: length(22.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                })
                .hover_fill(theme.bg_button_hover)
                .radius(4.0)
                .tooltip("Quitar".to_string())
                .on_click(Msg::BarWidgetRemove(surf, slot_idx, i))
                .text_aligned("✕".to_string(), 12.0, theme.accent, Alignment::Center);
                View::new(Style {
                    flex_direction: FlexDirection::Row,
                    align_items: Some(AlignItems::Center),
                    gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
                    padding: Rect { left: length(8.0_f32), right: length(4.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .radius(5.0)
                .tooltip(label.clone())
                .children(vec![
                    View::new(Style { size: Size { width: length(16.0_f32), height: length(16.0_f32) }, flex_shrink: 0.0, ..Default::default() })
                        .children(vec![llimphi_icons::glyph_or_text_view(&icon, 12.0, theme.fg_text, 1.7)]),
                    View::new(Style { size: Size { width: auto(), height: length(24.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
                        .text_aligned(label.clone(), 12.0, theme.fg_text, Alignment::Start),
                    x,
                ])
            })
            .collect();
        if chips.is_empty() {
            chips.push(
                View::new(Style { size: Size { width: auto(), height: length(24.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
                    .text_aligned("(vacío)".to_string(), 12.0, theme.fg_muted, Alignment::Start),
            );
        }
        let fila_widgets = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
            size: Size { width: percent(1.0_f32), height: auto() },
            gap: Size { width: length(6.0_f32), height: length(6.0_f32) },
            ..Default::default()
        })
        .children(chips);

        // Paleta: un iconito por widget del catálogo → agrega a este slot.
        let pal: Vec<View<Msg>> = palette
            .iter()
            .map(|w| {
                let kind = w.kind.to_string();
                View::new(Style {
                    size: Size { width: length(28.0_f32), height: length(26.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::Center),
                    ..Default::default()
                })
                .radius(5.0)
                .fill(theme.bg_panel)
                .hover_fill(theme.bg_button_hover)
                .border(1.0, theme.border)
                .tooltip(format!("Agregar {}", w.label))
                .on_click(Msg::BarWidgetAdd(surf, slot_idx, kind))
                .children(vec![View::new(Style {
                    size: Size { width: length(16.0_f32), height: length(16.0_f32) },
                    ..Default::default()
                })
                .children(vec![llimphi_icons::glyph_or_text_view(w.icon, 14.0, theme.fg_muted, 1.7)])])
            })
            .collect();
        let paleta = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
            size: Size { width: percent(1.0_f32), height: auto() },
            gap: Size { width: length(4.0_f32), height: length(4.0_f32) },
            ..Default::default()
        })
        .children(pal);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: auto() },
            gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style { size: Size { width: auto(), height: length(20.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
                .text_aligned(titulo.to_string(), 11.0, theme.fg_muted, Alignment::Start),
            fila_widgets,
            View::new(Style { size: Size { width: auto(), height: length(16.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
                .text_aligned("agregar ↓".to_string(), 10.0, theme.fg_muted, Alignment::Start),
            paleta,
        ])
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(14.0_f32) },
        padding: Rect { left: length(16.0_f32), right: length(16.0_f32), top: length(12.0_f32), bottom: length(8.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { size: Size { width: auto(), height: length(20.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned("Widgets de la barra".to_string(), 13.0, theme.fg_text, Alignment::Start),
        slot_view(0, "Inicio", &s.start),
        slot_view(1, "Centro", &s.center),
        slot_view(2, "Fin", &s.end),
    ])
}

/// Compositor de los **dientes** de un sidebar: cada diente (icono + rótulo +
/// contenido) como una fila con ✕, y una **paleta** de widgets `on_sidebar` del
/// catálogo para agregar un diente nuevo. Si se activan varios módulos sidebar,
/// todos sus dientes viven en este mismo rail.
fn sidebar_dientes_view(surf: usize, s: &pata_core::Surface, theme: &Theme) -> View<Msg> {
    // Filas de los dientes actuales.
    let mut filas: Vec<View<Msg>> = s
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let icon = if t.icon.chars().count() <= 2 { t.icon.clone() } else { "❖".to_string() };
            let label = if t.label.trim().is_empty() { t.content.kind.clone() } else { t.label.clone() };
            // Ícono del diente como vector (glifo→vector, determinista).
            let icono = View::new(Style {
                size: Size { width: length(16.0_f32), height: length(16.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![llimphi_icons::glyph_or_text_view(&icon, 13.0, theme.fg_text, 1.7)]);
            let nombre = View::new(Style {
                flex_grow: 1.0,
                size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(format!("{label}   ·  {}", t.content.kind), 13.0, theme.fg_text, Alignment::Start)
            .ellipsis(1);
            let del = View::new(Style {
                size: Size { width: length(28.0_f32), height: length(28.0_f32) },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .radius(5.0)
            .fill(theme.bg_panel_alt)
            .hover_fill(theme.bg_button_hover)
            .tooltip("Quitar diente".to_string())
            .on_click(Msg::SidebarTabRemove(surf, i))
            .text_aligned("✕".to_string(), 14.0, theme.accent, Alignment::Center);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
                padding: Rect { left: length(8.0_f32), right: length(4.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                ..Default::default()
            })
            .fill(theme.bg_panel)
            .radius(5.0)
            .children(vec![icono, nombre, del])
        })
        .collect();
    if filas.is_empty() {
        filas.push(
            View::new(Style { size: Size { width: auto(), height: length(24.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
                .text_aligned("(sin dientes — agregá uno abajo)".to_string(), 12.0, theme.fg_muted, Alignment::Start),
        );
    }

    // Paleta: widgets que pueden ser contenido de un diente.
    let pal: Vec<View<Msg>> = pata_core::widgets_for_surface(pata_core::SurfaceKind::Sidebar)
        .into_iter()
        .map(|w| {
            let kind = w.kind.to_string();
            View::new(Style {
                flex_direction: FlexDirection::Row,
                align_items: Some(AlignItems::Center),
                gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
                size: Size { width: auto(), height: length(28.0_f32) },
                padding: Rect { left: length(8.0_f32), right: length(8.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
                ..Default::default()
            })
            .radius(5.0)
            .fill(theme.bg_panel)
            .hover_fill(theme.bg_button_hover)
            .border(1.0, theme.border)
            .tooltip(format!("Agregar diente: {}", w.label))
            .on_click(Msg::SidebarTabAdd(surf, kind))
            .text_aligned(format!("＋ {} {}", w.icon, w.label), 12.0, theme.fg_muted, Alignment::Start)
        })
        .collect();
    let paleta = View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(6.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children(pal);

    let lista = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: auto() },
        gap: Size { width: length(4.0_f32), height: length(4.0_f32) },
        ..Default::default()
    })
    .children(filas);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        padding: Rect { left: length(16.0_f32), right: length(16.0_f32), top: length(12.0_f32), bottom: length(8.0_f32) },
        ..Default::default()
    })
    .children(vec![
        View::new(Style { size: Size { width: auto(), height: length(20.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned("Dientes del sidebar".to_string(), 13.0, theme.fg_text, Alignment::Start),
        lista,
        View::new(Style { size: Size { width: auto(), height: length(16.0_f32) }, align_items: Some(AlignItems::Center), ..Default::default() })
            .text_aligned("agregar diente ↓".to_string(), 10.0, theme.fg_muted, Alignment::Start),
        paleta,
    ])
}

/// Vista custom de una **biblioteca de sets** (perfiles/themes/atajos/animaciones):
/// una fila por item con [○ radio + nombre] [⧉ duplicar] [✕ borrar], y abajo
/// SÓLO «＋ Crear». `section_id` es el id de la sección-lista (`"atajos::conjuntos"`,
/// `"theme::acciones"`, …); las hojas de ruteo son uniformes (`usar`/`duplicar`/
/// `eliminar`/`crear`) y las maneja el `apply` de cada destino (dup/del aceptan
/// el NOMBRE del item por valor `Enum`).
fn lista_set_view(
    section_id: &str,
    items: &[String],
    active: &str,
    crear_label: &str,
    theme: &Theme,
) -> View<Msg> {
    let sid = section_id.to_string();
    let mk = move |leaf: &str, value: FieldValue| -> Msg {
        Msg::Allichay(AllichayMsg::Change(FieldPath(vec![sid.clone(), leaf.into()]), value))
    };

    let mut filas: Vec<View<Msg>> = Vec::new();
    for name in items {
        let sel = name == active;
        // ○ radio.
        let dot_inner = if sel {
            vec![View::new(Style {
                size: Size { width: length(8.0_f32), height: length(8.0_f32) },
                ..Default::default()
            })
            .radius(4.0)
            .fill(theme.accent)]
        } else {
            Vec::new()
        };
        let dot = View::new(Style {
            size: Size { width: length(16.0_f32), height: length(16.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(8.0)
        .border(1.5, if sel { theme.accent } else { theme.border })
        .children(dot_inner);
        let nombre = View::new(Style {
            flex_grow: 1.0,
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(name.clone(), 13.0, if sel { theme.fg_text } else { theme.fg_muted }, Alignment::Start)
        .ellipsis(1);
        // [radio + nombre] clickeable = seleccionar (usar).
        let seleccionable = View::new(Style {
            flex_direction: FlexDirection::Row,
            flex_grow: 1.0,
            size: Size { width: percent(0.0_f32), height: length(28.0_f32) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            padding: Rect { left: length(6.0_f32), right: length(6.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
            ..Default::default()
        })
        .radius(5.0)
        .fill(if sel { theme.bg_panel_alt } else { theme.bg_panel })
        .hover_fill(theme.bg_button_hover)
        .on_click(mk("usar", FieldValue::Enum(name.clone())))
        .children(vec![dot, nombre]);

        let icono = |glifo: &str, tip: &str, msg: Msg, col| {
            View::new(Style {
                size: Size { width: length(28.0_f32), height: length(28.0_f32) },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .radius(5.0)
            .fill(theme.bg_panel_alt)
            .hover_fill(theme.bg_button_hover)
            .tooltip(tip.to_string())
            .on_click(msg)
            .text_aligned(glifo.to_string(), 14.0, col, Alignment::Center)
        };
        let dup = icono("⧉", "Duplicar", mk("duplicar", FieldValue::Enum(name.clone())), theme.fg_muted);
        let del = icono("✕", "Borrar", mk("eliminar", FieldValue::Enum(name.clone())), theme.accent);

        let fila = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
            align_items: Some(AlignItems::Center),
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![seleccionable, dup, del]);
        filas.push(fila);
    }

    // ＋ Crear (único botón abajo).
    let crear = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(6.0)
    .fill(theme.bg_panel_alt)
    .hover_fill(theme.bg_button_hover)
    .on_click(mk("crear", FieldValue::Bool(true)))
    .text_aligned(format!("＋ {crear_label}"), 13.0, theme.fg_text, Alignment::Center);
    filas.push(crear);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(filas)
}

/// Si `section_id` es una sección-lista de biblioteca de sets, devuelve
/// `(nombres, activo, rótulo-crear)` para [`lista_set_view`]. `None` si no lo es.
fn set_list_data(m: &Model, section_id: &str) -> Option<(Vec<String>, String, &'static str)> {
    match section_id {
        "perfiles::acciones" => {
            Some((m.dprofiles.names(), m.dprofiles.active.clone(), "Crear perfil"))
        }
        "theme::acciones" => Some((m.themes.names(), active_theme_name(m), "Crear theme")),
        "atajos::conjuntos" => {
            Some((m.profiles.names(), m.profiles.active().to_string(), "Crear conjunto"))
        }
        "animaciones::conjuntos" => {
            Some((m.animaciones.names(), m.animaciones.active().to_string(), "Crear conjunto"))
        }
        _ => None,
    }
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
                // El editor visual del Prezi vive ARRIBA de los campos. El editor
                // tiene alto fijo (~`PREZI_EDITOR_H`); para que los campos no
                // queden aplastados en una franja sin scroll utilizable, su
                // viewport propio = el del panel menos el editor. Así ambos
                // (editor + campos) reparten el alto y el scroll de los campos
                // recorre toda la sección.
                let fields_vp = (VIEWPORT_H - PREZI_EDITOR_H).max(180.0);
                let fields = schema_panel(&one, &model.allichay, theme, fields_vp, Msg::Allichay);
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![prezi_editor_view(model, theme), fields])
            } else if sec.id.contains("mirada::monitores") {
                // La sección «Monitores» suma arriba el editor visual de
                // disposición (cajas-monitor arrastrables, sin giro); los campos
                // (dirección, overrides por salida) van debajo con su propio
                // viewport, igual que «Vista espacial».
                let fields_vp = (VIEWPORT_H - MONITOR_EDITOR_H).max(180.0);
                let fields = schema_panel(&one, &model.allichay, theme, fields_vp, Msg::Allichay);
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![monitor_editor_view(model, theme), fields])
            } else if sec.id == "barras::lista" {
                // Lista de barras: vista custom (fila = nombre + posición +
                // iconitos duplicar/borrar), que allichay no puede componer.
                barras_editor_view(model, theme)
            } else if let Some((items, active, crear)) = set_list_data(model, &sec.id) {
                // Bibliotecas de sets: fila = radio + nombre + iconitos dup/del;
                // abajo sólo «Crear». Misma estética que barras.
                lista_set_view(&sec.id, &items, &active, crear, theme)
            } else if let Some(surf) = sec
                .id
                .strip_prefix("pata::surface")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|&i| {
                    model
                        .pata
                        .surfaces
                        .get(i)
                        .is_some_and(|s| matches!(s.kind, pata_core::SurfaceKind::Bar | pata_core::SurfaceKind::Dock))
                })
            {
                // Superficie barra/dock: los campos del tipo + el compositor de
                // su fila de widgets (Inicio/Centro/Fin) debajo.
                let composer = bar_widgets_view(surf, &model.pata.surfaces[surf], theme);
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![composer, panel])
            } else if let Some(surf) = sec
                .id
                .strip_prefix("pata::surface")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|&i| {
                    model.pata.surfaces.get(i).is_some_and(|s| s.kind == pata_core::SurfaceKind::Sidebar)
                })
            {
                // Sidebar: compositor de sus dientes + los campos del tipo debajo.
                let composer = sidebar_dientes_view(surf, &model.pata.surfaces[surf], theme);
                View::new(Style {
                    flex_direction: FlexDirection::Column,
                    size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                    ..Default::default()
                })
                .children(vec![composer, panel])
            } else {
                panel
            }
        }
        None => resumen_view(title, sections, theme),
    };
    // Transición de escena: al cambiar de diente/item el contenido entra
    // deslizando suave desde abajo. La key es estable dentro de una misma
    // sección (no re-anima durante la edición/arrastre del lienzo).
    let canvas_content = canvas_content.animated_enter_from(
        key_of(&format!("canvas:{pest}:{sel_item:?}")),
        motion::SLOW,
        Affine::translate((0.0, 24.0)),
    );
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
    // Pop-in al montar la lista de items (cambio de diente): cada fila entra
    // con un fade keyed por su rótulo, estable mientras el diente no cambie.
    .animated_enter(key_of(&format!("item:{i}:{label}")), motion::NORMAL)
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
    // Cada diente se pinta como ícono vectorial (determinista en toda máquina),
    // no como glifo unicode. Mapeamos por título de pestaña a su IconSpec.
    let specs: Vec<tullpu_icon_core::IconSpec> =
        pestanas.iter().map(|p| iconos::spec_diente(&p.title)).collect();
    let rail = dock_rail_view(
        &items,
        RAIL_W,
        &DockRailPalette::from_theme(theme),
        move |id, size, color| tooth_icon(specs.get(id as usize).cloned(), size, color),
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

/// Icono de un diente: ícono **vectorial** (IconSpec → vello), pintado por el
/// puente `tullpu-icon-llimphi`. El `color` que resuelve el rail (activo/inactivo)
/// alimenta los `Color::Corriente` del spec; los acentos de color son fijos. A
/// diferencia del glifo de texto anterior, esto es determinista en toda máquina
/// (no depende de las fuentes del sistema).
fn tooth_icon(
    spec: Option<tullpu_icon_core::IconSpec>,
    size: f32,
    color: llimphi_ui::llimphi_raster::peniko::Color,
) -> View<Msg> {
    let spec = spec.unwrap_or_else(|| iconos::spec_diente(""));
    View::new(Style {
        size: Size {
            width: length(size),
            height: length(size),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
        ..Default::default()
    })
    .children(vec![tullpu_icon_llimphi::spec_view(spec, color)])
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

#[cfg(test)]
mod prezi_tests {
    use super::*;

    /// El editor arranca con un marco por escritorio (ids `1..=n`) y su plano
    /// vuelto a `overview_places` coincide con la grilla derivada (1×1 sin giro).
    #[test]
    fn arranca_con_un_marco_por_escritorio_y_round_trip() {
        let cfg = mirada_brain::Config::default();
        let pe = PreziEdit::from_config(&cfg);
        let n = mirada_brain::action::WORKSPACE_COUNT;
        assert_eq!(pe.rec.marcos.len(), n);
        assert_eq!(pe.rec.pasos.len(), n);
        let mut ids: Vec<u64> = pe.rec.marcos.iter().map(|m| m.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=n as u64).collect::<Vec<_>>());
        assert_eq!(pe.to_places(), cfg.overview_places_for(n));
    }

    /// Rotar el marco elegido se refleja en el giro del `OverviewPlace` correcto.
    #[test]
    fn rotar_marco_se_refleja_en_places() {
        let cfg = mirada_brain::Config::default();
        let mut pe = PreziEdit::from_config(&cfg);
        pe.sel = Some(1);
        pe.rec.rotar_marco(1, 0.5);
        let places = pe.to_places();
        assert!((places[0].rot - 0.5).abs() < 1e-6, "giro del escritorio 1 = {}", places[0].rot);
        // Los demás siguen rectos.
        assert!(places[1..].iter().all(|p| p.rot == 0.0));
    }

    /// Mover un marco una celda en X cambia su `x` persistido en 1 unidad de celda.
    #[test]
    fn mover_marco_una_celda_mueve_una_unidad() {
        let cfg = mirada_brain::Config::default();
        let mut pe = PreziEdit::from_config(&cfg);
        let before = pe.to_places()[0];
        pe.rec.mover_marco(1, PREZI_CELL, 0.0);
        let after = pe.to_places()[0];
        assert!((after.x - before.x - 1.0).abs() < 1e-5, "Δx = {}", after.x - before.x);
        assert!((after.y - before.y).abs() < 1e-5);
    }

    /// Un plano rico previo (posición libre + giro) se recupera como marcos y se
    /// vuelve a serializar idéntico — el editor preserva lo que mirada ya guardó.
    #[test]
    fn preserva_un_plano_rico_existente() {
        let mut cfg = mirada_brain::Config::default();
        let n = mirada_brain::action::WORKSPACE_COUNT;
        cfg.overview_places = (0..n)
            .map(|i| mirada_brain::OverviewPlace::new(i as f32 * 1.5, 0.3, 1.0, 1.0, 0.1 * i as f32))
            .collect();
        let pe = PreziEdit::from_config(&cfg);
        let places = pe.to_places();
        assert_eq!(places.len(), n);
        for (a, b) in places.iter().zip(cfg.overview_places.iter()) {
            assert!((a.x - b.x).abs() < 1e-4 && (a.rot - b.rot).abs() < 1e-4, "{a:?} vs {b:?}");
        }
    }
}

#[cfg(test)]
mod monitor_tests {
    use super::*;

    #[test]
    fn parse_modo_acepta_modos_drm() {
        assert_eq!(parse_modo("2560x1440"), Some((2560.0, 1440.0)));
        assert_eq!(parse_modo("1920x1080@60"), Some((1920.0, 1080.0)));
        assert_eq!(parse_modo("1366x768 60.0"), Some((1366.0, 768.0)));
        assert_eq!(parse_modo("basura"), None);
    }

    /// Arma un editor con dos cajas-monitor sintéticas (sin tocar el disco) para
    /// ejercitar `to_outputs` sin depender de `read_monitors`.
    fn editor_con(cajas: &[(&str, f64, f64, f64, f64)]) -> MonitorEdit {
        let mut rec = Recorrido::new();
        let mut nombres = Vec::new();
        for (i, (name, x, y, w, h)) in cajas.iter().enumerate() {
            rec.agregar_marco(Marco::new(
                (i + 1) as u64,
                DeckRect::new(*x, *y, *w, *h),
                ContenidoMarco::Texto { titulo: Some((*name).into()), parrafos: vec![] },
            ));
            nombres.push((*name).to_string());
        }
        MonitorEdit { rec, state: RecorridoState::new(), sel: None, grip: None, nombres }
    }

    /// Dos monitores lado a lado → dirección horizontal y `order` por la X.
    #[test]
    fn orden_horizontal_por_posicion_x() {
        // DP-1 a la derecha (x grande), HDMI-1 a la izquierda (x chico).
        let ed = editor_con(&[
            ("DP-1", 700.0, 0.0, 640.0, 360.0),
            ("HDMI-1", 0.0, 0.0, 480.0, 270.0),
        ]);
        let (outs, dir) = ed.to_outputs(&[]);
        assert_eq!(dir, "horizontal");
        // El de menor X queda primario (order 0).
        let hdmi = outs.iter().find(|o| o.name == "HDMI-1").unwrap();
        let dp = outs.iter().find(|o| o.name == "DP-1").unwrap();
        assert_eq!(hdmi.order, 0);
        assert_eq!(dp.order, 1);
    }

    /// Monitores apilados (más altos que anchos en conjunto) → dirección vertical.
    #[test]
    fn dos_apilados_dan_direccion_vertical() {
        let ed = editor_con(&[
            ("DP-1", 0.0, 0.0, 300.0, 400.0),
            ("HDMI-1", 0.0, 500.0, 300.0, 400.0),
        ]);
        let (_outs, dir) = ed.to_outputs(&[]);
        assert_eq!(dir, "vertical");
    }

    /// Los campos previos (wallpaper/escala/transform) se preservan y un override
    /// de salida desconectada (ausente del editor) sobrevive al volcado.
    #[test]
    fn preserva_campos_y_overrides_desconectados() {
        let prev = vec![
            mirada_brain::OutputOverride {
                name: "DP-1".into(),
                wallpaper_path: "/fondo.png".into(),
                wallpaper_fit: "fill".into(),
                order: 9,
                scale_120: 180,
                transform: "90".into(),
            },
            mirada_brain::OutputOverride {
                name: "VGA-1".into(), // desconectado: no está en el editor
                wallpaper_path: String::new(),
                wallpaper_fit: String::new(),
                order: 3,
                scale_120: 0,
                transform: String::new(),
            },
        ];
        let ed = editor_con(&[("DP-1", 0.0, 0.0, 640.0, 360.0)]);
        let (outs, _dir) = ed.to_outputs(&prev);
        let dp = outs.iter().find(|o| o.name == "DP-1").unwrap();
        // order recalculado, pero escala/wallpaper/transform intactos.
        assert_eq!(dp.order, 0);
        assert_eq!(dp.scale_120, 180);
        assert_eq!(dp.wallpaper_path, "/fondo.png");
        assert_eq!(dp.transform, "90");
        // El override de la salida desconectada sigue presente.
        assert!(outs.iter().any(|o| o.name == "VGA-1" && o.scale_120 == 0));
    }
}
