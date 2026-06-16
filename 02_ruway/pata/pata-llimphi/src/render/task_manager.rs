//! Task manager, workspace switcher, tray, portapapeles y botón de inicio:
//! todos los widgets de la barra que muestran estado dinámico del host.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style},
    Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::{
    Blob, ImageAlphaType, ImageBrush as Image, ImageData, ImageFormat,
};
use llimphi_ui::View;

use crate::tray::{TrayIcon, TrayItem};
use crate::{Msg};

use super::widgets::chip;

/// Largo máximo de la etiqueta de una ventana antes de recortar con `…`.
const WINDOW_LABEL_MAX: usize = 22;

/// Largo máximo de la etiqueta de un item del tray antes de recortar con `…`.
const TRAY_LABEL_MAX: usize = 14;

/// Largo máximo del preview del portapapeles antes de recortar con `…`.
const CLIPBOARD_PREVIEW_MAX: usize = 28;

/// Lado del ícono-badge (cuadrado) de una ventana en el task manager, en px.
const WIN_BADGE_PX: f32 = 18.0;
/// Ancho fijo de cada botón de tarea (taskbar): todos miden lo mismo.
const TASK_W: f32 = 170.0;

/// Tamaño del ícono del tray en la barra (px).
const TRAY_ICON_PX: f32 = 18.0;

/// Ancho del popup del historial de portapapeles (px).
const CLIP_MENU_W: f32 = 360.0;
/// Largo máximo de cada fila del historial antes de recortar.
const CLIP_ROW_MAX: usize = 48;

use crate::toplevel::WindowEntry;

/// El **task manager** (estilo KDE): un botón por ventana abierta.
pub(super) fn window_list_view(
    windows: &[WindowEntry],
    gap: f32,
    dir: FlexDirection,
    theme: &Theme,
) -> View<Msg> {
    let chips: Vec<View<Msg>> = windows.iter().map(|w| window_button(w, theme)).collect();

    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        // Alineados al inicio (a la izquierda en barra horizontal), estilo
        // taskbar — no centrados. El slot que lo hospeda ya le da el ancho.
        justify_content: Some(JustifyContent::FlexStart),
        flex_grow: 1.0,
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(chips)
}

