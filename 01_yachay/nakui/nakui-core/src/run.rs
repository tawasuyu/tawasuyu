//! `nakui run` server: a long-lived process that holds an in-memory store
//! reconstructed from the log, exposes a Unix Domain Socket, and serves
//! line-delimited JSON requests to drive the kernel.
//!
//! Why UDS + line-JSON for V1:
//!   - Multi-client without committing to a transport (HTTP/NATS later).
//!   - Filesystem permissions gate access; no port exposure.
//!   - Self-describing: `describe` returns the manifest's morphism specs
//!     so an agent (human or LLM) can drive the server without external
//!     docs.
//!
//! Concurrency: one connection at a time. Backed by `&mut Store`, the
//! kernel is single-writer by design. Multiple clients queue in
//! `accept()`. If/when we want concurrency, the right unit to parallelize
//! is reads, not writes — that's a future refactor with locks at the
//! right granularity.
//!
//! Recovery: every `execute` goes through `execute_and_log_with_recovery`
//! so a transient apply failure auto-rebuilds the in-memory store from
//! the log without taking the server down.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use crate::event_log::{
    EventLog, RecoverableExecuteError, ReplayError, Snapshot, SnapshotMismatchError,
    execute_and_log_with_recovery, replay_with_snapshot_into, verify_log,
};
use crate::executor::Executor;
use crate::store::Store;

