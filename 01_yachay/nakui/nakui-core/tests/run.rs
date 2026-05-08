//! End-to-end tests for `nakui run` — bind a socket from the main test
//! thread, drive it from a client thread with line-JSON requests, and
//! assert behaviour through the wire.
//!
//! Why server-on-main / client-on-thread: `Executor` is `!Send` (Rhai
//! caches AST in a `RefCell`). Moving it across thread boundaries is a
//! compile-time error, so the test thread runs the server and a worker
//! thread plays the client. The worker calls `shutdown` last, which lets
//! the main thread return from `run_server` and join.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use nakui_core::event_log::{EventLog, execute_and_log, seed_and_log};
use nakui_core::executor::Executor;
use nakui_core::run::run_server;
use nakui_core::store::MemoryStore;
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
    std::env::temp_dir().join(format!("nakui_run_log_{}.jsonl", Uuid::new_v4()))
}

fn fresh_socket_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_run_{}.sock", Uuid::new_v4()))
}

/// One client connection: keeps a single BufReader alive across
/// exchanges so buffered bytes from one response don't get dropped
/// before the next read.
struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Conn {
    fn connect_with_retry(path: &Path) -> Self {
        for _ in 0..100 {
            if let Ok(stream) = UnixStream::connect(path) {
                let reader_stream = stream.try_clone().expect("clone for reader");
                return Self {
                    writer: stream,
                    reader: BufReader::new(reader_stream),
                };
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("server never started accepting on {}", path.display());
    }

    fn exchange(&mut self, req: Value) -> Value {
        let mut s = serde_json::to_vec(&req).expect("serialize request");
        s.push(b'\n');
        self.writer.write_all(&s).expect("write request");
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("read response");
        assert!(n > 0, "server closed connection without responding");
        serde_json::from_str(line.trim()).expect("parse response")
    }
}

#[test]
fn run_server_full_protocol_round_trip() {
    let log_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja_id = Uuid::new_v4();
    {
        let executor = Executor::load_module(treasury_module()).expect("load module");
        let mut log = EventLog::open(&log_path).expect("open log");
        let mut store = MemoryStore::new();
        seed_and_log(
            &executor,
            &mut store,
            &mut log,
            "Caja",
            caja_id,
            json!({
                "id": caja_id.to_string(),
                "name": "A",
                "saldo": 100_000_i64,
                "currency": "USD",
            }),
        )
        .expect("seed");
    }

    let executor = Executor::load_module(treasury_module()).expect("load module");
    let log = EventLog::open(&log_path).expect("reopen log");
    let store = MemoryStore::new();

    let socket_for_client = socket_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = Conn::connect_with_retry(&socket_for_client);

        let resp = conn.exchange(json!({"op": "describe"}));
        if resp["ok"] != json!(true) {
            return Err(format!("describe not ok: {}", resp));
        }
        if resp["module"] != json!("treasury") {
            return Err(format!("module mismatch: {}", resp));
        }
        if resp["protocol"] != json!(1) {
            return Err(format!("protocol mismatch: {}", resp));
        }
        let morphisms = resp["morphisms"]
            .as_array()
            .ok_or("morphisms not array")?;
        if !morphisms.iter().any(|m| m["name"] == "register_cash_move") {
            return Err("register_cash_move missing from describe".into());
        }

        let resp = conn.exchange(json!({
            "op": "load",
            "entity": "Caja",
            "id": caja_id.to_string(),
        }));
        if resp["value"]["saldo"].as_i64() != Some(100_000) {
            return Err(format!("initial saldo wrong: {}", resp));
        }

        let resp = conn.exchange(json!({
            "op": "execute",
            "morphism": "register_cash_move",
            "inputs": {"caja": caja_id.to_string()},
            "params": {
                "monto": 5_000_i64,
                "tipo": "in",
                "timestamp": "2026-05-04T10:00:00Z",
                "memo": "via run",
                "movimiento_id": Uuid::new_v4().to_string(),
            },
        }));
        if resp["ok"] != json!(true) {
            return Err(format!("execute failed: {}", resp));
        }
        if resp["seq"].as_u64().is_none() {
            return Err(format!("execute missing seq: {}", resp));
        }
        if resp["ops"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
            return Err(format!("execute missing ops: {}", resp));
        }

        let resp = conn.exchange(json!({
            "op": "load",
            "entity": "Caja",
            "id": caja_id.to_string(),
        }));
        if resp["value"]["saldo"].as_i64() != Some(105_000) {
            return Err(format!("post-execute saldo wrong: {}", resp));
        }

        // Kernel rejection: returns ok=false with stage=pre_log.
        let other = Uuid::new_v4();
        let resp = conn.exchange(json!({
            "op": "execute",
            "morphism": "transfer_between_cajas",
            "inputs": {"source": caja_id.to_string(), "dest": other.to_string()},
            "params": {
                "monto": 999_999_999_i64,
                "timestamp": "2026-05-04T10:30:00Z",
                "memo": "overdraw",
                "transfer_id": Uuid::new_v4().to_string(),
            },
        }));
        if resp["ok"] != json!(false) || resp["stage"] != json!("pre_log") {
            return Err(format!("expected pre_log rejection: {}", resp));
        }

        // Bad JSON — connection survives, server keeps serving.
        conn.writer.write_all(b"not json\n").map_err(|e| e.to_string())?;
        let mut line = String::new();
        conn.reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let parsed: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
        if parsed["ok"] != json!(false) {
            return Err(format!("bad request didn't get error: {}", parsed));
        }

        let resp = conn.exchange(json!({"op": "verify"}));
        if resp["ok"] != json!(true) {
            return Err(format!("verify failed: {}", resp));
        }
        if resp["entries"].as_u64() != Some(2) {
            return Err(format!("verify entries wrong: {}", resp));
        }

        let resp = conn.exchange(json!({"op": "shutdown"}));
        if resp["ok"] != json!(true) || resp["shutdown"] != json!(true) {
            return Err(format!("shutdown response wrong: {}", resp));
        }
        Ok(())
    });

    run_server(executor, log, store, None, &socket_path).expect("server clean exit");
    client.join().expect("client thread joined").expect("client assertions");

    assert!(
        !socket_path.exists(),
        "shutdown must remove the socket file"
    );
    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn run_server_reconciles_drifted_store_on_startup() {
    let log_path = fresh_log_path();
    let socket_path = fresh_socket_path();

    let caja_id = Uuid::new_v4();
    {
        let executor = Executor::load_module(treasury_module()).expect("load");
        let mut log = EventLog::open(&log_path).expect("open log");
        let mut store = MemoryStore::new();
        seed_and_log(
            &executor,
            &mut store,
            &mut log,
            "Caja",
            caja_id,
            json!({
                "id": caja_id.to_string(),
                "name": "A",
                "saldo": 200_000_i64,
                "currency": "USD",
            }),
        )
        .expect("seed");
        execute_and_log(
            &executor,
            &mut store,
            &mut log,
            "register_cash_move",
            &[("caja", caja_id)],
            json!({
                "monto": 1_500_i64,
                "tipo": "in",
                "timestamp": "2026-05-04T09:00:00Z",
                "memo": "pre-run",
                "movimiento_id": Uuid::new_v4().to_string(),
            }),
        )
        .expect("deposit");
    }

    let executor = Executor::load_module(treasury_module()).expect("load");
    let log = EventLog::open(&log_path).expect("reopen");
    let empty_store = MemoryStore::new();

    let socket_for_client = socket_path.clone();
    let client = thread::spawn(move || -> Result<(), String> {
        let mut conn = Conn::connect_with_retry(&socket_for_client);
        let resp = conn.exchange(json!({
            "op": "load",
            "entity": "Caja",
            "id": caja_id.to_string(),
        }));
        if resp["value"]["saldo"].as_i64() != Some(201_500) {
            return Err(format!(
                "expected saldo 201_500 (200k seed + 1.5k replayed deposit), got {}",
                resp
            ));
        }
        conn.exchange(json!({"op": "shutdown"}));
        Ok(())
    });

    run_server(executor, log, empty_store, None, &socket_path).expect("clean exit");
    client.join().unwrap().expect("client assertions");

    let _ = std::fs::remove_file(&log_path);
}