/// Un botón de ventana del task manager: badge (inicial) + título recortado.
fn window_button(w: &WindowEntry, theme: &Theme) -> View<Msg> {
    let (fg, fill, badge_bg, badge_fg) = if w.active {
        (theme.fg_text, theme.bg_panel, theme.accent, theme.bg_panel)
    } else if w.minimized {
        (theme.fg_muted, theme.bg_panel_alt, theme.bg_panel, theme.fg_muted)
    } else {
        (theme.fg_text, theme.bg_panel_alt, theme.bg_panel, theme.fg_muted)
    };

    let badge = View::new(Style {
        size: Size {
            width: length(WIN_BADGE_PX),
            height: length(WIN_BADGE_PX),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(badge_bg)
    .radius(4.0)
    .text(w.inicial(), 11.0, badge_fg);

    let titulo = View::new(Style {
        // Crece para llenar el ancho fijo del botón; el texto va a la izquierda
        // (estilo botón de tarea: ícono + título alineado, no centrado).
        flex_grow: 1.0,
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        ..Default::default()
    })
    .text(super::recortar(&w.label, WINDOW_LABEL_MAX), 12.0, fg);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        // Ancho FIJO: todos los botones de tarea miden lo mismo (estilo taskbar),
        // no se encogen/crecen según el largo del título.
        flex_shrink: 0.0,
        size: Size {
            width: length(TASK_W),
            height: length(26.0_f32),
        },
        padding: TaffyRect {
            left: length(6.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(6.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(fill)
    .radius(4.0)
    .border(1.0, theme.border)
    .hover_fill(theme.bg_button_hover)
    .tooltip(w.label.clone())
    .on_click(Msg::ActivateWindow(w.id))
    .on_right_click(Msg::CloseWindow(w.id))
    .children(vec![badge, titulo])
}

/// El **workspace switcher**: una celda por escritorio virtual.
pub(super) fn workspaces_view(
    active: u8,
    count: u8,
    occupied: u16,
    gap: f32,
    dir: FlexDirection,
    theme: &Theme,
) -> View<Msg> {
    let celdas: Vec<View<Msg>> = (1..=count)
        .map(|n| {
            let ocupado = occupied & (1u16 << (n as u16 - 1)) != 0;
            workspace_cell(n, n == active, ocupado, theme)
        })
        .collect();
    let g = gap.max(2.0);
    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(g),
            height: length(g),
        },
        ..Default::default()
    })
    .children(celdas)
}

/// Una celda del workspace switcher: cuadradito con número y estado visual.
fn workspace_cell(n: u8, active: bool, occupied: bool, theme: &Theme) -> View<Msg> {
    let (fill, fg) = if active {
        (theme.accent, theme.bg_panel)
    } else if occupied {
        (theme.bg_panel, theme.fg_text)
    } else {
        (theme.bg_panel_alt, theme.fg_muted)
    };
    View::new(Style {
        // Más grande para acertar el click cómodo (antes 22×20 = chico).
        size: Size {
            width: length(30.0_f32),
            height: length(26.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(fill)
    .radius(5.0)
    .border(1.0, theme.border)
    .hover_fill(theme.bg_button_hover)
    .tooltip(format!("Escritorio {n}"))
    .on_click(Msg::SwitchWorkspace(n))
    .text(n.to_string(), 13.0, fg)
}

/// El **botón de inicio**: chip con su label/ícono. Clic → menú de apps.
pub(super) fn start_button_view(label: &str, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let click = match exec {
        Some(cmd) => Msg::Spawn(cmd.to_string()),
        None => Msg::StartToggle,
    };
    // Botón de inicio cómodo de clickear (antes usaba el `chip` de 22px = chico).
    View::new(Style {
        size: Size {
            width: auto(),
            height: length(32.0_f32),
        },
        padding: TaffyRect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(6.0)
    .border(1.0, theme.border)
    .hover_fill(theme.bg_button_hover)
    .tooltip(if exec.is_some() {
        "Lanzar"
    } else {
        "Menú de inicio (clic-der: cambiar estilo)"
    })
    .on_click(click)
    .on_right_click(Msg::StartStyleCycle)
    .text(label.to_string(), 14.0, theme.accent)
}

/// El `tray`: un chip clickeable por item de la bandeja del sistema.
pub(super) fn tray_view(items: &[TrayItem], gap: f32, dir: FlexDirection, theme: &Theme) -> View<Msg> {
    let chips: Vec<View<Msg>> = items
        .iter()
        .map(|it| {
            let tip = if it.label.trim().is_empty() {
                it.key.clone()
            } else {
                it.label.clone()
            };
            let base = chip(theme)
                .fill(theme.bg_panel_alt)
                .radius(6.0)
                .hover_fill(theme.bg_button_hover)
                .tooltip(tip)
                .on_click(Msg::TrayActivate(it.key.clone()));
            match &it.icon {
                Some(icon) => base.children(vec![tray_icon_node(icon)]),
                None => {
                    let fg = if it.status == "NeedsAttention" {
                        theme.accent
                    } else {
                        theme.fg_text
                    };
                    base.text(super::recortar(&it.label, TRAY_LABEL_MAX), 12.0, fg)
                }
            }
        })
        .collect();

    View::new(Style {
        flex_direction: dir,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(gap),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(chips)
}

/// Un nodo cuadrado con el ícono del item del tray.
fn tray_icon_node(icon: &TrayIcon) -> View<Msg> {
    let blob = Blob::from(icon.rgba.clone());
    let img = Image::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: icon.width, height: icon.height });
    View::new(Style {
        size: Size {
            width: length(TRAY_ICON_PX),
            height: length(TRAY_ICON_PX),
        },
        ..Default::default()
    })
    .image(img)
}

/// El widget `clipboard`: chip con preview del texto copiado.
pub(super) fn clipboard_view(text: Option<&str>, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let (etiqueta, fg) = match text {
        Some(t) if !t.is_empty() => (format!("📋 {}", super::recortar(t, CLIPBOARD_PREVIEW_MAX)), theme.fg_text),
        _ => ("📋".to_string(), theme.fg_muted),
    };
    let v = chip(theme)
        .hover_fill(theme.bg_button_hover)
        .radius(6.0)
        .text(etiqueta, 12.0, fg);
    let v = match text {
        Some(t) if !t.is_empty() => v.tooltip(t.to_string()),
        _ => v,
    };
    let v = v.on_click(Msg::ClipboardMenu);
    match exec {
        Some(cmd) => v.on_right_click(Msg::Spawn(cmd.to_string())),
        None => v,
    }
}

/// El **panel** del historial de portapapeles: cabecera + una fila por copia.
pub fn clipboard_panel(history: &[String], theme: &Theme) -> View<Msg> {
    let mut hijos: Vec<View<Msg>> = Vec::new();
    hijos.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexStart),
            ..Default::default()
        })
        .text("Portapapeles", 12.0, theme.fg_muted),
    );
    if history.is_empty() {
        hijos.push(
            View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text("(sin copias todavía)", 12.0, theme.fg_muted),
        );
    } else {
        for entry in history {
            let fila = View::new(Style {
                size: Size {
                    width: percent(1.0_f32),
                    height: length(26.0_f32),
                },
                padding: TaffyRect {
                    left: length(8.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::FlexStart),
                ..Default::default()
            })
            .radius(6.0)
            .hover_fill(theme.bg_button_hover)
            .tooltip(entry.clone())
            .on_click(Msg::ClipboardPick(entry.clone()))
            .text(super::recortar(entry, CLIP_ROW_MAX), 12.0, theme.fg_text);
            hijos.push(fila);
        }
    }

    View::new(Style {
        position: llimphi_ui::llimphi_layout::taffy::prelude::Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: auto(),
            bottom: auto(),
        },
        size: Size {
            width: length(CLIP_MENU_W),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(2.0_f32),
        },
        ..Default::default()
    })
    .children(hijos)])
}

/// El historial de portapapeles como **overlay** para winit.
pub fn clipboard_overlay(history: &[String], bar_h: f32, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::prelude::Position;
    let scrim = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(0.0_f32),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .on_click(Msg::ClipboardMenu)
    .children(vec![clipboard_panel(history, theme)]);
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(bar_h),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        ..Default::default()
    })
    .children(vec![scrim])
}
