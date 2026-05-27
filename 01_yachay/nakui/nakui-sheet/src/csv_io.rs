//! Import/export CSV. Útil para sacar la hoja a una herramienta
//! externa (Excel, Numbers, pandas, una tabla en Postgres) o cargar
//! datos existentes en un Workbook nuevo.
//!
//! Convención:
//!   - Export: para cada celda con contenido, escribimos su `raw`
//!     (el texto que tecleó el usuario). Esto preserva las fórmulas
//!     — Excel y Sheets ambos leen `=A1+B1` en un CSV y lo
//!     reactivan. Si querés el valor formateado, hacé export con
//!     `Mode::Values`.
//!   - Import: cada campo se aplica como `set_cell` en la posición
//!     (col, row). Aprovecha el parser normal: un campo "42"
//!     queda como Number, "hola" como Text, "=A1+1" como fórmula.
//!
//! Layout: la celda `(col=0, row=0)` corresponde al primer campo
//! de la primera línea. Filas con menos campos que la línea más
//! ancha rellenan con celdas vacías (no se emite SetCell para
//! ellas).

use crate::cell::CellRef;
use crate::workbook::{Workbook, WorkbookError};
use std::io::{Read, Write};

/// Qué exportar por celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    /// Texto original que tecleó el usuario, incluyendo el `=` líder
    /// de las fórmulas. Apto para round-trip Nakui → Excel → Nakui:
    /// las fórmulas siguen vivas al re-importar.
    Raw,
    /// Valor formateado tal como se muestra en la grilla. Las
    /// fórmulas pierden su origen — el CSV resulta en una "foto"
    /// estática del cálculo, lo cual es lo que querés cuando lo
    /// pasás a un sistema que no entiende fórmulas (pandas, una
    /// API REST, un PDF).
    Values,
}

pub fn export_csv<W: Write>(
    wb: &Workbook,
    mode: ExportMode,
    writer: W,
) -> Result<(), WorkbookError> {
    let mut w = csv::Writer::from_writer(writer);
    // 1. Determinar la bounding box.
    let bbox = bounding_box(wb);
    let (max_row, max_col) = match bbox {
        Some(bb) => bb,
        None => return Ok(()), // hoja vacía
    };
    // 2. Recorrer fila por fila.
    for row in 0..=max_row {
        let mut record: Vec<String> = Vec::with_capacity((max_col + 1) as usize);
        for col in 0..=max_col {
            let cr = CellRef::new(col, row);
            let cell = match mode {
                ExportMode::Raw => wb.raw(cr).unwrap_or("").to_string(),
                ExportMode::Values => match wb.value(cr) {
                    crate::value::SheetValue::Empty => String::new(),
                    _ => wb.formatted(cr),
                },
            };
            record.push(cell);
        }
        w.write_record(&record).map_err(io_from_csv)?;
    }
    w.flush()?;
    Ok(())
}

pub fn import_csv<R: Read>(
    wb: &mut Workbook,
    reader: R,
) -> Result<usize, WorkbookError> {
    // `flexible(true)` permite que filas tengan distinto número de
    // campos; rellenamos con vacíos. `has_headers(false)`: el
    // primer registro NO se trata como cabecera. La celda (0, 0) es
    // el primer campo del primer registro.
    let mut r = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(reader);
    let mut applied = 0usize;
    for (row_idx, record_res) in r.records().enumerate() {
        let record = record_res.map_err(io_from_csv)?;
        for (col_idx, field) in record.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            let cr = CellRef::new(col_idx as u32, row_idx as u32);
            wb.set_cell(cr, field)?;
            applied += 1;
        }
    }
    Ok(applied)
}

/// Devuelve `(max_row, max_col)` entre las celdas con contenido. None
/// si la hoja está vacía.
fn bounding_box(wb: &Workbook) -> Option<(u32, u32)> {
    let mut max_row: Option<u32> = None;
    let mut max_col: Option<u32> = None;
    for (cr, _) in wb.sheet().iter_values() {
        max_row = Some(max_row.map_or(cr.row, |r| r.max(cr.row)));
        max_col = Some(max_col.map_or(cr.col, |c| c.max(cr.col)));
    }
    match (max_row, max_col) {
        (Some(r), Some(c)) => Some((r, c)),
        _ => None,
    }
}

fn io_from_csv(e: csv::Error) -> WorkbookError {
    WorkbookError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        e.to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::SheetValue;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn export_raw_preserves_formulas() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "10").unwrap();
        wb.set_cell(cr("B1"), "20").unwrap();
        wb.set_cell(cr("C1"), "=A1+B1").unwrap();
        let mut buf = Vec::new();
        export_csv(&wb, ExportMode::Raw, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.trim(), "10,20,=A1+B1");
    }

    #[test]
    fn export_values_resolves_formulas() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "10").unwrap();
        wb.set_cell(cr("B1"), "20").unwrap();
        wb.set_cell(cr("C1"), "=A1+B1").unwrap();
        let mut buf = Vec::new();
        export_csv(&wb, ExportMode::Values, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.trim(), "10,20,30");
    }

    #[test]
    fn round_trip_through_csv() {
        let mut wb1 = Workbook::new();
        wb1.set_cell(cr("A1"), "5").unwrap();
        wb1.set_cell(cr("B1"), "10").unwrap();
        wb1.set_cell(cr("C1"), "=A1*B1").unwrap();
        wb1.set_cell(cr("A2"), "Hola").unwrap();
        let mut buf = Vec::new();
        export_csv(&wb1, ExportMode::Raw, &mut buf).unwrap();
        let mut wb2 = Workbook::new();
        import_csv(&mut wb2, buf.as_slice()).unwrap();
        // C1 reconstruye la fórmula y la re-evalúa: 5*10 = 50.
        assert_eq!(wb2.value(cr("A1")), SheetValue::Number(dec("5")));
        assert_eq!(wb2.value(cr("C1")), SheetValue::Number(dec("50")));
        assert_eq!(wb2.value(cr("A2")), SheetValue::Text("Hola".into()));
    }

    #[test]
    fn export_includes_empty_cells_within_bounding_box() {
        // Si tengo contenido en A1 y C1 pero no en B1, el export
        // debe escribir "valA,,valC" (B1 vacío entre los dos).
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "a").unwrap();
        wb.set_cell(cr("C1"), "c").unwrap();
        let mut buf = Vec::new();
        export_csv(&wb, ExportMode::Raw, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.trim(), "a,,c");
    }

    #[test]
    fn empty_workbook_exports_nothing() {
        let wb = Workbook::new();
        let mut buf = Vec::new();
        export_csv(&wb, ExportMode::Raw, &mut buf).unwrap();
        assert!(buf.is_empty());
    }

    #[test]
    fn import_handles_jagged_rows() {
        // Fila 1: 2 campos. Fila 2: 4 campos. El import debe
        // ubicarlos en sus posiciones reales sin error.
        let csv_data = "a,b\nc,d,e,f\n";
        let mut wb = Workbook::new();
        import_csv(&mut wb, csv_data.as_bytes()).unwrap();
        assert_eq!(wb.value(cr("A1")), SheetValue::Text("a".into()));
        assert_eq!(wb.value(cr("B1")), SheetValue::Text("b".into()));
        assert_eq!(wb.value(cr("A2")), SheetValue::Text("c".into()));
        assert_eq!(wb.value(cr("D2")), SheetValue::Text("f".into()));
    }
}
