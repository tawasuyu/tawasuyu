//! `PlanRecorder` — un [`Canvas`] que graba cada llamada como `RenderCmd`
//! en un [`RenderPlan`], en vez de dibujar.
//!
//! Es el puente entre los painters (que hablan contra `Canvas`) y los
//! backends diferidos: `pineal-export` consume el plan grabado y emite
//! SVG; los tests de snapshot comparan planes.

use crate::{Canvas, Color, Point, Rect, RenderCmd, RenderPlan, StrokeStyle};

/// Canvas que materializa todo lo dibujado en un `RenderPlan`.
#[derive(Debug, Default)]
pub struct PlanRecorder {
    plan: RenderPlan,
}

impl PlanRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume el recorder y devuelve el plan acumulado.
    pub fn into_plan(self) -> RenderPlan {
        self.plan
    }

    /// Acceso de sólo-lectura al plan en construcción.
    pub fn plan(&self) -> &RenderPlan {
        &self.plan
    }
}

impl Canvas for PlanRecorder {
    fn push_clip(&mut self, rect: Rect) {
        self.plan.push(RenderCmd::PushClip(rect));
    }

    fn pop_clip(&mut self) {
        self.plan.push(RenderCmd::PopClip);
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.plan.push(RenderCmd::FillRect { rect, color });
    }

    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle) {
        self.plan.push(RenderCmd::StrokeRect { rect, stroke });
    }

    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle) {
        self.plan.push(RenderCmd::StrokeLine { a, b, stroke });
    }

    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle) {
        self.plan.push(RenderCmd::StrokePolyline {
            coords: coords.to_vec(),
            stroke,
        });
    }

    fn fill_triangle_strip(&mut self, coords: &[f32], colors: &[Color]) {
        self.plan.push(RenderCmd::FillTriangleStrip {
            coords: coords.to_vec(),
            colors: colors.to_vec(),
        });
    }

    fn draw_text(&mut self, p: Point, text: &str, color: Color, size_px: f32) {
        self.plan.push(RenderCmd::DrawText {
            p,
            text: text.to_string(),
            color,
            size_px,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_calls_in_order() {
        let mut rec = PlanRecorder::new();
        rec.fill_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::WHITE);
        rec.stroke_line(
            Point::new(0.0, 0.0),
            Point::new(10.0, 10.0),
            StrokeStyle::new(1.0, Color::BLACK),
        );
        let plan = rec.into_plan();
        assert_eq!(plan.cmds.len(), 2);
        assert!(matches!(plan.cmds[0], RenderCmd::FillRect { .. }));
        assert!(matches!(plan.cmds[1], RenderCmd::StrokeLine { .. }));
    }
}
