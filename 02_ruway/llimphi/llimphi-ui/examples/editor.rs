//! Editor mínimo: text field con char insertion, backspace, enter, ctrl+L
//! para limpiar. Valida que el bucle Elm absorbe input de teclado.
//!
//! Corre con: `cargo run -p llimphi-ui --example editor --release`.

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

#[derive(Clone)]
enum Msg {
    Insert(String),
    Backspace,
    Clear,
}

struct Editor;

impl App for Editor {
    type Model = String;
    type Msg = Msg;

    fn title() -> &'static str {
        "llimphi · editor"
    }

    fn init(_: &Handle<Self::Msg>) -> Self::Model {
        String::new()
    }

    fn update(model: Self::Model, msg: Self::Msg, _: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::Insert(s) => {
                let mut m = model;
                m.push_str(&s);
                m
            }
            Msg::Backspace => {
                let mut m = model;
                m.pop();
                m
            }
            Msg::Clear => String::new(),
        }
    }

    fn on_key(_: &Self::Model, e: &KeyEvent) -> Option<Self::Msg> {
        if e.state != KeyState::Pressed {
            return None;
        }
        if e.modifiers.ctrl {
            if let Key::Character(c) = &e.key {
                if c.eq_ignore_ascii_case("l") {
                    return Some(Msg::Clear);
                }
            }
            return None;
        }
        match &e.key {
            Key::Named(NamedKey::Backspace) => Some(Msg::Backspace),
            Key::Named(NamedKey::Enter) => Some(Msg::Insert("\n".into())),
            Key::Named(NamedKey::Tab) => Some(Msg::Insert("    ".into())),
            _ => e.text.clone().map(Msg::Insert),
        }
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let body_text = if model.is_empty() {
            "tipea algo · ctrl+L limpia · enter salto · backspace borra".to_string()
        } else {
            // Cursor visual al final del contenido.
            format!("{model}\u{2588}")
        };
        let body_color = if model.is_empty() {
            Color::from_rgba8(110, 130, 150, 255)
        } else {
            Color::from_rgba8(220, 230, 240, 255)
        };

        let body = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_grow: 1.0,
            ..Default::default()
        })
        .text_aligned(body_text, 22.0, body_color, Alignment::Start);

        let status = View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(36.0_f32),
            },
            ..Default::default()
        })
        .fill(Color::from_rgba8(30, 36, 48, 255))
        .text(
            format!("{} chars", model.chars().count()),
            16.0,
            Color::from_rgba8(160, 180, 200, 255),
        );

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
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
        .children(vec![body, status])
    }
}

fn main() {
    llimphi_ui::run::<Editor>();
}
