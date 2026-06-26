//! Vista del modo Unidades: grilla de tarjetas vivas del plano de control
//! sandokan (por el contrato Engine).

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, Size, Style},
    AlignItems, FlexDirection,
};
use llimphi_theme::motion;
use llimphi_ui::View;

use sandokan_monitor_core::UnitObservation;
use ulid::Ulid;

use super::modelo::{Model, Msg};
use super::widgets::{action_btn, empty_state, fmt_mem, metric, pad, scroll_grid, sparkline, state_visual};

// ---------------------------------------------------------------------------
// Cuerpo del modo Unidades.
// ---------------------------------------------------------------------------

pub(crate) fn units_body(model: &Model) -> View<Msg> {
    let t = &model.theme;
    if model.snapshot.is_empty() {
        return empty_state(
            t,
            "Sin unidades vivas",
            "No hay init (arje-zero) ni daemon sandokan en este entorno: el \
             Engine cayó al LocalEngine in-process. Exportá \
             SANDOKAN_MONITOR_SEED=1 y reabrí para sembrar una demo viva.",
        );
    }

    let cards: Vec<View<Msg>> = model
        .snapshot
        .units
        .iter()
        .map(|u| unit_card(model, u))
        .collect();

    scroll_grid(t, cards)
}

// ---------------------------------------------------------------------------
// Tarjeta individual de unidad.
// ---------------------------------------------------------------------------

fn unit_card(model: &Model, u: &UnitObservation) -> View<Msg> {
    let t = &model.theme;
    let selected = model.selected == Some(u.card_id);
    let (dot, state_txt) = state_visual(t, &u.state);

    let cpu = u.telemetry.as_ref().map(|x| x.cpu_pct).unwrap_or(0.0);
    let mem = u.telemetry.as_ref().map(|x| x.mem_bytes).unwrap_or(0);
    let nproc = u.telemetry.as_ref().map(|x| x.nproc).unwrap_or(0);

    // Fila título: punto de estado + label.
    let title_row = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(vec![
        View::new(Style {
            size: Size {
                width: length(10.0),
                height: length(10.0),
            },
            ..Default::default()
        })
        .fill(dot)
        .radius(5.0),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text(&u.label, 14.0, t.fg_text),
        View::new(Style::default()).text(state_txt, 11.0, t.fg_muted),
    ]);

    // Sparkline de CPU.
    let spark = sparkline(t, model.history.get(&u.card_id), cpu);

    // Fila métricas.
    let restarts = if u.restarts > 0 {
        format!("↻{}", u.restarts)
    } else {
        String::new()
    };
    let metrics = View::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(12.0),
            height: length(4.0),
        },
        ..Default::default()
    })
    .children(vec![
        metric(t, &format!("{cpu:.0}% cpu")),
        metric(t, &fmt_mem(mem)),
        metric(t, &format!("{nproc} hilos")),
        View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        })
        .text(&restarts, 11.0, t.accent),
    ]);

    let mut children = vec![title_row, spark, metrics];

    // Acciones inline al seleccionar (detener/matar por el Engine).
    if selected {
        children.push(actions_row(t, u.card_id));
    }

    let bg = if selected { t.bg_selected } else { t.bg_panel_alt };
    View::new(Style {
        flex_direction: FlexDirection::Column,
        gap: Size {
            width: length(8.0),
            height: length(8.0),
        },
        padding: pad(13.0, 12.0),
        size: Size {
            width: length(260.0),
            height: auto(),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(10.0)
    .hover_fill(t.bg_row_hover)
    .on_click(Msg::Select(if selected {
        None
    } else {
        Some(u.card_id)
    }))
    // Pop-in: cada unidad nueva entra con fade la primera vez que aparece su key.
    .animated_enter(crate::key_of(&u.card_id.to_string()), motion::NORMAL)
}

fn actions_row(t: &llimphi_theme::Theme, id: Ulid) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(8.0),
            height: length(8.0),
        },
        ..Default::default()
    })
    .children(vec![
        action_btn(t, "⏹ detener", t.bg_button, t.fg_text, Msg::Stop(id)),
        action_btn(t, "✕ matar", t.fg_destructive, t.bg_app, Msg::Kill(id)),
    ])
}
