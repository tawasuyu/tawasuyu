//! Fase 4 de Llimphi: contador Elm puro.
//!
//! Bucle completo input→update→view→layout→raster→present. El click sobre
//! el botón inferior incrementa el contador; la fila superior muestra una
//! barra por unidad (placeholder hasta que llimphi tenga texto).
//!
//! Corre con: `cargo run -p llimphi-ui --example counter --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
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
        // Una barra por unidad, hasta 32.
        let count = (*model).min(32);
        let bars: Vec<View<Msg>> = (0..count)
            .map(|_| {
                View::new(Style {
                    size: Size {
                        width: length(18.0_f32),
                        height: length(48.0_f32),
                    },
                    ..Default::default()
                })
                .fill(Color::from_rgba8(90, 160, 230, 255))
                .radius(3.0)
            })
            .collect();

        let bar_row = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(64.0_f32),
            },
            gap: Size {
                width: length(6.0_f32),
                height: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(bars);

        let spacer = View::new(Style {
            flex_grow: 1.0,
            ..Default::default()
        });

        let increment = View::new(Style {
            size: Size {
                width: length(160.0_f32),
                height: length(48.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(60, 200, 130, 255))
        .radius(10.0)
        .on_click(Msg::Increment);

        let reset = View::new(Style {
            size: Size {
                width: length(100.0_f32),
                height: length(48.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(220, 80, 80, 255))
        .radius(10.0)
        .on_click(Msg::Reset);

        let buttons = View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: percent(1.0_f32),
                height: length(48.0_f32),
            },
            gap: Size {
                width: length(12.0_f32),
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
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 24, 32, 255))
        .children(vec![bar_row, spacer, buttons])
    }
}

fn main() {
    llimphi_ui::run::<Counter>();
}
