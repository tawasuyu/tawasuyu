//! Utilidades de testing para code que consume [`MetaBackend`].
//!
//! Provee [`MockBackend`]: implementación in-memory minimalista
//! del trait, sin acoplamiento a stores reales (event log,
//! SurrealDB, etc.). Útil para:
//!
//! - Tests del widget [`nahual_widget_meta_form::MetaApp`] que
//!   necesitan un backend funcional sin levantar nakui-core.
//! - Tests de cualquier consumer que tome `B: MetaBackend` y quiera
//!   asserts sobre lecturas/escrituras sin tocar disco.
//! - Fixtures pre-pobladas para demos/screenshots/CI.
//!
//! Está bajo `pub mod testing` (no `#[cfg(test)]`) deliberadamente
//! para que crates downstream puedan importarlo en sus dev/integ
//! tests. No tiene overhead en producción si no se usa.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;
use uuid::Uuid;

use crate::backend::{MetaBackend, WriteOutcome};

/// Backend in-memory para tests. Implementa el contrato completo
/// del [`MetaBackend`] con semantica simple:
///
/// - `seed`: genera Uuid v4, inserta record. `changed = 1`.
/// - `update`: aplica `set` (overrides) y `clear` (key removal).
///   Si ambos vacíos → `changed = 0`. Falla si record no existe.
/// - `delete`: remueve record. Falla si no existe.
/// - `morphism`: por default rebota con error
///   `"MockBackend no soporta morphism '<name>'"`. Si querés
///   simular morphisms, registrá callbacks via
///   [`MockBackend::with_morphism`].
/// - `list_records`: orden lexicográfico por id (estable).
/// - Sin `post_status`: el mock no tiene tick/compact.
///
/// Métodos de inspección públicos ([`total_records`],
/// [`records_for`], etc.) facilitan asserts en tests sin necesidad
/// de re-leer el state via las APIs del trait.
pub struct MockBackend {
    records: HashMap<(String, Uuid), Value>,
    morphisms: HashMap<String, MorphismHandler>,
}

type MorphismHandler =
    Box<dyn Fn(&BTreeMap<String, Uuid>, &Value) -> Result<usize, String> + Send + Sync>;

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockBackend {
    /// Backend vacío.
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
            morphisms: HashMap::new(),
        }
    }

    /// Pre-popula el backend con records `(entity, uuid, data)`.
    /// Útil para fixtures: asserts sobre lecturas sin tener que
    /// armar seeds via `seed()`.
    pub fn with_records<I>(records: I) -> Self
    where
        I: IntoIterator<Item = (String, Uuid, Value)>,
    {
        let mut b = Self::new();
        for (entity, id, data) in records {
            b.records.insert((entity, id), data);
        }
        b
    }

    /// Registra un handler para un morphism de nombre `name`.
    /// El handler recibe inputs + params y devuelve `changed` o
    /// `Err` para simular fallo del morphism. Sobrescribe cualquier
    /// handler previo del mismo nombre.
    pub fn with_morphism<F>(mut self, name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(&BTreeMap<String, Uuid>, &Value) -> Result<usize, String> + Send + Sync + 'static,
    {
        self.morphisms.insert(name.into(), Box::new(handler));
        self
    }

    /// Cantidad total de records en el backend (todas las entities).
    pub fn total_records(&self) -> usize {
        self.records.len()
    }

    /// Records de una entity como `Vec<(Uuid, &Value)>` sin clones
    /// (más liviano que `list_records` cuando el caller sólo quiere
    /// inspeccionar).
    pub fn records_for<'a>(&'a self, entity: &str) -> Vec<(Uuid, &'a Value)> {
        self.records
            .iter()
            .filter(|((e, _), _)| e == entity)
            .map(|((_, id), v)| (*id, v))
            .collect()
    }
}

impl MetaBackend for MockBackend {
    fn list_records(&self, entity: &str) -> Vec<(Uuid, Value)> {
        let mut out: Vec<(Uuid, Value)> = self
            .records
            .iter()
            .filter(|((e, _), _)| e == entity)
            .map(|((_, id), v)| (*id, v.clone()))
            .collect();
        out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        out
    }

    fn load_record(&self, entity: &str, id: Uuid) -> Option<Value> {
        self.records.get(&(entity.to_string(), id)).cloned()
    }

    fn seed(
        &mut self,
        entity: &str,
        data: serde_json::Map<String, Value>,
    ) -> Result<WriteOutcome, String> {
        let id = Uuid::new_v4();
        self.records
            .insert((entity.to_string(), id), Value::Object(data));
        Ok(WriteOutcome {
            id: Some(id),
            changed: 1,
            post_status: None,
        })
    }

