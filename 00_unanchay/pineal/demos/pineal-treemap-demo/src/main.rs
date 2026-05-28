//! `pineal-treemap-demo` — treemap squarified con 12 tiles.
//!
//! Pesos escogidos a mano para mostrar el algoritmo cuando hay
//! mezcla de tiles grandes y chicas. El squarified minimiza el
//! peor aspect ratio en cada fila/columna; las tiles chicas
//! quedan amontonadas en una banda angosta.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

use pineal_render::{Canvas as _, Color, Rect, SceneCanvas};
use pineal_treemap::{paint_treemap, Tile};

#[derive(Clone)]
enum Msg {}

struct TreemapDemo;

impl App for TreemapDemo {
    type Model = ();
    type Msg = Msg;

    fn title() -> &'static str {
        "Lapaloma — treemap (squarified)"
    }
    fn initial_size() -> (u32, u32) {
        (1000, 620)
    }

    fn init(_: &Handle<Msg>) {}
    fn update(_: (), _: Msg, _: &Handle<Msg>) {}

    fn view(_: &()) -> View<Msg> {
        let theme = Theme::dark();
        let plot_bg = Color::rgba(0.08, 0.10, 0.13, 1.0);

        let palette = [
            0x88c0d0, 0xd08770, 0xa3be8c, 0xebcb8b, 0xb48ead, 0x5e81ac,
            0x81a1c1, 0xbf616a, 0x8fbcbb, 0xd8dee9, 0xa3be8c, 0xebcb8b,
        ];
        let weights = [40.0, 28.0, 22.0, 18.0, 14.0, 10.0, 8.0, 6.0, 5.0, 4.0, 3.0, 2.0];
        let tiles: Vec<Tile> = weights
            .iter()
            .zip(palette.iter())
            .map(|(&w, &c)| Tile::new(w, Color::from_hex(c)))
            .collect();

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "Lapaloma — treemap".to_string(),
            18.0,
            theme.fg_text,
            Alignment::Start,
        );

        let legend = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "12 tiles · pesos 40, 28, 22, 18, 14, 10, 8, 6, 5, 4, 3, 2 · gap 2 px".to_string(),
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
            paint_treemap(&tiles, outer, 2.0, &mut canvas);
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
    llimphi_ui::run::<TreemapDemo>();
}
