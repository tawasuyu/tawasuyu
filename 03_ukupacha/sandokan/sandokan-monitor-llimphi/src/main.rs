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

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

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
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT};

mod procfs;
use procfs::{Scan, Sig};

/// Muestras de CPU guardadas por unidad para dibujar el sparkline.
const SPARK_LEN: usize = 48;
/// Cadencia del polling al Engine.
const POLL: Duration = Duration::from_millis(1000);
/// Filas de proceso visibles a la vez en el modo Sistema (ventana virtual).
const SYS_ROWS: usize = 26;

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
}

/// Un proceso del SO ya con %CPU/%MEM derivados, listo para pintar.
#[derive(Clone)]
struct SysProc {
    pid: i32,
    name: String,
    state: char,
    cpu_pct: f32,
    mem_pct: f32,
    rss_kb: u64,
    threads: u32,
    uid: u32,
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
                .item(MenuItem::new("Unidades", "view.units").shortcut("Ctrl+2"))
                .item(MenuItem::new("Wawa", "view.wawa").shortcut("Ctrl+3")),
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
    mem_total_kb: u64,
    /// Jiffies previos por PID + total, para derivar %CPU por delta.
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
        out.push(SysProc {
            pid: p.pid,
            name: p.name.clone(),
            state: p.state,
            cpu_pct,
            mem_pct: (p.rss_kb as f32 / mem_total) * 100.0,
            rss_kb: p.rss_kb,
            threads: p.threads,
            uid: p.uid,
            cmd: p.cmd.clone(),
        });
    }

    model.prev_proc = next_prev;
    model.prev_total = scan.total_jiffies;
    model.mem_total_kb = scan.mem_total_kb;
    model.system = out;
    sort_system(model);

    // El proceso seleccionado pudo morir entre barridos.
    if let Some(sel) = model.sys_sel {
        if !model.system.iter().any(|p| p.pid == sel) {
            model.sys_sel = None;
        }
    }
    let max = model.system.len().saturating_sub(SYS_ROWS);
    if model.sys_scroll > max {
        model.sys_scroll = max;
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
    }
}

fn switch_tab(model: &mut Model, tab: Tab, handle: &Handle<Msg>) {
    model.tab = tab;
    match tab {
        Tab::Wawa if model.wawa.is_empty() => {
            handle.spawn(|| Msg::WawaCensus(wawa_census()));
        }
        Tab::System => handle.spawn(|| Msg::System(procfs::scan())),
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
            mem_total_kb: 0,
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
                // Mantener la fila seleccionada dentro de la ventana visible.
                if let Some(i) = model
                    .sys_sel
                    .and_then(|p| model.system.iter().position(|x| x.pid == p))
                {
                    if i < model.sys_scroll {
                        model.sys_scroll = i;
                    } else if i >= model.sys_scroll + SYS_ROWS {
                        model.sys_scroll = i - SYS_ROWS + 1;
                    }
                }
            }
            Msg::SysSort(s) => {
                model.sys_sort = s;
                sort_system(&mut model);
            }
            Msg::SysScroll(steps) => {
                let max = model.system.len().saturating_sub(SYS_ROWS);
                let cur = model.sys_scroll as i64 + steps as i64;
                model.sys_scroll = cur.clamp(0, max as i64) as usize;
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
                    Tab::System => "view.units",
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
            Key::Named(NamedKey::Delete) if model.tab == Tab::System => {
                return model.sys_sel.map(|p| Msg::Signal(p, Sig::Term));
            }
            Key::Character(c) if ev.modifiers.ctrl => {
                match c.as_str().to_ascii_lowercase().as_str() {
                    "r" => return Some(Msg::MenuCmd("monitor.refresh".into())),
                    "q" => return Some(Msg::MenuCmd("app.quit".into())),
                    "1" => return Some(Msg::MenuCmd("view.system".into())),
                    "2" => return Some(Msg::MenuCmd("view.units".into())),
                    "3" => return Some(Msg::MenuCmd("view.wawa".into())),
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

/// Mueve la selección en la tabla de Sistema y reajusta el scroll para
/// mantenerla visible.
fn sys_move(model: &Model, dir: i32) -> Option<Msg> {
    if model.system.is_empty() {
        return None;
    }
    let cur = model
        .sys_sel
        .and_then(|p| model.system.iter().position(|x| x.pid == p));
    let next = match cur {
        Some(i) => (i as i32 + dir).clamp(0, model.system.len() as i32 - 1) as usize,
        None => 0,
    };
    Some(Msg::SysSelect(model.system[next].pid))
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
        Tab::System => {
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
const ROW_H: f32 = 21.0;

fn system_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.system.is_empty() {
        return empty_state(t, "Leyendo /proc…", "Barriendo los procesos del sistema.");
    }

    let total = model.system.len();
    let start = model.sys_scroll.min(total.saturating_sub(1));
    let end = (start + SYS_ROWS).min(total);

    let mut table: Vec<View<Msg>> = Vec::with_capacity(end - start + 2);
    table.push(sys_header_row(model));
    for p in &model.system[start..end] {
        table.push(sys_row(t, p, model.sys_sel == Some(p.pid)));
    }
    if end < total {
        table.push(
            View::new(Style {
                padding: pad(10.0, 4.0),
                ..Default::default()
            })
            .text(
                &format!("… {} procesos más abajo (rueda / ↑↓)", total - end),
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
        sys_action_bar(t, sel),
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

/// Barra de acciones sobre el proceso seleccionado. Sin selección, una pista.
fn sys_action_bar(t: &Theme, sel: Option<&SysProc>) -> View<Msg> {
    let mut row: Vec<View<Msg>> = Vec::new();
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
        cmd,
    ])
}

fn sys_row(t: &Theme, p: &SysProc, selected: bool) -> View<Msg> {
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
    let cpu_col = if p.cpu_pct >= 1.0 { t.fg_text } else { t.fg_muted };
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
        View::new(Style {
            flex_grow: 1.0,
            justify_content: Some(JustifyContent::Center),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .clip(true)
        .text(&p.cmd, 11.5, t.fg_text),
    ])
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
