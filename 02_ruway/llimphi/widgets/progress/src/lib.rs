//! `llimphi-widget-progress` — progreso determinado, lineal o radial.
//!
//! Determinado = la app conoce el porcentaje (`0.0..=1.0`). Para
//! progreso indeterminado (la op está corriendo, no sé cuánto falta),
//! usar `llimphi-widget-spinner`.
//!
//! Dos formas:
//! - [`linear_progress_view`] — barra horizontal con relleno proporcional.
//! - [`radial_progress_view`] — anillo cuya porción llena indica el avance.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Arc, Cap, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::radius;

/// Barra horizontal: una pista (`track`) con un fill proporcional al
/// `progress` (0.0..=1.0) pintado encima.
pub fn linear_progress_view<Msg: Clone + 'static>(
    progress: f32,
    track_color: Color,
    fill_color: Color,
    height_px: f32,
) -> View<Msg> {
    let p = progress.clamp(0.0, 1.0);
    let fill = View::new(Style {
        position: Position::Absolute,
        inset: Rect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: llimphi_ui::llimphi_layout::taffy::prelude::auto(),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(p),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .fill(fill_color)
    .radius(radius::XS);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(height_px),
        },
        ..Default::default()
    })
    .fill(track_color)
    .radius(radius::XS)
    .children(vec![fill])
}

/// Anillo cuya porción angular llena indica el avance. Empieza desde
/// arriba (12 en punto) y gira en sentido horario, igual que la
/// convención de relojes y muchos progress radiales.
pub fn radial_progress_view<Msg: Clone + 'static>(
    progress: f32,
    track_color: Color,
    fill_color: Color,
    stroke_width_ratio: f32,
) -> View<Msg> {
    let p = progress.clamp(0.0, 1.0);
    let sw = stroke_width_ratio;
    View::new(Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        let side = rect.w.min(rect.h) as f64;
        if side <= 0.0 {
            return;
        }
        let cx = rect.x as f64 + rect.w as f64 * 0.5;
        let cy = rect.y as f64 + rect.h as f64 * 0.5;
        let stroke_w = (side * sw as f64).max(1.0);
        let radius = (side - stroke_w) * 0.5;
        let stroke = Stroke::new(stroke_w).with_caps(Cap::Round);

        // Track completo (anillo gris).
        let track = Arc::new((cx, cy), (radius, radius), 0.0, std::f64::consts::TAU, 0.0);
        scene.stroke(&stroke, Affine::IDENTITY, track_color, None, &track);

        // Arco lleno — arranca en -π/2 (12 en punto) y barre `p * 2π`
        // en sentido horario (positivo en el sistema y-down de vello).
        if p > 0.0 {
            let theta0 = -std::f64::consts::FRAC_PI_2;
            let sweep = std::f64::consts::TAU * p as f64;
            let fill_arc = Arc::new((cx, cy), (radius, radius), theta0, sweep, 0.0);
            scene.stroke(&stroke, Affine::IDENTITY, fill_color, None, &fill_arc);
        }
    })
}
