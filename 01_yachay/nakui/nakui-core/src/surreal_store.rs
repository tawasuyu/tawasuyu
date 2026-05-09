//! SurrealDB-backed `Store` implementation.
//!
//! Wraps an embedded `kv-mem` SurrealDB instance behind the same sync
//! `Store` trait the kernel uses. Each instance owns a private `tokio`
//! current-thread runtime and `block_on`s every async call.
//!
//! Why everything goes through `db.query()`:
//! SurrealDB 2.x's typed-response API (`db.upsert(thing).content(data)`)
//! deserializes responses through a serializer that is hostile to
//! `serde_json::Value` and to dynamic record shapes. Using raw SurrealQL
//! with parameter binding sidesteps that — `Response::check()` validates
//! success without forcing us to materialize the response into a typed
//! shape.
//!
//! Identity handling: SurrealDB owns record identity via a `RecordId`
//! (table:id). We strip the application-level `id` field before sending
//! and restore it on read so KCL schemas (which require `id: str`) see
//! a stable shape.

use serde_json::Value;
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};
#[cfg(feature = "persistent")]
use surrealdb::engine::local::SurrealKv;
use thiserror::Error;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::delta::FieldOp;
use crate::store::{Store, StoreError};

/// Reserved table prefix for runtime metadata that lives alongside user
/// records. Anything starting with this prefix is hidden from `iter`
/// (and therefore from `hash_state`, `dump_records`, drift detection)
/// so user-facing views never see internal bookkeeping.
const META_TABLE_PREFIX: &str = "nakui_";

/// Single-record table where `last_applied_seq` lives. Singleton id =
/// `singleton`. Wiped by `clear()` because the table prefix is part of
/// the enumeration there — a cleared store has applied nothing.
const META_TABLE: &str = "nakui_runtime_meta";
const META_SINGLETON_ID: &str = "singleton";

/// Field alias used by `iter` to surface the application-level record
/// id alongside the rest of the row, in a single per-table query. The
/// alias is stripped before the row is handed back to the caller, so
/// it never shows up in user views. Reserved — a user record with a
/// field of this name would collide and `iter` would error on UUID
/// parse failure.
const ITER_ID_ALIAS: &str = "__nakui_app_id";

#[derive(Debug, Error)]
pub enum SurrealStoreError {
    #[error("io creating tokio runtime: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("surrealdb: {0}")]
    Backend(#[from] surrealdb::Error),
}

pub struct SurrealStore {
    runtime: Runtime,
    db: Surreal<Db>,
}

impl SurrealStore {
    /// Build an in-memory SurrealDB instance (`kv-mem`). Volatile —
    /// nothing persists when the process exits.
    pub fn new_in_memory() -> Result<Self, SurrealStoreError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let db = runtime.block_on(async {
            let db = Surreal::new::<Mem>(()).await?;
            db.use_ns("nakui").use_db("default").await?;
            Ok::<_, surrealdb::Error>(db)
        })?;
        Ok(Self { runtime, db })
    }

    /// Build a SurrealKV-backed SurrealDB instance at `path`. Records
    /// survive process restarts. Requires the `persistent` Cargo feature.
    ///
    /// Reopening an existing path resumes from the persisted state — the
    /// canonical use is `let store = SurrealStore::new_persistent(path)?`
    /// at process startup, with the path stable across runs.
    #[cfg(feature = "persistent")]
    pub fn new_persistent(
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, SurrealStoreError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let path = path.as_ref().to_path_buf();
        let db = runtime.block_on(async {
            let db = Surreal::new::<SurrealKv>(path).await?;
            db.use_ns("nakui").use_db("default").await?;
            Ok::<_, surrealdb::Error>(db)
        })?;
        Ok(Self { runtime, db })
    }
}

fn strip_app_id(mut data: Value) -> Value {
    if let Value::Object(map) = &mut data {
        map.remove("id");
    }
    data
}

fn restore_app_id(mut data: Value, id: Uuid) -> Value {
    if let Value::Object(map) = &mut data {
        map.insert("id".into(), Value::String(id.to_string()));
    }
    data
}

fn json_to_map(v: Value) -> Result<serde_json::Map<String, Value>, StoreError> {
    match v {
        Value::Object(map) => Ok(map),
        _ => Err(StoreError::Backend(
            "SurrealStore expects object-shaped records".into(),
        )),
    }
}

