use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

use crate::delta::FieldOp;

#[derive(Debug, Clone, Error)]
pub enum StoreError {
    #[error("entity {0} id {1} not found")]
    NotFound(String, Uuid),
    #[error("entity {0} id {1} already exists")]
    Conflict(String, Uuid),
    #[error("set on non-object record at {0} {1}")]
    NotAnObject(String, Uuid),
    /// Backend-specific transient or systemic failure (network, disk,
    /// driver). Distinct from the data-shape errors above.
    #[error("backend error: {0}")]
    Backend(String),
}

pub trait Store {
    fn load(&self, entity: &str, id: Uuid) -> Option<Value>;

    /// Insert or replace a record without going through the morphism
    /// pipeline. Represents external/boundary input — the source of
    /// records that didn't originate from a kernel-validated event.
    fn seed(&mut self, entity: &str, id: Uuid, data: Value);

    /// Read-only check: would `apply(ops)` succeed against current state?
    /// Does NOT mutate. The kernel runs this as the last step of `compute`
    /// so that, by the time we log an event, the apply is contractually
    /// guaranteed to land.
    fn apply_dry_run(&self, ops: &[FieldOp]) -> Result<(), StoreError>;

    fn apply(&mut self, ops: &[FieldOp]) -> Result<(), StoreError>;

    /// Drop every record. Used by `reconcile` to wipe a stale store before
    /// replaying the log. Must leave the store in the same state it would
    /// be in immediately after construction. Implementors that override
    /// `last_applied_seq` must reset that marker here too — a cleared
    /// store has applied nothing.
    fn clear(&mut self) -> Result<(), StoreError>;

    /// The last log seq whose effects are reflected in this store, if
    /// the store can persist that fact. Default `Ok(None)` covers
    /// transient backends. The startup path uses this to skip the full
    /// replay when the store is verifiably already in sync with the log.
    fn last_applied_seq(&self) -> Result<Option<u64>, StoreError> {
        Ok(None)
    }

    /// Persist the marker after a successful apply / seed / replay.
    /// Best-effort: callers ignore failures here because a stale marker
    /// only costs an extra full replay on next startup, never
    /// correctness — full replay starts with `clear()`, so it tolerates
    /// any prior state. Default impl is a no-op for transient backends.
    fn set_last_applied_seq(&mut self, _seq: u64) -> Result<(), StoreError> {
        Ok(())
    }

    /// Enumerate every record in canonical order: sorted first by entity
    /// name, then by id bytes. The canonical order is what makes
    /// `hash_state` reproducible — without it two stores with the same
    /// records would hash differently depending on insertion order.
    ///
    /// Returns owned `Value`s. For an in-memory backend this clones; for
    /// a remote backend it materializes a snapshot. V1 chooses simplicity
    /// over streaming — the hash and drift-comparison use cases need to
    /// see all records anyway, and an iterator over a Vec keeps the
    /// trait method object-safe and free of async lifetime concerns.
    fn iter(&self) -> Result<Box<dyn Iterator<Item = (String, Uuid, Value)> + '_>, StoreError>;

    /// Deterministic SHA-256 of the store's full state. Two stores with
    /// the same records (regardless of how they got there or which
    /// backend they live in) produce the same hash; any drift produces
    /// a different one. The default impl is the contract — backends
    /// should only override it for backend-native acceleration (e.g.
    /// server-side table digests), and an override must produce the
    /// same bytes as the default.
    ///
    /// Framing per record:
    ///   entity_bytes | 0x00 | id_bytes | 0x00 | canonical_value_hash
    /// The length prefix on entity/id prevents (entity="ab", id="c")
    /// from colliding with (entity="a", id="bc"). The value bytes are
    /// produced by `hash_value`, which walks the JSON tree with
    /// type-tagged framing — that decouples the hash from
    /// `serde_json::to_vec`'s representation choices (especially
    /// integer-valued floats vs ints) so cross-backend comparison
    /// works.
    fn hash_state(&self) -> Result<[u8; 32], StoreError> {
        let mut hasher = Sha256::new();
        for (entity, id, value) in self.iter()? {
            hasher.update(entity.as_bytes());
            hasher.update([0u8]);
            hasher.update(id.as_bytes());
            hasher.update([0u8]);
            hash_value(&mut hasher, &value);
        }
        Ok(hasher.finalize().into())
    }
}

