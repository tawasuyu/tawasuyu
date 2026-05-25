//! Showcase de `llimphi-widget-splitter`: dos splits anidados
//! draggables (Row con Column adentro).
//!
//! Corré con: `cargo run -p llimphi-widget-splitter --example showcase --release`.
//!
//! Probá: agarrá el divisor vertical y arrastralo izquierda/derecha
//! para resizar el pane izquierdo; agarrá el divisor horizontal de la
//! derecha para resizar el pane superior derecho.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_splitter::{splitter_two, Direction, PaneSize, SplitterPalette};

#[derive(Clone)]
enum Msg {
    ResizeOuter(f32),
    ResizeInner(f32),
}

struct Model {
    left_w: f32,
    top_h: f32,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · splitter showcase"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 720)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            left_w: 320.0,
            top_h: 240.0,
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::ResizeOuter(dx) => {
                m.left_w = (m.left_w + dx).clamp(120.0, 800.0);
            }
            Msg::ResizeInner(dy) => {
                m.top_h = (m.top_h + dy).clamp(80.0, 600.0);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let palette = SplitterPalette::default();

        let left = pane("izquierdo", Color::from_rgba8(28, 36, 50, 255));
        let top_right = pane(
            &format!("arriba · {:.0} px", model.top_h),
            Color::from_rgba8(38, 50, 70, 255),
        );
        let bottom_right = pane(
            "abajo · flex",
            Color::from_rgba8(48, 36, 60, 255),
        );

        let right = splitter_two(
            Direction::Column,
            top_right,
            PaneSize::Fixed(model.top_h),
            bottom_right,
            PaneSize::Flex,
            |phase, dy| match phase {
                DragPhase::Move => Some(Msg::ResizeInner(dy)),
                DragPhase::End => None,
            },
            &palette,
        );

        splitter_two(
            Direction::Row,
            left,
            PaneSize::Fixed(model.left_w),
            right,
            PaneSize::Flex,
            |phase, dx| match phase {
                DragPhase::Move => Some(Msg::ResizeOuter(dx)),
                DragPhase::End => None,
            },
            &palette,
        )
    }
}

fn pane(label: &str, bg: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(
        label.to_string(),
        18.0,
        Color::from_rgba8(220, 230, 240, 255),
        Alignment::Center,
    )
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
