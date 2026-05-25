//! `pluma-notebook-llimphi` — visor read-only de notebooks sobre Llimphi.
//!
//! Uso:
//!   pluma-notebook-llimphi [ruta.pluma-nb]
//!
//! Sin argumento, abre un notebook demo con las celdas posicionadas en
//! canvas para mostrar el modo espacial.
//!
//! Dos modos de presentación:
//!   - **Lineal**: cards apilados verticalmente (notebook tradicional).
//!     Se elige cuando ninguna celda del notebook tiene `position`.
//!   - **Canvas**: cards absolutamente posicionados en (x, y) según
//!     `Cell::position`, con conectores S-codo entre cada celda y sus
//!     dependencias. Se elige automáticamente si al menos una celda
//!     tiene `position` definida.
//!
//! MVP: sólo render. Sin edición de fuentes, sin ejecución contra kernel,
//! sin scroll/pan/zoom. Edición → integrar `pluma-editor-llimphi`.
//! Ejecución → cablear `pluma-notebook-exec::{run_all, run_from}`.

use std::env;
use std::path::PathBuf;

use async_trait::async_trait;
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};
use pluma_notebook_kernel_python::PythonKernel;
use pluma_notebook_kernel_wasm::WasmKernel;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use pluma_notebook_core::{
    Cell, CellId, CellKind, CellOutput, CellState, Notebook, Position as CanvasPos,
};

#[derive(Clone)]
enum Msg {
    /// Desplaza el viewport del canvas por `(dx, dy)`.
    PanBy(f32, f32),
    /// Mueve una celda en el canvas por `(dx, dy)` (delta desde el evento
    /// anterior — no acumulado desde el press).
    MoveCell { id: CellId, dx: f32, dy: f32 },
    /// El usuario pidió ejecutar desde una celda — corre `run_from` en un
    /// thread aparte y dispatcha `RunCompleted` al volver.
    RunFrom(CellId),
    /// El kernel terminó: reemplaza el notebook por la versión con los
    /// estados actualizados.
    RunCompleted(Notebook),
    /// Entra modo edición sobre una celda. Carga el `source` actual en
    /// el TextInput.
    StartEdit(CellId),
    /// Tecla aplicada al input en edición.
    EditKey(KeyEvent),
    /// Guarda el draft del input como nuevo `source` (vía `set_source`,
    /// que marca stale + propaga) y sale del modo edición.
    CommitEdit,
    /// Descarta el draft y sale del modo edición.
    CancelEdit,
}

/// Estado de una celda en edición. Single-line por la limitación del
/// `llimphi-widget-text-input`; las newlines se preservan en el state
/// pero no se muestran como nuevas líneas.
struct EditState {
    id: CellId,
    input: TextInputState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Linear,
    Canvas,
}

struct Model {
    notebook: Notebook,
    mode: Mode,
    /// Offset del viewport en modo canvas — el usuario lo cambia
    /// arrastrando el fondo o con la rueda del mouse. Se suma a cada
    /// `Cell::position` al render.
    viewport: (f32, f32),
    /// Celda raíz de una corrida en curso (si la hay). Bloquea nuevos
    /// pedidos hasta que el thread devuelva `RunCompleted`.
    running_from: Option<CellId>,
    /// Estado de la celda en edición, si la hay.
    editing: Option<EditState>,
    /// Archivo de origen (None = demo embebido).
    source: Option<PathBuf>,
    /// Mensaje de error si load falló — se muestra en el header.
    load_error: Option<String>,
}

struct Viewer;

