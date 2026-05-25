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

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Position, Rect, Size, Style},
    AlignItems,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, Modifiers, View, WheelDelta};
use pluma_notebook_core::{Cell, CellId, CellKind, CellState, Notebook, Position as CanvasPos};

#[derive(Clone)]
enum Msg {
    /// Desplaza el viewport del canvas por `(dx, dy)`.
    PanBy(f32, f32),
    /// Mueve una celda en el canvas por `(dx, dy)` (delta desde el evento
    /// anterior — no acumulado desde el press).
    MoveCell { id: CellId, dx: f32, dy: f32 },
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
        Model { notebook, mode, viewport: (0.0, 0.0), source, load_error }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
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
            Mode::Canvas => canvas_view(&model.notebook, model.viewport, &palette),
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
    let texto = format!(
        "pluma-notebook · {} celdas · modo {} · digest {} · {}",
        model.notebook.len(),
        modo,
        digest,
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
const CANVAS_CARD_H: f32 = 96.0;
const CANVAS_BODY_LINES_VISIBLE: usize = 4;

fn canvas_view(nb: &Notebook, viewport: (f32, f32), palette: &Palette) -> View<Msg> {
    let (vx, vy) = viewport;
    let mut children: Vec<View<Msg>> = Vec::new();

    // Aristas primero (capa de fondo) — del prerrequisito al dependiente.
    for cell in nb.cells() {
        let Some(child_pos) = cell.position else { continue };
        for dep_id in &cell.depends_on {
            let Some(dep) = nb.cell(*dep_id) else { continue };
            let Some(dep_pos) = dep.position else { continue };
            // Centro inferior del dep → centro superior del dependiente.
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
        children.push(canvas_card(cell, palette, vx + pos.x, vy + pos.y));
    }

    // Aviso si hay celdas sin posición — las omitimos en canvas.
    let huerfanas = nb.cells().iter().filter(|c| c.position.is_none()).count();
    if huerfanas > 0 {
        children.push(orphan_notice(huerfanas, palette));
    }

    // El contenedor canvas es draggable: pan del viewport. Las cards
    // están encima y atrapan su propio drag — el runtime hace hit-test
    // sobre el child más arriba, así el fondo sólo dispara cuando se
    // arrastra una zona vacía.
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

fn canvas_card(cell: &Cell, palette: &Palette, x: f32, y: f32) -> View<Msg> {
    let id = cell.id;
    card_with_height(
        cell,
        palette,
        Style {
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
        },
        CANVAS_CARD_H - 30.0,
    )
    .draggable(move |phase, dx, dy| match phase {
        DragPhase::Move => Some(Msg::MoveCell { id, dx, dy }),
        DragPhase::End => None,
    })
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
            right: length(10.0_f32),
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

    View::new(wrapper).fill(palette.bg_card).clip(true).children(vec![header, body])
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
        "# Cosecha de auyama\n\nNotebook demo del visor canvas.\nLas celdas viven en (x, y) y los conectores muestran el DAG.",
    );
    let datos = nb.push(
        CellKind::Code { language: "rust".into() },
        "let kilos = vec![12.0, 18.0, 9.5, 21.0];",
    );
    let media = nb.push(
        CellKind::Code { language: "rust".into() },
        "let media = kilos.iter().sum::<f64>() / kilos.len() as f64;\nprintln!(\"{media}\");",
    );
    let grafico = nb.push(
        CellKind::Embed { module: "pineal".into() },
        "barras: kilos por semana",
    );
    nb.add_dependency(media, datos);
    nb.add_dependency(grafico, datos);
    nb.add_dependency(grafico, media);

    // Layout en árbol descendente: intro arriba, datos al centro, media a
    // la izquierda y gráfico a la derecha — ambos hijos de datos.
    nb.set_position(intro, Some(P::new(40.0, 40.0)));
    nb.set_position(datos, Some(P::new(40.0, 170.0)));
    nb.set_position(media, Some(P::new(40.0, 320.0)));
    nb.set_position(grafico, Some(P::new(360.0, 320.0)));

    nb
}

fn main() {
    llimphi_ui::run::<Viewer>();
}
