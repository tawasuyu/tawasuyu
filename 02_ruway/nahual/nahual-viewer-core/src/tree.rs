//! `tree` — núcleo agnóstico del visor de tree de nahual (parseo + tipos de preview). El render vive en `nahual-tree-viewer-llimphi`.

use serde_json::Value;
use std::path::Path;

/// Tope de bytes a leer (1 MiB). Un árbol más grande que eso no se
/// escanea a ojo de todas formas; el caller puede subirlo si hace falta.
pub const DEFAULT_TREE_BYTES_MAX: u64 = 1024 * 1024;

/// Líneas y profundidad máximas del render. Cortan árboles enormes para
/// que parley no se atragante y el panel siga instantáneo.
const MAX_LINES: usize = 600;
const MAX_DEPTH: usize = 24;
/// Strings más largos que esto se truncan con `…` (un valor suelto no
/// debe empujar el árbol fuera de pantalla).
const MAX_STR: usize = 96;

/// Estado del visor. La forma replica al text viewer para que el shell
/// lo trate igual.
#[derive(Debug, Clone)]
pub enum TreePreview {
    /// Sin archivo seleccionado.
    Empty,
    /// Árbol renderizado (posiblemente truncado a [`MAX_LINES`]).
    Tree(String),
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// Parseo o E/S falló.
    Error(String),
}

impl Default for TreePreview {
    fn default() -> Self {
        TreePreview::Empty
    }
}

/// Lee y parsea el archivo. JSON vía `serde_json`, TOML vía `toml` (ambos
/// deserializan a `serde_json::Value`, el modelo unificado). El formato
/// se prueba JSON primero (lo más común) y TOML como fallback.
pub fn load_tree(path: &Path, max_bytes: u64) -> TreePreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return TreePreview::TooBig(meta.len()),
        Err(e) => return TreePreview::Error(e.to_string()),
        _ => {}
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return TreePreview::Error(e.to_string()),
    };
    let value = serde_json::from_str::<Value>(&src).or_else(|_| toml::from_str::<Value>(&src));
    match value {
        Ok(v) => TreePreview::Tree(render_tree(&v)),
        Err(e) => TreePreview::Error(e.to_string()),
    }
}

/// Renderiza el valor raíz como un árbol indentado.
fn render_tree(root: &Value) -> String {
    let mut out = String::new();
    let mut lines = 0usize;
    walk(root, 0, "root", &mut out, &mut lines);
    if lines >= MAX_LINES {
        out.push_str("\n… (árbol truncado)");
    }
    out
}

/// Emite una línea por nodo. Compuestos muestran `tipo (n)` y recursan;
/// escalares se imprimen inline. `label` es la clave/índice del padre.
fn walk(v: &Value, depth: usize, label: &str, out: &mut String, lines: &mut usize) {
    if *lines >= MAX_LINES {
        return;
    }
    let indent = "  ".repeat(depth);
    match v {
        Value::Object(map) => {
            push_line(
                out,
                lines,
                &format!("{indent}{label}: object ({})", map.len()),
            );
            if depth + 1 > MAX_DEPTH {
                push_line(out, lines, &format!("{indent}  … (demasiado profundo)"));
                return;
            }
            for (k, child) in map {
                walk(child, depth + 1, k, out, lines);
                if *lines >= MAX_LINES {
                    break;
                }
            }
        }
        Value::Array(arr) => {
            push_line(
                out,
                lines,
                &format!("{indent}{label}: array ({})", arr.len()),
            );
            if depth + 1 > MAX_DEPTH {
                push_line(out, lines, &format!("{indent}  … (demasiado profundo)"));
                return;
            }
            for (i, child) in arr.iter().enumerate() {
                let idx = format!("[{i}]");
                walk(child, depth + 1, &idx, out, lines);
                if *lines >= MAX_LINES {
                    break;
                }
            }
        }
        scalar => {
            push_line(
                out,
                lines,
                &format!("{indent}{label}: {}", fmt_scalar(scalar)),
            );
        }
    }
}

fn push_line(out: &mut String, lines: &mut usize, line: &str) {
    if *lines >= MAX_LINES {
        return;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    *lines += 1;
}

fn fmt_scalar(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let shown: String = if s.chars().count() > MAX_STR {
                let head: String = s.chars().take(MAX_STR).collect();
                format!("{head}…")
            } else {
                s.clone()
            };
            // Una línea: sin saltos que rompan la indentación.
            format!("\"{}\"", shown.replace('\n', "⏎"))
        }
        // Object/Array no llegan acá (los maneja `walk`).
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_minificado_se_indenta() {
        let v: Value = serde_json::from_str(r#"{"a":1,"b":[true,null],"c":{"d":"x"}}"#).unwrap();
        let out = render_tree(&v);
        assert!(out.contains("root: object (3)"));
        assert!(out.contains("  a: 1"));
        assert!(out.contains("  b: array (2)"));
        assert!(out.contains("    [0]: true"));
        assert!(out.contains("    [1]: null"));
        assert!(out.contains("  c: object (1)"));
        assert!(out.contains("    d: \"x\""));
    }

    #[test]
    fn string_largo_se_trunca() {
        let long = "z".repeat(MAX_STR + 50);
        let v = Value::String(long);
        let s = fmt_scalar(&v);
        assert!(s.ends_with("…\""));
        assert!(s.chars().count() <= MAX_STR + 4);
    }

    #[test]
    fn toml_tambien_parsea() {
        let tmp = std::env::temp_dir().join("nahual-tree-viewer-test.toml");
        std::fs::write(&tmp, b"title = \"x\"\n[owner]\nname = \"y\"\n").unwrap();
        match load_tree(&tmp, DEFAULT_TREE_BYTES_MAX) {
            TreePreview::Tree(s) => {
                assert!(s.contains("title: \"x\""));
                assert!(s.contains("owner: object (1)"));
            }
            other => panic!("esperaba Tree, obtuve {other:?}"),
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn basura_es_error() {
        let tmp = std::env::temp_dir().join("nahual-tree-viewer-test-bad.json");
        std::fs::write(&tmp, b"\x00\x01 no soy json ni toml =[").unwrap();
        assert!(matches!(
            load_tree(&tmp, DEFAULT_TREE_BYTES_MAX),
            TreePreview::Error(_)
        ));
        let _ = std::fs::remove_file(&tmp);
    }
}
