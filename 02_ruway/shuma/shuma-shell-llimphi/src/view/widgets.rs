//! Primitivos de UI compartidos por todos los sub-módulos de view.
//!
//! Aquí viven los constructores de View que no pertenecen a un dominio
//! concreto (sesión, herramienta, modal…): panel_frame, etiquetas,
//! botones, filas genéricas.

use super::super::*;
use llimphi_ui::llimphi_layout::taffy::prelude::{length, percent, Dimension, Style};
use llimphi_ui::llimphi_layout::taffy::{FlexDirection, Rect, Size};
use llimphi_ui::View;
use llimphi_theme::Theme;

// ─── Marco y encabezados de panel ──────────────────────────────────

/// Marco de un panel lateral: ancho fijo, **padding**, fondo y gap entre secciones.
pub(super) fn panel_frame(children: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .children(children)
}

/// Título de un panel (nombre de la sesión / herramienta).
pub(super) fn panel_title(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 13.0, theme.fg_text, Alignment::Start)
}

/// Etiqueta de sección (tenue, chica).
pub(super) fn panel_label(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(2.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 10.0, theme.fg_muted, Alignment::Start)
}

/// Nota/párrafo tenue dentro de un panel.
pub(super) fn panel_note(t: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(t.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}

/// Cabecera tenue de un panel/sección de herramienta.
pub(super) fn tool_header(titulo: &str, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo.to_string(), 11.0, theme.fg_muted, Alignment::Start)
}

// ─── Botones ────────────────────────────────────────────────────────

/// Un botón de acción (para el panel de matilda / shortcuts).
pub(super) fn action_button(label: &str, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(26.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        margin: Rect {
            left: length(0.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(6.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(5.0)
    .text_aligned(label.to_string(), 11.5, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

/// Botón de acción compacto (sin margen grande).
pub(super) fn action_button_small(label: &str, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::{AlignItems, JustifyContent};
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: Dimension::auto(), height: length(28.0_f32) },
        flex_shrink: 0.0,
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
    .fill(theme.bg_button)
    .hover_fill(theme.bg_button_hover)
    .radius(4.0)
    .text_aligned(label.to_string(), 11.0, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

// ─── Fila de chips y listas inline ─────────────────────────────────

/// Fila de chips, con wrap si no caben en el ancho del panel.
pub(super) fn chip_row(chips: Vec<View<Msg>>) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::FlexWrap;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        ..Default::default()
    })
    .children(chips)
}

/// Fila clickeable de un select expandido inline (form de sesión nueva).
pub(super) fn pick_row(label: String, msg: Msg, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(26.0_f32) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .hover_fill(theme.bg_row_hover)
    .radius(3.0)
    .text_aligned(label, 11.0, theme.fg_text, Alignment::Start)
    .on_click(msg)
}

/// Columna de `pick_row`s — el cuerpo expandido de un select inline.
pub(super) fn inline_list(rows: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: Dimension::auto() },
        gap: Size { width: length(0.0_f32), height: length(3.0_f32) },
        ..Default::default()
    })
    .children(rows)
}

/// Placeholder de área vacía o incompatible.
pub(crate) fn placeholder(theme: &Theme, text: &str) -> View<Msg> {
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(24.0_f32),
            right: length(24.0_f32),
            top: length(20.0_f32),
            bottom: length(20.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .text_aligned(text.to_string(), 13.0, theme.fg_muted, Alignment::Start)
}

// ─── Inventario (Matilda) ────────────────────────────────────────────

/// Lista de hosts del inventario de la sesión: nombre · dirección · tags.
pub(super) fn hosts_view(inv: &matilda_core::Inventory, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let mut filas: Vec<View<Msg>> = inv
        .hosts()
        .map(|h| {
            let tags = if h.tags.is_empty() {
                String::new()
            } else {
                format!("  [{}]", h.tags.join(", "))
            };
            inventory_row(
                format!("{}", h.name),
                format!("{}{tags}", h.address),
                theme,
            )
        })
        .collect();
    if filas.is_empty() {
        filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: Rect {
                    left: length(16.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                "sin hosts en el inventario".to_string(),
                12.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    }
    inventory_panel("Hosts", filas, theme)
}

/// Lista de vhosts del inventario: dominio · upstream · TLS.
pub(super) fn vhosts_view(inv: &matilda_core::Inventory, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    use matilda_core::Upstream;
    let mut filas: Vec<View<Msg>> = inv
        .vhosts()
        .map(|v| {
            let up = match &v.upstream {
                Upstream::Address(a) => a.clone(),
                Upstream::Container { name, port } => format!("{name}:{port}"),
            };
            let tls = if v.tls { "  TLS" } else { "" };
            inventory_row(v.domain.clone(), format!("-> {up}{tls}"), theme)
        })
        .collect();
    if filas.is_empty() {
        filas.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(28.0_f32) },
                padding: Rect {
                    left: length(16.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(
                "sin vhosts en el inventario".to_string(),
                12.0,
                theme.fg_muted,
                Alignment::Start,
            ),
        );
    }
    inventory_panel("Vhosts", filas, theme)
}

/// Una fila de inventario: título a la izquierda, detalle tenue a la derecha.
pub(super) fn inventory_row(titulo: String, detalle: String, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .hover_fill(theme.bg_row_hover)
    .children(vec![
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(titulo, 13.0, theme.fg_text, Alignment::Start),
        View::new(Style {
            size: Size { width: Dimension::auto(), height: percent(1.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(detalle, 12.0, theme.fg_muted, Alignment::End),
    ])
}

/// Marco de un panel de inventario: cabecera + filas en columna.
pub(super) fn inventory_panel(titulo: &str, filas: Vec<View<Msg>>, theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_layout::taffy::AlignItems;
    use llimphi_ui::llimphi_text::Alignment;
    let header = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        padding: Rect {
            left: length(16.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(titulo.to_string(), 12.0, theme.fg_muted, Alignment::Start);

    let mut children = vec![header];
    children.extend(filas);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}
