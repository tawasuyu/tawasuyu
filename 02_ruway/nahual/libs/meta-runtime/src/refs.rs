//! Validación cross-field de EntityRefs contra el store actual.
//!
//! Decoupling: en vez de un `trait Store` que ate este crate a un
//! backend específico, tomamos un cierre `load: Fn(&str, Uuid) ->
//! Option<Value>`. El caller (nakui-ui o cualquier otro runtime)
//! puede pasarlo trivialmente sobre cualquier store (MemoryStore,
//! SurrealStore, mock, ...).

use serde_json::Value;
use uuid::Uuid;

use crate::format::short_uuid;

/// Valida que cada UUID en `refs` apunte a un record que realmente
/// existe en el store bajo la entity esperada. Devuelve el primer
/// error encontrado (fail-fast).
///
/// `refs` es una lista de `(label, target_entity, uuid)`. El label
/// va al error message, así que conviene que sea legible (ej:
/// `FieldSpec.label` en lugar de `FieldSpec.name`).
///
/// `load` es el cierre que el caller usa para mirar el store —
/// típicamente `|e, id| store.load(e, id)`.
pub fn validate_entity_refs<F>(load: F, refs: &[(String, String, Uuid)]) -> Result<(), String>
where
    F: Fn(&str, Uuid) -> Option<Value>,
{
    for (label, target, id) in refs {
        if load(target, *id).is_none() {
            return Err(format!(
                "campo '{label}': record {} de '{target}' no existe en el store",
                short_uuid(id)
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    /// "Mock store" minimalista para tests: HashMap por (entity, uuid).
    fn mk_load(records: HashMap<(String, Uuid), Value>) -> impl Fn(&str, Uuid) -> Option<Value> {
        move |e, id| records.get(&(e.to_string(), id)).cloned()
    }

    #[test]
    fn passes_when_all_records_exist() {
        let stock = Uuid::new_v4();
        let caja = Uuid::new_v4();
        let mut records = HashMap::new();
        records.insert(("Stock".into(), stock), json!({"sku_id": "abc"}));
        records.insert(("Caja".into(), caja), json!({"name": "Principal"}));
        let load = mk_load(records);

        let refs = vec![
            ("Stock".into(), "Stock".into(), stock),
            ("Caja".into(), "Caja".into(), caja),
        ];
        assert!(validate_entity_refs(load, &refs).is_ok());
    }

    #[test]
    fn fails_on_first_missing() {
        let stock = Uuid::new_v4();
        let mut records = HashMap::new();
        records.insert(("Stock".into(), stock), json!({"sku_id": "abc"}));
        let load = mk_load(records);

        let missing_caja = Uuid::new_v4();
        let refs = vec![
            ("Stock".into(), "Stock".into(), stock),
            ("Caja".into(), "Caja".into(), missing_caja),
        ];
        let err = validate_entity_refs(load, &refs).unwrap_err();
        assert!(err.contains("Caja"));
        assert!(err.contains(&short_uuid(&missing_caja)));
    }

    #[test]
    fn uses_label_not_entity_in_msg() {
        let load = |_: &str, _: Uuid| -> Option<Value> { None };
        let id = Uuid::new_v4();
        let refs = vec![("Stock origen".into(), "Stock".into(), id)];
        let err = validate_entity_refs(load, &refs).unwrap_err();
        assert!(err.contains("Stock origen"));
    }

    #[test]
    fn empty_list_is_ok() {
        let load = |_: &str, _: Uuid| -> Option<Value> { None };
        assert!(validate_entity_refs(load, &[]).is_ok());
    }

    #[test]
    fn distinguishes_target_from_other_entities() {
        let id = Uuid::new_v4();
        let mut records = HashMap::new();
        // Mismo UUID bajo Customer pero NO bajo Stock.
        records.insert(("Customer".into(), id), json!({"name": "Acme"}));
        let load = mk_load(records);
        let refs = vec![("Stock".into(), "Stock".into(), id)];
        assert!(validate_entity_refs(load, &refs).is_err());
    }
}
