//! Demo del sello wawa. Tres tamaños sobre fondo oscuro neutro.
//!
//! `cargo run -p llimphi-widget-wawa-mark --example wawa_mark_demo --release`

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_wawa_mark::{wawa_mark_view, WawaMarkPalette};

struct Demo;

impl App for Demo {
    type Model = ();
    type Msg = ();

    fn title() -> &'static str {
        "wawa · sello"
    }

    fn initial_size() -> (u32, u32) {
        (820, 420)
    }

    fn init(_: &Handle<Self::Msg>) {}
    fn update(model: Self::Model, _: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        model
    }

    fn view(_: &Self::Model) -> View<Self::Msg> {
        let palette = WawaMarkPalette::default();

        let frame = |side: f32| -> View<()> {
            View::new(Style {
                size: Size {
                    width: length(side),
                    height: length(side),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .children(vec![wawa_mark_view(&palette)])
        };

        let row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::SpaceEvenly),
            gap: Size {
                width: length(24.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(vec![frame(72.0), frame(160.0), frame(288.0)]);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(32.0_f32),
                right: length(32.0_f32),
                top: length(32.0_f32),
                bottom: length(32.0_f32),
            },
            ..Default::default()
        })
        // Fondo grafito neutro para que el rombo destaque sin competir.
        .fill(Color::from_rgba8(18, 18, 22, 255))
        .children(vec![row])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
