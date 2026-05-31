use super::*;

pub(crate) fn text_caret_can_move_left(bar: &TextInputState) -> bool {
    bar.editor().cursor.caret.col > 0
}

pub(crate) fn text_caret_can_move_right(bar: &TextInputState) -> bool {
    let line = bar.editor().cursor.caret.line;
    let len = bar.editor().buffer.line_len_chars(line);
    bar.editor().cursor.caret.col < len
}

pub(crate) fn move_cell(cr: CellRef, dir: Dir) -> CellRef {
    let col = cr.col;
    let row = cr.row;
    // Sin clamp a VISIBLE_* — la hoja es virtualmente ilimitada;
    // el viewport sigue a la selección vía `ensure_visible`.
    match dir {
        Dir::Up => CellRef::new(col, row.saturating_sub(1)),
        Dir::Down => CellRef::new(col, row.saturating_add(1)),
        Dir::Left => CellRef::new(col.saturating_sub(1), row),
        Dir::Right => CellRef::new(col.saturating_add(1), row),
    }
}

pub(crate) fn applied_count(wb: &Workbook) -> usize {
    wb.applied_count()
}

/// Rectángulo de selección actual normalizado (top-left + bottom-right).
pub(crate) fn selection_rect(model: &Model) -> CellRange {
    CellRange::new(model.anchor, model.selected)
}

pub(crate) fn selection_is_single(model: &Model) -> bool {
    model.anchor == model.selected
}

/// Status descriptivo de la selección: una sola celda → vacío
/// (volvemos al estado neutro); un rango → "Sel: A1:C5 · 15 celdas
/// · suma 234.5" si hay numéricos.
pub(crate) fn selection_status(model: &Model) -> Status {
    if selection_is_single(model) {
        return Status::default();
    }
    let r = selection_rect(model);
    let count = r.cell_count();
    let mut sum = rust_decimal::Decimal::ZERO;
    let mut num_count = 0u32;
    for cr in r.iter() {
        if let SheetValue::Number(n) = model.wb.value(cr) {
            sum += n;
            num_count += 1;
        }
    }
    let text = if num_count == 0 {
        format!("  Sel: {} · {} celdas", r, count)
    } else {
        let avg = sum / rust_decimal::Decimal::from(num_count as i64);
        format!(
            "  Sel: {} · {} celdas · suma {} · prom {}",
            r,
            count,
            sum.normalize(),
            avg.normalize()
        )
    };
    Status {
        text,
        kind: StatusKind::Info,
    }
}

pub(crate) fn cell_in_selection(model: &Model, cr: CellRef) -> bool {
    if selection_is_single(model) {
        cr == model.selected
    } else {
        let r = selection_rect(model);
        cr.col >= r.start.col
            && cr.col <= r.end.col
            && cr.row >= r.start.row
            && cr.row <= r.end.row
    }
}

/// Aplica el contenido actual de la barra a la celda seleccionada
/// y actualiza el status. No toca `editing` — el caller decide qué
/// hacer con ese flag (Commit lo desactiva; Move lo desactiva tras
/// commit; SelectCell lo desactiva tras commit).
pub(crate) fn commit_bar(model: &mut Model) {
    let raw = model.bar.text();
    match model.wb.set_cell(model.selected, &raw) {
        Ok(report) => {
            model.status = Status {
                text: format!(
                    "  {} celda(s) recomputada(s)  ·  WAL: {} eventos",
                    report.changed.len(),
                    model.wb.events().len()
                ),
                kind: StatusKind::Info,
            };
        }
        Err(e) => {
            model.status = Status {
                text: format!("  ✗ {e}"),
                kind: StatusKind::Error,
            };
        }
    }
}

