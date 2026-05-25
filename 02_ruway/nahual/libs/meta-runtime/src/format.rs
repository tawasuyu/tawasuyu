//! Helpers de presentación humana para records y values.
//!
//! Sin GPUI: devuelven `String`s. El widget renderer los wrap-ea
//! en `div().child(...)` o equivalente.

use std::cmp::Ordering;

use serde_json::Value;
use uuid::Uuid;

use nahual_meta_schema::ValueFormat;

/// Compara dos valores de celda para ordenar una lista. `None`/`null`
/// ordenan antes que cualquier valor. Números por valor numérico,
/// strings case-insensitive, bools `false < true`; tipos mixtos por su
/// forma string (orden estable, no semántico).
pub fn cmp_values(a: Option<&Value>, b: Option<&Value>) -> Ordering {
    let nullish = |v: Option<&Value>| matches!(v, None | Some(Value::Null));
    match (nullish(a), nullish(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {}
    }
    match (a, b) {
        (Some(Value::Number(x)), Some(Value::Number(y))) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(Ordering::Equal),
        (Some(Value::String(x)), Some(Value::String(y))) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        (Some(x), Some(y)) => x.to_string().cmp(&y.to_string()),
        // Inalcanzable: el chequeo nullish de arriba cubre los None.
        _ => Ordering::Equal,
    }
}

/// Etiqueta humana para representar un record en el selector de
/// EntityRef y en columnas de referencia. Heurística: prefiere campos
/// de nombre comunes (ES + EN); fallback al UUID corto.
pub fn human_label_for_record(value: &Value, id: &Uuid) -> String {
    for key in [
        "name", "nombre", "label", "title", "titulo", "sku", "sku_id",
    ] {
        if let Some(v) = value.get(key).and_then(Value::as_str) {
            if !v.is_empty() {
                return format!("{} ({})", v, short_uuid(id));
            }
        }
    }
    short_uuid(id)
}

/// Render legible de un `Value` arbitrario para mostrar en una celda
/// de lista. Strings van pelados; bools como ✓/✗; el resto via
/// `Display`.
pub fn render_value(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "✓" } else { "✗" }.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Render de un valor de celda según un [`ValueFormat`]. `Plain`
/// delega en [`render_value`]; `Number`/`Currency` agrupan miles. Un
/// valor no numérico bajo `Number`/`Currency` cae a `render_value`.
pub fn format_value(v: Option<&Value>, fmt: &ValueFormat) -> String {
    match fmt {
        ValueFormat::Plain => render_value(v),
        ValueFormat::Number => match v {
            Some(Value::Number(n)) => group_thousands(n),
            _ => render_value(v),
        },
        ValueFormat::Currency { symbol } => match v {
            Some(Value::Number(n)) => format!("{symbol}{}", group_thousands(n)),
            _ => render_value(v),
        },
    }
}

/// Formatea un `Number` con separador de miles. Enteros sin decimales;
/// flotantes con dos.
fn group_thousands(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        group_int(i)
    } else if let Some(f) = n.as_f64() {
        let neg = f.is_sign_negative();
        let cents = (f.abs() * 100.0).round() as i64;
        format!(
            "{}{}.{:02}",
            if neg { "-" } else { "" },
            group_int(cents / 100),
            cents % 100,
        )
    } else {
        n.to_string()
    }
}

