//! `llimphi-widget-grid` — grilla virtualizada 2D para Llimphi.
//!
//! Hermano de [`llimphi-widget-list`], pero en mosaico: celdas clicables
//! dispuestas en `cols` columnas × filas, con selección, caption/hint
//! opcionales y recorte de overflow. Pensado como base para galerías de
//! miniaturas tipo gThumb / FastStone — capaz de listar miles de archivos
//! sin montar todo: el caller renderea **sólo la ventana visible**.
//!
//! Como `list`, el widget **no** scrollea por sí mismo. La virtualización
//! es del caller, que mantiene `scroll_fila` (primera fila de celdas
//! visible) en su estado y lo actualiza con la rueda (calco de
//! `nahual-file-explorer`). La diferencia con `list` es que en 2D el
//! cálculo de la ventana no es trivial — cuántas columnas caben depende
//! del ancho del viewport — así que este crate lo provee como función
//! pura testeable: [`ventana_visible`].
//!
//! El widget es **agnóstico del contenido**: cada [`GridCell`] lleva un
//! `View<Msg>` que el caller arma (un thumb `peniko::Image`, un skeleton
//! mientras decodifica, un ícono…). Así el pipeline de miniaturas (cola
//! async + cache) vive afuera y sólo llena la celda con imagen o
//! placeholder.
//!
//! Flujo típico del caller:
//!
//! ```ignore
//! let v = ventana_visible(total, viewport_w, viewport_h, scroll_fila, &metrics);
//! let cells: Vec<GridCell<Msg>> = (v.first..v.first + v.count)
//!     .map(|i| GridCell {
//!         content: thumb_o_placeholder(i),     // el caller decide
//!         label: Some(nombre(i)),
//!         selected: i == seleccionado,
//!         on_click: Msg::Seleccionar(i),
//!     })
//!     .collect();
//! let grid = grid_view(GridSpec {
//!     cells, cols: v.cols, metrics,
//!     caption: Some(format!("{total} imágenes")),
//!     truncated_hint: (v.first + v.count < total)
//!         .then(|| format!("… y {} más", total - (v.first + v.count))),
//!     palette: GridPalette::default(),
//! });
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Geometría de la grilla en pixels. `tile_h` debe incluir el alto del
/// label si el caller lo usa — el widget reserva una franja inferior para
/// él dentro de la celda. `gap` es el espacio entre celdas (y entre filas);
/// `pad` el margen interno del contenedor (cada lado).
#[derive(Debug, Clone, Copy)]
pub struct GridMetrics {
    pub tile_w: f32,
    pub tile_h: f32,
    pub gap: f32,
    pub pad: f32,
}

impl Default for GridMetrics {
    fn default() -> Self {
        // Default ~thumb mediano estilo gThumb.
        Self {
            tile_w: 128.0,
            tile_h: 148.0, // 128 imagen + ~20 label
            gap: 8.0,
            pad: 8.0,
        }
    }
}

/// Resultado del cálculo de virtualización: qué celdas montar. `first` y
/// `count` delimitan el rango de índices `[first, first + count)` que el
/// caller debe renderear; `cols` cuántas columnas caben (para `grid_view`).
/// Los demás campos son informativos (scrollbars, "fila X de Y", clamping).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleWindow {
    /// Columnas que caben en el ancho del viewport (≥ 1).
    pub cols: usize,
    /// Índice del primer item a renderear.
    pub first: usize,
    /// Cantidad de items a renderear desde `first`.
    pub count: usize,
    /// Fila (0-based) del primer item visible — ya clampeada al rango.
    pub first_row: usize,
    /// Total de filas que ocupa la colección completa.
    pub total_rows: usize,
    /// Filas que entran en el alto del viewport (incluye 1 de margen).
    pub filas_visibles: usize,
}

