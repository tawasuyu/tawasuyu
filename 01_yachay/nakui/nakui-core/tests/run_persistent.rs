//! Smoke test for the persistent backend wired into `nakui run`.
//!
//! Gated behind `--features persistent` because SurrealKV pulls in a
//! ~5 min cold native build. Run with:
//!     cargo test --features persistent --test run_persistent
//!
//! What this proves:
//!   1. `run_server` accepts a `SurrealStore` and serves the standard
//!      protocol (execute/load/shutdown round-trip).
//!   2. After shutdown, reopening the same backing store path reveals
//!      the records were actually written through to disk — i.e., the
//!      runtime wasn't just hitting an in-memory façade.
//!
//! What this does NOT prove (covered elsewhere or deferred):
//!   - That startup skips replay when the persistent state is current.
//!     V1 always replays from log, even with a persistent store; the
//!     persistent layer is durability for the runtime cache, not a
//!     replay shortcut. A future `last_applied_seq` tracker would
//!     change that.
//!   - Cross-backend hash equality (Memory vs Surreal). Different
//!     concern — round-trip parity of serde_json::Value through the
//!     SurrealDB driver.

#![cfg(feature = "persistent")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use nakui_core::event_log::{seed_and_log, EventLog};
use nakui_core::executor::Executor;
use nakui_core::run::run_server;
use nakui_core::store::Store;
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
    std::env::temp_dir().join(format!("nakui_runp_log_{}.jsonl", Uuid::new_v4()))
}

fn fresh_store_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_runp_store_{}", Uuid::new_v4()))
}

fn fresh_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_runp_{}.sock", Uuid::new_v4()))
}

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

#[test]
fn run_server_with_persistent_surreal_serves_protocol_and_writes_to_disk() {
    let log_path = fresh_log_path();
    let store_path = fresh_store_path();
    let socket_path = fresh_socket_path();

    // Pre-seed via the WAL so the log has a record the server can
    // replay into the persistent store on startup.
    let caja = Uuid::new_v4();
    {
        let mut store = SurrealStore::new_persistent(&store_path).expect("open persistent");
        let mut log = EventLog::open(&log_path).expect("open log");
        seed_and_log(
            &executor,
            &mut store,
            &mut log,
            "Caja",
            caja,
            json!({
                "id": caja.to_string(),
                "name": "A",
                "saldo": 100_000_i64,
                "currency": "USD",
            }),
        )
        .expect("seed");
    }

    // Start the server with the same persistent store path.
    let executor = Executor::load_module(treasury_module()).expect("load module");
    let log = EventLog::open(&log_path).expect("reopen log");
    let store = SurrealStore::new_persistent(&store_path).expect("reopen persistent");

    let socket_for_client = socket_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = connect_with_retry(&socket_for_client);

        // Initial load picks up the seed (replayed at startup into the
        // persistent store).
        let resp = exchange(
            &mut conn,
            json!({"op": "load", "entity": "Caja", "id": caja.to_string()}),
        );
        if resp["value"]["saldo"].as_i64() != Some(100_000) {
            return Err(format!("startup replay didn't land seed: {}", resp));
        }

        // Drive a deposit through the server — this writes through the
        // log AND the persistent store.
        let resp = exchange(
            &mut conn,
            json!({
                "op": "execute",
                "morphism": "register_cash_move",
                "inputs": {"caja": caja.to_string()},
                "params": {
                    "monto": 7_500_i64,
                    "tipo": "in",
                    "timestamp": "2026-05-04T10:00:00Z",
                    "memo": "persisted",
                    "movimiento_id": Uuid::new_v4().to_string(),
                }
            }),
        );
        if resp["ok"] != json!(true) {
            return Err(format!("execute failed: {}", resp));
        }

        let resp = exchange(
            &mut conn,
            json!({"op": "load", "entity": "Caja", "id": caja.to_string()}),
        );
        if resp["value"]["saldo"].as_i64() != Some(107_500) {
            return Err(format!("post-execute saldo wrong: {}", resp));
        }

        let _ = exchange(&mut conn, json!({"op": "shutdown"}));
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    // Now the server is gone. Open a fresh handle to the SAME persistent
    // store path — the records must be there without any replay. This
    // is what proves "persistent backend" beyond the unit-level tests
    // in surreal_persist.rs: the runtime actually wrote through.
    let store_again = SurrealStore::new_persistent(&store_path).expect("reopen final");
    let v = store_again
        .load("Caja", caja)
        .expect("Caja persisted across server shutdown");
    assert_eq!(
        v.get("saldo").and_then(Value::as_i64),
        Some(107_500),
        "deposit landed in persistent store"
    );

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_dir_all(&store_path);
}

