//! `llimphi-widget-status-bar` — barra de estado inferior.
//!
//! Patrón clásico de IDEs/editores: barra delgada en el borde inferior
//! de la ventana con tres regiones (left/center/right). Cada región
//! tiene N segmentos, cada uno puede llevar icono + texto + handler de
//! click opcional.
//!
//! Útil para mostrar: rama git activa, posición del cursor, tipo de
//! archivo, modo (insert/normal), notificaciones pendientes, etc.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_icons::{icon_view, Icon};
use llimphi_theme::Theme;

/// Paleta de la barra de estado.
#[derive(Debug, Clone, Copy)]
pub struct StatusBarPalette {
    pub bg: Color,
    pub fg: Color,
    pub fg_muted: Color,
    pub bg_hover: Color,
    pub border: Color,
}

impl StatusBarPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_panel_alt,
            fg: t.fg_text,
            fg_muted: t.fg_muted,
            bg_hover: t.bg_row_hover,
            border: t.border,
        }
    }
}

/// Un segmento de la barra. `icon` y `on_click` son opcionales.
#[derive(Clone)]
pub struct StatusSegment<Msg> {
    pub text: String,
    pub icon: Option<Icon>,
    pub on_click: Option<Msg>,
    /// Si `true`, usa `fg` en vez de `fg_muted` — útil para destacar
    /// estados importantes (ej. "modificado").
    pub emphasized: bool,
}

impl<Msg> StatusSegment<Msg> {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            on_click: None,
            emphasized: false,
        }
    }
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn clickable(mut self, msg: Msg) -> Self {
        self.on_click = Some(msg);
        self
    }
    pub fn emphasized(mut self) -> Self {
        self.emphasized = true;
        self
    }
}

const BAR_H: f32 = 22.0;
const SEG_GAP: f32 = 14.0;
const FONT_SIZE: f32 = 11.0;
const ICON_SIZE: f32 = 12.0;

pub fn status_bar_view<Msg: Clone + 'static>(
    left: Vec<StatusSegment<Msg>>,
    center: Vec<StatusSegment<Msg>>,
    right: Vec<StatusSegment<Msg>>,
    palette: &StatusBarPalette,
) -> View<Msg> {
    let make_region = |segs: Vec<StatusSegment<Msg>>, justify: JustifyContent| -> View<Msg> {
        let children: Vec<View<Msg>> = segs
            .into_iter()
            .map(|s| segment_view(s, palette))
            .collect();
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(justify),
            gap: Size {
                width: length(SEG_GAP),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(children)
    };

    let left_region = make_region(left, JustifyContent::FlexStart);
    let center_region = make_region(center, JustifyContent::Center);
    let right_region = make_region(right, JustifyContent::FlexEnd);

    // Borde top de 1px — separa visualmente de la zona principal.
    let border = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.border);

    let bar = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(BAR_H),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(palette.bg)
    .children(vec![left_region, center_region, right_region]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(BAR_H + 1.0),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![border, bar])
}

fn segment_view<Msg: Clone + 'static>(
    seg: StatusSegment<Msg>,
    palette: &StatusBarPalette,
) -> View<Msg> {
    let fg = if seg.emphasized { palette.fg } else { palette.fg_muted };
    let approx_w = seg.text.chars().count() as f32 * 6.0
        + if seg.icon.is_some() { ICON_SIZE + 4.0 } else { 0.0 }
        + 12.0;

    let mut children: Vec<View<Msg>> = Vec::with_capacity(2);
    if let Some(icon) = seg.icon {
        children.push(
            View::new(Style {
                size: Size {
                    width: length(ICON_SIZE),
                    height: length(ICON_SIZE),
                },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![icon_view(icon, fg, 1.4)]),
        );
    }
    children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(seg.text.clone(), FONT_SIZE, fg, Alignment::Start),
    );

    let mut node = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(approx_w),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(children);

    if let Some(msg) = seg.on_click {
        node = node.hover_fill(palette.bg_hover).on_click(msg);
    }
    node
}
