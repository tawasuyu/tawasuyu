//! `LapalomaPhosphorElement` — Element GPUI con trail CRT.
//!
//! El render pinta el RingBuffer como N segmentos polilíneas con
//! alpha decreciente del más nuevo al más viejo. Wraparound se
//! parte en dos sub-polilíneas para no introducir la línea
//! horizontal "del slot cap-1 al slot 0".

use std::panic;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window,
};

use pineal_core::ring::RingBuffer;
use pineal_render::{Canvas, Color, Rect, StrokeStyle, WindowCanvas};

/// Cantidad de tramos del trail. Más tramos = gradiente más suave,
/// más draw calls. 16 cubre la mayoría de los casos sin ser caro.
const DEFAULT_TRAIL_SEGMENTS: usize = 16;

pub struct LapalomaPhosphorElement {
    pub buffer: RingBuffer,
    /// Color y ancho base. El alpha se modula por tramo;
    /// `base_stroke.color.a` es el alpha máximo (cabeza del trail).
    pub base_stroke: StrokeStyle,
    pub background: Option<Color>,
    pub y_min: f32,
    pub y_max: f32,
    pub padding: f32,
    pub trail_segments: usize,
    /// Si > 0, se aplica una pasada adicional con `width × glow_width_mult`
    /// y `alpha × glow_alpha` debajo del trazo principal — efecto halo CRT.
    pub glow_width_mult: f32,
    pub glow_alpha: f32,
    scratch: Vec<f32>,
}

impl LapalomaPhosphorElement {
    pub fn new(buffer: RingBuffer, base_stroke: StrokeStyle) -> Self {
        Self {
            buffer,
            base_stroke,
            background: None,
            y_min: -1.0,
            y_max: 1.0,
            padding: 8.0,
            trail_segments: DEFAULT_TRAIL_SEGMENTS,
            glow_width_mult: 3.0,
            glow_alpha: 0.25,
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

    pub fn trail_segments(mut self, n: usize) -> Self {
        self.trail_segments = n.max(2);
        self
    }

    pub fn glow(mut self, width_mult: f32, alpha: f32) -> Self {
        self.glow_width_mult = width_mult;
        self.glow_alpha = alpha;
        self
    }

    pub fn no_glow(mut self) -> Self {
        self.glow_width_mult = 0.0;
        self.glow_alpha = 0.0;
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

impl IntoElement for LapalomaPhosphorElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LapalomaPhosphorElement {
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

        let filled = self.buffer.filled_len();
        if filled < 2 {
            return;
        }

        let cap = self.buffer.capacity();
        let head = self.buffer.head();
        let coords = self.buffer.coords();

        // El slot del sample temporalmente más viejo:
        // - is_full → `head` (el siguiente a sobrescribirse).
        // - !is_full → 0.
        let start_slot = if self.buffer.is_full() { head } else { 0 };

        let n_segs = self.trail_segments.min(filled / 2).max(2);
        let base_per_seg = filled / n_segs;
        let glow_enabled = self.glow_alpha > 0.0 && self.glow_width_mult > 1.0;

        for k in 0..n_segs {
            // Rango temporal del segmento. El segmento `k` cubre los
            // samples [k*base_per_seg, (k+1)*base_per_seg). El último
            // incluye el remainder.
            let t_lo = k * base_per_seg;
            let t_hi = if k == n_segs - 1 {
                filled
            } else {
                // +1 incluye el primer sample del siguiente segmento
                // para que las polilíneas se "toquen" sin gap visual.
                ((k + 1) * base_per_seg) + 1
            };
            if t_hi <= t_lo + 1 {
                continue;
            }

            // Alpha decrece linealmente del más nuevo al más viejo.
            // k = n_segs - 1 → 1.0; k = 0 → 1/n_segs.
            let life = (k as f32 + 1.0) / n_segs as f32;
            let alpha = self.base_stroke.color.a * life;
            let mut color = self.base_stroke.color;
            color.a = alpha;
            let stroke = StrokeStyle::new(self.base_stroke.width, color);

            // Glow underneath, mismo path con más ancho y menos alpha.
            let glow_stroke = if glow_enabled {
                let mut gc = color;
                gc.a *= self.glow_alpha;
                Some(StrokeStyle::new(
                    self.base_stroke.width * self.glow_width_mult,
                    gc,
                ))
            } else {
                None
            };

            // Proyectar el rango temporal a slots físicos, partiendo
            // si cruzamos el final del buffer.
            let seg_len = t_hi - t_lo;
            let abs_start = (start_slot + t_lo) % cap;
            let contiguous_len = cap - abs_start;

            if seg_len <= contiguous_len {
                let slice = &coords[abs_start * 2..(abs_start + seg_len) * 2];
                self.scratch.clear();
                project_segment(slice, plot, self.y_min, self.y_max, &mut self.scratch);
                if self.scratch.len() >= 4 {
                    if let Some(gs) = glow_stroke {
                        canvas.stroke_polyline(&self.scratch, gs);
                    }
                    canvas.stroke_polyline(&self.scratch, stroke);
                }
            } else {
                // Wraparound: dos sub-polilíneas separadas.
                let slice_a = &coords[abs_start * 2..];
                self.scratch.clear();
                project_segment(slice_a, plot, self.y_min, self.y_max, &mut self.scratch);
                if self.scratch.len() >= 4 {
                    if let Some(gs) = glow_stroke {
                        canvas.stroke_polyline(&self.scratch, gs);
                    }
                    canvas.stroke_polyline(&self.scratch, stroke);
                }

                let remaining = seg_len - contiguous_len;
                let slice_b = &coords[..remaining * 2];
                self.scratch.clear();
                project_segment(slice_b, plot, self.y_min, self.y_max, &mut self.scratch);
                if self.scratch.len() >= 4 {
                    if let Some(gs) = glow_stroke {
                        canvas.stroke_polyline(&self.scratch, gs);
                    }
                    canvas.stroke_polyline(&self.scratch, stroke);
                }
            }
        }
    }
}

/// Helper builder.
pub fn pineal_phosphor(
    buffer: RingBuffer,
    base_stroke: StrokeStyle,
) -> LapalomaPhosphorElement {
    LapalomaPhosphorElement::new(buffer, base_stroke)
}

/// Proyecta `[x_norm, y_value, …]` del ring a píxeles del plot.
fn project_segment(segment: &[f32], plot: Rect, y_min: f32, y_max: f32, out: &mut Vec<f32>) {
    let y_span = y_max - y_min;
    if y_span.abs() < 1e-9 {
        return;
    }
    let inv = 1.0 / y_span;
    for chunk in segment.chunks_exact(2) {
        let xn = chunk[0];
        let yv = chunk[1];
        let py_norm = (yv - y_min) * inv;
        out.push(plot.x + xn * plot.w);
        out.push(plot.bottom() - py_norm * plot.h);
    }
}
