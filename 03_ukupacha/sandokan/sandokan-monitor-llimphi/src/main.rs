//! `sandokan-monitor` — el monitor de procesos de gioser sobre Llimphi.
//!
//! Dos mundos, una sola fachada:
//!
//! - **Linux**: observa las unidades vivas **por el contrato**
//!   [`sandokan::Engine`] (`list`+`status`+`telemetry`), vía
//!   [`sandokan_monitor_core::observe`]. No mira `/proc` ni el card store
//!   crudo — eso sería una segunda fuente de verdad, justo el duplicado que
//!   `shared/sandokan/SDD.md` elimina. El Engine lo elige
//!   [`sandokan::auto_default`] por precedencia del SDD (init arje-zero →
//!   daemon → local in-process).
//! - **Wawa**: censo de las apps WASM instaladas (lectura host-side de los
//!   assets del kernel). El censo del *executor en vivo* + balizas del
//!   compositor es Fase 4 del SDD (lado-wawa, pieza futura) — se anuncia
//!   honestamente en el panel.
//!
//! Cada unidad es una tarjeta viva: punto de estado por color, CPU con
//! **sparkline** (paint_with), memoria, hilos y restarts. Seleccionar una
//! tarjeta revela detener (SIGTERM→grace) / matar (grace 0) — ambos viajan
//! por el **mismo** Engine, así "lo que ves" y "lo que controlás" son la
//! misma fuente.
//!
//! El monitor **no inventa** un canal de observación paralelo: es la cara de
//! sólo-lectura del plano de control (SDD §6).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use card_core::{Card, Payload, Supervision};
use sandokan::lifecycle::LifecycleState;
use sandokan::{auto_default, Engine, Intent, IsolationLevel};
use sandokan_monitor_core::{observe, MonitorSnapshot, UnitObservation};
use ulid::Ulid;

use app_bus::{AppMenu, Menu, MenuItem};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, FlexWrap, JustifyContent,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Gradient};
use llimphi_ui::llimphi_text::{draw_layout, measurement, Alignment};
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT};

mod procfs;
mod treemap;
use procfs::{Scan, Sig};

/// Muestras de CPU guardadas por unidad para dibujar el sparkline.
const SPARK_LEN: usize = 48;
/// Cadencia del polling al Engine.
const POLL: Duration = Duration::from_millis(1000);
/// Filas de proceso visibles a la vez en el modo Sistema (ventana virtual).
const SYS_ROWS: usize = 26;
/// Puntos de historial en los gráficos de CPU/memoria (~2 min a 1 Hz).
const GRAPH_LEN: usize = 120;

// ---------------------------------------------------------------------------
// Contexto de ejecución compartido (runtime tokio + Engine elegido).
// ---------------------------------------------------------------------------

/// El Engine es async; Llimphi es sync. Encapsulamos un runtime tokio y el
/// `Box<dyn Engine>` (que es `Send + Sync`) en un `Arc` que los hilos de
/// polling/control clonan barato.
struct EngineCtx {
    rt: tokio::runtime::Runtime,
    engine: Box<dyn Engine>,
}

