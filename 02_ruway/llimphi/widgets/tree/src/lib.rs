//! `llimphi-widget-tree` — árbol con expand/collapse y selección.
//!
//! Análogo Llimphi al `nahual-widget-tree` GPUI. No mantiene estado
//! propio: el `Model` del App lleva el set de nodos expandidos + el
//! seleccionado, le pasa al widget la lista aplanada de filas (sólo
//! las visibles según el estado de expansión) y maneja los Msg de
//! toggle/select.
//!
//! Aplanar el árbol vive del lado del caller para no imponer una
//! representación específica (recursiva, plana con paths, etc.).
//!
//! Cada fila lleva su `depth` (para indentar), `has_children` (para
//! decidir si dibujar la flecha ▸/▾) y `expanded` (cuál de las dos).
//! Click en la flecha → `on_toggle`; click en el resto de la fila →
//! `on_select`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Paleta del árbol — un subset del `Theme` semántico, igual que los
/// otros widgets de Llimphi.
#[derive(Debug, Clone, Copy)]
pub struct TreePalette {
    pub bg_panel: Color,
    pub bg_selected: Color,
    pub bg_hover: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_chevron: Color,
}

impl Default for TreePalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TreePalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_panel: t.bg_panel,
            bg_selected: t.bg_selected,
            bg_hover: t.bg_row_hover,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_chevron: t.fg_muted,
        }
    }
}

/// Una fila del árbol — ya posicionada en la lista plana visible.
pub struct TreeRow<Msg> {
    pub label: String,
    /// Nivel de anidación (0 = raíz). Se traduce a indentación visual.
    pub depth: usize,
    /// Si el nodo tiene hijos. `false` = hoja; no se dibuja el chevron.
    pub has_children: bool,
    /// Estado actual del nodo. Ignorado si `has_children = false`.
    pub expanded: bool,
    /// Si esta fila es la seleccionada.
    pub selected: bool,
    /// Msg al hacer click en el chevron. Sólo se usa si `has_children`.
    pub on_toggle: Msg,
    /// Msg al hacer click en la fila (label o área alrededor).
    pub on_select: Msg,
}

/// Especificación completa del árbol a renderear.
pub struct TreeSpec<Msg> {
    pub rows: Vec<TreeRow<Msg>>,
    pub row_height: f32,
    pub indent_px: f32,
    pub palette: TreePalette,
}

/// Compone el árbol como `View<Msg>`. El contenedor activa `clip` para
/// que filas que excedan el rect se recorten — usar dentro de un panel
/// del tamaño deseado.
pub fn tree_view<Msg: Clone + 'static>(spec: TreeSpec<Msg>) -> View<Msg> {
    let TreeSpec {
        rows,
        row_height,
        indent_px,
        palette,
    } = spec;

    let children: Vec<View<Msg>> = rows
        .into_iter()
        .map(|row| tree_row_view(row, row_height, indent_px, &palette))
        .collect();

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_panel)
    .clip(true)
    .children(children)
}

fn tree_row_view<Msg: Clone + 'static>(
    row: TreeRow<Msg>,
    height: f32,
    indent_px: f32,
    palette: &TreePalette,
) -> View<Msg> {
    let bg = if row.selected {
        palette.bg_selected
    } else {
        palette.bg_panel
    };
    let indent = (row.depth as f32) * indent_px;

    // Chevron a la izquierda — 16px de ancho, ▸ si colapsado, ▾ si
    // expandido. Si es hoja, espacio en blanco del mismo ancho para que
    // los labels alineen.
    // ASCII puro: fuentes default sin glyphs Unicode bloque dibujan
    // cuadrados de fallback. `v`/`>` son universales.
    let chevron_label = if row.has_children {
        if row.expanded {
            "v"
        } else {
            ">"
        }
    } else {
        " "
    };
    let chevron_msg = if row.has_children {
        Some(row.on_toggle)
    } else {
        None
    };
    let mut chevron = View::new(Style {
        size: Size {
            width: length(16.0_f32),
            height: length(height),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        chevron_label.to_string(),
        12.0,
        palette.fg_chevron,
        Alignment::Center,
    );
    if let Some(msg) = chevron_msg {
        // Hover sólo si es interactivo.
        chevron = chevron.hover_fill(palette.bg_hover).on_click(msg);
    }

    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(4.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(row.label, 12.0, palette.fg_text, Alignment::Start)
    .on_click(row.on_select);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        padding: Rect {
            left: length(8.0_f32 + indent),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .children(vec![chevron, label])
}
