//! Showcase de `llimphi-widget-theme-switcher`.
//!
//! Una ventana con el switcher en la cabecera + un sample de paneles
//! que cambian de color al ciclar. Validación visual de que el theme
//! propaga a la UI: al hacer click en el switcher, los paneles se
//! repintan con el siguiente preset.
//!
//! Corré: `cargo run -p llimphi-widget-theme-switcher --example theme_switcher_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_theme_switcher::theme_switcher_view;

#[derive(Clone, Debug)]
enum Msg {
    ChangeTheme(Theme),
}

struct Model {
    theme: Theme,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · theme-switcher"
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model {
            theme: Theme::dark(),
        }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::ChangeTheme(t) => m.theme = t,
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let switcher = theme_switcher_view(&model.theme, Msg::ChangeTheme);

        let header = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(48.0_f32),
            },
            flex_direction: FlexDirection::Row,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::SpaceBetween),
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_panel)
        .children(vec![
            View::new(Style {
                size: Size {
                    width: length(220.0_f32),
                    height: length(32.0_f32),
                },
                ..Default::default()
            })
            .text_aligned(
                format!("Preset actual: {}", model.theme.name),
                13.0,
                model.theme.fg_text,
                Alignment::Start,
            ),
            switcher,
        ]);

        let card_a = sample_card("Panel principal", &model.theme, model.theme.bg_panel);
        let card_b = sample_card(
            "Strip alternativo",
            &model.theme,
            model.theme.bg_panel_alt,
        );
        let card_c = sample_card("Input focado", &model.theme, model.theme.bg_input_focus);

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            padding: Rect {
                left: length(24.0_f32),
                right: length(24.0_f32),
                top: length(24.0_f32),
                bottom: length(24.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(14.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(vec![card_a, card_b, card_c]);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .fill(model.theme.bg_app)
        .children(vec![header, body])
    }
}

fn sample_card(label: &str, theme: &Theme, bg: llimphi_ui::llimphi_raster::peniko::Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(60.0_f32),
        },
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(6.0)
    .text_aligned(label.to_string(), 13.0, theme.fg_text, Alignment::Start)
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
