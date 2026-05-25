//! `llimphi-widget-card` — container card-shape para entries de
//! timeline, info cards, dashboards, etc.
//!
//! Aporta la **forma**: padding consistente (12/8), `radius` 4, gap
//! pequeño entre children, y opcionalmente un accent vertical
//! (4 px) pegado a la izquierda para entries semánticas (verde =
//! OK, rojo = error, ámbar = warning, etc).
//!
//! Análogo Llimphi al `nahual-widget-card` GPUI.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

#[derive(Debug, Clone, Copy)]
pub struct CardPalette {
    pub bg: Color,
}

impl Default for CardPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl CardPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self { bg: t.bg_panel }
    }
}

/// Opciones del card.
#[derive(Debug, Clone, Copy)]
pub struct CardOptions {
    /// Accent vertical a la izquierda (4 px). `None` = sin accent.
    pub accent: Option<Color>,
    pub padding: f32,
    pub gap: f32,
    pub radius: f64,
}

impl Default for CardOptions {
    fn default() -> Self {
        Self {
            accent: None,
            padding: 12.0,
            gap: 4.0,
            radius: 4.0,
        }
    }
}

/// Compone un card: bg + radius + padding + flex-column con gap entre
/// children. Si `opts.accent` está presente, hay una franja vertical
/// de 4 px del color del accent pegada al borde izquierdo.
pub fn card_view<Msg: Clone + 'static>(
    children: Vec<View<Msg>>,
    opts: CardOptions,
    palette: &CardPalette,
) -> View<Msg> {
    let pad = opts.padding;
    let body = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(pad),
            right: length(pad),
            top: length(pad * 0.66),
            bottom: length(pad * 0.66),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(opts.gap),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .radius(opts.radius)
    .children(children);

    let Some(accent) = opts.accent else {
        return body;
    };

    let accent_strip = View::new(Style {
        size: Size {
            width: length(4.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(accent)
    .radius(opts.radius);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        ..Default::default()
    })
    .children(vec![accent_strip, body])
}
