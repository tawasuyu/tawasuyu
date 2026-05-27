//! Bridge `nakui-sheet` ↔ `nakui-core::event_log`.
//!
//! `NakuiCoreSink` implementa el trait `EventSink` de `nakui-sheet`
//! usando `EventLog` de `nakui-core` como almacén append-only. Cada
//! `SheetEvent` se materializa como `LogEntry::Seed`:
//!
//! ```text
//!     Seed {
//!         seq:    next_seq,
//!         entity: "SheetEvent",
//!         id:     <uuid determinista del evento>,
//!         data:   serde_json::to_value(event),
//!     }
//! ```
//!
//! Por qué `Seed` y no `Morphism`:
//!   - `Morphism` exige `morphism: String` + `inputs: BTreeMap<...>` +
//!     `ops: Vec<FieldOp>` — toda la maquinaria de morfismos
//!     canonical del manifest. Sería forzar el grafo de Nakui a un
//!     dominio que en realidad no usa morfismos Rhai (la lógica de
//!     cascada vive dentro de `nakui-sheet::Sheet`, no en
//!     `nakui-core::Executor`).
//!   - `Seed` es justo lo que necesitamos: un evento opaco con `data:
//!     Value`, sin ops y sin executor. Nos da la durabilidad
//!     (`sync_all` en cada append → WAL fence) y el `verify_log`
//!     contra drift, sin pagar el costo de un morfismo simulado.
//!
//! El día que `nakui-sheet` quiera correr SUS reglas como morfismos
//! Nakui (invariantes en KCL/Nickel, executor con dry-run formal),
//! se puede reescribir este sink para emitir `LogEntry::Morphism`.
//! El bridge no se rompería del lado del Workbook — solo cambia la
//! forma persistida en disco.

use nakui_core::event_log::{EventLog, LogEntry, LogError};
use nakui_sheet::sink::{EventSink, SinkError};
use nakui_sheet::workbook::{RecordedEvent, SheetEvent};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("nakui-core event log: {0}")]
    Log(#[from] LogError),
    #[error("decode SheetEvent from LogEntry: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("found non-SheetEvent entry in log: entity=`{entity}`")]
    UnexpectedEntity { entity: String },
}

pub const SHEET_EVENT_ENTITY: &str = "SheetEvent";

pub struct NakuiCoreSink {
    log: EventLog,
    cache: Vec<RecordedEvent>,
}

impl NakuiCoreSink {
    /// Abre el log en `path`. Lee las entradas existentes y las
    /// reconstruye como `RecordedEvent` listos para consumir desde
    /// el Workbook. Cada entrada debe ser un `Seed { entity:
    /// "SheetEvent", ... }`; si encuentra cualquier otro tipo de
    /// entrada (un `Morphism` huérfano, p.ej.), aborta con
    /// `BridgeError::UnexpectedEntity`.
    pub fn open(path: impl Into<std::path::PathBuf>) -> Result<Self, BridgeError> {
        let log = EventLog::open(path)?;
        let entries = log.entries()?;
        let mut cache = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                LogEntry::Seed {
                    seq, entity, data, ..
                } => {
                    if entity != SHEET_EVENT_ENTITY {
                        return Err(BridgeError::UnexpectedEntity { entity });
                    }
                    let event: SheetEvent = serde_json::from_value(data)?;
                    // El `timestamp_ms` no viaja en el LogEntry de
                    // nakui-core — ahí es responsabilidad del WAL
                    // saber el momento físico via mtime del archivo,
                    // no de cada entrada. Reportamos 0 para el
                    // round-trip; quien necesite timestamps debería
                    // usar `MemorySink`/`FileSink` que sí los
                    // preservan.
                    cache.push(RecordedEvent {
                        seq,
                        timestamp_ms: 0,
                        event,
                    });
                }
                LogEntry::Morphism { .. } => {
                    return Err(BridgeError::UnexpectedEntity {
                        entity: "Morphism".to_string(),
                    });
                }
            }
        }
        Ok(Self { log, cache })
    }

    /// Acceso al `EventLog` subyacente — útil para llamar
    /// `verify_log`, `replay`, etc. directo desde nakui-core.
    pub fn log(&self) -> &EventLog {
        &self.log
    }
}

