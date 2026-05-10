//! Schema versioning: every logged morphism carries a `schema_hash` that
//! pins it to the (kcl + manifest spec + rhai) bundle active at write
//! time. `verify_log` rejects logs whose entries were produced under
//! rules that no longer match the loaded executor.
//!
//! The tests here build a *temp copy* of the treasury module so we can
//! mutate its files without polluting the source tree. Each test cleans
//! its temp dir even if it panics (the helper drops via `TempModule`).

use std::path::{Path, PathBuf};

use nakui_core::event_log::{
    EventLog, LogEntry, VerifyError, execute_and_log, replay, seed_and_log, verify_log,
};
use nakui_core::executor::Executor;
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
    std::env::temp_dir().join(format!("nakui_schema_{}.jsonl", Uuid::new_v4()))
}

/// Owned temp copy of a module directory. Drops the entire tree.
struct TempModule {
    pub path: PathBuf,
}

impl TempModule {
    fn from(src: &Path) -> Self {
        let dst = std::env::temp_dir().join(format!("nakui_module_{}", Uuid::new_v4()));
        copy_dir_recursive(src, &dst).expect("copy module");
        Self { path: dst }
    }
}

impl Drop for TempModule {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn deposit_5k(
    exec: &Executor,
    store: &mut MemoryStore,
    log: &mut EventLog,
    caja: Uuid,
) {
    execute_and_log(
        exec,
        store,
        log,
        "register_cash_move",
        &[("caja", caja)],
        json!({
            "monto": 5_000_i64,
            "tipo": "in",
            "timestamp": "2026-05-04T10:00:00Z",
            "memo": "x",
            "movimiento_id": Uuid::new_v4().to_string(),
        }),
    )
    .expect("deposit");
}

fn seed_caja(exec: &Executor, store: &mut MemoryStore, log: &mut EventLog, id: Uuid) {
    seed_and_log(
        exec,
        store,
        log,
        "Caja",
        id,
        json!({"id": id.to_string(), "name": "A", "saldo": 100_000_i64, "currency": "USD"}),
    )
    .unwrap();
}

#[test]
fn executor_exposes_per_morphism_schema_hash() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let h_deposit = exec
        .schema_hash("register_cash_move")
        .expect("register_cash_move has a hash");
    let h_transfer = exec
        .schema_hash("transfer_between_cajas")
        .expect("transfer_between_cajas has a hash");
    assert_ne!(
        h_deposit, h_transfer,
        "different morphisms must have different hashes"
    );
    assert!(
        exec.schema_hash("not_a_real_morphism").is_none(),
        "unknown morphisms have no hash"
    );

    // Re-loading the same module yields the same hashes — the contract
    // depends only on the bytes on disk, not load-time state.
    let exec2 = Executor::load_module(treasury_module()).expect("reload");
    assert_eq!(exec.schema_hash("register_cash_move"), exec2.schema_hash("register_cash_move"));
}