impl EngineCtx {
    fn poll(&self) -> Result<MonitorSnapshot, String> {
        self.rt
            .block_on(observe(&*self.engine))
            .map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Modelo / mensajes.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
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
enum Sort {
    Cpu,
    Mem,
    Pid,
    Name,
    Uptime,
}

/// Un proceso del SO ya con %CPU/%MEM derivados, listo para pintar.
#[derive(Clone)]
struct SysProc {
    pid: i32,
    ppid: i32,
    name: String,
    state: char,
    cpu_pct: f32,
    mem_pct: f32,
    rss_kb: u64,
    threads: u32,
    uid: u32,
    /// Antigüedad del proceso en segundos (uptime del sistema − starttime).
    uptime_secs: u64,
    cmd: String,
}

#[derive(Clone)]
struct WawaApp {
    name: String,
    bytes: u64,
}

#[derive(Clone)]
enum Msg {
    /// Resultado de un poll al Engine (snapshot o error de transporte).
    Snapshot(Result<MonitorSnapshot, String>),
    /// Barrido de `/proc` (modo Sistema). El %CPU se deriva en `update`.
    System(Scan),
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
    Signal(i32, Sig),
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

/// Menú de la app (Monitor / Ver / Ayuda). Los `command` los mapea
/// `update` en `Msg::MenuCmd`.
fn build_menu() -> AppMenu {
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

struct Model {
    theme: Theme,
    tab: Tab,
    snapshot: MonitorSnapshot,
    /// Historial de CPU por unidad → sparkline.
    history: HashMap<Ulid, VecDeque<f32>>,
    selected: Option<Ulid>,
    error: Option<String>,
    wawa: Vec<WawaApp>,
    // --- modo Sistema (/proc) ---
    system: Vec<SysProc>,
    sys_sel: Option<i32>,
    sys_sort: Sort,
    sys_scroll: usize,
    /// Modo árbol (padre/hijo) vs lista plana ordenable.
    sys_tree: bool,
    /// PIDs con su subárbol colapsado.
    collapsed: HashSet<i32>,
    /// Filtro por nombre/comando/PID (vacío = sin filtro).
    sys_filter: String,
    /// Capturando teclas para el filtro (modo búsqueda activo).
    filter_mode: bool,
    /// Treemap: `true` colorea/dimensiona por CPU, `false` por memoria.
    map_cpu: bool,
    /// Zoom del treemap: si `Some(pid)`, sólo se muestra ese subárbol.
    map_root: Option<i32>,
    /// Último click en el mapa (pid, instante) para detectar doble-click.
    last_map_click: Option<(i32, Instant)>,
    mem_total_kb: u64,
    mem_avail_kb: u64,
    /// Historial de %uso por core + historial de %MEM (un punto por barrido).
    core_hist: Vec<VecDeque<f32>>,
    /// Números de core (ordenados), para etiquetar los gráficos `CPUn`.
    core_ids: Vec<u32>,
    mem_hist: VecDeque<f32>,
    /// Lectura previa `(total, idle)` por core, para derivar %uso por delta.
    prev_core: Vec<(u64, u64)>,
    /// Jiffies previos por PID + total, para derivar %CPU por proceso.
    prev_proc: HashMap<i32, u64>,
    prev_total: u64,
    // --- menú ---
    menu: AppMenu,
    menu_open: Option<usize>,
    ctx: Arc<EngineCtx>,
}

struct Monitor;

// ---------------------------------------------------------------------------
// Arranque del Engine + siembra opcional de demo.
// ---------------------------------------------------------------------------

fn build_ctx() -> EngineCtx {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime tokio");
    let engine = rt.block_on(auto_default());
    let ctx = EngineCtx { rt, engine };
    // Si no hay init/daemon, `auto_default` cae al LocalEngine in-process y la
    // lista arranca vacía. Para que `cargo run` muestre algo vivo sin montar
    // un arje-zero, `SANDOKAN_MONITOR_SEED=1` siembra unas unidades reales
    // (procesos hijos de verdad — los observa el mismo Engine).
    if std::env::var("SANDOKAN_MONITOR_SEED").is_ok() {
        if ctx.poll().map(|s| s.is_empty()).unwrap_or(true) {
            seed_demo(&ctx);
        }
    }
    ctx
}

/// Siembra procesos reales vía el Engine (sin sandbox: `IsolationLevel::None`
/// = mismo namespace, sin root). Son `sh -c` portables: tres durmientes y un
/// "worker" que pulsa CPU para que el sparkline tenga vida.
fn seed_demo(ctx: &EngineCtx) {
    let specs: &[(&str, &str)] = &[
        ("reposo-α", "exec sleep 100000"),
        ("reposo-β", "exec sleep 100000"),
        ("vigía", "while :; do sleep 2; done"),
        (
            "worker-pulso",
            "while :; do dd if=/dev/zero of=/dev/null bs=1M count=64 2>/dev/null; sleep 1; done",
        ),
    ];
    for (label, script) in specs {
        let mut card = Card::new(*label);
        card.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec!["sh".into(), "-c".into(), (*script).into()],
            envp: vec![],
        };
        card.supervision = Supervision::OneShot;
        let intent = Intent::new(card).with_isolation(IsolationLevel::None);
        let _ = ctx.rt.block_on(ctx.engine.run(intent));
    }
}

/// Censo host-side de las apps WASM de Wawa (lectura de los assets del
/// kernel). Es **observación del manifiesto instalado**, no del executor en
/// vivo (eso es Fase 4). Honesto y barato: un `read_dir`.
fn wawa_census() -> Vec<WawaApp> {
    let candidates = [
        std::env::var("SANDOKAN_WAWA_ASSETS").unwrap_or_default(),
        "03_ukupacha/wawa/wawa-kernel/assets".into(),
        "wawa-kernel/assets".into(),
    ];
    for dir in candidates.iter().filter(|d| !d.is_empty()) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut apps: Vec<WawaApp> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("wasm") {
                    return None;
                }
                let name = p.file_stem()?.to_string_lossy().into_owned();
                let bytes = e.metadata().ok()?.len();
                Some(WawaApp { name, bytes })
            })
            .collect();
        apps.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        if !apps.is_empty() {
            return apps;
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Modo Sistema: deltas de CPU, orden, cambio de pestaña.
// ---------------------------------------------------------------------------

/// Toma un barrido crudo de `/proc` y deriva %CPU/%MEM contra la lectura
/// previa (guardada en el Model). Deja `model.system` ordenado.
/// Empuja una muestra al historial, recortando a `GRAPH_LEN`.
fn push_capped(buf: &mut VecDeque<f32>, v: f32) {
    if buf.len() == GRAPH_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}

fn ingest_system(model: &mut Model, scan: Scan) {
    let dtotal = scan.total_jiffies.saturating_sub(model.prev_total).max(1) as f32;
    let ncpu = scan.ncpu.max(1) as f32;
    let mem_total = scan.mem_total_kb.max(1) as f32;

    let mut next_prev = HashMap::with_capacity(scan.procs.len());
    let mut out = Vec::with_capacity(scan.procs.len());
    for p in &scan.procs {
        let dproc = p
            .cpu_jiffies
            .saturating_sub(model.prev_proc.get(&p.pid).copied().unwrap_or(p.cpu_jiffies))
            as f32;
        // delta_proc / delta_total_de_una_cpu = delta_proc / (dtotal/ncpu).
        let cpu_pct = (dproc / (dtotal / ncpu)).clamp(0.0, 100.0 * ncpu);
        next_prev.insert(p.pid, p.cpu_jiffies);
        let uptime_secs = if scan.clk_tck > 0 {
            (scan.uptime_secs - p.starttime_ticks as f64 / scan.clk_tck as f64).max(0.0) as u64
        } else {
            0
        };
        out.push(SysProc {
            pid: p.pid,
            ppid: p.ppid,
            name: p.name.clone(),
            state: p.state,
            cpu_pct,
            mem_pct: (p.rss_kb as f32 / mem_total) * 100.0,
            rss_kb: p.rss_kb,
            threads: p.threads,
            uid: p.uid,
            uptime_secs,
            cmd: p.cmd.clone(),
        });
    }

    // %uso por core: delta(busy)/delta(total) contra la lectura previa.
    if model.core_hist.len() != scan.cores.len() {
        model.core_hist = vec![VecDeque::new(); scan.cores.len()];
        model.prev_core = vec![(0, 0); scan.cores.len()];
    }
    for (i, &(_id, total, idle)) in scan.cores.iter().enumerate() {
        let (ptotal, pidle) = model.prev_core[i];
        let dtot = total.saturating_sub(ptotal) as f32;
        let didle = idle.saturating_sub(pidle) as f32;
        let usage = if dtot > 0.0 {
            ((dtot - didle) / dtot).clamp(0.0, 1.0) * 100.0
        } else {
            0.0
        };
        push_capped(&mut model.core_hist[i], usage);
    }
    model.core_ids = scan.cores.iter().map(|&(id, _, _)| id).collect();
    model.prev_core = scan.cores.iter().map(|&(_, t, i)| (t, i)).collect();

    // % de memoria usada para el gráfico de memoria.
    let mem_used_pct = if scan.mem_total_kb > 0 {
        (1.0 - scan.mem_avail_kb as f32 / scan.mem_total_kb as f32).clamp(0.0, 1.0) * 100.0
    } else {
        0.0
    };
    push_capped(&mut model.mem_hist, mem_used_pct);

    model.prev_proc = next_prev;
    model.prev_total = scan.total_jiffies;
    model.mem_total_kb = scan.mem_total_kb;
    model.mem_avail_kb = scan.mem_avail_kb;
    model.system = out;
    sort_system(model);

    // El proceso seleccionado pudo morir entre barridos.
    if let Some(sel) = model.sys_sel {
        if !model.system.iter().any(|p| p.pid == sel) {
            model.sys_sel = None;
        }
    }
    // Si la raíz de zoom del mapa murió, salir del zoom.
    if let Some(r) = model.map_root {
        if !model.system.iter().any(|p| p.pid == r) {
            model.map_root = None;
        }
    }
    let max = render_list(model).len().saturating_sub(SYS_ROWS);
    if model.sys_scroll > max {
        model.sys_scroll = max;
    }
}

/// Reajusta el scroll para que la fila seleccionada quede en la ventana visible
/// (según el orden de render actual: lista o árbol).
fn ensure_visible(model: &mut Model) {
    let Some(pid) = model.sys_sel else { return };
    let rows = render_list(model);
    if let Some(i) = rows.iter().position(|r| model.system[r.idx].pid == pid) {
        if i < model.sys_scroll {
            model.sys_scroll = i;
        } else if i >= model.sys_scroll + SYS_ROWS {
            model.sys_scroll = i + 1 - SYS_ROWS;
        }
    }
}

fn sort_system(model: &mut Model) {
    match model.sys_sort {
        Sort::Cpu => model
            .system
            .sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct)),
        Sort::Mem => model.system.sort_by(|a, b| b.rss_kb.cmp(&a.rss_kb)),
        Sort::Pid => model.system.sort_by(|a, b| a.pid.cmp(&b.pid)),
        Sort::Name => model
            .system
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
        // Más viejo primero (mayor uptime arriba).
        Sort::Uptime => model
            .system
            .sort_by(|a, b| b.uptime_secs.cmp(&a.uptime_secs)),
    }
}

/// Una fila tal como se va a pintar: índice en `model.system`, profundidad en
/// el árbol y si tiene hijos (para el triángulo de colapso).
#[derive(Clone, Copy)]
struct RenderRow {
    idx: usize,
    depth: u16,
    has_kids: bool,
}

/// La lista de filas a pintar/recorrer: plana (modo lista) o aplanada DFS del
/// árbol padre/hijo (modo árbol), respetando los subárboles colapsados. Es la
/// única fuente de orden — render, scroll, navegación ↑↓ comparten esto.
fn render_list(model: &Model) -> Vec<RenderRow> {
    let q = model.sys_filter.trim().to_lowercase();
    // Con filtro activo se aplana a lista plana de coincidencias (filtrar un
    // árbol rompería la jerarquía — comportamiento htop).
    if !q.is_empty() {
        return model
            .system
            .iter()
            .enumerate()
            .filter(|(_, p)| proc_matches(p, &q))
            .map(|(idx, _)| RenderRow {
                idx,
                depth: 0,
                has_kids: false,
            })
            .collect();
    }
    if !model.sys_tree {
        return (0..model.system.len())
            .map(|idx| RenderRow {
                idx,
                depth: 0,
                has_kids: false,
            })
            .collect();
    }
    flatten_tree(&model.system, &model.collapsed)
}

/// Un proceso coincide con `q` (ya en minúsculas) si lo contiene su nombre, su
/// línea de comando o su PID.
fn proc_matches(p: &SysProc, q: &str) -> bool {
    p.name.to_lowercase().contains(q)
        || p.cmd.to_lowercase().contains(q)
        || p.pid.to_string().contains(q)
}

/// Aplana el bosque padre/hijo de `system` (ya ordenado) en orden DFS,
/// saltando los subárboles colapsados. Pura para poder testearla.
fn flatten_tree(system: &[SysProc], collapsed: &HashSet<i32>) -> Vec<RenderRow> {
    // pid → índice (en el orden ya ordenado por sys_sort).
    let pos: HashMap<i32, usize> = system.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    // ppid → hijos (índices), preservando el orden global ordenado.
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    let mut roots: Vec<usize> = Vec::new();
    for (i, p) in system.iter().enumerate() {
        if p.ppid != p.pid && p.ppid != 0 && pos.contains_key(&p.ppid) {
            children.entry(p.ppid).or_default().push(i);
        } else {
            roots.push(i);
        }
    }

    let mut out = Vec::with_capacity(system.len());
    let mut seen: HashSet<i32> = HashSet::new();
    // Pila DFS (índice, profundidad); se empuja en reversa para emitir en orden.
    let mut stack: Vec<(usize, u16)> = roots.iter().rev().map(|&i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        let pid = system[i].pid;
        if !seen.insert(pid) {
            continue; // guarda anti-ciclo (ppid patológico)
        }
        let kids = children.get(&pid);
        let has_kids = kids.map(|k| !k.is_empty()).unwrap_or(false);
        out.push(RenderRow {
            idx: i,
            depth,
            has_kids,
        });
        if has_kids && !collapsed.contains(&pid) {
            for &c in kids.unwrap().iter().rev() {
                stack.push((c, depth + 1));
            }
        }
    }
    out
}

/// PIDs del subárbol con raíz `root` (incluida), siguiendo `ppid`.
fn subtree_pids(system: &[SysProc], root: i32) -> HashSet<i32> {
    let mut kids: HashMap<i32, Vec<i32>> = HashMap::new();
    for p in system {
        kids.entry(p.ppid).or_default().push(p.pid);
    }
    let mut set = HashSet::new();
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        if set.insert(pid) {
            if let Some(cs) = kids.get(&pid) {
                stack.extend(cs.iter().copied());
            }
        }
    }
    set
}

