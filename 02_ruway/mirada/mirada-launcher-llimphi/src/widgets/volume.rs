//! Volumen del sink default. Hablamos con PulseAudio/PipeWire vía
//! `pactl` (presente como compat layer en cualquier sistema con
//! pipewire-pulse instalado). El `Command::new("pactl")` falla silencioso
//! si el binario no está — el widget queda con `VOL —`.
//!
//! Por qué shell-out y no libpulse: traer libpulse-binding al workspace
//! agrega ~50 deps y system requirements; este widget es de status, no
//! tiene que ser autoritativo en latencia. Si necesitamos cambio
//! de volumen en vivo desde el widget, refactor a libpulse después.

use std::process::Command;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct Volume {
    pct: Option<f32>,
    muted: bool,
    text: String,
}

impl Volume {
    pub fn from_spec(_spec: &WidgetSpec) -> Self {
        let mut me = Self { pct: None, muted: false, text: "VOL —".into() };
        me.tick();
        me
    }
}

impl Widget for Volume {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tick(&mut self) {
        let pct = pactl_volume_pct();
        let muted = pactl_muted();
        match (pct, muted) {
            (Some(p), Some(true)) => {
                self.pct = Some(p);
                self.muted = true;
                self.text = format!("VOL muted ({:.0}%)", p);
            }
            (Some(p), _) => {
                self.pct = Some(p);
                self.muted = false;
                self.text = format!("VOL {:.0}%", p);
            }
            _ => {} // sin cambio si pactl no está
        }
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        let pct = self.pct.unwrap_or(0.0);
        let fill = if self.muted { theme.fg_muted } else { theme.accent };
        let bar = View::new(Style {
            size: Size { width: length(36.0_f32), height: length(6.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(2.0)
        .children(vec![View::new(Style {
            size: Size {
                width: length(36.0_f32 * (pct / 100.0)),
                height: length(6.0_f32),
            },
            ..Default::default()
        })
        .fill(fill)
        .radius(2.0)]);

        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: auto(), height: length(22.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .children(vec![
            View::new(Style {
                size: Size { width: auto(), height: length(22.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text(self.text.clone(), 12.0, theme.fg_muted),
            bar,
        ])
    }
}

/// Salida típica de `pactl get-sink-volume @DEFAULT_SINK@`:
///
/// ```text
/// Volume: front-left: 30000 / 46% / -19.93 dB, front-right: 30000 / 46% / -19.93 dB
///         balance 0.00
/// ```
///
/// Tomamos el primer `\d+%` que aparezca como % del canal izquierdo —
/// alcanza para barra de status.
fn pactl_volume_pct() -> Option<f32> {
    let out = Command::new("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    parse_first_percent(&text)
}

/// `pactl get-sink-mute @DEFAULT_SINK@` → `Mute: yes` o `Mute: no`.
fn pactl_muted() -> Option<bool> {
    let out = Command::new("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let line = text.lines().next()?;
    Some(line.trim().ends_with("yes"))
}

/// Busca el primer entero seguido de `%` en el texto.
fn parse_first_percent(text: &str) -> Option<f32> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'%' {
                return text[start..i].parse::<f32>().ok();
            }
            continue;
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_first_percent() {
        let line = "Volume: front-left: 30000 / 46% / -19.93 dB";
        assert_eq!(parse_first_percent(line), Some(46.0));
    }

    #[test]
    fn no_percent_returns_none() {
        assert_eq!(parse_first_percent("balance 0.00"), None);
    }
}
