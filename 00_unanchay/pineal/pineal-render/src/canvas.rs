//! El trait `Canvas` que todos los painters consumen.
//!
//! Mantenemos el set mínimo: line / polyline / rect (fill+stroke) /
//! triangle strip. Cualquier visualización compleja (curvas
//! bezier, gradients) se descompone en estos primitivos por el
//! painter — el backend no necesita entender la semántica.
//!
//! Convención: coordenadas en píxeles del viewport, origen
//! arriba-izquierda, +Y hacia abajo. La proyección de datos→pixel
//! la hace el painter via las escalas de `pineal-core`.

use crate::{Color, Point, Rect};

#[derive(Debug, Clone, Copy)]
pub struct StrokeStyle {
    pub width: f32,
    pub color: Color,
}

impl StrokeStyle {
    pub const fn new(width: f32, color: Color) -> Self {
        Self { width, color }
    }
}

pub trait Canvas {
    /// Clip subsiguiente al rect dado. Stack-discipline:
    /// `push_clip` + draw + `pop_clip`.
    fn push_clip(&mut self, rect: Rect);
    fn pop_clip(&mut self);

    /// Rectángulo relleno (sin stroke).
    fn fill_rect(&mut self, rect: Rect, color: Color);

    /// Rectángulo sólo stroke (sin fill).
    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle);

    /// Línea de a→b.
    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle);

    /// Polilínea sobre coords interleaved `[x0,y0,x1,y1,…]`.
    /// El backend la rendea como un solo draw call cuando puede.
    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle);

    /// Triangle strip rellenado, con un color por vértice
    /// (longitudes deben coincidir: `coords.len()/2 == colors.len()`).
    /// Es lo que usa el phosphor trail y los ribbons Sankey.
    fn fill_triangle_strip(&mut self, coords: &[f32], colors: &[Color]);

    /// Glyph de texto sencillo. El layout va a un text-cache
    /// dentro del backend; por ahora un trazo simple.
    fn draw_text(&mut self, p: Point, text: &str, color: Color, size_px: f32);
}
