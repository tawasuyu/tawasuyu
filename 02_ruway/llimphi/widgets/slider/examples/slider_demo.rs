//! Showcase de `llimphi-widget-slider`: tres sliders sobre un Model que
//! acumula deltas en vivo. Corré con:
//!
//! ```text
//! cargo run -p llimphi-widget-slider --example slider_demo
//! ```

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, DragPhase, Handle, View};
use llimphi_widget_slider::{slider_view, SliderPalette};

#[derive(Clone, Debug)]
enum Msg {
    EditPsique(f32),
    EditMateria(f32),
    EditPoder(f32),
}

struct Model {
    psique: f32,
    materia: f32,
    poder: f32,
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · slider demo"
    }

    fn initial_size() -> (u32, u32) {
        (520, 280)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model { psique: 0.0, materia: 0.5, poder: -0.25 }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::EditPsique(dv) => m.psique = (m.psique + dv).clamp(-1.0, 1.0),
            Msg::EditMateria(dv) => m.materia = (m.materia + dv).clamp(-1.0, 1.0),
            Msg::EditPoder(dv) => m.poder = (m.poder + dv).clamp(-1.0, 1.0),
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = Theme::dark();
        let palette = SliderPalette::from_theme(&theme);

        let header = View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
            ..Default::default()
        })
        .text_aligned(
            "ajustá los sliders — el Model acumula deltas en vivo".to_string(),
            13.0,
            theme.fg_text,
            Alignment::Start,
        );

        let psique = slider_view(
            "psique",
            model.psique,
            -1.0,
            1.0,
            &palette,
            |phase, dv| match phase {
                DragPhase::Move => Some(Msg::EditPsique(dv)),
                DragPhase::End => None,
            },
        );
        let materia = slider_view(
            "materia",
            model.materia,
            -1.0,
            1.0,
            &palette,
            |phase, dv| match phase {
                DragPhase::Move => Some(Msg::EditMateria(dv)),
                DragPhase::End => None,
            },
        );
        let poder = slider_view(
            "poder",
            model.poder,
            -1.0,
            1.0,
            &palette,
            |phase, dv| match phase {
                DragPhase::Move => Some(Msg::EditPoder(dv)),
                DragPhase::End => None,
            },
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(16.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, psique, materia, poder])
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
