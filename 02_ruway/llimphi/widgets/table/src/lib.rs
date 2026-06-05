//! `llimphi-widget-table` — tabla y lista editables para Llimphi.
//!
//! Una grilla de celdas-texto: encabezados opcionales por columna, una fila por
//! registro con botón **quitar** y un botón **agregar** al pie. La variante
//! [`list_view`] es el caso de una sola columna sin encabezado.
//!
//! **Stateless por diseño.** Como [`text_input_view`], el widget no posee el
//! foco ni los buffers de edición: el caller le dice cuál celda está focada
//! (`focused`) y le presta su [`TextInputState`] (`focused_state`); el resto de
//! las celdas se pintan desde su texto. El tecleo lo enruta el caller a su
//! `TextInputState` y reconstruye el valor — igual que con un text-input suelto.
//!
//! Es agnóstico: no sabe de config. Emite el `Msg` del caller por tres
//! callbacks (`on_focus_cell`, `on_remove_row`, `on_add_row`). El protocolo de
//! "qué cambió" (reemplazar una celda, sumar/quitar una fila) lo decide el
//! caller; el widget sólo dispara el evento con la coordenada.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};

/// Alto de una fila editable (px).
const ROW_H: f32 = 30.0;
/// Alto del encabezado de tabla (px).
const HEADER_H: f32 = 24.0;
/// Alto del botón "agregar" (px).
const ADD_H: f32 = 32.0;

/// Paleta de la tabla: la del text-input de las celdas + colores del cromo.
#[derive(Debug, Clone, Copy)]
pub struct TablePalette {
    /// Paleta de los inputs de celda.
    pub input: TextInputPalette,
    /// Color del texto de los encabezados.
    pub header_fg: Color,
    /// Color del glifo de "quitar fila".
    pub remove_fg: Color,
    /// Color del texto del botón "agregar".
    pub add_fg: Color,
    /// Borde del botón "agregar".
    pub add_border: Color,
    /// Relleno de hover de botones.
    pub hover: Color,
}

impl Default for TablePalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TablePalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            input: TextInputPalette::from_theme(t),
            header_fg: t.fg_placeholder,
            remove_fg: t.fg_muted,
            add_fg: t.accent,
            add_border: t.border,
            hover: t.bg_row_hover,
        }
    }
}

/// Alto total de una tabla/lista de `n_rows` filas (con o sin encabezado), para
/// que un contenedor con scroll estime el alto del control.
pub fn table_height(n_rows: usize, has_header: bool) -> f32 {
    let head = if has_header { HEADER_H } else { 0.0 };
    head + n_rows as f32 * ROW_H + ADD_H
}

/// Compone una **tabla** editable.
///
/// - `headers`: rótulos de columna. Si está vacío no se pinta encabezado (modo
///   lista). Su largo fija el número de columnas; si está vacío, se infiere del
///   ancho de la primera fila (o 1).
/// - `rows`: el texto de cada celda.
/// - `focused` / `focused_state`: la celda en edición y su buffer (prestado por
///   el caller). El resto de las celdas se pintan desde `rows`.
/// - `on_focus_cell(row, col)`: clic en una celda (el caller arranca a editarla).
/// - `on_remove_row(row)` / `on_add_row()`: quitar/agregar fila.
#[allow(clippy::too_many_arguments)]
pub fn table_view<Msg, FFocus, FRemove, FAdd>(
    headers: &[String],
    rows: &[Vec<String>],
    focused: Option<(usize, usize)>,
    focused_state: Option<&TextInputState>,
    add_label: &str,
    palette: &TablePalette,
    on_focus_cell: FFocus,
    on_remove_row: FRemove,
    on_add_row: FAdd,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FFocus: Fn(usize, usize) -> Msg + Clone + Send + Sync + 'static,
    FRemove: Fn(usize) -> Msg + Clone + Send + Sync + 'static,
    FAdd: Fn() -> Msg + Clone + Send + Sync + 'static,
{
    let ncols = if !headers.is_empty() {
        headers.len()
    } else {
        rows.first().map(Vec::len).unwrap_or(1).max(1)
    };

    let mut children: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 2);

    // Encabezados (sólo si hay rótulos): una etiqueta por columna + hueco del
    // botón "quitar".
    if !headers.is_empty() {
        let mut head: Vec<View<Msg>> = headers.iter().map(|h| flex_cell(header(h, palette))).collect();
        head.push(remove_spacer());
        children.push(row_container(head));
    }

    for (r, cells) in rows.iter().enumerate() {
        let mut row_kids: Vec<View<Msg>> = Vec::with_capacity(ncols + 1);
        for c in 0..ncols {
            let text = cells.get(c).map(String::as_str).unwrap_or("");
            let is_focused = focused == Some((r, c));
            let st = if is_focused { focused_state } else { None };
            let focus_msg = on_focus_cell(r, c);
            row_kids.push(flex_cell(cell_input(text, is_focused, st, &palette.input, focus_msg)));
        }
        let remove_msg = on_remove_row(r);
        row_kids.push(remove_button(remove_msg, palette));
        children.push(row_container(row_kids));
    }

    children.push(add_button(add_label, on_add_row(), palette));
    column_container(children)
}