#[derive(Debug, Error)]
pub enum RunError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("clear store on startup: {0}")]
    Clear(#[source] crate::store::StoreError),
    #[error("replay on startup: {0}")]
    Replay(#[from] ReplayError),
    #[error("log: {0}")]
    Log(#[from] crate::event_log::LogError),
    #[error("snapshot incompatible: {0}")]
    SnapshotMismatch(#[from] SnapshotMismatchError),
    #[error(
        "snapshot/log gap: snapshot covers up to seq {snap_seq}, log's first remaining entry is seq {log_first_seq} (expected ≤ {expected})"
    )]
    SnapshotGap {
        snap_seq: u64,
        log_first_seq: u64,
        expected: u64,
    },
}

/// Run the server until a `shutdown` request is received or `accept`
/// returns an unrecoverable error. On exit, removes the socket file.
///
/// Startup reconstruction:
///   - With `Some(snapshot)`: validate its `schema_hash` against the
///     executor, seed the store from the snapshot, replay only the log
///     tail (entries with `seq > snapshot.seq`).
///   - With `None`: full replay from seq 0. Slower for long logs.
///
/// In both cases the store is wiped first, so the server never serves
/// requests against a state the log can't reproduce. This is true for
/// `MemoryStore` and for persistent backends like `SurrealStore` —
/// persistence is a durability property of the runtime cache, not a
/// way to skip replay. (A future "skip replay if last_applied_seq
/// matches" optimization would change that.)
pub fn run_server<S: Store>(
    executor: Executor,
    mut log: EventLog,
    mut store: S,
    snapshot: Option<Snapshot>,
    socket_path: &Path,
) -> Result<(), RunError> {
    startup_replay(&executor, &log, &mut store, snapshot.as_ref())?;

    // Best-effort cleanup of stale sockets from a prior crashed run.
    // Bind itself will fail if a live process is already listening.
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;

    let result = accept_loop(&listener, &executor, &mut store, &mut log);
    let _ = std::fs::remove_file(socket_path);
    result
}

fn startup_replay<S: Store>(
    executor: &Executor,
    log: &EventLog,
    store: &mut S,
    snapshot: Option<&Snapshot>,
) -> Result<(), RunError> {
    // Snapshot validation runs first (cheap) so a bad snapshot is caught
    // even when we'd otherwise take the skip-replay fast path.
    if let Some(snap) = snapshot {
        snap.ensure_compatible_with(executor)?;
        let entries = log.entries()?;
        if let Some(first) = entries.first() {
            let expected = snap.seq.saturating_add(1);
            if first.seq() > expected {
                return Err(RunError::SnapshotGap {
                    snap_seq: snap.seq,
                    log_first_seq: first.seq(),
                    expected,
                });
            }
        }
    }

    // Fast path: persistent stores carry a `last_applied_seq` marker;
    // when it matches the log's last seq, the store is verifiably in
    // sync and we can skip the clear+replay entirely. Failures here
    // (e.g. backend can't read meta) just fall through to full replay
    // — never a correctness issue.
    let log_last_seq = log.entries()?.last().map(|e| e.seq());
    if let Ok(applied) = store.last_applied_seq() {
        if applied == log_last_seq && applied.is_some() {
            return Ok(());
        }
    }

    store.clear().map_err(RunError::Clear)?;
    replay_with_snapshot_into(log, snapshot, store)?;
    Ok(())
}

fn accept_loop<S: Store>(
    listener: &UnixListener,
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
) -> Result<(), RunError> {
    loop {
        let (stream, _addr) = listener.accept()?;
        let shutdown = handle_connection(stream, executor, store, log);
        if shutdown {
            return Ok(());
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Request {
    Execute {
        morphism: String,
        #[serde(default)]
        inputs: std::collections::BTreeMap<String, Uuid>,
        #[serde(default)]
        params: Value,
    },
    Load {
        entity: String,
        id: Uuid,
    },
    Describe,
    Verify,
    /// Return the SHA-256 of the live store's full state plus a record
    /// count. Used by the drift detector as the cheap fast-path check
    /// before asking for the full record dump.
    HashState,
    /// Return every record on the server in canonical order. Used after
    /// a hash mismatch to compute the per-record diff. Response can be
    /// large — the operator opts into it.
    DumpRecords,
    Shutdown,
}

/// Process one connection. Returns `true` if the client requested
/// shutdown — the caller should stop the accept loop after the response
/// has been flushed.
///
/// IO errors on a single connection don't kill the server: we log to
/// stderr and move on. Only a request-level shutdown ends the loop.
fn handle_connection<S: Store>(
    stream: UnixStream,
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
) -> bool {
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nakui run: clone stream: {}", e);
            return false;
        }
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("nakui run: read: {}", e);
                return false;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let (response, shutdown) = dispatch(&line, executor, store, log);
        let bytes = serde_json::to_vec(&response).expect("response serializes");
        if let Err(e) = writer.write_all(&bytes).and_then(|_| writer.write_all(b"\n")) {
            eprintln!("nakui run: write: {}", e);
            return false;
        }
        if shutdown {
            let _ = writer.flush();
            return true;
        }
    }
    false
}

fn dispatch<S: Store>(
    line: &str,
    executor: &Executor,
    store: &mut S,
    log: &mut EventLog,
) -> (Value, bool) {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return (error_response(&format!("bad request: {}", e)), false),
    };
    match req {
        Request::Execute {
            morphism,
            inputs,
            params,
        } => {
            let inputs_vec: Vec<(&str, Uuid)> =
                inputs.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            match execute_and_log_with_recovery(
                executor,
                store,
                log,
                &morphism,
                &inputs_vec,
                params,
            ) {
                Ok(ops) => (
                    json!({
                        "ok": true,
                        "seq": log.next_seq().saturating_sub(1),
                        "ops": ops,
                        "schema_hash": executor.schema_hash(&morphism).map(|h| hex_encode(&h)),
                    }),
                    false,
                ),
                Err(RecoverableExecuteError::PreLog(e)) => (
                    json!({"ok": false, "stage": "pre_log", "error": e.to_string()}),
                    false,
                ),
                Err(RecoverableExecuteError::LogAppend(e)) => (
                    json!({"ok": false, "stage": "log_append", "error": e.to_string()}),
                    false,
                ),
                Err(e @ RecoverableExecuteError::Unrecoverable { .. }) => (
                    json!({"ok": false, "stage": "unrecoverable", "error": e.to_string()}),
                    false,
                ),
            }
        }
        Request::Load { entity, id } => {
            let value = store.load(&entity, id);
            (json!({"ok": true, "value": value}), false)
        }
        Request::Describe => {
            let hashes: std::collections::BTreeMap<String, String> = executor
                .schema_hashes
                .iter()
                .map(|(k, v)| (k.clone(), hex_encode(v)))
                .collect();
            (
                json!({
                    "ok": true,
                    "protocol": 1,
                    "module": executor.manifest.module,
                    "schemas": executor.manifest.effective_schemas(),
                    "morphisms": executor.manifest.morphisms,
                    "schema_hashes": hashes,
                }),
                false,
            )
        }
        Request::Verify => match verify_log(log, executor) {
            Ok(()) => {
                let entries = log
                    .entries()
                    .map(|es| es.len())
                    .unwrap_or(0);
                (json!({"ok": true, "entries": entries}), false)
            }
            Err(e) => (
                json!({"ok": false, "error": e.to_string()}),
                false,
            ),
        },
        Request::HashState => {
            let records: Vec<_> = match store.iter() {
                Ok(it) => it.collect(),
                Err(e) => return (json!({"ok": false, "error": e.to_string()}), false),
            };
            let count = records.len();
            let hash = match store.hash_state() {
                Ok(h) => h,
                Err(e) => return (json!({"ok": false, "error": e.to_string()}), false),
            };
            (
                json!({
                    "ok": true,
                    "hash": hex_encode(&hash),
                    "records": count,
                }),
                false,
            )
        }
        Request::DumpRecords => match store.iter() {
            Ok(it) => {
                let records: Vec<Value> = it
                    .map(|(entity, id, value)| {
                        json!({"entity": entity, "id": id, "value": value})
                    })
                    .collect();
                (json!({"ok": true, "records": records}), false)
            }
            Err(e) => (json!({"ok": false, "error": e.to_string()}), false),
        },
        Request::Shutdown => (json!({"ok": true, "shutdown": true}), true),
    }
}

fn error_response(msg: &str) -> Value {
    json!({"ok": false, "error": msg})
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

