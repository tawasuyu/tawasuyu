//! `llimphi-widget-list` — lista vertical virtualizada.
//!
//! Compone una pila de filas con foco visual en la seleccionada y un Msg
//! por click. Pensado como bloque reusable para file explorers, árboles
//! lineales, paneles de log, listados de items, etc.
//!
//! El widget **no** maneja virtualización por sí mismo: el caller pasa
//! únicamente las filas que deberían renderearse (las visibles según su
//! propio `offset`/`scroll`). El widget se ocupa del resto: caption
//! opcional con el conteo, fondo de selección, hint "… y N más" cuando
//! `total > rows.len()`, y `clip` en el contenedor para que las filas no
//! sangren a vecinos.
//!
//! Ejemplo:
//!
//! ```ignore
//! let rows: Vec<ListRow<Msg>> = entries[offset..(offset + visible).min(entries.len())]
//!     .iter()
//!     .enumerate()
//!     .map(|(i, e)| ListRow {
//!         label: e.name.clone(),
//!         selected: offset + i == selected,
//!         on_click: Msg::Select(offset + i),
//!     })
//!     .collect();
//!
//! let panel = list_view(ListSpec {
//!     rows,
//!     total: entries.len(),
//!     caption: Some(format!("{} entradas", entries.len())),
//!     truncated_hint: (entries.len() > offset + rows.len())
//!         .then(|| format!("… y {} más", entries.len() - offset - rows.len())),
//!     row_height: 22.0,
//!     palette: ListPalette::default(),
//! });
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Paleta de la lista. Los defaults son una variante dark con selección
/// azulada — equivalente conceptual a `nahual_theme` en su tema oscuro.
#[derive(Debug, Clone, Copy)]
pub struct ListPalette {
    pub bg_panel: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
}

impl Default for ListPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ListPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
        }
    }
}

/// Una fila a renderear. `selected` cambia el fondo; `on_click` se emite al
/// hacer click sobre cualquier parte de la fila.
pub struct ListRow<Msg> {
    pub label: String,
    pub selected: bool,
    pub on_click: Msg,
}

/// Especificación completa de la lista a renderear.
pub struct ListSpec<Msg> {
    /// Filas a renderear, ya filtradas a la ventana visible.
    pub rows: Vec<ListRow<Msg>>,
    /// Total de items del modelo (usado para el caption — la lista
    /// mostrada puede ser un subconjunto virtualizado).
    pub total: usize,
    /// Caption opcional arriba de las filas (p. ej. "120 entradas").
    pub caption: Option<String>,
    /// Mensaje opcional al pie ("… y 12 más") cuando hay items fuera de
    /// la ventana visible. El caller decide qué texto usar.
    pub truncated_hint: Option<String>,
    /// Altura de cada fila en pixels.
    pub row_height: f32,
    pub palette: ListPalette,
}

/// Compone la lista como un `View<Msg>`. El contenedor tiene `clip = true`
/// para evitar overflow visual cuando el llamador subestima el tamaño
/// disponible — las filas que excedan el área del panel se recortan.
pub fn list_view<Msg: Clone + 'static>(spec: ListSpec<Msg>) -> View<Msg> {
    let ListSpec {
        rows,
        total: _,
        caption,
        truncated_hint,
        row_height,
        palette,
    } = spec;

    let mut children: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 2);

    if let Some(text) = caption {
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(text, 10.0, palette.fg_muted, Alignment::Start),
        );
    }

    for row in rows {
        children.push(row_view(row, row_height, &palette));
    }

    if let Some(text) = truncated_hint {
        children.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(16.0_f32),
                },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(10.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(text, 10.0, palette.fg_muted, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(children)
}

fn row_view<Msg: Clone + 'static>(row: ListRow<Msg>, height: f32, palette: &ListPalette) -> View<Msg> {
    let bg = if row.selected {
        palette.bg_selected
    } else {
        palette.bg_panel
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(row.label, 12.0, palette.fg_text, Alignment::Start)
    .on_click(row.on_click)
}
