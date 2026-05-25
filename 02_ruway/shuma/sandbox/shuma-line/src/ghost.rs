//! Sugerencia fantasma — el "ghosting" predictivo del prompt.
//!
//! Mientras se escribe, el shell predice el resto de la línea y lo pinta
//! en gris tenue. Esta función es el cerebro de esa predicción: dada la
//! línea parcial y un corpus de líneas conocidas (historial, secuencias
//! inferidas), devuelve el sufijo que falta.
//!
//! El orden del corpus es la prioridad: el caller pone primero lo más
//! relevante (la secuencia predicha por `shuma-infer`), luego el
//! historial de lo más reciente a lo más viejo.

/// Devuelve el sufijo fantasma: lo que falta para completar la primera
/// entrada del `corpus` que empieza con `line` y es estrictamente más
/// larga. `None` si nada coincide.
pub fn ghost_suggestion(line: &str, corpus: &[String]) -> Option<String> {
    if line.is_empty() {
        return None;
    }
    corpus
        .iter()
        .find(|c| c.len() > line.len() && c.starts_with(line))
        .map(|c| c[line.len()..].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_the_remainder_of_a_known_line() {
        let corpus = vec!["git pull".to_string(), "cargo build".to_string()];
        assert_eq!(ghost_suggestion("git pu", &corpus), Some("ll".to_string()));
    }

    #[test]
    fn corpus_order_is_priority() {
        // Dos coinciden; gana la primera del corpus.
        let corpus = vec!["cargo build --release".to_string(), "cargo build".to_string()];
        assert_eq!(
            ghost_suggestion("cargo b", &corpus),
            Some("uild --release".to_string())
        );
    }

    #[test]
    fn no_match_yields_none() {
        let corpus = vec!["ls -la".to_string()];
        assert_eq!(ghost_suggestion("git", &corpus), None);
    }

    #[test]
    fn exact_line_is_not_a_suggestion() {
        // El corpus contiene exactamente la línea: nada que sugerir.
        let corpus = vec!["git pull".to_string()];
        assert_eq!(ghost_suggestion("git pull", &corpus), None);
    }

    #[test]
    fn empty_line_yields_none() {
        let corpus = vec!["git pull".to_string()];
        assert_eq!(ghost_suggestion("", &corpus), None);
    }
}
