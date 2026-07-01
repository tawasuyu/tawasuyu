//! `sandokan-monitor` — el monitor de procesos de tawasuyu sobre Llimphi.
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

use std::sync::Arc;
use std::time::{Duration, Instant};

use llimphi_theme::motion;
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, FlexDirection, Size, Style},
};
use llimphi_widget_menubar::{menubar_overlay, menubar_view, MenuBarSpec, DEFAULT_HEIGHT};
use llimphi_widget_toast::{toast_stack_view, Toast};

// `pub(crate)`: el example `pantallazo_sandokan` incluye este archivo por
// `#[path]` y necesita nombrar `Sig`/`Scan` (variantes de `Msg`).
pub(crate) mod procfs;
mod treemap;

mod engine;
mod modelo;
mod sistema;
mod view_mapa;
mod view_sistema;
mod view_unidades;
mod view_wawa;
mod widgets;

// `pub(crate) use` (no `use` a secas) para que el pantallazo headless —que
// incluye este `main.rs` por `#[path]` como módulo `app`— pueda importar
// `app::{Model, Msg, …, map_body}`. La visibilidad pub(crate) vale igual al
// compilar el binario real y al compilarlo dentro del example.
pub(crate) use engine::{build_ctx, seed_demo, wawa_census};
pub(crate) use modelo::{
    build_menu, Model, Msg, Sort, SysProc, Tab, WawaApp, POLL, SPARK_LEN, SYS_ROWS,
};
use sistema::{ensure_visible, ingest_system, render_list, sort_system, switch_tab, sys_move};
pub(crate) use view_mapa::map_body;
pub(crate) use view_sistema::system_body;
pub(crate) use view_unidades::units_body;
pub(crate) use view_wawa::wawa_body;
use widgets::{chip, chip_warn, fmt_mem, pad, tab as tab_btn};

/// Cuánto vive un toast antes de auto-descartarse.
const TOAST_TTL: Duration = Duration::from_secs(4);
/// Viewport de referencia para apilar overlays (igual que `menu_spec`).
const VIEWPORT: (f32, f32) = (900.0, 600.0);

