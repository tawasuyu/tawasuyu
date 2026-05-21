//! Parseo de inputs del form a `serde_json::Value` tipado.

use serde_json::{json, Value};
use uuid::Uuid;

use nahual_meta_schema::{FieldKind, FieldSpec};

/// Convierte el texto raw de un input al `Value` tipado según el
/// `kind` del spec.
///
/// - `Text` / `Multiline` / `Date` → string passthrough.
/// - `EntityRef` → string del UUID **trimmed**, validado como UUID
///   parseable. Falla con mensaje claro si no parsea.
/// - `Boolean` → variantes comunes (`true/yes/1/on/y` y `false/no/0/off/n`).
/// - `Number` → i64 si parsea, sino f64.
pub fn parse_field_value(kind: FieldKind, raw: &str) -> Result<Value, String> {
    match kind {
        // Select y AutoId guardan un string: el valor de la opción
        // elegida y el UUID autogenerado, respectivamente.
        FieldKind::Text
        | FieldKind::Multiline
        | FieldKind::Date
        | FieldKind::Select
        | FieldKind::AutoId => Ok(json!(raw)),
        // EntityRef se almacena como string del UUID seleccionado.
        // El selector clickable garantiza UUIDs válidos en happy
        // path; este check protege paste manual o garbage tipeado.
        FieldKind::EntityRef => {
            let trimmed = raw.trim();
            Uuid::parse_str(trimmed)
                .map_err(|_| format!("'{raw}' no es UUID válido (usá el selector de records)"))?;
            Ok(json!(trimmed))
        }
        FieldKind::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" | "y" => Ok(json!(true)),
            "" | "false" | "no" | "0" | "off" | "n" => Ok(json!(false)),
            other => Err(format!("'{other}' no es booleano")),
        },
        FieldKind::Number => {
            if let Ok(i) = raw.parse::<i64>() {
                Ok(json!(i))
            } else if let Ok(f) = raw.parse::<f64>() {
                Ok(json!(f))
            } else {
                Err(format!("'{raw}' no es número"))
            }
        }
    }
}

/// Resuelve un param de morphism a su `Value` según el `FieldSpec`
/// del form. **Strict path**: si hay spec, valida `required` y parsea
/// con el `kind` declarado (ej. Boolean rebota con "abc" antes de
/// llegar al morphism). **Fallback path**: si no hay spec (param
/// declarado en `Action::Morphism.params` que no aparece en
/// `form.fields`), usa la heurística [`infer_param_value`] para no
/// quedar atado a un schema mal-formado.
///
/// Errores tienen el label legible del spec, así el toast de la UI
/// es interpretable.
pub fn resolve_param_value(
    field_name: &str,
    raw: &str,
    spec: Option<&FieldSpec>,
) -> Result<Value, String> {
    let Some(s) = spec else {
        return Ok(infer_param_value(raw));
    };

    let label = if s.label.is_empty() {
        field_name
    } else {
        &s.label
    };

    if s.required && raw.trim().is_empty() {
        return Err(format!("param '{label}' es obligatorio y está vacío"));
    }
    if raw.is_empty() && !s.required {
        return Ok(Value::Null);
    }
    parse_field_value(s.kind, raw).map_err(|e| format!("param '{label}': {e}"))
}