fn map_err(e: surrealdb::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

impl Store for SurrealStore {
    fn load(&self, entity: &str, id: Uuid) -> Option<Value> {
        let entity = entity.to_string();
        let id_str = id.to_string();
        self.runtime.block_on(async {
            // `OMIT id` skips SurrealDB's Thing-typed id which serde_json::Value
            // can't represent; we restore the application id ourselves.
            let mut response = self
                .db
                .query("SELECT * OMIT id FROM type::thing($table, $id)")
                .bind(("table", entity))
                .bind(("id", id_str))
                .await
                .ok()?;
            let rows: Vec<Value> = response.take(0).ok()?;
            let row = rows.into_iter().next()?;
            Some(restore_app_id(row, id))
        })
    }

    fn seed(&mut self, entity: &str, id: Uuid, data: Value) {
        let stripped = strip_app_id(data);
        let map = json_to_map(stripped).expect("seed data is object-shaped");
        let entity = entity.to_string();
        let id_str = id.to_string();
        self.runtime.block_on(async {
            self.db
                .query("UPSERT type::thing($table, $id) CONTENT $data")
                .bind(("table", entity))
                .bind(("id", id_str))
                .bind(("data", map))
                .await
                .and_then(|r| r.check())
                .expect("seed upsert");
        });
    }

    fn apply_dry_run(&self, ops: &[FieldOp]) -> Result<(), StoreError> {
        self.runtime.block_on(async {
            for op in ops {
                match op {
                    FieldOp::Set { path, .. } | FieldOp::Clear { path } => {
                        // Set y Clear comparten la misma pre-condición:
                        // el record padre tiene que existir. Clear de
                        // un field inexistente es no-op benigno (UNSET
                        // sobre un field ausente no falla).
                        let exists = self.exists(&path.entity, path.id).await?;
                        if !exists {
                            return Err(StoreError::NotFound(
                                path.entity.clone(),
                                path.id,
                            ));
                        }
                        // We don't model NotAnObject for SurrealStore: every
                        // record stored via this trait is map-shaped by
                        // construction (json_to_map enforces it on write).
                    }
                    FieldOp::Create { entity, id, .. } => {
                        if self.exists(entity, *id).await? {
                            return Err(StoreError::Conflict(entity.clone(), *id));
                        }
                    }
                    FieldOp::Delete { entity, id } => {
                        if !self.exists(entity, *id).await? {
                            return Err(StoreError::NotFound(entity.clone(), *id));
                        }
                    }
                }
            }
            Ok(())
        })
    }

    fn iter(&self) -> Result<Box<dyn Iterator<Item = (String, Uuid, Value)> + '_>, StoreError> {
        // One query per table: pull the application id alongside every
        // other field via an alias, strip the SurrealDB-typed `id` via
        // OMIT, then restore the application `id` field in code so the
        // output is byte-identical to what `load()` produces (cross-
        // backend hash equality and the `iter ↔ load` parity contract
        // both depend on this).
        //
        // Filters runtime metadata tables (META_TABLE_PREFIX) so client
        // views never leak internal bookkeeping.
        self.runtime.block_on(async {
            let mut info = self
                .db
                .query("INFO FOR DB")
                .await
                .and_then(|r| r.check())
                .map_err(map_err)?;
            let row: Option<Value> = info.take(0).map_err(map_err)?;
            let tables: Vec<String> = row
                .as_ref()
                .and_then(|v| v.get("tables"))
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.keys()
                        .filter(|k| !k.starts_with(META_TABLE_PREFIX))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            let mut out: Vec<(String, Uuid, Value)> = Vec::new();
            for table in &tables {
                // The alias is parameterised in the SELECT clause so the
                // SurrealQL parser sees a literal field name; we can't
                // bind it as a parameter (only values bind, not
                // identifiers), but it's a compile-time constant so
                // there's no injection surface.
                let select = format!(
                    "SELECT meta::id(id) AS {alias}, * OMIT id FROM type::table($t)",
                    alias = ITER_ID_ALIAS,
                );
                let mut resp = self
                    .db
                    .query(&select)
                    .bind(("t", table.clone()))
                    .await
                    .and_then(|r| r.check())
                    .map_err(map_err)?;
                let rows: Vec<Value> = resp.take(0).map_err(map_err)?;
                for row in rows {
                    let Value::Object(mut map) = row else {
                        return Err(StoreError::Backend(format!(
                            "row in table {} is not an object",
                            table
                        )));
                    };
                    let app_id_str = match map.remove(ITER_ID_ALIAS) {
                        Some(Value::String(s)) => s,
                        _ => {
                            return Err(StoreError::Backend(format!(
                                "row in table {} missing alias `{}`",
                                table, ITER_ID_ALIAS
                            )));
                        }
                    };
                    let id = Uuid::parse_str(&app_id_str).map_err(|e| {
                        StoreError::Backend(format!(
                            "non-uuid id in table {}: {} ({})",
                            table, app_id_str, e
                        ))
                    })?;
                    // Match `restore_app_id`: insert the application id
                    // back as a regular `id: <uuid_str>` field. Callers
                    // reading the row see exactly what `load()` returns.
                    map.insert("id".into(), Value::String(app_id_str));
                    out.push((table.clone(), id, Value::Object(map)));
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_bytes().cmp(b.1.as_bytes())));
            Ok(Box::new(out.into_iter())
                as Box<dyn Iterator<Item = (String, Uuid, Value)>>)
        })
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        // Wipes EVERY table including the runtime meta table — a
        // cleared store must report `last_applied_seq() == None`.
        self.runtime.block_on(async {
            let mut info = self
                .db
                .query("INFO FOR DB")
                .await
                .and_then(|r| r.check())
                .map_err(map_err)?;
            let row: Option<Value> = info.take(0).map_err(map_err)?;
            let tables = row
                .as_ref()
                .and_then(|v| v.get("tables"))
                .and_then(|v| v.as_object());
            let names: Vec<String> = match tables {
                Some(map) => map.keys().cloned().collect(),
                None => Vec::new(),
            };
            for name in names {
                self.db
                    .query("DELETE FROM type::table($t)")
                    .bind(("t", name))
                    .await
                    .and_then(|r| r.check())
                    .map_err(map_err)?;
            }
            Ok(())
        })
    }

    fn last_applied_seq(&self) -> Result<Option<u64>, StoreError> {
        self.runtime.block_on(async {
            let mut resp = self
                .db
                .query("SELECT VALUE last_applied_seq FROM type::thing($t, $id)")
                .bind(("t", META_TABLE))
                .bind(("id", META_SINGLETON_ID))
                .await
                .and_then(|r| r.check())
                .map_err(map_err)?;
            // The query yields either zero rows (no meta record yet) or
            // one row containing the i64 value.
            let rows: Vec<i64> = resp.take(0).map_err(map_err)?;
            Ok(rows.into_iter().next().map(|v| v as u64))
        })
    }

    fn set_last_applied_seq(&mut self, seq: u64) -> Result<(), StoreError> {
        let seq_signed = seq as i64;
        self.runtime.block_on(async {
            self.db
                .query("UPSERT type::thing($t, $id) CONTENT { last_applied_seq: $seq }")
                .bind(("t", META_TABLE))
                .bind(("id", META_SINGLETON_ID))
                .bind(("seq", seq_signed))
                .await
                .and_then(|r| r.check())
                .map_err(map_err)?;
            Ok(())
        })
    }

    fn apply(&mut self, ops: &[FieldOp]) -> Result<(), StoreError> {
        self.apply_dry_run(ops)?;
        self.runtime.block_on(async {
            for op in ops {
                match op {
                    FieldOp::Set { path, value } => {
                        let mut patch = serde_json::Map::new();
                        patch.insert(path.field.clone(), value.clone());
                        self.db
                            .query("UPDATE type::thing($table, $id) MERGE $patch")
                            .bind(("table", path.entity.clone()))
                            .bind(("id", path.id.to_string()))
                            .bind(("patch", patch))
                            .await
                            .and_then(|r| r.check())
                            .map_err(map_err)?;
                    }
                    FieldOp::Clear { path } => {
                        // SurrealQL `UNSET` borra la key. El field name
                        // viene de un FieldSpec validado upstream y
                        // SurrealQL no soporta binding de identifiers
                        // (sólo valores), así que va inline. Si en
                        // el futuro se permite que el field name venga
                        // de un input no-trusted, validar aquí.
                        self.db
                            .query(format!(
                                "UPDATE type::thing($table, $id) UNSET {}",
                                path.field
                            ))
                            .bind(("table", path.entity.clone()))
                            .bind(("id", path.id.to_string()))
                            .await
                            .and_then(|r| r.check())
                            .map_err(map_err)?;
                    }
                    FieldOp::Create { entity, id, data } => {
                        let stripped = strip_app_id(data.clone());
                        let map = json_to_map(stripped)?;
                        self.db
                            .query("CREATE type::thing($table, $id) CONTENT $data")
                            .bind(("table", entity.clone()))
                            .bind(("id", id.to_string()))
                            .bind(("data", map))
                            .await
                            .and_then(|r| r.check())
                            .map_err(map_err)?;
                    }
                    FieldOp::Delete { entity, id } => {
                        self.db
                            .query("DELETE type::thing($table, $id)")
                            .bind(("table", entity.clone()))
                            .bind(("id", id.to_string()))
                            .await
                            .and_then(|r| r.check())
                            .map_err(map_err)?;
                    }
                }
            }
            Ok(())
        })
    }
}

impl SurrealStore {
    async fn exists(&self, entity: &str, id: Uuid) -> Result<bool, StoreError> {
        let mut response = self
            .db
            .query("SELECT * OMIT id FROM type::thing($table, $id)")
            .bind(("table", entity.to_string()))
            .bind(("id", id.to_string()))
            .await
            .map_err(map_err)?;
        let rows: Vec<Value> = response.take(0).map_err(map_err)?;
        Ok(!rows.is_empty())
    }
}