/// Compone una **lista** editable: una sola columna sin encabezado.
/// `on_focus_cell(row)` recibe sólo la fila (la columna es siempre 0).
pub fn list_view<Msg, FFocus, FRemove, FAdd>(
    items: &[String],
    focused_row: Option<usize>,
    focused_state: Option<&TextInputState>,
    add_label: &str,
    palette: &TablePalette,
    on_focus_cell: FFocus,
    on_remove_row: FRemove,
    on_add_row: FAdd,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    FFocus: Fn(usize) -> Msg + Clone + Send + Sync + 'static,
    FRemove: Fn(usize) -> Msg + Clone + Send + Sync + 'static,
    FAdd: Fn() -> Msg + Clone + Send + Sync + 'static,
{
    let rows: Vec<Vec<String>> = items.iter().map(|s| vec![s.clone()]).collect();
    let focused = focused_row.map(|r| (r, 0usize));
    table_view(
        &[],
        &rows,
        focused,
        focused_state,
        add_label,
        palette,
        move |r, _c| on_focus_cell(r),
        on_remove_row,
        on_add_row,
    )
}

/// Un input de celda: como [`text_input_view`] pero con el foco provisto por el
/// caller (no por un `FieldPath`). Si la celda está focada usa el buffer
/// prestado; si no, pinta un input estático sembrado con su texto.
fn cell_input<Msg: Clone + 'static>(
    text: &str,
    focused: bool,
    state: Option<&TextInputState>,
    palette: &TextInputPalette,
    focus_msg: Msg,
) -> View<Msg> {
    if focused {
        if let Some(st) = state {
            return text_input_view(st, "", true, palette, focus_msg);
        }
    }
    let mut tmp = TextInputState::new();
    tmp.set_text(text);
    text_input_view(&tmp, "", false, palette, focus_msg)
}

/// Envuelve una celda repartiendo el ancho en partes iguales entre columnas.
fn flex_cell<Msg: Clone + 'static>(child: View<Msg>) -> View<Msg> {
    View::new(Style {
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        size: Size {
            width: Dimension::auto(),
            height: Dimension::auto(),
        },
        min_size: Size {
            width: length(0.0_f32),
            height: Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![child])
}

/// Una fila horizontal (celdas + botón quitar).
fn row_container<Msg: Clone + 'static>(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(children)
}

/// El contenedor vertical de la tabla/lista.
fn column_container<Msg: Clone + 'static>(rows: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(4.0_f32),
        },
        ..Default::default()
    })
    .children(rows)
}

/// El encabezado de una columna.
fn header<Msg: Clone + 'static>(label: &str, palette: &TablePalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(label.to_string(), 10.5, palette.header_fg, Alignment::Start)
}

/// El botón cuadrado de "quitar fila" (`×`, U+00D7 — la fuente del SO sí lo
/// trae, a diferencia de `✕`).
fn remove_button<Msg: Clone + 'static>(msg: Msg, palette: &TablePalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .radius(5.0)
    .hover_fill(palette.hover)
    .on_click(msg)
    .text_aligned("×".to_string(), 15.0, palette.remove_fg, Alignment::Center)
}

/// Un hueco del ancho del botón quitar, para alinear el encabezado.
fn remove_spacer<Msg: Clone + 'static>() -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(26.0_f32),
            height: length(16.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
}

/// El botón "agregar" al pie.
fn add_button<Msg: Clone + 'static>(label: &str, msg: Msg, palette: &TablePalette) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: length(26.0_f32),
        },
        align_self: Some(AlignItems::Start),
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(2.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(6.0)
    .border(1.0, palette.add_border)
    .hover_fill(palette.hover)
    .on_click(msg)
    .text_aligned(label.to_string(), 11.5, palette.add_fg, Alignment::Center)
}
