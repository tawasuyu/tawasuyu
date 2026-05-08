//! End-to-end tests for the snapshot lifecycle: capture, compact, and
//! boot from snapshot. Plus the schema-hash binding that ties a snapshot
//! to the bundle that produced it.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use nakui_core::event_log::{
    EventLog, Snapshot, SnapshotMismatchError, execute_and_log, replay, seed_and_log,
};
use nakui_core::executor::Executor;
use nakui_core::run::run_server;
use nakui_core::store::{MemoryStore, Store};
use serde_json::{Value, json};
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
    std::env::temp_dir().join(format!("nakui_snap_log_{}.jsonl", Uuid::new_v4()))
}

fn fresh_snap_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_snap_{}.json", Uuid::new_v4()))
}

fn fresh_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_snap_run_{}.sock", Uuid::new_v4()))
}

fn seed_caja(exec: &Executor, store: &mut MemoryStore, log: &mut EventLog, id: Uuid, saldo: i64) {
    seed_and_log(
        exec,
        store,
        log,
        "Caja",
        id,
        json!({"id": id.to_string(), "name": "A", "saldo": saldo, "currency": "USD"}),
    )
    .unwrap();
}

fn deposit(exec: &Executor, store: &mut MemoryStore, log: &mut EventLog, caja: Uuid, monto: i64) {
    execute_and_log(
        exec,
        store,
        log,
        "register_cash_move",
        &[("caja", caja)],
        json!({
            "monto": monto,
            "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();
}

#[test]
fn module_schema_hash_is_stable_and_independent_of_load_order() {
    let exec1 = Executor::load_module(treasury_module()).expect("load 1");
    let exec2 = Executor::load_module(treasury_module()).expect("load 2");
    assert_eq!(
        exec1.module_schema_hash(),
        exec2.module_schema_hash(),
        "two clean loads of the same module → identical module hash"
    );
}

#[test]
fn capture_records_executor_hash_legacy_does_not() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let mut store = MemoryStore::new();
    store.seed("Caja", Uuid::new_v4(), json!({"x": 1}));

    let captured = Snapshot::capture(&store, 0, &exec);
    assert_eq!(captured.schema_hash, Some(exec.module_schema_hash()));

    let legacy = Snapshot::from_memory_store(&store, 0);
    assert_eq!(legacy.schema_hash, None, "legacy constructor opts out");

    captured
        .ensure_compatible_with(&exec)
        .expect("captured snapshot is compatible with the executor that built it");
    legacy
        .ensure_compatible_with(&exec)
        .expect("legacy snapshot has no hash → no check, passes");
}

#[test]
fn ensure_compatible_with_rejects_mismatched_hash() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let mut snap = Snapshot::capture(&MemoryStore::new(), 0, &exec);
    // Tamper with the hash to simulate a snapshot from a different bundle.
    snap.schema_hash = Some([0xAB; 32]);
    match snap.ensure_compatible_with(&exec) {
        Err(SnapshotMismatchError::SchemaMismatch { .. }) => {}
        other => panic!("expected SchemaMismatch, got {:?}", other),
    }
}

#[test]
fn snapshot_then_compact_then_run_server_resumes_correctly() {
    // The full operator workflow:
    //   1. Run a series of WAL-validated ops.
    //   2. Capture a snapshot covering the last seq.
    //   3. Compact the log so it only retains entries past snap.seq.
    //   4. Start a server pointing at the (compacted) log + snapshot.
    //   5. Confirm the server's state is correct via the load op.
    //
    // After step 3 the log alone can't reconstruct the state — the
    // snapshot is the only thing that proves the server isn't lying.
    let log_path = fresh_log_path();
    let snap_path = fresh_snap_path();
    let socket_path = fresh_socket_path();

    let caja = Uuid::new_v4();
    let snap_seq;
    let captured_module_hash;
    {
        let exec = Executor::load_module(treasury_module()).expect("load");
        captured_module_hash = exec.module_schema_hash();
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, caja, 100_000);
        deposit(&exec, &mut store, &mut log, caja, 5_000);
        deposit(&exec, &mut store, &mut log, caja, 7_500);

        snap_seq = log.next_seq() - 1;
        let snap = Snapshot::capture(&store, snap_seq, &exec);
        snap.write(&snap_path).unwrap();
        log.compact_through(snap_seq).unwrap();

        // Sanity: after compaction the log has no surviving entries.
        let surviving = log.entries().unwrap();
        assert_eq!(surviving.len(), 0);
        // But next_seq is preserved, so future appends keep monotonicity.
        assert_eq!(log.next_seq(), snap_seq + 1);
    }

    // Verify the snapshot file carries the captured hash (resilient
    // through write+read).
    let reloaded = Snapshot::load(&snap_path).unwrap().unwrap();
    assert_eq!(reloaded.schema_hash, Some(captured_module_hash));
    assert_eq!(reloaded.seq, snap_seq);

    // Boot the server with snapshot + compacted log.
    let executor = Executor::load_module(treasury_module()).expect("reload");
    let log = EventLog::open(&log_path).unwrap();
    let store = MemoryStore::new();

    let socket_for_client = socket_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = connect_with_retry(&socket_for_client);
        let resp = exchange(&mut conn, json!({"op": "load", "entity": "Caja", "id": caja.to_string()}));
        if resp["value"]["saldo"].as_i64() != Some(112_500) {
            return Err(format!(
                "expected saldo 112_500 (100k seed + 5k + 7.5k from snapshot), got {}",
                resp
            ));
        }
        // Append a new op via the live server and load it back —
        // confirms the WAL still works on top of a snapshot-loaded state.
        let resp = exchange(
            &mut conn,
            json!({
                "op": "execute",
                "morphism": "register_cash_move",
                "inputs": {"caja": caja.to_string()},
                "params": {
                    "monto": 1_000_i64,
                    "tipo": "in",
                    "timestamp": "2026-05-04T11:00:00Z",
                    "memo": "post-snap",
                    "movimiento_id": Uuid::new_v4().to_string(),
                }
            }),
        );
        if resp["ok"] != json!(true) {
            return Err(format!("execute on snapshot-booted server failed: {}", resp));
        }
        let resp = exchange(&mut conn, json!({"op": "load", "entity": "Caja", "id": caja.to_string()}));
        if resp["value"]["saldo"].as_i64() != Some(113_500) {
            return Err(format!("post-execute saldo wrong: {}", resp));
        }
        send_shutdown(&mut conn);
        Ok(())
    });

    run_server(executor, log, store, Some(reloaded), &socket_path).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_file(&snap_path);
}

