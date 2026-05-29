//! Preview del último contenido del portapapeles. MVP: poll vía
//! `wl-paste` (Wayland) o `xclip -selection clipboard -o` (X11). El
//! historial real (los últimos N items) queda para una iteración futura
//! con un thread que escuche `wl-paste --watch`.

use std::process::Command;
use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct Clipboard {
    /// Cuántos chars de la preview mostrar; default 24.
    max_preview: usize,
    last: Option<String>,
    text: String,
}

impl Clipboard {
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let max_preview = spec.float_prop("max_preview", 24.0) as usize;
        let mut me = Self {
            max_preview: max_preview.max(4),
            last: None,
            text: "CLIP —".into(),
        };
        me.tick();
        me
    }
}

impl Widget for Clipboard {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tick(&mut self) {
        let Some(raw) = read_clipboard() else { return };
        let clean: String = raw.chars().filter(|c| !c.is_control()).collect();
        let preview = if clean.chars().count() > self.max_preview {
            let mut s: String = clean.chars().take(self.max_preview).collect();
            s.push('…');
            s
        } else if clean.is_empty() {
            "(vacío)".into()
        } else {
            clean.clone()
        };
        self.last = Some(clean);
        self.text = format!("CLIP {}", preview);
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        View::new(Style {
            size: Size { width: auto(), height: length(22.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(self.text.clone(), 12.0, theme.fg_muted)
    }
}

/// Probamos Wayland primero, X11 después. Cualquier error → None y
/// dejamos el texto previo intacto.
fn read_clipboard() -> Option<String> {
    if let Some(s) = run_with_timeout("wl-paste", &["--no-newline"]) {
        return Some(s);
    }
    if let Some(s) = run_with_timeout("xclip", &["-selection", "clipboard", "-o"]) {
        return Some(s);
    }
    None
}

/// Ejecuta el comando y devuelve su stdout como `String` si terminó en
/// < 200 ms con código 0. Si tarda más, lo mata (no queremos que el
/// poll del tick se cuelgue por un clipboard que ofrece datos lentos).
fn run_with_timeout(bin: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let deadline = std::time::Instant::now() + Duration::from_millis(200);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut out = Vec::new();
                if let Some(mut s) = child.stdout.take() {
                    use std::io::Read;
                    let _ = s.read_to_end(&mut out);
                }
                return String::from_utf8(out).ok();
            }
            Ok(Some(_)) => return None,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}
