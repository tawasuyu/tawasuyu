//! Brillo de pantalla. Sólo lectura: el set requiere root o regla udev
//! (la mayoría de distros lo tienen para `video` group, pero no asumimos).

use std::path::PathBuf;

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::widget::{Msg, Widget};

pub struct Brightness {
    /// Carpeta `/sys/class/backlight/<device>` elegida — cacheada porque
    /// no cambia entre arranques.
    device: Option<PathBuf>,
    /// 0..=100, o `None` si no hay backlight o no podemos leer.
    pct: Option<f32>,
    text: String,
}

impl Brightness {
    pub fn from_spec(_spec: &WidgetSpec) -> Self {
        let device = pick_backlight();
        let mut me = Self { device, pct: None, text: "BRI —".into() };
        me.tick();
        me
    }
}

impl Widget for Brightness {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn as_any(&self) -> &dyn std::any::Any { self }

    fn tick(&mut self) {
        let Some(dir) = &self.device else { return };
        let Some(cur) = read_u32(&dir.join("brightness")) else { return };
        let Some(max) = read_u32(&dir.join("max_brightness")) else { return };
        if max == 0 {
            return;
        }
        let pct = (cur as f32 * 100.0 / max as f32).clamp(0.0, 100.0);
        self.pct = Some(pct);
        self.text = format!("BRI {:.0}%", pct);
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        let pct = self.pct.unwrap_or(0.0);
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

/// Primer backlight encontrado en `/sys/class/backlight/`. `None` si no
/// hay ninguno (caso típico: torre con monitor externo sin DDC).
fn pick_backlight() -> Option<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir("/sys/class/backlight").ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    entries.into_iter().next()
}

fn read_u32(path: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()
}
