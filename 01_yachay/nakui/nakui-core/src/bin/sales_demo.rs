//! Cross-module demo: a `vender` morphism that touches a Stock entity
//! (defined in inventory's schema) and a Caja entity (defined in
//! treasury's schema). The sales module's `nsmc.json` lists three schema
//! files; the executor concatenates them at load time so KCL validates
//! against all three.

use nakui_core::event_log::{
    EventLog, ExecuteError, execute_and_log, replay, seed_and_log, verify_log,
};
use nakui_core::executor::Executor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::json;
use uuid::Uuid;

fn main() {
    let module_dir = std::env::var("NAKUI_MODULE")
        .unwrap_or_else(|_| "modules/sales".into());
    let exec = Executor::load_module(&module_dir).expect("load module");

    let log_path =
        std::env::temp_dir().join(format!("nakui_sales_{}.jsonl", Uuid::new_v4()));
    let mut log = EventLog::open(&log_path).expect("open log");
    let mut store = MemoryStore::new();

    let stock_id = Uuid::new_v4();
    let caja_id = Uuid::new_v4();
    seed_and_log(
        &exec,
        &mut store, &mut log, "Stock", stock_id,
        json!({
            "id": stock_id.to_string(),
            "sku_id": "kg-cafe-honduras-2026",
            "ubicacion": "almacen-norte",
            "cantidad": 500_i64,
        }),
    ).expect("seed stock");
    seed_and_log(
        &exec,
        &mut store, &mut log, "Caja", caja_id,
        json!({
            "id": caja_id.to_string(),
            "name": "Caja Principal",
            "saldo": 1_000_000_i64,  // $10_000.00 in cents
            "currency": "USD",
        }),
    ).expect("seed caja");

    section("== seed ==");
    print_stock(&store, "stock", stock_id);
    print_caja(&store, "caja", caja_id);

    // 1. Sell 100 kg cafe at $50.00 / kg = $5000.00 total.
    section("== vender 100 kg @ $50.00 c/u ==");
    run_and_report(&exec, &mut store, &mut log, "vender",
        &[("stock", stock_id), ("caja", caja_id)],
        json!({
            "cantidad": 100_i64,
            "precio_unitario": 5_000_i64,    // $50.00 in cents
            "timestamp": "2026-05-04T10:00:00Z",
            "venta_id": Uuid::new_v4().to_string(),
        }),
    );
    print_stock(&store, "stock", stock_id);
    print_caja(&store, "caja", caja_id);

    // 2. Try selling more than available stock — should fail Stock post-check.
    section("== vender 9999 kg (reject: stock <= 0) ==");
    run_and_report(&exec, &mut store, &mut log, "vender",
        &[("stock", stock_id), ("caja", caja_id)],
        json!({
            "cantidad": 9999_i64,
            "precio_unitario": 1_000_i64,
            "timestamp": "2026-05-04T11:00:00Z",
            "venta_id": Uuid::new_v4().to_string(),
        }),
    );

    // 3. Negative price — caught by Rhai.
    section("== vender con precio negativo (reject: rhai throw) ==");
    run_and_report(&exec, &mut store, &mut log, "vender",
        &[("stock", stock_id), ("caja", caja_id)],
        json!({
            "cantidad": 10_i64,
            "precio_unitario": -100_i64,
            "timestamp": "2026-05-04T11:30:00Z",
            "venta_id": Uuid::new_v4().to_string(),
        }),
    );

    // 4. Another good sale.
    section("== vender 50 kg @ $60.00 c/u ==");
    run_and_report(&exec, &mut store, &mut log, "vender",
        &[("stock", stock_id), ("caja", caja_id)],
        json!({
            "cantidad": 50_i64,
            "precio_unitario": 6_000_i64,
            "timestamp": "2026-05-04T12:00:00Z",
            "venta_id": Uuid::new_v4().to_string(),
        }),
    );
    print_stock(&store, "stock", stock_id);
    print_caja(&store, "caja", caja_id);

    section("== final live state ==");
    print_stock(&store, "stock", stock_id);
    print_caja(&store, "caja", caja_id);

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
    println!("  {} cantidad={} sku={}", label, cantidad, sku);
}

fn print_caja(store: &MemoryStore, label: &str, id: Uuid) {
    let v = store.load("Caja", id).expect("caja exists");
    let saldo = v.get("saldo").and_then(|v| v.as_i64()).unwrap_or(0);
    let cur = v.get("currency").and_then(|v| v.as_str()).unwrap_or("?");
    println!("  {} saldo={} {} (en centavos)", label, saldo, cur);
}

fn section(title: &str) {
    println!("\n{}", title);
}
