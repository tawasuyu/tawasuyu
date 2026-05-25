//! SurrealStore: kv-mem SurrealDB behind the same `Store` trait.
//!
//! Tests confirm: round-trip persistence preserving the application-level
//! `id` field, the dry-run contract, and the full WAL flow against the
//! real DB driver — execute_and_log → replay_into → live equals replayed.

use std::path::{Path, PathBuf};

use nakui_core::delta::{FieldOp, FieldPath};
use nakui_core::event_log::{
    execute_and_log, reconcile, replay_into, seed_and_log, verify_log, EventLog,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store, StoreError};
use nakui_core::surreal_store::SurrealStore;
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
    std::env::temp_dir().join(format!("nakui_surreal_{}.jsonl", Uuid::new_v4()))
}

fn caja_data(id: Uuid, saldo: i64, currency: &str) -> Value {
    json!({
        "id": id.to_string(),
        "name": "Caja",
        "saldo": saldo,
        "currency": currency,
    })
}

#[test]
fn seed_then_load_preserves_application_id() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    store.seed("Caja", id, caja_data(id, 100_000, "USD"));

    let loaded = store.load("Caja", id).expect("loaded");
    assert_eq!(
        loaded.get("id").and_then(Value::as_str),
        Some(id.to_string().as_str()),
        "load must restore the application-level id field"
    );
    assert_eq!(loaded.get("saldo").and_then(Value::as_i64), Some(100_000));
    assert_eq!(loaded.get("currency").and_then(Value::as_str), Some("USD"));
}

#[test]
fn apply_set_updates_field() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    store.seed("Caja", id, caja_data(id, 100_000, "USD"));

    store
        .apply(&[FieldOp::Set {
            path: FieldPath {
                entity: "Caja".into(),
                id,
                field: "saldo".into(),
            },
            value: json!(250_000_i64),
        }])
        .expect("apply Set");

    let loaded = store.load("Caja", id).expect("loaded");
    assert_eq!(loaded.get("saldo").and_then(Value::as_i64), Some(250_000));
    // Other fields preserved.
    assert_eq!(loaded.get("currency").and_then(Value::as_str), Some("USD"));
}

#[test]
fn apply_create_persists_record() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    store
        .apply(&[FieldOp::Create {
            entity: "Movimiento".into(),
            id,
            data: json!({
                "id": id.to_string(),
                "caja_id": Uuid::new_v4().to_string(),
                "monto": 1000,
                "tipo": "in",
                "timestamp": "2026-05-04T00:00:00Z",
            }),
        }])
        .expect("apply Create");

    let loaded = store.load("Movimiento", id).expect("loaded");
    assert_eq!(loaded.get("monto").and_then(Value::as_i64), Some(1000));
    assert_eq!(loaded.get("tipo").and_then(Value::as_str), Some("in"));
}

#[test]
fn apply_delete_removes_record() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    store.seed("Caja", id, caja_data(id, 100_000, "USD"));

    store
        .apply(&[FieldOp::Delete {
            entity: "Caja".into(),
            id,
        }])
        .expect("apply Delete");

    assert!(store.load("Caja", id).is_none());
}

#[test]
fn dry_run_rejects_create_conflict() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    store.seed("Caja", id, caja_data(id, 100, "USD"));

    let result = store.apply_dry_run(&[FieldOp::Create {
        entity: "Caja".into(),
        id,
        data: json!({"id": id.to_string()}),
    }]);
    assert!(matches!(result, Err(StoreError::Conflict(_, _))));
}

#[test]
fn dry_run_rejects_set_not_found() {
    let store = SurrealStore::new_in_memory().expect("surreal");
    let id = Uuid::new_v4();
    let result = store.apply_dry_run(&[FieldOp::Set {
        path: FieldPath {
            entity: "Caja".into(),
            id,
            field: "saldo".into(),
        },
        value: json!(0),
    }]);
    assert!(matches!(result, Err(StoreError::NotFound(_, _))));
}

