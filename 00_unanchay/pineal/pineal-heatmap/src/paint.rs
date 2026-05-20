//! Painter agnóstico: dibuja una `HeatmapMatrix` contra un `Canvas`.
//!
//! Emite un `fill_rect` por celda. Apto para matrices chicas y para
//! export SVG. Para matrices grandes el backend GPUI usa
//! [`crate::encoder`] + textura en vez de este camino.

use crate::matrix::HeatmapMatrix;
use crate::palette::Ramp;
use pineal_render::{Canvas, Rect};

/// Dibuja `matrix` dentro de `area`, una celda = un `fill_rect`.
/// Los valores se normalizan por min/max de la matriz.
pub fn paint(matrix: &HeatmapMatrix, ramp: Ramp, area: Rect, canvas: &mut dyn Canvas) {
    let (w, h) = (matrix.width(), matrix.height());
    if w == 0 || h == 0 {
        return;
    }
    let (min, max) = matrix.min_max();
    let span = max - min;
    let cell_w = area.w / w as f32;
    let cell_h = area.h / h as f32;

    for y in 0..h {
        for x in 0..w {
            let v = matrix.get(x, y);
            let t = if span > 0.0 { (v - min) / span } else { 0.0 };
            let color = ramp.sample(t);
            let rect = Rect::new(
                area.x + x as f32 * cell_w,
                area.y + y as f32 * cell_h,
                cell_w,
                cell_h,
            );
            canvas.fill_rect(rect, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pineal_render::{PlanRecorder, RenderCmd};

    #[test]
    fn emits_one_fill_rect_per_cell() {
        let m = HeatmapMatrix::from_data(vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0], 3, 2).unwrap();
        let mut rec = PlanRecorder::new();
        paint(&m, Ramp::Viridis, Rect::new(0.0, 0.0, 300.0, 200.0), &mut rec);
        let plan = rec.into_plan();
        assert_eq!(plan.cmds.len(), 6);
        assert!(plan.cmds.iter().all(|c| matches!(c, RenderCmd::FillRect { .. })));
    }

    #[test]
    fn empty_matrix_emits_nothing() {
        let m = HeatmapMatrix::new(0, 0);
        let mut rec = PlanRecorder::new();
        paint(&m, Ramp::Viridis, Rect::new(0.0, 0.0, 10.0, 10.0), &mut rec);
        assert!(rec.into_plan().cmds.is_empty());
    }
}
