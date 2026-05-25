//! Parser de wiki-links — los destinos `[[...]]` dentro de una nota.
//!
//! Un solo formato: dobles corchetes con el título adentro. El texto se
//! recorta; los enlaces vacíos se descartan; el orden de aparición se
//! conserva y se deduplica (un mismo destino enlazado dos veces cuenta
//! una sola vez como arista).

/// Extrae los destinos `[[...]]` de `text`, recortados y deduplicados,
/// en orden de aparición.
pub fn parse_links(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 3 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(close) = find_close(text, i + 2) {
                let inner = text[i + 2..close].trim();
                if !inner.is_empty() && !out.iter().any(|l| l == inner) {
                    out.push(inner.to_string());
                }
                i = close + 2;
                continue;
            }
        }
        // Avanza un carácter UTF-8 completo, no un byte.
        i += utf8_len(bytes[i]);
    }
    out
}

/// Posición del `]]` que cierra a partir de `from`, si existe.
fn find_close(text: &str, from: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b']' && bytes[i + 1] == b']' {
            return Some(i);
        }
        // Un `[[` antes del cierre aborta: enlaces anidados no son válidos.
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            return None;
        }
        i += 1;
    }
    None
}

/// Largo en bytes del carácter UTF-8 que empieza en `b`.
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_links() {
        let links = parse_links("ver [[Cocina]] y también [[Jardín]].");
        assert_eq!(links, vec!["Cocina", "Jardín"]);
    }

    #[test]
    fn trims_inner_whitespace() {
        assert_eq!(parse_links("[[  Taller  ]]"), vec!["Taller"]);
    }

    #[test]
    fn empty_links_are_dropped() {
        assert_eq!(parse_links("[[]] y [[   ]]"), Vec::<String>::new());
    }

    #[test]
    fn duplicates_collapse_to_one() {
        assert_eq!(parse_links("[[A]] [[B]] [[A]]"), vec!["A", "B"]);
    }

    #[test]
    fn unclosed_bracket_is_ignored() {
        assert_eq!(parse_links("texto [[sin cerrar"), Vec::<String>::new());
    }

    #[test]
    fn handles_unicode_content_around_links() {
        let links = parse_links("café ☕ con [[Niños]] — añoño");
        assert_eq!(links, vec!["Niños"]);
    }
}
