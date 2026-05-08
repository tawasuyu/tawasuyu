//! Manifest::validate covers the contract between authors (humans or AI)
//! and Nakui. Each test inline-builds a manifest with one specific defect
//! and asserts the right diagnostic fires.
//!
//! Most tests point at `modules/treasury/` so the schema/script paths
//! resolve. Two tests need a synthetic tempdir to express their defect
//! (missing schema file, duplicate schema across files).

use std::fs;
use std::path::{Path, PathBuf};

use nakui_core::executor::Executor;
use nakui_core::manifest::{
    ConserveRule, Invariants, Manifest, MorphismInput, MorphismSpec, ValidationError,
};
use uuid::Uuid;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root above core/")
        .to_path_buf()
}

fn treasury_dir() -> PathBuf {
    workspace_root().join("modules/treasury")
}

fn caja_input() -> MorphismInput {
    MorphismInput {
        role: "caja".into(),
        entity: "Caja".into(),
    }
}

fn baseline_morphism() -> MorphismSpec {
    MorphismSpec {
        name: "test_op".into(),
        inputs: vec![caja_input()],
        reads: vec!["caja.saldo".into()],
        writes: vec!["caja.saldo".into()],
        invariants: Invariants::default(),
        depends_on: vec![],
        script: "morphisms/register_cash_move.rhai".into(),
    }
}

fn baseline_manifest() -> Manifest {
    Manifest {
        module: "test".into(),
        schemas: vec![],
        morphisms: vec![baseline_morphism()],
    }
}

#[test]
fn production_modules_validate_clean() {
    for name in ["treasury", "inventory", "sales"] {
        let dir = workspace_root().join("modules").join(name);
        let manifest = Manifest::load(&dir.join("nsmc.json"))
            .unwrap_or_else(|e| panic!("load {}: {}", name, e));
        manifest
            .validate(&dir)
            .unwrap_or_else(|e| panic!("validate {}: {}", name, e));
    }
}

#[test]
fn rejects_duplicate_morphism_name() {
    let mut m = baseline_manifest();
    m.morphisms.push(baseline_morphism()); // same name as the first
    match m.validate(&treasury_dir()) {
        Err(ValidationError::DuplicateMorphism(name)) => assert_eq!(name, "test_op"),
        other => panic!("expected DuplicateMorphism, got {:?}", other),
    }
}

#[test]
fn rejects_duplicate_role_within_morphism() {
    let mut m = baseline_manifest();
    m.morphisms[0].inputs.push(caja_input()); // same role twice
    match m.validate(&treasury_dir()) {
        Err(ValidationError::DuplicateRole { morphism, role }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(role, "caja");
        }
        other => panic!("expected DuplicateRole, got {:?}", other),
    }
}

#[test]
fn rejects_input_unknown_entity() {
    let mut m = baseline_manifest();
    m.morphisms[0].inputs[0].entity = "Banana".into();
    match m.validate(&treasury_dir()) {
        Err(ValidationError::InputUnknownEntity {
            morphism,
            entity,
            known,
        }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(entity, "Banana");
            assert!(known.contains(&"Caja".to_string()));
        }
        other => panic!("expected InputUnknownEntity, got {:?}", other),
    }
}

#[test]
fn rejects_writes_unknown_role() {
    let mut m = baseline_manifest();
    m.morphisms[0].writes = vec!["ghost.saldo".into()];
    match m.validate(&treasury_dir()) {
        Err(ValidationError::WritesUnknownRole {
            morphism,
            token,
            role,
            ..
        }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(token, "ghost.saldo");
            assert_eq!(role, "ghost");
        }
        other => panic!("expected WritesUnknownRole, got {:?}", other),
    }
}

#[test]
fn rejects_writes_unknown_entity() {
    let mut m = baseline_manifest();
    m.morphisms[0].writes = vec!["BananaSplit".into()];
    match m.validate(&treasury_dir()) {
        Err(ValidationError::WritesUnknownEntity { morphism, token }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(token, "BananaSplit");
        }
        other => panic!("expected WritesUnknownEntity, got {:?}", other),
    }
}

