//! Render de candlesticks sobre cualquier `Canvas`.
//!
//! Por cada bar visible:
//! - **Wick** = línea vertical de `(t, low)` a `(t, high)`.
//! - **Body** = rect de `(t - body_w/2, open)` a `(t + body_w/2, close)`,
//!   relleno bull (close > open) / bear (close < open) / neutro.
//!
//! Esta función es agnóstica de gpui — habla contra el trait
//! `Canvas`. El `Element` GPUI que la consume vive en `element.rs`.

use pineal_cartesian::CoordinateSystem;
use pineal_render::{Canvas, Color, Point, Rect, StrokeStyle};

use crate::ohlc_buffer::OhlcBuffer;

/// Estilo visual de los candlesticks.
#[derive(Debug, Clone, Copy)]
pub struct CandlestickStyle {
    pub bull_color: Color,
    pub bear_color: Color,
    /// Color del body cuando open == close. Suele ser el axis color.
    pub neutral_color: Color,
    /// Ancho del wick (línea central). En píxeles.
    pub wick_width: f32,
    /// Ancho mínimo del body, en píxeles. Cuando el spacing entre
    /// bars cae por debajo, el body usa este floor.
    pub body_min_width: f32,
    /// Fracción del spacing entre bars consecutivas que ocupa el body.
    /// 0.7 deja un gap del 30% entre velas.
    pub body_width_ratio: f32,
}

impl Default for CandlestickStyle {
    fn default() -> Self {
        Self {
            bull_color: Color::from_hex(0x88c08a),
            bear_color: Color::from_hex(0xbf616a),
            neutral_color: Color::rgba(0.7, 0.7, 0.75, 1.0),
            wick_width: 1.0,
            body_min_width: 2.0,
            body_width_ratio: 0.7,
        }
    }
}

/// Dibuja todas las velas del buffer visibles en el viewport del
/// `CoordinateSystem`. Bars fuera de rango se skippean.
pub fn paint_candlesticks(
    canvas: &mut dyn Canvas,
    cs: &CoordinateSystem,
    data: &OhlcBuffer,
    style: CandlestickStyle,
) {
    let n = data.len();
    if n == 0 {
        return;
    }

    let plot = cs.plot;
    let viewport = cs.viewport;

    // Spacing entre bars consecutivas en píxeles. Asume bars
    // aproximadamente equiespaciadas en X (caso típico OHLC
    // post-aggregation).
    let body_width = if n >= 2 {
        let first_t = data.bar(0).t as f64;
        let last_t = data.bar(n - 1).t as f64;
        let span_t = (last_t - first_t).max(f32::EPSILON as f64);
        let span_px = (span_t / viewport.x_span()) * plot.w as f64;
        let spacing = span_px / (n as f64 - 1.0);
        ((spacing * style.body_width_ratio as f64) as f32).max(style.body_min_width)
    } else {
        style.body_min_width
    };
    let half_body = body_width * 0.5;

    for i in 0..n {
        let bar = data.bar(i);

        // Clip aproximado: si el bar entero queda fuera del viewport
        // X, lo saltamos.
        if (bar.t as f64) < viewport.x_min - viewport.x_span() * 0.05
            || (bar.t as f64) > viewport.x_max + viewport.x_span() * 0.05
        {
            continue;
        }

        let px_center = cs.data_to_pixel(bar.t as f64, bar.o as f64).x;
        let py_open = cs.data_to_pixel(bar.t as f64, bar.o as f64).y;
        let py_close = cs.data_to_pixel(bar.t as f64, bar.c as f64).y;
        let py_high = cs.data_to_pixel(bar.t as f64, bar.h as f64).y;
        let py_low = cs.data_to_pixel(bar.t as f64, bar.l as f64).y;

        let color = if bar.is_bull() {
            style.bull_color
        } else if bar.is_bear() {
            style.bear_color
        } else {
            style.neutral_color
        };

        // Wick: línea vertical de high a low.
        canvas.stroke_line(
            Point::new(px_center, py_high),
            Point::new(px_center, py_low),
            StrokeStyle::new(style.wick_width, color),
        );

        // Body: rect entre open y close.
        let (y_top, y_bot) = if py_open < py_close {
            (py_open, py_close)
        } else {
            (py_close, py_open)
        };
        let body_h = (y_bot - y_top).max(1.0); // floor 1px para doji
        canvas.fill_rect(
            Rect::new(px_center - half_body, y_top, body_width, body_h),
            color,
        );
    }
}
