//! Detector de "input pendiente" — cuándo Enter debe seguir escribiendo
//! en vez de submit-ear.
//!
//! Convenciones bash que cubrimos:
//!
//! - **Comilla simple sin cerrar**: `echo 'hola` → continuación.
//! - **Comilla doble sin cerrar**: `echo "hola` → continuación.
//! - **Paréntesis sin cerrar**: subshell `(...)`, command substitution
//!   `$(...)`. Anidan.
//! - **Heredoc abierto**: `cat <<EOF` (con o sin `-`, con tag entre
//!   comillas o sin ellas). El heredoc se cierra con una línea que sólo
//!   contenga el tag (estrip de tabs si `<<-`).
//! - **`\` al final de línea** (line continuation clásica).
//! - **Operador pendiente al final**: `cmd |`, `cmd &&`, `cmd ||`. Sólo
//!   cuenta si NO está dentro de una cadena.
//!
//! NO cubrimos (a propósito):
//!
//! - `{...}` y `[...]` — en bash son comandos (`test`), expansión de
//!   llaves, o brace-groups; detectar correctamente cuándo "abren"
//!   requiere casi un parser completo. Si el usuario los escribe en
//!   varias líneas, puede usar `\` al final o `<<EOF` heredoc.
//!
//! El detector es deliberadamente *barato* (un pase lineal por los
//! bytes, O(n)) — se llama en cada Enter del frontend.

