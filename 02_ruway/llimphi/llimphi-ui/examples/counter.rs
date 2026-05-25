//! Fase 4 de Llimphi: contador Elm puro con texto real.
//!
//! Bucle completo input→update→view→layout→raster→present. El click sobre
//! el botón inferior incrementa el contador; el panel central muestra el
//! número actual rasterizado por skrifa+vello.
//!
//! Corre con: `cargo run -p llimphi-ui --example counter --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, View};

#[derive(Clone)]
enum Msg {
    Increment,
    Reset,
}

struct Counter;

impl App for Counter {
    type Model = u32;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · counter"
    }

    fn init() -> Self::Model {
        0
    }

    fn update(model: Self::Model, msg: Self::Msg) -> Self::Model {
        match msg {
            Msg::Increment => model.saturating_add(1),
            Msg::Reset => 0,
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let number = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(model.to_string(), 160.0, Color::from_rgba8(230, 240, 250, 255));

        let increment = View::new(Style {
            size: Size {
                width: length(160.0_f32),
                height: length(56.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(60, 200, 130, 255))
        .radius(12.0)
        .text("+1", 28.0, Color::from_rgba8(10, 30, 20, 255))
        .on_click(Msg::Increment);

        let reset = View::new(Style {
            size: Size {
                width: length(120.0_f32),
                height: length(56.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(220, 80, 80, 255))
        .radius(12.0)
        .text("reset", 22.0, Color::from_rgba8(30, 10, 10, 255))
        .on_click(Msg::Reset);

        let buttons = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(56.0_f32),
            },
            gap: Size {
                width: length(16.0_f32),
                height: length(0.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .children(vec![increment, reset]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(24.0_f32),
            },
            padding: llimphi_ui::llimphi_layout::taffy::Rect {
                left: length(32.0_f32),
                right: length(32.0_f32),
                top: length(32.0_f32),
                bottom: length(32.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 24, 32, 255))
        .children(vec![number, buttons])
    }
}

fn main() {
    llimphi_ui::run::<Counter>();
}
