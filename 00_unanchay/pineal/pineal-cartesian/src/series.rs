//! `Series` — trait que abstrae cualquier dataset visualizable
//! sobre coordenadas cartesianas, + impl [`LineSeries`].
//!
//! La firma es agnóstica de `gpui`: el painter dibuja contra
//! `pineal_render::Canvas`. El Element GPUI envuelve esto y
//! pasa un adaptador del Canvas trait sobre el PaintContext nativo.

use pineal_core::buffer::DataBuffer;
use pineal_core::lttb;
use pineal_render::{Canvas, StrokeStyle};

use crate::coord_system::CoordinateSystem;

/// Hint para la serie sobre el nivel de detalle. A alta densidad
/// (muchos más puntos que pixeles) el painter saltea decoraciones
/// y aplica decimación; a baja densidad pinta marcadores y todo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    HighDensity,
    UiRich,
}

/// Contexto que la serie recibe en cada `paint`.
///
/// Lleva el coord system + el modo + un buffer scratch que el
/// caller mantiene entre frames para evitar allocations.
pub struct PaintCtx<'a> {
    pub cs: CoordinateSystem,
    pub mode: RenderMode,
    /// Buffer scratch reusable (compartido entre series del mismo
    /// chart). El caller hace `clear()` antes de cada serie.
    pub scratch: &'a mut Vec<f32>,
}

pub trait Series {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut dyn Canvas);

    /// Devuelve `Some(point_index)` del sample más cercano al
    /// pixel pasado, si está dentro del threshold de hit (default
    /// 8 px). El default impl asume que el data buffer expuesto
    /// por la serie está sorted por X (caso de [`LineSeries`]).
    fn hit_test(&self, _pixel: pineal_render::Point, _cs: &CoordinateSystem) -> Option<usize> {
        None
    }
}

/// Serie de polilínea simple. Decimación LTTB cuando
/// `data.len() > 3 × plot_width_px`.
pub struct LineSeries<'a> {
    pub data: &'a DataBuffer,
    pub stroke: StrokeStyle,
    /// Si `None`, se usa heurística `target = plot_width × 3`.
    pub lttb_target: Option<usize>,
}

impl<'a> LineSeries<'a> {
    pub fn new(data: &'a DataBuffer, stroke: StrokeStyle) -> Self {
        Self { data, stroke, lttb_target: None }
    }

    pub fn effective_target(&self, plot_w: f32) -> usize {
        self.lttb_target.unwrap_or_else(|| (plot_w as usize).saturating_mul(3))
    }

    /// Materializa las coords proyectadas a pixel space en `out`,
    /// aplicando LTTB cuando densidad > target. `out` se clearea.
    ///
    /// Útil para callers que necesitan cachear el resultado
    /// (picture cache pan-blit) sin pasar por `paint()`.
    pub fn compute_projected(&self, cs: &CoordinateSystem, out: &mut Vec<f32>) {
        out.clear();
        if self.data.len() < 2 {
            return;
        }
        let target = self.effective_target(cs.plot.w);
        if self.data.len() > target {
            let mut idx: Vec<usize> = Vec::with_capacity(target);
            lttb::lttb_indices(self.data.coords(), target, &mut idx);
            let mut decimated: Vec<f32> = Vec::with_capacity(idx.len() * 2);
            for i in idx {
                decimated.push(self.data.coords()[i * 2]);
                decimated.push(self.data.coords()[i * 2 + 1]);
            }
            cs.project_buffer(&decimated, out);
        } else {
            cs.project_buffer(self.data.coords(), out);
        }
    }
}

impl<'a> Series for LineSeries<'a> {
    fn paint(&self, ctx: &mut PaintCtx<'_>, canvas: &mut dyn Canvas) {
        self.compute_projected(&ctx.cs, ctx.scratch);
        if ctx.scratch.len() < 4 {
            return;
        }
        canvas.stroke_polyline(ctx.scratch, self.stroke);
    }