/// `true` si `text` tiene una construcción shell *abierta* que esperaba
/// más input. El frontend lo usa para decidir entre insertar `\n` (en
/// curso) o ejecutar (cerrado).
pub fn needs_continuation(text: &str) -> bool {
    let mut single_q = false;
    let mut double_q = false;
    let mut depth_paren: i32 = 0;
    let mut heredoc_tag: Option<String> = None;
    let mut heredoc_strip = false;
    // El último token shell relevante para detectar operadores
    // pendientes al final: pipe, &&, ||. Se resetea cuando vemos
    // contenido no-vacío después.
    let mut trailing_op: Option<TrailingOp> = None;
    // `\` justo antes de `\n` significa continuación pero también:
    // dentro de comillas simples, `\` es literal. Lo manejamos al final
    // mirando el último byte no-blanco del texto entero.

    let lines: Vec<&str> = text.split('\n').collect();
    for line in &lines {
        // Si estamos dentro de un heredoc body, sólo importa si esta
        // línea sola es el tag de cierre (con strip de tabs si <<-).
        if let Some(tag) = heredoc_tag.as_ref() {
            let candidate = if heredoc_strip {
                line.trim_start_matches('\t')
            } else {
                *line
            };
            if candidate == tag {
                heredoc_tag = None;
            }
            continue;
        }
        let bytes = line.as_bytes();
        let mut i = 0;
        let mut pending_heredoc: Option<(String, bool)> = None;
        let mut prev_backslash = false;
        // Por línea, resetemos el `trailing_op` y lo recalculamos.
        trailing_op = None;
        while i < bytes.len() {
            let c = bytes[i];
            if prev_backslash {
                prev_backslash = false;
                // Backslash escapó este caracter — no es operador ni quote.
                i += 1;
                continue;
            }
            if single_q {
                if c == b'\'' {
                    single_q = false;
                }
                i += 1;
                continue;
            }
            if double_q {
                if c == b'\\' && i + 1 < bytes.len() {
                    // En doble comilla, \" y \\ y \$ son escapes.
                    i += 2;
                    continue;
                }
                if c == b'"' {
                    double_q = false;
                }
                i += 1;
                continue;
            }
            match c {
                b'#' => break, // comentario hasta fin de línea
                b'\\' => {
                    prev_backslash = true;
                    trailing_op = None;
                }
                b'\'' => {
                    single_q = true;
                    trailing_op = None;
                }
                b'"' => {
                    double_q = true;
                    trailing_op = None;
                }
                b'(' => {
                    depth_paren += 1;
                    trailing_op = None;
                }
                b')' => {
                    if depth_paren > 0 {
                        depth_paren -= 1;
                    }
                    trailing_op = None;
                }
                b'<' if i + 1 < bytes.len() && bytes[i + 1] == b'<' => {
                    let mut start = i + 2;
                    let strip = bytes.get(start) == Some(&b'-');
                    if strip {
                        start += 1;
                    }
                    while start < bytes.len() && (bytes[start] == b' ' || bytes[start] == b'\t') {
                        start += 1;
                    }
                    // Tag entre comillas → literal (sin expansión); sin
                    // comillas → identificador.
                    let (tag, end) = if let Some(&q) = bytes.get(start) {
                        if q == b'\'' || q == b'"' {
                            let mut end = start + 1;
                            while end < bytes.len() && bytes[end] != q {
                                end += 1;
                            }
                            (
                                line[start + 1..end.min(line.len())].to_string(),
                                (end + 1).min(line.len()),
                            )
                        } else {
                            let mut end = start;
                            while end < bytes.len()
                                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                            {
                                end += 1;
                            }
                            (line[start..end].to_string(), end)
                        }
                    } else {
                        (String::new(), start)
                    };
                    if !tag.is_empty() {
                        pending_heredoc = Some((tag, strip));
                    }
                    i = end;
                    trailing_op = None;
                    continue;
                }
                b'|' => {
                    // `||` cuenta como `Or`, `|` como `Pipe`. Avanzamos
                    // dos bytes en el primer caso para no clasificarlo dos
                    // veces.
                    if bytes.get(i + 1) == Some(&b'|') {
                        trailing_op = Some(TrailingOp::Or);
                        i += 2;
                        continue;
                    }
                    trailing_op = Some(TrailingOp::Pipe);
                }
                b'&' => {
                    if bytes.get(i + 1) == Some(&b'&') {
                        trailing_op = Some(TrailingOp::And);
                        i += 2;
                        continue;
                    }
                    // `&` solo es background — el shell ya lo trata
                    // antes de ExecSpec; no abre nada.
                    trailing_op = None;
                }
                b' ' | b'\t' => {
                    // El whitespace no cancela el trailing_op (queremos
                    // que `cmd |   ` siga siendo pipe pendiente).
                }
                _ => {
                    trailing_op = None;
                }
            }
            i += 1;
        }
        if let Some((tag, strip)) = pending_heredoc {
            heredoc_tag = Some(tag);
            heredoc_strip = strip;
        }
        // `\` al final de la línea (sin terminar) = continuación.
        // `prev_backslash` quedó `true` si el último char era `\` sin
        // procesar; lo metemos como un trailing_op especial.
        if prev_backslash {
            trailing_op = Some(TrailingOp::Backslash);
        }
    }

    single_q
        || double_q
        || depth_paren > 0
        || heredoc_tag.is_some()
        || trailing_op.is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrailingOp {
    Pipe,
    And,
    Or,
    Backslash,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_complete_line_does_not_need_continuation() {
        assert!(!needs_continuation("echo hola"));
        assert!(!needs_continuation(""));
        assert!(!needs_continuation("ls -la"));
    }

    #[test]
    fn unclosed_single_quote_needs_continuation() {
        assert!(needs_continuation("echo 'hola"));
        // Cerrada en línea siguiente — sigue abierta hasta ver `'`.
        assert!(needs_continuation("echo 'hola\nmundo"));
        // Cerrada → no continúa.
        assert!(!needs_continuation("echo 'hola\nmundo'"));
    }

    #[test]
    fn unclosed_double_quote_needs_continuation() {
        assert!(needs_continuation("echo \"hola"));
        assert!(!needs_continuation("echo \"hola\""));
    }

    #[test]
    fn quoted_pipe_does_not_count() {
        // `echo "|"` está completo — el `|` está dentro de quotes.
        assert!(!needs_continuation("echo \"|\""));
        assert!(!needs_continuation("echo '|'"));
    }

    #[test]
    fn unbalanced_paren_needs_continuation() {
        assert!(needs_continuation("echo $(cat foo"));
        assert!(needs_continuation("(echo a"));
        // Balanceadas: ok.
        assert!(!needs_continuation("echo $(cat foo)"));
        assert!(!needs_continuation("(echo a)"));
    }

    #[test]
    fn trailing_pipe_needs_continuation() {
        assert!(needs_continuation("cat foo |"));
        assert!(needs_continuation("cat foo |  "));
        // `||` también, como pipe-y.
        assert!(needs_continuation("cmd ||"));
        assert!(needs_continuation("cmd &&"));
        // Pero un `cmd` sólo no.
        assert!(!needs_continuation("cmd"));
    }

    #[test]
    fn trailing_backslash_needs_continuation() {
        assert!(needs_continuation("cargo build \\"));
        // Múltiples líneas con `\` al final cada una.
        assert!(needs_continuation("a \\\nb \\"));
        // La última línea sin `\` ya está completa.
        assert!(!needs_continuation("a \\\nb"));
    }

    #[test]
    fn heredoc_open_needs_continuation_until_tag_seen() {
        assert!(needs_continuation("cat <<EOF"));
        assert!(needs_continuation("cat <<EOF\ncontenido"));
        assert!(!needs_continuation("cat <<EOF\ncontenido\nEOF"));
        // Tag entre comillas (sin expansión).
        assert!(needs_continuation("cat <<'EOF'\ncontenido"));
        assert!(!needs_continuation("cat <<'EOF'\ncontenido\nEOF"));
    }

    #[test]
    fn heredoc_with_dash_strips_tabs_on_close() {
        assert!(needs_continuation("cat <<-EOF\ncontenido"));
        // El tag de cierre puede llevar tabs adelante con `<<-`.
        assert!(!needs_continuation("cat <<-EOF\ncontenido\n\t\tEOF"));
        // Sin `<<-`, las tabs antes del tag NO cierran.
        assert!(needs_continuation("cat <<EOF\ncontenido\n\tEOF"));
    }

    #[test]
    fn comment_does_not_open_anything() {
        // Un `#` empieza un comentario hasta fin de línea.
        assert!(!needs_continuation("ls # un comentario \"raro' |"));
    }

    #[test]
    fn single_quote_makes_backslash_literal() {
        // Dentro de `'...'` el `\` es literal, así que `'\'` no escapa
        // el `'` siguiente — es comilla cerrada + `'` abre otra. Es
        // exactamente cómo lo hace bash.
        // Test pragmático: una sola apertura sigue abierta.
        assert!(needs_continuation("echo '\\"));
    }
}