#[test]
fn run_server_refuses_snapshot_with_wrong_schema_hash() {
    let log_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja = Uuid::new_v4();
    {
        let exec = Executor::load_module(treasury_module()).expect("load");
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, caja, 100_000);
        deposit(&exec, &mut store, &mut log, caja, 5_000);
    }

    // Build a snapshot with a fabricated hash — simulates "snapshot
    // taken under module A, loaded against module B."
    let exec = Executor::load_module(treasury_module()).expect("reload");
    let log = EventLog::open(&log_path).unwrap();
    let snap_state = replay(&log).unwrap();
    let last_seq = log.entries().unwrap().last().unwrap().seq();
    let mut bad_snap = Snapshot::capture(&snap_state, last_seq, &exec);
    bad_snap.schema_hash = Some([0xAB; 32]);

    let store = MemoryStore::new();
    let result = run_server(exec, log, store, Some(bad_snap), &socket_path);
    assert!(
        matches!(
            result,
            Err(nakui_core::run::RunError::SnapshotMismatch(_))
        ),
        "expected SnapshotMismatch, got {:?}",
        result
    );
    // Socket must not have been bound.
    assert!(!socket_path.exists());

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn run_server_detects_gap_between_snapshot_and_compacted_log() {
    // Snapshot says it covers up to seq K. Log was compacted further,
    // so its first remaining entry is K+5 — entries K+1..=K+4 are
    // gone. run_server must refuse rather than silently fabricate a
    // state that drops events.
    let log_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja = Uuid::new_v4();
    let exec = Executor::load_module(treasury_module()).expect("load");
    {
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, caja, 100_000);
        deposit(&exec, &mut store, &mut log, caja, 1_000);
        deposit(&exec, &mut store, &mut log, caja, 1_000);
        deposit(&exec, &mut store, &mut log, caja, 1_000);
        deposit(&exec, &mut store, &mut log, caja, 1_000);
        deposit(&exec, &mut store, &mut log, caja, 1_000);
    }

    // Snapshot at seq 0 (only the seed).
    let mut log = EventLog::open(&log_path).unwrap();
    let mut state = MemoryStore::new();
    nakui_core::event_log::replay_with_snapshot_into(&log, None, &mut state).unwrap();
    let snap = Snapshot::capture(&state, 0, &exec);

    // Compact the log past the snapshot — drop seqs 0..=3, leaving
    // entries from seq 4 onward. The snapshot can't reconstruct the
    // missing tail.
    log.compact_through(3).unwrap();
    drop(log);

    let exec = Executor::load_module(treasury_module()).expect("reload");
    let log = EventLog::open(&log_path).unwrap();
    let store = MemoryStore::new();
    let result = run_server(exec, log, store, Some(snap), &socket_path);
    match result {
        Err(nakui_core::run::RunError::SnapshotGap {
            snap_seq,
            log_first_seq,
            expected,
        }) => {
            assert_eq!(snap_seq, 0);
            assert_eq!(expected, 1);
            assert!(
                log_first_seq >= 4,
                "log's first surviving entry should be ≥ 4, got {}",
                log_first_seq
            );
        }
        other => panic!("expected SnapshotGap, got {:?}", other),
    }

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn snapshot_write_overwrites_existing_atomically() {
    // Two snapshots at different seqs written to the same path. The
    // second must completely replace the first; load() returns the
    // newer one.
    let snap_path = fresh_snap_path();
    let exec = Executor::load_module(treasury_module()).expect("load");

    let s1 = Snapshot::capture(&MemoryStore::new(), 0, &exec);
    s1.write(&snap_path).expect("write first");
    let loaded = Snapshot::load(&snap_path).unwrap().unwrap();
    assert_eq!(loaded.seq, 0);

    // Now write a different snapshot to the same path.
    let mut store = MemoryStore::new();
    let id = Uuid::new_v4();
    store.seed("Caja", id, json!({"id": id.to_string(), "saldo": 7}));
    let s2 = Snapshot::capture(&store, 42, &exec);
    s2.write(&snap_path).expect("overwrite");
    let loaded = Snapshot::load(&snap_path).unwrap().unwrap();
    assert_eq!(loaded.seq, 42, "second write must replace the first");
    assert!(loaded.records.contains_key("Caja"));

    // No leftover tempfile.
    let writing_path = snap_path.with_extension("writing");
    assert!(
        !writing_path.exists(),
        "tempfile must be renamed, not left behind"
    );

    let _ = std::fs::remove_file(&snap_path);
}

