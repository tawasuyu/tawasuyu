//! Task manager, workspace switcher, tray, portapapeles y botón de inicio:
//! todos los widgets de la barra que muestran estado dinámico del host.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{
        auto, length, percent, AlignItems, FlexDirection, JustifyContent, Position, Size, Style,
    },
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
        // Gap CHICO entre botones de tarea (no el `surface.gap` de entre widgets
        // grandes, que se veía exagerado). Capado a 4 px.
        gap: Size {
            width: length(gap.min(4.0)),
            height: length(gap.min(4.0)),
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
pub fn workspaces_view(
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
    // Gap CHICO entre celdas del switcher (capado a 4 px): el `surface.gap` de
    // entre widgets grandes se veía exagerado para estos cuadraditos.
    let g = gap.clamp(2.0, 4.0);
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
/// Tres estados bien diferenciados:
/// - **activo**: relleno con el acento (el escritorio que ves).
/// - **ocupado** (tiene ventanas): borde de acento grueso + número fuerte +
///   un puntito de acento abajo. Se distingue de un vistazo de los vacíos.
/// - **vacío**: apagado, borde tenue, número atenuado.
fn workspace_cell(n: u8, active: bool, occupied: bool, theme: &Theme) -> View<Msg> {
    let (fill, fg, borde_w, borde_col) = if active {
        (theme.accent, theme.bg_panel, 1.0, theme.accent)
    } else if occupied {
        (theme.bg_panel, theme.fg_text, 2.0, theme.accent)
    } else {
        (theme.bg_panel_alt, theme.fg_muted, 1.0, theme.border)
    };
    // Puntito de ocupación: un cuadradito de acento centrado abajo. Visible sólo
    // en los ocupados que NO están activos (el activo ya se ve por el relleno).
    let punto = View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: auto(),
            right: auto(),
            top: auto(),
            bottom: length(2.0_f32),
        },
        size: Size { width: length(6.0_f32), height: length(3.0_f32) },
        ..Default::default()
    })
    .fill(if occupied && !active { theme.accent } else { theme.bg_panel_alt })
    .radius(1.5);
    let numero = View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(n.to_string(), 13.0, fg);
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
    .border(borde_w, borde_col)
    .hover_fill(theme.bg_button_hover)
    .tooltip(if occupied {
        format!("Escritorio {n} · con ventanas")
    } else {
        format!("Escritorio {n} · vacío")
    })
    .on_click(Msg::SwitchWorkspace(n))
    .children(vec![numero, punto])
}

/// El **botón de inicio**: chip con su label/ícono. Clic → menú de apps.
pub fn start_button_view(label: &str, exec: Option<&str>, theme: &Theme) -> View<Msg> {
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
pub fn tray_view(items: &[TrayItem], gap: f32, dir: FlexDirection, theme: &Theme) -> View<Msg> {
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

/// El widget `clipboard`: ícono (vector) + preview del texto copiado.
pub(super) fn clipboard_view(text: Option<&str>, exec: Option<&str>, theme: &Theme) -> View<Msg> {
    let (preview, fg) = match text {
        Some(t) if !t.is_empty() => (Some(super::recortar(t, CLIPBOARD_PREVIEW_MAX)), theme.fg_text),
        _ => (None, theme.fg_muted),
    };
    let mut kids: Vec<View<Msg>> = vec![super::widgets::clipboard_icon(fg)];
    if let Some(p) = &preview {
        kids.push(
            View::new(Style {
                size: Size { width: auto(), height: length(22.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(p.clone(), 12.0, fg),
        );
    }
    let v = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: auto(), height: length(22.0_f32) },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(5.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .children(kids);
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
/// `anchor_x` = x (px) del widget que lo abrió, para que salga **justo debajo**;
/// `avail_w` = ancho de la barra, para no desbordar el borde.
pub fn clipboard_panel(history: &[String], anchor_x: f32, avail_w: f32, theme: &Theme) -> View<Msg> {
    // El panel se centra bajo el widget, acotado a la pantalla.
    let left = (anchor_x - CLIP_MENU_W * 0.5).clamp(8.0, (avail_w - CLIP_MENU_W - 8.0).max(8.0));
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
            left: length(left),
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
pub fn clipboard_overlay(history: &[String], bar_h: f32, anchor_x: f32, avail_w: f32, theme: &Theme) -> View<Msg> {
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
    .children(vec![clipboard_panel(history, anchor_x, avail_w, theme)]);
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