/// Canonical hash of a `serde_json::Value`. Type-tagged so a string
/// "true" can't collide with the boolean `true`; length-prefixed so
/// concatenation can't shift bytes between fields. Numbers normalize:
/// any integer-valued number (i64, u64, or a finite f64 with no
/// fractional part) is hashed as an i128 — that's what makes
/// cross-backend equality work, since SurrealDB may round-trip
/// what the caller wrote as `100_i64` back as the same numeric value
/// without us needing to commit to a wire-format-specific
/// representation.
pub fn hash_value(hasher: &mut Sha256, v: &Value) {
    match v {
        Value::Null => hasher.update([TAG_NULL]),
        Value::Bool(b) => {
            hasher.update([TAG_BOOL]);
            hasher.update([*b as u8]);
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                hash_int(hasher, i as i128);
            } else if let Some(u) = n.as_u64() {
                hash_int(hasher, u as i128);
            } else if let Some(f) = n.as_f64() {
                // Integer-valued floats canonicalize to int. Anything
                // else (fractions, NaN, infinities) hashes as the raw
                // f64 bit pattern — that's still deterministic, just
                // not normalized.
                if f.is_finite()
                    && f.fract() == 0.0
                    && f >= I128_MIN_AS_F64
                    && f <= I128_MAX_AS_F64
                {
                    hash_int(hasher, f as i128);
                } else {
                    hasher.update([TAG_FLOAT]);
                    hasher.update(f.to_bits().to_le_bytes());
                }
            } else {
                // serde_json::Number guarantees one of the above; this
                // branch only fires if a future variant appears.
                hasher.update([TAG_FLOAT]);
                hasher.update(f64::NAN.to_bits().to_le_bytes());
            }
        }
        Value::String(s) => {
            hasher.update([TAG_STRING]);
            hasher.update((s.len() as u64).to_le_bytes());
            hasher.update(s.as_bytes());
        }
        Value::Array(arr) => {
            hasher.update([TAG_ARRAY]);
            hasher.update((arr.len() as u64).to_le_bytes());
            for item in arr {
                hash_value(hasher, item);
            }
        }
        Value::Object(map) => {
            hasher.update([TAG_OBJECT]);
            hasher.update((map.len() as u64).to_le_bytes());
            // serde_json::Map without `preserve_order` is BTreeMap
            // (alphabetical). We sort defensively in case the build
            // pulls in `preserve_order` transitively from a future dep.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for k in keys {
                hasher.update((k.len() as u64).to_le_bytes());
                hasher.update(k.as_bytes());
                hash_value(hasher, &map[k]);
            }
        }
    }
}

fn hash_int(hasher: &mut Sha256, n: i128) {
    hasher.update([TAG_INT]);
    hasher.update(n.to_le_bytes());
}

const TAG_NULL: u8 = 0;
const TAG_BOOL: u8 = 1;
const TAG_INT: u8 = 2;
const TAG_FLOAT: u8 = 3;
const TAG_STRING: u8 = 4;
const TAG_ARRAY: u8 = 5;
const TAG_OBJECT: u8 = 6;

// f64 can't represent i128::MAX exactly; the cast truncates upward to
// the next representable f64. Use those as the comparison bounds so
// `f as i128` stays well-defined.
const I128_MIN_AS_F64: f64 = -1.7014118346046923e38;
const I128_MAX_AS_F64: f64 = 1.7014118346046923e38;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct MemoryStore {
    records: HashMap<String, HashMap<Uuid, Value>>,
    /// Last log seq whose effects are reflected here. In-process only —
    /// resets to `None` on construction or `clear`. The skip-replay
    /// optimization in `nakui run` benefits the persistent backends;
    /// for `MemoryStore` it's harmless bookkeeping (process restart =
    /// new store = `None`, which forces full replay).
    last_applied: Option<u64>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the internal records map. Used by `Snapshot::from_memory_store`
    /// to capture state for snapshot persistence.
    pub fn records(&self) -> &HashMap<String, HashMap<Uuid, Value>> {
        &self.records
    }
}

