//! Medidor de RAM. Lee `/proc/meminfo` y muestra `usada/total` como
//! porcentaje + barrita compacta.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct RamMeter {
    /// Porcentaje 0..=100 — actualizado en `tick`.
    used_pct: f32,
    /// Texto cacheado: `RAM 42%`.
    text: String,
}

impl RamMeter {
    pub fn from_spec(_spec: &WidgetSpec) -> Self {
        let mut me = Self { used_pct: 0.0, text: "RAM —".into() };
        me.tick();
        me
    }
}

impl Widget for RamMeter {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn tick(&mut self) {
        if let Some((total, available)) = read_meminfo() {
            let used = total.saturating_sub(available);
            let pct = if total > 0 { used as f32 * 100.0 / total as f32 } else { 0.0 };
            self.used_pct = pct.clamp(0.0, 100.0);
            self.text = format!("RAM {:.0}%", self.used_pct);
        }
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        let bar = View::new(Style {
            size: Size { width: length(48.0_f32), height: length(6.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_panel)
        .radius(2.0)
        .children(vec![View::new(Style {
            size: Size {
                width: length(48.0_f32 * (self.used_pct / 100.0)),
                height: length(6.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.accent)
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

/// Devuelve `(total_kb, available_kb)` desde `/proc/meminfo`. `None` si el
/// fs no existe (Mac/Win) o si las claves faltan.
fn read_meminfo() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut total = None;
    let mut avail = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "MemTotal:" => total = parts.next()?.parse::<u64>().ok(),
            "MemAvailable:" => avail = parts.next()?.parse::<u64>().ok(),
            _ => {}
        }
        if total.is_some() && avail.is_some() {
            break;
        }
    }
    Some((total?, avail?))
}