/// Inferencia de tipo para values pasados como `params` a un
/// morphism. Usada como fallback en [`resolve_param_value`] cuando el
/// param declarado en `Action::Morphism.params` no aparece en los
/// `form.fields` (módulo mal-formado).
///
/// Heurística simple: int → i64, float → f64, "true"/"false" → bool,
/// resto → string.
pub fn infer_param_value(raw: &str) -> Value {
    if raw.is_empty() {
        return Value::Null;
    }
    if let Ok(i) = raw.parse::<i64>() {
        return json!(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return json!(f);
    }
    match raw {
        "true" => return json!(true),
        "false" => return json!(false),
        _ => {}
    }
    json!(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nahual_meta_schema::FieldSpec;

    fn spec(name: &str, kind: FieldKind, required: bool) -> FieldSpec {
        FieldSpec {
            name: name.into(),
            label: name.into(),
            kind,
            default: None,
            required,
            help: None,
            ref_entity: None,
            options: Vec::new(),
        }
    }

    #[test]
    fn infer_handles_basic_types() {
        assert_eq!(infer_param_value(""), Value::Null);
        assert_eq!(infer_param_value("42"), json!(42));
        assert_eq!(infer_param_value("2.5"), json!(2.5));
        assert_eq!(infer_param_value("true"), json!(true));
        assert_eq!(infer_param_value("false"), json!(false));
        assert_eq!(infer_param_value("hola"), json!("hola"));
    }

    #[test]
    fn parse_text_passthrough() {
        let v = parse_field_value(FieldKind::Text, "hola").unwrap();
        assert_eq!(v, json!("hola"));
    }

    #[test]
    fn parse_select_and_auto_id_passthrough() {
        // Select guarda el valor de la opción elegida.
        assert_eq!(
            parse_field_value(FieldKind::Select, "ganada").unwrap(),
            json!("ganada")
        );
        // AutoId guarda el UUID autogenerado tal cual.
        let id = Uuid::new_v4().to_string();
        assert_eq!(
            parse_field_value(FieldKind::AutoId, &id).unwrap(),
            json!(id)
        );
    }

    #[test]
    fn parse_number_i64_or_f64() {
        assert_eq!(
            parse_field_value(FieldKind::Number, "42").unwrap(),
            json!(42)
        );
        assert_eq!(
            parse_field_value(FieldKind::Number, "2.5").unwrap(),
            json!(2.5)
        );
        assert!(parse_field_value(FieldKind::Number, "abc").is_err());
    }

    #[test]
    fn parse_boolean_recognizes_variants() {
        for s in ["true", "yes", "1", "on", "y"] {
            assert_eq!(
                parse_field_value(FieldKind::Boolean, s).unwrap(),
                json!(true)
            );
        }
        for s in ["false", "no", "0", "off", "n", ""] {
            assert_eq!(
                parse_field_value(FieldKind::Boolean, s).unwrap(),
                json!(false)
            );
        }
        assert!(parse_field_value(FieldKind::Boolean, "abc").is_err());
    }

    #[test]
    fn parse_entity_ref_accepts_valid_uuid() {
        let id = Uuid::new_v4();
        let v = parse_field_value(FieldKind::EntityRef, &id.to_string()).unwrap();
        assert_eq!(v, json!(id.to_string()));
    }

    #[test]
    fn parse_entity_ref_trims_whitespace() {
        let id = Uuid::new_v4();
        let padded = format!("  {id}\n");
        let v = parse_field_value(FieldKind::EntityRef, &padded).unwrap();
        assert_eq!(v, json!(id.to_string()));
    }

    #[test]
    fn parse_entity_ref_rejects_non_uuid() {
        let err = parse_field_value(FieldKind::EntityRef, "abc-123").unwrap_err();
        assert!(err.contains("'abc-123'"));
        assert!(err.contains("UUID") || err.contains("uuid"));
    }

    #[test]
    fn parse_entity_ref_rejects_empty_string() {
        let err = parse_field_value(FieldKind::EntityRef, "").unwrap_err();
        assert!(err.contains("UUID"));
    }

    #[test]
    fn resolve_param_strict_number_parses_i64() {
        let s = spec("qty", FieldKind::Number, true);
        let v = resolve_param_value("qty", "42", Some(&s)).unwrap();
        assert_eq!(v, json!(42));
    }

    #[test]
    fn resolve_param_strict_boolean_rejects_non_boolean() {
        let s = spec("active", FieldKind::Boolean, true);
        let err = resolve_param_value("active", "abc", Some(&s)).unwrap_err();
        assert!(err.contains("active"));
    }

    #[test]
    fn resolve_param_required_empty_rejected() {
        let s = spec("name", FieldKind::Text, true);
        let err = resolve_param_value("name", "   ", Some(&s)).unwrap_err();
        assert!(err.contains("obligatorio"));
    }

    #[test]
    fn resolve_param_optional_empty_returns_null() {
        let s = spec("notes", FieldKind::Text, false);
        let v = resolve_param_value("notes", "", Some(&s)).unwrap();
        assert_eq!(v, Value::Null);
    }

    #[test]
    fn resolve_param_no_spec_falls_back_to_infer() {
        let v = resolve_param_value("foo", "42", None).unwrap();
        assert_eq!(v, json!(42));
        let v = resolve_param_value("foo", "true", None).unwrap();
        assert_eq!(v, json!(true));
        let v = resolve_param_value("foo", "x", None).unwrap();
        assert_eq!(v, json!("x"));
    }

    #[test]
    fn resolve_param_strict_entity_ref_propagates_error() {
        let s = spec("stock_ref", FieldKind::EntityRef, true);
        let err = resolve_param_value("stock_ref", "not-a-uuid", Some(&s)).unwrap_err();
        assert!(err.contains("stock_ref"));
        assert!(err.contains("UUID"));
    }
}
