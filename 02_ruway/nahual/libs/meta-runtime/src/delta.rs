//! Cálculo del delta entre el record actual y la propuesta del form.
//!
//! Sirve a un runtime de edición para emitir SOLO los Set/Clear que
//! cambian algo: log + apply minimales, no-op edits = 0 entries.

use serde_json::Value;

/// Calcula el delta entre el record actual y los valores propuestos
/// del form. Devuelve un Map con sólo los campos cuyo valor difiere.
///
/// Comparación: igualdad estructural sobre `serde_json::Value`. Un
/// `current=Value::Null` (record no encontrado) hace que todos los
/// campos del `proposed` sean considerados nuevos. Un campo del
/// proposed que coincide con el del current se omite. Campos que
/// están en current pero NO en proposed se preservan tal cual (el
/// edit no los toca; ver [`compute_clear_fields`] para borrar
/// explícito desde un input vacío).
pub fn compute_field_delta(
    current: &Value,
    proposed: &serde_json::Map<String, Value>,
) -> serde_json::Map<String, Value> {
    proposed
        .iter()
        .filter(|(field, value)| current.get(field.as_str()) != Some(*value))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Decide cuáles fields del `to_clear` candidate list ameritan
/// realmente un `FieldOp::Clear`: sólo los que existen en el current
/// con un valor non-null. Para fields ausentes o ya null, Clear es
/// no-op semántico (el post-state es el mismo) y dropearlos
/// preserva la propiedad "1 op = 1 cambio efectivo" del log.
///
/// Preserva el orden del input para que el log entry sea estable.
pub fn compute_clear_fields(current: &Value, to_clear: &[String]) -> Vec<String> {
    to_clear
        .iter()
        .filter(|f| match current.get(f.as_str()) {
            None | Some(Value::Null) => false,
            Some(_) => true,
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(items: &[(&str, Value)]) -> serde_json::Map<String, Value> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn delta_empty_when_all_fields_match() {
        let current = json!({"name": "Acme", "saldo": 100_i64, "currency": "USD"});
        let proposed = map(&[
            ("name", json!("Acme")),
            ("saldo", json!(100_i64)),
            ("currency", json!("USD")),
        ]);
        assert!(compute_field_delta(&current, &proposed).is_empty());
    }

    #[test]
    fn delta_includes_only_changed_field() {
        let current = json!({"name": "Acme", "saldo": 100_i64});
        let proposed = map(&[("name", json!("Acme")), ("saldo", json!(200_i64))]);
        let d = compute_field_delta(&current, &proposed);
        assert_eq!(d.len(), 1);
        assert_eq!(d.get("saldo"), Some(&json!(200_i64)));
    }

    #[test]
    fn delta_treats_missing_record_as_all_new() {
        let current = Value::Null;
        let proposed = map(&[("name", json!("Acme")), ("saldo", json!(0_i64))]);
        assert_eq!(compute_field_delta(&current, &proposed).len(), 2);
    }

    #[test]
    fn delta_distinguishes_int_from_string_repr() {
        let current = json!({"qty": 100_i64});
        let proposed = map(&[("qty", json!(100_i64))]);
        assert!(compute_field_delta(&current, &proposed).is_empty());

        let current_str = json!({"qty": "100"});
        let proposed_int = map(&[("qty", json!(100_i64))]);
        assert_eq!(compute_field_delta(&current_str, &proposed_int).len(), 1);
    }

    #[test]
    fn delta_skips_fields_absent_from_proposed() {
        let current = json!({"name": "Acme", "saldo": 100_i64, "extra": "x"});
        let proposed = map(&[("name", json!("Acme")), ("saldo", json!(150_i64))]);
        let d = compute_field_delta(&current, &proposed);
        assert_eq!(d.len(), 1);
        assert!(!d.contains_key("extra"));
    }

    #[test]
    fn clear_fields_skips_absent_and_null() {
        let current = json!({"name": "Acme", "notes": "lorem", "tag": null});
        let to_clear = vec![
            "name".into(),
            "notes".into(),
            "tag".into(),
            "missing".into(),
        ];
        assert_eq!(
            compute_clear_fields(&current, &to_clear),
            vec!["name".to_string(), "notes".to_string()]
        );
    }

    #[test]
    fn clear_fields_preserves_input_order() {
        let current = json!({"a": 1, "b": 2, "c": 3});
        let to_clear = vec!["c".into(), "a".into(), "b".into()];
        assert_eq!(
            compute_clear_fields(&current, &to_clear),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn clear_fields_empty_when_current_is_null() {
        let current = Value::Null;
        let to_clear = vec!["name".into()];
        assert!(compute_clear_fields(&current, &to_clear).is_empty());
    }
}
