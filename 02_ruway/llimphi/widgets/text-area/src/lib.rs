//! `llimphi-widget-text-area` — input de texto multilínea para Llimphi.
//!
//! Versión multilínea del [`llimphi-widget-text-input`]. Mismo contrato Elm
//! (estado en el `Model`, `apply_key` desde el `update`, view con foco),
//! pero acepta `\n` como contenido válido: Enter inserta salto de línea
//! en lugar de "submit". El llamador decide cómo commitear (típicamente
//! Ctrl+Enter o un botón ✓ aparte).
//!
//! El render aprovecha que `View::text_aligned` ya hace layout multilínea
//! vía parley (line wrap por `max_width`, saltos `\n` respetados).
//!
//! Limitaciones del PMV (heredadas del text-input): sin posicionamiento
//! del cursor con flechas, sin selección, sin copy/paste, sin IME. El
//! caret se simula como un bloque sólido al final del texto.

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Key, KeyEvent, KeyState, NamedKey, View};

/// Paleta del text-area — mismos slots que el text-input.
#[derive(Debug, Clone, Copy)]
pub struct TextAreaPalette {
    pub bg: Color,
    pub bg_focus: Color,
    pub border: Color,
    pub border_focus: Color,
    pub fg_text: Color,
    pub fg_placeholder: Color,
}

impl Default for TextAreaPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TextAreaPalette {
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

/// Estado del text-area. Vive en el `Model`; `apply_key` se llama desde
/// el `update` para ediciones por tecla.
#[derive(Debug, Clone, Default)]
pub struct TextAreaState {
    text: String,
}

impl TextAreaState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
    }

    pub fn set_text(&mut self, s: impl Into<String>) {
        self.text = s.into();
    }

    /// Cantidad de líneas (≥ 1, mismo criterio que `str::lines` + 1 si
    /// el texto termina en `\n`).
    pub fn line_count(&self) -> usize {
        if self.text.is_empty() {
            return 1;
        }
        let mut n = self.text.lines().count();
        if self.text.ends_with('\n') {
            n += 1;
        }
        n.max(1)
    }

    /// Aplica una tecla al estado. Devuelve `true` si cambió el contenido.
    ///
    /// Maneja: Backspace, Enter (inserta `\n`), e inserción de
    /// caracteres imprimibles vía `event.text`. NO maneja: Tab (lo
    /// dejamos al caller — típicamente cambio de foco o indent),
    /// Escape, flechas.
    pub fn apply_key(&mut self, event: &KeyEvent) -> bool {
        if event.state != KeyState::Pressed {
            return false;
        }
        match &event.key {
            Key::Named(NamedKey::Backspace) => self.text.pop().is_some(),
            Key::Named(NamedKey::Enter) => {
                self.text.push('\n');
                true
            }
            _ => {
                let Some(text) = event.text.as_ref() else {
                    return false;
                };
                // Filtramos caracteres de control — el `\n` lo metemos
                // sólo desde NamedKey::Enter para tener un único path.
                if text.is_empty() || text.chars().any(|c| c.is_control()) {
                    return false;
                }
                self.text.push_str(text);
                true
            }
        }
    }
}

/// Render del text-area. `body_height` es el alto disponible del bloque
/// (el widget no calcula altura automática; el caller decide). Con foco
/// se pinta un caret bloque al final del texto.
pub fn text_area_view<Msg: Clone + 'static>(
    state: &TextAreaState,
    placeholder: &str,
    focused: bool,
    body_height: f32,
    palette: &TextAreaPalette,
    on_focus: Msg,
) -> View<Msg> {
    let is_empty = state.is_empty();
    let display = if is_empty {
        placeholder.to_string()
    } else if focused {
        // Caret bloque al final — sin blink. Parley lo pinta como un
        // glifo más, así no rompe el layout multilínea.
        format!("{}\u{2588}", state.text)
    } else {
        state.text.clone()
    };
    let text_color = if is_empty { palette.fg_placeholder } else { palette.fg_text };
    let (bg, border) = if focused {
        (palette.bg_focus, palette.border_focus)
    } else {
        (palette.bg, palette.border)
    };

    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(body_height),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(display, 12.0, text_color, Alignment::Start);

    // Wrapper que pinta el borde como fill del padre (1 px alrededor
    // del inner gracias al padding del padre).
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(body_height + 2.0),
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
    .on_click(on_focus)
    .children(vec![inner])
}

#[cfg(test)]
mod tests {
    use super::*;
    use llimphi_ui::Modifiers;

    fn k(named: NamedKey) -> KeyEvent {
        KeyEvent {
            key: Key::Named(named),
            state: KeyState::Pressed,
            text: None,
            modifiers: Modifiers::default(),
            repeat: false,
        }
    }

    fn k_text(s: &str) -> KeyEvent {
        KeyEvent {
            key: Key::Character(s.into()),
            state: KeyState::Pressed,
            text: Some(s.to_owned()),
            modifiers: Modifiers::default(),
            repeat: false,
        }
    }

    #[test]
    fn enter_inserta_salto_de_linea() {
        let mut s = TextAreaState::new();
        s.apply_key(&k_text("a"));
        s.apply_key(&k(NamedKey::Enter));
        s.apply_key(&k_text("b"));
        assert_eq!(s.text(), "a\nb");
        assert_eq!(s.line_count(), 2);
    }

    #[test]
    fn backspace_borra_el_salto_y_une_lineas() {
        let mut s = TextAreaState::new();
        s.set_text("a\nb");
        s.apply_key(&k(NamedKey::Backspace));
        s.apply_key(&k(NamedKey::Backspace));
        assert_eq!(s.text(), "a");
    }

    #[test]
    fn line_count_vacio_es_uno() {
        let s = TextAreaState::new();
        assert_eq!(s.line_count(), 1);
    }

    #[test]
    fn line_count_cuenta_trailing_newline() {
        let mut s = TextAreaState::new();
        s.set_text("a\nb\n");
        assert_eq!(s.line_count(), 3);
    }

    #[test]
    fn caracteres_de_control_se_filtran() {
        let mut s = TextAreaState::new();
        s.apply_key(&k_text("\t"));
        assert!(s.is_empty());
    }

    #[test]
    fn set_text_roundtrip() {
        let mut s = TextAreaState::new();
        s.set_text("hola\nmundo");
        assert_eq!(s.text(), "hola\nmundo");
        s.clear();
        assert!(s.is_empty());
    }
}
