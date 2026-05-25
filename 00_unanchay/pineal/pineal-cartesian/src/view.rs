//! Vista Llimphi del gráfico cartesiano.
//!
//! Paralelo del Element GPUI (`element.rs`). Misma lógica conceptual
//! — viewport, axes, series con LTTB, picture-cache pan-blit —
//! pero el `View<Msg>` declarativo de Llimphi reconstruye el árbol
//! por frame. El estado que debe persistir entre frames (el cache de
//! coords proyectadas para pan-blit) vive afuera del View, en el Model
//! del host, vía un `ChartCacheHandle = Arc<Mutex<ChartCache>>` que el
//! caller crea una vez y le pasa el clone a cada `chart_view(...)`.
//!
//! `scratch` del Element original era estado mutable del Element. En el
//! View es un `Vec<f32>` local del closure: por frame se asigna y se
//! drop al terminar el paint. Para series de hasta 50 K puntos el costo
//! del alloc/dealloc es despreciable comparado con la proyección y el
//! shaping de glifos.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;

use pineal_core::buffer::DataBuffer;
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas, StrokeStyle};

use crate::axis::{self, AxisStyle};
use crate::coord_system::CoordinateSystem;
use crate::series::LineSeries;
use crate::viewport::ChartViewport;

const TARGET_TICKS_X: usize = 8;
const TARGET_TICKS_Y: usize = 6;

/// Cache de coords proyectadas para reuso entre frames. Lo que habilita
/// el pan-blit: el caller lo crea una vez en su Model y le pasa el
/// `Arc<Mutex<_>>` clonado a cada frame.
#[derive(Default, Debug)]
pub struct ChartCache {
    projected: Vec<Vec<f32>>,
    structural_hash: u64,
    cached_x_min: f64,
    cached_y_min: f64,
    pan_blits: u64,
    rebuilds: u64,
    has_valid_cache: bool,
}

impl ChartCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn pan_blits(&self) -> u64 {
        self.pan_blits
    }
    pub fn rebuilds(&self) -> u64 {
        self.rebuilds
    }
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }
}

pub type ChartCacheHandle = Arc<Mutex<ChartCache>>;

/// Atajo: cache compartido listo para enchufar en el host.
pub fn chart_cache() -> ChartCacheHandle {
    Arc::new(Mutex::new(ChartCache::new()))
}

#[derive(Clone)]
pub struct ChartSeriesItem {
    pub data: DataBuffer,
    pub stroke: StrokeStyle,
    pub name: Option<String>,
}

impl ChartSeriesItem {
    pub fn new(data: DataBuffer, stroke: StrokeStyle) -> Self {
        Self { data, stroke, name: None }
    }
    pub fn named(data: DataBuffer, stroke: StrokeStyle, name: impl Into<String>) -> Self {
        Self { data, stroke, name: Some(name.into()) }
    }
}

/// Configuración del chart. Builder estilo Element. `view::<Msg>()`
/// materializa el `View<Msg>`.
pub struct ChartView {
    series: Vec<ChartSeriesItem>,
    viewport: ChartViewport,
    background: Option<Color>,
    axis_color: Color,
    axis_style: AxisStyle,
    margin_top: f32,
    margin_right: f32,
    margin_bottom: f32,
    margin_left: f32,
    cache: Option<ChartCacheHandle>,
}

impl ChartView {
    pub fn new(viewport: ChartViewport) -> Self {
        Self {
            series: Vec::new(),
            viewport,
            background: None,
            axis_color: Color::rgba(0.6, 0.6, 0.65, 0.8),
            axis_style: AxisStyle::default(),
            margin_top: 8.0,
            margin_right: 8.0,
            margin_bottom: 24.0,
            margin_left: 32.0,
            cache: None,
        }
    }

    pub fn add_series(mut self, data: DataBuffer, stroke: StrokeStyle) -> Self {
        self.series.push(ChartSeriesItem::new(data, stroke));
        self
    }

    pub fn add_series_named(
        mut self,
        data: DataBuffer,
        stroke: StrokeStyle,
        name: impl Into<String>,
    ) -> Self {
        self.series.push(ChartSeriesItem::named(data, stroke, name));
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn axis_color(mut self, color: Color) -> Self {
        self.axis_color = color;
        self
    }

    pub fn margins(mut self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        self.margin_top = top;
        self.margin_right = right;
        self.margin_bottom = bottom;
        self.margin_left = left;
        self
    }

    pub fn with_cache(mut self, cache: ChartCacheHandle) -> Self {
        self.cache = Some(cache);
        self
    }

    /// Materializa el `View<Msg>`. Devuelve un nodo que ocupa 100% del
    /// padre y pinta el chart dentro de su rect.
    pub fn view<Msg: Clone + 'static>(self) -> View<Msg> {
        let ChartView {
            series,
            viewport,
            background,
            axis_color,
            axis_style,
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
            cache,
        } = self;

        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .paint_with(move |scene, typesetter, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let plot = Rect::new(
                outer.x + margin_left,
                outer.y + margin_top,
                (outer.w - margin_left - margin_right).max(1.0),
                (outer.h - margin_top - margin_bottom).max(1.0),
            );
            let cs = CoordinateSystem::new(viewport, plot);

            let mut canvas = SceneCanvas::new(scene, typesetter);

            if let Some(bg) = background {
                canvas.fill_rect(outer, bg);
            }

            axis::paint_axes(
                &mut canvas,
                &cs,
                &viewport,
                axis_color,
                axis_style,
                TARGET_TICKS_X,
                TARGET_TICKS_Y,
            );

            let current_hash =
                structural_hash(plot, viewport.x_span(), viewport.y_span(), &series);
            let pan_only = cache
                .as_ref()
                .map(|h| {
                    let c = h.lock().unwrap();
                    c.has_valid_cache
                        && c.structural_hash == current_hash
                        && c.projected.len() == series.len()
                })
                .unwrap_or(false);

            if pan_only {
                pan_blit_paint(&series, plot, viewport, cache.as_ref().unwrap(), &mut canvas);
            } else {
                rebuild_and_paint(
                    &series,
                    &cs,
                    plot,
                    viewport,
                    current_hash,
                    cache.as_ref(),
                    &mut canvas,
                );
            }
        })
    }
}

