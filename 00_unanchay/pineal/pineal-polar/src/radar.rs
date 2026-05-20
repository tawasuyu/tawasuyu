//! Radar (spider) chart.
//!
//! `M` ejes equiespaciados desde las 12 en punto. Cada valor se proyecta
//! a una distancia del centro proporcional a `value / max_value`. El
//! polígono resultante se rellena (triangle fan) y se contornea.

use pineal_render::{Canvas, Color, Point, StrokeStyle};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Dibuja un radar de `values.len()` ejes. `max_value` define el borde.
/// Rellena con `fill` y contornea con `stroke`.
pub fn paint_radar(
    values: &[f32],
    max_value: f32,
    center: Point,
    radius: f32,
    fill: Color,
    stroke: StrokeStyle,
    canvas: &mut dyn Canvas,
) {
    let m = values.len();
    if m < 3 || max_value <= 0.0 || radius <= 0.0 {
        return;
    }

    // Punto de cada eje, en orden.
    let verts: Vec<Point> = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let angle = -FRAC_PI_2 + (i as f32 / m as f32) * TAU;
            let dist = (v / max_value).clamp(0.0, 1.0) * radius;
            Point::new(center.x + dist * angle.cos(), center.y + dist * angle.sin())
        })
        .collect();

    // Relleno: fan como strip [c, v0, c, v1, …, c, v0] (cierra el polígono).
    let mut coords = Vec::with_capacity((m + 1) * 4);
    let mut colors = Vec::with_capacity((m + 1) * 2);
    for i in 0..=m {
        let v = verts[i % m];
        coords.push(center.x);
        coords.push(center.y);
        coords.push(v.x);
        coords.push(v.y);
        colors.push(fill);
        colors.push(fill);
    }
    canvas.fill_triangle_strip(&coords, &colors);

    // Contorno: polilínea cerrada.
    let mut outline = Vec::with_capacity((m + 1) * 2);
    for i in 0..=m {
        let v = verts[i % m];
        outline.push(v.x);
        outline.push(v.y);
    }
    canvas.stroke_polyline(&outline, stroke);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    #[test]
    fn emits_fill_strip_and_outline() {
        let mut rec = PlanRecorder::new();
        paint_radar(
            &[1.0, 2.0, 3.0, 2.0, 1.0],
            3.0,
            Point::new(50.0, 50.0),
            40.0,
            Color::WHITE,
            StrokeStyle::new(1.5, Color::BLACK),
            &mut rec,
        );
        let cmds = rec.into_plan().cmds;
        assert!(cmds.iter().any(|c| matches!(c, RenderCmd::FillTriangleStrip { .. })));
        assert!(cmds.iter().any(|c| matches!(c, RenderCmd::StrokePolyline { .. })));
    }

    #[test]
    fn too_few_axes_draws_nothing() {
        let mut rec = PlanRecorder::new();
        paint_radar(
            &[1.0, 2.0],
            3.0,
            Point::new(0.0, 0.0),
            10.0,
            Color::WHITE,
            StrokeStyle::new(1.0, Color::BLACK),
            &mut rec,
        );
        assert!(rec.into_plan().cmds.is_empty());
    }
}
