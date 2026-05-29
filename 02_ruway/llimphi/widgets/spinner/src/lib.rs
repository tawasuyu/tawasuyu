//! `llimphi-widget-spinner` — spinner circular animado por reloj absoluto.
//!
//! El paint usa `Instant::now()` para calcular el ángulo de rotación,
//! así no hace falta que la app guarde un tween ni dispatchee ticks:
//! cuando llimphi-ui rasterize un frame (porque algo cambió en el
//! modelo o porque la app pidió un repaint), el spinner se ve girando.
//!
//! **Nota**: el spinner sólo se anima si HAY frames. Una app idle no
//! repintará por sí sola — usar `Handle::spawn_periodic(50ms, …)`
//! mientras el spinner esté visible para forzar redraw. O conectar
//! el spinner a un `Tween` y leer su `progress()` desde la `view`.
//!
//! Diseño visual: arco de 270° con stroke variable (más grueso al
//! frente del giro, más fino atrás) para dar sensación de aceleración.

#![forbid(unsafe_code)]

use std::time::Instant;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, Arc, Cap, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;

/// Construye el `View` que pinta un spinner circular animado dentro
/// del rect del padre.
///
/// - `color`: tinte del arco (típico: `theme.accent`).
/// - `stroke_width_ratio`: grosor del arco como fracción del lado
///   menor (0.10 = 10%). Default razonable es `0.12`.
/// - `speed_rev_per_sec`: revoluciones por segundo. Default `1.0`.
pub fn spinner_view<Msg: Clone + 'static>(
    color: Color,
    stroke_width_ratio: f32,
    speed_rev_per_sec: f32,
) -> View<Msg> {
    // Anchor temporal: arrancamos el reloj al construir el View. Como
    // la closure se evalúa por frame, cada repintado calcula `elapsed`
    // contra este origen — sin tween, sin model state.
    let started = Instant::now();
    let sw = stroke_width_ratio;
    let speed = speed_rev_per_sec;
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
        let elapsed = started.elapsed().as_secs_f64();
        // Ángulo de inicio del arco — gira completamente cada `1/speed` s.
        let theta0 = elapsed * speed as f64 * std::f64::consts::TAU;
        // Arco de 270° (= 3π/2 rad) — la "abertura" sugiere movimiento.
        let sweep = std::f64::consts::PI * 1.5;
        let arc = Arc::new((cx, cy), (radius, radius), theta0, sweep, 0.0);
        let stroke = Stroke::new(stroke_w).with_caps(Cap::Round);
        scene.stroke(&stroke, Affine::IDENTITY, color, None, &arc);
    })
}
