//! Showcase de `llimphi-widget-tabs`: 3 tabs con contenido distinto
//! cada uno. Hover en los tabs inactivos cambia el bg.
//!
//! Corré con: `cargo run -p llimphi-widget-tabs --example showcase --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};

#[derive(Clone)]
enum Msg {
    SelectTab(usize),
}

struct Model {
    active: usize,
}

struct Showcase;

impl App for Showcase {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · tabs showcase"
    }

    fn initial_size() -> (u32, u32) {
        (900, 600)
    }

    fn init(_: &Handle<Msg>) -> Model {
        Model { active: 0 }
    }

    fn update(model: Model, msg: Msg, _: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::SelectTab(i) => m.active = i,
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let body = match model.active {
            0 => content_pane(
                "General",
                "Acá vivirían los settings principales del módulo.\n\
                 El click cambia de tab; el hover sobre tabs inactivos\n\
                 ilumina el fondo levemente.",
                Color::from_rgba8(220, 230, 245, 255),
            ),
            1 => content_pane(
                "Avanzado",
                "Variables esotéricas, banderas experimentales.\n\
                 Probablemente no las toques.",
                Color::from_rgba8(200, 220, 240, 255),
            ),
            _ => content_pane(
                "Logs",
                "[12:01:33] arranqué\n[12:01:34] cargué config\n\
                 [12:01:35] esperando eventos…",
                Color::from_rgba8(180, 195, 215, 255),
            ),
        };

        tabs_view(TabsSpec {
            labels: vec!["General".into(), "Avanzado".into(), "Logs".into()],
            active: model.active,
            on_select: Msg::SelectTab,
            content: body,
            tab_height: 36.0,
            palette: TabsPalette::default(),
            tab_width: Some(160.0),
        })
    }
}

fn content_pane(title: &str, body: &str, fg: Color) -> View<Msg> {
    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(36.0_f32),
        },
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(8.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Start),
        ..Default::default()
    })
    .text_aligned(
        format!("# {title}"),
        18.0,
        Color::from_rgba8(220, 230, 245, 255),
        Alignment::Start,
    );

    let body_view = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        padding: Rect {
            left: length(20.0_f32),
            right: length(20.0_f32),
            top: length(0.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(body.to_string(), 13.0, fg, Alignment::Start);

    View::new(Style {
        flex_direction: llimphi_ui::llimphi_layout::taffy::FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .children(vec![header, body_view])
}

fn main() {
    llimphi_ui::run::<Showcase>();
}
