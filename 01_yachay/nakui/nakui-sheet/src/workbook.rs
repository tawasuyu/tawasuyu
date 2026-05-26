//! `Workbook` — `Sheet` con WAL persistente e invariantes.
//!
//! Cada `set_cell` ejecutado por el usuario se aplica sobre un
//! candidato (`Sheet::clone`), se validan todos los invariantes
//! declarados, y solo si todos pasan el cambio se promueve. Si algún
//! invariante falla, el workbook queda EXACTAMENTE como estaba —
//! "atomicidad de hoja", el principio del que se hablaba en el plan
//! inicial.
//!
//! Esta capa es donde Nakui se diferencia del Excel tradicional:
//! puedes declarar "el balance de caja nunca puede ser negativo" o
//! "SUM(D:D) = K1" como reglas, y el motor las hace cumplir contra
//! cada edición. No hay "fórmula rota y nadie se entera".
//!
//! El WAL aquí es local (Vec + JSONL). La integración con
//! `nakui-core::event_log` (canonical, drift-detected, replay vía
//! morfismos) es el siguiente bloque y vive como un trait que
//! implementa este `Vec` y, en producción, el log durable.

use crate::cell::CellRef;
use crate::formula::{self, CellResolver, FormulaExpr};
use crate::sheet::{SetError, SetReport, Sheet};
use crate::value::SheetValue;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkbookError {
    #[error(transparent)]
    Set(#[from] SetError),
    #[error("invariant `{name}` violated; edit reverted")]
    InvariantViolated { name: String, value: SheetValue },
    #[error("invariant parse error: {0}")]
    InvariantParse(#[from] formula::ParseError),
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("event log decode error at line {line}: {reason}")]
    LogDecode { line: usize, reason: String },
    #[error("event log refers to sequence numbers out of order")]
    LogOutOfOrder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SheetEvent {
    SetCell { cell: CellRef, raw: String },
    ClearCell { cell: CellRef },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordedEvent {
    pub seq: u64,
    /// Milisegundos desde Unix epoch. Sirve para time-travel por
    /// reloj, pero el orden canónico es `seq` — un sistema que
    /// rebobina el reloj no rompe el replay.
    pub timestamp_ms: u128,
    pub event: SheetEvent,
}

#[derive(Debug, Clone)]
struct Invariant {
    name: String,
    expr: FormulaExpr,
}

#[derive(Debug, Default)]
pub struct Workbook {
    sheet: Sheet,
    events: Vec<RecordedEvent>,
    invariants: Vec<Invariant>,
    next_seq: u64,
}

impl Workbook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    pub fn events(&self) -> &[RecordedEvent] {
        &self.events
    }

    pub fn value(&self, cr: CellRef) -> SheetValue {
        self.sheet.value(cr)
    }

    pub fn raw(&self, cr: CellRef) -> Option<&str> {
        self.sheet.raw(cr)
    }

    /// Declara un invariante que debe evaluar a `TRUE` tras cada
    /// edición. La fórmula se compila una vez; el name es para
    /// mensajes de error.
    pub fn add_invariant(&mut self, name: &str, formula: &str) -> Result<(), WorkbookError> {
        let expr = formula::compile(formula.strip_prefix('=').unwrap_or(formula))?;
        self.invariants.push(Invariant {
            name: name.to_string(),
            expr,
        });
        Ok(())
    }

    /// Aplica un `set_cell` con validación atómica de invariantes.
    /// Si cualquier invariante falla en el estado resultante, el
    /// workbook queda intacto y se devuelve el error.
    pub fn set_cell(&mut self, cr: CellRef, raw: &str) -> Result<SetReport, WorkbookError> {
        let event = if raw.is_empty() {
            SheetEvent::ClearCell { cell: cr }
        } else {
            SheetEvent::SetCell {
                cell: cr,
                raw: raw.to_string(),
            }
        };
        self.apply_user_event(event)
    }

    pub fn clear_cell(&mut self, cr: CellRef) -> Result<SetReport, WorkbookError> {
        self.apply_user_event(SheetEvent::ClearCell { cell: cr })
    }

    fn apply_user_event(&mut self, event: SheetEvent) -> Result<SetReport, WorkbookError> {
        let mut candidate = self.sheet.clone();
        let report = apply_to_sheet(&mut candidate, &event)?;
        Self::check_invariants(&self.invariants, &candidate)?;
        self.sheet = candidate;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push(RecordedEvent {
            seq,
            timestamp_ms: now_ms(),
            event,
        });
        Ok(report)
    }

    fn check_invariants(invariants: &[Invariant], sheet: &Sheet) -> Result<(), WorkbookError> {
        let resolver = SheetResolver { sheet };
        for inv in invariants {
            let value = formula::eval_formula(&inv.expr, &resolver);
            let ok = matches!(value, SheetValue::Bool(true));
            if !ok {
                return Err(WorkbookError::InvariantViolated {
                    name: inv.name.clone(),
                    value,
                });
            }
        }
        Ok(())
    }

    /// Serializa los eventos como JSONL — una línea por evento. El
    /// formato es estable: misma versión de Nakui produce el mismo
    /// bytes-for-bytes, lo cual es lo que permite verificar drift.
    pub fn write_log<W: Write>(&self, mut w: W) -> Result<(), WorkbookError> {
        for ev in &self.events {
            serde_json::to_writer(&mut w, ev).map_err(|e| {
                WorkbookError::LogDecode {
                    line: ev.seq as usize,
                    reason: e.to_string(),
                }
            })?;
            w.write_all(b"\n")?;
        }
        Ok(())
    }

    /// Reconstruye un workbook desde un log JSONL. Reaplica cada
    /// evento en orden de `seq` (debe ser estrictamente creciente
    /// desde 0). No reaplica invariantes — el log es la fuente de
    /// verdad de lo que ocurrió, y si fuera inconsistente lo
    /// detectaríamos al evaluar.
    pub fn from_log<R: BufRead>(r: R) -> Result<Self, WorkbookError> {
        let mut wb = Self::new();
        let mut expected = 0u64;
        for (line_no, line) in r.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: RecordedEvent =
                serde_json::from_str(&line).map_err(|e| WorkbookError::LogDecode {
                    line: line_no,
                    reason: e.to_string(),
                })?;
            if ev.seq != expected {
                return Err(WorkbookError::LogOutOfOrder);
            }
            expected += 1;
            // Aplica al sheet directo (sin invariantes — el evento
            // ya pasó la validación en su tiempo).
            apply_to_sheet(&mut wb.sheet, &ev.event)?;
            wb.events.push(ev);
        }
        wb.next_seq = expected;
        Ok(wb)
    }

    /// Time-travel: reconstruye la hoja como estaba después de
    /// procesar los primeros `n` eventos (`n=0` → hoja vacía;
    /// `n=events.len()` → hoja actual). El workbook actual no se
    /// modifica — devolvemos un `Sheet` snapshot.
    pub fn snapshot_at(&self, n: usize) -> Result<Sheet, WorkbookError> {
        let mut s = Sheet::new();
        for ev in self.events.iter().take(n) {
            apply_to_sheet(&mut s, &ev.event)?;
        }
        Ok(s)
    }
}

/// Aplica un `SheetEvent` directamente a un `Sheet`. Reusada por
/// `apply_user_event` (sobre el candidato) y por el replay del log.
fn apply_to_sheet(sheet: &mut Sheet, event: &SheetEvent) -> Result<SetReport, SetError> {
    match event {
        SheetEvent::SetCell { cell, raw } => sheet.set_cell(*cell, raw),
        SheetEvent::ClearCell { cell } => Ok(sheet.clear_cell(*cell)),
    }
}

struct SheetResolver<'a> {
    sheet: &'a Sheet,
}