    fn update(
        &mut self,
        entity: &str,
        id: Uuid,
        set: serde_json::Map<String, Value>,
        clear: Vec<String>,
    ) -> Result<WriteOutcome, String> {
        if set.is_empty() && clear.is_empty() {
            return Ok(WriteOutcome::no_change(id));
        }
        let rec = self
            .records
            .get_mut(&(entity.to_string(), id))
            .ok_or_else(|| format!("not found: {entity}/{id}"))?;
        let map = rec
            .as_object_mut()
            .ok_or_else(|| format!("not an object: {entity}/{id}"))?;
        let changed = set.len() + clear.len();
        for (k, v) in set {
            map.insert(k, v);
        }
        for k in clear {
            map.remove(&k);
        }
        Ok(WriteOutcome {
            id: Some(id),
            changed,
            post_status: None,
        })
    }

    fn delete(&mut self, entity: &str, id: Uuid) -> Result<WriteOutcome, String> {
        self.records
            .remove(&(entity.to_string(), id))
            .ok_or_else(|| format!("not found: {entity}/{id}"))?;
        Ok(WriteOutcome {
            id: Some(id),
            changed: 1,
            post_status: None,
        })
    }

    fn morphism(
        &mut self,
        _module_id: &str,
        name: &str,
        inputs: BTreeMap<String, Uuid>,
        params: Value,
    ) -> Result<WriteOutcome, String> {
        match self.morphisms.get(name) {
            Some(handler) => {
                let changed = handler(&inputs, &params)?;
                Ok(WriteOutcome {
                    id: None,
                    changed,
                    post_status: None,
                })
            }
            None => Err(format!("MockBackend no soporta morphism '{name}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map_of(items: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn with_records_populates_state() {
        let id = Uuid::new_v4();
        let b = MockBackend::with_records([("Customer".into(), id, json!({"name": "Acme"}))]);
        assert_eq!(b.total_records(), 1);
        assert_eq!(b.load_record("Customer", id), Some(json!({"name": "Acme"})));
    }

    #[test]
    fn seed_then_load_round_trip_via_trait() {
        let mut b = MockBackend::new();
        let out = b.seed("X", map_of(&[("k", json!(1))])).unwrap();
        let id = out.id.unwrap();
        assert_eq!(out.changed, 1);
        assert_eq!(b.load_record("X", id), Some(json!({"k": 1})));
    }

    #[test]
    fn update_no_op_returns_no_change() {
        let id = Uuid::new_v4();
        let mut b = MockBackend::with_records([("X".into(), id, json!({"k": 1}))]);
        let out = b.update("X", id, serde_json::Map::new(), vec![]).unwrap();
        assert_eq!(out, WriteOutcome::no_change(id));
    }

    #[test]
    fn update_set_and_clear_aplica_ambos() {
        let id = Uuid::new_v4();
        let mut b = MockBackend::with_records([("X".into(), id, json!({"a": 1, "b": 2}))]);
        let out = b
            .update("X", id, map_of(&[("a", json!(10))]), vec!["b".into()])
            .unwrap();
        assert_eq!(out.changed, 2);
        let rec = b.load_record("X", id).unwrap();
        assert_eq!(rec.get("a"), Some(&json!(10)));
        assert!(rec.get("b").is_none());
    }

    #[test]
    fn delete_then_load_returns_none() {
        let id = Uuid::new_v4();
        let mut b = MockBackend::with_records([("X".into(), id, json!({"k": 1}))]);
        b.delete("X", id).unwrap();
        assert!(b.load_record("X", id).is_none());
    }

    #[test]
    fn morphism_without_handler_errors_clearly() {
        let mut b = MockBackend::new();
        let err = b
            .morphism("mod", "foo", BTreeMap::new(), json!({}))
            .unwrap_err();
        assert!(err.contains("foo"));
    }

    #[test]
    fn with_morphism_lets_caller_simulate_logic() {
        let mut b = MockBackend::new().with_morphism("double_qty", |inputs, params| {
            assert!(inputs.is_empty());
            let qty = params.get("qty").and_then(|v| v.as_i64()).unwrap_or(0);
            if qty <= 0 {
                return Err("qty must be positive".into());
            }
            Ok(qty as usize)
        });
        let out = b
            .morphism("mod", "double_qty", BTreeMap::new(), json!({"qty": 7}))
            .unwrap();
        assert_eq!(out.changed, 7);
        assert!(out.id.is_none(), "morphism no devuelve id por convención");

        let err = b
            .morphism("mod", "double_qty", BTreeMap::new(), json!({"qty": 0}))
            .unwrap_err();
        assert!(err.contains("positive"));
    }

    #[test]
    fn list_records_orders_lexicographically() {
        let id_a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id_b = Uuid::parse_str("ffffffff-0000-0000-0000-000000000000").unwrap();
        let b = MockBackend::with_records([
            ("X".into(), id_b, json!({"n": 2})),
            ("X".into(), id_a, json!({"n": 1})),
        ]);
        let rows = b.list_records("X");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, id_a, "menor uuid primero (orden lex)");
    }

    #[test]
    fn records_for_returns_borrowed_view() {
        let id = Uuid::new_v4();
        let b = MockBackend::with_records([("X".into(), id, json!({"k": 1}))]);
        let view = b.records_for("X");
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].0, id);
        assert_eq!(view[0].1.get("k"), Some(&json!(1)));
    }
}
