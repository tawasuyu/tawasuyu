//! Input quake — input elegante toggleable.
//!
//! En modo "barra": ocupa poco, dice `›` y un placeholder. Al togglear se
//! expande dentro de la barra (defer: levantar overlay full-screen tipo
//! Quake/Spotlight). Escribir va al estado interno; Enter "submitea" — por
//! ahora dispara el comando como `sh -c`. Más adelante: target=auto
//! (terminal/app/ia/ssh).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
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
    /// `true` mientras espera una respuesta IA.
    pub pending: bool,
    /// Último resultado IA / error / status de comando shell, para
    /// renderear debajo del input.
    pub result: Option<Result<String, String>>,
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
            pending: false,
            result: None,
        }
    }

    /// Clasifica el buffer al submit: `!cmd` o `$cmd` van a shell; todo
    /// lo demás es prompt para IA. La detección la hace el app loop, que
    /// es quien tiene acceso al `Handle` para hacer `spawn`.
    pub fn classify(buffer: &str) -> SubmitKind<'_> {
        let trimmed = buffer.trim();
        if let Some(rest) = trimmed.strip_prefix('!').or_else(|| trimmed.strip_prefix('$')) {
            SubmitKind::Shell(rest.trim())
        } else if trimmed.is_empty() {
            SubmitKind::Empty
        } else {
            SubmitKind::Ia(trimmed)
        }
    }

    /// Vista del overlay full-screen cuando `open` — scrim semi-transparente
    /// con la card del input centrada. La app lo enchufa desde
    /// `view_overlay`; cuando `open == false` devuelve `None` y el runtime
    /// no pinta nada.
    pub fn overlay(&self, theme: &Theme) -> Option<View<Msg>> {
        if !self.open {
            return None;
        }

        let (content, content_color) = if self.buffer.is_empty() {
            (self.placeholder.clone(), theme.fg_placeholder)
        } else {
            (format!("› {}", self.buffer), theme.fg_text)
        };

        // Filas del card: input + (pending|result|hint).
        let mut rows: Vec<View<Msg>> = vec![View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(32.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .text(content, 22.0, content_color)];

        if self.pending {
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .text("…pensando", 13.0, theme.fg_muted),
            );
        } else if let Some(res) = &self.result {
            let (text, color) = match res {
                Ok(s) => (s.clone(), theme.fg_text),
                Err(e) => (e.clone(), theme.fg_destructive),
            };
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .text(text, 13.0, color),
            );
        } else {
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(16.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .text(
                    "Enter — IA · ! prefix — shell · Esc — cerrar",
                    11.0,
                    theme.fg_placeholder,
                ),
            );
        }

        // Card adaptativa: más alta si hay resultado para mostrar.
        let card_height = if self.result.is_some() || self.pending { 132.0_f32 } else { 96.0_f32 };

        let card = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: length(self.width_open.max(420.0_f32)),
                height: length(card_height),
            },
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(20.0_f32),
                bottom: length(20.0_f32),
            },
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(14.0)
        .children(rows);

        let scrim = View::new(Style {
            position: Position::Absolute,
            inset: Rect {
                left: length(0.0_f32),
                top: length(0.0_f32),
                right: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(Color::from_rgba8(0, 0, 0, 160))
        .on_click(Msg::QuakeToggle)
        .children(vec![card]);

        Some(scrim)
    }

    /// Llamado por la app para mutar al recibir mensajes del input. El
    /// submit en sí lo decide el caller (necesita Handle); acá sólo
    /// reflejamos los efectos visibles.
    pub fn apply(&mut self, msg: &Msg) {
        match msg {
            Msg::QuakeToggle => {
                self.open = !self.open;
                if !self.open {
                    self.buffer.clear();
                    self.result = None;
                    self.pending = false;
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
                // El routing real lo hace el app; acá limpiamos el buffer.
                // Si fue prompt IA, `pending` lo seteamos desde fuera.
            }
            Msg::QuakeIaResult(r) => {
                self.pending = false;
                self.result = Some(r.clone());
            }
            _ => {}
        }
    }

    /// Setear el resultado de un comando shell directamente (sin pasar
    /// por IA). El app lo llama tras spawn fire-and-forget.
    pub fn set_shell_result(&mut self, exec: &str, status: std::io::Result<()>) {
        let msg = match status {
            Ok(()) => Ok(format!("lanzado: {exec}")),
            Err(e) => Err(format!("{e}")),
        };
        self.result = Some(msg);
        self.pending = false;
    }

    /// El caller llama esto cuando arranca una request a IA: limpia el
    /// resultado previo y marca pending.
    pub fn mark_pending(&mut self) {
        self.result = None;
        self.pending = true;
    }
}

/// Categoría detectada en el buffer al hacer submit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitKind<'a> {
    Empty,
    Shell(&'a str),
    Ia(&'a str),
}

/// Llama a la IA bloqueando — pensado para correr en un thread spawned
/// vía `Handle::spawn`. `from_env` autodetecta el backend (cae a Mock
/// si no hay credenciales — en ese caso, la respuesta es un eco
/// determinista, útil para iterar UI sin red).
pub fn ask_ia_blocking(prompt: &str) -> Result<String, String> {
    use pluma_llm::pluma_llm_core::ChatRequest;
    let cli = pluma_llm::from_env().map_err(|e| format!("{e}"))?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("{e}"))?;
    rt.block_on(async {
        let req = ChatRequest::una_vuelta(prompt, 256)
            .con_sistema("Sos un asistente conciso del escritorio. Responde corto.");
        cli.complete(&req)
            .await
            .map(|r| r.content)
            .map_err(|e| format!("{e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty() {
        assert_eq!(QuakeInput::classify(""), SubmitKind::Empty);
        assert_eq!(QuakeInput::classify("   "), SubmitKind::Empty);
    }

    #[test]
    fn classify_shell_with_bang() {
        assert_eq!(QuakeInput::classify("!firefox"), SubmitKind::Shell("firefox"));
        assert_eq!(QuakeInput::classify("$ ls -la"), SubmitKind::Shell("ls -la"));
    }

    #[test]
    fn classify_default_is_ia() {
        assert_eq!(QuakeInput::classify("hola"), SubmitKind::Ia("hola"));
        assert_eq!(
            QuakeInput::classify("  qué hora es?  "),
            SubmitKind::Ia("qué hora es?"),
        );
    }
}

impl Widget for QuakeInput {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn try_key(&self, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if !self.hotkey.is_empty() && keys::matches(&self.hotkey, event) {
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
