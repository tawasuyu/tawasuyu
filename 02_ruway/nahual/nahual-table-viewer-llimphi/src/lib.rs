//! `nahual-table-viewer-llimphi` — visor de CSV/TSV.
//!
//! Octavo visor del shell meta-app. `shuma-discern` marca `.csv`/`.tsv`
//! con lens `table` (por el hint de path + presencia del delimitador);
//! hasta ahora caían al text viewer, que muestra las filas crudas sin
//! alinear. Este visor parsea la tabla (comillas básicas estilo CSV) y
//! la pinta **alineada por columnas** en fuente monoespaciada — un
//! preview rápido para ver la forma de los datos.
//!
//! NO es el editor de planillas (`nakui`): es de sólo-lectura, capado en
//! filas/columnas, pensado para "echarle un ojo" desde el shell. Patrón
//! fino de los otros viewers: carga sync en [`load_table`], render en
//! [`table_viewer_view`].

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

// El dominio (parseo + tipos) vive en `nahual-viewer-core`; lo
// re-exportamos para no romper a los consumidores.
pub use nahual_viewer_core::table::*;

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct TableViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for TableViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TableViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre · filas×cols) + body con la tabla monoespaciada.
pub fn table_viewer_view<Msg>(
    state: &TablePreview,
    path: Option<&Path>,
    palette: &TableViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = match path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => "(seleccioná un CSV/TSV)".to_string(),
    };
    let header_text = match state {
        TablePreview::Table { rows, cols, .. } => {
            format!("table · {name} · {rows} × {cols}")
        }
        _ => format!("table · {name}"),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let (body_text, body_color) = match state {
        TablePreview::Empty => ("—".to_string(), palette.fg_muted),
        TablePreview::Table { text, .. } => (text.clone(), palette.fg_text),
        TablePreview::TooBig(n) => (
            format!("(tabla muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        TablePreview::Error(e) => (format!("(error: {e})"), palette.fg_error),
    };

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned_full(
        body_text,
        12.0,
        body_color,
        Alignment::Start,
        false,
        Some("monospace".to_string()),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}
