//! `llimphi-widget-wrap` — contenedor con `flex-wrap: wrap` y gap.
//!
//! Lo que en Flutter es `Wrap` y en Compose `FlowRow`: items que fluyen
//! horizontalmente y saltan a la siguiente línea cuando se acaba el
//! ancho. Usalo para chips, tags, galerías fluidas, toolbars que
//! respiran. taffy ya soporta `flex-wrap`; este crate es el azúcar
//! mínima para no repetir el `Style` boilerplate.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, FlexWrap, Size, Style},
    AlignItems,
};
use llimphi_ui::View;

/// Eje principal del wrap.
#[derive(Debug, Clone, Copy)]
pub enum WrapAxis {
    Row,
    Column,
}

/// Wrap container.
///
/// - `axis` decide eje principal (row = horizontal, column = vertical).
/// - `h_gap` es el gap **entre items en la misma línea**.
/// - `v_gap` es el gap **entre líneas**.
pub fn wrap_view<Msg: Clone + 'static>(
    children: Vec<View<Msg>>,
    axis: WrapAxis,
    h_gap: f32,
    v_gap: f32,
) -> View<Msg> {
    let dir = match axis {
        WrapAxis::Row => FlexDirection::Row,
        WrapAxis::Column => FlexDirection::Column,
    };
    View::new(Style {
        flex_direction: dir,
        flex_wrap: FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        gap: Size {
            width: length(h_gap),
            height: length(v_gap),
        },
        align_items: Some(AlignItems::FlexStart),
        ..Default::default()
    })
    .children(children)
}

/// Atajo: row de chips con gap parejo 6px.
pub fn chip_row<Msg: Clone + 'static>(children: Vec<View<Msg>>) -> View<Msg> {
    wrap_view(children, WrapAxis::Row, 6.0, 6.0)
}
