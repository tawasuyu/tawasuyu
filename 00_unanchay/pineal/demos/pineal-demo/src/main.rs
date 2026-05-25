//! `pineal-demo` — demo cartesian multi-series sobre Llimphi.
//!
//! Ventana 900×560 con un chart cartesiano de **3 series** sobre 1024
//! muestras:
//!
//! - `sin(x · 0.04)` — azul nórdico
//! - `cos(x · 0.04)` — naranja
//! - `0.5·sin(x · 0.02) + 0.5·cos(x · 0.08)` — verde
//!
//! Interacción: wheel = zoom (uniforme alrededor del cursor),
//! click = reset viewport. El pan por drag requiere callbacks
//! mouse_move/down/up que llimphi-ui aún no expone — pendiente
//! para una pasada futura cuando esos hooks estén.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Modifiers, View, WheelDelta};

use pineal_cartesian::view::{chart_cache, ChartCacheHandle};
use pineal_cartesian::{ChartView, ChartViewport};
use pineal_core::buffer::DataBuffer;
use pineal_render::{Color, StrokeStyle};

const N_SAMPLES: usize = 1024;
const WHEEL_SENSITIVITY: f64 = 0.04;

const COLOR_SIN: (u8, u8, u8) = (0x88, 0xc0, 0xd0); // azul nórdico
const COLOR_COS: (u8, u8, u8) = (0xd0, 0x87, 0x70); // naranja
const COLOR_MIX: (u8, u8, u8) = (0xa3, 0xbe, 0x8c); // verde

fn color_rgb(c: (u8, u8, u8)) -> Color {
    Color::rgb(c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0)
}

#[derive(Clone)]
enum Msg {
    /// Zoom uniforme alrededor del cursor (en fracciones [0,1] del plot).
    Zoom { factor: f64, anchor_x: f64, anchor_y: f64 },
    /// Click sobre el chart → reset al viewport inicial.
    Reset,
}

struct Model {
    series_sin: DataBuffer,
    series_cos: DataBuffer,
    series_mix: DataBuffer,
    viewport: ChartViewport,
    initial_viewport: ChartViewport,
    chart_cache: ChartCacheHandle,
    /// Tamaño actual de la ventana — necesario para mapear cursor
    /// absoluto a fracciones del plot en el handler de wheel.
    win_w: f32,
    win_h: f32,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — multi-series (wheel = zoom, click = reset)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 560)
    }

    fn init(_: &Handle<Msg>) -> Model {
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
        Model {
            series_sin: sin,
            series_cos: cos,
            series_mix: mix,
            viewport,
            initial_viewport: viewport,
            chart_cache: chart_cache(),
            win_w: 900.0,
            win_h: 560.0,
        }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Zoom { factor, anchor_x, anchor_y } => {
                model.viewport.zoom_uniform(factor, (anchor_x, anchor_y));
            }
            Msg::Reset => {
                model.viewport = model.initial_viewport;
                model.chart_cache.lock().unwrap().invalidate();
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
        // Llimphi reporta cursor con +Y hacia abajo; el viewport quiere
        // +Y hacia arriba (anchor a fondo de plot = 0).
        let ay = (1.0 - cursor.1 / model.win_h).clamp(0.0, 1.0) as f64;
        Some(Msg::Zoom { factor, anchor_x: ax, anchor_y: ay })
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.10, 0.12, 0.16, 1.0);

        let chart = ChartView::new(model.viewport)
            .background(plot_bg)
            .with_cache(model.chart_cache.clone())
            .add_series_named(
                model.series_sin.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_SIN)),
                "sin",
            )
            .add_series_named(
                model.series_cos.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_COS)),
                "cos",
            )
            .add_series_named(
                model.series_mix.clone(),
                StrokeStyle::new(2.0, color_rgb(COLOR_MIX)),
                "mix",
            )
            .view::<Msg>();

        let (pan_blits, rebuilds) = {
            let c = model.chart_cache.lock().unwrap();
            (c.pan_blits(), c.rebuilds())
        };

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — demo cartesian multi-series".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = format!(
            "sin(x · 0.04)    cos(x · 0.04)    ½·sin(x · 0.02) + ½·cos(x · 0.08)    \
             cache: {} pan-blits / {} rebuilds",
            pan_blits, rebuilds,
        );
        let legend_row = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(legend, 11.0, theme.fg_muted, Alignment::Start);

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
            gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, legend_row, plot_panel])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