/// Paste con shift-de-fórmulas si la fuente coincide con
/// `clipboard_origin`. Si el clipboard del sistema cambió (el
/// usuario copió texto de otra app), pega literal.
pub(crate) fn paste_into(
    wb: &mut Workbook,
    dest: CellRef,
    origin: &Option<(String, CellRef)>,
) -> Status {
    let payload = match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
        Ok(t) => t,
        Err(e) => {
            return Status {
                text: format!("  ✗ clipboard vacío: {e}"),
                kind: StatusKind::Error,
            };
        }
    };
    // Caso 1: paste interno coherente con un copy/cut previo →
    // shift de fórmulas. La fuente y el raw deben coincidir
    // exactamente; si el user cambió la celda fuente entremedias,
    // el origin queda desactualizado y caemos al paste literal.
    if let Some((origin_raw, origin_cell)) = origin {
        if *origin_raw == payload {
            let drow = dest.row as i32 - origin_cell.row as i32;
            let dcol = dest.col as i32 - origin_cell.col as i32;
            let new_raw = shift_raw(&payload, drow, dcol);
            return match wb.set_cell(dest, &new_raw) {
                Ok(_) => Status {
                    text: format!("  ⇲ pegado en {dest} (shift {drow:+},{dcol:+})"),
                    kind: StatusKind::Info,
                },
                Err(e) => Status {
                    text: format!("  ✗ paste: {e}"),
                    kind: StatusKind::Error,
                },
            };
        }
    }
    // Caso 2: paste literal — clipboard de otra app o cambió de
    // contenido. Lo metemos tal cual.
    match wb.set_cell(dest, &payload) {
        Ok(_) => Status {
            text: format!("  ⇲ pegado en {dest}"),
            kind: StatusKind::Info,
        },
        Err(e) => Status {
            text: format!("  ✗ paste: {e}"),
            kind: StatusKind::Error,
        },
    }
}

/// Shifta el raw como lo haría un fill: parse → shift → render. Si
/// el raw no es una fórmula (no empieza con `=`) o no parsea, lo
/// devolvemos sin tocar — un literal numérico o texto no se shifta.
pub(crate) fn shift_raw(raw: &str, drow: i32, dcol: i32) -> String {
    let stripped = match raw.strip_prefix('=') {
        Some(s) => s,
        None => return raw.to_string(),
    };
    match nakui_sheet::formula::compile(stripped) {
        Ok(expr) => {
            let shifted = nakui_sheet::formula::shift(&expr, drow, dcol);
            format!("={}", nakui_sheet::formula::render(&shifted))
        }
        Err(_) => raw.to_string(),
    }
}

pub(crate) fn apply_scroll_axis(viewport: u32, delta: i32) -> u32 {
    if delta >= 0 {
        viewport.saturating_add(delta as u32)
    } else {
        viewport.saturating_sub((-delta) as u32)
    }
}

/// Índice de columna *en pantalla* (0 = primera columna tras el row
/// header) de una columna absoluta, teniendo en cuenta la banda
/// inmovilizada. Las columnas frozen ocupan las primeras `freeze_cols`
/// ranuras; el resto se mide desde el viewport scrolleable.
pub(crate) fn screen_col_index(model: &Model, col: u32) -> u32 {
    if col < model.freeze_cols {
        col
    } else {
        model.freeze_cols + col.saturating_sub(model.viewport_col)
    }
}

/// Análogo a [`screen_col_index`] sobre el eje de filas.
pub(crate) fn screen_row_index(model: &Model, row: u32) -> u32 {
    if row < model.freeze_rows {
        row
    } else {
        model.freeze_rows + row.saturating_sub(model.viewport_row)
    }
}

/// Empuja el viewport scrolleable de vuelta a respetar la banda
/// inmovilizada. Las filas/columnas `< freeze_*` se pintan aparte y
/// SIEMPRE; el área que scrollea arranca recién en `freeze_*`.
pub(crate) fn clamp_viewport_to_freeze(model: &mut Model) {
    model.viewport_row = model.viewport_row.max(model.freeze_rows);
    model.viewport_col = model.viewport_col.max(model.freeze_cols);
}

// ─────────────────────────── Pivot ───────────────────────────

/// Rota una columna (group/value) dentro de `[start.col, end.col]`
/// del rango fuente, con wrap.
pub(crate) fn cycle_col(col: u32, range: &CellRange, dir: i32) -> u32 {
    let lo = range.start.col;
    let hi = range.end.col;
    let span = (hi - lo + 1) as i32;
    let rel = col.saturating_sub(lo) as i32;
    let next = ((rel + dir) % span + span) % span;
    lo + next as u32
}

