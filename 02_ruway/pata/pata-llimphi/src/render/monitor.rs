//! Render del **monitor de sistema** (diente `monitor`/`sistema` del sidebar):
//! CPU (promedio + por core) y RAM, reusando los panels del quick-settings
//! (`cpu_panel`/`ram_panel`). Es el primer paso del control center de sistema +
//! flota; a futuro suma las unidades de sandokan y la flota de matilda.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, AlignItems, FlexDirection, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::View;

use pata_core::widget::WidgetCtx;

use super::panels::{cpu_panel_body, panel_box_flow, ram_panel_body};
use crate::Msg;

/// El panel del monitor de sistema, de alto completo. Apila el panel de CPU
/// (promedio + cores) y el de RAM (barra + total/usado/libre), ambos reusados del
/// quick-settings de la barra.
pub fn sistema_monitor_view(ctx: &WidgetCtx, panel_h: f32, theme: &Theme) -> View<Msg> {
    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("Sistema".to_string(), 14.0, theme.fg_text);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(panel_h) },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![
        titulo,
        panel_box_flow(cpu_panel_body(ctx, theme), theme),
        panel_box_flow(ram_panel_body(ctx, theme), theme),
    ])
}
