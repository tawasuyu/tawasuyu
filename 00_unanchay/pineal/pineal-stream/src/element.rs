//! `LapalomaStreamElement` — Element GPUI para visualización de
//! telemetría con RingBuffer.
//!
//! Modo **sweep** (canónico del osciloscopio):
//! - Los slots `[0, capacity)` tienen `x_norm` fijo precomputado.
//! - El `head` marca el slot donde se va a escribir el próximo
//!   sample → visualmente, el "cursor" que separa la traza vieja
//!   (a la derecha del head) de la nueva (a la izquierda).
//! - Render en **dos segmentos** split-at-head para evitar la
//!   línea horizontal del wraparound. Antes del fill (count <
//!   capacity), sólo se pinta `[0, head)`.

use std::panic;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window,
};

use pineal_core::ring::RingBuffer;
use pineal_render::{Canvas, Color, Rect, StrokeStyle, WindowCanvas};

/// Element que pinta un `RingBuffer` en modo sweep.
pub struct LapalomaStreamElement {
    pub buffer: RingBuffer,
    pub stroke: StrokeStyle,
    pub background: Option<Color>,
    /// Rango Y para proyectar los samples a píxeles. Default `-1..1`.
    pub y_min: f32,
    pub y_max: f32,
    pub padding: f32,
    /// Scratch reusable entre los dos segmentos del frame.
    scratch: Vec<f32>,
}

impl LapalomaStreamElement {
    pub fn new(buffer: RingBuffer, stroke: StrokeStyle) -> Self {
        Self {
            buffer,
            stroke,
            background: None,
            y_min: -1.0,
            y_max: 1.0,
            padding: 8.0,
            scratch: Vec::new(),
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

    fn plot_rect(&self, bounds: Rect) -> Rect {
        Rect::new(
            bounds.x + self.padding,
            bounds.y + self.padding,
            (bounds.w - self.padding * 2.0).max(1.0),
            (bounds.h - self.padding * 2.0).max(1.0),
        )
    }

}

/// Proyecta una slice de coords `[x_norm, y_value, …]` del
/// RingBuffer al sistema de píxeles del plot. `out` se extiende
/// (no se clearea acá; el caller decide).
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

impl IntoElement for LapalomaStreamElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LapalomaStreamElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        style.size.height = gpui::Length::Definite(gpui::DefiniteLength::Fraction(1.0));
        let id = window.request_layout(style, [], cx);
        (id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let ox: f32 = bounds.origin.x.into();
        let oy: f32 = bounds.origin.y.into();
        let w: f32 = bounds.size.width.into();
        let h: f32 = bounds.size.height.into();
        let outer = Rect::new(ox, oy, w, h);
        let plot = self.plot_rect(outer);

        let mut canvas = WindowCanvas::new(window);

        if let Some(bg) = self.background {
            canvas.fill_rect(outer, bg);
        }

        let coords = self.buffer.coords();
        let head = self.buffer.head();
        let cap = self.buffer.capacity();

        if !self.buffer.is_full() {
            // Pre-fill: sólo [0, head). Evita la línea plana del
            // 1.0.2 fix del Flutter doc.
            let filled = head;
            if filled >= 2 {
                let slice = &coords[..filled * 2];
                self.scratch.clear();
                project_segment(slice, plot, self.y_min, self.y_max, &mut self.scratch);
                canvas.stroke_polyline(&self.scratch, self.stroke);
            }
            return;
        }

        // Ya filled — dos segmentos split-at-head.
        let split = head * 2;
        // Segmento "viejo": [head*2 .. cap*2) — temporalmente más antiguo.
        if split < cap * 2 {
            let seg1 = &coords[split..];
            if seg1.len() >= 4 {
                self.scratch.clear();
                project_segment(seg1, plot, self.y_min, self.y_max, &mut self.scratch);
                canvas.stroke_polyline(&self.scratch, self.stroke);
            }
        }
        // Segmento "nuevo": [0 .. head*2) — más reciente, dibujado a
        // la izquierda del cursor.
        if split > 0 {
            let seg2 = &coords[..split];
            if seg2.len() >= 4 {
                self.scratch.clear();
                project_segment(seg2, plot, self.y_min, self.y_max, &mut self.scratch);
                canvas.stroke_polyline(&self.scratch, self.stroke);
            }
        }
    }
}

/// Helper builder-style.
pub fn pineal_stream(buffer: RingBuffer, stroke: StrokeStyle) -> LapalomaStreamElement {
    LapalomaStreamElement::new(buffer, stroke)
}
