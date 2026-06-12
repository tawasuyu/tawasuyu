//! Panel del rectificador de hora: jog de nacimiento, eventos conocidos,
//! barrido por direcciones primarias (Sistema GR) y curva de perfil.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{PaintRect, View};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};

use crate::glyphs;
use crate::model::{Model, Msg};
use crate::view;

/// Botoncito de texto reutilizable para el rectificador.
fn mini_btn(label: &str, msg: Msg, enabled: bool, theme: &Theme) -> View<Msg> {
    let fg = if enabled { theme.fg_text } else { theme.fg_muted };
    let mut v = View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(llimphi_ui::llimphi_layout::taffy::JustifyContent::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .radius(4.0)
    .fill(theme.bg_panel)
    .text_aligned(label.to_string(), 11.0, fg, Alignment::Center);
    if enabled {
        v = v.hover_fill(theme.bg_row_hover).on_click(msg);
    }
    v
}

fn mini_row(kids: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(26.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(4.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(kids)
}

/// Celda de texto de ancho fijo (alto auto, centrado vertical por la fila).
fn txt_cell(text: String, w: f32, size: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(w),
            height: auto(),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(text, size, color, Alignment::Start)
}

/// Curva del perfil de rectificación: error vs offset (su valle marca la
/// hora rectificada). Marca el mejor offset con una línea de acento.
fn profile_curve(perfil: &[(i64, f32)], best: i64, theme: &Theme) -> View<Msg> {
    let pts: Vec<(f32, f32)> = perfil.iter().map(|(o, e)| (*o as f32, *e)).collect();
    let line_col = theme.fg_muted;
    let accent = theme.accent;
    let track = theme.bg_panel_alt;
    let best_f = best as f32;
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(56.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(track)
    .radius(3.0)
    .paint_with(move |scene, _ts, rect: PaintRect| {
        use llimphi_ui::llimphi_raster::kurbo::{BezPath, Line as KLine, Stroke};
        if pts.len() < 2 {
            return;
        }
        let (mut min_o, mut max_o) = (f32::INFINITY, f32::NEG_INFINITY);
        let (mut min_e, mut max_e) = (f32::INFINITY, f32::NEG_INFINITY);
        for (o, e) in &pts {
            min_o = min_o.min(*o);
            max_o = max_o.max(*o);
            min_e = min_e.min(*e);
            max_e = max_e.max(*e);
        }
        let pad = 4.0_f32;
        let w = (rect.w - 2.0 * pad).max(1.0);
        let h = (rect.h - 2.0 * pad).max(1.0);
        let span_o = (max_o - min_o).max(1.0);
        let span_e = (max_e - min_e).max(1e-6);
        let sx = |o: f32| rect.x + pad + (o - min_o) / span_o * w;
        // Error menor arriba (valle visible como pico hacia abajo → lo
        // dibujamos con el menor error ABAJO para que el valle sea un pozo).
        let sy = |e: f32| rect.y + pad + (e - min_e) / span_e * h;
        // Marca del mejor offset.
        let bx = sx(best_f) as f64;
        scene.stroke(
            &Stroke::new(1.5),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            accent,
            None,
            &KLine::new((bx, rect.y as f64), (bx, (rect.y + rect.h) as f64)),
        );
        let mut path = BezPath::new();
        for (i, (o, e)) in pts.iter().enumerate() {
            let p = (sx(*o) as f64, sy(*e) as f64);
            if i == 0 {
                path.move_to(p);
            } else {
                path.line_to(p);
            }
        }
        scene.stroke(
            &Stroke::new(1.2),
            llimphi_ui::llimphi_raster::kurbo::Affine::IDENTITY,
            line_col,
            None,
            &path,
        );
    })
}

/// Panel del rectificador de hora: jog del nacimiento, eventos conocidos,
/// barrido por direcciones primarias (Sistema GR / Germán Rosas) y curva
/// de perfil con su valle.
pub(crate) fn rectify_view(model: &Model, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();

    // Jog de la hora.
    rows.push(view::section_label(
        format!("Jog de hora — offset {:+} min", model.rectify_offset_min),
        theme,
    ));
    rows.push(mini_row(vec![
        mini_btn("-60", Msg::RectifyNudge(-60), true, theme),
        mini_btn("-10", Msg::RectifyNudge(-10), true, theme),
        mini_btn("-1", Msg::RectifyNudge(-1), true, theme),
        mini_btn("+1", Msg::RectifyNudge(1), true, theme),
        mini_btn("+10", Msg::RectifyNudge(10), true, theme),
        mini_btn("+60", Msg::RectifyNudge(60), true, theme),
        mini_btn("0", Msg::RectifyResetOffset, true, theme),
    ]));

    // Clave arco↔año.
    rows.push(view::section_label("Clave arco↔año".to_string(), theme));
    rows.push(segmented_view(
        &["Naibod", "Ptolomeo"],
        if model.rectify_naibod { 0 } else { 1 },
        |i| Msg::RectifySetKey(i == 0),
        &SegmentedPalette::from_theme(theme),
    ));

    // Eventos conocidos.
    rows.push(view::section_label("Eventos conocidos (edad)".to_string(), theme));
    for (i, age) in model.rectify_events.iter().enumerate() {
        rows.push(mini_row(vec![
            view::line(format!("{age:.1} a"), 12.0, theme.fg_text),
            mini_btn("-1", Msg::RectifyEventDelta(i, -1.0), true, theme),
            mini_btn("+1", Msg::RectifyEventDelta(i, 1.0), true, theme),
            mini_btn("-0.1", Msg::RectifyEventDelta(i, -0.1), true, theme),
            mini_btn("+0.1", Msg::RectifyEventDelta(i, 0.1), true, theme),
            mini_btn("quitar", Msg::RectifyRemoveEvent(i), true, theme),
        ]));
    }
    rows.push(mini_row(vec![
        mini_btn("+ evento", Msg::RectifyAddEvent, true, theme),
        mini_btn(
            "Rectificar",
            Msg::RectifyRun,
            !model.rectify_events.is_empty(),
            theme,
        ),
    ]));

    // Resultado + curva de perfil.
    if let Some(res) = &model.rectify_result {
        let secs = res.mejor_offset_segundos;
        rows.push(view::line(
            format!(
                "mejor: {:+} s  ({:+} min {:02} s)  ·  error {:.3}",
                secs,
                secs / 60,
                (secs.abs() % 60),
                res.mejor_puntaje
            ),
            11.0,
            theme.accent,
        ));
        rows.push(mini_row(vec![mini_btn(
            "Aplicar al nacimiento",
            Msg::RectifyApply,
            true,
            theme,
        )]));
        rows.push(profile_curve(&res.perfil, res.mejor_offset_segundos, theme));
    }

    // HUD de triggers GR (contactos directo/converso a una edad).
    rows.push(view::section_label(
        format!("Triggers GR — edad {:.1} a", model.rectify_age),
        theme,
    ));
    rows.push(mini_row(vec![
        mini_btn("-5", Msg::RectifyAgeDelta(-5.0), true, theme),
        mini_btn("-1", Msg::RectifyAgeDelta(-1.0), true, theme),
        mini_btn("+1", Msg::RectifyAgeDelta(1.0), true, theme),
        mini_btn("+5", Msg::RectifyAgeDelta(5.0), true, theme),
        mini_btn("ver triggers", Msg::RectifyTriggers, true, theme),
    ]));
    for t in model.rectify_triggers.iter().take(24) {
        let col = if t.event { theme.accent } else { theme.fg_text };
        let dir = match t.direction {
            cosmos_render::GrDirection::Direct => "D",
            cosmos_render::GrDirection::Converse => "C",
        };
        let cells: Vec<View<Msg>> = vec![
            glyphs::body_view(&t.promissor, 15.0, col),
            txt_cell(dir.to_string(), 14.0, 11.0, theme.fg_muted),
            glyphs::body_view(&t.natal_target, 15.0, col),
            txt_cell(format!("{:.2}°", t.orb_deg), 52.0, 11.0, theme.fg_muted),
            txt_cell(
                if t.event { "convergencia".into() } else { String::new() },
                80.0,
                10.0,
                theme.accent,
            ),
        ];
        rows.push(
            View::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size {
                    width: percent(1.0_f32),
                    height: length(20.0_f32),
                },
                flex_shrink: 0.0,
                align_items: Some(AlignItems::Center),
                gap: Size {
                    width: length(4.0_f32),
                    height: length(0.0_f32),
                },
                ..Default::default()
            })
            .children(cells),
        );
    }

    view::tile_container(rows, theme)
}