/// Calcula la ventana visible de una grilla virtualizada. **Función pura.**
///
/// - `total`: cantidad total de items.
/// - `viewport_w` / `viewport_h`: dimensiones del área de la grilla en px.
/// - `scroll_fila`: primera fila que el caller quiere ver arriba (se
///   clampa al rango válido; el caller no necesita pre-clampar).
/// - `m`: geometría (tile + gap + pad).
///
/// El número de columnas se deriva del ancho: `cols = ⌊(ancho_útil + gap)
/// / (tile_w + gap)⌋`, mínimo 1. Las filas visibles incluyen una extra de
/// margen para que una fila parcial al borde no aparezca en blanco al
/// scrollear.
pub fn ventana_visible(
    total: usize,
    viewport_w: f32,
    viewport_h: f32,
    scroll_fila: usize,
    m: &GridMetrics,
) -> VisibleWindow {
    let paso_w = (m.tile_w + m.gap).max(1.0);
    let paso_h = (m.tile_h + m.gap).max(1.0);

    let util_w = (viewport_w - 2.0 * m.pad + m.gap).max(0.0);
    let cols = ((util_w / paso_w).floor() as usize).max(1);

    let total_rows = total.div_ceil(cols);

    let util_h = (viewport_h - 2.0 * m.pad + m.gap).max(0.0);
    let filas_visibles = (util_h / paso_h).ceil() as usize + 1;

    let max_first_row = total_rows.saturating_sub(1);
    let first_row = scroll_fila.min(max_first_row);
    let first = first_row * cols;
    let last_row = (first_row + filas_visibles).min(total_rows);
    let count = (last_row * cols).min(total).saturating_sub(first);

    VisibleWindow {
        cols,
        first,
        count,
        first_row,
        total_rows,
        filas_visibles,
    }
}

/// Paleta de la grilla. Defaults dark con selección azulada (calco de
/// `ListPalette`).
#[derive(Debug, Clone, Copy)]
pub struct GridPalette {
    pub bg_panel: Color,
    pub bg_cell: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
}

impl Default for GridPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl GridPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_cell: t.bg_panel_alt,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
        }
    }
}

/// Una celda de la grilla. `content` es el `View` que el caller arma para
/// el cuerpo (thumb/placeholder/ícono); el widget lo centra y, debajo,
/// pinta `label` truncable si está. `on_click` se emite al clickear la
/// celda completa.
pub struct GridCell<Msg> {
    pub content: View<Msg>,
    pub label: Option<String>,
    pub selected: bool,
    pub on_click: Msg,
}

/// Especificación completa de la grilla a renderear. `cells` ya viene
/// recortada a la ventana visible (ver [`ventana_visible`]); `cols` es el
/// número de columnas de esa ventana.
pub struct GridSpec<Msg> {
    pub cells: Vec<GridCell<Msg>>,
    pub cols: usize,
    pub metrics: GridMetrics,
    pub caption: Option<String>,
    pub truncated_hint: Option<String>,
    pub palette: GridPalette,
}

/// Compone la grilla como un `View<Msg>`. Agrupa `cells` en filas de
/// `cols` celdas y las apila. El contenedor recorta (`clip`) para que las
/// celdas no sangren a vecinos cuando el caller subestima el área.
pub fn grid_view<Msg: Clone + 'static>(spec: GridSpec<Msg>) -> View<Msg> {
    let GridSpec {
        cells,
        cols,
        metrics,
        caption,
        truncated_hint,
        palette,
    } = spec;
    let cols = cols.max(1);

    let mut children: Vec<View<Msg>> = Vec::new();

    if let Some(text) = caption {
        children.push(barra_texto(text, 11.0, palette.fg_muted, 20.0));
    }

    // Agrupar en filas de `cols`. La última fila puede quedar incompleta.
    let mut iter = cells.into_iter();
    loop {
        let fila: Vec<GridCell<Msg>> = iter.by_ref().take(cols).collect();
        if fila.is_empty() {
            break;
        }
        children.push(fila_view(fila, &metrics, &palette));
    }

    if let Some(text) = truncated_hint {
        children.push(barra_texto(text, 10.0, palette.fg_muted, 16.0));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(metrics.pad),
            right: length(metrics.pad),
            top: length(metrics.pad),
            bottom: length(metrics.pad),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(metrics.gap),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(children)
}

fn fila_view<Msg: Clone + 'static>(
    fila: Vec<GridCell<Msg>>,
    m: &GridMetrics,
    palette: &GridPalette,
) -> View<Msg> {
    let celdas: Vec<View<Msg>> = fila
        .into_iter()
        .map(|c| celda_view(c, m, palette))
        .collect();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(m.tile_h),
        },
        gap: Size {
            width: length(m.gap),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(celdas)
}