impl EventSink for NakuiCoreSink {
    fn record(
        &mut self,
        event: SheetEvent,
        _timestamp_ms: u128,
    ) -> Result<u64, SinkError> {
        let seq = self.log.next_seq();
        // UUID determinista a partir del seq — útil para que el
        // mismo evento aplicado dos veces (replay → re-replay)
        // produzca exactamente el mismo `LogEntry`. v4 random
        // rompería el hash de `verify_log`.
        let id = uuid_from_seq(seq);
        let data = serde_json::to_value(&event).map_err(|e| SinkError::Decode {
            line: seq as usize,
            reason: e.to_string(),
        })?;
        let entry = LogEntry::Seed {
            seq,
            entity: SHEET_EVENT_ENTITY.to_string(),
            id,
            data,
            schema_hash: None,
        };
        self.log
            .append(entry)
            .map_err(|e| SinkError::Decode {
                line: seq as usize,
                reason: e.to_string(),
            })?;
        self.cache.push(RecordedEvent {
            seq,
            timestamp_ms: 0,
            event,
        });
        Ok(seq)
    }

    fn next_seq(&self) -> u64 {
        self.log.next_seq()
    }

    fn events(&self) -> Vec<RecordedEvent> {
        self.cache.clone()
    }
}

/// UUID derivado del `seq`: primeros 8 bytes = seq (big-endian),
/// resto a cero, version 4 set. No es random — la idea es que el
/// mismo seq SIEMPRE produzca el mismo UUID, lo cual hace el log
/// reproducible byte-by-byte y compatible con drift detection.
fn uuid_from_seq(seq: u64) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&seq.to_be_bytes());
    // Version 4 marker (UUID variant DCE 1.1, version 4).
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nakui_sheet::cell::CellRef;
    use nakui_sheet::value::SheetValue;
    use nakui_sheet::Workbook;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn tmp_path(label: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("nakui-sheet-nakuicore-{label}-{pid}-{nanos}.log"));
        p
    }

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }
    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn round_trip_through_nakui_core_log() {
        let p = tmp_path("roundtrip");
        // Sesión 1: escribir vía Workbook + NakuiCoreSink.
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            let mut wb = Workbook::with_sink(sink).unwrap();
            wb.set_cell(cr("A1"), "10").unwrap();
            wb.set_cell(cr("B1"), "=A1*5").unwrap();
            wb.set_cell(cr("A1"), "7").unwrap();
            assert_eq!(wb.value(cr("B1")), SheetValue::Number(dec("35")));
        }
        // Sesión 2: abrir el mismo log y verificar replay.
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            let wb = Workbook::with_sink(sink).unwrap();
            assert_eq!(wb.value(cr("A1")), SheetValue::Number(dec("7")));
            assert_eq!(wb.value(cr("B1")), SheetValue::Number(dec("35")));
            assert_eq!(wb.events().len(), 3);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn seq_is_monotonic_across_sessions() {
        let p = tmp_path("monotonic");
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            let mut wb = Workbook::with_sink(sink).unwrap();
            wb.set_cell(cr("A1"), "1").unwrap();
        }
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            assert_eq!(sink.next_seq(), 1);
            let mut wb = Workbook::with_sink(sink).unwrap();
            wb.set_cell(cr("A2"), "2").unwrap();
            // El segundo evento debe llevar seq=1 (continúa, no
            // reinicia).
            let evs = wb.events();
            assert_eq!(evs.last().unwrap().seq, 1);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn fill_event_persists_as_single_entry() {
        let p = tmp_path("fill");
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            let mut wb = Workbook::with_sink(sink).unwrap();
            wb.set_cell(cr("A1"), "5").unwrap();
            wb.set_cell(cr("A2"), "10").unwrap();
            wb.set_cell(cr("B1"), "=A1*2").unwrap();
            // Fill: un solo evento en el log, no N events.
            wb.fill(cr("B1"), "B1:B2".parse().unwrap()).unwrap();
            assert_eq!(wb.events().len(), 4); // 3 set_cell + 1 fill
        }
        {
            let sink = Box::new(NakuiCoreSink::open(&p).unwrap());
            let wb = Workbook::with_sink(sink).unwrap();
            assert_eq!(wb.events().len(), 4);
            // El estado reconstruido tiene el fill aplicado.
            assert_eq!(wb.value(cr("B1")), SheetValue::Number(dec("10")));
            assert_eq!(wb.value(cr("B2")), SheetValue::Number(dec("20")));
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn deterministic_uuid_for_same_seq() {
        // Misma seq → mismo UUID (importante para reproducibilidad
        // del log byte-by-byte y para drift detection).
        let a = uuid_from_seq(42);
        let b = uuid_from_seq(42);
        assert_eq!(a, b);
        let c = uuid_from_seq(43);
        assert_ne!(a, c);
    }
}
