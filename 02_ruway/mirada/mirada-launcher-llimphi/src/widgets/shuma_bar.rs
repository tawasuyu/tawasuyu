//! Barra del shell **shuma** — pensada para ocupar (casi) toda una barra
//! inferior y servir de launcher de comandos. Al submit, despliega el
//! "shuma desplegado" como overlay con el output del comando.
//!
//! Diferencia con `quake_input`: éste va por default a `sh -c` (no a
//! IA), y muestra **stdout completo** en un overlay scrollable, no una
//! línea de status. Es la barra del shell, no un asistente.

use std::process::Stdio;

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

/// La barra del shuma en cualquiera de sus dos estados:
/// - **colapsada** (default): se ve como un input grande dentro de la
///   barra inferior, con placeholder.
/// - **expandida** (overlay full): scrim + panel con input + scroll del
///   stdout del último comando.
pub struct ShumaBar {
    /// True cuando el overlay full está visible.
    pub open: bool,
    pub buffer: String,
    pub placeholder: String,
    /// Hotkey opcional (e.g. "F11" o "/"). Vacío = sólo se abre por click.
    pub hotkey: String,
    /// Output acumulado del último comando (o error formateado).
    pub output: Option<Result<String, String>>,
    pub pending: bool,
    /// Prompt visible al frente del input (estilo `›`, `$`, etc.).
    pub prompt: String,
}

impl ShumaBar {
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        Self {
            open: false,
            buffer: String::new(),
            placeholder: spec.str_prop("placeholder", "› shuma").to_string(),
            hotkey: spec.str_prop("hotkey", "").to_string(),
            prompt: spec.str_prop("prompt", "›").to_string(),
            output: None,
            pending: false,
        }
    }

    pub fn apply(&mut self, msg: &Msg) {
        match msg {
            Msg::ShumaToggle => {
                self.open = !self.open;
                if !self.open {
                    self.buffer.clear();
                    self.output = None;
                    self.pending = false;
                }
            }
            Msg::ShumaChar(c) => {
                self.buffer.push(*c);
            }
            Msg::ShumaBackspace => {
                self.buffer.pop();
            }
            Msg::ShumaSubmit => {
                // El routing real (spawn process) lo hace el app loop;
                // acá sólo marcamos pending.
            }
            Msg::ShumaResult(r) => {
                self.output = Some(r.clone());
                self.pending = false;
            }
            _ => {}
        }
    }

    pub fn mark_pending(&mut self) {
        self.pending = true;
        self.output = None;
    }

    /// Toma posesión del buffer al hacer submit. El app loop usa esto
    /// para spawn del proceso.
    pub fn take_buffer(&mut self) -> String {
        std::mem::take(&mut self.buffer)
    }

    /// Pinta la barra colapsada (la del fondo del panel).
    pub fn collapsed_view(&self, theme: &Theme) -> View<Msg> {
        let label = if self.buffer.is_empty() {
            self.placeholder.clone()
        } else {
            format!("{} {}", self.prompt, self.buffer)
        };
        let color = if self.buffer.is_empty() {
            theme.fg_placeholder
        } else {
            theme.fg_text
        };

        let mut style = Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        };
        style.flex_grow = 1.0;

        View::new(style)
            .fill(theme.bg_input)
            .radius(6.0)
            .text(label, 14.0, color)
            .on_click(Msg::ShumaToggle)
    }

    /// Overlay full cuando `open`. Panel grande arriba (centro de la
    /// ventana) con: header, input, output scroll (sin scroll virtual
    /// real — defer; por ahora se trunca).
    pub fn overlay(&self, theme: &Theme) -> Option<View<Msg>> {
        if !self.open {
            return None;
        }

        let buffer_label = if self.buffer.is_empty() {
            self.placeholder.clone()
        } else {
            format!("{} {}", self.prompt, self.buffer)
        };
        let buffer_color = if self.buffer.is_empty() {
            theme.fg_placeholder
        } else {
            theme.fg_text
        };

        // Filas del panel: header + input + output|pending|hint.
        let mut rows: Vec<View<Msg>> = Vec::new();

        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .text("shuma — shell del escritorio", 12.0, theme.fg_muted),
        );

        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .fill(theme.bg_input_focus)
            .radius(8.0)
            .text(buffer_label, 22.0, buffer_color),
        );

        if self.pending {
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(180.0_f32) },
                    padding: Rect {
                        left: length(12.0_f32),
                        right: length(12.0_f32),
                        top: length(10.0_f32),
                        bottom: length(10.0_f32),
                    },
                    align_items: Some(AlignItems::FlexStart),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .radius(6.0)
                .text("…ejecutando", 13.0, theme.fg_muted),
            );
        } else if let Some(out) = &self.output {
            let (text, color) = match out {
                Ok(s) => (preview(s, 4096), theme.fg_text),
                Err(e) => (e.clone(), theme.fg_destructive),
            };
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(260.0_f32) },
                    padding: Rect {
                        left: length(12.0_f32),
                        right: length(12.0_f32),
                        top: length(10.0_f32),
                        bottom: length(10.0_f32),
                    },
                    align_items: Some(AlignItems::FlexStart),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .fill(theme.bg_panel_alt)
                .radius(6.0)
                .text(text, 12.0, color),
            );
        } else {
            rows.push(
                View::new(Style {
                    size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
                    align_items: Some(AlignItems::Center),
                    justify_content: Some(JustifyContent::FlexStart),
                    ..Default::default()
                })
                .text("Enter — ejecutar · Esc — cerrar", 11.0, theme.fg_placeholder),
            );
        }

        let panel = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: length(900.0_f32), height: length(420.0_f32) },
            padding: Rect {
                left: length(20.0_f32),
                right: length(20.0_f32),
                top: length(20.0_f32),
                bottom: length(20.0_f32),
            },
            gap: Size { width: length(0.0_f32), height: length(12.0_f32) },
            justify_content: Some(JustifyContent::FlexStart),
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
        .fill(Color::from_rgba8(0, 0, 0, 180))
        .on_click(Msg::ShumaToggle)
        .children(vec![panel]);

        Some(scrim)
    }
}

