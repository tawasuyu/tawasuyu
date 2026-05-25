//! Backend del trait [`crate::Canvas`] sobre un `vello::Scene` de Llimphi.
//!
//! Bajo el feature `llimphi`. Traduce los primitivos de pineal a las
//! llamadas nativas de vello/peniko/kurbo. No introduce dependencia
//! transitiva a `llimphi-ui` en los crates de visualización — éstos
//! siguen hablando contra el trait abstracto; sólo la función `view`
//! de cada widget enchufa este backend en un `View::paint_with`.
//!
//! Paridad con el backend GPUI:
//! - `push_clip` / `pop_clip` quedan como no-op por ahora — el `View`
//!   contenedor ya recorta vía taffy + `clip(true)`; los painters de
//!   pineal respetan su `plot_rect` y no salen del bounds.
//! - `fill_triangle_strip` no implementado (lo necesitan phosphor y
//!   Sankey, que aún no están). Cuando entren, va con un BezPath o un
//!   batch de quads.
//!
//! Coordenadas: pineal trabaja en pixels absolutos del scene (mismo
//! origen que `PaintRect.{x,y}`). No traduce — los callers ya
//! construyen sus rects en términos del `PaintRect` recibido.
//!
//! El texto se rendea usando el `Typesetter` cacheado del runtime; no
//! creamos `FontContext` por frame.

use crate::{Canvas, Color, Point, Rect, StrokeStyle};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Line, Rect as KurboRect, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color as PenikoColor, Fill};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::llimphi_text::{draw_block, TextBlock, Typesetter};

/// Adapter que pinta sobre un `&mut vello::Scene` + `&mut Typesetter`.
///
/// Construir uno nuevo dentro del closure de [`llimphi_ui::View::paint_with`].
/// Vida útil del borrow del scene iguala la del frame.
pub struct SceneCanvas<'a> {
    scene: &'a mut Scene,
    typesetter: &'a mut Typesetter,
}

impl<'a> SceneCanvas<'a> {
    pub fn new(scene: &'a mut Scene, typesetter: &'a mut Typesetter) -> Self {
        Self { scene, typesetter }
    }
}

/// Convierte el `Color` RGBA lineal de pineal al `Color` peniko. Mantiene
/// la convención sin gamma del resto del codebase. peniko trabaja en
/// `[0, 255]` enteros para el canal alfa también, así que clampeamos y
/// multiplicamos.
fn to_peniko(c: Color) -> PenikoColor {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    PenikoColor::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

fn to_kurbo_rect(r: Rect) -> KurboRect {
    KurboRect::new(
        r.x as f64,
        r.y as f64,
        (r.x + r.w) as f64,
        (r.y + r.h) as f64,
    )
}

impl<'a> Canvas for SceneCanvas<'a> {
    fn push_clip(&mut self, _rect: Rect) {
        // No-op por ahora. El View contenedor recorta vía taffy + clip(true).
    }
    fn pop_clip(&mut self) {}

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            to_peniko(color),
            None,
            &to_kurbo_rect(rect),
        );
    }

    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle) {
        self.scene.stroke(
            &Stroke::new(stroke.width as f64),
            Affine::IDENTITY,
            to_peniko(stroke.color),
            None,
            &to_kurbo_rect(rect),
        );
    }

    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle) {
        let line = Line::new((a.x as f64, a.y as f64), (b.x as f64, b.y as f64));
        self.scene.stroke(
            &Stroke::new(stroke.width as f64),
            Affine::IDENTITY,
            to_peniko(stroke.color),
            None,
            &line,
        );
    }

    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle) {
        if coords.len() < 4 {
            return; // <2 puntos → no hay segmento
        }
        let mut path = BezPath::new();
        path.move_to((coords[0] as f64, coords[1] as f64));
        let mut i = 2;
        while i + 1 < coords.len() {
            path.line_to((coords[i] as f64, coords[i + 1] as f64));
            i += 2;
        }
        self.scene.stroke(
            &Stroke::new(stroke.width as f64),
            Affine::IDENTITY,
            to_peniko(stroke.color),
            None,
            &path,
        );
    }

    fn fill_triangle_strip(&mut self, _coords: &[f32], _colors: &[Color]) {
        // TODO: cuando phosphor / Sankey lo necesiten. Vello no expone
        // mesh con per-vertex color directo — habrá que armar un BezPath
        // por triángulo o subir un buffer custom. Por ahora paridad con
        // gpui_backend: no-op.
    }

    fn draw_text(&mut self, p: Point, text: &str, color: Color, size_px: f32) {
        if text.is_empty() {
            return;
        }
        // Reutiliza el typesetter cacheado del runtime — `TextBlock::simple`
        // pone el origen donde pineal lo pide. `draw_block` hace shape +
        // glyph_run en una pasada.
        let block = TextBlock::simple(text, size_px, to_peniko(color), (p.x as f64, p.y as f64));
        draw_block(self.scene, self.typesetter, &block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rojo_redondea_a_255() {
        let c = to_peniko(Color::rgb(1.0, 0.0, 0.0)).to_rgba8();
        assert_eq!((c.r, c.g, c.b, c.a), (255, 0, 0, 255));
    }

    #[test]
    fn alpha_pasa_directo() {
        let c = to_peniko(Color::rgba(0.0, 0.0, 1.0, 0.25)).to_rgba8();
        assert_eq!(c.b, 255);
        assert_eq!(c.a, 64); // 0.25 * 255 = 63.75 → 64
    }

    #[test]
    fn fuera_de_rango_clampea() {
        let c = to_peniko(Color::rgba(1.5, -0.2, 0.5, 1.0)).to_rgba8();
        assert_eq!((c.r, c.g, c.b), (255, 0, 128));
    }
}