impl App for Viewer {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pluma-notebook"
    }

    fn initial_size() -> (u32, u32) {
        (980, 760)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let arg = env::args().nth(1).map(PathBuf::from);
        let (notebook, source, load_error) = match arg {
            None => (demo_notebook(), None, None),
            Some(p) => match pluma_notebook_store::load(&p) {
                Ok(nb) => (nb, Some(p), None),
                Err(e) => (Notebook::new(), Some(p), Some(e.to_string())),
            },
        };
        let mode = if notebook.cells().iter().any(|c| c.position.is_some()) {
            Mode::Canvas
        } else {
            Mode::Linear
        };
        Model {
            notebook,
            mode,
            viewport: (0.0, 0.0),
            running_from: None,
            editing: None,
            source,
            load_error,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::PanBy(dx, dy) => Model {
                viewport: (model.viewport.0 + dx, model.viewport.1 + dy),
                ..model
            },
            Msg::MoveCell { id, dx, dy } => {
                let mut nb = model.notebook;
                if let Some(p) = nb.position(id) {
                    nb.set_position(id, Some(CanvasPos::new(p.x + dx, p.y + dy)));
                }
                Model { notebook: nb, ..model }
            }
            Msg::RunFrom(id) => {
                // Ya hay una corrida en curso → ignoramos el pedido.
                if model.running_from.is_some() {
                    return model;
                }
                let mut nb = model.notebook.clone();
                handle.spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("tokio runtime");
                    let kernel = MultiKernel::new();
                    let _ = rt.block_on(pluma_notebook_exec::run_from(&mut nb, &kernel, id));
                    Msg::RunCompleted(nb)
                });
                Model { running_from: Some(id), ..model }
            }
            Msg::RunCompleted(nb) => Model {
                notebook: nb,
                running_from: None,
                ..model
            },
            Msg::StartEdit(id) => {
                let Some(cell) = model.notebook.cell(id) else { return model };
                let mut input = TextInputState::new();
                input.set_text(&cell.source);
                Model { editing: Some(EditState { id, input }), ..model }
            }
            Msg::EditKey(ev) => {
                let mut model = model;
                if let Some(edit) = model.editing.as_mut() {
                    edit.input.apply_key(&ev);
                }
                model
            }
            Msg::CommitEdit => {
                let mut model = model;
                if let Some(edit) = model.editing.take() {
                    let _ = model.notebook.set_source(edit.id, edit.input.text());
                    // set_source ya marca Stale + propaga; el último output
                    // queda visible para comparar.
                }
                model
            }
            Msg::CancelEdit => Model { editing: None, ..model },
        }
    }

    fn on_key(model: &Self::Model, event: &KeyEvent) -> Option<Self::Msg> {
        if model.editing.is_none() {
            return None;
        }
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Enter) => Some(Msg::CommitEdit),
            Key::Named(NamedKey::Escape) => Some(Msg::CancelEdit),
            _ => Some(Msg::EditKey(event.clone())),
        }
    }

    fn on_wheel(
        model: &Self::Model,
        delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: Modifiers,
    ) -> Option<Self::Msg> {
        // Sólo paneamos en modo canvas; en lineal dejamos el wheel para
        // un futuro scroll vertical interno.
        if model.mode != Mode::Canvas {
            return None;
        }
        // Una "línea" del wheel = 16 px de pan.
        const STEP: f32 = 16.0;
        let dx = delta.x * STEP;
        let dy = -delta.y * STEP; // wheel arriba mueve el contenido hacia abajo.
        if dx == 0.0 && dy == 0.0 {
            None
        } else {
            Some(Msg::PanBy(dx, dy))
        }
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = Palette::from_theme(&theme);

        let header = header_bar(model, &palette);
        let body = match model.mode {
            Mode::Linear => linear_view(&model.notebook, &palette),
            Mode::Canvas => canvas_view(
                &model.notebook,
                model.viewport,
                model.editing.as_ref(),
                &palette,
            ),
        };

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg)
        .children(vec![header, body])
    }
}

/// Paleta semántica del visor — sale del Theme y se pasa por las funciones
/// de render para no leer `Theme::dark()` desde cada una.
struct Palette {
    bg: Color,
    bg_panel: Color,
    bg_card: Color,
    fg_text: Color,
    fg_muted: Color,
    fg_error: Color,
    accent_stale: Color,
    accent_failed: Color,
    accent_fresh: Color,
    edge: Color,
}

