use nakui_core::event_log::{
    EventLog, ExecuteError, execute_and_log, replay, seed_and_log, verify_log,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let module_dir =
        std::env::var("NAKUI_MODULE").unwrap_or_else(|_| "modules/treasury".into());
    let exec = Executor::load_module(&module_dir).expect("load module");

    let log_path = std::env::temp_dir().join(format!("nakui_demo_{}.jsonl", Uuid::new_v4()));
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = MemoryStore::new();

    let caja_a = Uuid::new_v4();
    let caja_b = Uuid::new_v4();
    let caja_c = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut store,
        &mut log,
        "Caja",
        caja_a,
        json!({
            "id": caja_a.to_string(),
            "name": "Caja Principal",
            "saldo": 200_000_i64,
            "currency": "USD",
        }),
    )
    .expect("seed A");
    seed_and_log(
        &exec,
        &mut store,
        &mut log,
        "Caja",
        caja_b,
        json!({
            "id": caja_b.to_string(),
            "name": "Caja Chica",
            "saldo": 50_000_i64,
            "currency": "USD",
        }),
    )
    .expect("seed B");
    seed_and_log(
        &exec,
        &mut store,
        &mut log,
        "Caja",
        caja_c,
        json!({
            "id": caja_c.to_string(),
            "name": "Caja EUR",
            "saldo": 30_000_i64,
            "currency": "EUR",
        }),
    )
    .expect("seed C");

    section("== seed ==");
    print_caja(&store, "A", caja_a);
    print_caja(&store, "B", caja_b);
    print_caja(&store, "C", caja_c);

    section("== A: deposit 50_000 USD ==");
    run_and_report(
        &exec,
        &mut store,
        &mut log,
        "register_cash_move",
        &[("caja", caja_a)],
        json!({
            "monto": 50_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T12:00:00Z",
            "memo": "deposito A",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    );
    print_caja(&store, "A", caja_a);

    section("== transfer A -> B 100_000 USD ==");
    run_and_report(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", caja_a), ("dest", caja_b)],
        json!({
            "monto": 100_000_i64,
            "timestamp": "2026-05-04T12:30:00Z",
            "memo": "transferencia operativa",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );
    print_caja(&store, "A", caja_a);
    print_caja(&store, "B", caja_b);

    section("== transfer A -> B 999_999_999 USD (reject: post-check on source) ==");
    run_and_report(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", caja_a), ("dest", caja_b)],
        json!({
            "monto": 999_999_999_i64,
            "timestamp": "2026-05-04T13:00:00Z",
            "memo": "overdraw",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    section("== transfer A(USD) -> C(EUR) (reject: rhai throws) ==");
    run_and_report(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", caja_a), ("dest", caja_c)],
        json!({
            "monto": 10_000_i64,
            "timestamp": "2026-05-04T14:00:00Z",
            "memo": "USD -> EUR",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    section("== self-transfer A -> A (reject: DuplicateInputId) ==");
    run_and_report(
        &exec,
        &mut store,
        &mut log,
        "transfer_between_cajas",
        &[("source", caja_a), ("dest", caja_a)],
        json!({
            "monto": 1_000_i64,
            "timestamp": "2026-05-04T15:00:00Z",
            "memo": "self",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    section("== final live state ==");
    print_caja(&store, "A", caja_a);
    print_caja(&store, "B", caja_b);
    print_caja(&store, "C", caja_c);

    let entries = log.entries().expect("read log");
    section(&format!(
        "== log: {} entries at {} ==",
        entries.len(),
        log.path().display()
    ));
    for e in &entries {
        match e {
            nakui_core::event_log::LogEntry::Seed {
                seq, entity, id, ..
            } => println!("  #{:02} seed   {} {}", seq, entity, id),
            nakui_core::event_log::LogEntry::Morphism {
                seq,
                morphism,
                ops,
                ..
            } => println!("  #{:02} morph  {} ({} ops)", seq, morphism, ops.len()),
        }
    }

    section("== replay verification (state) ==");
    let replayed = replay(&log).expect("replay");
    if store == replayed {
        println!("  ok: replayed store byte-equal to live store");
    } else {
        println!("  MISMATCH: replay diverges from live");
    }

    section("== determinism verification (ops) ==");
    match verify_log(&log, &exec) {
        Ok(()) => println!(
            "  ok: every logged morphism reproduced its ops on re-execution"
        ),
        Err(e) => println!("  nondeterminism detected: {}", e),
    }

    if std::env::var_os("NAKUI_DEMO_KEEP").is_none() {
        let _ = std::fs::remove_file(&log_path);
    } else {
        println!("\n(NAKUI_DEMO_KEEP set — keeping log at {})", log_path.display());
    }
}

fn run_and_report(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    morphism: &str,
    inputs: &[(&str, Uuid)],
    params: serde_json::Value,
) {
    match execute_and_log(exec, store, log, morphism, inputs, params) {
        Ok(ops) => println!("  ok ({} ops, logged at #{})", ops.len(), log.next_seq() - 1),
        Err(ExecuteError::PreLog(e)) => println!("  rejected: {}", e),
        Err(ExecuteError::LogAppend(e)) => println!("  LOG APPEND FAILED: {}", e),
        Err(ExecuteError::PostLogStore(e)) => println!(
            "  POST-LOG STORE FAILED (log is canonical, store stale): {}",
            e
        ),
    }
}

fn print_caja(store: &MemoryStore, label: &str, id: Uuid) {
    let v = store.load("Caja", id).expect("caja exists");
    let saldo = v.get("saldo").and_then(|v| v.as_i64()).unwrap_or(0);
    let currency = v.get("currency").and_then(|v| v.as_str()).unwrap_or("?");
    println!("  {} {}: saldo={} {}", label, id, saldo, currency);
}

fn section(title: &str) {
    println!("\n{}", title);
}
