//! Extractor de **paths con puntos** sobre `serde_json::Value` — lo que deja
//! que el mapeo campo→JSON viva en un descriptor de texto. Gemelo del de
//! `shared/foreign-platform`: deliberadamente mínimo (no es JSONPath).
//!
//! Sintaxis: segmentos separados por `.`; segmento numérico ⇒ índice de array;
//! segmento de texto ⇒ clave de objeto; path vacío `""` ⇒ el propio valor.

use serde_json::Value;

/// Navega `root` siguiendo `path`. `None` si algún segmento no existe.
pub fn get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => arr.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Extrae un `String` desde `path` (coerciona número a texto).
pub fn get_string(root: &Value, path: &str) -> Option<String> {
    match get(root, path)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// El array que vive en `path`. `""` ⇒ la raíz debe ser un array.
pub fn get_array<'a>(root: &'a Value, path: &str) -> Option<&'a [Value]> {
    match get(root, path)? {
        Value::Array(arr) => Some(arr.as_slice()),
        _ => None,
    }
}
