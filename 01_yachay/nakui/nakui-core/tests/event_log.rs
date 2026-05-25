//! Integration tests for the event log: round-trip persistence,
//! replay-equivalence with the live store, and determinism verification.

use std::path::{Path, PathBuf};

use nakui_core::delta::FieldOp;
use nakui_core::event_log::{
    execute_and_log, execute_and_log_with_recovery, reconcile, replay, replay_with_snapshot_into,
    seed_and_log, verify_log, EventLog, ExecuteError, LogEntry, RecoverableExecuteError, Snapshot,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store, StoreError};
use serde_json::{json, Value};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn treasury_module() -> PathBuf {
    workspace_root().join("modules/treasury")
}

fn fresh_log_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_test_{}.jsonl", Uuid::new_v4()))
}

#[test]
fn replay_reconstructs_live_store() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        json!({
            "id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD",
        }),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        json!({
            "id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD",
        }),
    )
    .unwrap();

    let mov_id = Uuid::new_v4();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 25_000_i64, "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z", "memo": "x",
            "movimiento_id": mov_id.to_string(),
        }),
    )
    .unwrap();

    let xfer_id = Uuid::new_v4();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 75_000_i64,
            "timestamp": "2026-05-04T10:30:00Z", "memo": "xf",
            "transfer_id": xfer_id.to_string(),
        }),
    )
    .unwrap();

    // Failed morphism — should NOT be logged.
    let attempt = execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 999_999_999_i64,
            "timestamp": "2026-05-04T10:45:00Z", "memo": "overdraw",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );
    assert!(matches!(attempt, Err(ExecuteError::PreLog(_))));

    let replayed = replay(&log).expect("replay");
    assert_eq!(live, replayed, "replayed store must equal live store");

    // Failed attempt left no trace in the log.
    let entries = log.entries().unwrap();
    assert_eq!(
        entries.len(),
        4,
        "2 seeds + 2 successful morphisms = 4 entries; got {}",
        entries.len()
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_log_passes_for_deterministic_morphisms() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD"}),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        json!({"id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 25_000_i64,
            "timestamp": "2026-05-04T11:00:00Z", "memo": "v",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    verify_log(&log, &exec).expect("re-execution must produce identical ops");

    let _ = std::fs::remove_file(&log_path);
}

/// Store wrapper that passes dry_run through to MemoryStore but always
/// fails on apply. Used to simulate a transient backend failure landing
/// AFTER the kernel has validated and the log has been written.
struct FailOnApplyStore {
    inner: MemoryStore,
}

impl Store for FailOnApplyStore {
    fn load(&self, entity: &str, id: Uuid) -> Option<Value> {
        self.inner.load(entity, id)
    }
    fn seed(&mut self, entity: &str, id: Uuid, data: Value) {
        self.inner.seed(entity, id, data);
    }
    fn apply_dry_run(&self, ops: &[FieldOp]) -> Result<(), StoreError> {
        self.inner.apply_dry_run(ops)
    }
    fn apply(&mut self, _ops: &[FieldOp]) -> Result<(), StoreError> {
        Err(StoreError::NotFound(
            "synthetic_apply_failure".into(),
            Uuid::nil(),
        ))
    }
    fn clear(&mut self) -> Result<(), StoreError> {
        self.inner.clear()
    }
    fn iter(&self) -> Result<Box<dyn Iterator<Item = (String, Uuid, Value)> + '_>, StoreError> {
        self.inner.iter()
    }
}

