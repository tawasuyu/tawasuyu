//! Pie / donut chart.
//!
//! Las porciones arrancan a las 12 en punto (-90°) y avanzan en sentido
//! horario. Como el `Canvas` no tiene primitiva de arco, cada cuña se
//! tesela en un triangle strip; la calidad del arco escala con el ángulo.

use pineal_render::{Canvas, Color, Point};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Una porción del pie: un valor (peso) y su color.
#[derive(Debug, Clone, Copy)]
pub struct Slice {
    pub value: f32,
    pub color: Color,
}

impl Slice {
    pub fn new(value: f32, color: Color) -> Self {
        Self { value, color }
    }
}

/// Segmentos de arco por vuelta completa — controla la suavidad.
const ARC_SEGMENTS_PER_TURN: f32 = 96.0;

/// Dibuja un pie centrado en `center`. Si `inner_radius > 0` es un donut.
/// Los valores negativos se tratan como 0.
pub fn paint_pie(
    slices: &[Slice],
    center: Point,
    radius: f32,
    inner_radius: f32,
    canvas: &mut dyn Canvas,
) {
    let total: f32 = slices.iter().map(|s| s.value.max(0.0)).sum();
    if total <= 0.0 || radius <= 0.0 {
        return;
    }
    let mut angle = -FRAC_PI_2;
    for s in slices {
        let sweep = (s.value.max(0.0) / total) * TAU;
        if sweep > 0.0 {
            paint_wedge(center, radius, inner_radius.max(0.0), angle, angle + sweep, s.color, canvas);
        }
        angle += sweep;
    }
}

fn arc_point(center: Point, r: f32, angle: f32) -> Point {
    Point::new(center.x + r * angle.cos(), center.y + r * angle.sin())
}

fn paint_wedge(
    center: Point,
    r_out: f32,
    r_in: f32,
    a0: f32,
    a1: f32,
    color: Color,
    canvas: &mut dyn Canvas,
) {
    let segs = ((a1 - a0).abs() / TAU * ARC_SEGMENTS_PER_TURN).ceil() as usize;
    let segs = segs.max(1);
    let mut coords = Vec::with_capacity((segs + 1) * 4);
    let mut colors = Vec::with_capacity((segs + 1) * 2);
    for i in 0..=segs {
        let t = a0 + (a1 - a0) * (i as f32 / segs as f32);
        // Borde interno: el centro (pie) o el arco interno (donut).
        let inner = if r_in <= 0.0 {
            center
        } else {
            arc_point(center, r_in, t)
        };
        let outer = arc_point(center, r_out, t);
        coords.push(inner.x);
        coords.push(inner.y);
        coords.push(outer.x);
        coords.push(outer.y);
        colors.push(color);
        colors.push(color);
    }
    // Strip [in0,out0,in1,out1,…]: cada par de triángulos cubre un segmento.
    canvas.fill_triangle_strip(&coords, &colors);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    fn count_strips(cmds: &[RenderCmd]) -> usize {
        cmds.iter()
            .filter(|c| matches!(c, RenderCmd::FillTriangleStrip { .. }))
            .count()
    }

    #[test]
    fn one_strip_per_nonzero_slice() {
        let slices = [
            Slice::new(1.0, Color::WHITE),
            Slice::new(1.0, Color::BLACK),
            Slice::new(2.0, Color::from_hex(0xff0000)),
        ];
        let mut rec = PlanRecorder::new();
        paint_pie(&slices, Point::new(50.0, 50.0), 40.0, 0.0, &mut rec);
        assert_eq!(count_strips(&rec.into_plan().cmds), 3);
    }

    #[test]
    fn zero_total_draws_nothing() {
        let slices = [Slice::new(0.0, Color::WHITE)];
        let mut rec = PlanRecorder::new();
        paint_pie(&slices, Point::new(50.0, 50.0), 40.0, 0.0, &mut rec);
        assert!(rec.into_plan().cmds.is_empty());
    }

    #[test]
    fn donut_also_emits_strips() {
        let slices = [Slice::new(1.0, Color::WHITE), Slice::new(1.0, Color::BLACK)];
        let mut rec = PlanRecorder::new();
        paint_pie(&slices, Point::new(50.0, 50.0), 40.0, 20.0, &mut rec);
        assert_eq!(count_strips(&rec.into_plan().cmds), 2);
    }
}
