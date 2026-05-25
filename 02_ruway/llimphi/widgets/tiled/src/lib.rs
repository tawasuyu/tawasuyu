//! `llimphi-widget-tiled` — grilla auto cols×rows de tiles con title
//! bar fija arriba.
//!
//! Cada tile es un panel rectangular con:
//! - una franja superior (20 px) con `bg_panel_alt` + label centrado a
//!   la izquierda en `fg_muted`;
//! - un cuerpo flex que aloja el `View<Msg>` provisto por el caller.
//!
//! La grilla se calcula como `cols = ⌈√n⌉`, `rows = ⌈n/cols⌉` — mismo
//! algoritmo que el `nahual-widget-tiled` GPUI. Las celdas son
//! equipesos: `flex_grow = 1` sobre ambos ejes.
//!
//! ## Variantes
//!
//! - [`tiled_view`] — grilla estática, sin reordenamiento.
//! - [`tiled_view_reorderable`] — drag-to-swap: arrastrar la title bar
//!   de un tile y soltar sobre otro emite `on_reorder(from, to)`. El
//!   tile destino se ilumina (`drop_hover_fill` = `accent`) mientras
//!   el cursor está sobre él durante el drag. Usa los primitives
//!   `drag_payload` + `on_drop` + `drop_hover_fill` de `llimphi-ui`.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};

const TITLE_BAR_HEIGHT: f32 = 20.0;
const TITLE_TEXT_SIZE: f32 = 10.0;
const TILE_GAP: f32 = 4.0;
const TILE_PADDING: f32 = 4.0;

/// Paleta del tiled.
#[derive(Debug, Clone, Copy)]
pub struct TiledPalette {
    /// Fondo del container outer (visible en los gaps entre tiles).
    pub bg_outer: Color,
    /// Fondo del cuerpo del tile.
    pub bg_tile: Color,
    /// Fondo de la title bar del tile.
    pub bg_title: Color,
    /// Color del label de la title bar.
    pub fg_title: Color,
    /// Color del tile destino durante un drag (drop hover).
    pub bg_drop_hover: Color,
}

impl Default for TiledPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TiledPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg_outer: t.bg_app,
            bg_tile: t.bg_panel,
            bg_title: t.bg_panel_alt,
            fg_title: t.fg_muted,
            bg_drop_hover: t.bg_selected,
        }
    }
}

/// Un tile de la grilla: label que va en la title bar + view del cuerpo.
pub struct TileSpec<Msg> {
    pub label: String,
    pub content: View<Msg>,
}

type ReorderFn<Msg> = Arc<dyn Fn(usize, usize) -> Option<Msg> + Send + Sync>;

/// Construye una grilla estática (sin drag-to-swap). Equivalente a
/// [`tiled_view_reorderable`] sin handler de reorder.
pub fn tiled_view<Msg>(tiles: Vec<TileSpec<Msg>>, palette: &TiledPalette) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    build(tiles, palette, None)
}

/// Construye una grilla con drag-to-swap. Arrastrar la title bar de un
/// tile y soltar sobre otro invoca `on_reorder(from_index, to_index)`;
/// el `Msg` retornado se dispatchea al `update` antes de cerrar el
/// drag. El caller es responsable de filtrar `from == to`.
pub fn tiled_view_reorderable<Msg, F>(
    tiles: Vec<TileSpec<Msg>>,
    on_reorder: F,
    palette: &TiledPalette,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(usize, usize) -> Option<Msg> + Send + Sync + 'static,
{
    build(tiles, palette, Some(Arc::new(on_reorder)))
}

fn build<Msg>(
    tiles: Vec<TileSpec<Msg>>,
    palette: &TiledPalette,
    on_reorder: Option<ReorderFn<Msg>>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let n = tiles.len();
    if n == 0 {
        return View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(palette.bg_outer)
        .text(
            "(tiled vacío)".to_string(),
            11.0,
            palette.fg_title,
        );
    }

    let cols = ((n as f32).sqrt().ceil() as usize).max(1);
    let rows = (n + cols - 1) / cols;

    let mut tiles_iter = tiles.into_iter().enumerate();
    let mut rows_views: Vec<View<Msg>> = Vec::with_capacity(rows);

    for _r in 0..rows {
        let mut cells: Vec<View<Msg>> = Vec::with_capacity(cols);
        for _c in 0..cols {
            let cell = match tiles_iter.next() {
                Some((idx, tile)) => tile_view(idx, tile, palette, on_reorder.clone()),
                None => empty_cell_view(palette),
            };
            cells.push(cell);
        }
        rows_views.push(row_view(cells));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(TILE_GAP),
        },
        padding: Rect {
            left: length(TILE_PADDING),
            right: length(TILE_PADDING),
            top: length(TILE_PADDING),
            bottom: length(TILE_PADDING),
        },
        ..Default::default()
    })
    .fill(palette.bg_outer)
    .children(rows_views)
}

fn row_view<Msg>(cells: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        gap: Size {
            width: length(TILE_GAP),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(cells)
}

fn tile_view<Msg>(
    idx: usize,
    tile: TileSpec<Msg>,
    palette: &TiledPalette,
    on_reorder: Option<ReorderFn<Msg>>,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let mut title = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(TITLE_BAR_HEIGHT),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_title)
    .text_aligned(tile.label, TITLE_TEXT_SIZE, palette.fg_title, Alignment::Start);

    // Si hay reorder, la title bar arrastra con payload = idx.
    if on_reorder.is_some() {
        // Handler trivial: tiled no usa dx/dy. Devuelve None.
        title = title
            .draggable(|_phase: DragPhase, _dx: f32, _dy: f32| None)
            .drag_payload(idx as u64);
    }

    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        flex_grow: 1.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![tile.content]);

    let mut tile_view = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_tile)
    .radius(4.0)
    .clip(true)
    .children(vec![title, body]);

    // Drop target: si hay reorder, este tile entero recibe drops.
    if let Some(reorder) = on_reorder {
        let to_idx = idx;
        tile_view = tile_view
            .on_drop(move |from: u64| (reorder)(from as usize, to_idx))
            .drop_hover_fill(palette.bg_drop_hover);
    }

    tile_view
}

fn empty_cell_view<Msg>(palette: &TiledPalette) -> View<Msg>
where
    Msg: Clone + 'static,
{
    View::new(Style {
        size: Size {
            width: Dimension::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        flex_basis: length(0.0_f32),
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg_outer)
}