impl Store for MemoryStore {
    fn load(&self, entity: &str, id: Uuid) -> Option<Value> {
        self.records.get(entity)?.get(&id).cloned()
    }

    fn seed(&mut self, entity: &str, id: Uuid, data: Value) {
        self.records
            .entry(entity.to_string())
            .or_default()
            .insert(id, data);
    }

    fn apply_dry_run(&self, ops: &[FieldOp]) -> Result<(), StoreError> {
        for op in ops {
            match op {
                FieldOp::Set { path, .. } => {
                    match self.records.get(&path.entity).and_then(|m| m.get(&path.id)) {
                        None => {
                            return Err(StoreError::NotFound(path.entity.clone(), path.id));
                        }
                        Some(Value::Object(_)) => {}
                        Some(_) => {
                            return Err(StoreError::NotAnObject(path.entity.clone(), path.id));
                        }
                    }
                }
                FieldOp::Create { entity, id, .. } => {
                    if self
                        .records
                        .get(entity)
                        .and_then(|m| m.get(id))
                        .is_some()
                    {
                        return Err(StoreError::Conflict(entity.clone(), *id));
                    }
                }
                FieldOp::Delete { entity, id } => {
                    if self
                        .records
                        .get(entity)
                        .and_then(|m| m.get(id))
                        .is_none()
                    {
                        return Err(StoreError::NotFound(entity.clone(), *id));
                    }
                }
            }
        }
        Ok(())
    }

