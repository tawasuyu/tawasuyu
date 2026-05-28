//! `pineal-contour-demo` — campo escalar con 8 isolíneas + heatmap base.
//!
//! Renderiza primero el heatmap Viridis del campo (para contexto), y
//! encima 8 isolíneas extraídas por marching squares con gradiente
//! azul→rojo. Matriz 64×48; el campo es
//! `sin(x · 0.4 - t · 0.1) + cos(y · 0.4 + t · 0.07)` con un tick lento
//! para que se vea la deformación.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_contour::paint_contours;
use pineal_heatmap::{paint as paint_heatmap, HeatmapMatrix, Ramp};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const W: usize = 64;
const H: usize = 48;
const TICK: Duration = Duration::from_millis(80);

#[derive(Clone)]
enum Msg {
    Tick,
}

struct Model {
    matrix: Arc<Mutex<HeatmapMatrix>>,
    t: u64,
}

struct ContourDemo;

impl App for ContourDemo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — contour (campo + 8 isolíneas)"
    }
    fn initial_size() -> (u32, u32) {
        (960, 640)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
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
        let plot_bg = Color::rgba(0.05, 0.06, 0.09, 1.0);
        let matrix = model.matrix.clone();

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — contour".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!("campo {}×{} · 8 isolíneas · marching squares · tick = {}", W, H, model.t),
            11.0,
            theme.fg_muted,
            Alignment::Start,
        );

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
                paint_heatmap(&m, Ramp::Viridis, outer, &mut canvas);
                paint_contours(
                    &m,
                    8,
                    outer,
                    Color::rgba(0.4, 0.6, 1.0, 0.9),
                    Color::rgba(1.0, 0.4, 0.3, 0.95),
                    1.2,
                    &mut canvas,
                );
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
    let phase = t as f32 * 0.10;
    let phase2 = t as f32 * 0.07;
    let mut data = Vec::with_capacity(W * H);
    for y in 0..H {
        for x in 0..W {
            let v = (x as f32 * 0.4 - phase).sin() + (y as f32 * 0.4 + phase2).cos();
            data.push(v);
        }
    }
    m.replace_data(data);
}

fn main() {
    llimphi_ui::run::<ContourDemo>();
}
