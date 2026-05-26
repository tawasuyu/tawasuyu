//! `Sheet` — la hoja de cálculo en memoria. Coordina la tabla de
//! celdas, el grafo de dependencias y el evaluador. Es la API que
//! consume el binario CLI (Bloque 5) y, una capa arriba, la
//! integración con el WAL/executor de nakui-core (Bloque 4).
//!
//! Atomicidad: `set_cell` aplica el cambio solo si:
//!   1. La fórmula parsea.
//!   2. Las nuevas dependencias no introducen ciclo.
//!   3. (El check de invariantes lo hará el Bloque 4.)
//! En cualquier fallo, el estado anterior se preserva intacto.
//!
//! La evaluación es topológica sobre el subgrafo afectado, así que
//! editar una celda en una hoja de 100 000 fórmulas cuesta lo que
//! cuestan las que dependen de ella, no la hoja entera.

use crate::cell::CellRef;
use crate::formula::{self, CellResolver, FormulaExpr};
use crate::graph::{CycleError, SheetGraph};
use crate::value::{SheetError, SheetValue};
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SetError {
    #[error("parse error in formula: {0}")]
    Parse(#[from] formula::ParseError),
    #[error("dependency cycle: {0}")]
    Cycle(#[from] CycleError),
}

/// Detalle de un `set_cell`: qué celdas cambiaron de valor (la
/// editada más todas las downstream que se recomputaron).
#[derive(Debug, Default, Clone)]
pub struct SetReport {
    pub changed: Vec<(CellRef, SheetValue, SheetValue)>,
}

#[derive(Debug, Clone)]
struct CellState {
    /// Texto original tal como lo tecleó el usuario (con o sin `=`).
    /// Sirve para mostrar la fórmula al usuario y para re-parsear si
    /// el formato del AST cambia entre versiones.
    raw: String,
    expr: FormulaExpr,
    value: SheetValue,
}

#[derive(Debug, Default, Clone)]
pub struct Sheet {
    cells: HashMap<CellRef, CellState>,
    graph: SheetGraph,
}

impl Sheet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Texto original de la celda (con `=` líder si es fórmula).
    pub fn raw(&self, cr: CellRef) -> Option<&str> {
        self.cells.get(&cr).map(|s| s.raw.as_str())
    }

    /// Valor computado de la celda. Una celda nunca-tocada devuelve
    /// `SheetValue::Empty` por contrato — Excel-compatible.
    pub fn value(&self, cr: CellRef) -> SheetValue {
        self.cells
            .get(&cr)
            .map(|s| s.value.clone())
            .unwrap_or(SheetValue::Empty)
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Itera (CellRef, valor) sobre todas las celdas con contenido.
    /// Útil para serializar/exportar.
    pub fn iter_values(&self) -> impl Iterator<Item = (CellRef, &SheetValue)> {
        self.cells.iter().map(|(c, s)| (*c, &s.value))
    }

    /// Escribe (o reescribe) una celda. Pipeline:
    ///   1. Si `raw` está vacío, borra la celda.
    ///   2. Parsea como fórmula (`=...`) o como literal.
    ///   3. Calcula dependencias y actualiza el grafo. Si crea
    ///      ciclo, aborta sin tocar nada.
    ///   4. Recalcula el subgrafo downstream en orden topo.
    ///   5. Devuelve qué celdas cambiaron.
    pub fn set_cell(&mut self, cr: CellRef, raw: &str) -> Result<SetReport, SetError> {
        if raw.is_empty() {
            return Ok(self.clear_cell(cr));
        }

        let expr = parse_input(raw)?;
        let deps = formula::dependencies(&expr);

        // El grafo decide ciclos. Si rechaza, devolvemos sin
        // tocar `self.cells` — atómico.
        self.graph.set_deps(cr, &deps)?;

        // Inserto/actualizo la celda con un valor placeholder; el
        // recálculo de abajo lo sobrescribe.
        let prev_value = self
            .cells
            .get(&cr)
            .map(|s| s.value.clone())
            .unwrap_or(SheetValue::Empty);
        self.cells.insert(
            cr,
            CellState {
                raw: raw.to_string(),
                expr,
                value: SheetValue::Empty,
            },
        );

        Ok(self.recalc_from(&[cr], Some((cr, prev_value))))
    }

    /// Borra una celda. Equivale a `set_cell(cr, "")` excepto que no
    /// pasa por el parser.
    pub fn clear_cell(&mut self, cr: CellRef) -> SetReport {
        if !self.cells.contains_key(&cr) {
            return SetReport::default();
        }
        let prev_value = self.cells[&cr].value.clone();
        // Una celda vacía no tiene deps; el grafo absorbe el cambio.
        let _ = self.graph.set_deps(cr, &[]);
        self.cells.remove(&cr);
        // Aunque la celda en sí ya no existe, sus downstream sí siguen
        // referenciándola — se evaluarán contra `SheetValue::Empty`.
        let mut report = self.recalc_from(&[cr], Some((cr, prev_value.clone())));
        // El cambio principal (cr: prev → Empty) lo metemos manual al
        // inicio del reporte porque la celda ya no está en `cells`.
        report
            .changed
            .insert(0, (cr, prev_value, SheetValue::Empty));
        report
    }

    /// Recalcula el subgrafo downstream a partir de `seeds`. Si
    /// `seed_with_prev` se da, se trata como "esa celda acaba de
    /// cambiar; usa este valor como referencia para detectar si
    /// cambió". Útil para `set_cell`.
    fn recalc_from(
        &mut self,
        seeds: &[CellRef],
        seed_with_prev: Option<(CellRef, SheetValue)>,
    ) -> SetReport {
        let order = self.graph.downstream_topo(seeds);
        let mut report = SetReport::default();
        let mut seed_prev: HashMap<CellRef, SheetValue> = HashMap::new();
        if let Some((c, v)) = seed_with_prev {
            seed_prev.insert(c, v);
        }

        for cell in order {
            // Si el nodo está en el grafo pero no en `cells`, es una
            // referencia "vacía": un downstream apunta a ella pero
            // nunca se le asignó contenido. No hay nada que evaluar
            // ahí; los lectores la verán como Empty.
            let expr = match self.cells.get(&cell).map(|s| s.expr.clone()) {
                Some(e) => e,
                None => continue,
            };

            let resolver = ValueLookup { cells: &self.cells };
            let new_val = formula::eval_formula(&expr, &resolver);
            let old_val = match seed_prev.remove(&cell) {
                Some(v) => v,
                None => self
                    .cells
                    .get(&cell)
                    .map(|s| s.value.clone())
                    .unwrap_or(SheetValue::Empty),
            };

            if old_val != new_val {
                if let Some(state) = self.cells.get_mut(&cell) {
                    state.value = new_val.clone();
                }
                report.changed.push((cell, old_val, new_val));
            }
        }

        report
    }
}

/// Adaptador entre el `HashMap` interno y el trait `CellResolver` del
/// evaluador. No clona el mapa; solo presta una vista.
struct ValueLookup<'a> {
    cells: &'a HashMap<CellRef, CellState>,
}

impl<'a> CellResolver for ValueLookup<'a> {
    fn resolve(&self, cell: CellRef) -> SheetValue {
        self.cells
            .get(&cell)
            .map(|s| s.value.clone())
            .unwrap_or(SheetValue::Empty)
    }
}

/// Convierte un input crudo en `FormulaExpr`. `=...` se manda al
/// parser. Sin `=`, intentamos en este orden: número, bool, texto
/// (fallback siempre exitoso). Esto reproduce el comportamiento de
/// Excel donde `42` y `=42` son equivalentes a nivel valor pero
/// distintos a nivel "este es un cálculo".
fn parse_input(raw: &str) -> Result<FormulaExpr, formula::ParseError> {
    if let Some(formula_src) = raw.strip_prefix('=') {
        return formula::compile(formula_src);
    }
    // Literal: número (incluye signo y decimales), TRUE/FALSE, o texto.
    if let Ok(n) = Decimal::from_str(raw.trim()) {
        return Ok(FormulaExpr::Number(n));
    }
    match raw.trim().to_uppercase().as_str() {
        "TRUE" => return Ok(FormulaExpr::Bool(true)),
        "FALSE" => return Ok(FormulaExpr::Bool(false)),
        _ => {}
    }
    Ok(FormulaExpr::Text(raw.to_string()))
}

/// Helper para tests: comprueba que una secuencia es topológicamente
/// válida — todos los predecesores aparecen antes que sus sucesores.
#[cfg(test)]
fn assert_topo(order: &[CellRef], edges: &[(CellRef, CellRef)]) {
    let pos: HashMap<CellRef, usize> = order.iter().enumerate().map(|(i, c)| (*c, i)).collect();
    for (a, b) in edges {
        let pa = pos.get(a).copied();
        let pb = pos.get(b).copied();
        if let (Some(pa), Some(pb)) = (pa, pb) {
            assert!(pa < pb, "edge {a} → {b} violated by order {order:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn literal_input_becomes_number() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "42").unwrap();
        assert_eq!(s.value(cr("A1")), SheetValue::Number(dec("42")));
    }

    #[test]
    fn formula_evaluates_after_dependencies_set() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "10").unwrap();
        s.set_cell(cr("B1"), "20").unwrap();
        s.set_cell(cr("C1"), "=A1+B1").unwrap();
        assert_eq!(s.value(cr("C1")), SheetValue::Number(dec("30")));
    }

    #[test]
    fn editing_upstream_cascades_to_downstream() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "=B1+C1").unwrap();
        s.set_cell(cr("D1"), "=A1*2").unwrap();
        s.set_cell(cr("B1"), "3").unwrap();
        s.set_cell(cr("C1"), "4").unwrap();
        // A1 = 3+4 = 7; D1 = 14
        assert_eq!(s.value(cr("A1")), SheetValue::Number(dec("7")));
        assert_eq!(s.value(cr("D1")), SheetValue::Number(dec("14")));
        // Cambio B1 → 10. Cascada: A1=14, D1=28.
        let report = s.set_cell(cr("B1"), "10").unwrap();
        assert_eq!(s.value(cr("A1")), SheetValue::Number(dec("14")));
        assert_eq!(s.value(cr("D1")), SheetValue::Number(dec("28")));
        // Reporte debe contener B1, A1 y D1.
        let touched: HashSet<_> = report.changed.iter().map(|(c, _, _)| *c).collect();
        assert!(touched.contains(&cr("B1")));
        assert!(touched.contains(&cr("A1")));
        assert!(touched.contains(&cr("D1")));
    }

    #[test]
    fn topological_order_respected_diamond() {
        let mut s = Sheet::new();
        // D = B + C, B = A*2, C = A+1, A = 5.
        // Cambiar A debe recomputar A → (B,C) → D.
        s.set_cell(cr("A1"), "5").unwrap();
        s.set_cell(cr("B1"), "=A1*2").unwrap();
        s.set_cell(cr("C1"), "=A1+1").unwrap();
        s.set_cell(cr("D1"), "=B1+C1").unwrap();
        let report = s.set_cell(cr("A1"), "10").unwrap();
        // A1=10, B1=20, C1=11, D1=31
        assert_eq!(s.value(cr("D1")), SheetValue::Number(dec("31")));
        let order: Vec<_> = report.changed.iter().map(|(c, _, _)| *c).collect();
        assert_topo(
            &order,
            &[
                (cr("A1"), cr("B1")),
                (cr("A1"), cr("C1")),
                (cr("B1"), cr("D1")),
                (cr("C1"), cr("D1")),
            ],
        );
    }

    #[test]
    fn cycle_rejected_with_state_intact() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "=B1+1").unwrap();
        // A1 depende de B1. Intentar B1 = A1+1 cerraría el ciclo.
        let err = s.set_cell(cr("B1"), "=A1+1").unwrap_err();
        assert!(matches!(err, SetError::Cycle(_)));
        // B1 quedó sin contenido — el rechazo no debe dejar basura.
        assert_eq!(s.value(cr("B1")), SheetValue::Empty);
        // A1 sigue intacto.
        assert_eq!(s.raw(cr("A1")), Some("=B1+1"));
    }

    #[test]
    fn empty_cells_evaluate_as_empty_to_zero_in_sum() {
        let mut s = Sheet::new();
        // Las referencias a celdas vacías valen 0 en aritmética.
        s.set_cell(cr("A1"), "=B1+C1+10").unwrap();
        assert_eq!(s.value(cr("A1")), SheetValue::Number(dec("10")));
    }

    #[test]
    fn clearing_cell_propagates_to_downstream() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "5").unwrap();
        s.set_cell(cr("B1"), "=A1*10").unwrap();
        assert_eq!(s.value(cr("B1")), SheetValue::Number(dec("50")));
        s.clear_cell(cr("A1"));
        // A1 ahora Empty → 0; B1 = 0 * 10 = 0.
        assert_eq!(s.value(cr("A1")), SheetValue::Empty);
        assert_eq!(s.value(cr("B1")), SheetValue::Number(dec("0")));
    }

    #[test]
    fn reassigning_formula_changes_dependency_set() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "1").unwrap();
        s.set_cell(cr("B1"), "100").unwrap();
        s.set_cell(cr("C1"), "=A1+1").unwrap();
        assert_eq!(s.value(cr("C1")), SheetValue::Number(dec("2")));
        // Reescribir C1 para depender de B1, no de A1.
        s.set_cell(cr("C1"), "=B1+1").unwrap();
        assert_eq!(s.value(cr("C1")), SheetValue::Number(dec("101")));
        // Cambiar A1 ahora NO afecta a C1.
        let report = s.set_cell(cr("A1"), "999").unwrap();
        let touched: HashSet<_> = report.changed.iter().map(|(c, _, _)| *c).collect();
        assert!(touched.contains(&cr("A1")));
        assert!(!touched.contains(&cr("C1")));
    }

    #[test]
    fn sum_range_works_end_to_end() {
        let mut s = Sheet::new();
        for row in 0..5 {
            s.set_cell(CellRef::new(0, row), &(row + 1).to_string())
                .unwrap();
        }
        s.set_cell(cr("B1"), "=SUM(A1:A5)").unwrap();
        // 1+2+3+4+5 = 15
        assert_eq!(s.value(cr("B1")), SheetValue::Number(dec("15")));
        // Modificar A3 cascadea a B1.
        s.set_cell(cr("A3"), "100").unwrap();
        assert_eq!(s.value(cr("B1")), SheetValue::Number(dec("112")));
    }

    #[test]
    fn div_by_zero_error_propagates_into_downstream() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "=10/0").unwrap();
        s.set_cell(cr("B1"), "=A1+1").unwrap();
        assert_eq!(s.value(cr("A1")), SheetValue::Error(SheetError::DivZero));
        assert_eq!(s.value(cr("B1")), SheetValue::Error(SheetError::DivZero));
    }

    #[test]
    fn unchanged_value_not_reported() {
        let mut s = Sheet::new();
        s.set_cell(cr("A1"), "5").unwrap();
        s.set_cell(cr("B1"), "=A1*0").unwrap(); // B1 = 0
        // Cambiar A1 a otro número: A1 cambia, B1 sigue 0.
        let report = s.set_cell(cr("A1"), "7").unwrap();
        let touched: HashSet<_> = report.changed.iter().map(|(c, _, _)| *c).collect();
        assert!(touched.contains(&cr("A1")));
        assert!(!touched.contains(&cr("B1")));
    }
}
