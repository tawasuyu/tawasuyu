//! `pineal-financial-demo` — chart OHLC con random walk sobre Llimphi.
//!
//! Genera 120 "días" de bars con un random walk determinístico (sin RNG
//! runtime — derivado de un seed fijo + xorshift32 inline) y los pinta
//! con `CandlestickView`. Wheel = zoom uniforme alrededor del cursor,
//! click = reset al viewport inicial.
//!
//! Pan por drag pendiente: requiere callbacks mouse_move/down/up que
//! llimphi-ui aún no expone.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Modifiers, View, WheelDelta};

use pineal_cartesian::ChartViewport;
use pineal_financial::{lapaloma_candlestick_view, Bar, CandlestickStyle, OhlcBuffer};
use pineal_render::Color;

const N_BARS: usize = 120;
const WHEEL_SENSITIVITY: f64 = 0.04;

#[derive(Clone)]
enum Msg {
    Zoom { factor: f64, anchor_x: f64, anchor_y: f64 },
    Reset,
}

struct Model {
    data: OhlcBuffer,
    viewport: ChartViewport,
    initial_viewport: ChartViewport,
    win_w: f32,
    win_h: f32,
}

struct FinancialDemo;

impl App for FinancialDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — candlesticks (wheel = zoom, click = reset)"
    }
    fn initial_size() -> (u32, u32) {
        (960, 560)
    }

    fn init(_: &Handle<Msg>) -> Model {
        let data = synth_random_walk(N_BARS, 100.0, 0xc0ffee);
        let (lo, hi) = data.price_range().unwrap_or((0.0, 1.0));
        let pad = (hi - lo) * 0.08;
        let viewport = ChartViewport::new(
            -0.5,
            N_BARS as f64 - 0.5,
            (lo - pad) as f64,
            (hi + pad) as f64,
        );
        Model { data, viewport, initial_viewport: viewport, win_w: 960.0, win_h: 560.0 }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Zoom { factor, anchor_x, anchor_y } => {
                model.viewport.zoom_uniform(factor, (anchor_x, anchor_y));
            }
            Msg::Reset => {
                model.viewport = model.initial_viewport;
            }
        }
        model
    }

    fn on_wheel(
        model: &Model,
        delta: WheelDelta,
        cursor: (f32, f32),
        _mods: Modifiers,
    ) -> Option<Msg> {
        if model.win_w <= 0.0 || model.win_h <= 0.0 {
            return None;
        }
        let factor = (-delta.y as f64 * WHEEL_SENSITIVITY).exp();
        let ax = (cursor.0 / model.win_w).clamp(0.0, 1.0) as f64;
        let ay = (1.0 - cursor.1 / model.win_h).clamp(0.0, 1.0) as f64;
        Some(Msg::Zoom { factor, anchor_x: ax, anchor_y: ay })
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.06, 0.08, 0.10, 1.0);

        let style = CandlestickStyle {
            bull_color: Color::rgb(0.639, 0.745, 0.549),
            bear_color: Color::rgb(0.749, 0.380, 0.416),
            ..CandlestickStyle::default()
        };

        let chart = lapaloma_candlestick_view(model.data.clone(), model.viewport)
            .background(plot_bg)
            .style(style)
            .view::<Msg>();

        let (lo, hi) = model.data.price_range().unwrap_or((0.0, 0.0));

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — candlesticks".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let stats = format!(
            "{} bars (random walk)    price [{:.2}, {:.2}]    wheel = zoom, click = reset",
            N_BARS, lo, hi,
        );
        let stats_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let plot_panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .children(vec![chart])
        .on_click(Msg::Reset);

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
    let drift = 0.05;
    let vol = 1.2;
    for i in 0..n {
        let r1 = rand_f32(&mut rng) - 0.5;
        let r2 = rand_f32(&mut rng) - 0.5;
        let r3 = rand_f32(&mut rng) - 0.5;
        let r4 = rand_f32(&mut rng) - 0.5;

        let open = close;
        let move_close = drift + r1 * vol * 2.0;
        let new_close = (open + move_close).max(1.0);
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

fn main() {
    llimphi_ui::run::<FinancialDemo>();
}