#[test]
fn run_server_skips_replay_when_persistent_store_is_in_sync() {
    // The optimization: when the persistent store's `last_applied_seq`
    // matches the log's last seq, startup_replay must skip the
    // clear+replay entirely. We prove that by mutating the store
    // out-of-band between cycles — if skip happens, the mutation
    // survives; if full replay runs (clear+replay), it'd be wiped.
    let log_path = fresh_log_path();
    let store_path = fresh_store_path();
    let socket_path1 = fresh_socket_path();
    let socket_path2 = fresh_socket_path();
    let caja = Uuid::new_v4();

    // Cycle 1: drive a deposit through the server. After shutdown the
    // persistent store's marker should equal the log's last seq.
    {
        let executor = Executor::load_module(treasury_module()).expect("load");
        let mut log = EventLog::open(&log_path).expect("open log");
        let mut store = SurrealStore::new_persistent(&store_path).expect("open persistent");
        seed_and_log(
            &executor,
            &mut store,
            &mut log,
            "Caja",
            caja,
            json!({
                "id": caja.to_string(),
                "name": "A",
                "saldo": 100_000_i64,
                "currency": "USD",
            }),
        )
        .expect("seed");
        // We end the WAL flow without running run_server in this cycle —
        // the next cycle is the one that exercises the skip path.
        drop(store);
        drop(log);
        drop(executor);
    }

    // Out-of-band mutation: open the persistent store directly and
    // change the saldo. Marker stays at the same seq.
    {
        let mut store = SurrealStore::new_persistent(&store_path).expect("reopen for poison");
        store.seed(
            "Caja",
            caja,
            json!({
                "id": caja.to_string(),
                "name": "A",
                "saldo": 999_999_i64, // poison
                "currency": "USD",
            }),
        );
        // The marker we set during the WAL flow stays intact — seed()
        // alone does not bump it.
    }

    // Cycle 2: run_server with the poisoned store. Marker == log_last
    // (still 0 from the seed) → skip path → poison saldo survives.
    let executor = Executor::load_module(treasury_module()).expect("load");
    let log = EventLog::open(&log_path).expect("reopen log");
    let store = SurrealStore::new_persistent(&store_path).expect("reopen final");

    let socket_for_client = socket_path1.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = connect_with_retry(&socket_for_client);
        let resp = exchange(
            &mut conn,
            json!({"op": "load", "entity": "Caja", "id": caja.to_string()}),
        );
        let saldo = resp["value"]["saldo"].as_i64();
        let _ = exchange(&mut conn, json!({"op": "shutdown"}));
        if saldo != Some(999_999) {
            return Err(format!(
                "skip-replay should preserve out-of-band saldo (999_999), got {:?}",
                saldo
            ));
        }
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path1).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    // Cycle 3: explicitly invalidate the marker (simulating a backend
    // that lost track) and confirm full replay restores log-canonical
    // state — wiping the poison.
    {
        let mut store = SurrealStore::new_persistent(&store_path).expect("reopen for marker reset");
        // Force the marker into the "uninitialized" state by clearing
        // and reseeding the legitimate record without bumping it. The
        // simplest way is to clear() then re-seed; clear nukes
        // last_applied_seq.
        store.clear().expect("clear");
        store.seed(
            "Caja",
            caja,
            json!({
                "id": caja.to_string(),
                "name": "A",
                "saldo": 999_999_i64, // poison still present
                "currency": "USD",
            }),
        );
        // last_applied_seq is now None → mismatch with log_last → full replay path.
    }

    let executor = Executor::load_module(treasury_module()).expect("load");
    let log = EventLog::open(&log_path).expect("reopen");
    let store = SurrealStore::new_persistent(&store_path).expect("reopen");

    let socket_for_client = socket_path2.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = connect_with_retry(&socket_for_client);
        let resp = exchange(
            &mut conn,
            json!({"op": "load", "entity": "Caja", "id": caja.to_string()}),
        );
        let saldo = resp["value"]["saldo"].as_i64();
        let _ = exchange(&mut conn, json!({"op": "shutdown"}));
        if saldo != Some(100_000) {
            return Err(format!(
                "full replay should restore canonical saldo (100_000), got {:?}",
                saldo
            ));
        }
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path2).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    let _ = std::fs::remove_file(&log_path);
    let _ = std::fs::remove_dir_all(&store_path);
}
