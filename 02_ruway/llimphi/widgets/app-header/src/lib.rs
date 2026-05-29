//! `llimphi-widget-app-header` — tira superior estándar de las apps.
//!
//! Reproduce el contrato del `nahual-widget-app-header` GPUI: label
//! dinámico a la izquierda con `flex_grow`, slot a la derecha para
//! acciones (theme switcher, botones de toolbar, etc.). bg = `bg_panel`,
//! line-bottom como `border` del theme.
//!
//! Uso típico:
//!
//! ```ignore
//! app_header(format!("Log: {} · {} entries", path, n), vec![], &palette)
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_widget_panel::{panel_signature_painter, PanelStyle};

/// Paleta del header. Defaults desde el theme global.
#[derive(Debug, Clone, Copy)]
pub struct AppHeaderPalette {
    pub bg: Color,
    pub border_bottom: Color,
    pub fg_text: Color,
    pub height: f32,
    /// Firma visual: gradient sutil + hairline accent en el top edge. Se
    /// activa por defecto al construir desde theme. `None` cae al fill
    /// plano de `bg` (modo back-compat para sitios que arman la palette
    /// a mano sin theme).
    pub signature: Option<PanelStyle>,
}

impl Default for AppHeaderPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl AppHeaderPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_panel,
            border_bottom: t.border,
            fg_text: t.fg_text,
            height: 40.0,
            signature: Some(PanelStyle {
                radius: 0.0,
                ..PanelStyle::from_theme(t)
            }),
        }
    }
}

/// Header con `label` a la izquierda y `actions` a la derecha. `actions`
/// es vacío para apps sin toolbar; viene como Vec para que la app meta
/// botones / switcher / status pill / lo que necesite.
pub fn app_header<Msg: Clone + 'static>(
    label: impl Into<String>,
    actions: Vec<View<Msg>>,
    palette: &AppHeaderPalette,
) -> View<Msg> {
    let label_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(palette.height),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.into(), 14.0, palette.fg_text, Alignment::Start);

    let actions_view = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
            height: length(palette.height),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(actions);

    // Bottom border: el header rellena `bg` (o aplica la firma si está
    // habilitada), y debajo va una línea 1px de `border_bottom`. Lo
    // metemos como un wrapper column.
    let bar_style = Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(palette.height),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    };
    let bar = match palette.signature {
        Some(style) => View::new(bar_style)
            .paint_with(panel_signature_painter(style))
            .children(vec![label_view, actions_view]),
        None => View::new(bar_style)
            .fill(palette.bg)
            .children(vec![label_view, actions_view]),
    };

    let underline = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.border_bottom);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: length(palette.height + 1.0),
        },
        ..Default::default()
    })
    .children(vec![bar, underline])
}
