//! Utilidades de texto compartidas por los parsers iCalendar y vCard: ambos son
//! formatos de líneas `NAME[;PARAMS]:VALUE` con plegado de líneas largas y el
//! mismo esquema de escape (`\` `;` `,` newline).

/// Desdobla líneas plegadas (continuación: empieza con espacio o tab) y normaliza
/// CRLF/CR/LF a líneas lógicas, descartando vacías.
pub fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&raw[1..]);
        } else if !raw.is_empty() {
            out.push(raw.to_string());
        }
    }
    out
}

/// Parte `NAME[;PARAMS]:VALUE` en `(NAME_mayúsc, params, value)`.
pub fn split_line(line: &str) -> Option<(String, String, String)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let (name, params) = match head.find(';') {
        Some(sc) => (&head[..sc], &head[sc + 1..]),
        None => (head, ""),
    };
    Some((name.to_ascii_uppercase(), params.to_string(), value.to_string()))
}

/// Escapa texto (`\` `;` `,` y newline) para iCalendar/vCard.
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace(';', "\\;").replace(',', "\\,")
}

/// Desescapa texto iCalendar/vCard.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(';') => out.push(';'),
                Some(',') => out.push(','),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfold_junta_continuaciones() {
        let v = unfold("A:uno\r\n dos\r\nB:tres\r\n");
        assert_eq!(v, vec!["A:unodos", "B:tres"]);
    }

    #[test]
    fn split_separa_params() {
        assert_eq!(
            split_line("EMAIL;TYPE=work:a@x.com"),
            Some(("EMAIL".into(), "TYPE=work".into(), "a@x.com".into()))
        );
    }

    #[test]
    fn escape_simetrico() {
        let s = "a, b; c\\ d\nfin";
        assert_eq!(unescape(&escape(s)), s);
    }
}