fn switch_tab(model: &mut Model, tab: Tab, handle: &Handle<Msg>) {
    model.tab = tab;
    match tab {
        Tab::Wawa if model.wawa.is_empty() => {
            handle.spawn(|| Msg::WawaCensus(wawa_census()));
        }
        Tab::System | Tab::Map => handle.spawn(|| Msg::System(procfs::scan())),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// App.
// ---------------------------------------------------------------------------

impl App for Monitor {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Sandokan · Monitor de procesos"
    }

    fn app_id() -> Option<&'static str> {
        Some("sandokan.monitor")
    }

    fn initial_size() -> (u32, u32) {
        (900, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let ctx = Arc::new(build_ctx());

        // Primer poll inmediato (que la UI no espere un ciclo entero).
        let c0 = ctx.clone();
        handle.spawn(move || Msg::Snapshot(c0.poll()));

        // Polling periódico por el contrato Engine.
        let cp = ctx.clone();
        handle.spawn_periodic(POLL, move || Msg::Snapshot(cp.poll()));

        // Barrido de /proc para el modo Sistema (fuente del SO, no del Engine).
        handle.spawn(|| Msg::System(procfs::scan()));
        handle.spawn_periodic(POLL, || Msg::System(procfs::scan()));

        // Censo de Wawa en background (no bloquea el arranque).
        handle.spawn(|| Msg::WawaCensus(wawa_census()));

        Model {
            theme: Theme::dark(),
            tab: Tab::System,
            snapshot: MonitorSnapshot::default(),
            history: HashMap::new(),
            selected: None,
            error: None,
            wawa: Vec::new(),
            system: Vec::new(),
            sys_sel: None,
            sys_sort: Sort::Cpu,
            sys_scroll: 0,
            sys_tree: true,
            collapsed: HashSet::new(),
            sys_filter: String::new(),
            filter_mode: false,
            map_cpu: false,
            map_root: None,
            last_map_click: None,
            mem_total_kb: 0,
            mem_avail_kb: 0,
            core_hist: Vec::new(),
            core_ids: Vec::new(),
            mem_hist: VecDeque::new(),
            prev_core: Vec::new(),
            prev_proc: HashMap::new(),
            prev_total: 0,
            menu: build_menu(),
            menu_open: None,
            ctx,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Snapshot(Ok(snap)) => {
                // Empuja la muestra de CPU al historial de cada unidad viva.
                let mut alive = HashMap::new();
                for u in &snap.units {
                    let cpu = u.telemetry.as_ref().map(|t| t.cpu_pct as f32).unwrap_or(0.0);
                    let buf = model
                        .history
                        .remove(&u.card_id)
                        .unwrap_or_else(|| VecDeque::with_capacity(SPARK_LEN));
                    let mut buf = buf;
                    if buf.len() == SPARK_LEN {
                        buf.pop_front();
                    }
                    buf.push_back(cpu);
                    alive.insert(u.card_id, buf);
                }
                model.history = alive; // descarta historiales de unidades muertas
                model.snapshot = snap;
                model.error = None;
            }
            Msg::Snapshot(Err(e)) => model.error = Some(e),
            Msg::System(scan) => {
                ingest_system(&mut model, scan);
            }
            Msg::SysSelect(pid) => {
                model.sys_sel = (pid >= 0).then_some(pid);
                ensure_visible(&mut model);
            }
            Msg::SysSort(s) => {
                model.sys_sort = s;
                sort_system(&mut model);
            }
            Msg::SysScroll(steps) => {
                let max = render_list(&model).len().saturating_sub(SYS_ROWS);
                let cur = model.sys_scroll as i64 + steps as i64;
                model.sys_scroll = cur.clamp(0, max as i64) as usize;
            }
            Msg::SysTree(on) => {
                model.sys_tree = on;
                model.sys_scroll = 0;
                ensure_visible(&mut model);
            }
            Msg::SysToggleNode(pid) => {
                if !model.collapsed.remove(&pid) {
                    model.collapsed.insert(pid);
                }
                let max = render_list(&model).len().saturating_sub(SYS_ROWS);
                if model.sys_scroll > max {
                    model.sys_scroll = max;
                }
            }
            Msg::MapMetric(cpu) => model.map_cpu = cpu,
            Msg::MapClick(pid) => {
                model.sys_sel = Some(pid);
                let now = Instant::now();
                let dbl = matches!(model.last_map_click,
                    Some((p, t)) if p == pid && now.duration_since(t) < Duration::from_millis(450));
                if dbl {
                    model.map_root = Some(pid); // zoom al subárbol
                    model.last_map_click = None;
                } else {
                    model.last_map_click = Some((pid, now));
                }
            }
            Msg::MapRoot(r) => {
                model.map_root = r;
                model.last_map_click = None;
            }
            Msg::MapZoomOut => {
                // Sube al subárbol del padre; si el padre no está a la vista,
                // vuelve a "todo".
                if let Some(r) = model.map_root {
                    let parent = model.system.iter().find(|p| p.pid == r).map(|p| p.ppid);
                    model.map_root =
                        parent.filter(|pp| model.system.iter().any(|p| p.pid == *pp));
                }
                model.last_map_click = None;
            }
            Msg::FilterMode(on) => model.filter_mode = on,
            Msg::FilterSet(s) => {
                model.sys_filter = s;
                model.sys_scroll = 0;
                ensure_visible(&mut model);
            }
            Msg::FilterClose => {
                model.filter_mode = false;
                model.sys_filter.clear();
                model.sys_scroll = 0;
            }
            Msg::Signal(pid, sig) => {
                if let Err(e) = procfs::signal(pid, sig) {
                    model.error = Some(format!("señal a {pid}: {e}"));
                } else {
                    model.error = None;
                    handle.spawn(|| Msg::System(procfs::scan()));
                }
            }
            Msg::Switch(tab) => switch_tab(&mut model, tab, handle),
            Msg::Select(s) => model.selected = s,
            Msg::Stop(id) => {
                let ctx = model.ctx.clone();
                handle.spawn(move || {
                    let _ = ctx.rt.block_on(ctx.engine.stop(id, Duration::from_secs(3)));
                    Msg::Snapshot(ctx.poll())
                });
                model.selected = None;
            }
            Msg::Kill(id) => {
                let ctx = model.ctx.clone();
                handle.spawn(move || {
                    let _ = ctx.rt.block_on(ctx.engine.stop(id, Duration::ZERO));
                    Msg::Snapshot(ctx.poll())
                });
                model.selected = None;
            }
            Msg::WawaCensus(apps) => model.wawa = apps,
            Msg::MenuOpen(o) => model.menu_open = o,
            Msg::MenuCmd(cmd) => {
                model.menu_open = None;
                match cmd.as_str() {
                    "view.system" => switch_tab(&mut model, Tab::System, handle),
                    "view.map" => switch_tab(&mut model, Tab::Map, handle),
                    "view.units" => switch_tab(&mut model, Tab::Units, handle),
                    "view.wawa" => switch_tab(&mut model, Tab::Wawa, handle),
                    "monitor.refresh" => {
                        let ctx = model.ctx.clone();
                        handle.spawn(move || Msg::Snapshot(ctx.poll()));
                        handle.spawn(|| Msg::System(procfs::scan()));
                    }
                    "monitor.seed" => {
                        let ctx = model.ctx.clone();
                        handle.spawn(move || {
                            seed_demo(&ctx);
                            Msg::Snapshot(ctx.poll())
                        });
                    }
                    "app.quit" => handle.quit(),
                    _ => {}
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let t = &model.theme;
        let body = match model.tab {
            Tab::System => system_body(model),
            Tab::Map => map_body(model),
            Tab::Units => units_body(model),
            Tab::Wawa => wawa_body(model),
        };

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0),
                height: percent(1.0),
            },
            ..Default::default()
        })
        .fill(t.bg_app)
        .children(vec![
            menubar_view(&menu_spec(model)),
            header(model),
            tabs(model),
            body,
        ])
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        menubar_overlay(&menu_spec(model))
    }

    /// Bindings reales (los shortcuts del menú son sólo etiquetas; el binding
    /// vive acá). `Esc` cierra el menú o deselecciona · `Tab` cicla pestañas ·
    /// `↑/↓` mueven la selección en Sistema · `Supr`/`k` terminan/matan el
    /// proceso seleccionado · `Ctrl+R`/`F5` refresca · `Ctrl+Q` sale ·
    /// `Ctrl+1/2/3` van a Sistema/Unidades/Wawa.
    fn on_key(model: &Model, ev: &KeyEvent) -> Option<Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }

        // Modo filtro: el tipeo edita el texto de búsqueda. Las flechas y los
        // atajos con Ctrl caen al manejo normal (filtrar y navegar a la vez).
        if model.tab == Tab::System && model.filter_mode {
            match &ev.key {
                Key::Named(NamedKey::Escape) => return Some(Msg::FilterClose),
                Key::Named(NamedKey::Enter) => return Some(Msg::FilterMode(false)),
                Key::Named(NamedKey::Backspace) => {
                    let mut s = model.sys_filter.clone();
                    s.pop();
                    return Some(Msg::FilterSet(s));
                }
                _ => {
                    if !ev.modifiers.ctrl && !ev.modifiers.meta {
                        if let Some(txt) = &ev.text {
                            if !txt.is_empty() && txt.chars().all(|c| !c.is_control()) {
                                return Some(Msg::FilterSet(format!("{}{txt}", model.sys_filter)));
                            }
                        }
                    }
                }
            }
        }

        match &ev.key {
            Key::Named(NamedKey::Escape) => {
                return Some(if model.menu_open.is_some() {
                    Msg::MenuOpen(None)
                } else if model.sys_sel.is_some() {
                    Msg::SysSelect(-1)
                } else {
                    Msg::Select(None)
                });
            }
            Key::Named(NamedKey::F5) => return Some(Msg::MenuCmd("monitor.refresh".into())),
            Key::Named(NamedKey::Tab) => {
                let next = match model.tab {
                    Tab::System => "view.map",
                    Tab::Map => "view.units",
                    Tab::Units => "view.wawa",
                    Tab::Wawa => "view.system",
                };
                return Some(Msg::MenuCmd(next.into()));
            }
            Key::Named(NamedKey::ArrowDown) if model.tab == Tab::System => {
                return sys_move(model, 1);
            }
            Key::Named(NamedKey::ArrowUp) if model.tab == Tab::System => {
                return sys_move(model, -1);
            }
            Key::Named(NamedKey::Delete)
                if model.tab == Tab::System || model.tab == Tab::Map =>
            {
                return model.sys_sel.map(|p| Msg::Signal(p, Sig::Term));
            }
            // En el mapa, Backspace sube un nivel de zoom.
            Key::Named(NamedKey::Backspace)
                if model.tab == Tab::Map && model.map_root.is_some() =>
            {
                return Some(Msg::MapZoomOut);
            }
            // En árbol: ← colapsa, → expande el nodo seleccionado.
            Key::Named(NamedKey::ArrowLeft) if model.tab == Tab::System && model.sys_tree => {
                if let Some(p) = model.sys_sel {
                    if !model.collapsed.contains(&p) {
                        return Some(Msg::SysToggleNode(p));
                    }
                }
            }
            Key::Named(NamedKey::ArrowRight) if model.tab == Tab::System && model.sys_tree => {
                if let Some(p) = model.sys_sel {
                    if model.collapsed.contains(&p) {
                        return Some(Msg::SysToggleNode(p));
                    }
                }
            }
            // `/` abre el filtro en Sistema (estilo htop/less).
            Key::Character(c)
                if model.tab == Tab::System && !ev.modifiers.ctrl && c.as_str() == "/" =>
            {
                return Some(Msg::FilterMode(true));
            }
            Key::Character(c) if ev.modifiers.ctrl => {
                match c.as_str().to_ascii_lowercase().as_str() {
                    "f" if model.tab == Tab::System => return Some(Msg::FilterMode(true)),
                    "r" => return Some(Msg::MenuCmd("monitor.refresh".into())),
                    "q" => return Some(Msg::MenuCmd("app.quit".into())),
                    "1" => return Some(Msg::MenuCmd("view.system".into())),
                    "2" => return Some(Msg::MenuCmd("view.map".into())),
                    "3" => return Some(Msg::MenuCmd("view.units".into())),
                    "4" => return Some(Msg::MenuCmd("view.wawa".into())),
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    fn on_wheel(
        model: &Model,
        delta: llimphi_ui::WheelDelta,
        _cursor: (f32, f32),
        _mods: llimphi_ui::Modifiers,
    ) -> Option<Msg> {
        if model.tab == Tab::System {
            // Convención CSS: delta.y positivo = hacia abajo.
            let steps = delta.y.trunc() as i32;
            if steps != 0 {
                return Some(Msg::SysScroll(steps));
            }
        }
        None
    }
}

/// Mueve la selección en la tabla de Sistema siguiendo el **orden de render**
/// (en árbol, recorre la jerarquía aplanada visible).
fn sys_move(model: &Model, dir: i32) -> Option<Msg> {
    let rows = render_list(model);
    if rows.is_empty() {
        return None;
    }
    let cur = model
        .sys_sel
        .and_then(|p| rows.iter().position(|r| model.system[r.idx].pid == p));
    let next = match cur {
        Some(i) => (i as i32 + dir).clamp(0, rows.len() as i32 - 1) as usize,
        None => 0,
    };
    Some(Msg::SysSelect(model.system[rows[next].idx].pid))
}

/// Spec de la barra de menú — armado en cada `view()`/`view_overlay()`.
fn menu_spec(model: &Model) -> MenuBarSpec<'_, Msg> {
    MenuBarSpec {
        menu: &model.menu,
        open: model.menu_open,
        theme: &model.theme,
        viewport: (900.0, 600.0),
        height: DEFAULT_HEIGHT,
        on_open: Arc::new(Msg::MenuOpen),
        on_command: Arc::new(|s: &str| Msg::MenuCmd(s.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Cabecera + pestañas de mundo.
// ---------------------------------------------------------------------------

fn header(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut chips = match model.tab {
        Tab::System | Tab::Map => {
            let cpu: f32 = model.system.iter().map(|p| p.cpu_pct).sum();
            let rss: u64 = model.system.iter().map(|p| p.rss_kb).sum::<u64>() * 1024;
            vec![
                chip(t, "procesos", &model.system.len().to_string()),
                chip(t, "cpu", &format!("{cpu:.0}%")),
                chip(t, "rss", &fmt_mem(rss)),
                chip(t, "ram", &fmt_mem(model.mem_total_kb * 1024)),
            ]
        }
        Tab::Units => {
            let snap = &model.snapshot;
            let mem: u64 = snap
                .units
                .iter()
                .filter_map(|u| u.telemetry.as_ref().map(|x| x.mem_bytes))
                .sum();
            let cpu: f64 = snap
                .units
                .iter()
                .filter_map(|u| u.telemetry.as_ref().map(|x| x.cpu_pct))
                .sum();
            vec![
                chip(t, "unidades", &snap.len().to_string()),
                chip(t, "vivas", &snap.running().to_string()),
                chip(t, "memoria", &fmt_mem(mem)),
                chip(t, "cpu", &format!("{cpu:.0}%")),
            ]
        }
        Tab::Wawa => vec![chip(t, "apps wasm", &model.wawa.len().to_string())],
    };
    if let Some(e) = &model.error {
        chips.push(chip_warn(t, "aviso", e));
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: pad(16.0, 12.0),
        gap: Size {
            width: length(8.0),
            height: length(8.0),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(vec![
        View::new(Style::default()).text("Sandokan · Monitor", 17.0, t.fg_text),
        View::new(Style {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0),
                height: length(8.0),
            },
            ..Default::default()
        })
        .children(chips),
    ])
}

fn tabs(model: &Model) -> View<Msg> {
    let t = &model.theme;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(6.0),
            height: length(6.0),
        },
        padding: Rect {
            left: length(16.0),
            right: length(16.0),
            top: length(0.0),
            bottom: length(8.0),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(vec![
        tab(t, "Sistema", model.tab == Tab::System, Msg::Switch(Tab::System)),
        tab(t, "Mapa", model.tab == Tab::Map, Msg::Switch(Tab::Map)),
        tab(t, "Unidades", model.tab == Tab::Units, Msg::Switch(Tab::Units)),
        tab(t, "Wawa", model.tab == Tab::Wawa, Msg::Switch(Tab::Wawa)),
    ])
}

fn tab(t: &Theme, label: &str, active: bool, on: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (t.accent, t.bg_app)
    } else {
        (t.bg_button, t.fg_muted)
    };
    View::new(Style {
        padding: pad(14.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(7.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 13.0, fg)
    .on_click(on)
}

// ---------------------------------------------------------------------------
// Modo Mapa: treemap jerárquico (fractal) de procesos.
// ---------------------------------------------------------------------------

fn map_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.system.is_empty() {
        return empty_state(t, "Leyendo /proc…", "Armando el mapa de procesos.");
    }
    let cpu = model.map_cpu;
    // Con zoom, restringe al subárbol de la raíz (incluida).
    let subtree = model
        .map_root
        .filter(|r| model.system.iter().any(|p| p.pid == *r))
        .map(|r| subtree_pids(&model.system, r));

    // Datos para el painter (owned → Send + Sync + 'static).
    let items: Vec<treemap::Item> = model
        .system
        .iter()
        .filter(|p| subtree.as_ref().map(|s| s.contains(&p.pid)).unwrap_or(true))
        .map(|p| treemap::Item {
            pid: p.pid,
            ppid: p.ppid,
            weight: if cpu { p.cpu_pct as f64 } else { p.rss_kb as f64 },
            cpu: p.cpu_pct,
            mem_kb: p.rss_kb,
            label: p.name.clone(),
        })
        .collect();

    let border = t.bg_app;
    let label_col = Color::from_rgba8(0x0d, 0x10, 0x14, 0xff);
    let accent = t.accent;
    let sel = model.sys_sel;
    let hit_items = items.clone();

    let canvas = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .clip(true)
    .paint_with(move |scene, ts, rect| {
        let cells = treemap::layout(&items, (rect.x, rect.y, rect.w, rect.h), 15.0, 3.0);
        for c in &cells {
            let r = llimphi_ui::llimphi_raster::kurbo::Rect::new(
                c.x as f64,
                c.y as f64,
                (c.x + c.w) as f64,
                (c.y + c.h) as f64,
            );
            // Color categórico por proceso; la opacidad sube con el uso de CPU
            // y baja con la profundidad (sensación fractal). Contenedor: tenue.
            let base = name_color(&c.label);
            let a = if c.leaf {
                (0.60 + c.cpu / 100.0 * 0.34 - c.depth as f32 * 0.05).clamp(0.5, 0.95)
            } else {
                0.14
            };
            // Gradiente vertical leve: arriba un toque más claro, abajo el base
            // — da volumen sin estridencia.
            let top = base.map_lightness(|l| (l + 0.07).min(1.0)).with_alpha(a);
            let bot = base.map_lightness(|l| (l - 0.05).max(0.0)).with_alpha(a);
            let grad = Gradient::new_linear((c.x as f64, c.y as f64), (c.x as f64, (c.y + c.h) as f64))
                .with_stops([top, bot]);
            scene.fill(Fill::NonZero, Affine::IDENTITY, &grad, None, &r);
            if sel == Some(c.pid) {
                scene.stroke(&Stroke::new(2.5), Affine::IDENTITY, accent, None, &r);
            } else {
                scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &r);
            }

            // Etiqueta: nombre arriba y, si hay alto, %CPU · RAM debajo.
            if c.w > 46.0 && c.h > 15.0 {
                let name = ts.layout(&c.label, 11.0, None, Alignment::Start, 1.2, false, None);
                if measurement(&name).width <= c.w - 6.0 {
                    draw_layout(scene, &name, label_col, ((c.x + 3.0) as f64, (c.y + 2.0) as f64));
                }
                if c.h > 30.0 {
                    let stats = format!("{:.0}% · {}", c.cpu, fmt_mem(c.mem_kb * 1024));
                    let sl = ts.layout(&stats, 9.5, None, Alignment::Start, 1.2, false, None);
                    if measurement(&sl).width <= c.w - 6.0 {
                        let sc = label_col.with_alpha(0.72);
                        draw_layout(scene, &sl, sc, ((c.x + 3.0) as f64, (c.y + 15.0) as f64));
                    }
                }
            }
        }
    })
    .on_click_at(move |x, y, w, h| {
        // Recomputa el layout en coords LOCALES (0,0,w,h) —las mismas que
        // entrega `on_click_at`— y resuelve el rect más profundo (último
        // dibujado) que contiene el punto.
        let cells = treemap::layout(&hit_items, (0.0, 0.0, w, h), 15.0, 3.0);
        cells
            .iter()
            .rev()
            .find(|c| x >= c.x && x <= c.x + c.w && y >= c.y && y <= c.y + c.h)
            .map(|c| Msg::MapClick(c.pid))
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![map_toolbar(model), canvas])
}

fn map_toolbar(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut row = vec![
        View::new(Style::default()).text("Área por:", 12.0, t.fg_muted),
        seg_btn(t, "Memoria", !model.map_cpu, Msg::MapMetric(false)),
        seg_btn(t, "CPU", model.map_cpu, Msg::MapMetric(true)),
    ];
    // Breadcrumb de zoom (si estamos dentro de un subárbol).
    if let Some(r) = model.map_root {
        let name = model
            .system
            .iter()
            .find(|p| p.pid == r)
            .map(|p| p.name.as_str())
            .unwrap_or("?");
        row.push(seg_btn(t, "◂ Subir", false, Msg::MapZoomOut));
        row.push(seg_btn(t, "Todo", false, Msg::MapRoot(None)));
        row.push(
            View::new(Style::default())
                .text(format!("zoom: {name}"), 11.5, name_color(name)),
        );
    }
    match model.sys_sel.and_then(|pid| model.system.iter().find(|p| p.pid == pid)) {
        Some(p) => {
            row.push(
                View::new(Style {
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .text(format!("▸ PID {} · {}", p.pid, p.name), 12.0, name_color(&p.name)),
            );
            row.push(action_btn(t, "Terminar", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Term)));
            row.push(action_btn(t, "Matar", t.fg_destructive, t.bg_app, Msg::Signal(p.pid, Sig::Kill)));
        }
        None => row.push(
            View::new(Style {
                flex_grow: 1.0,
                ..Default::default()
            })
            .text(
                "Click: seleccionar · doble-click: zoom al subárbol · color por proceso",
                11.0,
                t.fg_muted,
            ),
        ),
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(6.0),
        },
        padding: pad(16.0, 8.0),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .children(row)
}

// ---------------------------------------------------------------------------
// Modo Unidades (sandokan): grilla de tarjetas vivas.
// ---------------------------------------------------------------------------

fn units_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.snapshot.is_empty() {
        return empty_state(
            t,
            "Sin unidades vivas",
            "No hay init (arje-zero) ni daemon sandokan en este entorno: el \
             Engine cayó al LocalEngine in-process. Exportá \
             SANDOKAN_MONITOR_SEED=1 y reabrí para sembrar una demo viva.",
        );
    }

    let cards: Vec<View<Msg>> = model
        .snapshot
        .units
        .iter()
        .map(|u| unit_card(model, u))
        .collect();

    scroll_grid(t, cards)
}

fn unit_card(model: &Model, u: &UnitObservation) -> View<Msg> {
    let t = &model.theme;
    let selected = model.selected == Some(u.card_id);
    let (dot, state_txt) = state_visual(t, &u.state);

    let cpu = u.telemetry.as_ref().map(|x| x.cpu_pct).unwrap_or(0.0);
    let mem = u.telemetry.as_ref().map(|x| x.mem_bytes).unwrap_or(0);
    let nproc = u.telemetry.as_ref().map(|x| x.nproc).unwrap_or(0);

    // Fila título: punto de estado + label.
    let title_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(10.0),
                height: length(10.0),
            },
            ..Default::default()
        })
        .fill(dot)
        .radius(5.0),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text(&u.label, 14.0, t.fg_text),
        View::new(Style::default()).text(state_txt, 11.0, t.fg_muted),
    ]);

    // Sparkline de CPU.
    let spark = sparkline(t, model.history.get(&u.card_id), cpu);

    // Fila métricas.
    let restarts = if u.restarts > 0 {
        format!("↻{}", u.restarts)
    } else {
        String::new()
    };
    let metrics = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(12.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(vec![
        metric(t, &format!("{cpu:.0}% cpu")),
        metric(t, &fmt_mem(mem)),
        metric(t, &format!("{nproc} hilos")),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text(&restarts, 11.0, t.accent),
    ]);

    let mut children = vec![title_row, spark, metrics];

    // Acciones inline al seleccionar (detener/matar por el Engine).
    if selected {
        children.push(actions_row(t, u.card_id));
    }

    let bg = if selected { t.bg_selected } else { t.bg_panel_alt };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(8.0),
            height: length(8.0),
        },
        padding: pad(13.0, 12.0),
        size: Size {
            width: length(260.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(10.0)
    .hover_fill(t.bg_row_hover)
    .on_click(Msg::Select(if selected {
        None
    } else {
        Some(u.card_id)
    }))
}

fn actions_row(t: &Theme, id: Ulid) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(8.0),
            height: length(8.0),
        },
        ..Default::default()
    })
    .children(vec![
        action_btn(t, "⏹ detener", t.bg_button, t.fg_text, Msg::Stop(id)),
        action_btn(t, "✕ matar", t.fg_destructive, t.bg_app, Msg::Kill(id)),
    ])
}

