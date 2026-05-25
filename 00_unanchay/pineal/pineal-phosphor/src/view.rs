//! Vista Llimphi del trail CRT.
//!
//! Mismo enfoque que el Element GPUI: el RingBuffer se parte en N
//! segmentos polilínea con alpha decreciente del más nuevo al más
//! viejo. Wraparound se parte en dos sub-polilíneas para evitar la
//! línea horizontal del slot cap-1 → 0. Opcionalmente bajo cada
//! tramo se pinta una pasada "glow" con stroke más ancho y alpha
//! reducido (efecto halo CRT).
//!
//! No usa `fill_triangle_strip` — sólo `stroke_polyline`. Paridad
//! visual completa con el Element GPUI.

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;

use pineal_core::ring::RingBuffer;
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas, StrokeStyle};

const DEFAULT_TRAIL_SEGMENTS: usize = 16;

pub struct PhosphorView {
    buffer: RingBuffer,
    base_stroke: StrokeStyle,
    background: Option<Color>,
    y_min: f32,
    y_max: f32,
    padding: f32,
    trail_segments: usize,
    glow_width_mult: f32,
    glow_alpha: f32,
}

impl PhosphorView {
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

    pub fn view<Msg: Clone + 'static>(self) -> View<Msg> {
        let PhosphorView {
            buffer,
            base_stroke,
            background,
            y_min,
            y_max,
            padding,
            trail_segments,
            glow_width_mult,
            glow_alpha,
        } = self;
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .paint_with(move |scene, typesetter, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let plot = Rect::new(
                outer.x + padding,
                outer.y + padding,
                (outer.w - padding * 2.0).max(1.0),
                (outer.h - padding * 2.0).max(1.0),
            );

            let mut canvas = SceneCanvas::new(scene, typesetter);

            if let Some(bg) = background {
                canvas.fill_rect(outer, bg);
            }

            let filled = buffer.filled_len();
            if filled < 2 {
                return;
            }
            let cap = buffer.capacity();
            let head = buffer.head();
            let coords = buffer.coords();
            let start_slot = if buffer.is_full() { head } else { 0 };
            let n_segs = trail_segments.min(filled / 2).max(2);
            let base_per_seg = filled / n_segs;
            let glow_enabled = glow_alpha > 0.0 && glow_width_mult > 1.0;

            let mut scratch: Vec<f32> = Vec::new();

            for k in 0..n_segs {
                let t_lo = k * base_per_seg;
                let t_hi = if k == n_segs - 1 { filled } else { (k + 1) * base_per_seg + 1 };
                if t_hi <= t_lo + 1 {
                    continue;
                }

                let life = (k as f32 + 1.0) / n_segs as f32;
                let alpha = base_stroke.color.a * life;
                let mut color = base_stroke.color;
                color.a = alpha;
                let stroke = StrokeStyle::new(base_stroke.width, color);

                let glow_stroke = if glow_enabled {
                    let mut gc = color;
                    gc.a *= glow_alpha;
                    Some(StrokeStyle::new(base_stroke.width * glow_width_mult, gc))
                } else {
                    None
                };

                let seg_len = t_hi - t_lo;
                let abs_start = (start_slot + t_lo) % cap;
                let contiguous_len = cap - abs_start;

                if seg_len <= contiguous_len {
                    let slice = &coords[abs_start * 2..(abs_start + seg_len) * 2];
                    scratch.clear();
                    project_segment(slice, plot, y_min, y_max, &mut scratch);
                    if scratch.len() >= 4 {
                        if let Some(gs) = glow_stroke {
                            canvas.stroke_polyline(&scratch, gs);
                        }
                        canvas.stroke_polyline(&scratch, stroke);
                    }
                } else {
                    let slice_a = &coords[abs_start * 2..];
                    scratch.clear();
                    project_segment(slice_a, plot, y_min, y_max, &mut scratch);
                    if scratch.len() >= 4 {
                        if let Some(gs) = glow_stroke {
                            canvas.stroke_polyline(&scratch, gs);
                        }
                        canvas.stroke_polyline(&scratch, stroke);
                    }
                    let remaining = seg_len - contiguous_len;
                    let slice_b = &coords[..remaining * 2];
                    scratch.clear();
                    project_segment(slice_b, plot, y_min, y_max, &mut scratch);
                    if scratch.len() >= 4 {
                        if let Some(gs) = glow_stroke {
                            canvas.stroke_polyline(&scratch, gs);
                        }
                        canvas.stroke_polyline(&scratch, stroke);
                    }
                }
            }
        })
    }
}

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

pub fn pineal_phosphor_view(buffer: RingBuffer, base_stroke: StrokeStyle) -> PhosphorView {
    PhosphorView::new(buffer, base_stroke)
}