    fn apply(&mut self, ops: &[FieldOp]) -> Result<(), StoreError> {
        self.apply_dry_run(ops)?;
        for op in ops {
            match op {
                FieldOp::Set { path, value } => {
                    let rec = self
                        .records
                        .get_mut(&path.entity)
                        .and_then(|m| m.get_mut(&path.id))
                        .expect("validated by dry_run");
                    let map = match rec {
                        Value::Object(m) => m,
                        _ => unreachable!("dry_run guards against non-object"),
                    };
                    map.insert(path.field.clone(), value.clone());
                }
                FieldOp::Create { entity, id, data } => {
                    self.records
                        .entry(entity.clone())
                        .or_default()
                        .insert(*id, data.clone());
                }
                FieldOp::Delete { entity, id } => {
                    self.records
                        .get_mut(entity)
                        .expect("validated by dry_run")
                        .remove(id);
                }
            }
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), StoreError> {
        self.records.clear();
        self.last_applied = None;
        Ok(())
    }

    fn last_applied_seq(&self) -> Result<Option<u64>, StoreError> {
        Ok(self.last_applied)
    }

    fn set_last_applied_seq(&mut self, seq: u64) -> Result<(), StoreError> {
        self.last_applied = Some(seq);
        Ok(())
    }

    fn iter(&self) -> Result<Box<dyn Iterator<Item = (String, Uuid, Value)> + '_>, StoreError> {
        let mut out: Vec<(String, Uuid, Value)> = self
            .records
            .iter()
            .flat_map(|(entity, m)| {
                m.iter()
                    .map(move |(id, v)| (entity.clone(), *id, v.clone()))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.as_bytes().cmp(b.1.as_bytes())));
        Ok(Box::new(out.into_iter()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delta::{FieldOp, FieldPath};
    use serde_json::json;

    #[test]
    fn dry_run_rejects_set_on_non_object() {
        let mut store = MemoryStore::new();
        let id = Uuid::new_v4();
        store.seed("Caja", id, json!(42)); // not an object
        let op = FieldOp::Set {
            path: FieldPath {
                entity: "Caja".into(),
                id,
                field: "saldo".into(),
            },
            value: json!(100),
        };
        match store.apply_dry_run(&[op.clone()]) {
            Err(StoreError::NotAnObject(e, i)) => {
                assert_eq!(e, "Caja");
                assert_eq!(i, id);
            }
            other => panic!("expected NotAnObject, got {:?}", other),
        }
        // apply must reject too without panicking.
        assert!(matches!(
            store.apply(&[op]),
            Err(StoreError::NotAnObject(_, _))
        ));
    }

    #[test]
    fn dry_run_rejects_create_conflict() {
        let mut store = MemoryStore::new();
        let id = Uuid::new_v4();
        store.seed("Caja", id, json!({"id": id.to_string()}));
        let op = FieldOp::Create {
            entity: "Caja".into(),
            id,
            data: json!({"id": id.to_string()}),
        };
        assert!(matches!(
            store.apply_dry_run(&[op]),
            Err(StoreError::Conflict(_, _))
        ));
    }

    #[test]
    fn dry_run_passes_for_valid_set() {
        let mut store = MemoryStore::new();
        let id = Uuid::new_v4();
        store.seed("Caja", id, json!({"saldo": 100, "currency": "USD"}));
        let op = FieldOp::Set {
            path: FieldPath {
                entity: "Caja".into(),
                id,
                field: "saldo".into(),
            },
            value: json!(150),
        };
        assert!(store.apply_dry_run(&[op]).is_ok());
    }

    #[test]
    fn iter_returns_canonical_order_regardless_of_insertion() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();

        let mut s1 = MemoryStore::new();
        s1.seed("Caja", a, json!({"id": a.to_string(), "x": 1}));
        s1.seed("Movimiento", c, json!({"id": c.to_string(), "y": 3}));
        s1.seed("Caja", b, json!({"id": b.to_string(), "x": 2}));

        let mut s2 = MemoryStore::new();
        s2.seed("Movimiento", c, json!({"id": c.to_string(), "y": 3}));
        s2.seed("Caja", b, json!({"id": b.to_string(), "x": 2}));
        s2.seed("Caja", a, json!({"id": a.to_string(), "x": 1}));

        let r1: Vec<_> = s1.iter().unwrap().collect();
        let r2: Vec<_> = s2.iter().unwrap().collect();
        assert_eq!(r1, r2, "iter order must be insertion-independent");

        // Entities lexicographically sorted (Caja before Movimiento).
        let entities: Vec<&str> = r1.iter().map(|(e, _, _)| e.as_str()).collect();
        assert_eq!(entities, vec!["Caja", "Caja", "Movimiento"]);

        // Within Caja, ids in byte order.
        let caja_ids: Vec<Uuid> = r1
            .iter()
            .filter(|(e, _, _)| e == "Caja")
            .map(|(_, id, _)| *id)
            .collect();
        let mut expected = vec![a, b];
        expected.sort_by(|x, y| x.as_bytes().cmp(y.as_bytes()));
        assert_eq!(caja_ids, expected);
    }

    #[test]
    fn hash_state_is_deterministic_and_independent_of_insertion_order() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();

        let mut s1 = MemoryStore::new();
        s1.seed("Caja", a, json!({"id": a.to_string(), "saldo": 100}));
        s1.seed("Caja", b, json!({"id": b.to_string(), "saldo": 200}));

        let mut s2 = MemoryStore::new();
        s2.seed("Caja", b, json!({"id": b.to_string(), "saldo": 200}));
        s2.seed("Caja", a, json!({"id": a.to_string(), "saldo": 100}));

        assert_eq!(
            s1.hash_state().unwrap(),
            s2.hash_state().unwrap(),
            "equal state must hash identically regardless of how it was built"
        );
    }

    #[test]
    fn hash_state_changes_when_state_changes() {
        let a = Uuid::new_v4();

        let mut s1 = MemoryStore::new();
        s1.seed("Caja", a, json!({"id": a.to_string(), "saldo": 100}));

        let mut s2 = MemoryStore::new();
        s2.seed("Caja", a, json!({"id": a.to_string(), "saldo": 101}));

        assert_ne!(
            s1.hash_state().unwrap(),
            s2.hash_state().unwrap(),
            "off-by-one in a single field must produce a different hash"
        );

        // Adding a record changes the hash too.
        let mut s3 = MemoryStore::new();
        s3.seed("Caja", a, json!({"id": a.to_string(), "saldo": 100}));
        s3.seed("Caja", Uuid::new_v4(), json!({"id": "extra", "saldo": 0}));
        assert_ne!(s1.hash_state().unwrap(), s3.hash_state().unwrap());
    }

    #[test]
    fn last_applied_seq_round_trips_and_resets_on_clear() {
        let mut store = MemoryStore::new();
        assert_eq!(
            store.last_applied_seq().unwrap(),
            None,
            "fresh MemoryStore has no marker"
        );
        store.set_last_applied_seq(5).unwrap();
        assert_eq!(store.last_applied_seq().unwrap(), Some(5));
        store.set_last_applied_seq(12).unwrap();
        assert_eq!(store.last_applied_seq().unwrap(), Some(12));
        store.clear().unwrap();
        assert_eq!(
            store.last_applied_seq().unwrap(),
            None,
            "clear() resets the marker — a cleared store has applied nothing"
        );
    }

    #[test]
    fn integer_and_integer_valued_float_hash_identically() {
        // The cross-backend property: the same numeric value, written
        // by a backend as i64 vs read back as integer-valued f64,
        // must hash the same.
        let int_value = json!({"saldo": 100_i64});
        let float_value = json!({"saldo": 100.0_f64});

        let mut h_int = sha2::Sha256::new();
        super::hash_value(&mut h_int, &int_value);
        let mut h_float = sha2::Sha256::new();
        super::hash_value(&mut h_float, &float_value);
        assert_eq!(
            h_int.finalize(),
            h_float.finalize(),
            "integer-valued numbers must canonicalize regardless of source representation"
        );
    }

    #[test]
    fn fractional_floats_do_not_canonicalize_to_int() {
        // Floats with fractional parts must remain floats — collapsing
        // 100.5 into 100 would hide real differences.
        let int_value = json!({"x": 100_i64});
        let frac_value = json!({"x": 100.5_f64});

        let mut h_int = sha2::Sha256::new();
        super::hash_value(&mut h_int, &int_value);
        let mut h_frac = sha2::Sha256::new();
        super::hash_value(&mut h_frac, &frac_value);
        assert_ne!(
            h_int.finalize(),
            h_frac.finalize(),
            "100 and 100.5 must hash differently"
        );
    }

    #[test]
    fn same_object_with_different_insertion_order_hashes_same() {
        // serde_json::Map is BTreeMap by default but we sort defensively
        // in case `preserve_order` is enabled by some transitive dep.
        let mut m1 = serde_json::Map::new();
        m1.insert("a".into(), json!(1));
        m1.insert("b".into(), json!(2));
        m1.insert("c".into(), json!(3));
        let mut m2 = serde_json::Map::new();
        m2.insert("c".into(), json!(3));
        m2.insert("a".into(), json!(1));
        m2.insert("b".into(), json!(2));

        let mut h1 = sha2::Sha256::new();
        super::hash_value(&mut h1, &Value::Object(m1));
        let mut h2 = sha2::Sha256::new();
        super::hash_value(&mut h2, &Value::Object(m2));
        assert_eq!(h1.finalize(), h2.finalize());
    }

    #[test]
    fn type_tagged_framing_distinguishes_string_from_number() {
        // The string "42" must not collide with the number 42.
        let str_v = json!("42");
        let num_v = json!(42);
        let mut h_str = sha2::Sha256::new();
        super::hash_value(&mut h_str, &str_v);
        let mut h_num = sha2::Sha256::new();
        super::hash_value(&mut h_num, &num_v);
        assert_ne!(h_str.finalize(), h_num.finalize());

        // Bool true must not collide with the number 1.
        let bool_v = json!(true);
        let one_v = json!(1);
        let mut h_bool = sha2::Sha256::new();
        super::hash_value(&mut h_bool, &bool_v);
        let mut h_one = sha2::Sha256::new();
        super::hash_value(&mut h_one, &one_v);
        assert_ne!(h_bool.finalize(), h_one.finalize());
    }

    #[test]
    fn empty_store_has_a_well_defined_hash() {
        let s1 = MemoryStore::new();
        let s2 = MemoryStore::new();
        assert_eq!(s1.hash_state().unwrap(), s2.hash_state().unwrap());
        // The empty hash is the SHA-256 of an empty input — fix the
        // expected bytes so an accidental framing change in `hash_state`
        // can't silently sail through.
        let expected = hex_decode(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        );
        assert_eq!(s1.hash_state().unwrap().to_vec(), expected);
    }

    fn hex_decode(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }
}