/// Hash estable de una cadena → `key` para animaciones implícitas (la misma
/// id/escena produce siempre la misma key entre rebuilds, así la entrada anima
/// una sola vez).
pub(crate) fn key_of(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `key` estable de la pestaña activa — cambia sólo al conmutar de vista, lo
/// que dispara la transición de entrada del cuerpo.
fn tab_key(tab: Tab) -> u64 {
    match tab {
        Tab::System => 1,
        Tab::Map => 2,
        Tab::Units => 3,
        Tab::Wawa => 4,
    }
}

/// Empuja un toast al stack y programa su expiración (`TOAST_TTL`).
fn push_toast(model: &mut Model, handle: &Handle<Msg>, make: impl FnOnce(u64) -> Toast) {
    let id = model.next_toast;
    model.next_toast += 1;
    model.toasts.push(make(id));
    handle.spawn(move || {
        std::thread::sleep(TOAST_TTL);
        Msg::ToastExpire(id)
    });
}

struct Monitor;

// ---------------------------------------------------------------------------
// Spec de la barra de menú.
// ---------------------------------------------------------------------------

pub(crate) fn menu_spec(model: &Model) -> MenuBarSpec<'_, Msg> {
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
// Cabecera y pestañas.
// ---------------------------------------------------------------------------

pub(crate) fn header(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut chips = match model.tab {
        Tab::System | Tab::Map => {
            let cpu: f32 = model.system.iter().map(|p| p.cpu_pct).sum();
            let rss: u64 = model.system.iter().map(|p| p.rss_kb).sum::<u64>() * 1024;
            vec![
                chip(t, &rimay_localize::t("sandokan-mon-chip-procesos"), &model.system.len().to_string()),
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
                chip(t, &rimay_localize::t("sandokan-mon-chip-unidades"), &snap.len().to_string()),
                chip(t, &rimay_localize::t("sandokan-mon-chip-vivas"), &snap.running().to_string()),
                chip(t, &rimay_localize::t("sandokan-mon-chip-memoria"), &fmt_mem(mem)),
                chip(t, "cpu", &format!("{cpu:.0}%")),
            ]
        }
        Tab::Wawa => vec![chip(t, &rimay_localize::t("sandokan-mon-chip-apps-wasm"), &model.wawa.len().to_string())],
    };
    if let Some(e) = &model.error {
        chips.push(chip_warn(t, &rimay_localize::t("sandokan-mon-chip-aviso"), e));
    }

    use llimphi_ui::llimphi_layout::taffy::prelude::{length, FlexDirection as FD, Size as S, Style as St};
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};

    View::new(St {
        flex_direction: FD::Row,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: pad(16.0, 12.0),
        gap: S {
            width: length(8.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(vec![
        View::new(St::default()).text("Sandokan · Monitor", 17.0, t.fg_text),
        View::new(St {
            flex_direction: FD::Row,
            align_items: Some(AlignItems::Center),
            gap: S {
                width: length(8.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .children(chips),
    ])
}

pub(crate) fn tabs(model: &Model) -> View<Msg> {
    let t = &model.theme;
    use llimphi_ui::llimphi_layout::taffy::prelude::{length, Size as S, Style as St};

    View::new(St {
        flex_direction: FlexDirection::Row,
        gap: S {
            width: length(6.0_f32),
            height: length(6.0_f32),
        },
        padding: llimphi_ui::llimphi_layout::taffy::geometry::Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(t.bg_panel)
    .children(vec![
        tab_btn(t, &rimay_localize::t("sandokan-mon-tab-system"), model.tab == Tab::System, Msg::Switch(Tab::System)),
        tab_btn(t, &rimay_localize::t("sandokan-mon-tab-map"), model.tab == Tab::Map, Msg::Switch(Tab::Map)),
        tab_btn(t, &rimay_localize::t("sandokan-mon-tab-units"), model.tab == Tab::Units, Msg::Switch(Tab::Units)),
        // "Wawa" es nombre propio del SO (marca) — no se traduce.
        tab_btn(t, "Wawa", model.tab == Tab::Wawa, Msg::Switch(Tab::Wawa)),
    ])
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
        // Carga los catálogos Fluent (es/en/qu) una sola vez. Idempotente.
        rimay_localize::init();
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
            theme: llimphi_theme::Theme::dark(),
            tab: Tab::System,
            snapshot: sandokan_monitor_core::MonitorSnapshot::default(),
            history: std::collections::HashMap::new(),
            selected: None,
            error: None,
            wawa: Vec::new(),
            system: Vec::new(),
            sys_sel: None,
            sys_sort: modelo::Sort::Cpu,
            sys_scroll: 0,
            sys_tree: true,
            collapsed: std::collections::HashSet::new(),
            sys_filter: String::new(),
            filter_mode: false,
            map_cpu: false,
            map_root: None,
            last_map_click: None,
            mem_total_kb: 0,
            mem_avail_kb: 0,
            core_hist: Vec::new(),
            core_ids: Vec::new(),
            mem_hist: std::collections::VecDeque::new(),
            prev_core: Vec::new(),
            prev_proc: std::collections::HashMap::new(),
            prev_total: 0,
            menu: build_menu(),
            menu_open: None,
            ctx,
            toasts: Vec::new(),
            next_toast: 0,
        }
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Snapshot(Ok(snap)) => {
                // Empuja la muestra de CPU al historial de cada unidad viva.
                let mut alive = std::collections::HashMap::new();
                for u in &snap.units {
                    let cpu = u.telemetry.as_ref().map(|t| t.cpu_pct as f32).unwrap_or(0.0);
                    let buf = model
                        .history
                        .remove(&u.card_id)
                        .unwrap_or_else(|| std::collections::VecDeque::with_capacity(SPARK_LEN));
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
                let now = std::time::Instant::now();
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
                    let msg = rimay_localize::t_args(
                        "sandokan-mon-toast-signal-err",
                        &[("pid", pid.to_string().into()), ("err", e.to_string().into())],
                    );
                    model.error = Some(msg.clone());
                    push_toast(&mut model, handle, |id| Toast::error(id, msg, TOAST_TTL));
                } else {
                    model.error = None;
                    push_toast(&mut model, handle, |id| {
                        Toast::info(
                            id,
                            rimay_localize::t_args(
                                "sandokan-mon-toast-signal-sent",
                                &[("pid", pid.to_string().into())],
                            ),
                            TOAST_TTL,
                        )
                    });
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
                push_toast(&mut model, handle, |tid| {
                    Toast::info(tid, rimay_localize::t("sandokan-mon-toast-stopping"), TOAST_TTL)
                });
                model.selected = None;
            }
            Msg::Kill(id) => {
                let ctx = model.ctx.clone();
                handle.spawn(move || {
                    let _ = ctx.rt.block_on(ctx.engine.stop(id, Duration::ZERO));
                    Msg::Snapshot(ctx.poll())
                });
                push_toast(&mut model, handle, |tid| {
                    Toast::warning(tid, rimay_localize::t("sandokan-mon-toast-killing"), TOAST_TTL)
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
            Msg::ToastExpire(id) => model.toasts.retain(|t| t.id != id),
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let t = &model.theme;
        let inner = match model.tab {
            Tab::System => system_body(model),
            Tab::Map => map_body(model),
            Tab::Units => units_body(model),
            Tab::Wawa => wawa_body(model),
        };

        // Transición de escena: al conmutar de pestaña la `tab_key` cambia y el
        // cuerpo entra con fade + slide-up suave en vez de saltar de golpe.
        let body = View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .children(vec![inner])
        .animated_enter_from(tab_key(model.tab), motion::SLOW, Affine::translate((0.0, 24.0)));

        let root = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(t.bg_app)
        .children(vec![
            menubar_view(&menu_spec(model)),
            header(model),
            tabs(model),
            body,
        ]);

        // Overlay de toasts (bottom-right). Sólo los que aún viven.
        let now = Instant::now();
        let alive: Vec<Toast> = model.toasts.iter().filter(|t| t.is_alive(now)).cloned().collect();
        if alive.is_empty() {
            root
        } else {
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: percent(1.0_f32),
                },
                ..Default::default()
            })
            .children(vec![root, toast_stack_view(&alive, VIEWPORT, Msg::ToastExpire)])
        }
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
                return model.sys_sel.map(|p| Msg::Signal(p, procfs::Sig::Term));
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

fn main() {
    llimphi_ui::run::<Monitor>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use modelo::SysProc;
    use sistema::{flatten_tree, proc_matches};
    use std::collections::HashSet;

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
