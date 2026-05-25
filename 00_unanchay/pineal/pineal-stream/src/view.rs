//! Vista Llimphi del widget de telemetría (sweep oscilloscope).
//!
//! Reemplazo del Element GPUI: dada un `RingBuffer` + `StrokeStyle`,
//! devuelve un `View<Msg>` que pinta la traza vía `paint_with` sobre el
//! scene de vello. La lógica de proyección (dos segmentos split-at-head,
//! padding, mapeo y_range → px) es la misma que el Element original;
//! sólo cambia el backend.
//!
//! Llimphi reconstruye el View por frame, así que `scratch` queda dentro
//! del closure: cada paint asigna un Vec local. Para capacidades chicas
//! (cap ≤ 4 K) el costo es despreciable; el día que importe se cachea
//! en el Model del host con un `RefCell`.

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;

use pineal_core::ring::RingBuffer;
use pineal_render::{Color, Rect, SceneCanvas, StrokeStyle};

/// Mismas defaults que `LapalomaStreamElement`. Builder chico sobre la
/// función `pineal_stream_view` para no hacer la signature larga.
#[derive(Debug, Clone)]
pub struct StreamView {
    buffer: RingBuffer,
    stroke: StrokeStyle,
    background: Option<Color>,
    y_min: f32,
    y_max: f32,
    padding: f32,
}

impl StreamView {
    pub fn new(buffer: RingBuffer, stroke: StrokeStyle) -> Self {
        Self {
            buffer,
            stroke,
            background: None,
            y_min: -1.0,
            y_max: 1.0,
            padding: 8.0,
        }
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn y_range(mut self, min: f32, max: f32) -> Self {
        debug_assert!(max > min);
        self.y_min = min;
        self.y_max = max;
        self
    }

    pub fn padding(mut self, px: f32) -> Self {
        self.padding = px;
        self
    }

    /// Materializa la vista. Devuelve un nodo que ocupa el 100% del padre
    /// y pinta la traza dentro de su rect. Llamar por frame; el costo de
    /// reconstruir es despreciable (los campos son `Copy`/`Clone` chicos).
    pub fn view<Msg: Clone + 'static>(self) -> View<Msg> {
        let StreamView { buffer, stroke, background, y_min, y_max, padding } = self;
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .paint_with(move |scene, typesetter, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let plot = plot_rect(outer, padding);

            let mut canvas = SceneCanvas::new(scene, typesetter);

            if let Some(bg) = background {
                use pineal_render::Canvas as _;
                canvas.fill_rect(outer, bg);
            }

            paint_traza(&mut canvas, &buffer, plot, stroke, y_min, y_max);
        })
    }
}

fn plot_rect(bounds: Rect, padding: f32) -> Rect {
    Rect::new(
        bounds.x + padding,
        bounds.y + padding,
        (bounds.w - padding * 2.0).max(1.0),
        (bounds.h - padding * 2.0).max(1.0),
    )
}

/// Proyecta `[x_norm, y_value, …]` del RingBuffer al sistema de píxeles
/// del plot. `out` se extiende — el caller decide el clear.
fn project_segment(segment: &[f32], plot: Rect, y_min: f32, y_max: f32, out: &mut Vec<f32>) {
    let y_span = y_max - y_min;
    if y_span.abs() < 1e-9 {
        return;
    }
    let inv_y_span = 1.0 / y_span;
    for chunk in segment.chunks_exact(2) {
        let xn = chunk[0];
        let yv = chunk[1];
        let py_norm = (yv - y_min) * inv_y_span;
        let px = plot.x + xn * plot.w;
        let py = plot.bottom() - py_norm * plot.h;
        out.push(px);
        out.push(py);
    }
}

/// Render canónico del modo sweep, mismo split-at-head que el Element.
fn paint_traza(
    canvas: &mut SceneCanvas<'_>,
    buffer: &RingBuffer,
    plot: Rect,
    stroke: StrokeStyle,
    y_min: f32,
    y_max: f32,
) {
    use pineal_render::Canvas as _;

    let coords = buffer.coords();
    let head = buffer.head();
    let cap = buffer.capacity();
    let mut scratch: Vec<f32> = Vec::new();

    if !buffer.is_full() {
        let filled = head;
        if filled >= 2 {
            let slice = &coords[..filled * 2];
            scratch.clear();
            project_segment(slice, plot, y_min, y_max, &mut scratch);
            canvas.stroke_polyline(&scratch, stroke);
        }
        return;
    }

    let split = head * 2;
    if split < cap * 2 {
        let seg1 = &coords[split..];
        if seg1.len() >= 4 {
            scratch.clear();
            project_segment(seg1, plot, y_min, y_max, &mut scratch);
            canvas.stroke_polyline(&scratch, stroke);
        }
    }
    if split > 0 {
        let seg2 = &coords[..split];
        if seg2.len() >= 4 {
            scratch.clear();
            project_segment(seg2, plot, y_min, y_max, &mut scratch);
            canvas.stroke_polyline(&scratch, stroke);
        }
    }
}

/// Helper builder-style — paralelo al `pineal_stream(...)` del Element GPUI.
pub fn pineal_stream_view(buffer: RingBuffer, stroke: StrokeStyle) -> StreamView {
    StreamView::new(buffer, stroke)
}
