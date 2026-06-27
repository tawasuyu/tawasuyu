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

/// Devuelve `ron_text` con el `wallpaper_path` del **override de la salida
/// `output`** puesto en `new_path`. Si esa salida no tiene override, le agrega
/// uno (creando la lista `outputs` si falta). Como [`set_wallpaper_path`],
/// preserva comentarios, sangría y el resto del archivo: edita sólo la tupla
/// del override (`(name: "<output>", …)`) sin pasar por un round-trip serde.
///
/// La localización de la tupla es por el campo `name`: se busca el literal
/// `"<output>"` precedido de `name:` y se expande al `(` … `)` que lo encierra.
/// Funciona para el formato inline que mirada escribe
/// (`(name: "DP-1", wallpaper_path: "x"），…)`). Por las dudas, el llamante
/// (`run_once`) revalida que el RON resultante reparsee antes de escribirlo.
pub fn set_output_wallpaper_path(ron_text: &str, output: &str, new_path: &str) -> String {
    let esc_path = escape_ron_string(new_path);
    match find_output_tuple(ron_text, output) {
        Some((start, end)) => {
            let new_tuple = set_string_field(&ron_text[start..end], "wallpaper_path", &esc_path);
            let mut out = String::with_capacity(ron_text.len() + esc_path.len() + 32);
            out.push_str(&ron_text[..start]);
            out.push_str(&new_tuple);
            out.push_str(&ron_text[end..]);
            out
        }
        None => insert_output_override(ron_text, output, &esc_path),
    }
}

/// Encuentra la tupla `( … )` del override cuya clave `name` vale `output`.
/// Devuelve `(inicio_del_paréntesis, fin_tras_el_paréntesis)`.
fn find_output_tuple(ron_text: &str, output: &str) -> Option<(usize, usize)> {
    let esc = escape_ron_string(output);
    // Buscamos el literal del nombre y confirmamos que lo precede `name:`.
    let needle = format!("\"{esc}\"");
    let mut from = 0;
    while let Some(rel) = ron_text[from..].find(&needle) {
        let name_val = from + rel;
        let before = ron_text[..name_val].trim_end();
        if before.ends_with("name:") {
            let open = ron_text[..name_val].rfind('(')?;
            let close = ron_text[name_val..].find(')').map(|i| name_val + i + 1)?;
            return Some((open, close));
        }
        from = name_val + needle.len();
    }
    None
}

/// Reemplaza (o inserta) el valor de cadena del campo `field` dentro de una
/// tupla RON `( … )`. Si el campo existe, reemplaza su literal `"…"`
/// (respetando escapes); si no, lo inserta antes del `)` de cierre.
fn set_string_field(tuple: &str, field: &str, esc_value: &str) -> String {
    let key = format!("{field}:");
    if let Some(kpos) = tuple.find(&key) {
        // Salto de ws hasta la comilla de apertura del valor.
        let after_key = kpos + key.len();
        let bytes = tuple.as_bytes();
        let mut i = after_key;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'"' {
            let val_end = string_literal_end(bytes, i);
            let mut out = String::with_capacity(tuple.len() + esc_value.len());
            out.push_str(&tuple[..i]);
            out.push('"');
            out.push_str(esc_value);
            out.push('"');
            out.push_str(&tuple[val_end..]);
            return out;
        }
    }
    // No estaba el campo: insertarlo antes del `)` de cierre.
    match tuple.rfind(')') {
        Some(idx) => {
            let mut out = String::with_capacity(tuple.len() + esc_value.len() + field.len() + 8);
            out.push_str(&tuple[..idx]);
            out.push_str(", ");
            out.push_str(field);
            out.push_str(": \"");
            out.push_str(esc_value);
            out.push_str("\"");
            out.push_str(&tuple[idx..]);
            out
        }
        None => tuple.to_string(),
    }
}

