//! Regression tests for the kernel's enforcement layers.
//!
//! Each test runs a deliberately-broken morphism that should be rejected by
//! a *specific* layer of the executor pipeline. After every rejection we also
//! assert the store is untouched — the kernel must never half-apply a delta.
//!
//! Layers exercised (in pipeline order):
//!   1. CapabilityViolation  (untracked write)
//!   2. ConservationViolation (delta sum != 0)
//!   3. SchemaPostCreate         (created record fails its schema)

use std::path::{Path, PathBuf};

use nakui_core::executor::{ExecError, Executor};
use nakui_core::graph::ManifestGraph;
use nakui_core::manifest::{ConserveRule, Invariants, Manifest, MorphismInput, MorphismSpec};
use nakui_core::rhai_executor::RhaiExecutor;
use nakui_core::store::{MemoryStore, Store};
use serde_json::{json, Value};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn build_executor(spec: MorphismSpec) -> Executor {
    let manifest = Manifest {
        module: "kernel_guards_test".into(),
        schemas: vec![],
        morphisms: vec![spec],
    };
    let graph = ManifestGraph::build(&manifest).expect("graph builds");
    Executor {
        manifest,
        graph,
        // module_dir is where script paths resolve; we point it at fixtures.
        module_dir: fixtures_dir(),
        // schema_path stays on the real treasury schema so we exercise the
        // production check blocks. `owned_bundle: false` so Drop leaves it
        // alone — it belongs to the source tree.
        schema_path: workspace_root().join("modules/treasury/schema.ncl"),
        rhai: RhaiExecutor::new_sandboxed(),
        owned_bundle: false,
        // Inline-built executors don't go through `load_module`, so they
        // have no schema-hash cache. These guard tests don't write to a
        // log, so verify_log never runs against this executor.
        schema_hashes: std::collections::HashMap::new(),
        schema_bundle_hash: [0u8; 32],
    }
}

fn seed_caja(store: &mut MemoryStore, id: Uuid, name: &str, saldo: i64, currency: &str) {
    store.seed(
        "Caja",
        id,
        json!({
            "id": id.to_string(),
            "name": name,
            "saldo": saldo,
            "currency": currency,
        }),
    );
}

fn caja_saldo(store: &MemoryStore, id: Uuid) -> i64 {
    store
        .load("Caja", id)
        .and_then(|v| v.get("saldo").and_then(Value::as_i64))
        .expect("caja with saldo")
}

#[test]
fn capability_violation_blocks_write_to_untracked_caja() {
    let spec = MorphismSpec {
        name: "evil_capability".into(),
        inputs: vec![MorphismInput {
            role: "caja".into(),
            entity: "Caja".into(),
            variadic: false,
        }],
        reads: vec!["caja.saldo".into()],
        writes: vec!["caja.saldo".into()],
        invariants: Invariants::default(),
        depends_on: vec![],
        script: "capability_violation.rhai".into(),
    };
    let exec = build_executor(spec);

    let mut store = MemoryStore::new();
    let caja_id = Uuid::new_v4();
    let phantom_id = Uuid::new_v4();
    seed_caja(&mut store, caja_id, "tracked", 100_000, "USD");
    seed_caja(&mut store, phantom_id, "phantom", 100_000, "USD");

    let params = json!({ "phantom_id": phantom_id.to_string() });

    let result = exec.run(&mut store, "evil_capability", &[("caja", caja_id)], params);

    match result {
        Err(ExecError::CapabilityViolation { token, .. }) => {
            assert!(
                token.contains("untracked"),
                "expected token to flag untracked id, got `{}`",
                token
            );
        }
        other => panic!("expected CapabilityViolation, got {:?}", other),
    }

    // Neither caja moved.
    assert_eq!(caja_saldo(&store, caja_id), 100_000);
    assert_eq!(caja_saldo(&store, phantom_id), 100_000);
}