impl Palette {
    fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_app,
            bg_panel: t.bg_panel,
            bg_card: t.bg_panel_alt,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
            accent_stale: t.fg_muted,
            accent_failed: t.fg_destructive,
            accent_fresh: t.fg_text,
            edge: t.accent,
        }
    }
}

fn header_bar(model: &Model, palette: &Palette) -> View<Msg> {
    let origen = match (&model.source, &model.load_error) {
        (_, Some(err)) => format!("error de carga: {err}"),
        (Some(p), None) => p.display().to_string(),
        (None, None) => "(demo embebido — pasá una ruta .pluma-nb para abrir un archivo)".to_string(),
    };
    let digest = model
        .notebook
        .notebook_digest()
        .map(|d| short_hex(&d))
        .unwrap_or_else(|| "—(ciclo)".to_string());
    let modo = match model.mode {
        Mode::Linear => "lineal".to_string(),
        Mode::Canvas => format!(
            "canvas (viewport {:+.0},{:+.0})",
            model.viewport.0, model.viewport.1
        ),
    };
    let running = model
        .running_from
        .map(|id| format!(" · ejecutando #{id}…"))
        .unwrap_or_default();
    let texto = format!(
        "pluma-notebook · {} celdas · modo {} · digest {}{} · {}",
        model.notebook.len(),
        modo,
        digest,
        running,
        origen,
    );
    let color = if model.load_error.is_some() { palette.fg_error } else { palette.fg_muted };

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(texto, 11.0, color, Alignment::Start)
}

// ---------------------------------------------------------------------
// Modo lineal — stack vertical, lo de siempre.
// ---------------------------------------------------------------------

fn linear_view(nb: &Notebook, palette: &Palette) -> View<Msg> {
    let cards: Vec<View<Msg>> = nb.cells().iter().map(|c| linear_card(c, palette)).collect();
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(cards)
}

fn linear_card(cell: &Cell, palette: &Palette) -> View<Msg> {
    let height = linear_body_height(&cell.source) + 30.0;
    card_with_height(
        cell,
        palette,
        Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: length(height) },
            margin: Rect {
                left: length(0.0_f32),
                right: length(0.0_f32),
                top: length(0.0_f32),
                bottom: length(8.0_f32),
            },
            ..Default::default()
        },
        linear_body_height(&cell.source),
    )
}

const LINEAR_MAX_BODY_LINES: usize = 8;
const LINEAR_MAX_BODY_CHARS: usize = 400;

fn linear_body_height(source: &str) -> f32 {
    let lines = source.lines().count().min(LINEAR_MAX_BODY_LINES).max(1);
    16.0 * lines as f32 + 12.0
}

// ---------------------------------------------------------------------
// Modo canvas — cada celda en su (x, y) + conectores S-codo del DAG.
// ---------------------------------------------------------------------

/// Tamaño fijo del card en canvas — el alto del body sale del card, no
/// del texto, para que los conectores sean estables.
const CANVAS_CARD_W: f32 = 240.0;
const CANVAS_CARD_H: f32 = 112.0;
const CANVAS_BODY_LINES_VISIBLE: usize = 3;
const CANVAS_HEADER_H: f32 = 18.0;
const CANVAS_FOOTER_H: f32 = 16.0;
const CANVAS_BODY_H: f32 = CANVAS_CARD_H - CANVAS_HEADER_H - CANVAS_FOOTER_H;

