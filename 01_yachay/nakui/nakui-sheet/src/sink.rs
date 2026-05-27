//! `EventSink` — abstracción del log de eventos del `Workbook`. La
//! decisión de a dónde van los `SheetEvent` (memoria, archivo,
//! SurrealDB, `nakui-core::event_log`) queda detrás de un trait, no
//! hardcoded.
//!
//! Implementaciones que ya viven aquí:
//!   - [`MemorySink`]: `Vec<RecordedEvent>` en memoria. Default del
//!     `Workbook::new()`; suficiente para pruebas y para apps de un
//!     solo proceso.
//!   - [`FileSink`]: append-only JSONL en disco. Cada evento se
//!     `fsync`-ea por defecto (configurable) — sobrevive un kill -9
//!     en medio de la sesión.
//!
//! Para integrar con `nakui-core::event_log` (drift detection
//! canonical, snapshots, replay con executor), implementa
//! `EventSink` mapeando cada `SheetEvent` al `LogEntry::Morphism`
//! del schema "sheet" (un morfismo `set_cell` con role-prefixed
//! writes a `Cell.raw`). El bridge está fuera de este crate porque
//! requiere depender de `nakui-core`; vive como un crate opcional
//! cuando se decida hacerlo.

use crate::workbook::{RecordedEvent, SheetEvent};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SinkError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("decode at line {line}: {reason}")]
    Decode { line: usize, reason: String },
    #[error("sequence skew: got {got}, expected {expected}")]
    Skew { got: u64, expected: u64 },
}

pub trait EventSink: Send {
    /// Persiste un nuevo evento. Devuelve el `seq` asignado.
    /// El sink es quien asigna el seq para garantizar
    /// monoticidad incluso si dos threads compiten (los sinks
    /// concurrentes tendrían que sincronizar internamente).
    fn record(&mut self, event: SheetEvent, timestamp_ms: u128) -> Result<u64, SinkError>;

    /// Próximo `seq` que se asignaría.
    fn next_seq(&self) -> u64;

    /// Snapshot de todos los eventos en orden de `seq`. Devuelve
    /// `Vec<RecordedEvent>` (clone) — más simple que un iterator
    /// con lifetime cruzando el trait object.
    fn events(&self) -> Vec<RecordedEvent>;

    fn len(&self) -> usize {
        self.events().len()
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Sink en memoria. Default de `Workbook::new()`.
#[derive(Debug, Default, Clone)]
pub struct MemorySink {
    events: Vec<RecordedEvent>,
    next_seq: u64,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }

    /// Carga eventos desde un reader JSONL (un evento por línea).
    /// Verifica monotonía estricta de `seq` empezando en 0.
    pub fn from_reader<R: BufRead>(r: R) -> Result<Self, SinkError> {
        let mut sink = Self::default();
        for (line_no, line) in r.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let ev: RecordedEvent =
                serde_json::from_str(&line).map_err(|e| SinkError::Decode {
                    line: line_no,
                    reason: e.to_string(),
                })?;
            if ev.seq != sink.next_seq {
                return Err(SinkError::Skew {
                    got: ev.seq,
                    expected: sink.next_seq,
                });
            }
            sink.next_seq += 1;
            sink.events.push(ev);
        }
        Ok(sink)
    }
}

impl EventSink for MemorySink {
    fn record(&mut self, event: SheetEvent, timestamp_ms: u128) -> Result<u64, SinkError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push(RecordedEvent {
            seq,
            timestamp_ms,
            event,
        });
        Ok(seq)
    }

    fn next_seq(&self) -> u64 {
        self.next_seq
    }

    fn events(&self) -> Vec<RecordedEvent> {
        self.events.clone()
    }
}

/// Sink append-only sobre un archivo JSONL. Cada `record` escribe
/// una línea y opcionalmente hace `fsync` (`durable = true`, default)
/// para que el evento sobreviva un crash.
///
/// La carga del archivo existente al construir es lectura completa
/// — adecuada para hojas de cálculo (decenas de miles de eventos en
/// el peor caso). Para escenarios mucho más grandes habría que
/// indexar; queda fuera del scope actual.
pub struct FileSink {
    cache: Vec<RecordedEvent>,
    writer: BufWriter<File>,
    next_seq: u64,
    durable: bool,
}

