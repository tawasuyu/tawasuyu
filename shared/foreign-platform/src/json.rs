//! Un extractor de **paths con puntos** sobre `serde_json::Value`. Es lo que
//! deja que el mapeo viva en un descriptor de texto: un campo del dominio se
//! describe como `"videoThumbnails.0.url"` y este módulo lo resuelve contra el
//! JSON crudo de la plataforma.
//!
//! Sintaxis del path (deliberadamente mínima, no es JSONPath completo):
//!   - segmentos separados por `.`
//!   - segmento numérico ⇒ índice de array (`0`, `1`, …)
//!   - segmento de texto ⇒ clave de objeto
//!   - path vacío `""` ⇒ el propio valor (útil para "la raíz es el array")

use serde_json::Value;

/// Navega `root` siguiendo `path` (segmentos separados por `.`). Devuelve
/// `None` si algún segmento no existe o el tipo no encaja.
pub fn get<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    let mut cur = root;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(map) => map.get(seg)?,
            Value::Array(arr) => {
                let idx: usize = seg.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(cur)
}

/// Extrae un `String` desde `path`. Acepta tanto JSON string como número
/// (algunas APIs mandan ids/fechas como número) coercionándolo a texto.
pub fn get_string(root: &Value, path: &str) -> Option<String> {
    match get(root, path)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Extrae un `u64` desde `path`. Acepta número JSON o string numérica
/// (las plataformas mezclan ambas para `viewCount`, `lengthSeconds`, …).
pub fn get_u64(root: &Value, path: &str) -> Option<u64> {
    match get(root, path)? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Devuelve el array que vive en `path` como slice. `path` vacío ⇒ la raíz
/// debe ser ella misma un array. `None` si no hay array ahí.
pub fn get_array<'a>(root: &'a Value, path: &str) -> Option<&'a [Value]> {
    match get(root, path)? {
        Value::Array(arr) => Some(arr.as_slice()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn navega_objetos_y_arrays() {
        let v = json!({
            "videoId": "abc",
            "lengthSeconds": 212,
            "viewCount": "104500",
            "videoThumbnails": [{ "url": "https://x/t.jpg" }],
        });
        assert_eq!(get_string(&v, "videoId").as_deref(), Some("abc"));
        assert_eq!(get_u64(&v, "lengthSeconds"), Some(212));
        // string numérica coercionada:
        assert_eq!(get_u64(&v, "viewCount"), Some(104_500));
        // índice de array + clave anidada:
        assert_eq!(
            get_string(&v, "videoThumbnails.0.url").as_deref(),
            Some("https://x/t.jpg")
        );
        // path inexistente:
        assert_eq!(get(&v, "nope"), None);
        assert_eq!(get(&v, "videoThumbnails.9.url"), None);
    }

    #[test]
    fn raiz_como_array() {
        let v = json!([{ "videoId": "a" }, { "videoId": "b" }]);
        let arr = get_array(&v, "").unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(get_string(&arr[1], "videoId").as_deref(), Some("b"));
    }
}
