//! `pineal-flow-demo` — Sankey de presupuesto familiar.
//!
//! 4 fuentes de ingreso → 5 categorías de gasto → 1 nodo de ahorro.
//! El algoritmo de layout (longest-path + barycenter) ubica los
//! nodos en columnas y minimiza cruces; las bandas se tesselan con
//! curva S (smoothstep) y se rendean como triangle strips.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_flow::{compute_layout, paint_sankey, SankeyLink, SankeyNode};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

#[derive(Clone)]
enum Msg {}

struct FlowDemo;

impl App for FlowDemo {
    type Model = ();
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — Sankey (presupuesto)"
    }
    fn initial_size() -> (u32, u32) {
        (1080, 620)
    }

    fn init(_: &Handle<Msg>) {}
    fn update(_: (), _: Msg, _: &Handle<Msg>) {}

    fn view(_: &()) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);

        // 0..4: ingresos · 5..9: categorías de gasto · 10: ahorro.
        let nodes: Vec<SankeyNode> = [
            "Sueldo", "Freelance", "Renta", "Dividendos",
            "Vivienda", "Comida", "Transporte", "Ocio", "Salud",
            "Ahorro",
        ]
        .iter()
        .map(|n| SankeyNode::new(*n))
        .collect();

        let links: Vec<SankeyLink> = vec![
            // Sueldo → todo
            SankeyLink { source: 0, target: 4, value: 1200.0 },
            SankeyLink { source: 0, target: 5, value: 600.0 },
            SankeyLink { source: 0, target: 6, value: 250.0 },
            SankeyLink { source: 0, target: 9, value: 950.0 },
            // Freelance
            SankeyLink { source: 1, target: 5, value: 200.0 },
            SankeyLink { source: 1, target: 7, value: 300.0 },
            SankeyLink { source: 1, target: 9, value: 400.0 },
            // Renta
            SankeyLink { source: 2, target: 4, value: 400.0 },
            SankeyLink { source: 2, target: 8, value: 150.0 },
            // Dividendos
            SankeyLink { source: 3, target: 9, value: 350.0 },
            SankeyLink { source: 3, target: 7, value: 80.0 },
        ];

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — Sankey".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "4 ingresos → 5 categorías + ahorro · longest-path + barycenter + ribbons smoothstep"
                .to_string(),
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

            // 20 px de margen interior para el cómputo del layout.
            let area = Rect::new(outer.x + 20.0, outer.y + 20.0, outer.w - 40.0, outer.h - 40.0);
            let layout = compute_layout(&nodes, &links, area, 18.0, 8.0);
            paint_sankey(
                &layout,
                Color::from_hex(0xe5e9f0),
                Color::rgba(0.533, 0.753, 0.816, 0.45),
                &mut canvas,
            );
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

fn main() {
    llimphi_ui::run::<FlowDemo>();
}
