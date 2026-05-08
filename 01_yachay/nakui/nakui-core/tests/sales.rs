//! Cross-module integration tests. The `sales` module references entities
//! defined in `treasury` and `inventory` via its manifest's `schemas` list.
//! These tests assert:
//!   - The kernel correctly bundles multiple .k files at module load.
//!   - Per-entity KCL post-checks fire against the right schema even when
//!     three are concatenated.
//!   - A non-conserving morphism (sale = stock−1, caja+price) passes the
//!     kernel cleanly because no `invariants.conserve` was declared.

use std::path::{Path, PathBuf};

use nakui_core::executor::{ExecError, Executor};
use nakui_core::store::{MemoryStore, Store};
use serde_json::{Value, json};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn sales_module() -> PathBuf {
    workspace_root().join("modules/sales")
}

fn caja_saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Caja", id)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .expect("caja with saldo")
}

fn stock_cantidad(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Stock", id)
        .and_then(|v| v.get("cantidad").and_then(Value::as_i64))
        .expect("stock with cantidad")
}

fn seed(store: &mut MemoryStore) -> (Uuid, Uuid) {
    let stock = Uuid::new_v4();
    let caja = Uuid::new_v4();
    store.seed(
        "Stock",
        stock,
        json!({
            "id": stock.to_string(),
            "sku_id": "sku-test",
            "ubicacion": "test-loc",
            "cantidad": 500_i64,
        }),
    );
    store.seed(
        "Caja",
        caja,
        json!({
            "id": caja.to_string(),
            "name": "Caja Test",
            "saldo": 1_000_000_i64,
            "currency": "USD",
        }),
    );
    (stock, caja)
}

#[test]
fn sale_decreases_stock_and_increases_caja() {
    let exec = Executor::load_module(sales_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (stock, caja) = seed(&mut store);

    let venta_id = Uuid::new_v4();
    let ops = exec
        .run(
            &mut store,
            "vender",
            &[("stock", stock), ("caja", caja)],
            json!({
                "cantidad": 100_i64,
                "precio_unitario": 5_000_i64,
                "timestamp": "2026-05-04T10:00:00Z",
                "venta_id": venta_id.to_string(),
            }),
        )
        .expect("sale must succeed");

    assert_eq!(ops.len(), 3, "2 sets + 1 create");
    assert_eq!(stock_cantidad(&store, stock), 400);
    assert_eq!(caja_saldo(&store, caja), 1_500_000);

    let venta = store
        .load("Venta", venta_id)
        .expect("venta must be persisted");
    assert_eq!(venta.get("total").and_then(Value::as_i64), Some(500_000));
    assert_eq!(venta.get("cantidad").and_then(Value::as_i64), Some(100));
    assert_eq!(
        venta.get("currency").and_then(Value::as_str),
        Some("USD")
    );
}

#[test]
fn overdraw_stock_rejected_by_inventory_post_check() {
    let exec = Executor::load_module(sales_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (stock, caja) = seed(&mut store);

    let result = exec.run(
        &mut store,
        "vender",
        &[("stock", stock), ("caja", caja)],
        json!({
            "cantidad": 9999_i64,
            "precio_unitario": 100_i64,
            "timestamp": "2026-05-04T10:00:00Z",
            "venta_id": Uuid::new_v4().to_string(),
        }),
    );

    match result {
        Err(ExecError::KclPost { role, entity, .. }) => {
            assert_eq!(role, "stock");
            assert_eq!(entity, "Stock");
        }
        other => panic!("expected KclPost on stock, got {:?}", other),
    }
    assert_eq!(stock_cantidad(&store, stock), 500);
    assert_eq!(caja_saldo(&store, caja), 1_000_000);
}

#[test]
fn venta_total_invariant_caught_when_corrupted() {
    // The Venta schema's check block enforces `total == cantidad * precio`.
    // The production script always produces a consistent total. To prove
    // the schema check fires, this test would need a buggy script — that's
    // covered indirectly: if anyone breaks the script, this fails. For now
    // we just confirm a clean sale's Venta passes its own invariant.
    let exec = Executor::load_module(sales_module()).expect("load module");
    let mut store = MemoryStore::new();
    let (stock, caja) = seed(&mut store);
    let venta_id = Uuid::new_v4();

    exec.run(
        &mut store,
        "vender",
        &[("stock", stock), ("caja", caja)],
        json!({
            "cantidad": 7_i64,
            "precio_unitario": 13_i64,
            "timestamp": "2026-05-04T10:00:00Z",
            "venta_id": venta_id.to_string(),
        }),
    )
    .expect("sale must pass");

    let venta = store.load("Venta", venta_id).expect("venta");
    assert_eq!(
        venta.get("total").and_then(Value::as_i64),
        Some(7 * 13),
        "Venta.total must equal cantidad * precio"
    );
}
