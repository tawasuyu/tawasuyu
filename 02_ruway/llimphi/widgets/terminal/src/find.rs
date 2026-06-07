//! Búsqueda sobre el `Scrollback` — base de Ctrl+F del SDD-TERMINAL §Fase 3.
//!
//! Diseño: barata pero correcta. Recorre todas las líneas del store y
//! reporta los rangos `(line, start_byte, end_byte)` de cada ocurrencia.
//! Sin streaming, sin índice — para los típicos cientos de miles de líneas
//! del shell es suficiente (un `memmem` por línea, lineal en el contenido).
//!
//! Para infinitos masivos (millones de líneas), el `find` ya es O(N) en el
//! contenido, no en el render — el scroll y la pintada siguen siendo O(1).
//! Si en algún momento aprieta, se puede pre-indexar n-gramas; no hoy.
//!
//! Case-insensitive: lowercase ambos lados (sin Unicode-aware folding por
//! ahora — ASCII alcanza para el caso shell típico).

use crate::store::Scrollback;

/// Una coincidencia de búsqueda en el scrollback. `start`/`end` son offsets
/// **en bytes** UTF-8 del texto de la línea (slice-safe: el caller puede
/// hacer `&text[start..end]` sin clampear).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatch {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// Opciones de búsqueda. Defaults: case-sensitive, query literal (sin regex).
#[derive(Debug, Clone, Copy, Default)]
pub struct FindOpts {
    /// `true` = lowercase ambos lados antes de comparar (ASCII fold).
    pub case_insensitive: bool,
}

/// Busca todas las ocurrencias **no superpuestas** de `query` en el `store`,
/// línea por línea, en orden. Empty query → `Vec::new()` (paridad con la
/// barra de find de la mayoría de editores: vacío = "no hay nada que
/// resaltar"). El consumo es O(total_bytes) en el contenido del store.
///
/// Las coincidencias caen siempre en límites de char UTF-8 (vienen del
/// scanner de bytes y se snap-ean al borde más cercano hacia abajo si la
/// query atraviesa una codepoint, que con `find` literal no debería pasar).
pub fn find_matches(store: &Scrollback, query: &str, opts: FindOpts) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = if opts.case_insensitive {
        query.to_ascii_lowercase()
    } else {
        query.to_string()
    };
    let mut out = Vec::new();
    for line in 0..store.len() {
        let Some(text) = store.line(line) else { continue };
        let haystack_owned;
        let haystack: &str = if opts.case_insensitive {
            haystack_owned = text.to_ascii_lowercase();
            &haystack_owned
        } else {
            text
        };
        let mut cursor = 0usize;
        while cursor < haystack.len() {
            let Some(rel) = haystack[cursor..].find(&needle) else {
                break;
            };
            let start = cursor + rel;
            let end = start + needle.len();
            out.push(FindMatch { line, start, end });
            // Avance no-superposición: una ocurrencia consume su rango, la
            // siguiente arranca DESPUÉS. Si la query es vacía no llegamos
            // acá (ya filtrado arriba), así que `end > start` siempre.
            cursor = end;
        }
    }
    out
}

/// Avanza al siguiente match desde `current` (envuelve al primero si está
/// al final). Si `matches` está vacío devuelve `None`. `None` en `current`
/// equivale a "no hay actual" → arranca por el primero.
pub fn next_match(matches: &[FindMatch], current: Option<usize>) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(match current {
        None => 0,
        Some(i) => (i + 1) % matches.len(),
    })
}

/// Retrocede al match previo desde `current` (envuelve al último si está
/// al principio). Mismas semánticas que [`next_match`].
pub fn prev_match(matches: &[FindMatch], current: Option<usize>) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(match current {
        None => matches.len() - 1,
        Some(i) => (i + matches.len() - 1) % matches.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_of(lines: &[&str]) -> Scrollback {
        let mut s = Scrollback::new(0);
        for l in lines {
            s.push_line(l);
        }
        s
    }

    #[test]
    fn query_vacia_no_devuelve_nada() {
        let s = store_of(&["foo", "bar"]);
        assert_eq!(find_matches(&s, "", FindOpts::default()), Vec::new());
    }

    #[test]
    fn una_ocurrencia_por_linea() {
        let s = store_of(&["foo bar baz", "qux foo quux"]);
        let m = find_matches(&s, "foo", FindOpts::default());
        assert_eq!(
            m,
            vec![
                FindMatch { line: 0, start: 0, end: 3 },
                FindMatch { line: 1, start: 4, end: 7 },
            ]
        );
    }

    #[test]
    fn varias_ocurrencias_en_la_misma_linea_no_se_superponen() {
        // "aaa" → en "aaaaa" hay 1 match en 0..3 y otro en 3..6 (no en 1..4).
        let s = store_of(&["aaaaaa"]);
        let m = find_matches(&s, "aaa", FindOpts::default());
        assert_eq!(
            m,
            vec![
                FindMatch { line: 0, start: 0, end: 3 },
                FindMatch { line: 0, start: 3, end: 6 },
            ]
        );
    }

    #[test]
    fn case_sensitive_por_defecto() {
        let s = store_of(&["Foo", "FOO", "foo"]);
        let m = find_matches(&s, "foo", FindOpts::default());
        assert_eq!(m, vec![FindMatch { line: 2, start: 0, end: 3 }]);
    }

    #[test]
    fn case_insensitive_matchea_todas_las_variantes() {
        let s = store_of(&["Foo", "FOO", "foo"]);
        let m = find_matches(&s, "foo", FindOpts { case_insensitive: true });
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].line, 0);
        assert_eq!(m[1].line, 1);
        assert_eq!(m[2].line, 2);
    }

    #[test]
    fn no_match_devuelve_vec_vacio() {
        let s = store_of(&["uno", "dos"]);
        assert!(find_matches(&s, "xyz", FindOpts::default()).is_empty());
    }

    #[test]
    fn match_utf8_funciona_al_ser_busqueda_literal_byte_a_byte() {
        // "café" tiene 'é' = 2 bytes. Buscamos "afé" — match en bytes 1..5.
        let s = store_of(&["café"]);
        let m = find_matches(&s, "afé", FindOpts::default());
        assert_eq!(m, vec![FindMatch { line: 0, start: 1, end: 5 }]);
    }

    #[test]
    fn next_match_envuelve_al_primero() {
        let m = vec![
            FindMatch { line: 0, start: 0, end: 1 },
            FindMatch { line: 1, start: 0, end: 1 },
        ];
        assert_eq!(next_match(&m, None), Some(0));
        assert_eq!(next_match(&m, Some(0)), Some(1));
        assert_eq!(next_match(&m, Some(1)), Some(0));
    }

    #[test]
    fn prev_match_envuelve_al_ultimo() {
        let m = vec![
            FindMatch { line: 0, start: 0, end: 1 },
            FindMatch { line: 1, start: 0, end: 1 },
        ];
        assert_eq!(prev_match(&m, None), Some(1));
        assert_eq!(prev_match(&m, Some(0)), Some(1));
        assert_eq!(prev_match(&m, Some(1)), Some(0));
    }

    #[test]
    fn next_y_prev_en_lista_vacia_son_none() {
        let m: Vec<FindMatch> = Vec::new();
        assert_eq!(next_match(&m, None), None);
        assert_eq!(next_match(&m, Some(0)), None);
        assert_eq!(prev_match(&m, None), None);
    }
}