fn action_btn(t: &Theme, label: &str, bg: Color, fg: Color, on: Msg) -> View<Msg> {
    View::new(Style {
        padding: pad(12.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(7.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 12.0, fg)
    .on_click(on)
}

// ---------------------------------------------------------------------------
// Modo Sistema: tabla de procesos del SO (/proc) — el process monitor.
// ---------------------------------------------------------------------------

// Anchos de columna (px); la última (comando) crece.
const W_PID: f32 = 62.0;
const W_CPU: f32 = 58.0;
const W_MEM: f32 = 58.0;
const W_RSS: f32 = 78.0;
const W_ST: f32 = 28.0;
const W_THR: f32 = 46.0;
const W_UID: f32 = 54.0;
const W_TIME: f32 = 66.0;
const ROW_H: f32 = 21.0;

fn system_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.system.is_empty() {
        return empty_state(t, "Leyendo /proc…", "Barriendo los procesos del sistema.");
    }

    let rows = render_list(model);
    let total = rows.len();
    let start = model.sys_scroll.min(total.saturating_sub(1));
    let end = (start + SYS_ROWS).min(total);

    let mut table: Vec<View<Msg>> = Vec::with_capacity(end - start + 2);
    table.push(sys_header_row(model));
    for r in &rows[start..end] {
        let p = &model.system[r.idx];
        let node = model.sys_tree.then_some((r.depth, r.has_kids, model.collapsed.contains(&p.pid)));
        table.push(sys_row(t, p, model.sys_sel == Some(p.pid), node));
    }
    if end < total {
        table.push(
            View::new(Style {
                padding: pad(10.0, 4.0),
                ..Default::default()
            })
            .text(
                &format!("… {} filas más abajo (rueda / ↑↓)", total - end),
                10.5,
                t.fg_muted,
            ),
        );
    }

    let sel = model
        .sys_sel
        .and_then(|pid| model.system.iter().find(|p| p.pid == pid));

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![
        sys_graphs(model),
        sys_action_bar(model, sel),
        sys_filter_bar(model, total),
        View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0),
                height: auto(),
            },
            padding: Rect {
                left: length(12.0),
                right: length(12.0),
                top: length(0.0),
                bottom: length(8.0),
            },
            ..Default::default()
        })
        .clip(true)
        .children(table),
    ])
}