    fn hit_test(&self, pixel: pineal_render::Point, cs: &CoordinateSystem) -> Option<usize> {
        let (target_x, _) = cs.pixel_to_data(pixel);
        let idx = pineal_core::spatial::SpatialIndex::new(self.data.coords())
            .nearest(target_x as f32)?;
        // Threshold de 8px sobre la distancia en pixeles real,
        // no sólo la X — evita match cuando el punto está lejos
        // verticalmente.
        let (dx, dy) = self.data.xy(idx);
        let p = cs.data_to_pixel(dx as f64, dy as f64);
        let dist2 = (p.x - pixel.x).powi(2) + (p.y - pixel.y).powi(2);
        if dist2 <= 64.0 { Some(idx) } else { None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewport::ChartViewport;
    use pineal_render::{Color, Point, Rect, RenderCmd, RenderPlan};

    /// Canvas mock que captura comandos en un `RenderPlan`.
    struct Capture {
        plan: RenderPlan,
    }
    impl Capture {
        fn new() -> Self {
            Self { plan: RenderPlan::new() }
        }
    }
    impl Canvas for Capture {
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
                text: text.into(),
                color,
                size_px,
            });
        }
    }

    fn line_series_pinta_polyline() -> (Capture, usize) {
        let mut buf = DataBuffer::with_capacity(5);
        for i in 0..5 {
            buf.push(i as f32, (i as f32).powi(2));
        }
        let series = LineSeries::new(&buf, StrokeStyle::new(2.0, Color::WHITE));
        let cs = CoordinateSystem::new(
            ChartViewport::new(0.0, 4.0, 0.0, 16.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        let mut scratch = Vec::new();
        let mut ctx = PaintCtx {
            cs,
            mode: RenderMode::UiRich,
            scratch: &mut scratch,
        };
        let mut cap = Capture::new();
        series.paint(&mut ctx, &mut cap);

        let n = cap.plan.cmds.len();
        (cap, n)
    }

    #[test]
    fn line_series_emite_un_solo_drawcall() {
        let (cap, n) = line_series_pinta_polyline();
        assert_eq!(n, 1, "una sola draw call (P3 del ARCHITECTURE.md)");
        match &cap.plan.cmds[0] {
            RenderCmd::StrokePolyline { coords, .. } => {
                assert_eq!(coords.len(), 10, "5 puntos × 2 = 10 floats");
            }
            other => panic!("se esperaba StrokePolyline, se vio {:?}", other),
        }
    }

    #[test]
    fn lttb_se_dispara_con_alta_densidad() {
        // 10k puntos sobre plot de 50px → target = 150 → debe decimar
        let mut buf = DataBuffer::with_capacity(10_000);
        for i in 0..10_000 {
            buf.push(i as f32, (i as f32 * 0.01).sin());
        }
        let series = LineSeries::new(&buf, StrokeStyle::new(1.0, Color::WHITE));
        let cs = CoordinateSystem::new(
            ChartViewport::new(0.0, 9999.0, -1.0, 1.0),
            Rect::new(0.0, 0.0, 50.0, 100.0),
        );
        let mut scratch = Vec::new();
        let mut ctx = PaintCtx {
            cs,
            mode: RenderMode::HighDensity,
            scratch: &mut scratch,
        };
        let mut cap = Capture::new();
        series.paint(&mut ctx, &mut cap);

        match &cap.plan.cmds[0] {
            RenderCmd::StrokePolyline { coords, .. } => {
                // Debe haber muchos menos que 10k puntos (target ≈ 150).
                assert!(coords.len() / 2 <= 160);
                assert!(coords.len() / 2 >= 100);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn line_series_vacio_no_emite() {
        let buf = DataBuffer::new();
        let series = LineSeries::new(&buf, StrokeStyle::new(1.0, Color::WHITE));
        let cs = CoordinateSystem::new(
            ChartViewport::new(0.0, 1.0, 0.0, 1.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        let mut scratch = Vec::new();
        let mut ctx = PaintCtx {
            cs,
            mode: RenderMode::UiRich,
            scratch: &mut scratch,
        };
        let mut cap = Capture::new();
        series.paint(&mut ctx, &mut cap);
        assert_eq!(cap.plan.cmds.len(), 0);
    }

    #[test]
    fn hit_test_acepta_punto_cercano() {
        let mut buf = DataBuffer::with_capacity(5);
        for i in 0..5 {
            buf.push(i as f32, (i as f32).powi(2));
        }
        let series = LineSeries::new(&buf, StrokeStyle::new(1.0, Color::WHITE));
        let cs = CoordinateSystem::new(
            ChartViewport::new(0.0, 4.0, 0.0, 16.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        // Punto (2,4) en data → ¿qué pixel? (2/4)·100=50, (1-4/16)·100=75
        let target = pineal_render::Point::new(50.0, 75.0);
        let hit = series.hit_test(target, &cs);
        assert_eq!(hit, Some(2));
    }

    #[test]
    fn hit_test_rechaza_punto_lejano() {
        let mut buf = DataBuffer::with_capacity(2);
        buf.push(0.0, 0.0);
        buf.push(1.0, 1.0);
        let series = LineSeries::new(&buf, StrokeStyle::new(1.0, Color::WHITE));
        let cs = CoordinateSystem::new(
            ChartViewport::new(0.0, 1.0, 0.0, 1.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        );
        // Pixel muy lejos de la línea.
        let far = pineal_render::Point::new(50.0, 99.0);
        assert!(series.hit_test(far, &cs).is_none());
    }
}
