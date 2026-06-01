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
use llimphi_ui::{App, Handle, View};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT};

/// Muestras de CPU guardadas por unidad para dibujar el sparkline.
const SPARK_LEN: usize = 48;
/// Cadencia del polling al Engine.
const POLL: Duration = Duration::from_millis(1000);

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
enum World {
    Linux,
    Wawa,
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
    Switch(World),
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
                .item(MenuItem::new("Linux", "view.linux"))
                .item(MenuItem::new("Wawa", "view.wawa")),
        )
        .menu(Menu::new("Ayuda").item(MenuItem::new("Observa por el contrato Engine", "help.about")))
}

struct Model {
    theme: Theme,
    world: World,
    snapshot: MonitorSnapshot,
    /// Historial de CPU por unidad → sparkline.
    history: HashMap<Ulid, VecDeque<f32>>,
    selected: Option<Ulid>,
    error: Option<String>,
    wawa: Vec<WawaApp>,
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

        // Censo de Wawa en background (no bloquea el arranque).
        handle.spawn(|| Msg::WawaCensus(wawa_census()));

        Model {
            theme: Theme::dark(),
            world: World::Linux,
            snapshot: MonitorSnapshot::default(),
            history: HashMap::new(),
            selected: None,
            error: None,
            wawa: Vec::new(),
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
            Msg::Switch(w) => {
                model.world = w;
                if matches!(w, World::Wawa) && model.wawa.is_empty() {
                    handle.spawn(|| Msg::WawaCensus(wawa_census()));
                }
            }
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
                    "view.linux" => model.world = World::Linux,
                    "view.wawa" => {
                        model.world = World::Wawa;
                        if model.wawa.is_empty() {
                            handle.spawn(|| Msg::WawaCensus(wawa_census()));
                        }
                    }
                    "monitor.refresh" => {
                        let ctx = model.ctx.clone();
                        handle.spawn(move || Msg::Snapshot(ctx.poll()));
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
        let body = match model.world {
            World::Linux => linux_body(model),
            World::Wawa => wawa_body(model),
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
    let snap = &model.snapshot;
    let total = snap.len();
    let running = snap.running();
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

    let mut chips = vec![
        chip(t, "unidades", &total.to_string()),
        chip(t, "vivas", &running.to_string()),
        chip(t, "memoria", &fmt_mem(mem)),
        chip(t, "cpu", &format!("{cpu:.0}%")),
    ];
    if let Some(e) = &model.error {
        chips.push(chip_warn(t, "engine", e));
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
        tab(t, "Linux", model.world == World::Linux, Msg::Switch(World::Linux)),
        tab(t, "Wawa", model.world == World::Wawa, Msg::Switch(World::Wawa)),
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
// Mundo Linux: grilla de tarjetas vivas.
// ---------------------------------------------------------------------------

fn linux_body(model: &Model) -> View<Msg> {
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
