//! `pineal-phosphor-demo` — osciloscopio con trail CRT sobre Llimphi.
//!
//! Mismo setup que `pineal-stream-demo` (RingBuffer 512 + timer 60 Hz)
//! pero el render usa `PhosphorView`: el trail decae en alpha del cursor
//! hacia atrás y arrastra un halo (glow). Visualmente queda como un
//! osciloscopio analógico con fósforo persistente.

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_core::ring::RingBuffer;
use pineal_phosphor::pineal_phosphor_view;
use pineal_render::{Color, StrokeStyle};

const RING_CAPACITY: usize = 512;
const SAMPLE_PERIOD: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Model {
    buffer: RingBuffer,
    t: u64,
}

struct PhosphorDemo;

impl App for PhosphorDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — phosphor trail (CRT 60 Hz)"
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
        let plot_bg = Color::rgba(0.03, 0.05, 0.04, 1.0);
        let trace = StrokeStyle::new(1.6, Color::rgb(0.608, 1.0, 0.549));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned("Lapaloma — phosphor".to_string(), 18.0, theme.fg_text, Alignment::Start);

        let stats = format!(
            "cap = {}    head = {}    trail = 24 segs    glow = 4× / α 0.18    t = {}",
            RING_CAPACITY,
            model.buffer.head(),
            model.t,
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let phosphor = pineal_phosphor_view(model.buffer.clone(), trace)
            .background(plot_bg)
            .y_range(-1.2, 1.2)
            .trail_segments(24)
            .glow(4.0, 0.18)
            .view::<Msg>();

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![phosphor]);

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

fn synthesize(t: u64) -> f32 {
    let phase = t as f32;
    let signal = (phase * 0.07).sin() * 0.75 + (phase * 0.19).sin() * 0.22;
    let jitter = ((phase * 37.0).sin() * 1000.0).fract() * 0.04;
    signal + jitter
}

fn main() {
    llimphi_ui::run::<PhosphorDemo>();
}
