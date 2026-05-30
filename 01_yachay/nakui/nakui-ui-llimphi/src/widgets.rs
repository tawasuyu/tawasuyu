//! Helpers de layout y estilo reusados por los paneles: celdas, filas,
//! líneas de texto, estilos y paletas de botón. Todos son hojas (no
//! tocan el `Model`) y devuelven `View<Msg>` o tipos de Llimphi.

use super::*;

/// Label corto de un record para un selector `EntityRef`: id corto + un
/// preview del primer campo de texto.
pub(crate) fn entity_ref_label(id: &Uuid, rec: &Value) -> String {
    let preview = rec.as_object().and_then(|m| {
        m.values()
            .find_map(|v| v.as_str().map(|s| s.to_string()))
    });
    match preview {
        Some(name) => format!("{} · {}", short_uuid(id), preview_value(&Value::String(name), 24)),
        None => short_uuid(id),
    }
}

pub(crate) fn column(children: Vec<View<Msg>>, gap: f32) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(gap),
        },
        ..Default::default()
    })
    .children(children)
}

pub(crate) fn chip_row(children: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size {
            width: percent(1.0_f32),
            height: length(32.0),
        },
        gap: Size {
            width: length(6.0),
            height: length(6.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .children(children)
}

pub(crate) fn placeholder_panel(
    module: &Module,
    title: &str,
    body_lines: Vec<String>,
    theme: &Theme,
) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![text_line(
        format!("{} · {}", module.label, title),
        16.0,
        theme.fg_text,
    )];
    if let Some(desc) = &module.description {
        children.push(text_line(desc.clone(), 11.0, theme.fg_muted));
    }
    for line in body_lines {
        children.push(text_line(line, 12.0, theme.fg_text));
    }
    column(children, 6.0)
}

pub(crate) fn empty_panel(theme: &Theme, msg: &str) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(16.0_f32),
            right: length(16.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned(msg.to_string(), 12.0, theme.fg_muted, Alignment::Start)
}

pub(crate) fn text_line(content: String, size_px: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(size_px + 8.0),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, size_px, color, Alignment::Start)
}

/// Celda de ancho fijo (px) para columnas tipo id/acción.
pub(crate) fn cell_text(content: String, width: f32, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(24.0),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, 12.0, color, Alignment::Start)
}

/// Celda elástica para columnas de datos.
pub(crate) fn cell_flex(content: String, color: Color) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(24.0),
        },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(content, 12.0, color, Alignment::Start)
}

/// Style de botón de ancho fijo.
pub(crate) fn btn_style(width: f32) -> Style {
    Style {
        size: Size {
            width: length(width),
            height: length(30.0),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(10.0),
            right: length(10.0),
            top: length(4.0),
            bottom: length(4.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Style de botón que se ajusta al contenido (chips de select/ref).
pub(crate) fn btn_style_auto() -> Style {
    Style {
        size: Size {
            width: length(140.0),
            height: length(26.0),
        },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(8.0),
            right: length(8.0),
            top: length(2.0),
            bottom: length(2.0),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    }
}

/// Paleta de botón con acento (acción primaria / selección activa).
pub(crate) fn accent_btn(theme: &Theme) -> ButtonPalette {
    let mut p = ButtonPalette::from_theme(theme);
    p.bg = theme.accent;
    p.bg_hover = theme.accent;
    p.fg = theme.bg_app;
    p
}

/// Paleta de botón destructivo (borrar).
pub(crate) fn danger_btn(theme: &Theme) -> ButtonPalette {
    let mut p = ButtonPalette::from_theme(theme);
    p.bg = theme.fg_destructive;
    p.bg_hover = theme.fg_destructive;
    p.fg = theme.bg_app;
    p
}
