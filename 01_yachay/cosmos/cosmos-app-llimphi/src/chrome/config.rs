//! Vista de configuración del panel de herramientas.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{DragPhase, View};
use llimphi_widget_segmented::{segmented_view, SegmentedPalette};
use llimphi_widget_slider::{slider_view, SliderPalette};
use llimphi_widget_switch::{switch_view, SwitchPalette};

use crate::model::{Model, Msg, OverlayKind, WheelOpt, HARMONICS};
use crate::view;

/// Fila de configuración: etiqueta + switch. Reutilizable en toda la vista.
fn switch_row(label: &str, on: bool, msg: Msg, pal: &SwitchPalette, theme: &Theme) -> View<Msg> {
    let lbl = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(0.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(label.to_string(), 12.0, theme.fg_text, Alignment::Start);

    let sw = View::new(Style {
        size: Size {
            width: length(44.0_f32),
            height: length(24.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![switch_view(if on { 1.0 } else { 0.0 }, msg, pal)]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(28.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(vec![lbl, sw])
}

/// Panel de configuración completo.
pub(crate) fn config_view(model: &Model, theme: &Theme) -> View<Msg> {
    let seg_pal = SegmentedPalette::from_theme(theme);
    let sw_pal = SwitchPalette::from_theme(theme);
    let sl_pal = SliderPalette::from_theme(theme);

    let mut rows: Vec<View<Msg>> = Vec::new();

    rows.push(view::section_label("Tema".to_string(), theme));
    rows.push(segmented_view(
        &["Oscuro", "Claro", "Impresión"],
        model.cfg.theme_idx(),
        |i| Msg::SetThemeMode(i),
        &seg_pal,
    ));

    rows.push(view::section_label("Armónico".to_string(), theme));
    let h_idx = HARMONICS.iter().position(|h| *h == model.harmonic).unwrap_or(0);
    rows.push(segmented_view(
        &["H1", "H4", "H5", "H7", "H9"],
        h_idx,
        |i| Msg::SetHarmonic(HARMONICS.get(i).copied().unwrap_or(1)),
        &seg_pal,
    ));

    rows.push(view::section_label("Rueda".to_string(), theme));
    rows.push(switch_row(
        "Aspectos menores",
        model.cfg.minor_aspects,
        Msg::ToggleWheelOpt(WheelOpt::MinorAspects),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Etiquetas de coordenadas",
        model.cfg.coord_labels,
        Msg::ToggleWheelOpt(WheelOpt::CoordLabels),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Dial 3D",
        model.cfg.dial_3d,
        Msg::ToggleWheelOpt(WheelOpt::Dial3d),
        &sw_pal,
        theme,
    ));
    rows.push(switch_row(
        "Cruz ascensional",
        model.cfg.asc_cross,
        Msg::ToggleWheelOpt(WheelOpt::AscCross),
        &sw_pal,
        theme,
    ));
    rows.push(slider_view(
        "Rotación",
        model.cfg.rot_offset_deg,
        0.0,
        360.0,
        &sl_pal,
        |phase, dv| match phase {
            DragPhase::Move => Some(Msg::SetRotOffset(dv)),
            DragPhase::End => None,
        },
    ));

    rows.push(view::section_label("Astronomía".to_string(), theme));
    rows.push(switch_row(
        "Usar instante actual (ahora)",
        model.cfg.use_now,
        Msg::SetUseNow(!model.cfg.use_now),
        &sw_pal,
        theme,
    ));
    let (instante, lugar) = match &model.astro {
        Some(a) => (a.instant_iso.clone(), a.place_label.clone()),
        None => ("calculando…".to_string(), "calculando…".to_string()),
    };
    rows.push(view::line(format!("instante: {instante}"), 11.0, theme.fg_muted));
    rows.push(view::line(format!("lugar: {lugar}"), 11.0, theme.fg_muted));

    rows.push(view::section_label("Capas".to_string(), theme));
    for k in OverlayKind::all() {
        rows.push(switch_row(
            k.nombre(),
            model.overlays.contains(k),
            Msg::ToggleOverlay(*k),
            &sw_pal,
            theme,
        ));
    }

    view::tile_container(rows, theme)
}
