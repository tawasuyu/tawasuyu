//! Persistence test for SurrealStore against the RocksDB backend.
//!
//! Gated behind the `persistent` Cargo feature because RocksDB is a heavy
//! native dep (~5 min to compile cold). Run with:
//!     cargo test --features persistent --test surreal_persist

#![cfg(feature = "persistent")]

use std::path::PathBuf;

use nakui_core::store::Store;
use nakui_core::surreal_store::SurrealStore;
use serde_json::{Value, json};
use uuid::Uuid;

fn fresh_db_path() -> PathBuf {
    std::env::temp_dir().join(format!("nakui_persist_{}", Uuid::new_v4()))
}

#[test]
fn data_survives_close_and_reopen() {
    let path = fresh_db_path();
    let id = Uuid::new_v4();

    {
        let mut store = SurrealStore::new_persistent(&path).expect("open persistent");
        store.seed(
            "Caja",
            id,
            json!({
                "id": id.to_string(),
                "name": "persisted",
                "saldo": 12_345_i64,
                "currency": "USD",
            }),
        );
        // Drop store; runtime + db released.
    }

    {
        let store = SurrealStore::new_persistent(&path).expect("reopen persistent");
        let loaded = store
            .load("Caja", id)
            .expect("record must survive reopen");
        assert_eq!(
            loaded.get("saldo").and_then(Value::as_i64),
            Some(12_345),
            "saldo persisted"
        );
        assert_eq!(
            loaded.get("currency").and_then(Value::as_str),
            Some("USD"),
            "currency persisted"
        );
    }

    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn applied_ops_persist_across_reopens() {
    use nakui_core::delta::{FieldOp, FieldPath};

    let path = fresh_db_path();
    let id = Uuid::new_v4();

    {
        let mut store = SurrealStore::new_persistent(&path).expect("open");
        store.seed(
            "Caja",
            id,
            json!({"id": id.to_string(), "saldo": 100_i64, "currency": "USD"}),
        );
        store
            .apply(&[FieldOp::Set {
                path: FieldPath {
                    entity: "Caja".into(),
                    id,
                    field: "saldo".into(),
                },
                value: json!(999_i64),
            }])
            .expect("apply Set");
    }

    {
        let store = SurrealStore::new_persistent(&path).expect("reopen");
        let v = store.load("Caja", id).expect("present");
        assert_eq!(
            v.get("saldo").and_then(Value::as_i64),
            Some(999),
            "Set op persisted across restart"
        );
    }

    let _ = std::fs::remove_dir_all(&path);
}
