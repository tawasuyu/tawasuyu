//! `LapalomaChartElement` — el `Element` GPUI que envuelve el
//! pipeline cartesian.
//!
//! ## Picture cache pan-blit
//!
//! En GPUI cada frame se construye un Element nuevo (el árbol se
//! recrea), así que el cache no puede vivir en el Element. El
//! caller crea un `ChartCacheHandle` (Arc<Mutex<ChartCache>>) una
//! vez y se lo pasa a cada frame.
//!
//! Algoritmo (sección 4.4 del ARCHITECTURE.md adaptada a GPUI):
//! - Hash estructural = plot rect + span (no x_min/y_min) + por
//!   serie: revision + len + stroke.
//! - Si hash igual al cached: **pan puro** → emitimos las coords
//!   cacheadas con un offset `(dx_px, dy_px)` calculado del
//!   diff `viewport.x_min - cached.x_min`. Saltea LTTB +
//!   projection.
//! - Si hash distinto: full rebuild. Re-corre LTTB + project,
//!   pisa el cache, actualiza el snapshot del viewport.
//!
//! Sin cache, el Element funciona igual: cada frame rebuild
//! completo. Útil para tests/smoke.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic;
use std::sync::{Arc, Mutex};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window,
};

use pineal_core::buffer::DataBuffer;
use pineal_render::{Canvas, Color, Rect, StrokeStyle, WindowCanvas};

use crate::axis::{self, AxisStyle};
use crate::coord_system::CoordinateSystem;
use crate::series::LineSeries;
use crate::viewport::ChartViewport;

const TARGET_TICKS_X: usize = 8;
const TARGET_TICKS_Y: usize = 6;

