//! ManifestGraph: cycle detection on `depends_on`, data-flow indexes for
//! `reads`/`writes`, and the `affected_by` query that powers dirty-marking.

use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::graph::{DirtyTracker, GraphError, ManifestGraph};
use nakui_core::manifest::{
    ConserveRule, Invariants, Manifest, MorphismInput, MorphismSpec,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn module(name: &str) -> PathBuf {
    workspace_root().join("modules").join(name)
}

fn morphism(name: &str, depends_on: Vec<String>) -> MorphismSpec {
    MorphismSpec {
        name: name.into(),
        inputs: vec![MorphismInput {
            role: "caja".into(),
            entity: "Caja".into(),
        }],
        reads: vec!["caja.saldo".into()],
        writes: vec!["caja.saldo".into()],
        invariants: Invariants::default(),
        depends_on,
        script: "morphisms/register_cash_move.rhai".into(),
    }
}

fn manifest_with(morphisms: Vec<MorphismSpec>) -> Manifest {
    Manifest {
        module: "graph_test".into(),
        schemas: vec![],
        morphisms,
    }
}

#[test]
fn detects_two_node_cycle() {
    let m = manifest_with(vec![
        morphism("a", vec!["b".into()]),
        morphism("b", vec!["a".into()]),
    ]);
    match ManifestGraph::build(&m) {
        Err(GraphError::Cycle(names)) => {
            assert!(names.contains(&"a".to_string()));
            assert!(names.contains(&"b".to_string()));
        }
        other => panic!("expected Cycle, got {:?}", other),
    }
}

#[test]
fn detects_self_loop() {
    let m = manifest_with(vec![morphism("loop", vec!["loop".into()])]);
    match ManifestGraph::build(&m) {
        Err(GraphError::Cycle(names)) => {
            assert_eq!(names, vec!["loop".to_string()]);
        }
        other => panic!("expected Cycle, got {:?}", other),
    }
}

#[test]
fn detects_three_node_cycle() {
    let m = manifest_with(vec![
        morphism("a", vec!["b".into()]),
        morphism("b", vec!["c".into()]),
        morphism("c", vec!["a".into()]),
    ]);
    match ManifestGraph::build(&m) {
        Err(GraphError::Cycle(names)) => {
            assert_eq!(names.len(), 3);
        }
        other => panic!("expected Cycle, got {:?}", other),
    }
}

#[test]
fn topological_order_respects_explicit_dependencies() {
    // a <- b <- c (c depends on b depends on a)
    let m = manifest_with(vec![
        morphism("a", vec![]),
        morphism("b", vec!["a".into()]),
        morphism("c", vec!["b".into()]),
    ]);
    let g = ManifestGraph::build(&m).expect("acyclic");
    let order = g.topological_order();
    let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
    assert!(pos("a") < pos("b"));
    assert!(pos("b") < pos("c"));
}

#[test]
fn unknown_depends_on_target_errors() {
    let m = manifest_with(vec![morphism("a", vec!["ghost".into()])]);
    match ManifestGraph::build(&m) {
        Err(GraphError::UnknownMorphism(name)) => assert_eq!(name, "ghost"),
        other => panic!("expected UnknownMorphism, got {:?}", other),
    }
}

#[test]
fn treasury_data_flow_indexes_match_manifest() {
    let exec = Executor::load_module(module("treasury")).expect("load");
    let g = &exec.graph;

    // Both register_cash_move and transfer_between_cajas write Caja.saldo.
    let mut writers: Vec<&str> = g.writers_of("Caja.saldo").iter().map(|s| s.as_str()).collect();
    writers.sort();
    assert_eq!(writers, vec!["register_cash_move", "transfer_between_cajas"]);

    // Both read Caja.saldo too.
    let mut readers: Vec<&str> = g.readers_of("Caja.saldo").iter().map(|s| s.as_str()).collect();
    readers.sort();
    assert_eq!(readers, vec!["register_cash_move", "transfer_between_cajas"]);

    // Movimiento is written only by register_cash_move.
    assert_eq!(
        g.writers_of("Movimiento"),
        &["register_cash_move".to_string()]
    );

    // Transferencia is written only by transfer_between_cajas.
    assert_eq!(
        g.writers_of("Transferencia"),
        &["transfer_between_cajas".to_string()]
    );

    // Nothing in treasury reads Movimiento or Transferencia.
    assert!(g.readers_of("Movimiento").is_empty());
    assert!(g.readers_of("Transferencia").is_empty());
}

#[test]
fn affected_by_excludes_self_and_finds_overlap() {
    // A simple two-morphism manifest where one writes what the other reads.
    let m = manifest_with(vec![
        MorphismSpec {
            name: "writer".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec![],
            writes: vec!["caja.saldo".into()],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
        MorphismSpec {
            name: "reader".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec!["caja.saldo".into()],
            writes: vec![],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
        MorphismSpec {
            name: "self_loop".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec!["caja.saldo".into()],
            writes: vec!["caja.saldo".into()],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
    ]);
    let g = ManifestGraph::build(&m).expect("acyclic");

    let mut affected = g.affected_by("writer");
    affected.sort();
    // writer writes Caja.saldo; readers are reader + self_loop, but
    // self_loop is "writer"? no, self_loop is a separate morphism here,
    // and it does read Caja.saldo so it's affected by writer.
    assert_eq!(affected, vec!["reader", "self_loop"]);

    // self_loop writes its own field but should not list itself.
    let affected_self = g.affected_by("self_loop");
    assert_eq!(affected_self, vec!["reader"]);
}

#[test]
fn cross_module_graph_canonicalizes_to_entity_tokens() {
    // sales/vender uses role "stock" (entity Stock) and role "caja" (entity Caja).
    // Reads and writes should canonicalize to "Stock.cantidad" and "Caja.saldo".
    let exec = Executor::load_module(module("sales")).expect("load sales");
    let g = &exec.graph;

    assert_eq!(g.writers_of("Stock.cantidad"), &["vender".to_string()]);
    assert_eq!(g.writers_of("Caja.saldo"), &["vender".to_string()]);
    assert_eq!(g.writers_of("Venta"), &["vender".to_string()]);

    let reads = g.morphism_reads("vender");
    assert!(reads.contains(&"Stock.cantidad".to_string()));
    assert!(reads.contains(&"Caja.saldo".to_string()));
    assert!(reads.contains(&"Caja.currency".to_string()));
}

#[test]
fn executor_load_module_rejects_cyclic_manifest() {
    // Synthesize a tempdir with a cyclic manifest and confirm Executor
    // surfaces ExecError::Graph rather than running.
    let tmp = std::env::temp_dir().join(format!("nakui_cycle_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(tmp.join("morphisms")).unwrap();
    std::fs::write(
        tmp.join("schema.k"),
        "schema Caja:\n    saldo: int\n    check:\n        saldo >= 0\n",
    )
    .unwrap();
    std::fs::write(tmp.join("morphisms/op.rhai"), "[]").unwrap();
    std::fs::write(
        tmp.join("nsmc.json"),
        r#"{
            "module": "cycle",
            "morphisms": [
                {"name": "a", "inputs": [{"role":"caja","entity":"Caja"}],
                 "reads": [], "writes": ["caja.saldo"], "depends_on": ["b"],
                 "script": "morphisms/op.rhai"},
                {"name": "b", "inputs": [{"role":"caja","entity":"Caja"}],
                 "reads": [], "writes": ["caja.saldo"], "depends_on": ["a"],
                 "script": "morphisms/op.rhai"}
            ]
        }"#,
    )
    .unwrap();

    let err = match Executor::load_module(&tmp) {
        Ok(_) => panic!("must fail with cycle"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(msg.contains("graph") || msg.contains("cycle"),
        "expected graph diagnostic, got `{}`", msg);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn dirty_tracker_marks_after_treasury_morphism() {
    let exec = Executor::load_module(module("treasury")).expect("load");
    let mut tracker = DirtyTracker::new();

    // register_cash_move writes Caja.saldo + Movimiento. Both are read by
    // transfer_between_cajas (Caja.saldo) but Movimiento is read by no one.
    tracker.mark_dirty_after("register_cash_move", &exec.graph);

    let dirty = tracker.dirty();
    assert!(
        dirty.contains(&"transfer_between_cajas".to_string()),
        "transfer_between_cajas reads Caja.saldo, must be dirty after deposit; got {:?}",
        dirty
    );
    assert!(
        !tracker.is_dirty("register_cash_move"),
        "self should not be marked dirty by its own write"
    );
}

#[test]
fn dirty_tracker_clear_works() {
    let exec = Executor::load_module(module("treasury")).expect("load");
    let mut tracker = DirtyTracker::new();
    tracker.mark_dirty_after("transfer_between_cajas", &exec.graph);
    let count_before = tracker.len();
    assert!(count_before > 0);

    let first = tracker.dirty().into_iter().next().unwrap();
    tracker.clear(&first);
    assert!(!tracker.is_dirty(&first));
    assert_eq!(tracker.len(), count_before - 1);
}

#[test]
fn dirty_tracker_accumulates_across_morphisms() {
    // Manifest with three morphisms where each writes what the next reads.
    // After running A then B, both readers should be marked.
    let m = manifest_with(vec![
        MorphismSpec {
            name: "writer_a".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec![],
            writes: vec!["caja.saldo".into()],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
        MorphismSpec {
            name: "writer_b".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec![],
            writes: vec!["Movimiento".into()],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
        MorphismSpec {
            name: "reader_caja".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec!["caja.saldo".into()],
            writes: vec![],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
        MorphismSpec {
            name: "reader_mov".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec!["Movimiento".into()],
            writes: vec![],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/register_cash_move.rhai".into(),
        },
    ]);
    let g = ManifestGraph::build(&m).unwrap();
    let mut tracker = DirtyTracker::new();

    tracker.mark_dirty_after("writer_a", &g);
    assert!(tracker.is_dirty("reader_caja"));
    assert!(!tracker.is_dirty("reader_mov"));

    tracker.mark_dirty_after("writer_b", &g);
    assert!(tracker.is_dirty("reader_caja"));
    assert!(tracker.is_dirty("reader_mov"));

    assert_eq!(tracker.len(), 2);
}
