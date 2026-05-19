//! `ChartViewport` — ventana visible en el dominio de datos.
//!
//! El viewport NO conoce pixeles. Sólo describe qué rango de
//! valores X/Y es visible. La proyección a píxeles la hace
//! [`crate::coord_system::CoordinateSystem`] cuando le pasás
//! el `plot_rect`.
//!
//! Pan y zoom mutan el viewport, no los datos. Esto preserva el
//! P2 zero-alloc: los buffers de DataBuffer / RingBuffer se quedan
//! quietos; sólo cambian cuatro `f64` en el viewport.

use pineal_render::Rect;

/// Rango visible en coordenadas de dominio. `f64` porque ejes
/// temporales con epoch ms se desbordan en `f32`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartViewport {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl ChartViewport {
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> Self {
        debug_assert!(x_max > x_min && y_max > y_min);
        Self { x_min, x_max, y_min, y_max }
    }

    pub fn x_span(&self) -> f64 {
        self.x_max - self.x_min
    }
    pub fn y_span(&self) -> f64 {
        self.y_max - self.y_min
    }

    /// Pan en unidades de **dominio**. Suma dx y dy a ambos
    /// extremos del rango respectivo.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.x_min += dx;
        self.x_max += dx;
        self.y_min += dy;
        self.y_max += dy;
    }

    /// Pan en **píxeles** dado el `plot_rect`. Convierte dx_px →
    /// unidades de dominio usando el span actual / ancho del plot.
    ///
    /// Convención de signos: `dx_px > 0` significa "el mouse se
    /// movió a la derecha", que arrastra el viewport a la
    /// **izquierda** (los datos parecen ir hacia la derecha).
    pub fn pan_pixels(&mut self, dx_px: f32, dy_px: f32, plot: Rect) {
        let dx = -(dx_px as f64) * self.x_span() / plot.w as f64;
        // En la convención canvas (+Y hacia abajo) pero queremos
        // que arrastrar para arriba muestre valores más altos,
        // así que también invertimos Y.
        let dy = (dy_px as f64) * self.y_span() / plot.h as f64;
        self.pan(dx, dy);
    }

    /// Pan en **fracción del viewport**. `fx = 0.5` arrastra medio
    /// span hacia la izquierda. Útil cuando el caller no conoce el
    /// `plot_rect` exacto y trabaja con coords normalizadas
    /// (drag dividido por el ancho de la window).
    pub fn pan_fraction(&mut self, fx: f64, fy: f64) {
        self.pan(-fx * self.x_span(), fy * self.y_span());
    }

    /// Zoom anchor-preserving (sección 5.3 del ARCHITECTURE.md).
    /// `anchor_norm` es la posición del ancla **normalizada al
    /// viewport** en `[0, 1]` por eje (típicamente: la posición
    /// del mouse dentro del plot_rect, normalizada).
    ///
    /// `factor > 1` aleja (zoom out), `< 1` acerca (zoom in).
    pub fn zoom_at(&mut self, factor_x: f64, factor_y: f64, anchor_norm: (f64, f64)) {
        let (ax, ay) = anchor_norm;
        let anchor_x = self.x_min + ax * self.x_span();
        let anchor_y = self.y_min + ay * self.y_span();
        let new_xspan = self.x_span() * factor_x;
        let new_yspan = self.y_span() * factor_y;
        self.x_min = anchor_x - ax * new_xspan;
        self.x_max = self.x_min + new_xspan;
        self.y_min = anchor_y - ay * new_yspan;
        self.y_max = self.y_min + new_yspan;
    }

    /// Zoom uniforme con el mismo factor en X e Y.
    pub fn zoom_uniform(&mut self, factor: f64, anchor_norm: (f64, f64)) {
        self.zoom_at(factor, factor, anchor_norm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_no_cambia_span() {
        let mut v = ChartViewport::new(0.0, 10.0, -1.0, 1.0);
        v.pan(2.0, 0.5);
        assert!((v.x_min - 2.0).abs() < 1e-9);
        assert!((v.x_max - 12.0).abs() < 1e-9);
        assert!((v.x_span() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_in_preserva_anchor() {
        // Zoom in 2× con anchor en el centro: el valor que estaba
        // en el centro sigue en el centro.
        let mut v = ChartViewport::new(0.0, 10.0, 0.0, 10.0);
        v.zoom_uniform(0.5, (0.5, 0.5));
        let new_center_x = v.x_min + v.x_span() * 0.5;
        let new_center_y = v.y_min + v.y_span() * 0.5;
        assert!((new_center_x - 5.0).abs() < 1e-9);
        assert!((new_center_y - 5.0).abs() < 1e-9);
        assert!((v.x_span() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_anchor_esquina() {
        // Anchor en (0,0): la esquina inferior-izquierda no se mueve.
        let mut v = ChartViewport::new(0.0, 10.0, 0.0, 10.0);
        v.zoom_uniform(0.5, (0.0, 0.0));
        assert!((v.x_min - 0.0).abs() < 1e-9);
        assert!((v.y_min - 0.0).abs() < 1e-9);
        assert!((v.x_span() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn pan_pixels_invertido() {
        // Plot de 100px ancho, span de dominio 10. Arrastrar 50px
        // a la derecha = pan dominio -5.
        let mut v = ChartViewport::new(0.0, 10.0, 0.0, 10.0);
        v.pan_pixels(50.0, 0.0, Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!((v.x_min - (-5.0)).abs() < 1e-9);
        assert!((v.x_max - 5.0).abs() < 1e-9);
    }

    #[test]
    fn pan_fraction_es_independiente_de_plot() {
        let mut v = ChartViewport::new(0.0, 10.0, 0.0, 10.0);
        // 50% del span hacia la derecha = viewport se mueve -5 en X.
        v.pan_fraction(0.5, 0.0);
        assert!((v.x_min - (-5.0)).abs() < 1e-9);
        assert!((v.x_max - 5.0).abs() < 1e-9);
    }
}
