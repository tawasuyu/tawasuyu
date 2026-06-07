//! `llimphi-widget-gauge` — medidor radial.
//!
//! Arco de 270° (de 7:30 a 4:30, manecillas de reloj) con un track
//! gris fino y un arco activo del color `accent` que crece con el
//! valor. En el centro, una etiqueta opcional con el valor formateado.
//! Pensado para dashboards (CPU, RAM, throughput) y métricas con
//! contexto (lleno / vacío / objetivo).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;
use llimphi_theme::Theme;

#[derive(Debug, Clone, Copy)]
pub struct GaugePalette {
    pub track: Color,
    pub active: Color,
    pub fg: Color,
}

impl GaugePalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            track: t.border,
            active: t.accent,
            fg: t.fg_text,
        }
    }
}

/// Render del gauge. `value` es la fracción 0..=1 que representa la
/// progresión del arco. `label` es texto centrado opcional.
pub fn gauge_view<Msg: Clone + 'static>(
    value: f32,
    size_px: f32,
    label: Option<String>,
    palette: &GaugePalette,
) -> View<Msg> {
    let v = value.clamp(0.0, 1.0);
    let track = palette.track;
    let active = palette.active;
    let mut node = View::new(Style {
        size: Size { width: length(size_px), height: length(size_px) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        paint_gauge(scene, rect, v, track, active);
    });
    if let Some(lbl) = label {
        node = node.text_aligned(lbl, (size_px * 0.22).max(11.0), palette.fg, Alignment::Center);
    }
    node
}

fn paint_gauge(
    scene: &mut llimphi_ui::llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    value: f32,
    track: Color,
    active: Color,
) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Arc, Point, Stroke};
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let cx = (rect.x + rect.w * 0.5) as f64;
    let cy = (rect.y + rect.h * 0.5) as f64;
    let s = rect.w.min(rect.h) as f64;
    let stroke_w = (s * 0.10).max(2.0);
    let r = (s * 0.5) - stroke_w * 0.6;

    // Arco completo (270° = 1.5π), empieza en 135° (7:30) y avanza
    // sentido horario. En kurbo, `start_angle` está en radianes; CCW
    // positivo. Tomamos start = 135° (back of bottom-left), sweep
    // negativo para ir CW.
    let start_deg = 135.0_f64;
    let total_sweep_deg = -270.0_f64; // CW
    let active_sweep_deg = total_sweep_deg * (value as f64);

    let center = Point::new(cx, cy);
    let radii = (r, r);

    let track_arc = Arc {
        center,
        radii: radii.into(),
        start_angle: start_deg.to_radians(),
        sweep_angle: total_sweep_deg.to_radians(),
        x_rotation: 0.0,
    };
    let stroke = Stroke::new(stroke_w).with_caps(llimphi_ui::llimphi_raster::kurbo::Cap::Round);
    scene.stroke(&stroke, Affine::IDENTITY, track, None, &track_arc);

    if value > 0.001 {
        let active_arc = Arc {
            center,
            radii: radii.into(),
            start_angle: start_deg.to_radians(),
            sweep_angle: active_sweep_deg.to_radians(),
            x_rotation: 0.0,
        };
        scene.stroke(&stroke, Affine::IDENTITY, active, None, &active_arc);
    }
}
