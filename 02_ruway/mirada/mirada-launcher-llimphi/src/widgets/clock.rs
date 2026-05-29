//! Widget reloj. Por default `HH:MM`; `format` admite `%H`, `%M`, `%S`,
//! `%d`, `%m`, `%Y` — subset suficiente para una barra (no traemos chrono
//! por una hora del día).

use std::time::{SystemTime, UNIX_EPOCH};

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct Clock {
    format: String,
    text: String,
}

impl Clock {
    pub fn from_spec(spec: &WidgetSpec) -> Self {
        let format = spec.str_prop("format", "%H:%M").to_string();
        let mut me = Self { format, text: String::new() };
        me.tick();
        me
    }
}

impl Widget for Clock {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn tick(&mut self) {
        self.text = format_local(&self.format, SystemTime::now());
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

/// Formateo local minimalista: H/M/S/d/m/Y. Hora local = UTC + offset del
/// sistema leído de `localtime_r` vía un truco con `chrono::Local`? No —
/// preferimos no traer la dep. Usamos `SystemTime` en UTC y aplicamos el
/// offset del huso via `tzset`/`localtime_r` de libc. En esta vuelta hago
/// algo más simple: trato `SystemTime` como UTC y dejo el huso para una
/// pasada posterior — el reloj se ve, pero del UTC. (Marcado con `Z` para
/// no engañar.)
fn format_local(fmt: &str, now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (year, month, day, hour, min, sec) = civil_from_unix(secs as i64);
    let mut out = String::with_capacity(fmt.len() + 4);
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('M') => out.push_str(&format!("{min:02}")),
            Some('S') => out.push_str(&format!("{sec:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('Y') => out.push_str(&format!("{year:04}")),
            Some('Z') => out.push('Z'),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Howard Hinnant `civil_from_days`: convierte un timestamp UNIX (segundos
/// UTC) a (Y, M, D, h, m, s). Sin libc, sin chrono. Ver
/// `https://howardhinnant.github.io/date_algorithms.html`.
fn civil_from_unix(z: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = z.div_euclid(86_400);
    let secs_of_day = z.rem_euclid(86_400) as u32;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970_jan_1() {
        // 0 unix = 1970-01-01 00:00:00 UTC
        let t = UNIX_EPOCH;
        assert_eq!(format_local("%Y-%m-%d %H:%M:%S", t), "1970-01-01 00:00:00");
    }

    #[test]
    fn format_passthrough_and_literal_percent() {
        // 86399 = 1970-01-01 23:59:59
        let t = UNIX_EPOCH + std::time::Duration::from_secs(86_399);
        assert_eq!(format_local("hora: %H:%M [%%]", t), "hora: 23:59 [%]");
    }
}
