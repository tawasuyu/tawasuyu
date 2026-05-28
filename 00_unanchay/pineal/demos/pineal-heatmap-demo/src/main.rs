//! `pineal-heatmap-demo` — campo 2D con onda viajera.
//!
//! Matriz 48×32 que se reescribe cada 33 ms (≈ 30 Hz). El valor de
//! cada celda combina dos sinusoides con desplazamiento de fase
//! ligado al tick: `sin(x·0.25 - t·0.1) + cos(y·0.30 + t·0.07)`.
//! El ramp Viridis mapea `[min, max]` de la matriz a color.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_heatmap::{paint, HeatmapMatrix, Ramp};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const W: usize = 48;
const H: usize = 32;
const TICK_PERIOD: Duration = Duration::from_millis(33);

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Model {
    matrix: Arc<Mutex<HeatmapMatrix>>,
    t: u64,
}

struct HeatmapDemo;

impl App for HeatmapDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — heatmap 48×32 (Viridis · onda viajera)"
    }
    fn initial_size() -> (u32, u32) {
        (960, 600)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK_PERIOD, || Msg::Tick);
        let mut m = HeatmapMatrix::new(W, H);
        fill(&mut m, 0);
        Model { matrix: Arc::new(Mutex::new(m)), t: 0 }
    }

    fn update(mut model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                model.t = model.t.wrapping_add(1);
                if let Ok(mut m) = model.matrix.lock() {
                    fill(&mut m, model.t);
                }
            }
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.06, 0.07, 0.10, 1.0);
        let matrix = model.matrix.clone();

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — heatmap".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let stats = format!("matriz {}×{} · tick = {} · ramp = Viridis", W, H, model.t);
        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(stats, 11.0, theme.fg_muted, Alignment::Start);

        let panel = View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);
            if let Ok(m) = matrix.lock() {
                paint(&m, Ramp::Viridis, outer, &mut canvas);
            }
        });

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: TaffyRect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, legend, panel])
    }
}

fn fill(m: &mut HeatmapMatrix, t: u64) {
    let phase = t as f32 * 0.1;
    let phase2 = t as f32 * 0.07;
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let v = (x as f32 * 0.25 - phase).sin() + (y as f32 * 0.30 + phase2).cos();
            data.push(v);
        }
    }
    m.replace_data(data);
}

fn main() {
    llimphi_ui::run::<HeatmapDemo>();
}
