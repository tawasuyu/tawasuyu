//! `llimphi-widget-detail-table` — la vista **detalle** de un file manager.
//!
//! Una grilla read-only de columnas (nombre · tamaño · fecha · tipo…) con
//! **encabezados clicables que ordenan**: click en una columna emite
//! `on_sort(col)`; la columna activa muestra una flecha `▲`/`▼`. Cada fila
//! es clicable (selección) y opcionalmente lleva un tinte de acento (para
//! labels/colores, Fase 4.5).
//!
//! Como el resto de los widgets Llimphi es **render-only y stateless**: el
//! orden, el filtro y la selección viven en el `Model` del caller (típicamente
//! un `nahual_source_core::Navigator`); el widget recibe las filas **ya
//! ordenadas y ya filtradas** (igual que `widget-list` recibe sólo la ventana
//! visible) y sólo pinta + avisa.
//!
//! Las columnas declaran su ancho como [`ColWidth::Flex`] (reparte el sobrante
//! proporcionalmente — para la columna nombre) o [`ColWidth::Fixed`] (px
//! constantes — para tamaño/fecha/tipo). Encabezado y filas usan el MISMO
//! reparto, así que las columnas quedan alineadas.
//!
//! ```ignore
//! detail_table_view(
//!     DetailSpec {
//!         columns: &[Column::flex("Nombre", 1.0), Column::fixed("Tamaño", 90.0).right(),
//!                    Column::fixed("Modificado", 150.0), Column::fixed("Tipo", 80.0)],
//!         rows,                       // ya ordenadas/filtradas por el caller
//!         sort: Some((1, SortDir::Desc)),
//!         row_height: 22.0,
//!         caption: Some("42 entradas".into()),
//!         palette: DetailPalette::from_theme(&theme),
//!     },
//!     Msg::SortBy,                    // Fn(usize) -> Msg
//! )
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Ancho de una columna.
#[derive(Debug, Clone, Copy)]
pub enum ColWidth {
    /// Reparte el sobrante proporcionalmente al peso (la columna "nombre").
    Flex(f32),
    /// Ancho fijo en px (tamaño/fecha/tipo).
    Fixed(f32),
}

/// Dirección de orden — sólo para pintar la flecha del encabezado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    /// La flecha del encabezado activo.
    fn arrow(self) -> &'static str {
        match self {
            SortDir::Asc => " ▲",
            SortDir::Desc => " ▼",
        }
    }
}

/// Una columna de la grilla: rótulo + ancho + alineación del texto.
#[derive(Debug, Clone)]
pub struct Column {
    pub title: String,
    pub width: ColWidth,
    pub align: Alignment,
}

impl Column {
    /// Columna flexible (reparte sobrante). Alineada a la izquierda.
    pub fn flex(title: impl Into<String>, weight: f32) -> Self {
        Self { title: title.into(), width: ColWidth::Flex(weight), align: Alignment::Start }
    }

    /// Columna de ancho fijo. Alineada a la izquierda.
    pub fn fixed(title: impl Into<String>, px: f32) -> Self {
        Self { title: title.into(), width: ColWidth::Fixed(px), align: Alignment::Start }
    }

    /// Variante alineada a la derecha (números: tamaño).
    pub fn right(mut self) -> Self {
        self.align = Alignment::End;
        self
    }
}

/// Una fila de datos. `cells` se aparea posicionalmente con las columnas;
/// celdas de más se ignoran, de menos se pintan vacías.
pub struct DetailRow<Msg> {
    pub cells: Vec<String>,
    pub selected: bool,
    /// Tinte de fila opcional (labels/colores, Fase 4.5). `None` = sin tinte.
    pub accent: Option<Color>,
    pub on_click: Msg,
}

/// Paleta de la grilla detalle.
#[derive(Debug, Clone, Copy)]
pub struct DetailPalette {
    pub bg_panel: Color,
    pub bg_header: Color,
    pub bg_selected: Color,
    pub bg_hover: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_header: Color,
    pub accent: Color,
    pub border: Color,
}

impl Default for DetailPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl DetailPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_header: t.bg_panel_alt,
            bg_selected: t.bg_selected,
            bg_hover: t.bg_row_hover,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_header: t.fg_placeholder,
            accent: t.accent,
            border: t.border,
        }
    }
}

