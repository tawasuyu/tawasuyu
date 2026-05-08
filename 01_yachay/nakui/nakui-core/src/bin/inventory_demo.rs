use nakui_core::event_log::{
    EventLog, ExecuteError, execute_and_log, replay, seed_and_log, verify_log,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let module_dir = std::env::var("NAKUI_MODULE")
        .unwrap_or_else(|_| "modules/inventory".into());
    let exec = Executor::load_module(&module_dir).expect("load module");

    let log_path =
        std::env::temp_dir().join(format!("nakui_inv_{}.jsonl", Uuid::new_v4()));
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = MemoryStore::new();

    // Two stocks of SKU "kg-cafe-honduras-2026" at warehouses A and B,
    // plus a third stock of SKU "lt-aceite-girasol" at warehouse C.
    let stock_a = Uuid::new_v4();
    let stock_b = Uuid::new_v4();
    let stock_c = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut store, &mut log, "Stock", stock_a,
        json!({
            "id": stock_a.to_string(),
            "sku_id": "kg-cafe-honduras-2026",
            "ubicacion": "almacen-norte",
            "cantidad": 500_i64,
        }),
    ).expect("seed A");
    seed_and_log(
        &exec,
        &mut store, &mut log, "Stock", stock_b,
        json!({
            "id": stock_b.to_string(),
            "sku_id": "kg-cafe-honduras-2026",
            "ubicacion": "almacen-sur",
            "cantidad": 100_i64,
        }),
    ).expect("seed B");
    seed_and_log(
        &exec,
        &mut store, &mut log, "Stock", stock_c,
        json!({
            "id": stock_c.to_string(),
            "sku_id": "lt-aceite-girasol",
            "ubicacion": "almacen-sur",
            "cantidad": 200_i64,
        }),
    ).expect("seed C");

    section("== seed ==");
    print_stock(&store, "A (cafe norte)", stock_a);
    print_stock(&store, "B (cafe sur)", stock_b);
    print_stock(&store, "C (aceite sur)", stock_c);

    section("== recibir 250 kg cafe en A ==");
    run_and_report(&exec, &mut store, &mut log, "recibir_stock",
        &[("stock", stock_a)],
        json!({
            "cantidad": 250_i64,
            "timestamp": "2026-05-04T08:00:00Z",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    );
    print_stock(&store, "A", stock_a);

    section("== transferir 200 kg cafe A -> B (conserva por sku_id) ==");
    run_and_report(&exec, &mut store, &mut log, "transferir_stock",
        &[("source", stock_a), ("dest", stock_b)],
        json!({
            "cantidad": 200_i64,
            "timestamp": "2026-05-04T09:00:00Z",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );
    print_stock(&store, "A", stock_a);
    print_stock(&store, "B", stock_b);

    section("== transferir 999_999 kg cafe A -> B (reject: stock <= 0) ==");
    run_and_report(&exec, &mut store, &mut log, "transferir_stock",
        &[("source", stock_a), ("dest", stock_b)],
        json!({
            "cantidad": 999_999_i64,
            "timestamp": "2026-05-04T10:00:00Z",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    section("== transferir 50 cafe(A) -> aceite(C) (reject: rhai SKU mismatch) ==");
    run_and_report(&exec, &mut store, &mut log, "transferir_stock",
        &[("source", stock_a), ("dest", stock_c)],
        json!({
            "cantidad": 50_i64,
            "timestamp": "2026-05-04T11:00:00Z",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    section("== final live state ==");
    print_stock(&store, "A", stock_a);
    print_stock(&store, "B", stock_b);
    print_stock(&store, "C", stock_c);

    let entries = log.entries().expect("read log");
    section(&format!(
        "== log: {} entries at {} ==",
        entries.len(),
        log.path().display()
    ));
    for e in &entries {
        match e {
            nakui_core::event_log::LogEntry::Seed { seq, entity, id, .. } =>
                println!("  #{:02} seed   {} {}", seq, entity, id),
            nakui_core::event_log::LogEntry::Morphism { seq, morphism, ops, .. } =>
                println!("  #{:02} morph  {} ({} ops)", seq, morphism, ops.len()),
        }
    }

    section("== replay verification (state) ==");
    let replayed = replay(&log).expect("replay");
    if store == replayed {
        println!("  ok: replayed store byte-equal to live store");
    } else {
        println!("  MISMATCH");
    }

    section("== determinism verification (ops) ==");
    match verify_log(&log, &exec) {
        Ok(()) => println!(
            "  ok: every logged morphism reproduced its ops on re-execution"
        ),
        Err(e) => println!("  nondeterminism detected: {}", e),
    }

    let _ = std::fs::remove_file(&log_path);
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
            "  POST-LOG STORE FAILED (log canonical, store stale): {}", e
        ),
    }
}

fn print_stock(store: &MemoryStore, label: &str, id: Uuid) {
    let v = store.load("Stock", id).expect("stock exists");
    let cantidad = v.get("cantidad").and_then(|v| v.as_i64()).unwrap_or(0);
    let sku = v.get("sku_id").and_then(|v| v.as_str()).unwrap_or("?");
    let loc = v.get("ubicacion").and_then(|v| v.as_str()).unwrap_or("?");
    println!("  {}: cantidad={} sku={} ubic={}", label, cantidad, sku, loc);
}

fn section(title: &str) {
    println!("\n{}", title);
}
