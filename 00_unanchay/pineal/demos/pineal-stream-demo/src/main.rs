//! `pineal-stream-demo` — osciloscopio sintético sobre Llimphi.
//!
//! Ventana con un `StreamView` montado sobre un `RingBuffer` de 512
//! slots. Un thread periódico empuja un sample cada **16 ms** (≈ 60 Hz)
//! vía `Handle::spawn_periodic` y dispatcha `Msg::Tick` al update.
//!
//! El efecto visual: la traza barre la ventana como en un osciloscopio
//! CRT — split-at-head deja un "cursor" donde arranca la traza fresca,
//! la traza vieja se mantiene a la derecha hasta que el cursor la
//! sobrescriba.
//!
//! Showcase del **P2 zero-alloc en hot path**: el `push(v)` del
//! RingBuffer son 2 escrituras + 2 increments. Cero allocations por
//! frame, ningún `Vec` se reasigna en el sampler.

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_core::ring::RingBuffer;
use pineal_render::{Color, StrokeStyle};
use pineal_stream::pineal_stream_view;

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Model {
    buffer: RingBuffer,
    /// Tick count global. Sirve de fase para la señal sintética y se
    /// muestra en el header para verificar que el timer corre.
    t: u64,
}

struct StreamDemo;

impl App for StreamDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — stream (osciloscopio sintético 60 Hz)"
    }

    fn initial_size() -> (u32, u32) {
        (900, 480)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(SAMPLE_PERIOD, || Msg::Tick);
        Model { buffer: RingBuffer::new(RING_CAPACITY), t: 0 }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                let v = synthesize(model.t);
                model.buffer.push(v);
                model.t = model.t.wrapping_add(1);
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);
        let stroke = StrokeStyle::new(1.8, Color::rgb(0.639, 0.745, 0.549));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            padding: Rect {
                left: length(2.0_f32),
                right: length(2.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .text_aligned("Lapaloma — stream".to_string(), 18.0, theme.fg_text, Alignment::Start);

        let fill_pct = (model.buffer.filled_len() * 100) / RING_CAPACITY;
        let stats = format!(
            "cap = {}    head = {}    filled = {}%    t = {}    rev = {}",
            RING_CAPACITY,
            model.buffer.head(),
            fill_pct,
            model.t,
            model.buffer.revision(),
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let stream = pineal_stream_view(model.buffer.clone(), stroke)
            .background(plot_bg)
            .y_range(-1.2, 1.2)
            .view::<Msg>();

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![stream]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, stats_row, plot_panel])
    }
}

/// Señal sintética: suma de dos sinusoides + jitter determinístico. El
/// rango efectivo queda en `[-1, 1]` aproximadamente.
fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

fn main() {
    llimphi_ui::run::<StreamDemo>();
}
