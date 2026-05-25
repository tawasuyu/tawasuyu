//! `pluma-notebook-llimphi` — visor read-only de notebooks sobre Llimphi.
//!
//! Uso:
//!   pluma-notebook-llimphi [ruta.pluma-nb]
//!
//! Sin argumento, abre un notebook demo (markdown + dos celdas de código +
//! embed de pineal) para mostrar la vista sin necesitar un archivo en disco.
//!
//! MVP: solo render. Sin edición de fuentes, sin ejecución contra kernel,
//! sin scroll (todas las celdas se apilan; si exceden la ventana se cortan).
//! Edición → integrar `pluma-editor-llimphi`. Ejecución → cablear
//! `pluma-notebook-exec::run_all` desde un botón.

use std::env;
use std::path::PathBuf;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use pluma_notebook_core::{Cell, CellKind, CellState, Notebook};

#[derive(Clone)]
enum Msg {}

struct Model {
    notebook: Notebook,
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
        (820, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let arg = env::args().nth(1).map(PathBuf::from);
        match arg {
            None => Model { notebook: demo_notebook(), source: None, load_error: None },
            Some(p) => match pluma_notebook_store::load(&p) {
                Ok(nb) => Model { notebook: nb, source: Some(p), load_error: None },
                Err(e) => Model {
                    notebook: Notebook::new(),
                    source: Some(p),
                    load_error: Some(e.to_string()),
                },
            },
        }
    }

    fn update(model: Model, _: Msg, _: &Handle<Msg>) -> Model {
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = Palette::from_theme(&theme);

        let header = header_bar(model, &palette);
        let cards: Vec<View<Msg>> = model.notebook.cells().iter().map(|c| cell_card(c, &palette)).collect();

        let stack = View::new(Style {
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
        .children(cards);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(palette.bg)
        .children(vec![header, stack])
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
    let texto = format!("pluma-notebook · {} celdas · digest {} · {}", model.notebook.len(), digest, origen);
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

fn cell_card(cell: &Cell, palette: &Palette) -> View<Msg> {
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
        size: Size { width: percent(1.0_f32), height: length(body_height(&cell.source)) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(truncate_source(&cell.source), 12.0, palette.fg_text, Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(body_height(&cell.source) + 30.0) },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_card)
    .clip(true)
    .children(vec![header, body])
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

const MAX_BODY_LINES: usize = 8;
const MAX_BODY_CHARS: usize = 400;

fn truncate_source(s: &str) -> String {
    let mut out = String::new();
    for (i, line) in s.lines().enumerate() {
        if i >= MAX_BODY_LINES || out.len() + line.len() + 1 > MAX_BODY_CHARS {
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

/// Alto fijo del body: una línea base + extras por salto, capeado.
/// Sin scroll todavía, así que mejor un alto predecible que uno que
/// crezca con texto arbitrario.
fn body_height(source: &str) -> f32 {
    let lines = source.lines().count().min(MAX_BODY_LINES).max(1);
    16.0 * lines as f32 + 12.0
}

fn short_hex(d: &[u8; 32]) -> String {
    d[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// Notebook embebido — se usa cuando se invoca el binario sin argumento.
fn demo_notebook() -> Notebook {
    let mut nb = Notebook::new();
    nb.push(
        CellKind::Markdown,
        "# Cosecha de auyama\n\nNotebook demo del visor.\nCada celda se renderiza como un card con su tipo, estado y fuente.",
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
    nb
}

fn main() {
    llimphi_ui::run::<Viewer>();
}
