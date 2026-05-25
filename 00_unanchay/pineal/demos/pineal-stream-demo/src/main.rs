//! `pineal-stream-demo` — osciloscopio sintético.
//!
//! Ventana con un `LapalomaStreamElement` montado sobre un
//! `RingBuffer` de 512 slots. Un timer en el background executor
//! empuja un sample cada **16 ms** (≈ 60 Hz) y dispara
//! `cx.notify()`. El sample es la suma de dos sinusoides desfasadas
//! más un poquito de ruido determinístico.
//!
//! El efecto visual: la traza barre la ventana como en un
//! osciloscopio CRT — split-at-head deja un "cursor" donde
//! arranca la traza fresca, la traza vieja se mantiene a la
//! derecha hasta que el cursor la sobrescriba.
//!
//! Showcase del **P2 zero-alloc en hot path**: el `push(v)` del
//! RingBuffer son 2 escrituras + 2 increments. Cero allocations
//! por frame, ningún `Vec` se reasigna.

use std::time::Duration;

use gpui::{div, prelude::*, px, Context, IntoElement, Render, Window};

use pineal_core::ring::RingBuffer;
use pineal_render::{Color, StrokeStyle};
use pineal_stream::pineal_stream;
use nahual_launcher::launch_app;
use nahual_theme::Theme;

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

fn main() {
    launch_app(
        "Lapaloma — stream (osciloscopio sintético 60 Hz)",
        (900., 480.),
        StreamDemo::new,
    );
}

struct StreamDemo {
    buffer: RingBuffer,
    /// Tick count global. Sirve de fase para la señal sintética y
    /// se muestra en el header para verificar que el timer corre.
    t: u64,
}

impl StreamDemo {
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

/// Señal sintética: suma de dos sinusoides + jitter determinístico.
/// El rango efectivo queda en `[-1, 1]` aproximadamente.
fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

impl Render for StreamDemo {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();

        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);
        let stroke = StrokeStyle::new(1.8, Color::from_hex(0xa3be8c));

        let stream = pineal_stream(self.buffer.clone(), stroke)
            .background(plot_bg)
            .y_range(-1.2, 1.2);

        let fill_pct = (self.buffer.filled_len() * 100) / RING_CAPACITY;

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
                    .child("Lapaloma — stream"),
            )
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .text_size(px(11.))
                    .text_color(theme.fg_muted)
                    .child(format!("cap = {}", RING_CAPACITY))
                    .child(format!("head = {}", self.buffer.head()))
                    .child(format!("filled = {}%", fill_pct))
                    .child(format!("t = {}", self.t))
                    .child(format!("rev = {}", self.buffer.revision())),
            )
            .child(div().w_full().flex_grow().child(stream))
    }
}
