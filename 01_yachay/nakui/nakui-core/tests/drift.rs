//! End-to-end drift detector: spin up `run_server` against log A, run
//! `check_against_socket` first against the same log (in-sync) and then
//! against a divergent log B (drift detected, with the expected diff
//! list).
//!
//! Same threading inversion as `tests/run.rs`: server on main thread
//! (Executor is `!Send`), client on a worker thread.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;

use nakui_core::drift::{DriftDiff, check_against_socket};
use nakui_core::event_log::{EventLog, execute_and_log, seed_and_log};
use nakui_core::executor::Executor;
use nakui_core::run::run_server;
use nakui_core::store::MemoryStore;
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
    std::env::temp_dir().join(format!("nakui_drift_log_{}.jsonl", Uuid::new_v4()))
}

fn fresh_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_drift_{}.sock", Uuid::new_v4()))
}

/// Build a real WAL-formed log: two cajas seeded + one deposit.
fn build_log_a(path: &Path, caja_a: Uuid, caja_b: Uuid) {
    let executor = Executor::load_module(treasury_module()).expect("load");
    let mut log = EventLog::open(path).expect("open log");
    let mut store = MemoryStore::new();
    seed_and_log(
        &executor,
        &mut store,
        &mut log,
        "Caja",
        caja_a,
        json!({"id": caja_a.to_string(), "name": "A", "saldo": 100_000_i64, "currency": "USD"}),
    )
    .unwrap();
    seed_and_log(
        &executor,
        &mut store,
        &mut log,
        "Caja",
        caja_b,
        json!({"id": caja_b.to_string(), "name": "B", "saldo": 50_000_i64, "currency": "USD"}),
    )
    .unwrap();
    execute_and_log(
        &executor,
        &mut store,
        &mut log,
        "register_cash_move",
        &[("caja", caja_a)],
        json!({
            "monto": 5_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .unwrap();
}

/// Build a divergent log: only caja_a seeded, no deposit, no caja_b.
/// Replaying B produces a different state than the server (which used A).
fn build_log_b(path: &Path, caja_a: Uuid) {
    let executor = Executor::load_module(treasury_module()).expect("load");
    let mut log = EventLog::open(path).expect("open log b");
    let mut store = MemoryStore::new();
    seed_and_log(
        &executor,
        &mut store,
        &mut log,
        "Caja",
        caja_a,
        json!({"id": caja_a.to_string(), "name": "A", "saldo": 100_000_i64, "currency": "USD"}),
    )
    .unwrap();
}

/// Wait for the socket to exist and be connectable, then return a
/// connected stream. Used by helpers that send raw requests bypassing
/// `check_against_socket` (e.g. shutdown).
fn connect_with_retry(path: &Path) -> UnixStream {
    for _ in 0..100 {
        if let Ok(s) = UnixStream::connect(path) {
            return s;
        }
        thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("server never started accepting on {}", path.display());
}

fn send_shutdown(socket_path: &Path) {
    let mut stream = connect_with_retry(socket_path);
    stream.write_all(b"{\"op\":\"shutdown\"}\n").unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
}

#[test]
fn drift_check_reports_in_sync_when_log_matches_server() {
    let log_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja_a = Uuid::new_v4();
    let caja_b = Uuid::new_v4();
    build_log_a(&log_path, caja_a, caja_b);

    let executor = Executor::load_module(treasury_module()).expect("load");
    let log = EventLog::open(&log_path).expect("reopen");
    let store = MemoryStore::new();

    let socket_for_client = socket_path.clone();
    let log_for_client = log_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let report = check_against_socket(&log_for_client, &socket_for_client)
            .map_err(|e| format!("check failed: {}", e))?;
        if !report.in_sync() {
            return Err(format!(
                "expected in_sync, got {} diffs: {:?}",
                report.diffs.len(),
                report.diffs
            ));
        }
        if report.log_hash != report.server_hash {
            return Err("hashes diverged with empty diff — invariant broken".into());
        }
        if report.log_records != report.server_records {
            return Err(format!(
                "record count diverged: log={} server={}",
                report.log_records, report.server_records
            ));
        }
        send_shutdown(&socket_for_client);
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn drift_check_surfaces_expected_per_record_diffs() {
    let log_a_path = fresh_log_path();
    let log_b_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja_a = Uuid::new_v4();
    let caja_b = Uuid::new_v4();
    build_log_a(&log_a_path, caja_a, caja_b);
    build_log_b(&log_b_path, caja_a);

    let executor = Executor::load_module(treasury_module()).expect("load");
    let log = EventLog::open(&log_a_path).expect("reopen");
    let store = MemoryStore::new();

    let socket_for_client = socket_path.clone();
    let log_b_for_client = log_b_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        // Server is running log A's state; we audit using log B's
        // canonical view. Expected diffs:
        //   - Caja caja_a: tampered (B says saldo=100_000, server has 105_000 from deposit)
        //   - Caja caja_b: only_on_server (B never seeded it)
        //   - Movimiento <some uuid>: only_on_server (B never executed the deposit)
        let report = check_against_socket(&log_b_for_client, &socket_for_client)
            .map_err(|e| format!("check failed: {}", e))?;
        if report.in_sync() {
            return Err("expected drift, got in_sync".into());
        }

        let mut tampered = 0;
        let mut only_on_server = 0;
        let mut only_in_log = 0;
        let mut tampered_caja_a = false;
        let mut server_extra_caja_b = false;
        let mut server_extra_movimiento = false;

        for d in &report.diffs {
            match d {
                DriftDiff::Tampered {
                    entity,
                    id,
                    log_value,
                    server_value,
                } => {
                    tampered += 1;
                    if entity == "Caja" && *id == caja_a {
                        tampered_caja_a = true;
                        if log_value["saldo"] != json!(100_000_i64) {
                            return Err(format!("log saldo wrong: {}", log_value));
                        }
                        if server_value["saldo"] != json!(105_000_i64) {
                            return Err(format!("server saldo wrong: {}", server_value));
                        }
                    }
                }
                DriftDiff::OnlyOnServer { entity, id, .. } => {
                    only_on_server += 1;
                    if entity == "Caja" && *id == caja_b {
                        server_extra_caja_b = true;
                    }
                    if entity == "Movimiento" {
                        server_extra_movimiento = true;
                    }
                }
                DriftDiff::OnlyInLog { .. } => only_in_log += 1,
            }
        }
        if tampered != 1 {
            return Err(format!("expected 1 tampered, got {}", tampered));
        }
        if only_on_server != 2 {
            return Err(format!("expected 2 only_on_server, got {}", only_on_server));
        }
        if only_in_log != 0 {
            return Err(format!("expected 0 only_in_log, got {}", only_in_log));
        }
        if !tampered_caja_a {
            return Err("expected tampered diff for caja_a".into());
        }
        if !server_extra_caja_b {
            return Err("expected only_on_server diff for caja_b".into());
        }
        if !server_extra_movimiento {
            return Err("expected only_on_server diff for some Movimiento".into());
        }

        send_shutdown(&socket_for_client);
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path).expect("server clean exit");
    client.join().unwrap().expect("client assertions");

    let _ = std::fs::remove_file(&log_a_path);
    let _ = std::fs::remove_file(&log_b_path);
}
