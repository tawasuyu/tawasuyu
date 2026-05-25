//! `pineal-phosphor-demo` — osciloscopio con trail CRT.
//!
//! Igual setup que `pineal-stream-demo` (RingBuffer 512 +
//! timer 60 Hz) pero el render usa `LapalomaPhosphorElement`:
//! el trail decae en alpha del cursor hacia atrás y arrastra un
//! halo (glow). Visualmente queda como un osciloscopio analógico
//! con fósforo persistente.
//!
//! Sliders para `trail_segments` y `glow` se dejan para más
//! adelante; este demo usa los defaults.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, IntoElement, Render, Window};

use pineal_core::ring::RingBuffer;
use pineal_phosphor::pineal_phosphor;
use pineal_render::{Color, StrokeStyle};
use nahual_launcher::launch_app;
use nahual_theme::Theme;

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

fn main() {
    launch_app(
        "Lapaloma — phosphor trail (CRT 60 Hz)",
        (900., 480.),
        PhosphorDemo::new,
    );
}

struct PhosphorDemo {
    buffer: RingBuffer,
    t: u64,
}

impl PhosphorDemo {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            loop {
                timer.timer(SAMPLE_PERIOD).await;
                let r = this.update(cx, |me, cx| {
                    me.tick();
                    cx.notify();
                });
                if r.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            buffer: RingBuffer::new(RING_CAPACITY),
            t: 0,
        }
    }

    fn tick(&mut self) {
        let v = synthesize(self.t);
        self.buffer.push(v);
        self.t = self.t.wrapping_add(1);
    }
}

fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

impl Render for PhosphorDemo {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        // Verde fósforo CRT clásico (Tektronix vibes).
        let plot_bg = Color::rgba(0.03, 0.05, 0.04, 1.0);
        let trace = StrokeStyle::new(1.6, Color::from_hex(0x9bff8c));

        let phosphor = pineal_phosphor(self.buffer.clone(), trace)
            .background(plot_bg)
            .y_range(-1.2, 1.2)
            .trail_segments(24)
            .glow(4.0, 0.18);

        div()
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
                    .child("Lapaloma — phosphor"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .text_size(px(11.))
                    .text_color(theme.fg_muted)
                    .child(format!("cap = {}", RING_CAPACITY))
                    .child(format!("head = {}", self.buffer.head()))
                    .child("trail = 24 segs")
                    .child("glow = 4× / α 0.18")
                    .child(format!("t = {}", self.t)),
            )
            .child(div().w_full().flex_grow().child(phosphor))
    }
}
