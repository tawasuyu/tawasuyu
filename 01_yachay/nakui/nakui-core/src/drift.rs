//! Drift detection: compare two snapshots of store state and surface
//! the records that differ.
//!
//! "Drift" here means the live store has departed from what the log can
//! reproduce. The `Store::hash_state` contract makes the binary check
//! cheap (32 bytes); when those disagree, `compare_states` walks both
//! enumerations and produces a diff list the operator can act on.
//!
//! No IO in this module. The wire bits (asking a `nakui run` server for
//! its hash and records) live in the CLI; this is the pure comparison
//! used by both the CLI and any future automated drift-watcher.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

use crate::event_log::{EventLog, replay};
use crate::store::Store;

/// A single record-level difference between two snapshots. Variants are
/// labeled from the perspective of the operator running the check: the
/// "log" side is the canonical state (what the log replays to), the
/// "server" side is the live state being audited.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriftDiff {
    /// Server has a record the log doesn't know about. Phantom data —
    /// either an out-of-band write, or a successful op that never
    /// reached the WAL (which would itself be a kernel bug).
    OnlyOnServer {
        entity: String,
        id: Uuid,
        value: Value,
    },
    /// Log expects a record the server lost. Either the server's apply
    /// rolled back without a reconcile, or someone deleted a record
    /// out-of-band.
    OnlyInLog {
        entity: String,
        id: Uuid,
        value: Value,
    },
    /// Same (entity, id) on both sides but the values differ — the most
    /// dangerous case, because it means a logged event was overwritten
    /// or a field was tampered with.
    Tampered {
        entity: String,
        id: Uuid,
        log_value: Value,
        server_value: Value,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct DriftReport {
    pub log_hash: [u8; 32],
    pub server_hash: [u8; 32],
    pub log_records: usize,
    pub server_records: usize,
    /// Empty iff the two snapshots are byte-identical. Sorted by
    /// (entity, id_bytes) so two runs against the same drift produce
    /// the same report.
    pub diffs: Vec<DriftDiff>,
}

impl DriftReport {
    pub fn in_sync(&self) -> bool {
        self.log_hash == self.server_hash && self.diffs.is_empty()
    }
}

/// Pure comparison: take two canonical-order enumerations (as returned
/// by `Store::iter`) plus their hashes, and return the diff list.
///
/// Inputs need not be pre-sorted — we re-key by (entity, id) and walk
/// the union — but if the iterators were produced via `Store::iter`,
/// they're already in canonical order and the report's `diffs` will be
/// emitted in that same order.
pub fn compare_states(
    log_records: Vec<(String, Uuid, Value)>,
    log_hash: [u8; 32],
    server_records: Vec<(String, Uuid, Value)>,
    server_hash: [u8; 32],
) -> DriftReport {
    let log_count = log_records.len();
    let server_count = server_records.len();

    let mut log_map: HashMap<(String, Uuid), Value> = log_records
        .into_iter()
        .map(|(e, id, v)| ((e, id), v))
        .collect();
    let server_map: HashMap<(String, Uuid), Value> = server_records
        .into_iter()
        .map(|(e, id, v)| ((e, id), v))
        .collect();

    let mut diffs: Vec<DriftDiff> = Vec::new();
    for ((entity, id), server_value) in &server_map {
        match log_map.remove(&(entity.clone(), *id)) {
            None => diffs.push(DriftDiff::OnlyOnServer {
                entity: entity.clone(),
                id: *id,
                value: server_value.clone(),
            }),
            Some(log_value) => {
                if log_value != *server_value {
                    diffs.push(DriftDiff::Tampered {
                        entity: entity.clone(),
                        id: *id,
                        log_value,
                        server_value: server_value.clone(),
                    });
                }
            }
        }
    }
    // Whatever is left in log_map is missing on the server.
    for ((entity, id), value) in log_map {
        diffs.push(DriftDiff::OnlyInLog { entity, id, value });
    }

    // Canonical sort: (entity, id_bytes), then by variant kind so
    // diff ordering is fully deterministic even when the same key
    // appears (which it can't here, but defensively).
    diffs.sort_by(|a, b| {
        let (ea, ia) = key(a);
        let (eb, ib) = key(b);
        ea.cmp(eb)
            .then_with(|| ia.as_bytes().cmp(ib.as_bytes()))
            .then_with(|| variant_order(a).cmp(&variant_order(b)))
    });

    DriftReport {
        log_hash,
        server_hash,
        log_records: log_count,
        server_records: server_count,
        diffs,
    }
}

fn key(d: &DriftDiff) -> (&str, &Uuid) {
    match d {
        DriftDiff::OnlyOnServer { entity, id, .. }
        | DriftDiff::OnlyInLog { entity, id, .. }
        | DriftDiff::Tampered { entity, id, .. } => (entity.as_str(), id),
    }
}

fn variant_order(d: &DriftDiff) -> u8 {
    match d {
        DriftDiff::OnlyInLog { .. } => 0,
        DriftDiff::Tampered { .. } => 1,
        DriftDiff::OnlyOnServer { .. } => 2,
    }
}

#[derive(Debug, Error)]
pub enum DriftError {
    #[error("open log: {0}")]
    Log(#[from] crate::event_log::LogError),
    #[error("replay log: {0}")]
    Replay(#[from] crate::event_log::ReplayError),
    #[error("store: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("connect to server socket: {0}")]
    Connect(#[source] std::io::Error),
    #[error("server io: {0}")]
    Io(#[from] std::io::Error),
    #[error("server response not json: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("server returned error for `{op}`: {msg}")]
    Server { op: String, msg: String },
    #[error("server response missing field `{field}` for op `{op}`")]
    MissingField { op: String, field: String },
    #[error("server hash `{0}` is not 32 hex bytes")]
    BadHash(String),
}

/// Audit a live `nakui run` server against the canonical state derived
/// from a log file.
///
/// Cheap path: ask the server for `hash_state`, replay the log locally,
/// hash that. If the hashes match, we return immediately with an empty
/// diff list — no large `dump_records` round-trip.
///
/// Expensive path: hashes differ. Pull the full record dump from the
/// server, run `compare_states`, return the structured report.
pub fn check_against_socket(
    log_path: &Path,
    socket_path: &Path,
) -> Result<DriftReport, DriftError> {
    // Local: replay log → MemoryStore, snapshot.
    let log = EventLog::open(log_path)?;
    let local_store = replay(&log)?;
    let local_records: Vec<(String, Uuid, Value)> = local_store.iter()?.collect();
    let local_hash = local_store.hash_state()?;

    // Wire: open the connection once and reuse it for both requests.
    let stream = UnixStream::connect(socket_path).map_err(DriftError::Connect)?;
    let mut conn = SocketClient::new(stream)?;

    // Cheap path.
    let hash_resp = conn.exchange(serde_json::json!({"op": "hash_state"}))?;
    require_ok(&hash_resp, "hash_state")?;
    let server_hash = parse_hash(&hash_resp, "hash_state")?;
    let server_count = hash_resp
        .get("records")
        .and_then(Value::as_u64)
        .ok_or_else(|| DriftError::MissingField {
            op: "hash_state".into(),
            field: "records".into(),
        })? as usize;

    if server_hash == local_hash {
        return Ok(DriftReport {
            log_hash: local_hash,
            server_hash,
            log_records: local_records.len(),
            server_records: server_count,
            diffs: Vec::new(),
        });
    }

    // Expensive path: pull the full server snapshot.
    let dump_resp = conn.exchange(serde_json::json!({"op": "dump_records"}))?;
    require_ok(&dump_resp, "dump_records")?;
    let server_records = parse_records(&dump_resp)?;

    Ok(compare_states(
        local_records,
        local_hash,
        server_records,
        server_hash,
    ))
}

struct SocketClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl SocketClient {
    fn new(stream: UnixStream) -> Result<Self, DriftError> {
        let reader_stream = stream.try_clone()?;
        Ok(Self {
            writer: stream,
            reader: BufReader::new(reader_stream),
        })
    }

    fn exchange(&mut self, req: Value) -> Result<Value, DriftError> {
        let mut bytes = serde_json::to_vec(&req).expect("request serializes");
        bytes.push(b'\n');
        self.writer.write_all(&bytes)?;
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Err(DriftError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "server closed connection without responding",
            )));
        }
        Ok(serde_json::from_str(line.trim())?)
    }
}

fn require_ok(resp: &Value, op: &str) -> Result<(), DriftError> {
    if resp.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(DriftError::Server {
            op: op.into(),
            msg: resp
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("(no error message)")
                .to_string(),
        })
    }
}

fn parse_hash(resp: &Value, op: &str) -> Result<[u8; 32], DriftError> {
    let s = resp
        .get("hash")
        .and_then(Value::as_str)
        .ok_or_else(|| DriftError::MissingField {
            op: op.into(),
            field: "hash".into(),
        })?;
    if s.len() != 64 {
        return Err(DriftError::BadHash(s.into()));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let hi = hex_nibble(s.as_bytes()[i * 2]).ok_or_else(|| DriftError::BadHash(s.into()))?;
        let lo =
            hex_nibble(s.as_bytes()[i * 2 + 1]).ok_or_else(|| DriftError::BadHash(s.into()))?;
        *byte = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn parse_records(resp: &Value) -> Result<Vec<(String, Uuid, Value)>, DriftError> {
    let arr = resp
        .get("records")
        .and_then(Value::as_array)
        .ok_or_else(|| DriftError::MissingField {
            op: "dump_records".into(),
            field: "records".into(),
        })?;
    let mut out: Vec<(String, Uuid, Value)> = Vec::with_capacity(arr.len());
    for item in arr {
        let entity = item
            .get("entity")
            .and_then(Value::as_str)
            .ok_or_else(|| DriftError::MissingField {
                op: "dump_records".into(),
                field: "records[].entity".into(),
            })?
            .to_string();
        let id_str = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| DriftError::MissingField {
                op: "dump_records".into(),
                field: "records[].id".into(),
            })?;
        let id = Uuid::parse_str(id_str).map_err(|_| DriftError::MissingField {
            op: "dump_records".into(),
            field: format!("records[].id (not uuid: {})", id_str),
        })?;
        let value = item
            .get("value")
            .cloned()
            .ok_or_else(|| DriftError::MissingField {
                op: "dump_records".into(),
                field: "records[].value".into(),
            })?;
        out.push((entity, id, value));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn h(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn empty_inputs_yield_no_diffs() {
        let report = compare_states(Vec::new(), h(0), Vec::new(), h(0));
        assert!(report.in_sync());
        assert!(report.diffs.is_empty());
    }

    #[test]
    fn equal_records_yield_no_diffs_even_if_hashes_were_lied_to() {
        // The function compares records, not hashes — hash equality is
        // the operator's fast-path, but the report's truth is the diffs.
        let a = Uuid::new_v4();
        let log = vec![(
            "Caja".to_string(),
            a,
            json!({"saldo": 100}),
        )];
        let server = vec![(
            "Caja".to_string(),
            a,
            json!({"saldo": 100}),
        )];
        let report = compare_states(log, h(1), server, h(2));
        assert!(report.diffs.is_empty(), "records equal → no diffs");
    }

    #[test]
    fn detects_only_on_server() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let log = vec![(
            "Caja".to_string(),
            a,
            json!({"saldo": 100}),
        )];
        let server = vec![
            ("Caja".to_string(), a, json!({"saldo": 100})),
            ("Caja".to_string(), b, json!({"saldo": 999})),
        ];
        let report = compare_states(log, h(0), server, h(1));
        assert_eq!(report.diffs.len(), 1);
        match &report.diffs[0] {
            DriftDiff::OnlyOnServer { entity, id, .. } => {
                assert_eq!(entity, "Caja");
                assert_eq!(*id, b);
            }
            other => panic!("expected OnlyOnServer, got {:?}", other),
        }
    }

    #[test]
    fn detects_only_in_log() {
        let a = Uuid::new_v4();
        let log = vec![("Caja".to_string(), a, json!({"saldo": 100}))];
        let server = vec![];
        let report = compare_states(log, h(0), server, h(1));
        assert_eq!(report.diffs.len(), 1);
        match &report.diffs[0] {
            DriftDiff::OnlyInLog { id, .. } => assert_eq!(*id, a),
            other => panic!("expected OnlyInLog, got {:?}", other),
        }
    }

    #[test]
    fn detects_tampered() {
        let a = Uuid::new_v4();
        let log = vec![("Caja".to_string(), a, json!({"saldo": 100}))];
        let server = vec![("Caja".to_string(), a, json!({"saldo": 999}))];
        let report = compare_states(log, h(0), server, h(1));
        assert_eq!(report.diffs.len(), 1);
        match &report.diffs[0] {
            DriftDiff::Tampered {
                id,
                log_value,
                server_value,
                ..
            } => {
                assert_eq!(*id, a);
                assert_eq!(log_value["saldo"], json!(100));
                assert_eq!(server_value["saldo"], json!(999));
            }
            other => panic!("expected Tampered, got {:?}", other),
        }
    }

    #[test]
    fn diffs_emerge_in_canonical_order() {
        // Two entities, mixed drift kinds. Result must be sorted by
        // (entity, id_bytes) so two runs produce the same report.
        let id_caja = Uuid::nil(); // sorts first byte-wise
        let id_mov = Uuid::from_u128(u128::MAX);

        let log = vec![
            ("Movimiento".to_string(), id_mov, json!({"x": 1})),
        ];
        let server = vec![
            ("Caja".to_string(), id_caja, json!({"saldo": 0})),
        ];
        let report = compare_states(log, h(0), server, h(1));
        assert_eq!(report.diffs.len(), 2);
        // Caja sorts before Movimiento.
        match (&report.diffs[0], &report.diffs[1]) {
            (DriftDiff::OnlyOnServer { entity: e1, .. }, DriftDiff::OnlyInLog { entity: e2, .. }) => {
                assert_eq!(e1, "Caja");
                assert_eq!(e2, "Movimiento");
            }
            other => panic!("unexpected order: {:?}", other),
        }
    }

    #[test]
    fn in_sync_requires_both_hashes_and_no_diffs() {
        // Defensive: if hashes match but somehow diffs is non-empty
        // (caller mismatch), in_sync says no.
        let report = DriftReport {
            log_hash: h(0),
            server_hash: h(0),
            log_records: 1,
            server_records: 1,
            diffs: vec![DriftDiff::Tampered {
                entity: "x".into(),
                id: Uuid::nil(),
                log_value: json!(1),
                server_value: json!(2),
            }],
        };
        assert!(!report.in_sync());
    }
}
