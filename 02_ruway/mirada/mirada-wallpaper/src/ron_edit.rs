//! Edición **quirúrgica** del `config.ron` de mirada: cambiar el valor de
//! `wallpaper_path` sin tocar nada más —ni comentarios, ni el resto de los
//! campos, ni el formato—. Un round-trip por `serde`/`ron` borraría la
//! plantilla comentada que mirada le escribe al usuario; acá reemplazamos
//! sólo la línea del campo top-level.
//!
//! El campo top-level `wallpaper_path` se declara en el struct `Config` de
//! mirada **antes** de `outputs`, así que en el RON serializado aparece antes
//! que cualquier `wallpaper_path` de un `OutputOverride`. Tomamos la **primera**
//! línea cuyo contenido (sin sangría) empiece con `wallpaper_path:` — los
//! overrides van inline tras `(name:`, nunca abren línea con ese campo cuando
//! la config la escribió mirada. Si no hay ninguna, insertamos el campo antes
//! del paréntesis raíz de cierre.

/// Devuelve `ron_text` con el valor del campo top-level `wallpaper_path`
/// puesto en `new_path`. Preserva comentarios, sangría y el resto del archivo.
pub fn set_wallpaper_path(ron_text: &str, new_path: &str) -> String {
    let escaped = escape_ron_string(new_path);
    let mut out = String::with_capacity(ron_text.len() + escaped.len() + 24);
    let mut replaced = false;

    for line in ron_text.split_inclusive('\n') {
        if !replaced {
            let body = line.trim_start();
            if body.starts_with("wallpaper_path:") {
                let indent_len = line.len() - body.len();
                let indent = &line[..indent_len];
                let newline = if line.ends_with('\n') { "\n" } else { "" };
                out.push_str(indent);
                out.push_str("wallpaper_path: \"");
                out.push_str(&escaped);
                out.push_str("\",");
                out.push_str(newline);
                replaced = true;
                continue;
            }
        }
        out.push_str(line);
    }

    if replaced {
        return out;
    }
    // No existía el campo: insertarlo antes del último `)` (cierre del root).
    insert_before_root_close(ron_text, &escaped)
}

/// Inserta un campo `wallpaper_path` nuevo antes del paréntesis raíz de
/// cierre. Fallback para configs hechas a mano sin el campo (serde lo
/// completa al cargar, pero el texto no lo tiene).
fn insert_before_root_close(ron_text: &str, escaped: &str) -> String {
    match ron_text.rfind(')') {
        Some(idx) => {
            let mut out = String::with_capacity(ron_text.len() + escaped.len() + 24);
            out.push_str(&ron_text[..idx]);
            out.push_str("    wallpaper_path: \"");
            out.push_str(escaped);
            out.push_str("\",\n");
            out.push_str(&ron_text[idx..]);
            out
        }
        // Sin `)` no es un RON de Config válido; devolvemos intacto.
        None => ron_text.to_string(),
    }
}

/// Escapa `\` y `"` para que el path quepa en una cadena RON entre comillas.
fn escape_ron_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El campo top-level se reemplaza y los comentarios sobreviven.
    #[test]
    fn replaces_toplevel_and_keeps_comments() {
        let src = "(\n    // fondo\n    wallpaper_path: \"\",\n    wallpaper_fit: \"stretch\",\n)\n";
        let out = set_wallpaper_path(src, "/cache/bing-20260605.jpg");
        assert!(out.contains("wallpaper_path: \"/cache/bing-20260605.jpg\","));
        assert!(out.contains("// fondo"), "comentario preservado");
        assert!(out.contains("wallpaper_fit: \"stretch\""), "otros campos intactos");
        // Sigue siendo parseable por la Config de mirada.
        let cfg = mirada_brain::Config::from_ron(&out).expect("RON válido");
        assert_eq!(cfg.wallpaper_path, "/cache/bing-20260605.jpg");
    }

    /// No debe pisar el `wallpaper_path` de un override de salida: se toca el
    /// primero (top-level), que va antes en el texto serializado.
    #[test]
    fn does_not_touch_output_override() {
        let src = "(\n    wallpaper_path: \"old.png\",\n    outputs: [\n        (name: \"DP-1\", wallpaper_path: \"dp.png\"),\n    ],\n)\n";
        let out = set_wallpaper_path(src, "new.png");
        assert!(out.contains("wallpaper_path: \"new.png\","));
        assert!(out.contains("(name: \"DP-1\", wallpaper_path: \"dp.png\")"), "override intacto");
        let cfg = mirada_brain::Config::from_ron(&out).unwrap();
        assert_eq!(cfg.wallpaper_path, "new.png");
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "dp.png");
    }

    /// Una config minimal sin el campo: se inserta antes del cierre.
    #[test]
    fn inserts_when_absent() {
        let src = "(\n    gap: 8,\n)\n";
        let out = set_wallpaper_path(src, "x.png");
        let cfg = mirada_brain::Config::from_ron(&out).unwrap();
        assert_eq!(cfg.wallpaper_path, "x.png");
        assert_eq!(cfg.gap, 8);
    }

    /// Paths con comillas o backslashes se escapan y reparsean igual.
    #[test]
    fn escapes_special_chars() {
        let src = "(\n    wallpaper_path: \"\",\n)\n";
        let weird = "/home/a b/\"raro\".png";
        let out = set_wallpaper_path(src, weird);
        let cfg = mirada_brain::Config::from_ron(&out).unwrap();
        assert_eq!(cfg.wallpaper_path, weird);
    }
}
