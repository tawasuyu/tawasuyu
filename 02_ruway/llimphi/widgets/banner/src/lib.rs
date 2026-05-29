//! `llimphi-widget-banner` — tiras horizontales de status.
//!
//! Cuatro variants con paleta consistente entre apps:
//!
//! - [`BannerKind::Info`] — azul tenue, mensajes neutros.
//! - [`BannerKind::Success`] — verde, confirmaciones de op exitosa.
//! - [`BannerKind::Warning`] — amber, llamadas de atención.
//! - [`BannerKind::Error`] — rojo, errores fatales o de carga.
//!
//! Análogo Llimphi al `nahual-widget-banner` GPUI. Los colores son
//! **semánticos** y no cambian con el theme (un Error en dark y en
//! light tiene que seguir leyéndose como rojo).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Severidad / tono del banner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Success,
    Warning,
    Error,
}

impl BannerKind {
    pub fn bg(self) -> Color {
        match self {
            BannerKind::Info => Color::from_rgba8(0x1d, 0x2a, 0x3a, 0xff),
            BannerKind::Success => Color::from_rgba8(0x2d, 0x3a, 0x2a, 0xff),
            BannerKind::Warning => Color::from_rgba8(0x4a, 0x3a, 0x1a, 0xff),
            BannerKind::Error => Color::from_rgba8(0x4a, 0x20, 0x20, 0xff),
        }
    }

    pub fn fg(self) -> Color {
        match self {
            BannerKind::Info => Color::from_rgba8(0xc0, 0xd0, 0xe0, 0xff),
            BannerKind::Success => Color::from_rgba8(0xc0, 0xe0, 0xa0, 0xff),
            BannerKind::Warning => Color::from_rgba8(0xf0, 0xe0, 0xa0, 0xff),
            BannerKind::Error => Color::from_rgba8(0xff, 0xd0, 0xd0, 0xff),
        }
    }
}

/// Ancho del rail de severidad en el edge izquierdo. Mismo valor que
/// `llimphi-widget-toast` — banner y toast son las versiones persistente
/// y efímera del mismo lenguaje (P5 → P8).
const RAIL_W: f32 = 3.0;

/// Banner simple: una fila con `message` centrado verticalmente y
/// alineado a la izquierda. bg/fg vienen del `kind`.
pub fn banner_view<Msg: Clone + 'static>(
    kind: BannerKind,
    message: impl Into<String>,
) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::FlexDirection;

    // Rail de severidad en el edge izquierdo — stripe del color fg
    // semántico, visible al pasar el ojo. Mismo patrón que toast P5.
    let rail = View::new(Style {
        size: Size {
            width: length(RAIL_W),
            height: percent(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(kind.fg());

    // Contenedor del mensaje: padding original ahora vive acá para que
    // el rail pegue al borde sin offset y el texto arranque después.
    let body = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(message.into(), 11.0, kind.fg(), Alignment::Start);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(kind.bg())
    .radius(3.0)
    .clip(true)
    .children(vec![rail, body])
}