#[test]
fn conservation_violation_blocks_unbalanced_transfer() {
    let spec = MorphismSpec {
        name: "evil_conservation".into(),
        inputs: vec![
            MorphismInput {
                role: "source".into(),
                entity: "Caja".into(),
                variadic: false,
            },
            MorphismInput {
                role: "dest".into(),
                entity: "Caja".into(),
                variadic: false,
            },
        ],
        reads: vec![
            "source.saldo".into(),
            "source.currency".into(),
            "dest.saldo".into(),
            "dest.currency".into(),
        ],
        writes: vec!["source.saldo".into(), "dest.saldo".into()],
        invariants: Invariants {
            conserve: vec![ConserveRule {
                entity: "Caja".into(),
                field: "saldo".into(),
                group_by: Some("currency".into()),
            }],
        },
        depends_on: vec![],
        script: "conservation_violation.rhai".into(),
    };
    let exec = build_executor(spec);

    let mut store = MemoryStore::new();
    let source = Uuid::new_v4();
    let dest = Uuid::new_v4();
    seed_caja(&mut store, source, "A", 200_000, "USD");
    seed_caja(&mut store, dest, "B", 50_000, "USD");

    let result = exec.run(
        &mut store,
        "evil_conservation",
        &[("source", source), ("dest", dest)],
        json!({}),
    );

    match result {
        Err(ExecError::ConservationViolation {
            entity,
            field,
            total,
            ..
        }) => {
            assert_eq!(entity, "Caja");
            assert_eq!(field, "saldo");
            assert_eq!(total, -101, "expected Δ = -100 + -1 = -101");
        }
        other => panic!("expected ConservationViolation, got {:?}", other),
    }

    assert_eq!(caja_saldo(&store, source), 200_000);
    assert_eq!(caja_saldo(&store, dest), 50_000);
}

#[test]
fn capability_rejects_entity_mismatch_on_tracked_id() {
    // The script writes `Stock.cantidad` using the Caja's UUID. The id is
    // tracked (it's the caja role's id) but the entity differs — the
    // capability layer must catch this regardless of UUID coincidence.
    let spec = MorphismSpec {
        name: "evil_entity_mismatch".into(),
        inputs: vec![MorphismInput {
            role: "caja".into(),
            entity: "Caja".into(),
            variadic: false,
        }],
        reads: vec!["caja.saldo".into()],
        writes: vec!["caja.saldo".into()],
        invariants: Invariants::default(),
        depends_on: vec![],
        script: "entity_mismatch.rhai".into(),
    };
    let exec = build_executor(spec);

    let mut store = MemoryStore::new();
    let caja_id = Uuid::new_v4();
    seed_caja(&mut store, caja_id, "tracked", 100_000, "USD");

    let result = exec.run(
        &mut store,
        "evil_entity_mismatch",
        &[("caja", caja_id)],
        json!({}),
    );

    match result {
        Err(ExecError::CapabilityViolation { token, .. }) => {
            assert!(
                token.contains("entity-mismatch"),
                "expected entity-mismatch token, got `{}`",
                token
            );
        }
        other => panic!("expected CapabilityViolation, got {:?}", other),
    }
    assert_eq!(caja_saldo(&store, caja_id), 100_000);
}

#[test]
fn delete_primary_skips_post_check_and_removes_record() {
    // A morphism that deletes its primary input must succeed without the
    // post-check running against a stale-then-stripped state.
    let spec = MorphismSpec {
        name: "delete_caja".into(),
        inputs: vec![MorphismInput {
            role: "caja".into(),
            entity: "Caja".into(),
            variadic: false,
        }],
        reads: vec![],
        writes: vec!["Caja".into()],
        invariants: Invariants::default(),
        depends_on: vec![],
        script: "delete_primary.rhai".into(),
    };
    let exec = build_executor(spec);

    let mut store = MemoryStore::new();
    let caja_id = Uuid::new_v4();
    seed_caja(&mut store, caja_id, "doomed", 100_000, "USD");

    let ops = exec
        .run(&mut store, "delete_caja", &[("caja", caja_id)], json!({}))
        .expect("delete must succeed");

    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], nakui_core::delta::FieldOp::Delete { .. }));
    assert!(
        store.load("Caja", caja_id).is_none(),
        "Caja must be gone after Delete"
    );
}

#[test]
fn bad_created_record_blocks_negative_movimiento() {
    let spec = MorphismSpec {
        name: "evil_create".into(),
        inputs: vec![MorphismInput {
            role: "caja".into(),
            entity: "Caja".into(),
            variadic: false,
        }],
        reads: vec!["caja.saldo".into()],
        writes: vec!["Movimiento".into()],
        invariants: Invariants::default(),
        depends_on: vec![],
        script: "bad_created_record.rhai".into(),
    };
    let exec = build_executor(spec);

    let mut store = MemoryStore::new();
    let caja_id = Uuid::new_v4();
    seed_caja(&mut store, caja_id, "A", 100_000, "USD");
    let mov_id = Uuid::new_v4();

    let params = json!({ "mov_id": mov_id.to_string() });

    let result = exec.run(&mut store, "evil_create", &[("caja", caja_id)], params);

    match result {
        Err(ExecError::SchemaPostCreate { entity, .. }) => {
            assert_eq!(entity, "Movimiento");
        }
        other => panic!("expected SchemaPostCreate, got {:?}", other),
    }

    // Caja unchanged, Movimiento never landed.
    assert_eq!(caja_saldo(&store, caja_id), 100_000);
    assert!(
        store.load("Movimiento", mov_id).is_none(),
        "Movimiento must not be persisted"
    );
}
