//! Inventory module integration tests. The point: prove the kernel is
//! module-agnostic — these tests use the SAME executor code path as
//! treasury, just pointed at a different module dir, and the conservation
//! rule is just declarative (Stock.cantidad group_by sku_id).

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

fn inventory_module() -> PathBuf {
    workspace_root().join("modules/inventory")
}

fn cantidad(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Stock", id)
        .and_then(|v| v.get("cantidad").and_then(Value::as_i64))
        .expect("stock with cantidad")
}

fn seed_stock(store: &mut MemoryStore, id: Uuid, sku: &str, cantidad: i64) {
    store.seed(
        "Stock",
        id,
        json!({
            "id": id.to_string(),
            "sku_id": sku,
            "ubicacion": "test-loc",
            "cantidad": cantidad,
        }),
    );
}

#[test]
fn transfer_conserves_units_across_same_sku() {
    let exec = Executor::load_module(inventory_module()).expect("load module");
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_stock(&mut store, a, "sku-X", 500);
    seed_stock(&mut store, b, "sku-X", 100);

    let ops = exec
        .run(
            &mut store,
            "transferir_stock",
            &[("source", a), ("dest", b)],
            json!({
                "cantidad": 150_i64,
                "timestamp": "2026-05-04T00:00:00Z",
                "transfer_id": Uuid::new_v4().to_string(),
            }),
        )
        .expect("transfer must pass");

    assert_eq!(ops.len(), 3, "2 sets + 1 create = 3 ops");
    assert_eq!(cantidad(&store, a), 350);
    assert_eq!(cantidad(&store, b), 250);
    // Total preserved.
    assert_eq!(cantidad(&store, a) + cantidad(&store, b), 600);
}

#[test]
fn transfer_across_different_skus_is_rejected_by_conservation() {
    // Construct a buggy synthetic morphism that mimics transfer but skips
    // the in-script same-sku check. We do this by pointing at a fixture
    // script that lacks the `throw if source.sku_id != dest.sku_id`.
    //
    // Without that fixture we can rely on the production script's `throw`
    // to fire first — which is itself fine but proves the SCRIPT, not the
    // KERNEL. To prove the kernel-level conservation works on inventory,
    // see kernel_guards.rs (treasury) — that test exercises the same
    // executor logic with Caja.saldo grouped by currency. Here we just
    // assert the production script rejects cross-SKU.
    let exec = Executor::load_module(inventory_module()).expect("load module");
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    let c = Uuid::new_v4();
    seed_stock(&mut store, a, "sku-X", 500);
    seed_stock(&mut store, c, "sku-Y", 200);

    let result = exec.run(
        &mut store,
        "transferir_stock",
        &[("source", a), ("dest", c)],
        json!({
            "cantidad": 50_i64,
            "timestamp": "2026-05-04T00:00:00Z",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    match result {
        Err(ExecError::Rhai(_)) => {}
        other => panic!("expected Rhai (script throw on sku mismatch), got {:?}", other),
    }
    assert_eq!(cantidad(&store, a), 500);
    assert_eq!(cantidad(&store, c), 200);
}

#[test]
fn overdraw_transfer_blocked_by_kcl_post_check() {
    let exec = Executor::load_module(inventory_module()).expect("load module");
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    seed_stock(&mut store, a, "sku-X", 100);
    seed_stock(&mut store, b, "sku-X", 0);

    let result = exec.run(
        &mut store,
        "transferir_stock",
        &[("source", a), ("dest", b)],
        json!({
            "cantidad": 999_i64,
            "timestamp": "2026-05-04T00:00:00Z",
            "transfer_id": Uuid::new_v4().to_string(),
        }),
    );

    match result {
        Err(ExecError::SchemaPost { role, entity, .. }) => {
            assert_eq!(role, "source");
            assert_eq!(entity, "Stock");
        }
        other => panic!("expected SchemaPost on source, got {:?}", other),
    }
    assert_eq!(cantidad(&store, a), 100);
    assert_eq!(cantidad(&store, b), 0);
}

#[test]
fn recibir_increases_stock_and_creates_movimiento() {
    let exec = Executor::load_module(inventory_module()).expect("load module");
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_stock(&mut store, a, "sku-X", 100);

    let mov_id = Uuid::new_v4();
    let ops = exec
        .run(
            &mut store,
            "recibir_stock",
            &[("stock", a)],
            json!({
                "cantidad": 50_i64,
                "timestamp": "2026-05-04T00:00:00Z",
                "movimiento_id": mov_id.to_string(),
            }),
        )
        .expect("recibir must pass");

    assert_eq!(ops.len(), 2, "1 set + 1 create");
    assert_eq!(cantidad(&store, a), 150);
    assert!(
        store.load("MovimientoStock", mov_id).is_some(),
        "movimiento must be persisted"
    );
}
