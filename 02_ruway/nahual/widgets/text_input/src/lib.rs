//! `nahual_widget_text_input` — input de texto minimal.
//!
//! Diseñado para diálogos cortos (rename, prompts). NO es un editor — no
//! soporta:
//! - cursor positioning con flechas / mouse,
//! - selección con shift / arrastre,
//! - copy / cut / paste,
//! - IME / multilínea.
//!
//! Soporta lo justo:
//! - escribir caracteres (cualquier `key_char` printable los appendea al final),
//! - `Backspace` quita el último char,
//! - `Enter` emite [`TextInputEvent::Confirmed`] con el texto actual,
//! - `Escape` emite [`TextInputEvent::Cancelled`].
//!
//! Cuando montes el widget, llamá `request_focus(window)` para que reciba
//! teclas de inmediato. El padre se subscribe vía `cx.subscribe(&input,
//! …)` para recibir Confirmed/Cancelled.
//!
//! Cuando necesitemos algo serio (selección, posiciones, IME), portamos el
//! ejemplo `gpui::examples::input` o adoptamos `gpui-input` cuando exista.

use std::time::Duration;

use gpui::{
    div, prelude::*, px, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
    Render, SharedString, Task, Window,
};

use nahual_theme::Theme;

/// Período de toggle del caret. 500ms es el estándar de los inputs
/// del SO; ni rápido demasiado (distrae) ni lento (parece muerto).
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub enum TextInputEvent {
    /// El usuario apretó Enter. El payload es el texto actual.
    Confirmed(String),
    /// El usuario apretó Escape. El padre suele cerrar el modal.
    Cancelled,
}

pub struct TextInput {
    text: String,
    focus_handle: FocusHandle,
    /// Placeholder visible cuando `text` está vacío.
    placeholder: SharedString,
    /// Si `true`, el contenido se dibuja como puntos (`•`) en vez del
    /// texto real — para campos de contraseña.
    mask: bool,
    /// Toggle del caret: alterna cada [`CARET_BLINK_INTERVAL`]
    /// entre `true` (visible) y `false` (oculto). El render lo
    /// considera junto con focus para decidir si dibujar `|`.
    caret_visible: bool,
    /// Task del loop de blink. Se mantiene en self para que el
    /// drop del widget cancele el loop (sino seguiría tickeando
    /// y notificando contra un Entity ya muerto).
    _blink_task: Task<()>,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(initial: impl Into<String>, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        // Loop de blink: alterna `caret_visible` y notifica para
        // re-render. Vive en `_blink_task` (drop = cancel).
        let blink_task = cx.spawn(async move |this, cx| {
            let timer = cx.background_executor().clone();
            loop {
                timer.timer(CARET_BLINK_INTERVAL).await;
                let updated = this
                    .update(cx, |me, cx| {
                        me.caret_visible = !me.caret_visible;
                        cx.notify();
                    })
                    .is_ok();
                if !updated {
                    // Entity drop → salimos del loop.
                    break;
                }
            }
        });
        Self {
            text: initial.into(),
            focus_handle: cx.focus_handle(),
            placeholder: SharedString::from(""),
            mask: false,
            caret_visible: true,
            _blink_task: blink_task,
        }
    }

    /// Setea el placeholder mostrado cuando el campo está vacío.
    #[allow(dead_code)]
    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Dibuja el contenido como puntos (`•`) — para campos de
    /// contraseña. El texto real sigue accesible vía [`Self::text`].
    pub fn with_mask(mut self) -> Self {
        self.mask = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Reemplaza el contenido completo (e.g. al abrir un modal pre-cargado).
    pub fn set_text(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.text = text.into();
        cx.notify();
    }

    /// Pide focus para que las próximas teclas vayan al input. Llamar
    /// cuando montás el widget en un modal para que esté "activo".
    pub fn request_focus(&self, window: &mut Window) {
        window.focus(&self.focus_handle);
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, _w: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                cx.emit(TextInputEvent::Confirmed(self.text.clone()));
                return;
            }
            "escape" => {
                cx.emit(TextInputEvent::Cancelled);
                return;
            }
            "backspace" => {
                self.text.pop();
                cx.notify();
                return;
            }
            _ => {}
        }
        // Char "imprimible": tomamos `key_char` (que respeta el layout +
        // modificadores) si está presente. `key_char` es el que el sistema
        // dice "esto es lo que el usuario realmente escribió".
        if let Some(ch) = event.keystroke.key_char.as_deref() {
            // Solo apendeamos si NO contiene control chars (newline,
            // backspace, etc — que llegarían como key_char en algunas
            // plataformas).
            if !ch.chars().any(|c| c.is_control()) {
                self.text.push_str(ch);
                cx.notify();
            }
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::global(cx).clone();
        let is_empty = self.text.is_empty();
        // Border-color depende del focus: focused → accent (señal
        // clara de "vas a tipear acá"); blur → border (silencioso).
        // Sin esto era imposible saber qué input estaba activo en
        // un form con varios fields.
        let is_focused = self.focus_handle.is_focused(w);
        let border_color = if is_focused {
            theme.accent_strong
        } else {
            theme.border
        };
        // Caret visible cuando: (1) input tiene focus AND (2) el
        // toggle del blink loop está en `true`. El loop alterna
        // cada 500ms — feel familiar a los inputs del SO.
        let show_caret = is_focused && self.caret_visible;
        let shown = display_text(&self.text, self.mask);
        let display: SharedString = if is_empty {
            self.placeholder.clone()
        } else if show_caret {
            SharedString::from(format!("{shown}|"))
        } else {
            SharedString::from(shown)
        };
        let text_color = if is_empty {
            theme.fg_disabled
        } else {
            theme.fg_text
        };

        div()
            .id("nahual-text-input")
            .track_focus(&self.focus_handle)
            .key_context("YahwehTextInput")
            .on_key_down(cx.listener(Self::handle_key_down))
            .px(px(10.0))
            .py(px(6.0))
            .min_w(px(200.0))
            .bg(theme.bg_panel)
            .border_1()
            .border_color(border_color)
            .rounded(px(4.0))
            .text_size(px(13.0))
            .text_color(text_color)
            .child(display)
    }
}

/// Texto a mostrar: el contenido tal cual, o un punto (`•`) por cada
/// carácter si el campo está enmascarado.
fn display_text(text: &str, mask: bool) -> String {
    if mask {
        "•".repeat(text.chars().count())
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::display_text;

    #[test]
    fn plain_text_shown_verbatim() {
        assert_eq!(display_text("hola", false), "hola");
    }

    #[test]
    fn masked_text_is_dots_one_per_char() {
        assert_eq!(display_text("hola", true), "••••");
        // Un punto por carácter Unicode, no por byte.
        assert_eq!(display_text("ñoño", true), "••••");
        assert_eq!(display_text("", true), "");
    }
}
