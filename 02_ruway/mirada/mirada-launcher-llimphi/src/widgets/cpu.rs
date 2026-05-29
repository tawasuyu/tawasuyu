//! Medidor de CPU. Toma delta entre dos lecturas de `/proc/stat` y reporta
//! `1 − idle_delta/total_delta` como porcentaje. La primera lectura sale 0%
//! por falta de referencia previa (esperado).

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct CpuMeter {
    /// Última lectura (total, idle) en jiffies.
    prev: Option<(u64, u64)>,
    used_pct: f32,
    text: String,
}

impl CpuMeter {
    pub fn from_spec(_spec: &WidgetSpec) -> Self {
        let mut me = Self { prev: None, used_pct: 0.0, text: "CPU —".into() };
        me.tick();
        me
    }
}

impl Widget for CpuMeter {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tick(&mut self) {
        let Some((total, idle)) = read_cpu_total() else { return };
        if let Some((p_total, p_idle)) = self.prev {
            let dt = total.saturating_sub(p_total) as f32;
            let di = idle.saturating_sub(p_idle) as f32;
            if dt > 0.0 {
                self.used_pct = ((dt - di) / dt * 100.0).clamp(0.0, 100.0);
                self.text = format!("CPU {:.0}%", self.used_pct);
            }
        }
        self.prev = Some((total, idle));
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

/// Primera línea de `/proc/stat` (`cpu user nice system idle iowait irq
/// softirq steal guest guest_nice`). `total` = suma de los 10; `idle` =
/// idle+iowait.
fn read_cpu_total() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let fields: Vec<u64> = parts.filter_map(|s| s.parse::<u64>().ok()).collect();
    if fields.len() < 4 {
        return None;
    }
    let total: u64 = fields.iter().sum();
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0); // idle + iowait
    Some((total, idle))
}
