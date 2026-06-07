//! `llimphi-widget-scaffold` — chasis de página.
//!
//! Inspiración Flutter `Scaffold`/Material 3 layout. Compone un layout
//! de página común:
//!
//! ```text
//! ┌──────────────────────────────┐
//! │  app bar (opcional)          │  ← 48 px
//! ├──────────────────────────────┤
//! │                              │
//! │   body (flex-grow 1)         │
//! │                          ╭──╮│
//! │                          │+ ││  ← FAB opcional (bottom-end)
//! │                          ╰──╯│
//! ├──────────────────────────────┤
//! │  bottom bar (opcional)       │  ← 56 px
//! └──────────────────────────────┘
//! ```
//!
//! Los drawers laterales se ofrecen como **overlays** (la app los pasa
//! por `view_overlay`, no por el body — así no roban el layout cuando
//! están cerrados). Este widget sólo aporta el chasis central; los
//! drawers son responsabilidad de quien los abre.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Position,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

#[derive(Debug, Clone, Copy)]
pub struct ScaffoldPalette {
    pub bg: Color,
}

impl ScaffoldPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self { bg: t.bg_app }
    }
}

/// Opciones del scaffold — todas las superficies son opcionales (un
/// scaffold con sólo `body` es válido y degenera en `View` con bg).
pub struct ScaffoldSpec<Msg: Clone + 'static> {
    pub app_bar: Option<View<Msg>>,
    pub body: View<Msg>,
    pub bottom_bar: Option<View<Msg>>,
    pub fab: Option<View<Msg>>,
}

/// Compone el scaffold. Llamado típicamente desde el `view(model)` de la
/// app como root. Para drawers, pasarlos por `view_overlay`.
pub fn scaffold_view<Msg: Clone + 'static>(
    spec: ScaffoldSpec<Msg>,
    palette: &ScaffoldPalette,
) -> View<Msg> {
    let mut col_children: Vec<View<Msg>> = Vec::with_capacity(3);

    if let Some(bar) = spec.app_bar {
        col_children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![bar]),
        );
    }

    // Body con FAB anclado.
    let mut body_layer_children: Vec<View<Msg>> = vec![spec.body];
    if let Some(f) = spec.fab {
        body_layer_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: llimphi_ui::llimphi_layout::taffy::Rect {
                    left: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
                    right: length(16.0_f32),
                    top: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
                    bottom: length(16.0_f32),
                },
                ..Default::default()
            })
            .children(vec![f]),
        );
    }
    let body_layer = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: llimphi_ui::llimphi_layout::taffy::prelude::Dimension::auto(),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(body_layer_children);
    col_children.push(body_layer);

    if let Some(bar) = spec.bottom_bar {
        col_children.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(56.0_f32) },
                flex_shrink: 0.0,
                ..Default::default()
            })
            .children(vec![bar]),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(palette.bg)
    .children(col_children)
}

/// App bar estándar — barra superior 48 px con título a la izquierda y
/// slot de acciones a la derecha. El caller pasa el `View` de las
/// acciones (botones de ícono típicamente).
pub fn app_bar_view<Msg: Clone + 'static>(
    title: impl Into<String>,
    actions: Vec<View<Msg>>,
    palette: &ScaffoldPalette,
    theme: &Theme,
) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    let _ = palette;
    let title_view = View::new(Style {
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(16.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(title.into(), 16.0, theme.fg_text, Alignment::Start)
    .bold();
    let actions_view = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(4.0_f32), height: length(0.0_f32) },
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            left: length(0.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(actions);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![title_view, actions_view])
}