/// Inserta comas cada tres dígitos en un entero con signo.
fn group_int(i: i64) -> String {
    let digits = i.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::new();
    for (idx, &b) in bytes.iter().enumerate() {
        if idx > 0 && (bytes.len() - idx).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    if i < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Conversión inversa a `parse_field_value`: del JSON al texto raw
/// que un input puede tomar y volver a parsearse igual al submit.
/// Usado para pre-llenar inputs en modo edit.
pub fn value_to_input_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Primeros 8 chars del UUID en forma canónica. Útil para logs y UI
/// donde el UUID full es ruido visual.
pub fn short_uuid(id: &Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

/// Hex string de los primeros 4 bytes de un hash SHA-256 (8
/// caracteres). Útil para mostrar bundle/schema hashes en UI sin
/// quemar pantalla con los 64 chars completos.
pub fn short_hash(h: &[u8; 32]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(8);
    for b in h.iter().take(4) {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Renderea un `serde_json::Value` en una sola línea, truncado a
/// `max` caracteres con `...` al final si excede. Para preview en
/// timelines/cards/listas — NO para edición.
///
/// `max` es un upper-bound aproximado: el resultado nunca excede
/// `max` chars, pero puede ser más corto si el value es chico.
pub fn preview_value(v: &Value, max: usize) -> String {
    let s = v.to_string();
    if s.chars().count() <= max {
        s
    } else if max < 3 {
        s.chars().take(max).collect()
    } else {
        let truncated: String = s.chars().take(max - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn human_label_prefers_name_over_id() {
        let id = Uuid::new_v4();
        let v = json!({"name": "Acme S.A.", "email": "x@y.z"});
        let label = human_label_for_record(&v, &id);
        assert!(label.starts_with("Acme S.A."));
        assert!(label.contains(&short_uuid(&id)));
    }

    #[test]
    fn human_label_falls_back_through_label_title_sku() {
        let id = Uuid::new_v4();
        let only_label = json!({"label": "X"});
        assert!(human_label_for_record(&only_label, &id).starts_with("X "));
        let only_title = json!({"title": "Y"});
        assert!(human_label_for_record(&only_title, &id).starts_with("Y "));
        let only_sku = json!({"sku": "Z"});
        assert!(human_label_for_record(&only_sku, &id).starts_with("Z "));
        let only_sku_id = json!({"sku_id": "W"});
        assert!(human_label_for_record(&only_sku_id, &id).starts_with("W "));
    }

    #[test]
    fn human_label_falls_back_to_short_uuid_when_no_keys_match() {
        let id = Uuid::new_v4();
        let v = json!({"random": "field"});
        assert_eq!(human_label_for_record(&v, &id), short_uuid(&id));
    }

    #[test]
    fn human_label_recognizes_spanish_name_fields() {
        let id = Uuid::new_v4();
        assert!(human_label_for_record(&json!({"nombre": "Acme"}), &id).starts_with("Acme "));
        assert!(human_label_for_record(&json!({"titulo": "Trato"}), &id).starts_with("Trato "));
    }

    #[test]
    fn format_value_number_groups_thousands() {
        assert_eq!(
            format_value(Some(&json!(12000)), &ValueFormat::Number),
            "12,000"
        );
        assert_eq!(format_value(Some(&json!(5)), &ValueFormat::Number), "5");
        assert_eq!(
            format_value(Some(&json!(-1234567)), &ValueFormat::Number),
            "-1,234,567"
        );
    }

    #[test]
    fn format_value_currency_prefixes_symbol() {
        let fmt = ValueFormat::Currency { symbol: "$".into() };
        assert_eq!(format_value(Some(&json!(25000)), &fmt), "$25,000");
    }

    #[test]
    fn format_value_float_gets_two_decimals() {
        assert_eq!(
            format_value(Some(&json!(1234.5)), &ValueFormat::Number),
            "1,234.50"
        );
    }

    #[test]
    fn cmp_values_orders_numbers_strings_nulls() {
        // Números por valor, no lexicográfico.
        assert_eq!(
            cmp_values(Some(&json!(2)), Some(&json!(10))),
            Ordering::Less
        );
        // Strings case-insensitive.
        assert_eq!(
            cmp_values(Some(&json!("banana")), Some(&json!("Apple"))),
            Ordering::Greater
        );
        // null/None ordena primero.
        assert_eq!(cmp_values(None, Some(&json!(1))), Ordering::Less);
        assert_eq!(
            cmp_values(Some(&Value::Null), Some(&json!("x"))),
            Ordering::Less
        );
        assert_eq!(
            cmp_values(Some(&json!(5)), Some(&json!(5))),
            Ordering::Equal
        );
        // Bools.
        assert_eq!(
            cmp_values(Some(&json!(false)), Some(&json!(true))),
            Ordering::Less
        );
    }

    #[test]
    fn format_value_non_number_falls_back_to_render_value() {
        assert_eq!(
            format_value(Some(&json!("hola")), &ValueFormat::Plain),
            "hola"
        );
        let fmt = ValueFormat::Currency { symbol: "$".into() };
        assert_eq!(format_value(Some(&json!("x")), &fmt), "x");
        assert_eq!(format_value(None, &ValueFormat::Number), "");
    }

    #[test]
    fn render_value_handles_basic_kinds() {
        assert_eq!(render_value(None), "");
        assert_eq!(render_value(Some(&Value::Null)), "");
        assert_eq!(render_value(Some(&json!("hola"))), "hola");
        assert_eq!(render_value(Some(&json!(true))), "✓");
        assert_eq!(render_value(Some(&json!(false))), "✗");
        assert_eq!(render_value(Some(&json!(42))), "42");
    }

    #[test]
    fn value_to_input_text_round_trip_with_strings_and_numbers() {
        assert_eq!(value_to_input_text(&Value::Null), "");
        assert_eq!(value_to_input_text(&json!("x")), "x");
        assert_eq!(value_to_input_text(&json!(true)), "true");
        assert_eq!(value_to_input_text(&json!(false)), "false");
        assert_eq!(value_to_input_text(&json!(42)), "42");
    }

    #[test]
    fn short_hash_takes_first_4_bytes_hex() {
        let mut h = [0u8; 32];
        h[0] = 0xaa;
        h[1] = 0xbb;
        h[2] = 0xcc;
        h[3] = 0xdd;
        assert_eq!(short_hash(&h), "aabbccdd");
    }

    #[test]
    fn short_hash_zeros() {
        let h = [0u8; 32];
        assert_eq!(short_hash(&h), "00000000");
    }

    #[test]
    fn preview_value_keeps_short_strings_intact() {
        let v = json!({"a": 1});
        assert_eq!(preview_value(&v, 30), "{\"a\":1}");
    }

    #[test]
    fn preview_value_truncates_long_strings_with_ellipsis() {
        let v = json!({"a": "x".repeat(200)});
        let p = preview_value(&v, 30);
        assert!(p.chars().count() <= 30);
        assert!(p.ends_with("..."));
    }

    #[test]
    fn preview_value_handles_max_smaller_than_ellipsis() {
        // Edge case: max < 3 (no espacio para "..."). Devuelve
        // los primeros `max` chars sin sufijo, sin panic.
        let v = json!("xxxxxxxxxxxxxxxx");
        let p = preview_value(&v, 2);
        assert!(p.chars().count() <= 2);
    }

    #[test]
    fn short_uuid_returns_first_8_chars() {
        let id = Uuid::parse_str("01ARZ3ND-EKTS-V4RR-FFQ6-9G5FAV000000").ok();
        // Si el parse falla, usamos uno fresco — el invariant es la
        // longitud, no el contenido.
        let id = id.unwrap_or_else(Uuid::new_v4);
        assert_eq!(short_uuid(&id).len(), 8);
    }
}
