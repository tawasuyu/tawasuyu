//! Galería de los iconos de marca de todas las apps de gioser.
//!
//! Pinta los 29 [`AppIcon`] en una grilla, cada uno en su color de marca
//! con su nombre debajo. Sirve para eyeballear de un vistazo que el set
//! es coherente (mismo peso de trazo, mismo aire) y que cada glifo es
//! reconocible.
//!
//! `cargo run -p llimphi-icons --example app_icons_gallery --release`

use llimphi_icons::app_icons::{app_icon_view, AppIcon, ALL};
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect;
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};

const COLS: usize = 6;
const BG: Color = Color::from_rgb8(18, 20, 24);
const CELL: Color = Color::from_rgb8(28, 31, 38);
const LABEL: Color = Color::from_rgb8(196, 202, 212);

struct Model;

#[derive(Clone)]
enum Msg {}

fn cell(icon: AppIcon) -> View<Msg> {
    // Recuadro del glifo (cuadrado, el icono se escala al lado menor).
    let icon_box = View::new(Style {
        size: Size {
            width: length(52.0_f32),
            height: length(52.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![app_icon_view(icon, 2.0)]);

    let label = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(16.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(icon.name().to_string(), 11.0, LABEL, Alignment::Center);

    View::new(Style {
        size: Size {
            width: length(118.0_f32),
            height: length(96.0_f32),
        },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(CELL)
    .radius(12.0)
    .children(vec![icon_box, label])
}

fn row(icons: &[AppIcon]) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: auto(),
            height: auto(),
        },
        flex_direction: FlexDirection::Row,
        gap: Size {
            width: length(14.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(icons.iter().copied().map(cell).collect())
}

struct Gallery;

impl App for Gallery {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi-icons · galería de apps"
    }

    fn initial_size() -> (u32, u32) {
        (820, 620)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model
    }

    fn update(_model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        match msg {}
    }

    fn view(_: &Model) -> View<Msg> {
        let rows: Vec<View<Msg>> = ALL.chunks(COLS).map(row).collect();
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_direction: FlexDirection::Column,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(0.0_f32),
                height: length(14.0_f32),
            },
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(20.0_f32),
                bottom: length(20.0_f32),
            },
            ..Default::default()
        })
        .fill(BG)
        .children(rows)
    }
}

fn main() {
    llimphi_ui::run::<Gallery>();
}
