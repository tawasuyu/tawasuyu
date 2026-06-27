//! Render del diente **«Unidades»** (sandokan): estado + telemetría de las
//! unidades vivas del plano de control, en una lista read-only. Los datos llegan
//! por [`crate::unidades`] (arje-bus). Sin plano de control → aviso.

use llimphi_theme::{Color, Theme};
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::View;

use sandokan_lifecycle::LifecycleState;
use sandokan_monitor_core::{MonitorSnapshot, UnitObservation};

use super::panels::panel_box_flow;
use crate::Msg;

/// El panel de unidades, de alto completo: resumen (activas/total) + una fila por
/// unidad (punto de estado + etiqueta + telemetría). Aviso si no hay snapshot.
pub fn unidades_view(snap: Option<&MonitorSnapshot>, panel_h: f32, theme: &Theme) -> View<Msg> {
    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("Unidades".to_string(), 14.0, theme.fg_text);

    let mut hijos = vec![titulo];
    match snap {
        Some(s) if !s.is_empty() => {
            let resumen = View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(format!("{} activas · {} en total", s.running(), s.len()), 12.0, theme.fg_muted);
            hijos.push(resumen);

            let filas: Vec<View<Msg>> = s.units.iter().map(|u| fila(u, theme)).collect();
            hijos.push(panel_box_flow(filas, theme));
        }
        _ => {
            hijos.push(aviso(theme));
        }
    }

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
    .children(hijos)
}

fn rgba(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgba8(r, g, b, 255)
}

/// `(color del punto, rótulo)` del estado de ciclo de vida.
fn estado_visual(s: &LifecycleState) -> (Color, &'static str) {
    match s {
        LifecycleState::Running => (rgba(0x4A, 0xDE, 0x80), "activa"),
        LifecycleState::Pending => (rgba(0xFB, 0xBF, 0x24), "pendiente"),
        LifecycleState::Parked { .. } => (rgba(0xFB, 0xBF, 0x24), "en pausa"),
        LifecycleState::Exited { .. } => (rgba(0x94, 0xA3, 0xB8), "terminó"),
        LifecycleState::Failed { .. } => (rgba(0xF8, 0x71, 0x71), "falló"),
        LifecycleState::Killed => (rgba(0xF8, 0x71, 0x71), "matada"),
    }
}

/// Una fila: punto de estado + etiqueta a la izquierda, telemetría a la derecha.
fn fila(u: &UnitObservation, theme: &Theme) -> View<Msg> {
    let (col, rotulo) = estado_visual(&u.state);
    let punto = View::new(Style {
        size: Size { width: length(8.0_f32), height: length(8.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(col)
    .radius(4.0);
    let etiqueta = View::new(Style {
        size: Size { width: auto(), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text(u.label.clone(), 12.0, theme.fg_text);
    // Telemetría: mem en MiB + cpu% (o el rótulo del estado si no hay corriendo).
    let detalle = match &u.telemetry {
        Some(t) => format!("{} MiB · {:.0}%", t.mem_bytes / (1024 * 1024), t.cpu_pct),
        None => rotulo.to_string(),
    };
    let der = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    })
    .text(detalle, 11.0, theme.fg_muted);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![punto, etiqueta, der])
}

fn aviso(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(60.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text(
        "Sin plano de control. Arrancá arje (ENTE_BUS_SOCK) para ver las unidades.".to_string(),
        12.0,
        theme.fg_muted,
    )
}