/// Especificación de la grilla. Las `rows` vienen YA ordenadas y filtradas
/// por el caller; `sort` es sólo para la flecha del encabezado.
pub struct DetailSpec<'a, Msg> {
    pub columns: &'a [Column],
    pub rows: Vec<DetailRow<Msg>>,
    /// Columna activa de orden + su dirección (para la flecha). `None` = sin
    /// indicador.
    pub sort: Option<(usize, SortDir)>,
    pub row_height: f32,
    pub caption: Option<String>,
    pub palette: DetailPalette,
}

/// Compone la grilla detalle. `on_sort(col)` se emite al clickear un
/// encabezado.
pub fn detail_table_view<Msg, FSort>(spec: DetailSpec<Msg>, on_sort: FSort) -> View<Msg>
where
    Msg: Clone + 'static,
    FSort: Fn(usize) -> Msg + Clone + 'static,
{
    let DetailSpec { columns, rows, sort, row_height, caption, palette } = spec;

    let mut children: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 2);

    // Caption opcional (conteo).
    if let Some(text) = caption {
        children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
                padding: pad_lr(10.0),
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(text, 10.0, palette.fg_muted, Alignment::Start),
        );
    }

    // Encabezado: una celda clicable por columna.
    let header_cells: Vec<View<Msg>> = columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let activa = sort.map(|(c, _)| c == i).unwrap_or(false);
            let flecha = match sort {
                Some((c, dir)) if c == i => dir.arrow(),
                _ => "",
            };
            let label = format!("{}{flecha}", col.title);
            let fg = if activa { palette.fg_header } else { palette.fg_header };
            col_cell(
                col.width,
                View::new(full())
                    .text_aligned(label, 10.5, fg, col.align)
                    .ellipsis(1),
            )
            .hover_fill(palette.bg_hover)
            .on_click(on_sort(i))
        })
        .collect();
    children.push(
        row_box(header_height(row_height))
            .fill(palette.bg_header)
            .children(header_cells),
    );

    // Filas de datos.
    for row in rows {
        let DetailRow { cells, selected, accent, on_click } = row;
        let bg = if selected { palette.bg_selected } else { palette.bg_panel };
        let cell_views: Vec<View<Msg>> = columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let text = cells.get(i).cloned().unwrap_or_default();
                // La primera columna (nombre) lleva el acento si hay; el resto
                // va en fg_muted salvo el nombre que va en fg_text.
                let fg = if i == 0 {
                    accent.unwrap_or(palette.fg_text)
                } else {
                    palette.fg_muted
                };
                col_cell(
                    col.width,
                    View::new(full())
                        .text_aligned(text, 11.5, fg, col.align)
                        .ellipsis(1),
                )
            })
            .collect();
        children.push(
            row_box(row_height)
                .fill(bg)
                .hover_fill(palette.bg_hover)
                .on_click(on_click)
                .children(cell_views),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(children)
}

/// Alto del encabezado: como una fila pero un toque más bajo, con piso.
fn header_height(row_height: f32) -> f32 {
    (row_height - 2.0).max(18.0)
}

/// Una fila horizontal de alto fijo (encabezado o registro). El caller le
/// agrega `.fill`/`.on_click`/`.children`.
fn row_box<Msg: Clone + 'static>(height: f32) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(height) },
        padding: pad_lr(8.0),
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
}

/// Envuelve el contenido de una celda con el ancho de la columna (flex o
/// fijo). Encabezado y registro usan esto idéntico → columnas alineadas.
fn col_cell<Msg: Clone + 'static>(width: ColWidth, child: View<Msg>) -> View<Msg> {
    let style = match width {
        ColWidth::Flex(w) => Style {
            flex_grow: w,
            flex_basis: length(0.0_f32),
            min_size: Size { width: length(0.0_f32), height: Dimension::auto() },
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
        ColWidth::Fixed(px) => Style {
            flex_shrink: 0.0,
            size: Size { width: length(px), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
    };
    View::new(style).children(vec![child])
}

/// Estilo de un hijo que ocupa todo el ancho de su celda.
fn full() -> Style {
    Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    }
}

/// Padding horizontal `px` (top/bottom en cero).
fn pad_lr(px: f32) -> Rect<llimphi_ui::llimphi_layout::taffy::LengthPercentage> {
    Rect {
        left: length(px),
        right: length(px),
        top: length(0.0_f32),
        bottom: length(0.0_f32),
    }
}