#[test]
fn post_log_store_failure_leaves_log_canonical() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");

    // Seed the inner store directly (no logging — we're simulating the
    // backend independently of the log).
    let mut inner = MemoryStore::new();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    inner.seed(
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD"}),
    );
    inner.seed(
        "Caja",
        b,
        json!({"id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    );
    let mut store = FailOnApplyStore { inner };

    let result = execute_and_log(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 25_000_i64,
            "timestamp": "2026-05-04T11:00:00Z",
            "memo": "wal-test",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    match result {
        Err(ExecuteError::PostLogStore(_)) => {}
        other => panic!("expected PostLogStore, got {:?}", other),
    }

    // Log is canonical: the morphism event is durable.
    let entries = log.entries().expect("read");
    assert_eq!(entries.len(), 1, "log must contain the morphism event");
    assert!(matches!(&entries[0], LogEntry::Morphism { .. }));

    // Live store is stale: apply was rejected, so saldos are unchanged.
    assert_eq!(
        store
            .load("Caja", a)
            .unwrap()
            .get("saldo")
            .unwrap()
            .as_i64(),
        Some(200_000)
    );
    assert_eq!(
        store
            .load("Caja", b)
            .unwrap()
            .get("saldo")
            .unwrap()
            .as_i64(),
        Some(50_000)
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn reopen_log_resumes_from_correct_seq() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let a = Uuid::new_v4();

    {
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_and_log(
            &exec,
            &mut store,
            &mut log,
            "Caja",
            a,
            json!({"id": a.to_string(), "name": "A", "saldo": 100_i64, "currency": "USD"}),
        )
        .unwrap();
        assert_eq!(log.next_seq(), 1);
    }

    {
        let log = EventLog::open(&log_path).unwrap();
        assert_eq!(log.next_seq(), 1, "next_seq must persist across reopens");
        let entries = log.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0], LogEntry::Seed { seq: 0, .. }));
    }

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn snapshot_plus_log_tail_replays_to_same_state() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let snap_path = log_path.with_extension("snap");
    let mut log = EventLog::open(&log_path).expect("open");
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD"}),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        json!({"id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 25_000_i64, "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z", "memo": "before-snap",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Take snapshot at this point: seq 0 (seed A), 1 (seed B), 2 (deposit)
    // are reflected in `live`. Next event will be seq 3.
    let snap = Snapshot::from_memory_store(&live, log.next_seq() - 1);
    snap.write(&snap_path).expect("write snapshot");
    assert_eq!(snap.seq, 2);

    // More events after the snapshot.
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 75_000_i64,
            "timestamp": "2026-05-04T10:30:00Z", "memo": "after-snap",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Replay from snapshot + log tail; must equal live store.
    let loaded_snap = Snapshot::load(&snap_path).expect("load").expect("present");
    let mut replayed = MemoryStore::new();
    replay_with_snapshot_into(&log, Some(&loaded_snap), &mut replayed).expect("replay");

    assert_eq!(live, replayed, "snapshot + tail must equal full replay");

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&snap_path);
}

#[test]
fn compact_through_drops_old_entries_keeps_seq() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open");

    let mut live = MemoryStore::new();
    for i in 0..5 {
        let id = Uuid::new_v4();
        seed_and_log(
            &exec,
            &mut live,
            &mut log,
            "Caja",
            id,
            json!({"id": id.to_string(), "name": format!("c{}", i), "saldo": 100_i64, "currency": "USD"}),
        )
        .unwrap();
    }

    assert_eq!(log.next_seq(), 5);
    assert_eq!(log.entries().unwrap().len(), 5);

    // Compact through seq 2: entries 0,1,2 are dropped; 3,4 remain.
    log.compact_through(2).expect("compact");

    let surviving = log.entries().unwrap();
    assert_eq!(surviving.len(), 2);
    assert_eq!(surviving[0].seq(), 3);
    assert_eq!(surviving[1].seq(), 4);

    // next_seq stays at 5 — we kept the surviving entries' counter intact.
    // (Reopen to confirm the persisted log roundtrips this.)
    drop(log);
    let reopened = EventLog::open(&log_path).expect("reopen after compact");
    assert_eq!(reopened.next_seq(), 5);

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn snapshot_then_compact_then_replay_equals_pre_compaction() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let snap_path = log_path.with_extension("snap");
    let mut log = EventLog::open(&log_path).expect("open");
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 1_000_i64, "currency": "USD"}),
    )
    .unwrap();
    for i in 0..3 {
        execute_and_log(
            &exec,
            &mut live,
            &mut log,
            "register_cash_move",
            &[("caja", a)],
            json!({
                "monto": 100_i64, "tipo": "in",
                "timestamp": format!("2026-05-04T10:0{}:00Z", i), "memo": "x",
                "movimiento_id": Uuid::new_v4().to_string(),
            }),
        )
        .unwrap();
    }
    // Snapshot at seq 3 (1 seed + 3 morphisms = seqs 0..=3).
    let snap = Snapshot::from_memory_store(&live, log.next_seq() - 1);
    snap.write(&snap_path).expect("write snap");
    log.compact_through(snap.seq).expect("compact");

    // After compaction: log has 0 entries (all subsumed). next_seq = 4.
    assert_eq!(log.entries().unwrap().len(), 0);

    // More events after compaction.
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 500_i64, "tipo": "in",
            "timestamp": "2026-05-04T11:00:00Z", "memo": "post-compact",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Reconstruct from snapshot + remaining log.
    let loaded_snap = Snapshot::load(&snap_path).unwrap().unwrap();
    let mut replayed = MemoryStore::new();
    replay_with_snapshot_into(&log, Some(&loaded_snap), &mut replayed).expect("replay");
    assert_eq!(
        live, replayed,
        "snapshot + post-compact log must equal live"
    );

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&snap_path);
}

