//! Composición visual del panel: barra horizontal o vertical con tres
//! slots `left | center | right`. Conky-style floating queda para una
//! iteración posterior — la barra alcanza para validar el modelo.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::PanelConfig;
use crate::widget::{Msg, Widget};

/// Direcciona el panel según `position`.
pub fn flex_dir(pos: &str) -> FlexDirection {
    match pos {
        "left" | "right" => FlexDirection::Column,
        _ => FlexDirection::Row,
    }
}

/// Construye el `View<Msg>` raíz a partir de la config y los widgets vivos.
pub fn build(
    cfg: &PanelConfig,
    theme: &Theme,
    left: &[Box<dyn Widget>],
    center: &[Box<dyn Widget>],
    right: &[Box<dyn Widget>],
) -> View<Msg> {
    let dir = flex_dir(&cfg.position);

    let slot = |ws: &[Box<dyn Widget>], justify: JustifyContent, flex_grow: f32| -> View<Msg> {
        let items: Vec<View<Msg>> = ws.iter().map(|w| w.view(theme)).collect();
        let mut style = Style {
            flex_direction: dir,
            size: Size {
                width: if matches!(dir, FlexDirection::Row) {
                    percent(0.0_f32)
                } else {
                    percent(1.0_f32)
                },
                height: if matches!(dir, FlexDirection::Row) {
                    percent(1.0_f32)
                } else {
                    percent(0.0_f32)
                },
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(justify),
            gap: Size {
                width: length(cfg.gap),
                height: length(cfg.gap),
            },
            ..Default::default()
        };
        style.flex_grow = flex_grow;
        View::new(style).children(items)
    };

    let bar_style = Style {
        flex_direction: dir,
        size: Size {
            width: percent(1.0_f32),
            height: if matches!(dir, FlexDirection::Row) {
                length(cfg.height)
            } else {
                percent(1.0_f32)
            },
        },
        padding: Rect {
            left: length(cfg.padding),
            right: length(cfg.padding),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::SpaceBetween),
        ..Default::default()
    };

    let root_style = Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    };

    View::new(root_style).fill(theme.bg_app).children(vec![View::new(bar_style)
        .fill(theme.bg_panel_alt)
        .children(vec![
            slot(left, JustifyContent::FlexStart, 1.0),
            slot(center, JustifyContent::Center, 1.0),
            slot(right, JustifyContent::FlexEnd, 1.0),
        ])])
}