#[test]
fn rejects_conserve_unknown_entity() {
    let mut m = baseline_manifest();
    m.morphisms[0].invariants.conserve = vec![ConserveRule {
        entity: "Banana".into(),
        field: "x".into(),
        group_by: None,
    }];
    match m.validate(&treasury_dir()) {
        Err(ValidationError::ConserveUnknownEntity { morphism, entity }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(entity, "Banana");
        }
        other => panic!("expected ConserveUnknownEntity, got {:?}", other),
    }
}

#[test]
fn rejects_depends_on_unknown_morphism() {
    let mut m = baseline_manifest();
    m.morphisms[0].depends_on = vec!["ghost_morphism".into()];
    match m.validate(&treasury_dir()) {
        Err(ValidationError::DependsOnUnknown { morphism, dep }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(dep, "ghost_morphism");
        }
        other => panic!("expected DependsOnUnknown, got {:?}", other),
    }
}

#[test]
fn rejects_missing_script() {
    let mut m = baseline_manifest();
    m.morphisms[0].script = "morphisms/ghost.rhai".into();
    match m.validate(&treasury_dir()) {
        Err(ValidationError::ScriptMissing { morphism, script, .. }) => {
            assert_eq!(morphism, "test_op");
            assert_eq!(script, "morphisms/ghost.rhai");
        }
        other => panic!("expected ScriptMissing, got {:?}", other),
    }
}

#[test]
fn rejects_missing_schema_file() {
    let mut m = baseline_manifest();
    m.schemas = vec!["nonexistent.k".into()];
    match m.validate(&treasury_dir()) {
        Err(ValidationError::SchemaFileMissing { path, .. }) => {
            assert_eq!(path, "nonexistent.k");
        }
        other => panic!("expected SchemaFileMissing, got {:?}", other),
    }
}

#[test]
fn rejects_duplicate_schema_across_files() {
    // Synthesize a tempdir with two .k files that both declare `schema X`.
    let tmp = std::env::temp_dir().join(format!("nakui_dup_{}", Uuid::new_v4()));
    fs::create_dir_all(&tmp).unwrap();
    fs::create_dir_all(tmp.join("morphisms")).unwrap();
    fs::write(
        tmp.join("a.k"),
        "schema Caja:\n    saldo: int\n    check:\n        saldo >= 0\n",
    )
    .unwrap();
    fs::write(
        tmp.join("b.k"),
        "schema Caja:\n    monto: int\n    check:\n        monto >= 0\n",
    )
    .unwrap();
    fs::write(tmp.join("morphisms/op.rhai"), "[]").unwrap();

    let m = Manifest {
        module: "dup".into(),
        schemas: vec!["a.k".into(), "b.k".into()],
        morphisms: vec![MorphismSpec {
            name: "op".into(),
            inputs: vec![MorphismInput {
                role: "caja".into(),
                entity: "Caja".into(),
            }],
            reads: vec![],
            writes: vec![],
            invariants: Invariants::default(),
            depends_on: vec![],
            script: "morphisms/op.rhai".into(),
        }],
    };

    match m.validate(&tmp) {
        Err(ValidationError::DuplicateSchema { name, files }) => {
            assert_eq!(name, "Caja");
            assert!(files.contains(&"a.k".to_string()));
            assert!(files.contains(&"b.k".to_string()));
        }
        other => panic!("expected DuplicateSchema, got {:?}", other),
    }

    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn executor_load_module_runs_validation() {
    // Synthesize a module dir whose manifest references a missing script —
    // load_module must surface ManifestValidation, not a runtime kernel error.
    let tmp = std::env::temp_dir().join(format!("nakui_bad_{}", Uuid::new_v4()));
    fs::create_dir_all(&tmp).unwrap();
    fs::write(
        tmp.join("schema.k"),
        "schema Caja:\n    saldo: int\n    check:\n        saldo >= 0\n",
    )
    .unwrap();
    fs::write(
        tmp.join("nsmc.json"),
        r#"{
            "module": "bad",
            "morphisms": [{
                "name": "op",
                "inputs": [{"role": "caja", "entity": "Caja"}],
                "reads": [],
                "writes": ["caja.saldo"],
                "depends_on": [],
                "script": "morphisms/missing.rhai"
            }]
        }"#,
    )
    .unwrap();

    let err = match Executor::load_module(&tmp) {
        Ok(_) => panic!("must fail validation"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("validation") && msg.contains("missing.rhai"),
        "expected validation diagnostic naming the missing script, got `{}`",
        msg
    );

    let _ = fs::remove_dir_all(&tmp);
}
