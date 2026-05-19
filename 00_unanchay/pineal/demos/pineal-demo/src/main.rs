//! `pineal-demo` — demo visual de Lapaloma sobre nahual.
//!
//! Ventana 900×560 con un chart cartesiano de **3 series**
//! simultáneas sobre 1024 muestras:
//!
//! - `sin(x · 0.04)` — azul nórdico
//! - `cos(x · 0.04)` — naranja
//! - `0.5·sin(x · 0.02) + 0.5·cos(x · 0.08)` — verde
//!
//! Interacción: click+drag = pan, wheel = zoom, doble-click = reset.

use gpui::{
    div, prelude::*, px, ClickEvent, Context, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Point, Render, ScrollDelta, ScrollWheelEvent, Window,
};

use pineal_cartesian::{chart_cache, ChartCacheHandle, ChartViewport, LapalomaChartElement};
use pineal_core::buffer::DataBuffer;
use pineal_render::{Color, StrokeStyle};
use nahual_launcher::launch_app;
use nahual_theme::Theme;

const N_SAMPLES: usize = 1024;
const WHEEL_SENSITIVITY: f64 = 0.0015;

fn main() {
    launch_app(
        "Lapaloma — multi-series (drag = pan, wheel = zoom, dbl-click = reset)",
        (900., 560.),
        Demo::new,
    );
}

struct Demo {
    series_sin: DataBuffer,
    series_cos: DataBuffer,
    series_mix: DataBuffer,
    viewport: ChartViewport,
    initial_viewport: ChartViewport,
    drag: Option<DragAnchor>,
    chart_cache: ChartCacheHandle,
}

#[derive(Clone, Copy)]
struct DragAnchor {
    start_position: Point<gpui::Pixels>,
    viewport_at_start: ChartViewport,
}

impl Demo {
    fn new(_cx: &mut Context<Self>) -> Self {
        let mut sin = DataBuffer::with_capacity(N_SAMPLES);
        let mut cos = DataBuffer::with_capacity(N_SAMPLES);
        let mut mix = DataBuffer::with_capacity(N_SAMPLES);
        for i in 0..N_SAMPLES {
            let x = i as f32;
            sin.push(x, (x * 0.04).sin());
            cos.push(x, (x * 0.04).cos());
            mix.push(x, 0.5 * (x * 0.02).sin() + 0.5 * (x * 0.08).cos());
        }
        let viewport = ChartViewport::new(0.0, (N_SAMPLES - 1) as f64, -1.3, 1.3);
        Self {
            series_sin: sin,
            series_cos: cos,
            series_mix: mix,
            viewport,
            initial_viewport: viewport,
            drag: None,
            chart_cache: chart_cache(),
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

/// Color helper para usar el mismo hex tanto en `pineal_render`
/// como en el body de texto del header del demo.
const COLOR_SIN: u32 = 0x88c0d0; // azul nórdico
const COLOR_COS: u32 = 0xd08770; // naranja
const COLOR_MIX: u32 = 0xa3be8c; // verde

impl Render for Demo {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        let plot_bg = Color::rgba(0.10, 0.12, 0.16, 1.0);
        let chart = LapalomaChartElement::new(self.viewport)
            .background(plot_bg)
            .with_cache(self.chart_cache.clone())
            .add_series_named(
                self.series_sin.clone(),
                StrokeStyle::new(2.0, Color::from_hex(COLOR_SIN)),
                "sin",
            )
            .add_series_named(
                self.series_cos.clone(),
                StrokeStyle::new(2.0, Color::from_hex(COLOR_COS)),
                "cos",
            )
            .add_series_named(
                self.series_mix.clone(),
                StrokeStyle::new(2.0, Color::from_hex(COLOR_MIX)),
                "mix",
            );

        let drag_active = self.drag.is_some();
        let (pan_blits, rebuilds) = {
            let c = self.chart_cache.lock().unwrap();
            (c.pan_blits(), c.rebuilds())
        };

        div()
            .id("pineal-demo-root")
            .size_full()
            .bg(theme.bg_app.clone())
            .p(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_color(theme.fg_text)
                    .text_size(px(18.))
                    .child("Lapaloma — demo cartesian multi-series"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(14.))
                    .text_size(px(11.))
                    .child(
                        div()
                            .text_color(gpui::rgb(COLOR_SIN))
                            .child("■ sin(x · 0.04)"),
                    )
                    .child(
                        div()
                            .text_color(gpui::rgb(COLOR_COS))
                            .child("■ cos(x · 0.04)"),
                    )
                    .child(
                        div()
                            .text_color(gpui::rgb(COLOR_MIX))
                            .child("■ ½·sin(x · 0.02) + ½·cos(x · 0.08)"),
                    )
                    .child(
                        div()
                            .text_color(theme.fg_muted)
                            .child(format!(
                                "· cache: {} pan-blits / {} rebuilds {}",
                                pan_blits,
                                rebuilds,
                                if drag_active { "· dragging" } else { "" },
                            )),
                    ),
            )
            .child(
                div()
                    .id("pineal-chart-host")
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
