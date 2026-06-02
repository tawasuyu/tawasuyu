//! Render del overlay de la tabla dinámica. El motor (`Agg`, `PivotState`,
//! `compute_pivot`, `pivot_col_label`, `PivotResult`) vive en
//! `nakui_sheet::pivot` (regla #2) — acá sólo se pinta `View<Msg>`.

use super::*;

/// Una fila del panel del pivot: etiqueta a la izquierda, valor a la
/// derecha, en un contenedor flex con `space-between`.
pub(crate) fn pivot_panel_row(
    left: String,
    right: String,
    left_fg: Color,
    right_fg: Color,
    bg: Color,
    bold_h: f32,
) -> View<Msg> {
    let left_view = View::new(Style {
        size: Size {
            width: percent(0.66_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(left, 13.0, left_fg, Alignment::Start);
    let right_view = View::new(Style {
        size: Size {
            width: percent(0.34_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(right, 13.0, right_fg, Alignment::End);
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(bold_h),
        },
        justify_content: Some(JustifyContent::SpaceBetween),
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .children(vec![left_view, right_view])
}

/// Cuántos grupos se listan como máximo en el panel (el resto se
/// resume en una línea "… +k grupos").
pub(crate) const PIVOT_MAX_ROWS: usize = 18;

/// Overlay modal de la tabla dinámica: scrim + tarjeta centrada con
/// encabezado, filas agregadas, total y la línea de atajos.
pub(crate) fn pivot_overlay_view(wb: &Workbook, p: &PivotState) -> View<Msg> {
    let res = compute_pivot(wb, p);
    let gcol = pivot_col_label(wb, p, p.group_col);
    let vcol = pivot_col_label(wb, p, p.value_col);

    let mut card_children: Vec<View<Msg>> = Vec::new();

    // Encabezado: título + botón cerrar.
    let title = View::new(Style {
        size: Size {
            width: percent(0.8_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("Tabla dinámica".to_string(), 15.0, palette::FG_TEXT, Alignment::Start);
    let close = View::new(Style {
        size: Size {
            width: percent(0.2_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("✕  Esc".to_string(), 12.5, palette::FG_MUTED, Alignment::End)
    .on_click(Msg::ClosePivot);
    card_children.push(
        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: length(30.0_f32),
            },
            justify_content: Some(JustifyContent::SpaceBetween),
            padding: Rect {
                left: length(14.0_f32),
                right: length(14.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            ..Default::default()
        })
        .fill(palette::BG_PANEL_ALT)
        .children(vec![title, close]),
    );

    // Subtítulo: descripción de la agregación.
    card_children.push(pivot_panel_row(
        format!("Agrupar por «{gcol}»  ·  {}(«{vcol}»)", p.agg.label()),
        format!("{} sobre {}", p.source, if p.header_row { "c/encab." } else { "s/encab." }),
        palette::ACCENT,
        palette::FG_MUTED,
        palette::BG_PANEL,
        26.0,
    ));

    // Header de la tabla.
    card_children.push(pivot_panel_row(
        gcol.clone(),
        p.agg.label().to_string(),
        palette::FG_HEADER,
        palette::FG_HEADER,
        palette::BG_HEADER,
        24.0,
    ));

    // Filas agregadas (capadas).
    let shown = res.rows.len().min(PIVOT_MAX_ROWS);
    for (i, (key, val)) in res.rows.iter().take(shown).enumerate() {
        let bg = if i % 2 == 0 {
            palette::BG_CELL
        } else {
            palette::BG_PANEL
        };
        card_children.push(pivot_panel_row(
            key.clone(),
            val.normalize().to_string(),
            palette::FG_TEXT,
            palette::FG_TEXT,
            bg,
            24.0,
        ));
    }
    if res.rows.len() > shown {
        card_children.push(pivot_panel_row(
            format!("… +{} grupos", res.rows.len() - shown),
            String::new(),
            palette::FG_MUTED,
            palette::FG_MUTED,
            palette::BG_PANEL,
            22.0,
        ));
    }

    // Total.
    card_children.push(pivot_panel_row(
        format!("TOTAL  ·  {} grupos · {} filas", res.groups, res.n),
        res.total.normalize().to_string(),
        palette::ACCENT,
        palette::ACCENT,
        palette::BG_PANEL_ALT,
        28.0,
    ));

    // Línea de atajos.
    card_children.push(pivot_panel_row(
        "A función · G grupo · V valor · H encabezado · Esc cerrar".to_string(),
        String::new(),
        palette::FG_PLACEHOLDER,
        palette::FG_PLACEHOLDER,
        palette::BG_PANEL,
        24.0,
    ));

    let card = View::new(Style {
        size: Size {
            width: length(560.0_f32),
            height: auto(),
        },
        flex_direction: FlexDirection::Column,
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette::GRID_LINE)
    .children(card_children);

    // Scrim de pantalla completa con la tarjeta centrada.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 170))
    .children(vec![card])
}