/// Dado `bytes` y el índice de una comilla `"` de apertura, devuelve el índice
/// **tras** la comilla de cierre, saltando escapes `\"` y `\\`.
fn string_literal_end(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Inserta un override nuevo `(name: "<output>", wallpaper_path: "<path>")` en
/// la lista `outputs`. Si la lista no existe, la crea antes del `)` raíz.
fn insert_output_override(ron_text: &str, output: &str, esc_path: &str) -> String {
    let esc_name = escape_ron_string(output);
    let entry = format!("(name: \"{esc_name}\", wallpaper_path: \"{esc_path}\")");
    // ¿Hay `outputs: [` ? Insertamos justo después del `[`.
    if let Some(kpos) = ron_text.find("outputs:") {
        if let Some(rel) = ron_text[kpos..].find('[') {
            let bracket = kpos + rel + 1;
            let mut out = String::with_capacity(ron_text.len() + entry.len() + 16);
            out.push_str(&ron_text[..bracket]);
            out.push_str("\n        ");
            out.push_str(&entry);
            out.push(',');
            out.push_str(&ron_text[bracket..]);
            return out;
        }
    }
    // Sin lista `outputs`: crearla antes del `)` raíz.
    match ron_text.rfind(')') {
        Some(idx) => {
            let mut out = String::with_capacity(ron_text.len() + entry.len() + 24);
            out.push_str(&ron_text[..idx]);
            out.push_str("    outputs: [");
            out.push_str(&entry);
            out.push_str("],\n");
            out.push_str(&ron_text[idx..]);
            out
        }
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

    /// Per-output: reemplaza el `wallpaper_path` del override existente sin
    /// tocar el global ni el de otras salidas.
    #[test]
    fn set_output_replaces_existing_override() {
        let src = "(\n    wallpaper_path: \"global.png\",\n    outputs: [\n        (name: \"DP-1\", wallpaper_path: \"viejo.png\"),\n        (name: \"HDMI-A-1\", wallpaper_path: \"sala.png\"),\n    ],\n)\n";
        let out = set_output_wallpaper_path(src, "DP-1", "nuevo.png");
        let cfg = mirada_brain::Config::from_ron(&out).expect("RON válido");
        assert_eq!(cfg.wallpaper_path, "global.png", "global intacto");
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "nuevo.png");
        assert_eq!(cfg.wallpaper_path_for("HDMI-A-1"), "sala.png", "otra salida intacta");
    }

    /// Per-output: si el override existe pero sin `wallpaper_path`, lo inserta.
    #[test]
    fn set_output_inserts_field_in_override_without_it() {
        let src = "(\n    outputs: [\n        (name: \"DP-1\", scale_120: 180),\n    ],\n)\n";
        let out = set_output_wallpaper_path(src, "DP-1", "x.png");
        let cfg = mirada_brain::Config::from_ron(&out).expect("RON válido");
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "x.png");
        assert_eq!(cfg.outputs[0].scale_120, 180, "otros campos del override intactos");
    }

    /// Per-output: si la salida no tiene override, lo agrega a la lista.
    #[test]
    fn set_output_appends_override_to_list() {
        let src = "(\n    outputs: [\n        (name: \"DP-1\", wallpaper_path: \"dp.png\"),\n    ],\n)\n";
        let out = set_output_wallpaper_path(src, "HDMI-A-1", "tv.png");
        let cfg = mirada_brain::Config::from_ron(&out).expect("RON válido");
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "dp.png", "el existente queda");
        assert_eq!(cfg.wallpaper_path_for("HDMI-A-1"), "tv.png", "el nuevo se agrega");
    }

    /// Per-output: sin lista `outputs`, se crea con el override.
    #[test]
    fn set_output_creates_outputs_list_when_absent() {
        let src = "(\n    wallpaper_path: \"global.png\",\n)\n";
        let out = set_output_wallpaper_path(src, "DP-1", "dp.png");
        let cfg = mirada_brain::Config::from_ron(&out).expect("RON válido");
        assert_eq!(cfg.wallpaper_path, "global.png");
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "dp.png");
    }

    /// Per-output: el comentario de la config se preserva.
    #[test]
    fn set_output_keeps_comments() {
        let src = "(\n    // fondo del escritorio\n    outputs: [\n        (name: \"DP-1\", wallpaper_path: \"a.png\"),\n    ],\n)\n";
        let out = set_output_wallpaper_path(src, "DP-1", "b.png");
        assert!(out.contains("// fondo del escritorio"), "comentario preservado");
        let cfg = mirada_brain::Config::from_ron(&out).unwrap();
        assert_eq!(cfg.wallpaper_path_for("DP-1"), "b.png");
    }
}
