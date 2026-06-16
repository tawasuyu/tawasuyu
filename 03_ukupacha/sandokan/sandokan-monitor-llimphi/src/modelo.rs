//! Tipos del dominio: modelo de estado, mensajes, constantes globales.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use sandokan_monitor_core::MonitorSnapshot;
use ulid::Ulid;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;

use super::engine::EngineCtx;

// ---------------------------------------------------------------------------
// Constantes de configuración.
// ---------------------------------------------------------------------------

/// Muestras de CPU guardadas por unidad para dibujar el sparkline.
pub(crate) const SPARK_LEN: usize = 48;
/// Cadencia del polling al Engine.
pub(crate) const POLL: std::time::Duration = std::time::Duration::from_millis(1000);
/// Filas de proceso visibles a la vez en el modo Sistema (ventana virtual).
pub(crate) const SYS_ROWS: usize = 26;
/// Puntos de historial en los gráficos de CPU/memoria (~2 min a 1 Hz).
pub(crate) const GRAPH_LEN: usize = 120;

// ---------------------------------------------------------------------------
// Tipos de navegación.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    /// Todos los procesos del SO (lectura de `/proc`) — el process monitor.
    System,
    /// Treemap jerárquico (fractal) de procesos por memoria o CPU.
    Map,
    /// Unidades del plano de control sandokan (por el contrato Engine).
    Units,
    /// Censo de apps WASM instaladas de Wawa.
    Wawa,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sort {
    Cpu,
    Mem,
    Pid,
    Name,
    Uptime,
}

// ---------------------------------------------------------------------------
// Tipos de datos del dominio.
// ---------------------------------------------------------------------------

/// Un proceso del SO ya con %CPU/%MEM derivados, listo para pintar.
#[derive(Clone)]
pub(crate) struct SysProc {
    pub(crate) pid: i32,
    pub(crate) ppid: i32,
    pub(crate) name: String,
    pub(crate) state: char,
    pub(crate) cpu_pct: f32,
    pub(crate) mem_pct: f32,
    pub(crate) rss_kb: u64,
    pub(crate) threads: u32,
    pub(crate) uid: u32,
    /// Antigüedad del proceso en segundos (uptime del sistema − starttime).
    pub(crate) uptime_secs: u64,
    pub(crate) cmd: String,
}

#[derive(Clone)]
pub(crate) struct WawaApp {
    pub(crate) name: String,
    pub(crate) bytes: u64,
}

// ---------------------------------------------------------------------------
// Mensajes.
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum Msg {
    /// Resultado de un poll al Engine (snapshot o error de transporte).
    Snapshot(Result<MonitorSnapshot, String>),
    /// Barrido de `/proc` (modo Sistema). El %CPU se deriva en `update`.
    System(super::procfs::Scan),
    SysSelect(i32),
    SysSort(Sort),
    SysScroll(i32),
    /// Cambiar entre lista plana y árbol padre/hijo.
    SysTree(bool),
    /// Colapsar/expandir el subárbol de un PID.
    SysToggleNode(i32),
    /// Métrica del treemap: `true` = CPU, `false` = memoria.
    MapMetric(bool),
    /// Click en un rectángulo del mapa: selecciona; doble-click hace zoom.
    MapClick(i32),
    /// Fija la raíz de zoom del mapa (`None` = todo).
    MapRoot(Option<i32>),
    /// Sube un nivel de zoom (al padre de la raíz actual).
    MapZoomOut,
    /// Entrar/salir del modo filtro (sin borrar el texto).
    FilterMode(bool),
    /// Texto del filtro (edición en vivo).
    FilterSet(String),
    /// Salir del modo filtro y limpiar el texto.
    FilterClose,
    Signal(i32, super::procfs::Sig),
    Switch(Tab),
    Select(Option<Ulid>),
    Stop(Ulid),
    Kill(Ulid),
    WawaCensus(Vec<WawaApp>),
    /// Abrir/cerrar un menú raíz de la barra (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Command id elegido en un dropdown de la barra.
    MenuCmd(String),
}

// ---------------------------------------------------------------------------
// Modelo principal.
// ---------------------------------------------------------------------------

pub(crate) struct Model {
    pub(crate) theme: Theme,
    pub(crate) tab: Tab,
    pub(crate) snapshot: MonitorSnapshot,
    /// Historial de CPU por unidad → sparkline.
    pub(crate) history: HashMap<Ulid, VecDeque<f32>>,
    pub(crate) selected: Option<Ulid>,
    pub(crate) error: Option<String>,
    pub(crate) wawa: Vec<WawaApp>,
    // --- modo Sistema (/proc) ---
    pub(crate) system: Vec<SysProc>,
    pub(crate) sys_sel: Option<i32>,
    pub(crate) sys_sort: Sort,
    pub(crate) sys_scroll: usize,
    /// Modo árbol (padre/hijo) vs lista plana ordenable.
    pub(crate) sys_tree: bool,
    /// PIDs con su subárbol colapsado.
    pub(crate) collapsed: HashSet<i32>,
    /// Filtro por nombre/comando/PID (vacío = sin filtro).
    pub(crate) sys_filter: String,
    /// Capturando teclas para el filtro (modo búsqueda activo).
    pub(crate) filter_mode: bool,
    /// Treemap: `true` colorea/dimensiona por CPU, `false` por memoria.
    pub(crate) map_cpu: bool,
    /// Zoom del treemap: si `Some(pid)`, sólo se muestra ese subárbol.
    pub(crate) map_root: Option<i32>,
    /// Último click en el mapa (pid, instante) para detectar doble-click.
    pub(crate) last_map_click: Option<(i32, Instant)>,
    pub(crate) mem_total_kb: u64,
    pub(crate) mem_avail_kb: u64,
    /// Historial de %uso por core + historial de %MEM (un punto por barrido).
    pub(crate) core_hist: Vec<VecDeque<f32>>,
    /// Números de core (ordenados), para etiquetar los gráficos `CPUn`.
    pub(crate) core_ids: Vec<u32>,
    pub(crate) mem_hist: VecDeque<f32>,
    /// Lectura previa `(total, idle)` por core, para derivar %uso por delta.
    pub(crate) prev_core: Vec<(u64, u64)>,
    /// Jiffies previos por PID + total, para derivar %CPU por proceso.
    pub(crate) prev_proc: HashMap<i32, u64>,
    pub(crate) prev_total: u64,
    // --- menú ---
    pub(crate) menu: AppMenu,
    pub(crate) menu_open: Option<usize>,
    pub(crate) ctx: Arc<EngineCtx>,
}

// ---------------------------------------------------------------------------
// Construcción del menú de la app.
// ---------------------------------------------------------------------------

/// Menú de la app (Monitor / Ver / Ayuda). Los `command` los mapea
/// `update` en `Msg::MenuCmd`.
pub(crate) fn build_menu() -> AppMenu {
    AppMenu::new()
        .menu(
            Menu::new("Monitor")
                .item(MenuItem::new("Refrescar", "monitor.refresh").shortcut("Ctrl+R").icon("⟳"))
                .item(MenuItem::new("Sembrar demo", "monitor.seed").icon("✚").separated())
                .item(MenuItem::new("Salir", "app.quit").shortcut("Ctrl+Q").separated()),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Sistema", "view.system").shortcut("Ctrl+1"))
                .item(MenuItem::new("Mapa", "view.map").shortcut("Ctrl+2"))
                .item(MenuItem::new("Unidades", "view.units").shortcut("Ctrl+3"))
                .item(MenuItem::new("Wawa", "view.wawa").shortcut("Ctrl+4")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Observa por el contrato Engine", "help.about")))
}
