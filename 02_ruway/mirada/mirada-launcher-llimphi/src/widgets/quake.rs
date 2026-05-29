//! Input quake — input elegante toggleable.
//!
//! En modo "barra": ocupa poco, dice `›` y un placeholder. Al togglear se
//! expande dentro de la barra (defer: levantar overlay full-screen tipo
//! Quake/Spotlight). Escribir va al estado interno; Enter "submitea" — por
//! ahora dispara el comando como `sh -c`. Más adelante: target=auto
//! (terminal/app/ia/ssh).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::{KeyEvent, KeyState, View};

use crate::config::WidgetSpec;
use crate::keys;
use crate::widget::{Msg, Widget};

pub struct QuakeInput {
    pub open: bool,
    pub buffer: String,
    pub placeholder: String,
    pub width_open: f32,
    pub width_closed: f32,
    /// Etiqueta del hotkey leída del TOML (p. ej. "F12"). Vacío =
    /// sin hotkey (el widget se abre por click).
    pub hotkey: String,
}

impl QuakeInput {
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let placeholder = spec.str_prop("placeholder", "› preguntá, lanzá, navegá").to_string();
        let width_open = spec.float_prop("width_open", 360.0) as f32;
        let width_closed = spec.float_prop("width_closed", 140.0) as f32;
        let hotkey = spec.str_prop("hotkey", "").to_string();
        Self {
            open: false,
            buffer: String::new(),
            placeholder,
            width_open,
            width_closed,
            hotkey,
        }
    }

    /// Llamado por la app para mutar al recibir mensajes del input.
    pub fn apply(&mut self, msg: &Msg) {
        match msg {
            Msg::QuakeToggle => {
                self.open = !self.open;
                if !self.open {
                    self.buffer.clear();
                }
            }
            Msg::QuakeChar(c) => {
                if self.open {
                    self.buffer.push(*c);
                }
            }
            Msg::QuakeBackspace => {
                if self.open {
                    self.buffer.pop();
                }
            }
            Msg::QuakeSubmit => {
                if self.open && !self.buffer.is_empty() {
                    eprintln!("quake · submit: {}", self.buffer);
                    self.buffer.clear();
                    self.open = false;
                }
            }
            _ => {}
        }
    }
}

impl Widget for QuakeInput {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn try_key(&self, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if !self.hotkey.is_empty() && keys::matches(&self.hotkey, &event.key) {
            return Some(Msg::QuakeToggle);
        }
        None
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        let (label, color, bg) = if self.open {
            let content = if self.buffer.is_empty() {
                self.placeholder.clone()
            } else {
                format!("› {}", self.buffer)
            };
            let color = if self.buffer.is_empty() {
                theme.fg_placeholder
            } else {
                theme.fg_text
            };
            (content, color, theme.bg_input_focus)
        } else {
            ("› hablar".to_string(), theme.fg_muted, theme.bg_input)
        };

        let len = if self.open { self.width_open } else { self.width_closed };

        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: length(len), height: length(22.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .fill(bg)
        .radius(4.0)
        .text(label, 12.0, color)
        .on_click(Msg::QuakeToggle)
    }
}
