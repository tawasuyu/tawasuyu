//! Vista del modo Wawa: censo de apps WASM instaladas en el kernel.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems,
};
use llimphi_theme::motion;
use llimphi_ui::View;

use super::modelo::{Model, Msg, WawaApp};
use super::widgets::{empty_state, fmt_mem, metric, note, pad, scroll_grid};

// ---------------------------------------------------------------------------
// Cuerpo del modo Wawa.
// ---------------------------------------------------------------------------

pub(crate) fn wawa_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    let mut children = vec![note(t, &rimay_localize::t("sandokan-mon-wawa-note"))];

    if model.wawa.is_empty() {
        children.push(empty_state(
            t,
            &rimay_localize::t("sandokan-mon-wawa-empty-title"),
            &rimay_localize::t("sandokan-mon-wawa-empty-body"),
        ));
    } else {
        let cards: Vec<View<Msg>> = model.wawa.iter().map(|a| wawa_card(t, a)).collect();
        children.push(scroll_grid(t, cards));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(children)
}

// ---------------------------------------------------------------------------
// Tarjeta individual de app Wawa.
// ---------------------------------------------------------------------------

fn wawa_card(t: &llimphi_theme::Theme, a: &WawaApp) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(6.0_f32),
            height: length(6.0_f32),
        },
        padding: pad(13.0, 12.0),
        size: Size {
            width: length(190.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .radius(10.0)
    .children(vec![
        View::new(Style {
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            gap: Size {
                width: length(8.0_f32),
                height: length(4.0_f32),
            },
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size {
                    width: length(10.0_f32),
                    height: length(10.0_f32),
                },
                ..Default::default()
            })
            .fill(t.accent)
            .radius(2.0),
            View::new(Style::default()).text(&a.name, 14.0, t.fg_text),
        ]),
        metric(t, &format!("{} · wasm", fmt_mem(a.bytes))),
    ])
    // Pop-in: cada app del censo entra con fade la primera vez que aparece.
    .animated_enter(crate::key_of(&a.name), motion::NORMAL)
}
