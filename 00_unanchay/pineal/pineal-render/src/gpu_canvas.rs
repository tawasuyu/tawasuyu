//! Backend GPU directo del trait [`crate::Canvas`] — Fase 4 del SDD
//! `02_ruway/llimphi/SDD.md` §"GPU directo wgpu".
//!
//! Implementación drop-in del trait que delega cada primitivo a un
//! `llimphi_raster::GpuBatch`. El painter de pineal no se entera: sigue
//! escribiendo `canvas.fill_rect(...)`, `canvas.stroke_line(...)`, etc.
//! La app que monta la visualización elige el backend al enchufar el
//! `paint_with` (SceneCanvas, vello, ~100 K primitivas) o
//! `gpu_paint_with` (GpuSceneCanvas, GPU directo, 1–10 M primitivas).
//!
//! Trade-offs respecto a `SceneCanvas`:
//!
//! - **Texto**: el SDD prohíbe expresamente texto en este backend
//!   ("Texto siempre por vello+parley"). `draw_text` queda como
//!   **no-op silencioso** — el caller debe pintar labels en un
//!   `View::paint_with` (vello) hermano o en `App::view_overlay`.
//! - **Anchura de stroke**: `GpuBatch` tiene una sola `line_width` por
//!   flush. Visualizaciones que mezclen grosores van a ver "la última
//!   gana". Workaround: emitir cada grosor en su propio flush, o
//!   simplemente componer todo con el grosor más común. Para los
//!   painters densos típicos (starfield, particles, scatter) no aplica
//!   — ahí las strokes son todas del mismo grosor.
//! - **Clip**: `push_clip`/`pop_clip` son no-op, igual que en
//!   `SceneCanvas` (el `View` contenedor recorta vía taffy + `clip(true)`).
//! - **Per-vertex color en strips**: ventaja sobre vello — el GPU sí
//!   soporta color por vértice nativo, así que `fill_triangle_strip`
//!   preserva el color real de cada vértice (no el promedio del
//!   triángulo). Beneficia phosphor trail, ribbons Sankey, fan radar.

use crate::{Canvas, Color, Point, Rect, StrokeStyle};
use llimphi_ui::llimphi_raster::peniko::Color as PenikoColor;
use llimphi_ui::llimphi_raster::GpuBatch;

/// Adapter que pinta primitivos de `Canvas` sobre un `GpuBatch`. El
/// batch lo construye y flushea la app dentro del `gpu_paint_with`;
/// `GpuSceneCanvas` vive sólo durante las calls del painter.
pub struct GpuSceneCanvas<'a, 'b> {
    batch: &'a mut GpuBatch<'b>,
}

impl<'a, 'b> GpuSceneCanvas<'a, 'b> {
    pub fn new(batch: &'a mut GpuBatch<'b>) -> Self {
        Self { batch }
    }
}

fn to_peniko(c: Color) -> PenikoColor {
    let to_byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    PenikoColor::from_rgba8(to_byte(c.r), to_byte(c.g), to_byte(c.b), to_byte(c.a))
}

impl<'a, 'b> Canvas for GpuSceneCanvas<'a, 'b> {
    fn push_clip(&mut self, _rect: Rect) {}
    fn pop_clip(&mut self) {}

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        self.batch
            .add_rect(rect.x, rect.y, rect.w, rect.h, to_peniko(color));
    }

    fn stroke_rect(&mut self, rect: Rect, stroke: StrokeStyle) {
        // 4 segmentos cerrando el rect. La line_width del batch es
        // compartida — actualizamos en cada call (el caller asume "una
        // anchura por flush", ver doc del módulo).
        self.batch.line_width(stroke.width);
        let c = to_peniko(stroke.color);
        let tl = (rect.x, rect.y);
        let tr = (rect.x + rect.w, rect.y);
        let br = (rect.x + rect.w, rect.y + rect.h);
        let bl = (rect.x, rect.y + rect.h);
        self.batch.add_line(tl, tr, c);
        self.batch.add_line(tr, br, c);
        self.batch.add_line(br, bl, c);
        self.batch.add_line(bl, tl, c);
    }

    fn stroke_line(&mut self, a: Point, b: Point, stroke: StrokeStyle) {
        self.batch.line_width(stroke.width);
        self.batch
            .add_line((a.x, a.y), (b.x, b.y), to_peniko(stroke.color));
    }

    fn stroke_polyline(&mut self, coords: &[f32], stroke: StrokeStyle) {
        if coords.len() < 4 {
            return;
        }
        self.batch.line_width(stroke.width);
        let c = to_peniko(stroke.color);
        let mut prev = (coords[0], coords[1]);
        let mut i = 2;
        while i + 1 < coords.len() {
            let cur = (coords[i], coords[i + 1]);
            self.batch.add_line(prev, cur, c);
            prev = cur;
            i += 2;
        }
    }

    fn fill_triangle_strip(&mut self, coords: &[f32], colors: &[Color]) {
        let n = coords.len() / 2;
        if n < 3 {
            return;
        }
        // Expandir strip a tri list, con color real por vértice (no el
        // promedio del backend vello). Cada triángulo del strip toma
        // los índices (t, t+1, t+2).
        for t in 0..n - 2 {
            let i0 = t;
            let i1 = t + 1;
            let i2 = t + 2;
            let p0 = (coords[i0 * 2], coords[i0 * 2 + 1]);
            let p1 = (coords[i1 * 2], coords[i1 * 2 + 1]);
            let p2 = (coords[i2 * 2], coords[i2 * 2 + 1]);
            let c0 = colors.get(i0).copied().unwrap_or(Color::TRANSPARENT);
            let c1 = colors.get(i1).copied().unwrap_or(Color::TRANSPARENT);
            let c2 = colors.get(i2).copied().unwrap_or(Color::TRANSPARENT);
            self.batch
                .add_tri(p0, p1, p2, to_peniko(c0), to_peniko(c1), to_peniko(c2));
        }
    }

    fn draw_text(&mut self, _p: Point, _text: &str, _color: Color, _size_px: f32) {
        // No-op por diseño: el SDD prohíbe texto en este backend. Los
        // labels van por un `paint_with` (vello) hermano o por el
        // overlay del runtime.
    }
}
