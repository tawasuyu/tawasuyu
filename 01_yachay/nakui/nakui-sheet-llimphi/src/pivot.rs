use super::*;

/// Acumulador de un grupo (o del total global) del pivot.
pub(crate) struct PivotAcc {
    key: String,
    sum: rust_decimal::Decimal,
    num_count: usize,
    row_count: usize,
    min: Option<rust_decimal::Decimal>,
    max: Option<rust_decimal::Decimal>,
}

impl PivotAcc {
    fn new(key: String) -> Self {
        Self {
            key,
            sum: rust_decimal::Decimal::ZERO,
            num_count: 0,
            row_count: 0,
            min: None,
            max: None,
        }
    }

    fn push(&mut self, num: Option<rust_decimal::Decimal>) {
        self.row_count += 1;
        if let Some(n) = num {
            self.num_count += 1;
            self.sum += n;
            self.min = Some(self.min.map_or(n, |m| m.min(n)));
            self.max = Some(self.max.map_or(n, |m| m.max(n)));
        }
    }

    fn value(&self, agg: Agg) -> rust_decimal::Decimal {
        use rust_decimal::Decimal;
        match agg {
            Agg::Sum => self.sum,
            Agg::Count => Decimal::from(self.row_count as i64),
            Agg::Avg => {
                if self.num_count > 0 {
                    self.sum / Decimal::from(self.num_count as i64)
                } else {
                    Decimal::ZERO
                }
            }
            Agg::Min => self.min.unwrap_or(Decimal::ZERO),
            Agg::Max => self.max.unwrap_or(Decimal::ZERO),
        }
    }
}

/// Resultado de computar una tabla dinámica: filas agregadas (en
/// orden de aparición), total global, cantidad de grupos y de filas
/// efectivamente agregadas.
pub(crate) struct PivotResult {
    rows: Vec<(String, rust_decimal::Decimal)>,
    total: rust_decimal::Decimal,
    groups: usize,
    n: usize,
}

/// Clave de grupo de una celda: su display formateado, o `(vacío)`.
pub(crate) fn pivot_key(wb: &Workbook, cr: CellRef) -> String {
    match wb.value(cr) {
        SheetValue::Empty => "(vacío)".to_string(),
        _ => {
            let s = wb.formatted(cr);
            if s.is_empty() {
                "(vacío)".to_string()
            } else {
                s
            }
        }
    }
}

/// Agrega el rango del pivot agrupando por `group_col` y resumiendo
/// `value_col` con `agg`. Lineal sobre las filas; los grupos se
/// guardan en orden de aparición (los rangos del editor son chicos,
/// así que la búsqueda lineal por clave es de sobra).
pub(crate) fn compute_pivot(wb: &Workbook, p: &PivotState) -> PivotResult {
    let mut groups: Vec<PivotAcc> = Vec::new();
    let mut total = PivotAcc::new(String::new());
    let first_row = p.source.start.row;
    for row in p.source.start.row..=p.source.end.row {
        if p.header_row && row == first_row {
            continue;
        }
        let key = pivot_key(wb, CellRef::new(p.group_col, row));
        let num = match wb.value(CellRef::new(p.value_col, row)) {
            SheetValue::Number(n) => Some(n),
            _ => None,
        };
        match groups.iter_mut().find(|g| g.key == key) {
            Some(g) => g.push(num),
            None => {
                let mut acc = PivotAcc::new(key);
                acc.push(num);
                groups.push(acc);
            }
        }
        total.push(num);
    }
    let rows = groups
        .iter()
        .map(|g| (g.key.clone(), g.value(p.agg)))
        .collect();
    PivotResult {
        rows,
        total: total.value(p.agg),
        groups: groups.len(),
        n: total.row_count,
    }
}

/// Etiqueta corta de una columna para el encabezado del pivot: si la
/// fila 0 del rango es encabezado, usa su texto; si no, la letra de
/// columna (A, B, …).
pub(crate) fn pivot_col_label(wb: &Workbook, p: &PivotState, col: u32) -> String {
    if p.header_row {
        let head = wb.formatted(CellRef::new(col, p.source.start.row));
        if !head.is_empty() {
            return head;
        }
    }
    format!("col {}", CellRef::col_label(col))
}

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

