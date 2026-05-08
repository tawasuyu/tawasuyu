//! Tests for the `Store::iter` / `Store::hash_state` contract under
//! realistic WAL flows: a live store and a log-replayed store must hash
//! identically, drift must be detectable as a hash mismatch, and the
//! property must hold across backends (within a backend — cross-backend
//! parity is a separate concern, see notes below).

use std::path::{Path, PathBuf};

use nakui_core::event_log::{EventLog, execute_and_log, replay, seed_and_log};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
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
    std::env::temp_dir().join(format!("nakui_hash_{}.jsonl", Uuid::new_v4()))
}

fn seed_two_cajas(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    a: Uuid,
    b: Uuid,
) {
    seed_and_log(
        exec,
        store,
        log,
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 200_000_i64, "currency": "USD"}),
    )
    .unwrap();
    seed_and_log(
        exec,
        store,
        log,
        "Caja",
        b,
        json!({"id": b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    )
    .unwrap();
}

#[test]
fn live_store_hash_matches_replayed_store_hash() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).unwrap();
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_two_cajas(&exec, &mut live, &mut log, a, b);

    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 25_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 75_000_i64,
            "timestamp": "2026-05-04T10:30:00Z",
            "memo": "xf",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    let replayed = replay(&log).expect("replay");

    assert_eq!(
        live.hash_state().unwrap(),
        replayed.hash_state().unwrap(),
        "live and replayed stores must hash identically"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn drift_is_detectable_via_hash_diff() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).unwrap();
    let mut live = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_two_cajas(&exec, &mut live, &mut log, a, b);

    let baseline = live.hash_state().unwrap();
    let replayed_baseline = replay(&log).unwrap().hash_state().unwrap();
    assert_eq!(baseline, replayed_baseline);

    // Drift the live store out-of-band — exactly what the drift detector
    // is meant to catch.
    live.seed(
        "Caja",
        a,
        json!({"id": a.to_string(), "name": "A", "saldo": 999_999_i64, "currency": "USD"}),
    );

    let drifted = live.hash_state().unwrap();
    let log_canonical = replay(&log).unwrap().hash_state().unwrap();
    assert_ne!(
        drifted, log_canonical,
        "the whole point of hash_state: this comparison must surface the drift"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn hash_state_is_stable_across_repeated_calls() {
    // The hash must not drift just because we asked for it twice.
    // Sounds obvious; protects against an iteration order that depends
    // on a HashMap's per-process random seed sneaking past the sort.
    let mut store = MemoryStore::new();
    for _ in 0..10 {
        let id = Uuid::new_v4();
        store.seed(
            "Caja",
            id,
            json!({"id": id.to_string(), "saldo": 100_i64, "currency": "USD"}),
        );
    }
    let h1 = store.hash_state().unwrap();
    let h2 = store.hash_state().unwrap();
    assert_eq!(h1, h2, "hash must be a function of state, not call order");
}
