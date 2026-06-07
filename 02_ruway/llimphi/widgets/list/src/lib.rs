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

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};

/// Paleta de la lista. Los defaults son una variante dark con selección
/// azulada — equivalente conceptual a `nahual_theme` en su tema oscuro.
#[derive(Debug, Clone, Copy)]
pub struct ListPalette {
    pub bg_panel: Color,
    pub bg_selected: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    /// Resalte de la fila destino mientras un drag de reorder pasa por
    /// encima. Sólo se usa en [`reorderable_list_view`]. Default = accent
    /// translúcido (40 %) sobre `bg_selected` — distinguible del hover y
    /// la selección estable.
    pub bg_drop_hover: Color,
}

impl Default for ListPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl ListPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        // Resalte de drop = accent del theme con 40 % de opacidad
        // multiplicada — gana sobre `bg_selected` por luminancia para que
        // un drag sobre una fila ya seleccionada se note.
        let mut drop = t.accent;
        drop.components[3] *= 0.40;
        Self {
            bg_panel: t.bg_panel,
            bg_selected: t.bg_selected,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            bg_drop_hover: drop,
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
    // Labels largos terminan en `…` (single-line) en vez de cortarse seco.
    .ellipsis(1)
    .on_click(row.on_click)
}

/// Función que el caller usa para reaccionar a un reorder. Recibe `(from,
/// to)` — índices en `rows` — y devuelve el `Msg` a despachar (o `None`
/// para ignorar el drop, p. ej. si `from == to`).
pub type ReorderFn<Msg> = Arc<dyn Fn(usize, usize) -> Option<Msg> + Send + Sync>;

/// Una fila para [`reorderable_list_view`]. Pesa más que [`ListRow`]
/// porque cada fila acepta `on_click` opcional y siempre lleva drag
/// handle al borde izquierdo (gripper `⋮⋮` en `fg_muted`) — convención
/// kanban/Trello/Flutter `ReorderableListView`.
pub struct ReorderableListRow<Msg> {
    pub label: String,
    pub selected: bool,
    pub on_click: Option<Msg>,
}

/// Especificación de una lista reordenable por drag&drop (Bloque 14 de
/// PARIDAD-FLUTTER, sigue Tier 5). Cada fila lleva un gripper a la
/// izquierda; arrastrar una fila y soltarla sobre otra emite
/// `on_reorder(from, to)`. La fila destino se ilumina con
/// `palette.bg_drop_hover` mientras el cursor está sobre ella durante el
/// drag.
pub struct ReorderableListSpec<Msg> {
    pub rows: Vec<ReorderableListRow<Msg>>,
    pub caption: Option<String>,
    pub row_height: f32,
    pub palette: ListPalette,
    pub on_reorder: ReorderFn<Msg>,
}

/// Compone una lista reordenable (Bloque 14). Patrón: cada fila exhibe
/// un gripper al borde izquierdo y es `draggable` con `payload = idx`;
/// la **fila entera** (no sólo el gripper) recibe drops con `on_drop` y
/// `drop_hover_fill`. El handler `on_reorder(from, to)` cae al caller
/// que decide qué `Msg` despachar — el widget no muta nada por sí solo.
///
/// Composición pura sobre los primitives `drag_payload` / `on_drop` /
/// `drop_hover_fill` / `draggable` de `llimphi-ui` (ver `tiled` que
/// reordena paneles bajo el mismo idiom).
pub fn reorderable_list_view<Msg>(spec: ReorderableListSpec<Msg>) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let ReorderableListSpec {
        rows,
        caption,
        row_height,
        palette,
        on_reorder,
    } = spec;

    let mut children: Vec<View<Msg>> = Vec::with_capacity(rows.len() + 1);

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

    for (idx, row) in rows.into_iter().enumerate() {
        children.push(reorderable_row_view(idx, row, row_height, &palette, on_reorder.clone()));
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

fn reorderable_row_view<Msg>(
    idx: usize,
    row: ReorderableListRow<Msg>,
    height: f32,
    palette: &ListPalette,
    on_reorder: ReorderFn<Msg>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let bg = if row.selected {
        palette.bg_selected
    } else {
        palette.bg_panel
    };

    // Gripper `⋮⋮` al borde izquierdo, en `fg_muted` y arrastrable. El
    // drag entrega `payload = idx`. Devolvemos `None` por evento de drag
    // (no usamos dx/dy aquí — el destino se decide en el `on_drop` del
    // otro nodo).
    let gripper = View::new(Style {
        size: Size {
            width: length(20.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("⋮⋮", 14.0, palette.fg_muted, Alignment::Center)
    .draggable(|_phase: DragPhase, _dx: f32, _dy: f32| None)
    .drag_payload(idx as u64)
    .cursor(llimphi_ui::Cursor::Grab);

    // Etiqueta: ocupa el resto de la fila, con ellipsis y click opcional.
    let mut label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(row.label, 12.0, palette.fg_text, Alignment::Start)
    .ellipsis(1);
    if let Some(msg) = row.on_click {
        label = label.on_click(msg);
    }

    // El **row entero** es target de drop. Cuando el cursor pasa por
    // encima durante un drag, `drop_hover_fill` lo ilumina. Al soltar,
    // emitimos el reorder si from != to.
    let to_idx = idx;
    let reorder = on_reorder.clone();
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(height),
        },
        padding: Rect {
            left: length(4.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .on_drop(move |from: u64| {
        let from = from as usize;
        if from == to_idx {
            None
        } else {
            (reorder)(from, to_idx)
        }
    })
    .drop_hover_fill(palette.bg_drop_hover)
    .children(vec![gripper, label])
}
