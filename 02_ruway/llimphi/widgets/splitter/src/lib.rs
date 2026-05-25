//! `llimphi-widget-splitter` — split container con divisor draggable.
//!
//! Análogo Llimphi al `nahual-widget-splitter` GPUI: dos panes con un
//! divisor entre medio que el usuario arrastra para reasignar el tamaño.
//! El widget no mantiene estado: el caller acumula el tamaño de un pane
//! en su `Model` y le pasa el valor actual + un handler `Fn(DragPhase,
//! f32) -> Option<Msg>` que materializa el delta en un Msg de update.
//!
//! Uso típico (dos panes, izquierdo fijo y derecho flex):
//!
//! ```ignore
//! splitter_two(
//!     Direction::Row,
//!     left_view,
//!     PaneSize::Fixed(model.left_size),
//!     right_view,
//!     PaneSize::Flex,
//!     |phase, dx| match phase {
//!         DragPhase::Move => Some(Msg::ResizeLeft(dx)),
//!         DragPhase::End => Some(Msg::PersistLayout),
//!     },
//!     &SplitterPalette::default(),
//! )
//! ```

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{DragPhase, View};

/// Dirección del split. `Row` apila los panes horizontalmente
/// (divisor vertical, drag horizontal); `Column` los apila verticalmente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Row,
    Column,
}

/// Tamaño de un pane sobre el eje principal del split.
#[derive(Debug, Clone, Copy)]
pub enum PaneSize {
    /// Ancho/alto fijo en pixels. El otro pane se ajusta con `flex_grow`.
    Fixed(f32),
    /// Toma todo el espacio sobrante (`flex_grow = 1`).
    Flex,
}

/// Paleta del divisor. Cambia de color al hover para señalar
/// "agarrame y arrastrá".
#[derive(Debug, Clone, Copy)]
pub struct SplitterPalette {
    pub divider: Color,
    pub divider_hover: Color,
    pub thickness: f32,
}

impl Default for SplitterPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl SplitterPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            divider: t.border,
            divider_hover: t.accent,
            thickness: 6.0,
        }
    }
}

/// Split de dos panes con divisor draggable entre medio. `on_resize`
/// se invoca con el delta del eje principal (positivo → divisor se
/// mueve a la derecha/abajo).
pub fn splitter_two<Msg, F>(
    direction: Direction,
    a: View<Msg>,
    a_size: PaneSize,
    b: View<Msg>,
    b_size: PaneSize,
    on_resize: F,
    palette: &SplitterPalette,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
    F: Fn(DragPhase, f32) -> Option<Msg> + Send + Sync + 'static,
{
    let flex_dir = match direction {
        Direction::Row => FlexDirection::Row,
        Direction::Column => FlexDirection::Column,
    };

    // El divisor sólo necesita Msg en el eje principal — escondemos el
    // otro detrás del closure.
    let on_resize = Arc::new(on_resize);
    let cb_dir = direction;
    let cb = on_resize.clone();
    let divider = divider_view::<Msg>(direction, palette, move |phase, dx, dy| {
        let main = match cb_dir {
            Direction::Row => dx,
            Direction::Column => dy,
        };
        (cb)(phase, main)
    });

    let pane_a = wrap_pane(a, direction, a_size);
    let pane_b = wrap_pane(b, direction, b_size);

    View::new(Style {
        flex_direction: flex_dir,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![pane_a, divider, pane_b])
}

fn wrap_pane<Msg>(view: View<Msg>, direction: Direction, size: PaneSize) -> View<Msg> {
    let (width, height, flex_grow) = match (direction, size) {
        (Direction::Row, PaneSize::Fixed(px)) => (length(px), percent(1.0_f32), 0.0),
        (Direction::Row, PaneSize::Flex) => (Dimension::auto(), percent(1.0_f32), 1.0),
        (Direction::Column, PaneSize::Fixed(px)) => (percent(1.0_f32), length(px), 0.0),
        (Direction::Column, PaneSize::Flex) => (percent(1.0_f32), Dimension::auto(), 1.0),
    };
    View::new(Style {
        size: Size { width, height },
        flex_grow,
        flex_shrink: 0.0,
        min_size: Size {
            width: length(0.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![view])
}

fn divider_view<Msg>(
    direction: Direction,
    palette: &SplitterPalette,
    handler: impl Fn(DragPhase, f32, f32) -> Option<Msg> + Send + Sync + 'static,
) -> View<Msg>
where
    Msg: Clone + Send + Sync + 'static,
{
    let (width, height) = match direction {
        Direction::Row => (length(palette.thickness), percent(1.0_f32)),
        Direction::Column => (percent(1.0_f32), length(palette.thickness)),
    };
    View::new(Style {
        size: Size { width, height },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.divider)
    .hover_fill(palette.divider_hover)
    .draggable(handler)
}
