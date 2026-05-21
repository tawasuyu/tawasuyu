//! Serialización de filas a CSV (RFC 4180) para exportar listas.

/// Arma un documento CSV: una línea de headers + una por fila. Cada
/// celda se escapa si contiene coma, comilla o salto de línea.
pub fn to_csv(headers: &[String], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    push_csv_line(&mut out, headers);
    for row in rows {
        push_csv_line(&mut out, row);
    }
    out
}

/// Agrega una línea CSV (celdas separadas por coma + `\n` final).
fn push_csv_line(out: &mut String, cells: &[String]) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&csv_escape(cell));
    }
    out.push('\n');
}

/// Escapa una celda: la envuelve en comillas y duplica las comillas
/// internas si contiene coma, comilla, CR o LF. Si no, va tal cual.
fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cells_unquoted() {
        let csv = to_csv(
            &["Nombre".into(), "Edad".into()],
            &[vec!["Ana".into(), "30".into()]],
        );
        assert_eq!(csv, "Nombre,Edad\nAna,30\n");
    }

    #[test]
    fn cells_with_comma_or_quote_are_escaped() {
        let csv = to_csv(
            &["a".into(), "b".into()],
            &[vec!["x,y".into(), "dijo \"hola\"".into()]],
        );
        assert_eq!(csv, "a,b\n\"x,y\",\"dijo \"\"hola\"\"\"\n");
    }

    #[test]
    fn newline_in_cell_is_quoted() {
        let csv = to_csv(&["n".into()], &[vec!["línea1\nlínea2".into()]]);
        assert_eq!(csv, "n\n\"línea1\nlínea2\"\n");
    }

    #[test]
    fn empty_rows_yields_just_header() {
        assert_eq!(to_csv(&["x".into()], &[]), "x\n");
    }
}
