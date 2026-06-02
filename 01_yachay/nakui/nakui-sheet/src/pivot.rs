//! Motor de tabla dinámica (pivot) agnóstico de GUI: agrupa las filas de
//! un rango por el valor de una columna y resume otra con una función de
//! agregación. Opera sólo sobre [`Workbook`]/[`CellRef`]/[`CellRange`]/
//! [`SheetValue`] + `rust_decimal` — sin stack de UI. Extraído de
//! `nakui-sheet-llimphi/src/pivot.rs` (regla #2: el motor vive en el core,
//! el frontend sólo pinta el overlay).

use rust_decimal::Decimal;

use crate::{CellRange, CellRef, SheetValue, Workbook};

/// Función de agregación de una tabla dinámica.
#[derive(Clone, Copy, PartialEq)]
pub enum Agg {
    Sum,
    Count,
    Avg,
    Min,
    Max,
}

impl Agg {
    pub const ALL: [Agg; 5] = [Agg::Sum, Agg::Count, Agg::Avg, Agg::Min, Agg::Max];

    pub fn label(self) -> &'static str {
        match self {
            Agg::Sum => "SUMA",
            Agg::Count => "CONTAR",
            Agg::Avg => "PROM",
            Agg::Min => "MÍN",
            Agg::Max => "MÁX",
        }
    }

    /// Rota a la siguiente/anterior función (con wrap).
    pub fn cycle(self, dir: i32) -> Agg {
        let n = Self::ALL.len() as i32;
        let idx = Self::ALL.iter().position(|a| *a == self).unwrap_or(0) as i32;
        let next = ((idx + dir) % n + n) % n;
        Self::ALL[next as usize]
    }
}

/// Estado de la tabla dinámica (pivot) abierta sobre una selección.
/// Agrupa las filas del rango por el valor de `group_col` y agrega
/// `value_col` con `agg`.
#[derive(Clone)]
pub struct PivotState {
    /// Rango fuente sobre el que se computa (snapshot de la selección
    /// al abrir el pivot — no sigue cambiando si después scrolleás).
    pub source: CellRange,
    /// Columna absoluta cuyos valores definen los grupos.
    pub group_col: u32,
    /// Columna absoluta que se agrega dentro de cada grupo.
    pub value_col: u32,
    /// Función de agregación activa.
    pub agg: Agg,
    /// Si la primera fila del rango son encabezados (se excluye de la
    /// agregación y rotula las columnas group/value).
    pub header_row: bool,
}

/// Acumulador de un grupo (o del total global) del pivot.
struct PivotAcc {
    key: String,
    sum: Decimal,
    num_count: usize,
    row_count: usize,
    min: Option<Decimal>,
    max: Option<Decimal>,
}

impl PivotAcc {
    fn new(key: String) -> Self {
        Self {
            key,
            sum: Decimal::ZERO,
            num_count: 0,
            row_count: 0,
            min: None,
            max: None,
        }
    }

    fn push(&mut self, num: Option<Decimal>) {
        self.row_count += 1;
        if let Some(n) = num {
            self.num_count += 1;
            self.sum += n;
            self.min = Some(self.min.map_or(n, |m| m.min(n)));
            self.max = Some(self.max.map_or(n, |m| m.max(n)));
        }
    }

    fn value(&self, agg: Agg) -> Decimal {
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
pub struct PivotResult {
    pub rows: Vec<(String, Decimal)>,
    pub total: Decimal,
    pub groups: usize,
    pub n: usize,
}

/// Clave de grupo de una celda: su display formateado, o `(vacío)`.
pub fn pivot_key(wb: &Workbook, cr: CellRef) -> String {
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
pub fn compute_pivot(wb: &Workbook, p: &PivotState) -> PivotResult {
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
pub fn pivot_col_label(wb: &Workbook, p: &PivotState, col: u32) -> String {
    if p.header_row {
        let head = wb.formatted(CellRef::new(col, p.source.start.row));
        if !head.is_empty() {
            return head;
        }
    }
    format!("col {}", CellRef::col_label(col))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Construye un workbook 1 columna grupo (A) + 1 columna valor (B) a
    // partir de filas (grupo, valor) arrancando en la fila `start_row`.
    fn wb_con(filas: &[(&str, i64)], start_row: u32) -> Workbook {
        let mut wb = Workbook::new();
        for (i, (g, v)) in filas.iter().enumerate() {
            let r = start_row + i as u32;
            wb.set_cell(CellRef::new(0, r), g).unwrap();
            wb.set_cell(CellRef::new(1, r), &v.to_string()).unwrap();
        }
        wb
    }

    fn estado(start: u32, end: u32, agg: Agg, header: bool) -> PivotState {
        PivotState {
            source: CellRange::new(CellRef::new(0, start), CellRef::new(1, end)),
            group_col: 0,
            value_col: 1,
            agg,
            header_row: header,
        }
    }

    #[test]
    fn agrupa_y_suma_en_orden_de_aparicion() {
        let wb = wb_con(&[("norte", 10), ("sur", 5), ("norte", 3), ("sur", 2)], 0);
        let r = compute_pivot(&wb, &estado(0, 3, Agg::Sum, false));
        assert_eq!(r.groups, 2);
        assert_eq!(r.n, 4);
        assert_eq!(r.rows[0].0, "norte");
        assert_eq!(r.rows[0].1, Decimal::from(13));
        assert_eq!(r.rows[1].1, Decimal::from(7));
        assert_eq!(r.total, Decimal::from(20));
    }

    #[test]
    fn header_row_excluye_la_primera_fila() {
        let wb = wb_con(&[("región", 0), ("norte", 10), ("norte", 4)], 0);
        let r = compute_pivot(&wb, &estado(0, 2, Agg::Sum, true));
        assert_eq!(r.n, 2);
        assert_eq!(r.groups, 1);
        assert_eq!(r.rows[0].0, "norte");
        assert_eq!(r.rows[0].1, Decimal::from(14));
    }

    #[test]
    fn count_avg_min_max() {
        let wb = wb_con(&[("a", 2), ("a", 8), ("a", 5)], 0);
        assert_eq!(compute_pivot(&wb, &estado(0, 2, Agg::Count, false)).total, Decimal::from(3));
        assert_eq!(compute_pivot(&wb, &estado(0, 2, Agg::Avg, false)).total, Decimal::from(5));
        assert_eq!(compute_pivot(&wb, &estado(0, 2, Agg::Min, false)).total, Decimal::from(2));
        assert_eq!(compute_pivot(&wb, &estado(0, 2, Agg::Max, false)).total, Decimal::from(8));
    }

    #[test]
    fn celda_vacia_es_su_propio_grupo() {
        let wb = wb_con(&[("", 1), ("x", 2)], 0);
        let r = compute_pivot(&wb, &estado(0, 1, Agg::Sum, false));
        assert!(r.rows.iter().any(|(k, _)| k == "(vacío)"));
    }

    #[test]
    fn agg_cycle_envuelve_en_ambos_sentidos() {
        assert!(Agg::Sum.cycle(-1) == Agg::Max);
        assert!(Agg::Max.cycle(1) == Agg::Sum);
    }
}
