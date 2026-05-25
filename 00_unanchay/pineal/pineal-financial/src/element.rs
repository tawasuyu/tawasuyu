//! `LapalomaCandlestickElement` — Element GPUI para charts OHLC.
//!
//! Reusa `pineal_cartesian::axis::paint_axes` para los ejes y el
//! `WindowCanvas` adapter de `pineal-render` para el output.
//! El render mismo de las velas lo hace `paint_candlesticks` que
//! es agnóstico de gpui — facilita futuros backends (SVG, wgpu).
//!
//! Sin cache pan-blit en v0.1: las velas se redibujan cada frame.
//! Para hasta ~500 bars on-screen son sub-millisecond; con
//! aggregation razonable (1 bar por columna de pixel) eso cubre
//! cualquier caso humano. Si se necesita más, se replica el
//! patrón de `LapalomaChartElement::with_cache`.

use std::panic;

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Style, Window,
};

use pineal_cartesian::axis::{self, AxisStyle};
use pineal_cartesian::{ChartViewport, CoordinateSystem};
use pineal_render::{Canvas, Color, Rect, WindowCanvas};

use crate::candlestick::{paint_candlesticks, CandlestickStyle};
use crate::ohlc_buffer::OhlcBuffer;

const TARGET_TICKS_X: usize = 8;
const TARGET_TICKS_Y: usize = 6;

pub struct LapalomaCandlestickElement {
    pub data: OhlcBuffer,
    pub viewport: ChartViewport,
    pub style: CandlestickStyle,
    pub background: Option<Color>,
    pub axis_color: Color,
    pub axis_style: AxisStyle,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_top: f32,
    pub margin_right: f32,
}

impl LapalomaCandlestickElement {
    pub fn new(data: OhlcBuffer, viewport: ChartViewport) -> Self {
        Self {
            data,
            viewport,
            style: CandlestickStyle::default(),
            background: None,
            axis_color: Color::rgba(0.6, 0.6, 0.65, 0.8),
            axis_style: AxisStyle::default(),
            margin_bottom: 24.0,
            margin_left: 48.0,
            margin_top: 8.0,
            margin_right: 8.0,
        }
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }
    pub fn axis_color(mut self, color: Color) -> Self {
        self.axis_color = color;
        self
    }
    pub fn style(mut self, style: CandlestickStyle) -> Self {
        self.style = style;
        self
    }
    pub fn margins(mut self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        self.margin_top = top;
        self.margin_right = right;
        self.margin_bottom = bottom;
        self.margin_left = left;
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
}

impl IntoElement for LapalomaCandlestickElement {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LapalomaCandlestickElement {
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

        axis::paint_axes(
            &mut canvas,
            &cs,
            &self.viewport,
            self.axis_color,
            self.axis_style,
            TARGET_TICKS_X,
            TARGET_TICKS_Y,
        );

        paint_candlesticks(&mut canvas, &cs, &self.data, self.style);
    }
}

pub fn lapaloma_candlestick(
    data: OhlcBuffer,
    viewport: ChartViewport,
) -> LapalomaCandlestickElement {
    LapalomaCandlestickElement::new(data, viewport)
}
