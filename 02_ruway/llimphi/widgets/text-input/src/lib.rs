//! `llimphi-widget-text-input` — input de texto single-line para Llimphi.
//!
//! Reproduce el contrato del `nahual-widget-text-input` GPUI pero
//! adaptado al modelo Elm: el estado vive en el `Model` del App, no
//! dentro del widget. Esto encaja con el bucle pure
//! `update(model, msg) -> model`.
//!
//! Uso típico:
//!
//! 1. El `Model` tiene un campo `username: TextInputState` (y
//!    `password: TextInputState::masked()` para campos enmascarados).
//! 2. El `App::on_key` ruta las teclas no especiales como `Msg::EditKey(ev)`.
//! 3. El `App::update` delega a `state.apply_key(&ev)` para que el widget
//!    aplique inserts y backspace al estado focado.
//! 4. El `App::view` invoca `text_input_view(state, placeholder, focused,
//!    palette, on_focus_msg)` para renderear cada campo.
//!
//! Limitaciones (heredadas del nahual-widget-text-input): single-line,
//! sin cursor positioning con flechas, sin selección, sin copy/paste,
//! sin IME. Para algo serio: portar el ejemplo `gpui::examples::input`.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};

/// Paleta del input. Defaults son una variante dark con borde tenue que
/// se enciende al focar, equivalente conceptual al `nahual-theme` dark.
#[derive(Debug, Clone, Copy)]
pub struct TextInputPalette {
    pub bg: Color,
    pub bg_focus: Color,
    pub border: Color,
    pub border_focus: Color,
    pub fg_text: Color,
    pub fg_placeholder: Color,
}

impl Default for TextInputPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TextInputPalette {
    /// Construye la paleta desde un `Theme` semántico.
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_input,
            bg_focus: t.bg_input_focus,
            border: t.border,
            border_focus: t.border_focus,
            fg_text: t.fg_text,
            fg_placeholder: t.fg_placeholder,
        }
    }
}

/// Estado del input. Vive en el `Model` del App; el widget lo lee para
/// renderear y el `update` lo muta vía `apply_key`.
#[derive(Debug, Clone, Default)]
pub struct TextInputState {
    text: String,
    /// Si `true`, el contenido se muestra como `•` por cada carácter — el
    /// texto real sigue accesible vía `text()`.
    masked: bool,
}

impl TextInputState {
    /// Input vacío visible (texto plano).
    pub fn new() -> Self {
        Self::default()
    }

    /// Input enmascarado — para campos de contraseña.
    pub fn masked() -> Self {
        Self {
            text: String::new(),
            masked: true,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn is_masked(&self) -> bool {
        self.masked
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn set_text(&mut self, s: impl Into<String>) {
        self.text = s.into();
    }

    pub fn push_str(&mut self, s: &str) {
        self.text.push_str(s);
    }

    pub fn pop(&mut self) -> Option<char> {
        self.text.pop()
    }

    /// Aplica una tecla al estado. Devuelve `true` si cambió el contenido.
    ///
    /// Maneja: Backspace, e inserción de caracteres imprimibles (vía
    /// `event.text`, que ya respeta layout + modifiers + IME). NO maneja:
    /// Tab, Enter, Escape, flechas — el caller decide qué hacer con esas
    /// (típicamente: cambio de foco, submit, cancel).
    pub fn apply_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != KeyState::Pressed {
            return false;
        }
        match &event.key {
            Key::Named(NamedKey::Backspace) => self.text.pop().is_some(),
            _ => {
                let Some(text) = event.text.as_ref() else {
                    return false;
                };
                if text.is_empty() || text.chars().any(|c| c.is_control()) {
                    return false;
                }
                self.text.push_str(text);
                true
            }
        }
    }
}

/// Compone el input box: borde de 1 px (rect padre coloreado), relleno
/// interno, texto o placeholder, caret simulado al final si está focado.
/// Click sobre el box emite `on_focus` (típicamente `Msg::Focus(Field)`).
pub fn text_input_view<Msg: Clone + 'static>(
    state: &TextInputState,
    placeholder: &str,
    focused: bool,
    palette: &TextInputPalette,
    on_focus: Msg,
) -> View<Msg> {
    let is_empty = state.is_empty();
    let shown = if is_empty {
        placeholder.to_string()
    } else if state.masked {
        "•".repeat(state.text.chars().count())
    } else {
        state.text.clone()
    };
    // Caret: un bloque sólido al final del texto, sin blink. El cambio
    // de borde + bg en foco ya transmite "este es el activo".
    let display = if focused && !is_empty {
        format!("{shown}\u{2588}")
    } else {
        shown
    };
    let text_color = if is_empty {
        palette.fg_placeholder
    } else {
        palette.fg_text
    };
    let (bg, border) = if focused {
        (palette.bg_focus, palette.border_focus)
    } else {
        (palette.bg, palette.border)
    };

    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .text_aligned(display, 13.0, text_color, Alignment::Start);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(34.0_f32),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(border)
    .radius(4.0)
    .on_click(on_focus)
    .children(vec![inner])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_press(key: Key, text: Option<&str>) -> KeyEvent {
        KeyEvent {
            key,
            state: KeyState::Pressed,
            text: text.map(|s| s.to_string()),
            modifiers: Default::default(),
            repeat: false,
        }
    }

    #[test]
    fn apply_key_inserts_printable_chars() {
        let mut s = TextInputState::new();
        let ev = key_press(Key::Character("a".into()), Some("a"));
        assert!(s.apply_key(&ev));
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn apply_key_backspace_pops() {
        let mut s = TextInputState::new();
        s.set_text("hola");
        let ev = key_press(Key::Named(NamedKey::Backspace), None);
        assert!(s.apply_key(&ev));
        assert_eq!(s.text(), "hol");
    }

    #[test]
    fn apply_key_ignores_tab_and_enter() {
        let mut s = TextInputState::new();
        s.set_text("hola");
        let tab = key_press(Key::Named(NamedKey::Tab), None);
        let enter = key_press(Key::Named(NamedKey::Enter), None);
        assert!(!s.apply_key(&tab));
        assert!(!s.apply_key(&enter));
        assert_eq!(s.text(), "hola");
    }

    #[test]
    fn masked_state_is_masked() {
        let s = TextInputState::masked();
        assert!(s.is_masked());
    }
}
