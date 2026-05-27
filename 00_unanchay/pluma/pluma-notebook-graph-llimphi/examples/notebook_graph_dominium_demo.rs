//! Showcase end-to-end: notebook visual + kernel ECS de dominium +
//! ejecución reactiva + preview de imagen.
//!
//! El notebook arranca con la cadena
//! `world → {seed, params} → tick(0) → tick(50) → {stats, render}`.
//! El usuario puede:
//!
//! - **Mover celdas**: arrastrar la title bar para reubicar.
//! - **Conectar celdas**: arrastrar pin output → pin input; la
//!   conexión dispara automáticamente `run_from(destino)` vía
//!   `apply_connect_and_exec`. Conexiones que cerrarían ciclo se
//!   rechazan en silencio.
//! - **Ejecutar desde una celda**: right-click sobre la title bar de
//!   un nodo emite `Msg::ExecFrom(cell)` y el shell corre
//!   `pluma_notebook_exec::run_from` desde esa celda.
//!
//! Después de cada corrida el shell decodifica el PNG del último
//! `OutputPayload::Image` que produjo cualquier celda `dominium-render`
//! y lo pinta en el sidebar derecho (256×256). Es la pieza que cierra
//! el ciclo "kernel produce imagen → UI la muestra". El status bar
//! abajo muestra el último `RunReport`.
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
use llimphi_ui::llimphi_raster::peniko::{Blob, Image as PenikoImage, ImageFormat};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_nodegraph::{NodegraphMetrics, NodegraphPalette};
use pluma_notebook_core::{CellId, CellKind, Notebook, OutputPayload};
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
    /// El worker decodificó el PNG de una celda render y lo dejó
    /// listo como peniko::Image. `None` = ninguna celda tiene imagen
    /// todavía.
    PreviewReady(Option<PreviewImage>),
}

#[derive(Clone)]
struct PreviewImage {
    cell: CellId,
    image: PenikoImage,
    width: u32,
    height: u32,
}

struct Model {
    /// Compartido con los workers async — el view sólo lee.
    notebook: Arc<Mutex<Notebook>>,
    kernel: Arc<DominiumKernel>,
    last_report: Option<(CellId, usize, usize, usize)>,
    preview: Option<PreviewImage>,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma + dominium · notebook reactivo con preview de imagen"
    }

    fn initial_size() -> (u32, u32) {
        (1400, 820)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let mut nb = Notebook::new();
        let world = nb.push(
            CellKind::Code { language: "dominium-world".into() },
            "32 24",
        );
        let seed = nb.push(
            CellKind::Code { language: "dominium-seed".into() },
            "200 7",
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
        let _stats = nb.push(
            CellKind::Code { language: "dominium-stats".into() },
            "",
        );
        let render = nb.push(
            CellKind::Code { language: "dominium-render".into() },
            "256 192",
        );
        nb.add_dependency(seed, world);
        nb.add_dependency(params, world);
        nb.add_dependency(tick0, seed);
        nb.add_dependency(tick0, params);
        nb.add_dependency(tick50, tick0);
        nb.add_dependency(_stats, tick50);
        nb.add_dependency(render, tick50);

        let model = Model {
            notebook: Arc::new(Mutex::new(nb)),
            kernel: Arc::new(DominiumKernel::new()),
            last_report: None,
            preview: None,
        };

        // Primera corrida al arrancar — así el usuario abre la app con
        // todas las celdas Fresh y el preview listo.
        spawn_exec_from(world, &model.notebook, &model.kernel, handle);

        model
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        let layout = AutoLayout::default();
        match msg {
            Msg::DragCell { id, phase, dx, dy } => {
                let mut nb = m.notebook.lock().expect("notebook envenenado");
                apply_drag(&mut nb, layout, id, phase, dx, dy);
            }
            Msg::Connect { from, to } => {
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
                spawn_exec_from(cell, &m.notebook, &m.kernel, handle);
            }
            Msg::RunFinished {
                from,
                executed,
                failed,
                skipped,
            } => {
                m.last_report = Some((from, executed, failed, skipped));
                // Después de cada corrida, buscamos imágenes nuevas en
                // segundo plano y emitimos PreviewReady.
                spawn_decode_preview(&m.notebook, handle);
            }
            Msg::PreviewReady(p) => {
                m.preview = p;
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

        let preview_panel = preview_panel_view(&model.preview, &theme);

        let canvas_wrapper = View::new(Style {
            size: Size {
                width: Dimension::auto(),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![canvas]);

        let row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .children(vec![canvas_wrapper, preview_panel]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![row, status_bar])
    }
}

fn preview_panel_view(preview: &Option<PreviewImage>, theme: &Theme) -> View<Msg> {
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(
        match preview {
            None => "preview · (sin imagen)".to_string(),
            Some(p) => format!("preview · celda #{} · {}×{} px", p.cell, p.width, p.height),
        },
        11.0,
        theme.fg_muted,
        Alignment::Start,
    );

    let body = match preview {
        Some(p) => View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .image(p.image.clone()),
        None => View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .text_aligned(
            "agregá una celda `dominium-render \"W H\"` para ver el sustrato".to_string(),
            11.0,
            theme.fg_muted,
            Alignment::Center,
        ),
    };

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(320.0_f32),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![header, body])
}

fn spawn_exec_from(
    cell: CellId,
    nb_arc: &Arc<Mutex<Notebook>>,
    k_arc: &Arc<DominiumKernel>,
    handle: &Handle<Msg>,
) {
    let nb_arc = Arc::clone(nb_arc);
    let k_arc = Arc::clone(k_arc);
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

fn spawn_decode_preview(nb_arc: &Arc<Mutex<Notebook>>, handle: &Handle<Msg>) {
    let nb_arc = Arc::clone(nb_arc);
    let h = handle.clone();
    std::thread::spawn(move || {
        let bytes_and_cell = {
            let nb = nb_arc.lock().expect("notebook envenenado");
            // Buscamos la celda con id más alto (la más nueva del orden
            // de definición) que tenga OutputPayload::Image. Para
            // notebooks con varias celdas render, el caller verá la
            // última definida. Es MVP feo — un selector lateral
            // explícito es siguiente paso.
            let mut best: Option<(CellId, Vec<u8>)> = None;
            for c in nb.cells() {
                if let Some(out) = &c.last_output {
                    if let OutputPayload::Image { bytes, .. } = &out.payload {
                        let take = match &best {
                            None => true,
                            Some((id, _)) => c.id > *id,
                        };
                        if take {
                            best = Some((c.id, bytes.clone()));
                        }
                    }
                }
            }
            best
        };
        let preview = bytes_and_cell.and_then(|(cell, bytes)| {
            decode_png(&bytes).map(|(image, width, height)| PreviewImage {
                cell,
                image,
                width,
                height,
            })
        });
        h.dispatch(Msg::PreviewReady(preview));
    });
}

fn decode_png(bytes: &[u8]) -> Option<(PenikoImage, u32, u32)> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = (img.width(), img.height());
    let blob = Blob::from(img.into_raw());
    Some((PenikoImage::new(blob, ImageFormat::Rgba8, w, h), w, h))
}

fn futures_block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime")
        .block_on(fut)
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