fn canvas_view(
    nb: &Notebook,
    viewport: (f32, f32),
    editing: Option<&EditState>,
    palette: &Palette,
) -> View<Msg> {
    let (vx, vy) = viewport;
    let mut children: Vec<View<Msg>> = Vec::new();

    // Aristas primero (capa de fondo) — del prerrequisito al dependiente.
    for cell in nb.cells() {
        let Some(child_pos) = cell.position else { continue };
        for dep_id in &cell.depends_on {
            let Some(dep) = nb.cell(*dep_id) else { continue };
            let Some(dep_pos) = dep.position else { continue };
            let x1 = vx + dep_pos.x + CANVAS_CARD_W * 0.5;
            let y1 = vy + dep_pos.y + CANVAS_CARD_H;
            let x2 = vx + child_pos.x + CANVAS_CARD_W * 0.5;
            let y2 = vy + child_pos.y;
            children.extend(edge_segments(x1, y1, x2, y2, palette.edge));
        }
    }

    // Cards encima — draggables (mueven Cell::position).
    for cell in nb.cells() {
        let Some(pos) = cell.position else { continue };
        let edit = editing.filter(|e| e.id == cell.id);
        children.push(canvas_card(cell, edit, palette, vx + pos.x, vy + pos.y));
    }

    let huerfanas = nb.cells().iter().filter(|c| c.position.is_none()).count();
    if huerfanas > 0 {
        children.push(orphan_notice(huerfanas, palette));
    }

    View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .draggable(|phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::PanBy(dx, dy)),
        DragPhase::End => None,
    })
    .children(children)
}

fn canvas_card(
    cell: &Cell,
    edit: Option<&EditState>,
    palette: &Palette,
    x: f32,
    y: f32,
) -> View<Msg> {
    let id = cell.id;
    let (header, body) = card_header_body(cell, palette, CANVAS_BODY_H);
    let footer = output_footer(cell.last_output.as_ref(), palette);
    let run_button = run_button_view(id, palette);
    let edit_button = edit_button_view(id, edit.is_some(), palette);

    // En edición el body se reemplaza por el text input.
    let body = match edit {
        None => body,
        Some(es) => edit_input_view(&es.input, palette),
    };

    // En edición la card no debe ser draggable (interfiere con foco
    // del input) ni renderizar conectores reactivos al delta.
    let mut wrapper = View::new(Style {
        flex_direction: FlexDirection::Column,
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(CANVAS_CARD_W), height: length(CANVAS_CARD_H) },
        ..Default::default()
    })
    .fill(palette.bg_card)
    .clip(true);
    if edit.is_none() {
        wrapper = wrapper.draggable(move |phase, dx, dy| match phase {
            DragPhase::Move => Some(Msg::MoveCell { id, dx, dy }),
            DragPhase::End => None,
        });
    }
    wrapper.children(vec![header, body, footer, edit_button, run_button])
}

fn edit_input_view(input: &TextInputState, palette: &Palette) -> View<Msg> {
    // El text-input asume su propia paleta — derivamos del theme dark
    // para que combine con el resto del visor.
    let tp = TextInputPalette::from_theme(&Theme::dark());
    let _ = palette; // (reservado por si después pintamos un borde activo)
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(CANVAS_BODY_H) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(vec![text_input_view(input, "fuente…", true, &tp, Msg::CancelEdit)])
}

fn output_footer(out: Option<&CellOutput>, palette: &Palette) -> View<Msg> {
    let (text, color) = match out {
        None => ("∅ sin output".to_string(), palette.fg_muted),
        Some(o) => (format_output(o), palette.accent_fresh),
    };
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(CANVAS_FOOTER_H) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .text_aligned(text, 10.0, color, Alignment::Start)
}

/// Una línea legible del output. Prioriza `value`; cae al primer renglón
/// de stdout si no hay value; muestra `[port_kind]` como prefijo del tipo.
fn format_output(o: &CellOutput) -> String {
    let port = o.payload.port_kind();
    if let Some(v) = &o.value {
        return format!("→[{}] {}", port, truncate_line(v, 28));
    }
    if !o.stdout.is_empty() {
        let line = o.stdout.lines().next().unwrap_or("");
        return format!("→[{}] {}", port, truncate_line(line, 28));
    }
    format!("→[{}]", port)
}

