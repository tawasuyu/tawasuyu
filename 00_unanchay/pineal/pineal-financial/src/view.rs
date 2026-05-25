//! Vista Llimphi del chart OHLC / candlesticks.
//!
//! Paralelo de `element.rs` para el bucle de Llimphi. Reusa
//! `pineal_cartesian::axis::paint_axes` para los ejes y la función
//! agnóstica `paint_candlesticks` de `candlestick.rs` — sólo cambia
//! el backend (`SceneCanvas` en lugar de `WindowCanvas`).

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::View;

use pineal_cartesian::axis::{self, AxisStyle};
use pineal_cartesian::{ChartViewport, CoordinateSystem};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

use crate::candlestick::{paint_candlesticks, CandlestickStyle};
use crate::ohlc_buffer::OhlcBuffer;

const TARGET_TICKS_X: usize = 8;
const TARGET_TICKS_Y: usize = 6;

pub struct CandlestickView {
    data: OhlcBuffer,
    viewport: ChartViewport,
    style: CandlestickStyle,
    background: Option<Color>,
    axis_color: Color,
    axis_style: AxisStyle,
    margin_top: f32,
    margin_right: f32,
    margin_bottom: f32,
    margin_left: f32,
}

impl CandlestickView {
    pub fn new(data: OhlcBuffer, viewport: ChartViewport) -> Self {
        Self {
            data,
            viewport,
            style: CandlestickStyle::default(),
            background: None,
            axis_color: Color::rgba(0.6, 0.6, 0.65, 0.8),
            axis_style: AxisStyle::default(),
            margin_top: 8.0,
            margin_right: 8.0,
            margin_bottom: 24.0,
            margin_left: 48.0,
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

    /// Materializa el `View<Msg>`. Pinta velas + ejes dentro del rect.
    pub fn view<Msg: Clone + 'static>(self) -> View<Msg> {
        let CandlestickView {
            data,
            viewport,
            style,
            background,
            axis_color,
            axis_style,
            margin_top,
            margin_right,
            margin_bottom,
            margin_left,
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

            paint_candlesticks(&mut canvas, &cs, &data, style);
        })
    }
}

/// Helper builder-style — paralelo al `lapaloma_candlestick(...)` GPUI.
pub fn lapaloma_candlestick_view(
    data: OhlcBuffer,
    viewport: ChartViewport,
) -> CandlestickView {
    CandlestickView::new(data, viewport)
}