/// Cache de coords proyectadas para reuso entre frames. Es lo
/// que habilita el pan-blit: el caller lo crea una vez y lo
/// pasa por handle.
#[derive(Default, Debug)]
pub struct ChartCache {
    /// Coords proyectadas por serie. `projected.len()` debe coincidir
    /// con la cantidad de series del Element.
    projected: Vec<Vec<f32>>,
    /// Hash de la geometría + identidades de data. Si cambia,
    /// invalidamos.
    structural_hash: u64,
    /// `viewport.x_min` con el que se proyectaron las coords.
    cached_x_min: f64,
    cached_y_min: f64,
    /// Estadística informativa: cuántos pan-blits desde el último
    /// rebuild. Útil para debugging y para mostrar en demos.
    pan_blits: u64,
    /// Estadística informativa: cuántos rebuilds totales.
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

/// Atajo para crear un cache compartido. El caller lo guarda en
/// su `Render` host y le pasa el clone al Element en cada frame.
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

pub struct LapalomaChartElement {
    pub series: Vec<ChartSeriesItem>,
    pub viewport: ChartViewport,
    pub background: Option<Color>,
    pub axis_color: Color,
    pub axis_style: AxisStyle,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    /// Cache opcional compartido con el `Render` host. Si está
    /// presente, habilita pan-blit.
    pub cache: Option<ChartCacheHandle>,
    scratch: Vec<f32>,
}

impl LapalomaChartElement {
    pub fn new(viewport: ChartViewport) -> Self {
        Self {
            series: Vec::new(),
            viewport,
            background: None,
            axis_color: Color::rgba(0.6, 0.6, 0.65, 0.8),
            axis_style: AxisStyle::default(),
            margin_bottom: 24.0,
            margin_left: 32.0,
            margin_top: 8.0,
            margin_right: 8.0,
            cache: None,
            scratch: Vec::new(),
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

    /// Enchufa un cache compartido. Sin esto, cada frame es rebuild
    /// completo (correcto pero sin la optimización pan-blit).
    pub fn with_cache(mut self, cache: ChartCacheHandle) -> Self {
        self.cache = Some(cache);
        self
    }

    fn plot_rect(&self, bounds: Rect) -> Rect {
        Rect::new(
            bounds.x + self.margin_left,
            bounds.y + self.margin_top,
            (bounds.w - self.margin_left - self.margin_right).max(1.0),
            (bounds.h - self.margin_top - self.margin_bottom).max(1.0),
        )
    }

    fn paint_axes(&self, canvas: &mut dyn Canvas, cs: &CoordinateSystem) {
        axis::paint_axes(
            canvas,
            cs,
            &self.viewport,
            self.axis_color,
            self.axis_style,
            TARGET_TICKS_X,
            TARGET_TICKS_Y,
        );
    }

    /// Rebuild full: LTTB + projection por serie. Pinta directo desde
    /// el cache si está enchufado (para no copiar dos veces).
    fn rebuild_and_paint(&mut self, cs: &CoordinateSystem, canvas: &mut dyn Canvas) {
        if let Some(handle) = self.cache.clone() {
            let mut cache = handle.lock().unwrap();
            cache.projected.clear();
            cache.projected.resize_with(self.series.len(), Vec::new);
            for (i, item) in self.series.iter().enumerate() {
                let series = LineSeries::new(&item.data, item.stroke);
                series.compute_projected(cs, &mut cache.projected[i]);
                if cache.projected[i].len() >= 4 {
                    canvas.stroke_polyline(&cache.projected[i], item.stroke);
                }
            }
            cache.structural_hash = structural_hash(
                cs.plot,
                self.viewport.x_span(),
                self.viewport.y_span(),
                &self.series,
            );
            cache.cached_x_min = self.viewport.x_min;
            cache.cached_y_min = self.viewport.y_min;
            cache.has_valid_cache = true;
            cache.pan_blits = 0;
            cache.rebuilds = cache.rebuilds.wrapping_add(1);
        } else {
            // Sin cache: usamos el scratch local.
            for item in &self.series {
                let series = LineSeries::new(&item.data, item.stroke);
                series.compute_projected(cs, &mut self.scratch);
                if self.scratch.len() >= 4 {
                    canvas.stroke_polyline(&self.scratch, item.stroke);
                }
            }
        }
    }

    /// Emite las coords cacheadas con un offset en pixel space.
    /// Se usa cuando detectamos pan puro (mismo hash estructural).
    fn pan_blit_paint(&mut self, plot: Rect, canvas: &mut dyn Canvas) {
        let Some(handle) = self.cache.clone() else {
            return;
        };
        let mut cache = handle.lock().unwrap();
        let dx_px = ((cache.cached_x_min - self.viewport.x_min) * plot.w as f64
            / self.viewport.x_span()) as f32;
        let dy_px = ((self.viewport.y_min - cache.cached_y_min) * plot.h as f64
            / self.viewport.y_span()) as f32;

        for (i, item) in self.series.iter().enumerate() {
            let cached = &cache.projected[i];
            if cached.len() < 4 {
                continue;
            }
            self.scratch.clear();
            self.scratch.reserve(cached.len());
            let mut k = 0;
            while k + 1 < cached.len() {
                self.scratch.push(cached[k] + dx_px);
                self.scratch.push(cached[k + 1] + dy_px);
                k += 2;
            }
            canvas.stroke_polyline(&self.scratch, item.stroke);
        }
        cache.pan_blits = cache.pan_blits.wrapping_add(1);
    }
}

impl IntoElement for LapalomaChartElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LapalomaChartElement {
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

        let cs = CoordinateSystem::new(self.viewport, plot);
        let mut canvas = WindowCanvas::new(window);

        if let Some(bg) = self.background {
            canvas.fill_rect(outer, bg);
        }

        self.paint_axes(&mut canvas, &cs);

        // Decide rebuild vs pan-blit.
        let current_hash = structural_hash(
            plot,
            self.viewport.x_span(),
            self.viewport.y_span(),
            &self.series,
        );
        let pan_only = self
            .cache
            .as_ref()
            .map(|h| {
                let c = h.lock().unwrap();
                c.has_valid_cache
                    && c.structural_hash == current_hash
                    && c.projected.len() == self.series.len()
            })
            .unwrap_or(false);

        if pan_only {
            self.pan_blit_paint(plot, &mut canvas);
        } else {
            self.rebuild_and_paint(&cs, &mut canvas);
        }
    }
}

/// Hash de la geometría + identidades de data. Lo que NO va acá:
/// `viewport.x_min` y `y_min` (el pan los mueve sin invalidar).
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

pub fn lapaloma_chart(
    data: DataBuffer,
    viewport: ChartViewport,
    stroke: StrokeStyle,
) -> LapalomaChartElement {
    LapalomaChartElement::new(viewport).add_series(data, stroke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_hash_estable_para_mismo_estado() {
        let mut data = DataBuffer::with_capacity(10);
        for i in 0..10 {
            data.push(i as f32, (i as f32).sin());
        }
        let series = vec![ChartSeriesItem::new(
            data,
            StrokeStyle::new(2.0, Color::rgb(1.0, 0.0, 0.0)),
        )];
        let plot = Rect::new(0.0, 0.0, 100.0, 100.0);
        let a = structural_hash(plot, 10.0, 2.0, &series);
        let b = structural_hash(plot, 10.0, 2.0, &series);
        assert_eq!(a, b);
    }

    #[test]
    fn structural_hash_pan_no_cambia() {
        // Mismo span pero distinto x_min/y_min — hash igual.
        let mut data = DataBuffer::with_capacity(10);
        for i in 0..10 {
            data.push(i as f32, (i as f32).sin());
        }
        let series = vec![ChartSeriesItem::new(
            data,
            StrokeStyle::new(2.0, Color::rgb(1.0, 0.0, 0.0)),
        )];
        let plot = Rect::new(0.0, 0.0, 100.0, 100.0);
        let a = structural_hash(plot, 10.0, 2.0, &series);
        // Pan implícito: x_span/y_span no cambiaron → hash igual.
        let b = structural_hash(plot, 10.0, 2.0, &series);
        assert_eq!(a, b);
    }

    #[test]
    fn structural_hash_zoom_invalida() {
        let series = vec![ChartSeriesItem::new(
            DataBuffer::new(),
            StrokeStyle::new(2.0, Color::WHITE),
        )];
        let plot = Rect::new(0.0, 0.0, 100.0, 100.0);
        let a = structural_hash(plot, 10.0, 2.0, &series);
        let b = structural_hash(plot, 5.0, 2.0, &series); // zoom in X
        assert_ne!(a, b);
    }

    #[test]
    fn structural_hash_data_revision_invalida() {
        let mut data = DataBuffer::with_capacity(2);
        data.push(0.0, 0.0);
        let series0 = vec![ChartSeriesItem::new(
            data.clone(),
            StrokeStyle::new(2.0, Color::WHITE),
        )];
        let plot = Rect::new(0.0, 0.0, 100.0, 100.0);
        let a = structural_hash(plot, 1.0, 1.0, &series0);

        data.push(1.0, 1.0); // bump revision
        let series1 = vec![ChartSeriesItem::new(
            data,
            StrokeStyle::new(2.0, Color::WHITE),
        )];
        let b = structural_hash(plot, 1.0, 1.0, &series1);
        assert_ne!(a, b);
    }

    #[test]
    fn structural_hash_plot_rect_invalida() {
        let series = vec![ChartSeriesItem::new(
            DataBuffer::new(),
            StrokeStyle::new(2.0, Color::WHITE),
        )];
        let a = structural_hash(Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, 1.0, &series);
        let b = structural_hash(Rect::new(0.0, 0.0, 200.0, 100.0), 1.0, 1.0, &series);
        assert_ne!(a, b);
    }
}