#[test]
fn full_wal_flow_against_surreal() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut live = SurrealStore::new_in_memory().expect("live store");

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        caja_data(a, 200_000, "USD"),
    )
    .expect("seed A");
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        caja_data(b, 50_000, "USD"),
    )
    .expect("seed B");

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
            "memo": "test",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .expect("deposit ok");

    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "transfer_between_cajas",
        &[("source", a), ("dest", b)],
        json!({
            "monto": 75_000_i64,
            "timestamp": "2026-05-04T10:30:00Z",
            "memo": "xfer",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    )
    .expect("transfer ok");

    // Replay into a fresh SurrealStore and confirm field-by-field that
    // saldos and entity counts match the live one.
    let mut replayed = SurrealStore::new_in_memory().expect("replay store");
    replay_into(&log, &mut replayed).expect("replay");

    let live_a = live.load("Caja", a).expect("live A");
    let replayed_a = replayed.load("Caja", a).expect("replayed A");
    assert_eq!(
        live_a.get("saldo").and_then(Value::as_i64),
        replayed_a.get("saldo").and_then(Value::as_i64)
    );

    let live_b = live.load("Caja", b).expect("live B");
    let replayed_b = replayed.load("Caja", b).expect("replayed B");
    assert_eq!(
        live_b.get("saldo").and_then(Value::as_i64),
        replayed_b.get("saldo").and_then(Value::as_i64)
    );

    assert_eq!(live_a.get("saldo").and_then(Value::as_i64), Some(150_000));
    assert_eq!(live_b.get("saldo").and_then(Value::as_i64), Some(125_000));

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_log_against_surreal_passes() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut live = SurrealStore::new_in_memory().expect("live");

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        caja_data(a, 200_000, "USD"),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        caja_data(b, 50_000, "USD"),
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
            "timestamp": "2026-05-04T11:00:00Z",
            "memo": "v",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // verify_log internally creates its own MemoryStore for re-execution;
    // even though `live` is SurrealStore, the determinism check is
    // re-running each morphism through the kernel and comparing ops, so
    // the verification store backend doesn't need to match the live one.
    verify_log(&log, &exec).expect("re-execution must produce identical ops");

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn replay_into_memorystore_from_surreal_run_log() {
    // Ensure logs produced by SurrealStore-backed runs replay correctly
    // into a *different* backend (MemoryStore). The log is the source of
    // truth — backend choice shouldn't change the replay result.
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open");

    let mut surreal_live = SurrealStore::new_in_memory().expect("surreal");
    let a = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut surreal_live,
        &mut log,
        "Caja",
        a,
        caja_data(a, 100_000, "USD"),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut surreal_live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 50_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T08:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    let mut mem_replay = MemoryStore::new();
    replay_into(&log, &mut mem_replay).expect("replay");

    let live_saldo = surreal_live
        .load("Caja", a)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .unwrap();
    let replay_saldo = mem_replay
        .load("Caja", a)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .unwrap();
    assert_eq!(live_saldo, replay_saldo);
    assert_eq!(live_saldo, 150_000);

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn clear_drops_all_records_across_tables() {
    let mut store = SurrealStore::new_in_memory().expect("surreal");
    let caja_id = Uuid::new_v4();
    let mov_id = Uuid::new_v4();
    store.seed("Caja", caja_id, caja_data(caja_id, 100_000, "USD"));
    store.seed(
        "Movimiento",
        mov_id,
        json!({
            "id": mov_id.to_string(),
            "caja_id": caja_id.to_string(),
            "monto": 1_000,
            "tipo": "in",
            "timestamp": "2026-05-04T00:00:00Z",
        }),
    );
    assert!(store.load("Caja", caja_id).is_some());
    assert!(store.load("Movimiento", mov_id).is_some());

    store.clear().expect("clear");

    assert!(
        store.load("Caja", caja_id).is_none(),
        "clear must drop records from every table"
    );
    assert!(store.load("Movimiento", mov_id).is_none());

    // Store is reusable after clear — seed a new record and load it back.
    let fresh = Uuid::new_v4();
    store.seed("Caja", fresh, caja_data(fresh, 1, "USD"));
    assert!(store.load("Caja", fresh).is_some());
}

#[test]
fn cross_backend_hash_equals_for_equivalent_data() {
    // The whole point of the canonical Value hasher: a SurrealStore
    // and a MemoryStore that hold the same logical records must hash
    // identically. Same WAL log replayed into each backend ⇒
    // hash_state produces byte-equal output.
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");

    let mut surreal = SurrealStore::new_in_memory().expect("surreal");
    let mut memory = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    // Seed both backends through the WAL so they go through identical
    // op sequences. We seed each backend separately because seed_and_log
    // takes one store at a time.
    seed_and_log(
        &exec,
        &mut surreal,
        &mut log,
        "Caja",
        a,
        caja_data(a, 200_000, "USD"),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut surreal,
        &mut log,
        "Caja",
        b,
        caja_data(b, 50_000, "USD"),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut surreal,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 1_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T08:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Replay that same log into a fresh MemoryStore.
    nakui_core::event_log::replay_into(&log, &mut memory).expect("replay");

    let h_surreal = surreal.hash_state().expect("surreal hash");
    let h_memory = memory.hash_state().expect("memory hash");
    assert_eq!(
        h_surreal, h_memory,
        "MemoryStore and SurrealStore must hash identically for the same WAL state"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn iter_and_hash_state_round_trip_against_surreal() {
    // Build the same WAL flow against two independent SurrealStores.
    // Each store reaches the same logical state via a different path
    // (one via execute_and_log, the other via replay_into) and they
    // must hash identically — that's the contract drift detection
    // sits on top of.
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");

    let mut live = SurrealStore::new_in_memory().expect("live");
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        a,
        caja_data(a, 200_000, "USD"),
    )
    .unwrap();
    seed_and_log(
        &exec,
        &mut live,
        &mut log,
        "Caja",
        b,
        caja_data(b, 50_000, "USD"),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut live,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 1_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T08:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // iter must enumerate every record.
    let recs: Vec<_> = live.iter().expect("iter").collect();
    let by_entity: std::collections::HashMap<&str, usize> =
        recs.iter()
            .fold(std::collections::HashMap::new(), |mut m, (e, _, _)| {
                *m.entry(e.as_str()).or_insert(0) += 1;
                m
            });
    assert_eq!(by_entity.get("Caja").copied(), Some(2), "two Cajas");
    assert_eq!(
        by_entity.get("Movimiento").copied(),
        Some(1),
        "one Movimiento"
    );

    // canonical order: entities sorted, ids byte-sorted within entity.
    let entities: Vec<&str> = recs.iter().map(|(e, _, _)| e.as_str()).collect();
    assert!(
        entities.windows(2).all(|w| w[0] <= w[1]),
        "entities must be sorted: {:?}",
        entities
    );

    // Replay the log into a fresh SurrealStore — same hash.
    let mut replayed = SurrealStore::new_in_memory().expect("replay store");
    replay_into(&log, &mut replayed).expect("replay");
    assert_eq!(
        live.hash_state().unwrap(),
        replayed.hash_state().unwrap(),
        "live and replayed SurrealStores must hash identically"
    );

    // Drift detection: tamper one saldo and confirm the hash diverges.
    live.seed("Caja", a, caja_data(a, 999_999, "USD"));
    assert_ne!(
        live.hash_state().unwrap(),
        replayed.hash_state().unwrap(),
        "out-of-band saldo change must show up as a hash mismatch"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn reconcile_rebuilds_drifted_surreal_store_from_log() {
    let exec = Executor::load_module(treasury_module()).expect("load module");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = SurrealStore::new_in_memory().expect("surreal");

    let a = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut store,
        &mut log,
        "Caja",
        a,
        caja_data(a, 100_000, "USD"),
    )
    .unwrap();
    execute_and_log(
        &exec,
        &mut store,
        &mut log,
        "register_cash_move",
        &[("caja", a)],
        json!({
            "monto": 5_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();

    // Drift: a poison record nobody logged + an out-of-band saldo bump.
    let ghost = Uuid::new_v4();
    store.seed("Caja", ghost, caja_data(ghost, 0, "USD"));
    store.seed("Caja", a, caja_data(a, 999_999, "USD"));
    assert_eq!(
        store
            .load("Caja", a)
            .and_then(|v| v.get("saldo").and_then(Value::as_i64)),
        Some(999_999),
        "drift was applied"
    );

    reconcile(&mut store, &log).expect("reconcile");

    // After reconcile: ghost gone, saldo = 100_000 (seed) + 5_000 (deposit).
    assert!(store.load("Caja", ghost).is_none(), "poison record wiped");
    assert_eq!(
        store
            .load("Caja", a)
            .and_then(|v| v.get("saldo").and_then(Value::as_i64)),
        Some(105_000),
        "reconcile must restore log-canonical saldo"
    );

    let _ = std::fs::remove_file(&log_path);
}