impl FileSink {
    /// Abre el archivo, leyendo cualquier evento ya presente. Crea
    /// el archivo si no existe. El cursor de escritura queda al
    /// final.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SinkError> {
        let path = path.as_ref();
        // 1. Leer eventos existentes.
        let cache = if path.exists() {
            let f = File::open(path)?;
            let mem = MemorySink::from_reader(BufReader::new(f))?;
            mem.events
        } else {
            Vec::new()
        };
        let next_seq = cache.last().map(|e| e.seq + 1).unwrap_or(0);
        // 2. Abrir para append.
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            cache,
            writer: BufWriter::new(f),
            next_seq,
            durable: true,
        })
    }

    /// Si `durable=false`, el sink no fuerza fsync tras cada
    /// record. Aumenta throughput a cambio de poder perder los
    /// últimos eventos en un kill -9. Útil para benchmarks o para
    /// escenarios donde la durabilidad la garantiza el filesystem
    /// (ZFS, btrfs con sync mounts).
    pub fn set_durable(&mut self, durable: bool) {
        self.durable = durable;
    }
}

impl EventSink for FileSink {
    fn record(&mut self, event: SheetEvent, timestamp_ms: u128) -> Result<u64, SinkError> {
        let seq = self.next_seq;
        let entry = RecordedEvent {
            seq,
            timestamp_ms,
            event,
        };
        // Serializa, agrega newline, flushea, opcional fsync.
        serde_json::to_writer(&mut self.writer, &entry).map_err(|e| SinkError::Decode {
            line: seq as usize,
            reason: e.to_string(),
        })?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        if self.durable {
            self.writer.get_ref().sync_data()?;
        }
        self.next_seq += 1;
        self.cache.push(entry);
        Ok(seq)
    }

    fn next_seq(&self) -> u64 {
        self.next_seq
    }

    fn events(&self) -> Vec<RecordedEvent> {
        self.cache.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }

    #[test]
    fn memory_sink_assigns_monotonic_seq() {
        let mut s = MemorySink::new();
        let a = s
            .record(
                SheetEvent::SetCell {
                    cell: cr("A1"),
                    raw: "1".into(),
                },
                1000,
            )
            .unwrap();
        let b = s
            .record(
                SheetEvent::SetCell {
                    cell: cr("A2"),
                    raw: "2".into(),
                },
                1001,
            )
            .unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(s.next_seq(), 2);
        assert_eq!(s.events().len(), 2);
    }

    #[test]
    fn file_sink_round_trip_through_disk() {
        let tmp = tempfile_path();
        // Sesión 1: escribir.
        {
            let mut s = FileSink::open(&tmp).unwrap();
            s.record(
                SheetEvent::SetCell {
                    cell: cr("A1"),
                    raw: "100".into(),
                },
                1000,
            )
            .unwrap();
            s.record(
                SheetEvent::ClearCell { cell: cr("A1") },
                1001,
            )
            .unwrap();
        }
        // Sesión 2: leer.
        let s2 = FileSink::open(&tmp).unwrap();
        assert_eq!(s2.events().len(), 2);
        assert_eq!(s2.next_seq(), 2);
        // Cleanup.
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn file_sink_continues_seq_after_reopen() {
        let tmp = tempfile_path();
        {
            let mut s = FileSink::open(&tmp).unwrap();
            s.record(
                SheetEvent::SetCell {
                    cell: cr("A1"),
                    raw: "1".into(),
                },
                1000,
            )
            .unwrap();
        }
        {
            let mut s = FileSink::open(&tmp).unwrap();
            let new_seq = s
                .record(
                    SheetEvent::SetCell {
                        cell: cr("A2"),
                        raw: "2".into(),
                    },
                    1001,
                )
                .unwrap();
            assert_eq!(new_seq, 1, "el seq debe continuar desde donde quedó");
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn memory_sink_rejects_out_of_order_load() {
        // Construyo manualmente un JSONL con seq fuera de orden.
        let bad = r#"{"seq":1,"timestamp_ms":1,"event":{"op":"set_cell","cell":{"col":0,"row":0,"col_absolute":false,"row_absolute":false},"raw":"x"}}
"#;
        let err = MemorySink::from_reader(bad.as_bytes()).unwrap_err();
        assert!(matches!(err, SinkError::Skew { .. }));
    }

    /// Devuelve un path de archivo temporal único (suficiente para
    /// tests; no usamos `tempfile` para no agregar otra dep).
    fn tempfile_path() -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        p.push(format!("nakui-sheet-sink-{pid}-{nanos}.jsonl"));
        p
    }
}