/// Fila de gráficos del tope: un gráfico de %uso por **core** + uno de memoria,
/// en FlexWrap (en ventanas angostas los cores bajan de fila).
fn sys_graphs(model: &Model) -> View<Msg> {
    let t = &model.theme;

    let mut items: Vec<View<Msg>> = Vec::with_capacity(model.core_hist.len() + 1);
    for (i, hist) in model.core_hist.iter().enumerate() {
        let id = model.core_ids.get(i).copied().unwrap_or(i as u32);
        let now = hist.back().copied().unwrap_or(0.0);
        // El valor de la cabecera toma el color del nivel actual; la línea se
        // colorea por tramo según el uso (verde→ámbar→rojo).
        items.push(meter(t, &format!("CPU{id}"), &format!("{now:.0}%"), hist, usage_color(now), 126.0, true));
    }
    let mem_now = model.mem_hist.back().copied().unwrap_or(0.0);
    let used_kb = model.mem_total_kb.saturating_sub(model.mem_avail_kb);
    items.push(meter(
        t,
        "Memoria",
        &format!("{} / {} · {mem_now:.0}%", fmt_mem(used_kb * 1024), fmt_mem(model.mem_total_kb * 1024)),
        &model.mem_hist,
        t.accent,
        236.0,
        false,
    ));

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        gap: Size {
            width: length(10.0),
            height: length(8.0),
        },
        padding: pad(16.0, 10.0),
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(items)
}