fn truncate_line(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

const RUN_BTN_SIZE: f32 = 18.0;

fn run_button_view(id: CellId, palette: &Palette) -> View<Msg> {
    // Esquina superior derecha.
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(2.0_f32),
            right: length(4.0_f32),
            bottom: auto(),
        },
        size: Size { width: length(RUN_BTN_SIZE), height: length(RUN_BTN_SIZE) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.edge)
    .hover_fill(palette.accent_fresh)
    .on_click(Msg::RunFrom(id))
    .text_aligned("▶", 10.0, palette.bg, Alignment::Center)
}

fn edit_button_view(id: CellId, editing: bool, palette: &Palette) -> View<Msg> {
    // A la izquierda del botón ▶. Cuando hay edición activa, este
    // mismo botón pasa a representar "commit" (✓).
    let (glyph, msg) = if editing {
        ("✓", Msg::CommitEdit)
    } else {
        ("✎", Msg::StartEdit(id))
    };
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: auto(),
            top: length(2.0_f32),
            right: length(26.0_f32),
            bottom: auto(),
        },
        size: Size { width: length(RUN_BTN_SIZE), height: length(RUN_BTN_SIZE) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .hover_fill(palette.accent_fresh)
    .on_click(msg)
    .text_aligned(glyph, 10.0, palette.fg_text, Alignment::Center)
}

fn orphan_notice(n: usize, palette: &Palette) -> View<Msg> {
    let texto = format!(
        "{n} celda(s) sin posición — no se muestran en canvas. Asigná `Cell::position` para incluirlas."
    );
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(12.0_f32),
            top: length(8.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(560.0_f32), height: length(18.0_f32) },
        ..Default::default()
    })
    .text_aligned(texto, 10.0, palette.fg_muted, Alignment::Start)
}

// ---------------------------------------------------------------------
// Conectores S-codo — copia del patrón de pluma-editor-llimphi.
// ---------------------------------------------------------------------

fn edge_segments(x1: f32, y1: f32, x2: f32, y2: f32, color: Color) -> Vec<View<Msg>> {
    let stroke = 1.6f32;
    let half = stroke * 0.5;
    let mid_y = (y1 + y2) * 0.5;
    let mut out: Vec<View<Msg>> = Vec::with_capacity(3);

    out.push(line_view(x1 - half, y1, stroke, (mid_y - y1).abs().max(stroke), color));
    if (x2 - x1).abs() > stroke {
        let (xl, xr) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
        out.push(line_view(xl - half, mid_y - half, (xr - xl) + stroke, stroke, color));
    }
    out.push(line_view(x2 - half, mid_y, stroke, (y2 - mid_y).abs().max(stroke), color));
    out
}

fn line_view(x: f32, y: f32, w: f32, h: f32, color: Color) -> View<Msg> {
    View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(x),
            top: length(y),
            right: auto(),
            bottom: auto(),
        },
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .fill(color)
}

// ---------------------------------------------------------------------
// Card compartido — usado por ambos modos. El style del wrapper lo pone
// cada modo (lineal = flex column; canvas = absoluto en (x,y)).
// ---------------------------------------------------------------------

fn card_with_height(cell: &Cell, palette: &Palette, wrapper: Style, body_h: f32) -> View<Msg> {
    let (header, body) = card_header_body(cell, palette, body_h);
    View::new(wrapper).fill(palette.bg_card).clip(true).children(vec![header, body])
}

