//! Showcase end-to-end: notebook visual + kernel ECS de dominium +
//! ejecución reactiva.
//!
//! El notebook arranca con la cadena
//! `world → seed → params → tick(0) → tick(50) → stats`. El usuario
//! puede:
//!
//! - **Mover celdas**: arrastrar la title bar para reubicar.
//! - **Conectar celdas**: arrastrar pin output → pin input; la
//!   conexión dispara automáticamente `run_from(destino)` vía
//!   `apply_connect_and_exec` para que el cono nuevo se recompute en
//!   el acto. Conexiones que cerrarían ciclo se rechazan en silencio.
//! - **Ejecutar desde una celda**: right-click sobre la title bar de
//!   un nodo emite `Msg::ExecFrom(cell)` y el shell corre
//!   `pluma_notebook_exec::run_from` desde esa celda — útil cuando se
//!   edita la fuente de una celda y se quiere recomputar solo su cono.
//!
//! Status bar abajo: nº de celdas + nº de cables + estado del último
//! reporte (corridas/falladas/saltadas).
//!
//! Corré con: `cargo run -p pluma-notebook-graph-llimphi --example
//! notebook_graph_dominium_demo --release`.

use std::sync::Arc;
use std::sync::Mutex;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_nodegraph::{NodegraphMetrics, NodegraphPalette};
use pluma_notebook_core::{CellId, CellKind, Notebook};
use pluma_notebook_graph_llimphi::{
    apply_drag, notebook_graph_view_with_exec, AutoLayout,
};
use pluma_notebook_kernel_dominium::DominiumKernel;

#[derive(Clone)]
enum Msg {
    DragCell {
        id: CellId,
        #[allow(dead_code)]
        phase: DragPhase,
        dx: f32,
        dy: f32,
    },
    Connect {
        from: CellId,
        to: CellId,
    },
    ExecFrom(CellId),
    RunFinished {
        from: CellId,
        executed: usize,
        failed: usize,
        skipped: usize,
    },
}

struct Model {
    /// Compartido con los workers async — el view sólo lee.
    notebook: Arc<Mutex<Notebook>>,
    kernel: Arc<DominiumKernel>,
    last_report: Option<(CellId, usize, usize, usize)>,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma + dominium · notebook reactivo (right-click = ejecutar desde aquí)"
    }

    fn initial_size() -> (u32, u32) {
        (1280, 800)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let mut nb = Notebook::new();
        let world = nb.push(
            CellKind::Code { language: "dominium-world".into() },
            "32 24",
        );
        let seed = nb.push(
            CellKind::Code { language: "dominium-seed".into() },
            "150 7",
        );
        let params = nb.push(
            CellKind::Code { language: "dominium-param".into() },
            "move_speed=0.4\nsync_rate=0.05",
        );
        let tick0 = nb.push(
            CellKind::Code { language: "dominium-tick".into() },
            "0",
        );
        let tick50 = nb.push(
            CellKind::Code { language: "dominium-tick".into() },
            "50",
        );
        let stats = nb.push(
            CellKind::Code { language: "dominium-stats".into() },
            "",
        );
        nb.add_dependency(seed, world);
        nb.add_dependency(params, world);
        nb.add_dependency(tick0, seed);
        nb.add_dependency(tick0, params);
        nb.add_dependency(tick50, tick0);
        nb.add_dependency(stats, tick50);

        Model {
            notebook: Arc::new(Mutex::new(nb)),
            kernel: Arc::new(DominiumKernel::new()),
            last_report: None,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let m = model;
        let layout = AutoLayout::default();
        match msg {
            Msg::DragCell { id, phase, dx, dy } => {
                let mut nb = m.notebook.lock().expect("notebook envenenado");
                apply_drag(&mut nb, layout, id, phase, dx, dy);
            }
            Msg::Connect { from, to } => {
                // Conexión + auto-exec en un solo paso, en un worker
                // async para no bloquear la UI.
                let nb_arc = Arc::clone(&m.notebook);
                let k_arc = Arc::clone(&m.kernel);
                let h = handle.clone();
                std::thread::spawn(move || {
                    let report = futures_block_on(async move {
                        let mut nb = nb_arc.lock().expect("notebook envenenado");
                        pluma_notebook_graph_llimphi::apply_connect_and_exec(
                            &mut nb, from, to, &*k_arc,
                        )
                        .await
                    });
                    if let Some(rep) = report {
                        h.dispatch(Msg::RunFinished {
                            from: to,
                            executed: rep.executed.len(),
                            failed: rep.failed.len(),
                            skipped: rep.skipped.len(),
                        });
                    }
                });
            }
            Msg::ExecFrom(cell) => {
                let nb_arc = Arc::clone(&m.notebook);
                let k_arc = Arc::clone(&m.kernel);
                let h = handle.clone();
                std::thread::spawn(move || {
                    let report = futures_block_on(async move {
                        let mut nb = nb_arc.lock().expect("notebook envenenado");
                        pluma_notebook_graph_llimphi::exec_from(&mut nb, &*k_arc, cell).await
                    });
                    if let Some(rep) = report {
                        h.dispatch(Msg::RunFinished {
                            from: cell,
                            executed: rep.executed.len(),
                            failed: rep.failed.len(),
                            skipped: rep.skipped.len(),
                        });
                    }
                });
            }
            Msg::RunFinished {
                from,
                executed,
                failed,
                skipped,
            } => {
                let mut m = m;
                m.last_report = Some((from, executed, failed, skipped));
                return m;
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = NodegraphPalette::from_theme(&theme);
        let metrics = NodegraphMetrics::default();
        let layout = AutoLayout::default();
        let nb = model.notebook.lock().expect("notebook envenenado");
        let canvas = notebook_graph_view_with_exec(
            &nb,
            layout,
            &palette,
            &metrics,
            |id, phase, dx, dy| Some(Msg::DragCell { id, phase, dx, dy }),
            |from, to| Some(Msg::Connect { from, to }),
            |id| Some(Msg::ExecFrom(id)),
        );

        let n_cells = nb.len();
        let n_wires: usize = nb.cells().iter().map(|c| c.depends_on.len()).sum();
        drop(nb);

        let status_text = match model.last_report {
            None => format!("celdas: {n_cells}  ·  cables: {n_wires}  ·  right-click = ejecutar desde aquí"),
            Some((from, e, f, s)) => format!(
                "celdas: {n_cells}  ·  cables: {n_wires}  ·  última corrida desde #{from}: ejec={e} fall={f} skip={s}"
            ),
        };
        let status_bar = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(24.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(12.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(status_text, 11.0, theme.fg_muted, Alignment::Start);

        let canvas_wrapper = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![canvas]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![canvas_wrapper, status_bar])
    }
}

fn futures_block_on<F: std::future::Future>(fut: F) -> F::Output {
    // tokio en current_thread alcanza: los ejecutores dominium son
    // CPU-bound dentro del Mutex, no hay awaits reales que requieran
    // multi-thread.
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
        .block_on(fut)
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