/// Rebuild full: LTTB + projection por serie. Pinta directo del cache
/// (si está enchufado) para no copiar dos veces.
fn rebuild_and_paint(
    series: &[ChartSeriesItem],
    cs: &CoordinateSystem,
    _plot: Rect,
    viewport: ChartViewport,
    current_hash: u64,
    cache: Option<&ChartCacheHandle>,
    canvas: &mut SceneCanvas<'_>,
) {
    if let Some(handle) = cache {
        let mut cached = handle.lock().unwrap();
        cached.projected.clear();
        cached.projected.resize_with(series.len(), Vec::new);
        for (i, item) in series.iter().enumerate() {
            let s = LineSeries::new(&item.data, item.stroke);
            s.compute_projected(cs, &mut cached.projected[i]);
            if cached.projected[i].len() >= 4 {
                canvas.stroke_polyline(&cached.projected[i], item.stroke);
            }
        }
        cached.structural_hash = current_hash;
        cached.cached_x_min = viewport.x_min;
        cached.cached_y_min = viewport.y_min;
        cached.has_valid_cache = true;
        cached.pan_blits = 0;
        cached.rebuilds = cached.rebuilds.wrapping_add(1);
    } else {
        let mut scratch = Vec::new();
        for item in series {
            let s = LineSeries::new(&item.data, item.stroke);
            s.compute_projected(cs, &mut scratch);
            if scratch.len() >= 4 {
                canvas.stroke_polyline(&scratch, item.stroke);
            }
        }
    }
}

/// Emite las coords cacheadas con un offset en pixel space. Se usa
/// cuando el hash estructural coincide — solo cambió el origen del
/// viewport (pan puro).
fn pan_blit_paint(
    series: &[ChartSeriesItem],
    plot: Rect,
    viewport: ChartViewport,
    cache: &ChartCacheHandle,
    canvas: &mut SceneCanvas<'_>,
) {
    let mut cached = cache.lock().unwrap();
    let dx_px =
        ((cached.cached_x_min - viewport.x_min) * plot.w as f64 / viewport.x_span()) as f32;
    let dy_px =
        ((viewport.y_min - cached.cached_y_min) * plot.h as f64 / viewport.y_span()) as f32;
    let mut scratch = Vec::new();
    for (i, item) in series.iter().enumerate() {
        let projected = &cached.projected[i];
        if projected.len() < 4 {
            continue;
        }
        scratch.clear();
        scratch.reserve(projected.len());
        let mut k = 0;
        while k + 1 < projected.len() {
            scratch.push(projected[k] + dx_px);
            scratch.push(projected[k + 1] + dy_px);
            k += 2;
        }
        canvas.stroke_polyline(&scratch, item.stroke);
    }
    cached.pan_blits = cached.pan_blits.wrapping_add(1);
}

/// Hash de la geometría + identidades de data. NO incluye `x_min`/`y_min`:
/// el pan los mueve sin invalidar.
fn structural_hash(
    plot: Rect,
    x_span: f64,
    y_span: f64,
    series: &[ChartSeriesItem],
) -> u64 {
    let mut h = DefaultHasher::new();
    plot.x.to_bits().hash(&mut h);
    plot.y.to_bits().hash(&mut h);
    plot.w.to_bits().hash(&mut h);
    plot.h.to_bits().hash(&mut h);
    x_span.to_bits().hash(&mut h);
    y_span.to_bits().hash(&mut h);
    (series.len() as u64).hash(&mut h);
    for s in series {
        s.data.revision().hash(&mut h);
        (s.data.len() as u64).hash(&mut h);
        s.stroke.width.to_bits().hash(&mut h);
        s.stroke.color.r.to_bits().hash(&mut h);
        s.stroke.color.g.to_bits().hash(&mut h);
        s.stroke.color.b.to_bits().hash(&mut h);
        s.stroke.color.a.to_bits().hash(&mut h);
    }
    h.finish()
}

/// Helper builder-style — paralelo al `lapaloma_chart(...)` del Element GPUI.
pub fn lapaloma_chart_view(
    data: DataBuffer,
    viewport: ChartViewport,
    stroke: StrokeStyle,
) -> ChartView {
    ChartView::new(viewport).add_series(data, stroke)
}