fn card_header_body(cell: &Cell, palette: &Palette, body_h: f32) -> (View<Msg>, View<Msg>) {
    let header_text = format!(
        "[{}] #{}  ·  {}",
        kind_label(&cell.kind),
        cell.id,
        state_label(cell.state)
    );
    let header_color = match cell.state {
        CellState::Fresh => palette.accent_fresh,
        CellState::Stale => palette.accent_stale,
        CellState::Failed => palette.accent_failed,
    };

    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            // Espacio reservado para los botones ✎ y ▶ en modo canvas.
            right: length(50.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, header_color, Alignment::Start);

    let body = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(body_h) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(
        truncate_source(&cell.source, CANVAS_BODY_LINES_VISIBLE.max(2)),
        12.0,
        palette.fg_text,
        Alignment::Start,
    );

    (header, body)
}

fn kind_label(k: &CellKind) -> String {
    match k {
        CellKind::Markdown => "markdown".into(),
        CellKind::Code { language } => format!("code:{language}"),
        CellKind::Embed { module } => format!("embed:{module}"),
    }
}

fn state_label(s: CellState) -> &'static str {
    match s {
        CellState::Fresh => "fresh",
        CellState::Stale => "stale",
        CellState::Failed => "failed",
    }
}

fn truncate_source(s: &str, max_lines: usize) -> String {
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i >= max_lines || out.len() + line.len() + 1 > LINEAR_MAX_BODY_CHARS {
            out.push_str("\n…");
            break;
        }
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.is_empty() {
        out.push_str(s);
    }
    out
}

fn short_hex(d: &[u8; 32]) -> String {
    d[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Notebook embebido — modo canvas: cuatro celdas con posición en (x, y)
/// para que el binario sin argumento muestre el modo espacial.
fn demo_notebook() -> Notebook {
    use pluma_notebook_core::Position as P;

    let mut nb = Notebook::new();
    let intro = nb.push(
        CellKind::Markdown,
        "Demo · ✎ edita, Enter commit, Esc cancela. ▶ corre run_from.",
    );
    let datos = nb.push(
        CellKind::Code { language: "wat".into() },
        "(module (func (export \"main\") (result i32) i32.const 21))",
    );
    let media = nb.push(
        CellKind::Code { language: "wat".into() },
        "(module (func (export \"main\") (result i32) i32.const 42))",
    );
    let py = nb.push(
        CellKind::Code { language: "python".into() },
        "sum(range(1, 11))",
    );
    let grafico = nb.push(
        CellKind::Embed { module: "pineal".into() },
        "barras: kilos por semana",
    );
    nb.add_dependency(media, datos);
    nb.add_dependency(grafico, datos);
    nb.add_dependency(grafico, media);
    nb.add_dependency(grafico, py);

    // Layout: intro arriba, datos al centro, media+python como hijos a
    // izquierda y centro, gráfico a la derecha como sink de los tres.
    nb.set_position(intro, Some(P::new(40.0, 40.0)));
    nb.set_position(datos, Some(P::new(40.0, 170.0)));
    nb.set_position(media, Some(P::new(40.0, 320.0)));
    nb.set_position(py, Some(P::new(310.0, 170.0)));
    nb.set_position(grafico, Some(P::new(310.0, 320.0)));

    nb
}

/// Dispatcher por `language` — la pieza que junta wasmi + RustPython
/// detrás del mismo trait `Kernel`. El visor delega acá y deja que cada
/// celda elija su intérprete con un string.
struct MultiKernel {
    wasm: WasmKernel,
    python: PythonKernel,
}

impl MultiKernel {
    fn new() -> Self {
        Self { wasm: WasmKernel::new(), python: PythonKernel::new() }
    }
}

#[async_trait]
impl Kernel for MultiKernel {
    async fn execute(&self, source: &str, language: &str) -> Result<KernelOutput, KernelError> {
        match language {
            "wasm" | "wat" => self.wasm.execute(source, language).await,
            "python" | "py" => self.python.execute(source, language).await,
            other => Err(KernelError::Runtime(format!(
                "ningún kernel registrado para '{other}' (disponibles: wasm/wat, python/py)"
            ))),
        }
    }
}

fn main() {
    llimphi_ui::run::<Viewer>();
}
