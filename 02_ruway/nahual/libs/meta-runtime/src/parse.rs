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
        // Un Array no es un valor escalar: se parsea con `parse_array_value`
        // (necesita `item_fields`). Llegar acá es un misuse del caller.
        FieldKind::Array => {
            Err("un campo array se parsea con parse_array_value, no como escalar".into())
        }
    }
}

/// Parsea el texto multilínea de un campo [`FieldKind::Array`] a un
/// `Value::Array` de objetos. Una fila por línea no vacía; las columnas
/// se separan por `delimiter` y se mapean POSICIONALMENTE a `item_fields`.
///
/// Una columna `AutoId` NO consume celda: se le pone un UUID v4 por fila
/// (para los ids de idempotencia de cada record que cree el morfismo). El
/// resto de columnas se parsean con [`parse_field_value`] según su kind.
/// Una celda vacía en columna requerida rebota; en opcional → `Null`.
pub fn parse_array_value(
    raw: &str,
    item_fields: &[FieldSpec],
    delimiter: &str,
) -> Result<Value, String> {
    let col_label = |f: &FieldSpec| {
        if f.label.is_empty() {
            f.name.clone()
        } else {
            f.label.clone()
        }
    };
    let mut rows: Vec<Value> = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split(delimiter).map(str::trim).collect();
        let mut obj = serde_json::Map::new();
        let mut cell_idx = 0usize;
        for f in item_fields {
            let value = if f.kind == FieldKind::AutoId {
                json!(Uuid::new_v4().to_string())
            } else {
                let cell = cells.get(cell_idx).copied().unwrap_or("");
                cell_idx += 1;
                if cell.is_empty() {
                    if f.required {
                        return Err(format!(
                            "fila {}: columna '{}' es obligatoria",
                            i + 1,
                            col_label(f)
                        ));
                    }
                    Value::Null
                } else {
                    parse_field_value(f.kind, cell)
                        .map_err(|e| format!("fila {}, columna '{}': {e}", i + 1, col_label(f)))?
                }
            };
            obj.insert(f.name.clone(), value);
        }
        rows.push(Value::Object(obj));
    }
    Ok(Value::Array(rows))
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
    // Un Array se resuelve con su parser dedicado (necesita item_fields).
    if s.kind == FieldKind::Array {
        let delim = s.delimiter.as_deref().unwrap_or("|");
        return parse_array_value(raw, &s.item_fields, delim)
            .map_err(|e| format!("param '{label}': {e}"));
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
            section: None,
            item_fields: Vec::new(),
            delimiter: None,
        }
    }

    #[test]
    fn parse_array_maps_columns_and_autogenerates_ids() {
        // item_fields: id (AutoId, no consume celda) + concepto (Text) +
        // cantidad (Number) + precio (Number).
        let cols = vec![
            spec("id", FieldKind::AutoId, false),
            spec("concepto", FieldKind::Text, true),
            spec("cantidad", FieldKind::Number, true),
            spec("precio", FieldKind::Number, true),
        ];
        let raw = "Servicio de diseño | 2 | 500\nHosting anual | 1 | 300\n";
        let arr = parse_array_value(raw, &cols, "|").unwrap();
        let rows = arr.as_array().unwrap();
        assert_eq!(rows.len(), 2, "dos filas no vacías");

        let r0 = &rows[0];
        assert_eq!(r0.get("concepto").and_then(Value::as_str), Some("Servicio de diseño"));
        assert_eq!(r0.get("cantidad").and_then(Value::as_i64), Some(2));
        assert_eq!(r0.get("precio").and_then(Value::as_i64), Some(500));
        // El id se autogeneró y es un UUID válido (no vino del texto).
        let id = r0.get("id").and_then(Value::as_str).unwrap();
        assert!(Uuid::parse_str(id).is_ok());
        // Cada fila trae un id distinto.
        let id1 = rows[1].get("id").and_then(Value::as_str).unwrap();
        assert_ne!(id, id1);
    }

    #[test]
    fn parse_array_skips_blank_lines_and_rejects_missing_required() {
        let cols = vec![
            spec("concepto", FieldKind::Text, true),
            spec("monto", FieldKind::Number, true),
        ];
        // Línea en blanco en el medio se ignora.
        let ok = parse_array_value("a | 10\n\n  \nb | 20", &cols, "|").unwrap();
        assert_eq!(ok.as_array().unwrap().len(), 2);

        // Falta la columna requerida `monto` → error con número de fila.
        let err = parse_array_value("solo concepto", &cols, "|").unwrap_err();
        assert!(err.contains("fila 1"), "err: {err}");
        assert!(err.contains("monto") || err.contains("obligatoria"), "err: {err}");
    }

    #[test]
    fn resolve_param_array_dispatches_to_array_parser() {
        let mut s = spec("lineas", FieldKind::Array, true);
        s.item_fields = vec![
            spec("concepto", FieldKind::Text, true),
            spec("monto", FieldKind::Number, true),
        ];
        let v = resolve_param_value("lineas", "café | 5\nté | 3", Some(&s)).unwrap();
        let rows = v.as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("monto").and_then(Value::as_i64), Some(5));
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