/// Color categórico estable por nombre de proceso (mismo nombre → mismo color),
/// para que el treemap y la lista sean coloridos y coherentes entre sí.
fn name_color(name: &str) -> Color {
    const P: [(u8, u8, u8); 16] = [
        (0x5a, 0x9b, 0xd4),
        (0x6a, 0xc4, 0x6a),
        (0xe0, 0xb0, 0x3a),
        (0xd9, 0x65, 0x5a),
        (0xb0, 0x7a, 0xd9),
        (0x40, 0xc4, 0xc4),
        (0xe0, 0x8a, 0x4a),
        (0xd8, 0x6a, 0xa8),
        (0x8a, 0xc2, 0x4a),
        (0x4a, 0x8a, 0xd9),
        (0xc4, 0xa0, 0x40),
        (0x6a, 0xd9, 0xa0),
        (0xd9, 0x6a, 0x6a),
        (0x9a, 0x8a, 0xd9),
        (0x50, 0xb0, 0xd9),
        (0xc4, 0x6a, 0x9a),
    ];
    // FNV-1a sobre el nombre.
    let mut h: u32 = 2166136261;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    let (r, g, b) = P[(h as usize) % P.len()];
    Color::from_rgba8(r, g, b, 0xff)
}

/// Color por nivel de uso: verde (bajo) → ámbar (medio) → rojo (alto).
fn usage_color(pct: f32) -> Color {
    if pct >= 85.0 {
        Color::from_rgba8(0xd9, 0x53, 0x4f, 0xff)
    } else if pct >= 60.0 {
        Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff)
    } else {
        Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff)
    }
}

/// Un medidor de ancho fijo: cabecera (label + valor) sobre un gráfico de área
/// del historial (escala fija 0..100 %). Pintado con `paint_with`. Si
/// `by_usage`, cada tramo de la línea se colorea por su nivel (CPU); si no, usa
/// `color` plano (memoria).
fn meter(
    t: &Theme,
    label: &str,
    value: &str,
    hist: &VecDeque<f32>,
    color: Color,
    width: f32,
    by_usage: bool,
) -> View<Msg> {
    let samples: Vec<f32> = hist.iter().copied().collect();
    let area_col = color.with_alpha(0.18);
    let track = t.bg_input;

    let head = View::new(Style {
        flex_direction: FlexDirection::Row,
        justify_content: Some(JustifyContent::SpaceBetween),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![
        View::new(Style::default()).text(label, 10.5, t.fg_muted),
        View::new(Style::default()).text(value, 11.0, color),
    ]);

    let graph = View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(32.0),
        },
        ..Default::default()
    })
    .fill(track)
    .radius(6.0)
    .clip(true)
    .paint_with(move |scene, _ts, rect| {
        let n = samples.len();
        if n < 2 {
            return;
        }
        let pad = 2.0_f32;
        let w = (rect.w - pad * 2.0).max(1.0);
        let h = (rect.h - pad * 2.0).max(1.0);
        let x0 = rect.x + pad;
        let ybase = (rect.y + pad + h) as f64;
        let step = w / (n as f32 - 1.0);
        let xat = |i: usize| (x0 + step * i as f32) as f64;
        let yat = |v: f32| (rect.y + pad + h * (1.0 - (v / 100.0).clamp(0.0, 1.0))) as f64;

        // Área bajo la curva.
        let mut area = BezPath::new();
        area.move_to((xat(0), ybase));
        for (i, v) in samples.iter().enumerate() {
            area.line_to((xat(i), yat(*v)));
        }
        area.line_to((xat(n - 1), ybase));
        area.close_path();
        scene.fill(Fill::NonZero, Affine::IDENTITY, area_col, None, &area);

        // Línea superior. Con `by_usage`, cada tramo se tiñe por su nivel.
        let stroke = Stroke::new(1.5);
        if by_usage {
            for i in 1..n {
                let mut seg = BezPath::new();
                seg.move_to((xat(i - 1), yat(samples[i - 1])));
                seg.line_to((xat(i), yat(samples[i])));
                let c = usage_color(samples[i].max(samples[i - 1]));
                scene.stroke(&stroke, Affine::IDENTITY, c, None, &seg);
            }
        } else {
            let mut line = BezPath::new();
            for (i, v) in samples.iter().enumerate() {
                let p = (xat(i), yat(*v));
                if i == 0 {
                    line.move_to(p);
                } else {
                    line.line_to(p);
                }
            }
            scene.stroke(&stroke, Affine::IDENTITY, color, None, &line);
        }
    });

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        size: Size {
            width: length(width),
            height: auto(),
        },
        gap: Size {
            width: length(0.0),
            height: length(5.0),
        },
        ..Default::default()
    })
    .children(vec![head, graph])
}

/// Barra de acciones: toggle Lista/Árbol + acciones sobre el seleccionado.
fn sys_action_bar(model: &Model, sel: Option<&SysProc>) -> View<Msg> {
    let t = &model.theme;
    let mut row: Vec<View<Msg>> = vec![
        seg_btn(t, "Árbol", model.sys_tree, Msg::SysTree(true)),
        seg_btn(t, "Lista", !model.sys_tree, Msg::SysTree(false)),
    ];
    match sel {
        Some(p) => {
            row.push(
                View::new(Style {
                    flex_grow: 1.0,
                    ..Default::default()
                })
                .text(&format!("PID {} · {}", p.pid, p.name), 12.5, t.fg_text),
            );
            row.push(action_btn(t, "Terminar", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Term)));
            row.push(action_btn(t, "Matar", t.fg_destructive, t.bg_app, Msg::Signal(p.pid, Sig::Kill)));
            row.push(action_btn(t, "Pausar", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Stop)));
            row.push(action_btn(t, "Seguir", t.bg_button, t.fg_text, Msg::Signal(p.pid, Sig::Cont)));
        }
        None => row.push(
            View::new(Style {
                flex_grow: 1.0,
                ..Default::default()
            })
            .text(
                "Elegí un proceso (click / ↑↓) para terminar, matar, pausar o seguir.",
                12.0,
                t.fg_muted,
            ),
        ),
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(6.0),
        },
        padding: pad(16.0, 8.0),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .children(row)
}

