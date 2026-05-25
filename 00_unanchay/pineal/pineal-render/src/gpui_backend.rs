//! Backend CPU del trait [`crate::Canvas`] sobre `gpui::Window`.
//!
//! Bajo el feature `gpui`. Traduce los primitivos de Lapaloma a las
//! llamadas nativas de GPUI 0.2 (`paint_quad`, `paint_path`). No
//! introduce dependencia transitiva a gpui en los crates de
//! visualización — éstos siguen hablando contra el trait abstracto;
//! sólo el `Element` GPUI de cada widget importa este módulo.
//!
//! Limitaciones de la implementación CPU:
//! - `push_clip` / `pop_clip` quedan como no-op por ahora — GPUI
//!   maneja content mask via builders de alto nivel; el chart se
//!   apoya en el bounds del Element para no pintar fuera.
//! - `fill_triangle_strip` no implementado (lo necesitan phosphor
//!   y Sankey, que aún no están).
//! - `draw_text` no implementado (axis labels lo necesitan; va con
//!   `WindowTextSystem` en una fase próxima).

use crate::{Canvas, Color, Point, Rect, StrokeStyle};
use gpui::{
    fill, font, hsla, point as gpui_point, px, size as gpui_size, Bounds, Hsla, PathBuilder,
    SharedString, TextRun, Window,
};

/// Adapter que pinta sobre un `&mut Window` de GPUI.
///
/// Vida útil del borrow del window iguala la de la pintura. Construir
/// uno nuevo en cada `paint()` del Element.
pub struct WindowCanvas<'a> {
    window: &'a mut Window,
}

impl<'a> WindowCanvas<'a> {
    pub fn new(window: &'a mut Window) -> Self {
        Self { window }
    }
}

/// Conversión RGB(a) → HSL(a). GPUI consume `Hsla` para casi todo
/// el path. Linear, sin gamma — coincide con la convención del
/// resto del codebase nahual.
pub(crate) fn color_to_hsla(c: Color) -> Hsla {
    let (r, g, b, a) = (c.r, c.g, c.b, c.a);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let delta = max - min;
    if delta.abs() < 1e-6 {
        return hsla(0.0, 0.0, l, a);
    }
    let s = if l < 0.5 { delta / (max + min) } else { delta / (2.0 - max - min) };
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    hsla(h / 6.0, s, l, a)
}

fn to_bounds(r: Rect) -> Bounds<gpui::Pixels> {
    Bounds {
        origin: gpui_point(px(r.x), px(r.y)),
        size: gpui_size(px(r.w), px(r.h)),
    }
}

impl<'a> Canvas for WindowCanvas<'a> {
    fn push_clip(&mut self, _rect: Rect) {
        // Sin clip explícito por ahora. El Element pinta dentro
        // de sus bounds y los painters de pineal respetan el
        // plot_rect en sus proyecciones.
    }
    fn pop_clip(&mut self) {}

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let hsla = color_to_hsla(color);
        self.window.paint_quad(fill(to_bounds(rect), hsla));
    }

    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle) {
        // 4 line segments con PathBuilder en stroke mode.
        let mut pb = PathBuilder::stroke(px(stroke.width));
        pb.move_to(gpui_point(px(rect.x), px(rect.y)));
        pb.line_to(gpui_point(px(rect.right()), px(rect.y)));
        pb.line_to(gpui_point(px(rect.right()), px(rect.bottom())));
        pb.line_to(gpui_point(px(rect.x), px(rect.bottom())));
        pb.close();
        if let Ok(path) = pb.build() {
            self.window.paint_path(path, color_to_hsla(stroke.color));
        }
    }

    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle) {
        let mut pb = PathBuilder::stroke(px(stroke.width));
        pb.move_to(gpui_point(px(a.x), px(a.y)));
        pb.line_to(gpui_point(px(b.x), px(b.y)));
        if let Ok(path) = pb.build() {
            self.window.paint_path(path, color_to_hsla(stroke.color));
        }
    }

    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle) {
        if coords.len() < 4 {
            return; // <2 puntos → no hay segmento
        }
        let mut pb = PathBuilder::stroke(px(stroke.width));
        pb.move_to(gpui_point(px(coords[0]), px(coords[1])));
        let mut i = 2;
        while i + 1 < coords.len() {
            pb.line_to(gpui_point(px(coords[i]), px(coords[i + 1])));
            i += 2;
        }
        if let Ok(path) = pb.build() {
            self.window.paint_path(path, color_to_hsla(stroke.color));
        }
    }

    fn fill_triangle_strip(&mut self, _coords: &[f32], _colors: &[Color]) {
        // TODO: cuando phosphor / Sankey lo necesiten. GPUI no
        // tiene API directa para triangle strips con per-vertex
        // color — habrá que descomponer en quads o subir un
        // vertex buffer wgpu.
    }

    fn draw_text(&mut self, p: Point, text: &str, color: Color, size_px: f32) {
        if text.is_empty() {
            return;
        }
        let hsla = color_to_hsla(color);
        let font_size = px(size_px);
        let text_str: SharedString = text.to_string().into();
        let runs = [TextRun {
            len: text.len(),
            font: font("Monospace"),
            color: hsla,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];

        let shaped = self
            .window
            .text_system()
            .shape_line(text_str, font_size, &runs, None);

        // Iteramos glyphs vía `paint_glyph` para evitar la
        // dependencia con `&mut App` que pide `ShapedLine::paint`.
        // Eso encaja con el contrato actual del Canvas trait que
        // sólo expone `&mut Window`.
        let origin_x = px(p.x);
        let origin_y = px(p.y);
        for run in shaped.runs.iter() {
            for glyph in run.glyphs.iter() {
                let gx = origin_x + glyph.position.x;
                let gy = origin_y + glyph.position.y;
                let _ = self
                    .window
                    .paint_glyph(gpui_point(gx, gy), run.font_id, glyph.id, font_size, hsla);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_a_hsla_grises() {
        // (0.5, 0.5, 0.5) → h=0, s=0, l=0.5
        let h = color_to_hsla(Color::rgb(0.5, 0.5, 0.5));
        assert!((h.s - 0.0).abs() < 1e-6);
        assert!((h.l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rgb_a_hsla_rojo_puro() {
        let h = color_to_hsla(Color::rgb(1.0, 0.0, 0.0));
        // Rojo: h=0, s=1, l=0.5
        assert!((h.h - 0.0).abs() < 1e-6);
        assert!((h.s - 1.0).abs() < 1e-6);
        assert!((h.l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rgb_a_hsla_alpha_pasa_directo() {
        let h = color_to_hsla(Color::rgba(0.0, 0.0, 1.0, 0.3));
        assert!((h.a - 0.3).abs() < 1e-6);
    }
}