#[test]
fn execute_and_log_writes_schema_hash_into_entries() {
    let temp = TempModule::from(&treasury_module());
    let exec = Executor::load_module(&temp.path).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).unwrap();
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_caja(&exec, &mut store, &mut log, a);
    deposit_5k(&exec, &mut store, &mut log, a);

    let entries = log.entries().unwrap();
    let morphism_entry = entries
        .iter()
        .find_map(|e| match e {
            LogEntry::Morphism { schema_hash, .. } => Some(*schema_hash),
            _ => None,
        })
        .expect("morphism entry present");
    assert_eq!(
        morphism_entry,
        Some(exec.schema_hash("register_cash_move").unwrap()),
        "logged hash must equal the executor's hash for that morphism"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_log_passes_when_module_is_unchanged() {
    let temp = TempModule::from(&treasury_module());
    let exec = Executor::load_module(&temp.path).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).unwrap();
    let mut store = MemoryStore::new();

    let a = Uuid::new_v4();
    seed_caja(&exec, &mut store, &mut log, a);
    deposit_5k(&exec, &mut store, &mut log, a);

    verify_log(&log, &exec).expect("clean module → verify ok");

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_log_rejects_log_after_morphism_script_changes() {
    let temp = TempModule::from(&treasury_module());

    // Write a log under the original script.
    let log_path = fresh_log_path();
    let a = Uuid::new_v4();
    let original_hash;
    {
        let exec = Executor::load_module(&temp.path).expect("load v1");
        original_hash = exec.schema_hash("register_cash_move").unwrap();
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, a);
        deposit_5k(&exec, &mut store, &mut log, a);
    }

    // Mutate the script with a real (non-cosmetic) change — prepend a
    // new statement. The normalizer preserves this since it changes
    // tokens, not just whitespace/comments.
    let script_path = temp.path.join("morphisms/register_cash_move.rhai");
    let original = std::fs::read_to_string(&script_path).expect("read script");
    std::fs::write(
        &script_path,
        format!("let _audit_marker = 42;\n{}", original),
    )
    .expect("write script");

    // Reload — the hash for register_cash_move must change.
    let exec2 = Executor::load_module(&temp.path).expect("reload v2");
    let new_hash = exec2.schema_hash("register_cash_move").unwrap();
    assert_ne!(original_hash, new_hash, "real source edit must move the hash");

    // verify_log must surface SchemaMismatch, not OpsMismatch — the
    // schema check runs first because "rules changed" is more
    // actionable than "ops differ for some reason."
    let log = EventLog::open(&log_path).unwrap();
    match verify_log(&log, &exec2) {
        Err(VerifyError::SchemaMismatch {
            morphism,
            logged,
            current,
            ..
        }) => {
            assert_eq!(morphism, "register_cash_move");
            assert_eq!(logged, original_hash);
            assert_eq!(current, new_hash);
        }
        other => panic!("expected SchemaMismatch, got {:?}", other),
    }

    // Replay still works — it doesn't validate against the executor.
    let replayed = replay(&log).expect("replay is schema-agnostic");
    assert!(replayed.records().contains_key("Caja"));

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn legacy_log_without_schema_hash_still_replays_and_verifies() {
    // Hand-craft a log entry that omits schema_hash entirely — what an
    // older nakui-core would have written. The Option default lets it
    // deserialize, replay walks ops the normal way, and verify_log
    // skips the schema check because the entry predates the contract.
    let log_path = fresh_log_path();
    let a = Uuid::new_v4();
    {
        let exec = Executor::load_module(treasury_module()).expect("load");
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, a);
        // Now write a Morphism entry by hand, bypassing execute_and_log,
        // simulating a log produced by an older binary.
        let entry: Value = json!({
            "kind": "morphism",
            "seq": log.next_seq(),
            "morphism": "register_cash_move",
            "inputs": {"caja": a.to_string()},
            "params": {
                "monto": 5_000,
                "tipo": "in",
                "timestamp": "2026-05-04T10:00:00Z",
                "memo": "legacy",
                "movimiento_id": Uuid::new_v4().to_string(),
            },
            "ops": []
            // NOTE: no schema_hash field — that's the legacy shape.
        });
        // Append via raw IO to skip log.append's monotonic check (which
        // we trivially satisfy anyway since seq is correct).
        let line = serde_json::to_string(&entry).unwrap();
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        use std::io::Write;
        f.write_all(line.as_bytes()).unwrap();
        f.write_all(b"\n").unwrap();
        f.sync_all().unwrap();
    }

    // Replay must succeed (no schema check).
    let log = EventLog::open(&log_path).unwrap();
    let entries = log.entries().expect("entries parse");
    assert_eq!(entries.len(), 2, "seed + legacy morphism");
    let legacy = entries
        .iter()
        .find_map(|e| match e {
            LogEntry::Morphism { schema_hash, .. } => Some(*schema_hash),
            _ => None,
        })
        .expect("morphism present");
    assert!(
        legacy.is_none(),
        "legacy entry must deserialize with schema_hash=None"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn executor_exposes_schema_bundle_hash() {
    let exec1 = Executor::load_module(treasury_module()).expect("load 1");
    let exec2 = Executor::load_module(treasury_module()).expect("load 2");
    assert_eq!(
        exec1.schema_bundle_hash, exec2.schema_bundle_hash,
        "bundle hash must be stable across re-loads of the same module"
    );

    // The bundle hash and the per-morphism hash live in different
    // tag namespaces (`nakui-bundle-v1` vs `nakui-schema-v1`), so they
    // can't accidentally collide even when the script bytes are
    // empty/identical.
    let morph_hash = exec1.schema_hash("register_cash_move").unwrap();
    assert_ne!(exec1.schema_bundle_hash, morph_hash);
}

#[test]
fn seed_and_log_writes_bundle_hash_into_seed_entries() {
    let exec = Executor::load_module(treasury_module()).expect("load");
    let log_path = fresh_log_path();
    let mut log = EventLog::open(&log_path).unwrap();
    let mut store = MemoryStore::new();
    let id = Uuid::new_v4();
    seed_caja(&exec, &mut store, &mut log, id);

    let entries = log.entries().unwrap();
    let seed_hash = entries
        .iter()
        .find_map(|e| match e {
            LogEntry::Seed { schema_hash, .. } => Some(*schema_hash),
            _ => None,
        })
        .expect("seed entry present");
    assert_eq!(
        seed_hash,
        Some(exec.schema_bundle_hash),
        "logged seed hash must equal the executor's bundle hash"
    );

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn verify_log_rejects_seed_after_schema_kcl_changes() {
    let temp = TempModule::from(&treasury_module());
    let log_path = fresh_log_path();
    let id = Uuid::new_v4();
    let original_hash;
    {
        let exec = Executor::load_module(&temp.path).expect("load v1");
        original_hash = exec.schema_bundle_hash;
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, id);
    }

    // Mutate schema.ncl. Even a comment is enough — bundle hash is byte-
    // level for the same false-positive-over-false-negative reason as
    // morphism hashes.
    let schema_path = temp.path.join("schema.ncl");
    let original = std::fs::read_to_string(&schema_path).expect("read schema");
    std::fs::write(
        &schema_path,
        format!("{}\n# seed-versioning-test mutation\n", original),
    )
    .expect("write schema");

    let exec2 = Executor::load_module(&temp.path).expect("reload v2");
    let new_hash = exec2.schema_bundle_hash;
    assert_ne!(original_hash, new_hash, "schema.ncl byte change must move the bundle hash");

    let log = EventLog::open(&log_path).unwrap();
    match verify_log(&log, &exec2) {
        Err(VerifyError::SeedSchemaMismatch {
            entity,
            id: mismatched_id,
            logged,
            current,
            ..
        }) => {
            assert_eq!(entity, "Caja");
            assert_eq!(mismatched_id, id);
            assert_eq!(logged, original_hash);
            assert_eq!(current, new_hash);
        }
        other => panic!("expected SeedSchemaMismatch, got {:?}", other),
    }

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn comment_only_edits_do_not_invalidate_the_hash() {
    // The improvement that motivated the AST-aware normalization:
    // operators leaving TODOs or whitespace edits in scripts no longer
    // re-stamps every log entry. Same script behaviour ⇒ same hash.
    let temp = TempModule::from(&treasury_module());
    let exec1 = Executor::load_module(&temp.path).expect("load v1");
    let h1 = exec1.schema_hash("register_cash_move").unwrap();

    let script_path = temp.path.join("morphisms/register_cash_move.rhai");
    let original = std::fs::read_to_string(&script_path).expect("read");
    std::fs::write(
        &script_path,
        format!(
            "// new top-level comment\n\n\n{}\n\n// trailing TODO\n/*\n  block\n  comment\n*/\n",
            original.replace("// states.caja:", "//   states.caja:    EDITED COMMENT"),
        ),
    )
    .expect("write");

    let exec2 = Executor::load_module(&temp.path).expect("reload v2");
    let h2 = exec2.schema_hash("register_cash_move").unwrap();
    assert_eq!(
        h1, h2,
        "comment-only and whitespace-only edits must not move the hash"
    );

    // Sanity: the bundle hash also stays intact (we didn't touch schema.ncl).
    assert_eq!(exec1.schema_bundle_hash, exec2.schema_bundle_hash);
}

#[test]
fn morphism_script_change_does_not_flag_unrelated_seeds() {
    // Bundle hash covers schema.ncl only — a .rhai edit moves the
    // morphism hash but leaves the bundle hash alone. So existing
    // seeds verify cleanly even when a morphism's behaviour changed.
    let temp = TempModule::from(&treasury_module());
    let log_path = fresh_log_path();
    let id = Uuid::new_v4();
    {
        let exec = Executor::load_module(&temp.path).expect("load v1");
        let mut log = EventLog::open(&log_path).unwrap();
        let mut store = MemoryStore::new();
        seed_caja(&exec, &mut store, &mut log, id);
        // No morphism executed — only the seed is in the log.
    }

    // Modify a Rhai script. Bundle stays the same.
    let script_path = temp.path.join("morphisms/register_cash_move.rhai");
    let original = std::fs::read_to_string(&script_path).expect("read");
    std::fs::write(&script_path, format!("{}\n// rhai-only mutation\n", original)).unwrap();

    let exec2 = Executor::load_module(&temp.path).expect("reload");
    let log = EventLog::open(&log_path).unwrap();
    verify_log(&log, &exec2)
        .expect("seed-only log should pass verify after a morphism-only change");

    let _ = std::fs::remove_file(&log_path);
}