/// Barra de filtro (búsqueda por nombre/comando/PID). Click la enfoca; `/`
/// también. Muestra el texto en vivo con caret, el conteo de coincidencias y
/// una ✕ para limpiar.
fn sys_filter_bar(model: &Model, matches: usize) -> View<Msg> {
    let t = &model.theme;
    let has = !model.sys_filter.is_empty();
    let active = model.filter_mode;

    let (shown, color) = if !has && !active {
        (
            "Filtrar por nombre o PID  ·  «/» o Ctrl+F".to_string(),
            t.fg_placeholder,
        )
    } else {
        let caret = if active { "▏" } else { "" };
        (format!("{}{caret}", model.sys_filter), t.fg_text)
    };

    let mut row: Vec<View<Msg>> = vec![
        View::new(Style {
            size: Size {
                width: length(24.0),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text("/", 13.0, t.fg_muted),
        View::new(Style {
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .clip(true)
        .text(shown, 12.0, color)
        .on_click(Msg::FilterMode(true)),
    ];
    if has {
        row.push(
            View::new(Style::default())
                .text(format!("{matches} coinciden"), 11.0, t.fg_muted),
        );
        row.push(action_btn(t, "✕", t.bg_button, t.fg_text, Msg::FilterClose));
    }

    let bg = if active { t.bg_input_focus } else { t.bg_input };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(6.0),
        },
        padding: pad(16.0, 6.0),
        ..Default::default()
    })
    .fill(bg)
    .children(row)
}

fn sys_header_row(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let hcell = |label: &str, w: f32, sort: Option<Sort>| {
        let active = sort.map(|s| s == model.sys_sort).unwrap_or(false);
        let fg = if active { t.accent } else { t.fg_muted };
        let mut v = View::new(Style {
            size: Size {
                width: length(w),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text(label, 10.5, fg);
        if let Some(s) = sort {
            v = v.on_click(Msg::SysSort(s));
        }
        v
    };
    let cmd = {
        let active = model.sys_sort == Sort::Name;
        let fg = if active { t.accent } else { t.fg_muted };
        View::new(Style {
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text("COMANDO (nombre↕)", 10.5, fg)
        .on_click(Msg::SysSort(Sort::Name))
    };
    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        size: Size {
            width: percent(1.0),
            height: length(ROW_H + 4.0),
        },
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .children(vec![
        hcell("PID", W_PID, Some(Sort::Pid)),
        hcell("%CPU", W_CPU, Some(Sort::Cpu)),
        hcell("%MEM", W_MEM, Some(Sort::Mem)),
        hcell("RSS", W_RSS, Some(Sort::Mem)),
        hcell("S", W_ST, None),
        hcell("HILOS", W_THR, None),
        hcell("UID", W_UID, None),
        hcell("TIEMPO", W_TIME, Some(Sort::Uptime)),
        cmd,
    ])
}

/// `node = Some((depth, has_kids, collapsed))` en modo árbol; `None` en lista.
fn sys_row(t: &Theme, p: &SysProc, selected: bool, node: Option<(u16, bool, bool)>) -> View<Msg> {
    let cell = |s: String, w: f32, color: Color| {
        View::new(Style {
            size: Size {
                width: length(w),
                height: percent(1.0),
            },
            flex_shrink: 0.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .text(s, 11.5, color)
    };
    let bg = if selected { t.bg_selected } else { t.bg_app };
    // %CPU coloreado por nivel cuando hay actividad; el comando toma el color
    // categórico del proceso (coherente con el treemap).
    let cpu_col = if p.cpu_pct >= 0.5 {
        usage_color(p.cpu_pct)
    } else {
        t.fg_muted
    };
    let cmd_col = name_color(&p.name);

    // Celda de comando: en árbol lleva sangría por profundidad + triángulo de
    // colapso (dibujado, no glifo de fuente) antes del texto.
    let cmd_cell = {
        let mut parts: Vec<View<Msg>> = Vec::new();
        if let Some((depth, has_kids, collapsed)) = node {
            let indent = depth as f32 * 14.0;
            if indent > 0.0 {
                parts.push(spacer(indent));
            }
            parts.push(tri_node(t, has_kids, collapsed, p.pid));
        }
        parts.push(command_node(&p.cmd, cmd_col));
        View::new(Style {
            flex_grow: 1.0,
            flex_basis: length(0.0),
            min_size: Size {
                width: length(0.0),
                height: auto(),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .clip(true)
        .children(parts)
    };

    View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        size: Size {
            width: percent(1.0),
            height: length(ROW_H),
        },
        gap: Size {
            width: length(6.0),
            height: length(0.0),
        },
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(0.0),
            bottom: length(0.0),
        },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(t.bg_row_hover)
    .on_click(Msg::SysSelect(p.pid))
    .children(vec![
        cell(p.pid.to_string(), W_PID, t.fg_muted),
        cell(format!("{:.1}", p.cpu_pct), W_CPU, cpu_col),
        cell(format!("{:.1}", p.mem_pct), W_MEM, t.fg_muted),
        cell(fmt_mem(p.rss_kb * 1024), W_RSS, t.fg_muted),
        cell(p.state.to_string(), W_ST, state_color(t, p.state)),
        cell(p.threads.to_string(), W_THR, t.fg_muted),
        cell(p.uid.to_string(), W_UID, t.fg_muted),
        cell(fmt_dur(p.uptime_secs), W_TIME, t.fg_muted),
        cmd_cell,
    ])
}

/// Triángulo de colapso del árbol, **dibujado** (no glifo de fuente, que salía
/// tofu): ▶ colapsado / ▼ expandido. Las hojas quedan en blanco. Clickeable.
fn tri_node(t: &Theme, has_kids: bool, collapsed: bool, pid: i32) -> View<Msg> {
    let col = t.fg_muted;
    let mut v = View::new(Style {
        size: Size {
            width: length(15.0),
            height: length(ROW_H),
        },
        flex_shrink: 0.0,
        ..Default::default()
    });
    if has_kids {
        v = v.paint_with(move |scene, _ts, rect| {
            let cx = rect.x + rect.w / 2.0;
            let cy = rect.y + rect.h / 2.0;
            let s = 3.6_f32;
            let mut tri = BezPath::new();
            if collapsed {
                // apunta a la derecha ▶
                tri.move_to(((cx - s) as f64, (cy - s) as f64));
                tri.line_to(((cx - s) as f64, (cy + s) as f64));
                tri.line_to(((cx + s) as f64, cy as f64));
            } else {
                // apunta abajo ▼
                tri.move_to(((cx - s) as f64, (cy - s) as f64));
                tri.line_to(((cx + s) as f64, (cy - s) as f64));
                tri.line_to((cx as f64, (cy + s) as f64));
            }
            tri.close_path();
            scene.fill(Fill::NonZero, Affine::IDENTITY, col, None, &tri);
        });
        v = v.on_click(Msg::SysToggleNode(pid));
    }
    v
}

/// Celda de comando: **rellena el espacio disponible** (flex), texto a la
/// izquierda, una sola línea, y se pica con `...` si no entra. Pintado con
/// `paint_with` para medir contra el ancho REAL de la columna (responsive) y
/// elipsar pixel-exacto. Esto evita reservar una columna gigante.
fn command_node(cmd: &str, color: Color) -> View<Msg> {
    let cmd = cmd.chars().take(512).collect::<String>();
    View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0),
        min_size: Size {
            width: length(0.0),
            height: auto(),
        },
        size: Size {
            width: auto(),
            height: length(ROW_H),
        },
        ..Default::default()
    })
    .clip(true)
    .paint_with(move |scene, ts, rect| {
        if cmd.is_empty() {
            return;
        }
        let avail = (rect.w - 4.0).max(1.0);
        let layout = ts.layout(&cmd, 11.5, None, Alignment::Start, 1.2, false, None);
        let m = measurement(&layout);
        let x = (rect.x + 2.0) as f64;
        let y = (rect.y + ((rect.h - m.height) / 2.0).max(0.0)) as f64;
        if m.width <= avail {
            draw_layout(scene, &layout, color, (x, y));
        } else {
            // Picar por estimación (ancho promedio de glifo) + "...".
            let n = cmd.chars().count().max(1);
            let avg = m.width / n as f32;
            let fit = ((avail / avg).floor() as usize).saturating_sub(2).min(n);
            let mut s: String = cmd.chars().take(fit).collect();
            s.push_str("...");
            let lay = ts.layout(&s, 11.5, None, Alignment::Start, 1.2, false, None);
            draw_layout(scene, &lay, color, (x, y));
        }
    })
}

/// Espaciador horizontal de ancho fijo (sangría del árbol).
fn spacer(w: f32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: percent(1.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

/// Botón segmentado chico (toggle Lista/Árbol).
fn seg_btn(t: &Theme, label: &str, active: bool, on: Msg) -> View<Msg> {
    let (bg, fg) = if active {
        (t.accent, t.bg_app)
    } else {
        (t.bg_button, t.fg_muted)
    };
    View::new(Style {
        padding: pad(11.0, 5.0),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .hover_fill(t.bg_button_hover)
    .text(label, 11.5, fg)
    .on_click(on)
}

fn state_color(t: &Theme, s: char) -> Color {
    match s {
        'R' => Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff),
        'D' => Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff),
        'Z' => t.fg_destructive,
        'T' | 't' => t.accent,
        _ => t.fg_muted,
    }
}

// ---------------------------------------------------------------------------
// Mundo Wawa: censo de apps WASM instaladas.
// ---------------------------------------------------------------------------

fn wawa_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut children = vec![note(
        t,
        "Censo del manifiesto (apps WASM instaladas, lectura host-side de los \
         assets del kernel). El censo del executor en vivo + balizas del \
         compositor es Fase 4 del SDD (lado-wawa, pieza futura).",
    )];

    if model.wawa.is_empty() {
        children.push(empty_state(
            t,
            "Sin assets de Wawa",
            "No encontré los .wasm del kernel. Apuntá SANDOKAN_WAWA_ASSETS al \
             directorio assets de wawa-kernel.",
        ));
    } else {
        let cards: Vec<View<Msg>> = model.wawa.iter().map(|a| wawa_card(t, a)).collect();
        children.push(scroll_grid(t, cards));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .children(children)
}

fn wawa_card(t: &Theme, a: &WawaApp) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(6.0),
            height: length(6.0),
        },
        padding: pad(13.0, 12.0),
        size: Size {
            width: length(190.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(10.0)
    .children(vec![
        View::new(Style {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0),
                height: length(4.0),
            },
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size {
                    width: length(10.0),
                    height: length(10.0),
                },
                ..Default::default()
            })
            .fill(t.accent)
            .radius(2.0),
            View::new(Style::default()).text(&a.name, 14.0, t.fg_text),
        ]),
        metric(t, &format!("{} · wasm", fmt_mem(a.bytes))),
    ])
}

// ---------------------------------------------------------------------------
// Primitivas de UI reutilizadas.
// ---------------------------------------------------------------------------

fn scroll_grid(t: &Theme, cards: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        align_items: Some(AlignItems::Start),
        gap: Size {
            width: length(12.0),
            height: length(12.0),
        },
        padding: pad(16.0, 16.0),
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .clip(true)
    .children(cards)
}

fn chip(t: &Theme, label: &str, value: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::End),
        padding: pad(10.0, 5.0),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(7.0)
    .children(vec![
        View::new(Style::default()).text(value, 14.0, t.fg_text),
        View::new(Style::default()).text(label, 9.5, t.fg_muted),
    ])
}

fn chip_warn(t: &Theme, label: &str, value: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::End),
        padding: pad(10.0, 5.0),
        size: Size {
            width: length(220.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(7.0)
    .children(vec![
        View::new(Style::default()).text(value, 11.0, t.fg_destructive),
        View::new(Style::default()).text(label, 9.5, t.fg_muted),
    ])
}

fn metric(t: &Theme, txt: &str) -> View<Msg> {
    View::new(Style::default()).text(txt, 11.5, t.fg_muted)
}

fn note(t: &Theme, txt: &str) -> View<Msg> {
    View::new(Style {
        padding: pad(16.0, 10.0),
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .line_height(1.35)
    .text(txt, 11.5, t.fg_muted)
}

fn empty_state(t: &Theme, title: &str, body: &str) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(10.0),
            height: length(10.0),
        },
        padding: pad(40.0, 40.0),
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(vec![
        View::new(Style::default()).text(title, 16.0, t.fg_text),
        View::new(Style {
            size: Size {
                width: length(420.0),
                height: auto(),
            },
            ..Default::default()
        })
        .line_height(1.4)
        .text(body, 12.0, t.fg_muted),
    ])
}

// ---------------------------------------------------------------------------
// Sparkline de CPU (canvas custom vía paint_with).
// ---------------------------------------------------------------------------

fn sparkline(t: &Theme, hist: Option<&VecDeque<f32>>, _cpu: f64) -> View<Msg> {
    let samples: Vec<f32> = hist.map(|h| h.iter().copied().collect()).unwrap_or_default();
    let line = t.accent;
    let track = t.bg_input;
    View::new(Style {
        size: Size {
            width: percent(1.0),
            height: length(34.0),
        },
        ..Default::default()
    })
    .fill(track)
    .radius(6.0)
    .paint_with(move |scene, _ts, rect| {
        if samples.len() < 2 {
            return;
        }
        // Escala vertical: 0..max(100, pico) para que picos sobre 100% no
        // se recorten, pero la línea base sea siempre 100%.
        let peak = samples.iter().cloned().fold(100.0_f32, f32::max);
        let pad = 5.0_f32;
        let w = (rect.w - pad * 2.0).max(1.0);
        let h = (rect.h - pad * 2.0).max(1.0);
        let n = samples.len();
        let step = w / (n as f32 - 1.0);
        let mut path = BezPath::new();
        for (i, v) in samples.iter().enumerate() {
            let x = rect.x + pad + step * i as f32;
            let y = rect.y + pad + h * (1.0 - (v / peak).clamp(0.0, 1.0));
            if i == 0 {
                path.move_to((x as f64, y as f64));
            } else {
                path.line_to((x as f64, y as f64));
            }
        }
        scene.stroke(&Stroke::new(1.6), Affine::IDENTITY, line, None, &path);
    })
}

// ---------------------------------------------------------------------------
// Helpers de estado / formato.
// ---------------------------------------------------------------------------

fn state_visual(t: &Theme, s: &LifecycleState) -> (Color, &'static str) {
    match s {
        LifecycleState::Running => (Color::from_rgba8(0x3f, 0xcf, 0x6a, 0xff), "vivo"),
        LifecycleState::Pending => (Color::from_rgba8(0xe0, 0xb0, 0x3a, 0xff), "pendiente"),
        LifecycleState::Exited { .. } => (t.fg_muted, "salió"),
        LifecycleState::Failed { .. } => (t.fg_destructive, "falló"),
        LifecycleState::Killed => (Color::from_rgba8(0x9a, 0x55, 0x55, 0xff), "matado"),
    }
}

fn fmt_mem(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GiB", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} MiB")
    } else {
        format!("{} KiB", bytes / 1024)
    }
}

/// Duración compacta: `3d4h`, `5h02`, `12:34` (mm:ss), `45s`.
fn fmt_dur(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{d}d{h}h")
    } else if h > 0 {
        format!("{h}h{m:02}")
    } else if m > 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{s}s")
    }
}

/// Padding horizontal/vertical uniforme.
fn pad(h: f32, v: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(h),
        right: length(h),
        top: length(v),
        bottom: length(v),
    }
}

fn main() {
    llimphi_ui::run::<Monitor>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32) -> SysProc {
        SysProc {
            pid,
            ppid,
            name: format!("p{pid}"),
            state: 'S',
            cpu_pct: 0.0,
            mem_pct: 0.0,
            rss_kb: 0,
            threads: 1,
            uid: 0,
            uptime_secs: 0,
            cmd: format!("p{pid}"),
        }
    }

    #[test]
    fn arbol_anida_por_ppid_con_profundidad() {
        // 1 → {2 → {4}, 3};  9 huérfano (ppid fuera de la vista) = raíz.
        let sys = vec![
            proc(1, 0),
            proc(2, 1),
            proc(3, 1),
            proc(4, 2),
            proc(9, 999),
        ];
        let rows = flatten_tree(&sys, &HashSet::new());
        let seq: Vec<(i32, u16)> = rows.iter().map(|r| (sys[r.idx].pid, r.depth)).collect();
        assert_eq!(seq, vec![(1, 0), (2, 1), (4, 2), (3, 1), (9, 0)]);
        // 1 y 2 tienen hijos; 4, 3, 9 no.
        assert!(rows[0].has_kids && rows[1].has_kids);
        assert!(!rows[2].has_kids && !rows[3].has_kids && !rows[4].has_kids);
    }

    #[test]
    fn filtro_matchea_nombre_comando_y_pid() {
        let mut p = proc(1234, 1);
        p.name = "firefox".into();
        p.cmd = "/usr/lib/firefox/firefox -contentproc".into();
        assert!(proc_matches(&p, "fire")); // por nombre
        assert!(proc_matches(&p, "contentproc")); // por comando
        assert!(proc_matches(&p, "234")); // por PID (substring)
        assert!(!proc_matches(&p, "chrome"));
    }

    #[test]
    fn colapsar_oculta_el_subarbol() {
        let sys = vec![proc(1, 0), proc(2, 1), proc(4, 2)];
        let mut collapsed = HashSet::new();
        collapsed.insert(2); // colapsa 2 → su hijo 4 desaparece
        let rows = flatten_tree(&sys, &collapsed);
        let pids: Vec<i32> = rows.iter().map(|r| sys[r.idx].pid).collect();
        assert_eq!(pids, vec![1, 2]);
        assert!(rows[1].has_kids, "2 sigue marcando que tiene hijos (colapsado)");
    }
}