#[test]
fn reconcile_rebuilds_drifted_store_from_log() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 100_000_i64, "currency": "USD"}),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 5_000_i64, "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z", "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Drift the store out-of-band: a poison record nobody logged, plus a
    // tampered saldo on the legitimate one.
    let ghost = Uuid::new_v4();
    live.seed(
        "Caja",
        ghost,
        json!({"id": ghost.to_string(), "name": "GHOST", "saldo": 0, "currency": "USD"}),
    );
    live.seed(
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 999_999_i64, "currency": "USD"}),
    );

    // Canonical state: replay from log into a clean store.
    let canonical = replay(&log).expect("replay");
    assert_ne!(live, canonical, "drift was set up to differ from log");

    reconcile(&mut live, &log).expect("reconcile");
    assert_eq!(
        live, canonical,
        "reconcile must restore log-canonical state"
    );
    assert!(
        live.load("Caja", ghost).is_none(),
        "poison record must be wiped"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn execute_and_log_with_recovery_succeeds_on_clean_path() {
    // The clean path: no apply failure means the wrapper returns the same
    // ops as `execute_and_log` and leaves the store consistent.
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut store,
        &mut log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 10_000_i64, "currency": "USD"}),
    )
    .unwrap();

    let ops = execute_and_log_with_recovery(
        &exec,
        &mut store,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 1_000_i64, "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z", "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .expect("recovery wrapper");
    assert!(!ops.is_empty(), "morphism produced ops");

    let replayed = replay(&log).expect("replay");
    assert_eq!(store, replayed, "store and log agree on clean path");

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn execute_and_log_with_recovery_reports_unrecoverable_when_replay_also_fails() {
    // Apply is permanently broken — reconcile (which replays through apply)
    // will also fail. The wrapper must surface `Unrecoverable` so the
    // caller knows the store is no longer in sync with the log.
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");

    let mut inner = MemoryStore::new();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    inner.seed(
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD"}),
    );
    inner.seed(
        "Caja",
        b,
        json!({"id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    );
    let mut store = FailOnApplyStore { inner };

    let result = execute_and_log_with_recovery(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 25_000_i64,
            "timestamp": "2026-05-04T11:00:00Z",
            "memo": "recover-fail",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );
    assert!(
        matches!(result, Err(RecoverableExecuteError::Unrecoverable { .. })),
        "expected Unrecoverable, got {:?}",
        result
    );

    // The log entry is still canonical: an operator who fixes the backend
    // can recover via `nakui replay` later.
    let entries = log.entries().expect("read log");
    assert_eq!(entries.len(), 1);
    assert!(matches!(&entries[0], LogEntry::Morphism { .. }));

    let _ = std::fs::remove_file(&log_path);
}
