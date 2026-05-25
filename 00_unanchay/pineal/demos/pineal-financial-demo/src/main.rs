//! `pineal-financial-demo` — chart OHLC con random walk.
//!
//! Genera 120 "días" de bars con un random walk determinístico
//! (sin RNG runtime — derivado de un seed fijo + xorshift32 inline)
//! y los pinta con `LapalomaCandlestickElement`. Pan + zoom igual
//! al cartesian demo.

use gpui::{
    div, prelude::*, px, ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Point, Render, ScrollDelta, ScrollWheelEvent, Window,
};

use pineal_cartesian::ChartViewport;
use pineal_financial::{
    lapaloma_candlestick, Bar, CandlestickStyle, OhlcBuffer,
};
use pineal_render::Color;
use nahual_launcher::launch_app;
use nahual_theme::Theme;

const N_BARS: usize = 120;
const WHEEL_SENSITIVITY: f64 = 0.0015;

fn main() {
    launch_app(
        "Lapaloma — candlesticks (drag = pan, wheel = zoom, dbl-click = reset)",
        (960., 560.),
        FinancialDemo::new,
    );
}

struct FinancialDemo {
    data: OhlcBuffer,
    viewport: ChartViewport,
    initial_viewport: ChartViewport,
    drag: Option<DragAnchor>,
}

#[derive(Clone, Copy)]
struct DragAnchor {
    start_position: Point<gpui::Pixels>,
    viewport_at_start: ChartViewport,
}

impl FinancialDemo {
    fn new(_cx: &mut Context<Self>) -> Self {
        let data = synth_random_walk(N_BARS, 100.0, 0xc0ffee);
        let (lo, hi) = data.price_range().unwrap_or((0.0, 1.0));
        let pad = (hi - lo) * 0.08;
        let viewport = ChartViewport::new(
            -0.5,
            N_BARS as f64 - 0.5,
            (lo - pad) as f64,
            (hi + pad) as f64,
        );
        Self {
            data,
            viewport,
            initial_viewport: viewport,
            drag: None,
        }
    }

    fn on_mouse_down(&mut self, e: &MouseDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        self.drag = Some(DragAnchor {
            start_position: e.position,
            viewport_at_start: self.viewport,
        });
        cx.notify();
    }
    fn on_mouse_move(&mut self, e: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(anchor) = self.drag else { return };
        let win = window.viewport_size();
        let w: f32 = win.width.into();
        let h: f32 = win.height.into();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let sx: f32 = e.position.x.into();
        let sy: f32 = e.position.y.into();
        let ax: f32 = anchor.start_position.x.into();
        let ay: f32 = anchor.start_position.y.into();
        let dfx = ((sx - ax) / w) as f64;
        let dfy = ((sy - ay) / h) as f64;
        let mut vp = anchor.viewport_at_start;
        vp.pan_fraction(dfx, dfy);
        self.viewport = vp;
        cx.notify();
    }
    fn on_mouse_up(&mut self, _e: &MouseUpEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            cx.notify();
        }
    }
    fn on_scroll(&mut self, e: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let win = window.viewport_size();
        let w: f32 = win.width.into();
        let h: f32 = win.height.into();
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let dy_px: f32 = match e.delta {
            ScrollDelta::Pixels(p) => p.y.into(),
            ScrollDelta::Lines(p) => p.y * 16.0,
        };
        let factor = (-dy_px as f64 * WHEEL_SENSITIVITY).exp();
        let sx: f32 = e.position.x.into();
        let sy: f32 = e.position.y.into();
        let ax = (sx / w).clamp(0.0, 1.0) as f64;
        let ay = (1.0 - sy / h).clamp(0.0, 1.0) as f64;
        self.viewport.zoom_uniform(factor, (ax, ay));
        cx.notify();
    }
    fn on_click(&mut self, e: &ClickEvent, _w: &mut Window, cx: &mut Context<Self>) {
        if let ClickEvent::Mouse(m) = e {
            if m.up.click_count >= 2 {
                self.viewport = self.initial_viewport;
                cx.notify();
            }
        }
    }
}

impl Render for FinancialDemo {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let plot_bg = Color::rgba(0.06, 0.08, 0.10, 1.0);

        let style = CandlestickStyle {
            bull_color: Color::from_hex(0xa3be8c),
            bear_color: Color::from_hex(0xbf616a),
            ..CandlestickStyle::default()
        };

        let chart = lapaloma_candlestick(self.data.clone(), self.viewport)
            .background(plot_bg)
            .style(style);

        let (lo, hi) = self.data.price_range().unwrap_or((0.0, 0.0));
        let drag_active = self.drag.is_some();

        div()
            .id("pineal-financial-root")
            .size_full()
            .bg(theme.bg_app.clone())
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_color(theme.fg_text)
                    .text_size(px(18.))
                    .child("Lapaloma — candlesticks"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .text_size(px(11.))
                    .text_color(theme.fg_muted)
                    .child(format!("{} bars (random walk)", N_BARS))
                    .child(format!("price [{:.2}, {:.2}]", lo, hi))
                    .child(if drag_active { "· dragging" } else { "" }),
            )
            .child(
                div()
                    .id("pineal-financial-chart")
                    .w_full()
                    .flex_grow()
                    .child(chart)
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .on_click(cx.listener(Self::on_click)),
            )
    }
}

/// xorshift32 inline — RNG determinístico mínimo. No criptográfico,
/// pero perfecto para series sintéticas reproducibles.
fn xorshift32(state: &mut u32) -> u32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    x
}

fn rand_f32(state: &mut u32) -> f32 {
    xorshift32(state) as f32 / u32::MAX as f32
}

fn synth_random_walk(n: usize, start_price: f32, seed: u32) -> OhlcBuffer {
    let mut rng = seed.max(1);
    let mut buf = OhlcBuffer::with_capacity(n);
    let mut close = start_price;
    let drift = 0.05; // tendencia mínima alcista
    let vol = 1.2;
    for i in 0..n {
        let r1 = rand_f32(&mut rng) - 0.5;
        let r2 = rand_f32(&mut rng) - 0.5;
        let r3 = rand_f32(&mut rng) - 0.5;
        let r4 = rand_f32(&mut rng) - 0.5;

        let open = close;
        let move_close = drift + r1 * vol * 2.0;
        let new_close = (open + move_close).max(1.0);
        // Wicks: ruido por encima/debajo del rango open-close.
        let body_hi = open.max(new_close);
        let body_lo = open.min(new_close);
        let wick_up = (r2.abs() * vol * 1.2).max(0.05);
        let wick_dn = (r3.abs() * vol * 1.2).max(0.05);
        let high = body_hi + wick_up;
        let low = (body_lo - wick_dn).max(0.1);
        let volume = 1000.0 + r4.abs() * 8000.0;

        buf.push_bar(Bar {
            t: i as f32,
            o: open,
            h: high,
            l: low,
            c: new_close,
            v: volume,
        });
        close = new_close;
    }
    buf
}