fn celda_view<Msg: Clone + 'static>(
    cell: GridCell<Msg>,
    m: &GridMetrics,
    palette: &GridPalette,
) -> View<Msg> {
    let bg = if cell.selected {
        palette.bg_selected
    } else {
        palette.bg_cell
    };

    let mut hijos: Vec<View<Msg>> = Vec::with_capacity(2);
    // Cuerpo de la celda: el content del caller, centrado, ocupa el alto
    // restante (tile_h menos la franja de label).
    hijos.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(1.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .clip(true)
        .children(vec![cell.content]),
    );
    if let Some(label) = cell.label {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(18.0_f32),
                },
                padding: Rect {
                    left: length(4.0_f32),
                    right: length(4.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .clip(true)
            .text_aligned(label, 10.0, palette.fg_text, Alignment::Center),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(m.tile_w),
            height: length(m.tile_h),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .clip(true)
    .children(hijos)
    .on_click(cell.on_click)
}

fn barra_texto<Msg: Clone + 'static>(
    text: String,
    size: f32,
    color: Color,
    height: f32,
) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(4.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn metrics() -> GridMetrics {
        GridMetrics {
            tile_w: 80.0,
            tile_h: 96.0,
            gap: 8.0,
            pad: 8.0,
        }
    }

    #[test]
    fn cols_se_deriva_del_ancho() {
        let m = metrics();
        // util_w = 400 - 16 + 8 = 392; paso = 88; 392/88 = 4.45 → 4.
        let v = ventana_visible(100, 400.0, 300.0, 0, &m);
        assert_eq!(v.cols, 4);
        assert_eq!(v.total_rows, 25);
    }

    #[test]
    fn ancho_minimo_da_al_menos_una_columna() {
        let m = metrics();
        let v = ventana_visible(10, 50.0, 300.0, 0, &m);
        assert_eq!(v.cols, 1, "nunca menos de 1 columna");
    }

    #[test]
    fn ventana_arriba_monta_filas_visibles_mas_margen() {
        let m = metrics();
        // util_h = 300 - 16 + 8 = 292; paso_h = 104; ceil(292/104)=3; +1 = 4.
        let v = ventana_visible(100, 400.0, 300.0, 0, &m);
        assert_eq!(v.filas_visibles, 4);
        assert_eq!(v.first, 0);
        // 4 filas × 4 cols = 16 items.
        assert_eq!(v.count, 16);
    }

    #[test]
    fn ventana_al_fondo_clampa_y_recorta_la_cola() {
        let m = metrics();
        // 100 items, 4 cols → 25 filas. Pedir fila 22 (cerca del fondo).
        let v = ventana_visible(100, 400.0, 300.0, 22, &m);
        assert_eq!(v.first_row, 22);
        assert_eq!(v.first, 88);
        // last_row = min(22+4, 25) = 25 → count = min(100,100) - 88 = 12.
        assert_eq!(v.count, 12);
    }

    #[test]
    fn scroll_mas_alla_del_fondo_se_clampa() {
        let m = metrics();
        let v = ventana_visible(100, 400.0, 300.0, 999, &m);
        // total_rows 25 → max_first_row 24.
        assert_eq!(v.first_row, 24);
        assert_eq!(v.first, 96);
        // Sólo la última fila: 100 - 96 = 4 items.
        assert_eq!(v.count, 4);
    }

    #[test]
    fn coleccion_vacia_no_monta_nada() {
        let m = metrics();
        let v = ventana_visible(0, 400.0, 300.0, 0, &m);
        assert!(v.cols >= 1);
        assert_eq!(v.total_rows, 0);
        assert_eq!(v.count, 0);
        assert_eq!(v.first, 0);
    }

    #[test]
    fn ultima_fila_parcial_cuenta_completa_en_total_rows() {
        let m = metrics();
        // 10 items, 4 cols → 3 filas (la última con 2).
        let v = ventana_visible(10, 400.0, 1000.0, 0, &m);
        assert_eq!(v.cols, 4);
        assert_eq!(v.total_rows, 3);
        // Viewport alto: entran todas.
        assert_eq!(v.count, 10);
    }

    #[test]
    fn grid_view_agrupa_en_filas_sin_panicar() {
        // Smoke: 7 celdas en 3 columnas → 3 filas (3+3+1). Sólo verifica
        // que compone sin panicar y devuelve un View.
        let cells: Vec<GridCell<i32>> = (0..7)
            .map(|i| GridCell {
                content: View::new(Style::default()),
                label: Some(format!("img{i}")),
                selected: i == 2,
                on_click: i,
            })
            .collect();
        let _v = grid_view(GridSpec {
            cells,
            cols: 3,
            metrics: metrics(),
            caption: Some("7 imágenes".into()),
            truncated_hint: None,
            palette: GridPalette::default(),
        });
    }
}