impl<'a> CellResolver for SheetResolver<'a> {
    fn resolve(&self, cell: CellRef) -> SheetValue {
        self.sheet.value(cell)
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;
    use rust_decimal::Decimal;
    use std::io::Cursor;
    use std::str::FromStr;

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn events_record_in_order() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "1").unwrap();
        wb.set_cell(cr("B1"), "=A1+10").unwrap();
        wb.set_cell(cr("A1"), "5").unwrap();
        assert_eq!(wb.events().len(), 3);
        for (i, ev) in wb.events().iter().enumerate() {
            assert_eq!(ev.seq, i as u64);
        }
    }

    #[test]
    fn replay_reconstructs_state() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "10").unwrap();
        wb.set_cell(cr("B1"), "=A1*3").unwrap();
        wb.set_cell(cr("A1"), "7").unwrap();

        let mut buf = Vec::new();
        wb.write_log(&mut buf).unwrap();
        let wb2 = Workbook::from_log(Cursor::new(buf)).unwrap();
        assert_eq!(wb2.value(cr("A1")), SheetValue::Number(dec("7")));
        assert_eq!(wb2.value(cr("B1")), SheetValue::Number(dec("21")));
    }

    #[test]
    fn snapshot_at_walks_history() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "1").unwrap(); // seq 0
        wb.set_cell(cr("A1"), "2").unwrap(); // seq 1
        wb.set_cell(cr("A1"), "3").unwrap(); // seq 2

        assert_eq!(wb.snapshot_at(0).unwrap().value(cr("A1")), SheetValue::Empty);
        assert_eq!(
            wb.snapshot_at(1).unwrap().value(cr("A1")),
            SheetValue::Number(dec("1"))
        );
        assert_eq!(
            wb.snapshot_at(2).unwrap().value(cr("A1")),
            SheetValue::Number(dec("2"))
        );
        assert_eq!(
            wb.snapshot_at(3).unwrap().value(cr("A1")),
            SheetValue::Number(dec("3"))
        );
    }

    #[test]
    fn invariant_blocks_violating_edit() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "100").unwrap();
        // Regla: saldo (A1) jamás negativo.
        wb.add_invariant("saldo_no_negativo", "=A1>=0").unwrap();
        // Edición OK: A1 = 50.
        wb.set_cell(cr("A1"), "50").unwrap();
        assert_eq!(wb.value(cr("A1")), SheetValue::Number(dec("50")));
        // Edición prohibida: A1 = -10. Debe rechazarse.
        let err = wb.set_cell(cr("A1"), "-10").unwrap_err();
        assert!(matches!(err, WorkbookError::InvariantViolated { .. }));
        // El workbook quedó en el estado anterior intacto.
        assert_eq!(wb.value(cr("A1")), SheetValue::Number(dec("50")));
        // Y el evento NO se registró en el log.
        assert_eq!(wb.events().len(), 2);
    }

    #[test]
    fn invariant_evaluates_downstream_sum() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "10").unwrap();
        wb.set_cell(cr("A2"), "20").unwrap();
        wb.set_cell(cr("A3"), "30").unwrap();
        wb.set_cell(cr("B1"), "=SUM(A1:A3)").unwrap();
        // Regla: el total nunca > 100.
        wb.add_invariant("tope_total", "=B1<=100").unwrap();
        // Permitido: total 70.
        wb.set_cell(cr("A3"), "40").unwrap();
        assert_eq!(wb.value(cr("B1")), SheetValue::Number(dec("70")));
        // Prohibido: total 130.
        assert!(wb.set_cell(cr("A2"), "80").is_err());
        // El total sigue siendo 70.
        assert_eq!(wb.value(cr("B1")), SheetValue::Number(dec("70")));
    }

    #[test]
    fn cycle_error_propagates_through_workbook() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "=B1+1").unwrap();
        let err = wb.set_cell(cr("B1"), "=A1+1").unwrap_err();
        assert!(matches!(err, WorkbookError::Set(SetError::Cycle(_))));
    }

    #[test]
    fn out_of_order_log_rejected() {
        let mut wb = Workbook::new();
        wb.set_cell(cr("A1"), "1").unwrap();
        wb.set_cell(cr("A1"), "2").unwrap();
        // Manipulación maliciosa del log: invertimos los eventos.
        let mut wb2_events = wb.events().to_vec();
        wb2_events.reverse();
        let mut buf = Vec::new();
        for ev in &wb2_events {
            serde_json::to_writer(&mut buf, ev).unwrap();
            buf.push(b'\n');
        }
        let err = Workbook::from_log(Cursor::new(buf)).unwrap_err();
        assert!(matches!(err, WorkbookError::LogOutOfOrder));
    }
}
