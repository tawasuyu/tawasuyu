//! Demo del `llimphi-widget-rive-button`: un botón reactivo dirigido por máquina
//! de estados. Pasá el puntero por encima (idle→hover, gira) y hacé click
//! (pop de press). El widget guarda toda la máquina; la app sólo reenvía
//! puntero/click y avanza el reloj.
//!
//!   `cargo run -p llimphi-widget-rive-button --example rive_button_demo --release`

use std::time::Duration;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_rive_button::RiveButton;

const TICK: Duration = Duration::from_millis(16);

#[derive(Clone)]
enum Msg {
    Tick,
    Pointer(Option<(f64, f64)>),
    Click,
}

struct Model {
    boton: RiveButton,
    clicks: u32,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · rive-button"
    }

    fn initial_size() -> (u32, u32) {
        (420, 500)
    }

    fn init(handle: &Handle<Self::Msg>) -> Self::Model {
        handle.spawn_periodic(TICK, || Msg::Tick);
        Model { boton: RiveButton::builtin(), clicks: 0 }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Tick => model.boton.advance(TICK.as_secs_f64()),
            Msg::Pointer(p) => model.boton.pointer(p),
            Msg::Click => {
                model.boton.press();
                model.clicks += 1;
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let stage = View::new(Style {
            size: Size { width: length(240.0_f32), height: length(240.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .radius(16.0)
        .fill(Color::from_rgba8(26, 30, 40, 255))
        .children(vec![model.boton.view(Msg::Pointer, Msg::Click)]);

        let estado = if model.boton.is_transitioning() {
            "· · · crossfade · · ·".to_string()
        } else {
            format!("estado: {}   ·   clicks: {}", model.boton.current_state(), model.clicks)
        };
        let status = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(estado, 17.0, Color::from_rgba8(180, 200, 230, 255));

        let hint = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(
            "puntero encima = hover · click = press".to_string(),
            14.0,
            Color::from_rgba8(120, 135, 160, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            gap: Size { width: length(0.0_f32), height: length(16.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            padding: Rect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(18, 22, 30, 255))
        .children(vec![stage, status, hint])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
