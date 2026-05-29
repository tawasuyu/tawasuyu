//! Widget reloj. Por default `HH:MM`; `format` admite el subset clásico de
//! strftime (chrono). La zona horaria viene de la config global:
//!
//! - `timezone = "auto"` (default): `chrono::Local` — toma `TZ` env o
//!   `/etc/localtime` del sistema.
//! - `timezone = "UTC"`: explícitamente UTC.
//! - Otros valores IANA (`America/Lima`): defer hasta que enchufemos
//!   chrono-tz; por ahora caen a `auto`.

use chrono::{Local, Utc};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

/// Mode efectivo de zona horaria — resuelto al construir el widget desde
/// la config global.
#[derive(Debug, Clone, Copy)]
pub enum TzMode {
    /// Hora del sistema (`/etc/localtime`).
    Auto,
    /// UTC explícito.
    Utc,
}

impl TzMode {
    pub fn from_config(name: &str) -> Self {
        match name {
            "UTC" | "utc" => TzMode::Utc,
            // "auto" o cualquier IANA todavía sin soporte → auto del sistema.
            _ => TzMode::Auto,
        }
    }
}

pub struct Clock {
    format: String,
    tz: TzMode,
    text: String,
}

impl Clock {
    pub fn from_spec(spec: &WidgetSpec, tz: TzMode) -> Self {
        let format = spec.str_prop("format", "%H:%M").to_string();
        let mut me = Self { format, tz, text: String::new() };
        me.tick();
        me
    }
}

impl Widget for Clock {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tick(&mut self) {
        self.text = match self.tz {
            TzMode::Auto => Local::now().format(&self.format).to_string(),
            TzMode::Utc => Utc::now().format(&self.format).to_string(),
        };
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        View::new(Style {
            size: Size { width: auto(), height: length(22.0_f32) },
            padding: Rect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(self.text.clone(), 13.0, theme.fg_text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn tz_mode_parses_utc_variants() {
        assert!(matches!(TzMode::from_config("UTC"), TzMode::Utc));
        assert!(matches!(TzMode::from_config("utc"), TzMode::Utc));
        assert!(matches!(TzMode::from_config("auto"), TzMode::Auto));
        assert!(matches!(TzMode::from_config("America/Lima"), TzMode::Auto));
    }

    #[test]
    fn utc_clock_renders_current_year_in_text() {
        let spec = WidgetSpec {
            kind: "clock".into(),
            props: std::collections::HashMap::new(),
        };
        let mut clock = Clock::from_spec(&spec, TzMode::Utc);
        // Forzamos format con %Y para verificar que chrono entra.
        clock.format = "%Y".into();
        clock.tick();
        let year = Utc::now().year();
        assert_eq!(clock.text, year.to_string());
    }
}
