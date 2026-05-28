//! `pineal-hexbin-demo` — 5 000 puntos sintéticos bineados.
//!
//! Generador determinista (LCG sobre `t`) que produce dos clusters
//! gaussianos solapados. El hexbin revela la densidad — cada celda se
//! colorea con Viridis según count. Sin animación: el chart se computa
//! una vez al iniciar.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_heatmap::Ramp;
use pineal_hexbin::{paint_hexbin, HexGrid};
use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};

const N_POINTS: usize = 5000;
const HEX_RADIUS: f32 = 9.0;

#[derive(Clone)]
enum Msg {}

struct HexbinDemo;

impl App for HexbinDemo {
    type Model = HexGrid;
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — hexbin (5 000 puntos gaussianos)"
    }
    fn initial_size() -> (u32, u32) {
        (900, 620)
    }

    fn init(_: &Handle<Msg>) -> HexGrid {
        let mut g = HexGrid::new(HEX_RADIUS);
        // LCG determinista — no agrega dep para randomness.
        let mut state: u64 = 0xC0FFEE;
        let mut rng = || -> f32 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 32) as f32) / (u32::MAX as f32)
        };
        let mut gauss = || -> (f32, f32) {
            // Box-Muller.
            let u1 = (rng()).max(1e-9);
            let u2 = rng();
            let r = (-2.0 * u1.ln()).sqrt();
            let theta = 2.0 * std::f32::consts::PI * u2;
            (r * theta.cos(), r * theta.sin())
        };
        // Cluster A: centro (300, 300), sigma 50. Cluster B: (520, 380), sigma 80.
        for i in 0..N_POINTS {
            let (g0, g1) = gauss();
            if i % 3 == 0 {
                g.push(300.0 + g0 * 50.0, 300.0 + g1 * 50.0);
            } else {
                g.push(520.0 + g0 * 80.0, 380.0 + g1 * 80.0);
            }
        }
        g
    }

    fn update(model: HexGrid, _: Msg, _: &Handle<Msg>) -> HexGrid {
        model
    }

    fn view(grid: &HexGrid) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.06, 0.08, 0.10, 1.0);
        let snapshot = grid.clone();
        let (min, max) = grid.min_max();

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — hexbin".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            format!(
                "{} pts · radio {} px · {} bines · count ∈ [{}, {}] · Viridis",
                N_POINTS,
                HEX_RADIUS as i32,
                grid.cells().count(),
                min,
                max,
            ),
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
            paint_hexbin(&snapshot, Ramp::Viridis, (outer.x, outer.y), &mut canvas);
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
    llimphi_ui::run::<HexbinDemo>();
}
