//! Showcase de `llimphi-widget-button`: tres botones con hover.
//!
//! Corré con: `cargo run -p llimphi-widget-button --example showcase --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_button::{button_styled, button_view, ButtonPalette};

#[derive(Clone, Debug)]
enum Msg {
    A,
    B,
    C,
}

struct Model {
    last: Option<Msg>,
    counter: u32,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · button showcase"
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            last: None,
            counter: 0,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        m.counter += 1;
        m.last = Some(msg);
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = ButtonPalette::default();
        let warning = ButtonPalette {
            bg: Color::from_rgba8(140, 70, 30, 255),
            bg_hover: Color::from_rgba8(200, 100, 40, 255),
            ..palette
        };
        let danger = ButtonPalette {
            bg: Color::from_rgba8(150, 40, 40, 255),
            bg_hover: Color::from_rgba8(220, 70, 70, 255),
            ..palette
        };

        let a = button_view("acción A", &palette, Msg::A);
        let b = button_view("acción B (warning)", &warning, Msg::B);
        let c = button_styled(
            "borrar (left-aligned, fixed width)",
            Style {
                size: Size {
                    width: length(320.0_f32),
                    height: length(34.0_f32),
                },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            },
            Alignment::Start,
            &danger,
            Msg::C,
        );

        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(40.0_f32),
            },
            ..Default::default()
        })
        .text_aligned(
            format!(
                "clicks: {} · último: {}",
                model.counter,
                match model.last {
                    Some(Msg::A) => "A",
                    Some(Msg::B) => "B",
                    Some(Msg::C) => "C",
                    None => "—",
                }
            ),
            14.0,
            Color::from_rgba8(180, 190, 205, 255),
            Alignment::Start,
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(14.0_f32),
            },
            padding: Rect {
                left: length(32.0_f32),
                right: length(32.0_f32),
                top: length(32.0_f32),
                bottom: length(32.0_f32),
            },
            align_items: Some(AlignItems::Start),
            justify_content: Some(JustifyContent::Start),
            ..Default::default()
        })
        .fill(Color::from_rgba8(20, 24, 32, 255))
        .children(vec![a, b, c, status])
    }
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
