//! `CoordinateSystem` — proyección dominio ↔ pixel.
//!
//! Compone `ChartViewport` (qué se ve) + `plot_rect` (dónde, en
//! píxeles) en una transformación afín. La invocación es
//! pointwise; no toca los buffers de datos.
//!
//! Convención Y: +Y de pantalla apunta abajo; +Y de datos arriba.
//! La proyección invierte Y para que un valor alto quede arriba.

use crate::viewport::ChartViewport;
use pineal_render::{Point, Rect};

#[derive(Debug, Clone, Copy)]
pub struct CoordinateSystem {
    pub viewport: ChartViewport,
    pub plot: Rect,
}

impl CoordinateSystem {
    pub fn new(viewport: ChartViewport, plot: Rect) -> Self {
        Self { viewport, plot }
    }

    /// `(value_x, value_y)` → `(pixel_x, pixel_y)`.
    pub fn data_to_pixel(&self, x: f64, y: f64) -> Point {
        let nx = (x - self.viewport.x_min) / self.viewport.x_span();
        let ny = (y - self.viewport.y_min) / self.viewport.y_span();
        let px = self.plot.x + nx as f32 * self.plot.w;
        // +Y de datos = arriba → restar de bottom.
        let py = self.plot.bottom() - ny as f32 * self.plot.h;
        Point::new(px, py)
    }

    /// `(pixel_x, pixel_y)` → `(value_x, value_y)`.
    /// Usado para hit-test y tooltip-on-hover.
    pub fn pixel_to_data(&self, p: Point) -> (f64, f64) {
        let nx = ((p.x - self.plot.x) / self.plot.w) as f64;
        let ny = ((self.plot.bottom() - p.y) / self.plot.h) as f64;
        let x = self.viewport.x_min + nx * self.viewport.x_span();
        let y = self.viewport.y_min + ny * self.viewport.y_span();
        (x, y)
    }

    /// Proyecta un buffer entero de coords interleaved
    /// `[x, y, x, y, …]` (en dominio) a `[px, py, px, py, …]`
    /// (en píxeles), escribiendo a `out` sin allocar.
    ///
    /// El caller debe hacer `out.clear()` previo si quiere reuso
    /// del buffer; este método sólo extiende.
    pub fn project_buffer(&self, data: &[f32], out: &mut Vec<f32>) {
        debug_assert!(data.len() % 2 == 0);
        // Factorizamos para evitar la división por iteración.
        let sx = self.plot.w / self.viewport.x_span() as f32;
        let sy = self.plot.h / self.viewport.y_span() as f32;
        let tx = self.plot.x - self.viewport.x_min as f32 * sx;
        let ty = self.plot.bottom() + self.viewport.y_min as f32 * sy;

        out.reserve(data.len());
        let mut i = 0;
        while i < data.len() {
            let px = data[i] * sx + tx;
            let py = ty - data[i + 1] * sy;
            out.push(px);
            out.push(py);
            i += 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CoordinateSystem {
        // Viewport [0..10, 0..10] sobre plot de 100×100 en (0, 0).
        CoordinateSystem::new(
            ChartViewport::new(0.0, 10.0, 0.0, 10.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
        )
    }

    #[test]
    fn data_to_pixel_origen() {
        let cs = fixture();
        // (0,0) data → (0, 100) pixel (bottom-left del plot).
        let p = cs.data_to_pixel(0.0, 0.0);
        assert!((p.x - 0.0).abs() < 1e-6);
        assert!((p.y - 100.0).abs() < 1e-6);
    }

    #[test]
    fn data_to_pixel_centro() {
        let cs = fixture();
        // (5, 5) → (50, 50)
        let p = cs.data_to_pixel(5.0, 5.0);
        assert!((p.x - 50.0).abs() < 1e-6);
        assert!((p.y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn data_to_pixel_top_right() {
        let cs = fixture();
        // (10, 10) → (100, 0)
        let p = cs.data_to_pixel(10.0, 10.0);
        assert!((p.x - 100.0).abs() < 1e-6);
        assert!((p.y - 0.0).abs() < 1e-6);
    }

    #[test]
    fn pixel_to_data_roundtrip() {
        let cs = fixture();
        for (x, y) in [(2.5_f64, 7.5), (0.0, 0.0), (10.0, 10.0), (3.14, 1.59)] {
            let p = cs.data_to_pixel(x, y);
            let (x2, y2) = cs.pixel_to_data(p);
            assert!((x - x2).abs() < 1e-4, "x roundtrip: {} vs {}", x, x2);
            assert!((y - y2).abs() < 1e-4, "y roundtrip: {} vs {}", y, y2);
        }
    }

    #[test]
    fn project_buffer_consistente_con_pointwise() {
        let cs = fixture();
        let data: Vec<f32> = vec![0.0, 0.0, 5.0, 5.0, 10.0, 10.0];
        let mut out = Vec::new();
        cs.project_buffer(&data, &mut out);
        assert_eq!(out.len(), data.len());
        for i in 0..3 {
            let expected = cs.data_to_pixel(data[i * 2] as f64, data[i * 2 + 1] as f64);
            assert!((out[i * 2] - expected.x).abs() < 1e-4);
            assert!((out[i * 2 + 1] - expected.y).abs() < 1e-4);
        }
    }
}
