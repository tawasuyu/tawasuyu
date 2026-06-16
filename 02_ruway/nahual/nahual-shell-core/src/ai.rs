//! Helpers puros de la acción IA del shell: detección de texto, lectura de
//! snippets de contenido y saneo de los nombres que propone el LLM. Es lógica
//! reusable sin UI, sin LLM y sin `Handle`; el armado de prompts, la llamada al
//! backend (`pluma-llm`) y el ruteo de `Msg` viven en el frontend (orquestación,
//! Regla 2).

use std::path::Path;

/// ¿La extensión sugiere texto legible para incluir su contenido en el prompt?
pub fn es_texto(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some(
            "rs" | "toml" | "md" | "txt" | "json" | "yaml" | "yml" | "html" | "css" | "js"
                | "ts" | "py" | "c" | "h" | "cpp" | "go" | "sh" | "lua" | "rb" | "sql" | "xml"
                | "ini" | "cfg" | "conf" | "csv" | "rhai"
        )
    )
}

/// Lee hasta `max` bytes del inicio de `path` como texto (lossy).
pub fn leer_snippet(path: &Path, max: usize) -> Option<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; max];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Sanea una propuesta de nombre de la IA para que sea un filename válido:
/// saca rutas (`/`, `\`), recorta y colapsa espacios. Vacío → cadena vacía
/// (el batch la trata como "conservar el original").
pub fn sanear_nombre(s: &str) -> String {
    let limpio: String = s
        .trim()
        // La IA a veces numera ("1. nombre") o agrega comillas/backticks.
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == ' ')
        .trim_matches(|c| c == '"' || c == '`' || c == '\'')
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    limpio.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El saneo robustece el parseo de la respuesta LLM: saca numeración,
    /// comillas/backticks, rutas y espacios sobrantes.
    #[test]
    fn sanear_nombre_robusto() {
        assert_eq!(sanear_nombre("  atardecer_playa.jpg "), "atardecer_playa.jpg");
        assert_eq!(sanear_nombre("1. informe_anual.pdf"), "informe_anual.pdf");
        assert_eq!(sanear_nombre("`notas.md`"), "notas.md");
        assert_eq!(sanear_nombre("\"foto final.png\""), "foto final.png");
        // Las rutas se aplanan: ningún `/` sobrevive (no se renombra fuera del
        // dir). El `..` inicial se recorta; las barras se vuelven `_`.
        let aplanado = sanear_nombre("../x/y.txt");
        assert!(!aplanado.contains('/'), "sin barras: {aplanado}");
        assert_eq!(aplanado, "_x_y.txt");
        assert_eq!(sanear_nombre("   "), "");
    }
}