impl Widget for ShumaBar {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn try_key(&self, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        if !self.hotkey.is_empty() && keys::matches(&self.hotkey, &event.key) {
            return Some(Msg::ShumaToggle);
        }
        None
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        self.collapsed_view(theme)
    }
}

/// Ejecuta `sh -c <cmd>` bloqueando, captura stdout+stderr y devuelve
/// el contenido. Pensado para correr en un thread spawned.
pub fn run_shell_blocking(cmd: &str) -> Result<String, String> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("no pude lanzar sh: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut combined = String::new();
    combined.push_str(&stdout);
    if !stderr.is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }
    if out.status.success() {
        Ok(combined)
    } else {
        Err(format!(
            "exit {}\n{}",
            out.status.code().unwrap_or(-1),
            combined.trim_end()
        ))
    }
}

/// Trunca a `max_bytes` chars (no bytes — para no romper UTF-8).
fn preview(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars).collect();
        out.push_str("\n…");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_shell_success_captures_stdout() {
        let r = run_shell_blocking("echo hola").unwrap();
        assert_eq!(r.trim(), "hola");
    }

    #[test]
    fn run_shell_failure_returns_err_with_status() {
        let r = run_shell_blocking("false");
        assert!(r.is_err());
        assert!(r.unwrap_err().starts_with("exit "));
    }

    #[test]
    fn preview_truncates_long_strings() {
        let long: String = "a".repeat(5000);
        let prev = preview(&long, 100);
        assert!(prev.chars().count() <= 110); // 100 + sufijo
        assert!(prev.ends_with('…'));
    }
}
