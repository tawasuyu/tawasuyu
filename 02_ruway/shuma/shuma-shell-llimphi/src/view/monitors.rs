//! Monitor stack: las stat-cards de CPU/MEM + curvas históricas + extras
//! aportados por los módulos.

use super::super::*;
use super::widgets::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, Style};
use llimphi_ui::llimphi_layout::taffy::{FlexDirection, Rect, Size};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, PathEl, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{PaintRect, View};
use llimphi_theme::Theme;
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};

pub(crate) fn monitor_stack(model: &Model, theme: &Theme) -> View<Msg> {
    let palette = StatCardPalette::from_theme(theme);

    let (cpu_value, mem_value) = match model.last_snapshot {
        Some(s) if s.valid => (s.cpu_percent, s.mem_percent),
        _ => (0.0, 0.0),
    };

    let cpu_card = monitor_card(
        "CPU",
        format!("{cpu_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!(
                "{} de {} muestras",
                model.sysmon.cpu_history().len(),
                HISTORY
            ),
            _ => rimay_localize::t("shuma-empty-no-data-linux"),
        },
        Color::from_rgb8(0x82, 0xCF, 0xF2),
        model.sysmon.cpu_history().values(),
        &palette,
    );

    let mem_card = monitor_card(
        "MEM",
        format!("{mem_value:>3.0}%"),
        match model.last_snapshot {
            Some(s) if s.valid => format!("{} MB de {} MB", s.mem_used_mb, s.mem_total_mb),
            _ => rimay_localize::t("shuma-empty-no-data"),
        },
        Color::from_rgb8(0xF7, 0xC8, 0x7A),
        model.sysmon.mem_history().values(),
        &palette,
    );

    let mut children = vec![cpu_card, mem_card];

    // Stat-cards extra: una por cada `MonitorSpec` aportado por los módulos vivos.
    for (slot, contribs) in collect_contributions(model) {
        for spec in &contribs.monitors {
            let key = monitor_key(&slot, spec);
            let history = model.extra_history.get(&key).cloned().unwrap_or_default();
            let display = model
                .extra_display
                .get(&key)
                .cloned()
                .unwrap_or_else(|| "-".into());
            let accent = Color::from_rgb8(spec.accent.r, spec.accent.g, spec.accent.b);
            children.push(monitor_card(
                spec.label.as_str(),
                display,
                rimay_localize::t_args(
                    "shuma-stat-samples",
                    &[
                        ("have", history.len().to_string().into()),
                        ("total", HISTORY.to_string().into()),
                    ],
                ),
                accent,
                history,
                &palette,
            ));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

pub(crate) fn monitor_card(
    label: &str,
    value: String,
    description: String,
    accent: Color,
    history: Vec<f32>,
    palette: &StatCardPalette,
) -> View<Msg> {
    let card = stat_card_view::<Msg>(label, value, description.as_str(), accent, &[], palette);
    let curve = curve_view(history, accent);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .children(vec![card, curve])
}

pub(crate) fn curve_view(history: Vec<f32>, accent: Color) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(56.0_f32) },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect: PaintRect| {
        if history.len() < 2 {
            return;
        }
        let n = history.len() as f32;
        let dx = if n > 1.0 { rect.w / (n - 1.0) } else { rect.w };
        let mut path = BezPath::new();
        for (i, v) in history.iter().enumerate() {
            let x = rect.x + dx * i as f32;
            let y = rect.y + rect.h - (v.clamp(0.0, 100.0) / 100.0) * rect.h;
            let p = Point::new(x as f64, y as f64);
            if i == 0 {
                path.push(PathEl::MoveTo(p));
            } else {
                path.push(PathEl::LineTo(p));
            }
        }
        scene.stroke(&Stroke::new(1.5), Affine::IDENTITY, accent, None, &path);
    })
}
