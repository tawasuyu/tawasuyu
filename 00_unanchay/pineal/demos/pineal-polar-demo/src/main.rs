//! `pineal-polar-demo` — pie/donut + radar sobre Llimphi.
//!
//! Dos paneles uno al lado del otro:
//! - **Pie chart (donut)** — 6 porciones de un presupuesto sintético.
//! - **Radar (spider)** — perfil de 6 atributos contra un máximo común.
//!
//! Ambos se ajustan al rect del panel: el centro y el radio se calculan
//! en el closure de `paint_with` a partir del `PaintRect` recibido.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_polar::{paint_pie, paint_radar, Slice};
use pineal_render::{Canvas as _, Color, Point, Rect, SceneCanvas, StrokeStyle};

#[derive(Clone)]
enum Msg {}

struct PolarDemo;

impl App for PolarDemo {
    type Model = ();
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — polar (pie · donut · radar)"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 520)
    }

    fn init(_: &Handle<Msg>) {}

    fn update(_: (), _: Msg, _: &Handle<Msg>) {}

    fn view(_: &()) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.10, 0.12, 0.16, 1.0);

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — polar".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "izq: donut (6 porciones) · der: radar (6 ejes, max=10)".to_string(),
            11.0,
            theme.fg_muted,
            Alignment::Start,
        );

        let pie_panel = View::new(Style {
            size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);

            let cx = outer.x + outer.w * 0.5;
            let cy = outer.y + outer.h * 0.5;
            let r_out = (outer.w.min(outer.h) * 0.42).max(20.0);
            let r_in = r_out * 0.45;

            let slices = [
                Slice::new(28.0, Color::from_hex(0x88c0d0)),
                Slice::new(18.0, Color::from_hex(0xd08770)),
                Slice::new(14.0, Color::from_hex(0xa3be8c)),
                Slice::new(12.0, Color::from_hex(0xebcb8b)),
                Slice::new(10.0, Color::from_hex(0xb48ead)),
                Slice::new(8.0, Color::from_hex(0x5e81ac)),
            ];
            paint_pie(&slices, Point::new(cx, cy), r_out, r_in, &mut canvas);
        });

        let radar_panel = View::new(Style {
            size: Size { width: percent(0.5_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            ..Default::default()
        })
        .clip(true)
        .paint_with(move |scene, ts, rect| {
            let outer = Rect::new(rect.x, rect.y, rect.w, rect.h);
            let mut canvas = SceneCanvas::new(scene, ts);
            canvas.fill_rect(outer, plot_bg);

            let cx = outer.x + outer.w * 0.5;
            let cy = outer.y + outer.h * 0.5;
            let r = (outer.w.min(outer.h) * 0.42).max(20.0);

            // Ejes guía: 4 círculos concéntricos cada 25% del radio.
            for step in 1..=4 {
                let t = step as f32 / 4.0;
                let ring: Vec<f32> = (0..=72)
                    .flat_map(|i| {
                        let a = (i as f32 / 72.0) * std::f32::consts::TAU
                            - std::f32::consts::FRAC_PI_2;
                        [cx + (r * t) * a.cos(), cy + (r * t) * a.sin()]
                    })
                    .collect();
                canvas.stroke_polyline(
                    &ring,
                    StrokeStyle::new(0.6, Color::rgba(0.55, 0.6, 0.7, 0.35)),
                );
            }

            let values = [8.0_f32, 6.5, 9.0, 4.0, 7.0, 5.5];
            paint_radar(
                &values,
                10.0,
                Point::new(cx, cy),
                r,
                Color::rgba(0.639, 0.745, 0.549, 0.35),
                StrokeStyle::new(1.6, Color::from_hex(0xa3be8c)),
                &mut canvas,
            );
        });

        let row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![pie_panel, radar_panel]);

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
        .children(vec![header, legend, row])
    }
}

fn main() {
    llimphi_ui::run::<PolarDemo>();
}