#[test]
fn snapshot_write_recovers_from_stale_tempfile() {
    // A prior write crashed after creating .writing but before rename.
    // The next write must succeed regardless — File::create truncates
    // the stale tempfile.
    let snap_path = fresh_snap_path();
    let writing_path = snap_path.with_extension("writing");

    // Plant a stale tempfile with garbage content.
    std::fs::write(&writing_path, b"junk from a prior crashed write").unwrap();
    assert!(writing_path.exists());

    let exec = Executor::load_module(treasury_module()).expect("load");
    let snap = Snapshot::capture(&MemoryStore::new(), 0, &exec);
    snap.write(&snap_path).expect("write despite stale tempfile");

    // Tempfile should be renamed (not orphaned), so it's gone.
    assert!(
        !writing_path.exists(),
        "stale tempfile must be consumed by the rename"
    );
    let loaded = Snapshot::load(&snap_path).unwrap().unwrap();
    assert_eq!(loaded.seq, 0);

    let _ = std::fs::remove_file(&snap_path);
}

// === helpers shared with the run-server protocol tests ===

struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

fn connect_with_retry(path: &Path) -> Conn {
    for _ in 0..200 {
        if let Ok(stream) = UnixStream::connect(path) {
            let reader_stream = stream.try_clone().expect("clone");
            return Conn {
                writer: stream,
                reader: BufReader::new(reader_stream),
            };
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("server never started accepting on {}", path.display());
}

fn exchange(conn: &mut Conn, req: Value) -> Value {
    let mut bytes = serde_json::to_vec(&req).unwrap();
    bytes.push(b'\n');
    conn.writer.write_all(&bytes).unwrap();
    let mut line = String::new();
    conn.reader.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

fn send_shutdown(conn: &mut Conn) {
    let _ = exchange(conn, json!({"op": "shutdown"}));
}
