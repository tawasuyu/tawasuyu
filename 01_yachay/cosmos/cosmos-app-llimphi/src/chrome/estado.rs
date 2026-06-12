//! Barra de menú principal y barra de estado inferior.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

use crate::model::{MenuKind, Model, Msg, MENU_BAR_H, MENU_BTN_W, STATUS_H};

// =====================================================================
// Barra de menú principal
// =====================================================================

pub(crate) fn menu_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let mut kids: Vec<View<Msg>> = Vec::new();

    // Pill de marca.
    kids.push(
        View::new(Style {
            size: Size {
                width: length(68.0_f32),
                height: length(20.0_f32),
            },
            flex_shrink: 0.0,
            margin: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(5.0_f32),
                bottom: length(5.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(theme.accent)
        .radius(4.0)
        .text_aligned("cosmos".to_string(), 11.0, theme.bg_app, Alignment::Center),
    );

    for k in MenuKind::order() {
        let active = model.menu_open == Some(*k);
        let mut btn = View::new(Style {
            size: Size {
                width: length(MENU_BTN_W),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(k.label().to_string(), 12.0, theme.fg_text, Alignment::Center)
        .hover_fill(theme.bg_row_hover)
        .on_click(Msg::OpenMenu(*k));
        if active {
            btn = btn.fill(theme.bg_selected);
        }
        kids.push(btn);
    }

    // Spacer + etiqueta de la carta a la derecha.
    kids.push(
        View::new(Style {
            flex_grow: 1.0,
            size: Size {
                width: percent(0.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        }),
    );
    kids.push(
        View::new(Style {
            size: Size {
                width: length(260.0_f32),
                height: percent(1.0_f32),
            },
            flex_shrink: 0.0,
            padding: Rect {
                left: length(0.0_f32),
                right: length(12.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(model.chart.label.clone(), 11.0, theme.fg_muted, Alignment::End),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(MENU_BAR_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(kids)
}

// =====================================================================
// Barra de estado
// =====================================================================

pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    let txt = if let Some(err) = &model.error {
        format!("error: {err}")
    } else if let Some(note) = &model.status_note {
        note.clone()
    } else {
        format!(
            "{}  ·  {} ms  ·  {} capas  ·  {} aspectos  ·  {} overlays",
            model.active_label(),
            model.render.compute_ms,
            model.render.layers.len(),
            model.render.aspect_summary.len(),
            model.render.overlays.len(),
        )
    };
    let color = if model.error.is_some() {
        theme.fg_destructive
    } else {
        theme.fg_muted
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(txt, 11.0, color, Alignment::Start)
}
