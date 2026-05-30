//! `nahual-table-viewer-llimphi` — visor de CSV/TSV.
//!
//! Octavo visor del shell meta-app. `shuma-discern` marca `.csv`/`.tsv`
//! con lens `table` (por el hint de path + presencia del delimitador);
//! hasta ahora caían al text viewer, que muestra las filas crudas sin
//! alinear. Este visor parsea la tabla (comillas básicas estilo CSV) y
//! la pinta **alineada por columnas** en fuente monoespaciada — un
//! preview rápido para ver la forma de los datos.
//!
//! NO es el editor de planillas (`nakui`): es de sólo-lectura, capado en
//! filas/columnas, pensado para "echarle un ojo" desde el shell. Patrón
//! fino de los otros viewers: carga sync en [`load_table`], render en
//! [`table_viewer_view`].

#![forbid(unsafe_code)]

use std::path::Path;

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Tope de bytes a leer (2 MiB). Un preview no necesita el archivo entero.
pub const DEFAULT_TABLE_BYTES_MAX: u64 = 2 * 1024 * 1024;

/// Límites del render: filas y columnas mostradas, y ancho máximo de
/// celda (chars). Cortan tablas enormes para no atragantar el layout.
const MAX_ROWS: usize = 200;
const MAX_COLS: usize = 32;
const MAX_CELL_W: usize = 32;

/// Estado del visor.
#[derive(Debug, Clone)]
pub enum TablePreview {
    /// Sin archivo seleccionado.
    Empty,
    /// Tabla renderizada + metadatos de tamaño real (para el header).
    Table { text: String, rows: usize, cols: usize },
    /// Excede el tope de tamaño.
    TooBig(u64),
    /// E/S falló.
    Error(String),
}

impl Default for TablePreview {
    fn default() -> Self {
        TablePreview::Empty
    }
}

/// Lee y renderiza el archivo. El delimitador se elige por extensión:
/// `.tsv` → tab, cualquier otra → coma.
pub fn load_table(path: &Path, max_bytes: u64) -> TablePreview {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => return TablePreview::TooBig(meta.len()),
        Err(e) => return TablePreview::Error(e.to_string()),
        _ => {}
    }
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return TablePreview::Error(e.to_string()),
    };
    let delim = if path.extension().and_then(|s| s.to_str()) == Some("tsv") {
        '\t'
    } else {
        ','
    };
    render(&src, delim)
}

/// Parsea `src` y arma la tabla alineada. Cuenta filas/columnas reales
/// (no las capadas) para el header.
fn render(src: &str, delim: char) -> TablePreview {
    let all_rows: Vec<Vec<String>> = src
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| parse_row(line, delim))
        .collect();
    if all_rows.is_empty() {
        return TablePreview::Table {
            text: "(tabla vacía)".to_string(),
            rows: 0,
            cols: 0,
        };
    }
    let total_rows = all_rows.len();
    let total_cols = all_rows.iter().map(Vec::len).max().unwrap_or(0);

    // Vista capada.
    let rows: Vec<&Vec<String>> = all_rows.iter().take(MAX_ROWS).collect();
    let cols = total_cols.min(MAX_COLS);

    // Ancho por columna = máx celda (capado), sobre las filas mostradas.
    let mut widths = vec![0usize; cols];
    for row in &rows {
        for (c, width) in widths.iter_mut().enumerate() {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            *width = (*width).max(cell.chars().count().min(MAX_CELL_W));
        }
    }

    let mut out = String::new();
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            out.push('\n');
        }
        for c in 0..cols {
            if c > 0 {
                out.push_str(" │ ");
            }
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            out.push_str(&pad(cell, widths[c]));
        }
        // Separador bajo la cabecera.
        if r == 0 {
            out.push('\n');
            for c in 0..cols {
                if c > 0 {
                    out.push_str("─┼─");
                }
                out.push_str(&"─".repeat(widths[c]));
            }
        }
    }
    if total_rows > rows.len() || total_cols > cols {
        out.push_str(&format!(
            "\n… ({total_rows} filas × {total_cols} cols; mostradas {}×{})",
            rows.len(),
            cols
        ));
    }

    TablePreview::Table {
        text: out,
        rows: total_rows,
        cols: total_cols,
    }
}

/// Trunca/rellena `cell` al ancho `w` (en chars). Trunca con `…`.
fn pad(cell: &str, w: usize) -> String {
    let n = cell.chars().count();
    if n > w {
        let head: String = cell.chars().take(w.saturating_sub(1)).collect();
        format!("{head}…")
    } else {
        let mut s = cell.to_string();
        s.extend(std::iter::repeat(' ').take(w - n));
        s
    }
}

/// Parsea una línea CSV/TSV con comillas dobles básicas: un campo entre
/// `"` puede contener el delimitador y `""` como comilla escapada.
fn parse_row(line: &str, delim: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                cur.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == delim {
            fields.push(std::mem::take(&mut cur).trim().to_string());
        } else {
            cur.push(ch);
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

/// Paleta del viewer.
#[derive(Debug, Clone, Copy)]
pub struct TableViewerPalette {
    pub bg: Color,
    pub fg_text: Color,
    pub fg_muted: Color,
    pub fg_error: Color,
}

impl Default for TableViewerPalette {
    fn default() -> Self {
        Self::from_theme(&llimphi_theme::Theme::dark())
    }
}

impl TableViewerPalette {
    pub fn from_theme(t: &llimphi_theme::Theme) -> Self {
        Self {
            bg: t.bg_app,
            fg_text: t.fg_text,
            fg_muted: t.fg_muted,
            fg_error: t.fg_destructive,
        }
    }
}

/// Pinta header (nombre · filas×cols) + body con la tabla monoespaciada.
pub fn table_viewer_view<Msg>(
    state: &TablePreview,
    path: Option<&Path>,
    palette: &TableViewerPalette,
) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let name = match path {
        Some(p) => p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| p.display().to_string()),
        None => "(seleccioná un CSV/TSV)".to_string(),
    };
    let header_text = match state {
        TablePreview::Table { rows, cols, .. } => {
            format!("table · {name} · {rows} × {cols}")
        }
        _ => format!("table · {name}"),
    };

    let header = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(header_text, 10.0, palette.fg_muted, Alignment::Start);

    let (body_text, body_color) = match state {
        TablePreview::Empty => ("—".to_string(), palette.fg_muted),
        TablePreview::Table { text, .. } => (text.clone(), palette.fg_text),
        TablePreview::TooBig(n) => (
            format!("(tabla muy grande: {n} bytes — sin preview)"),
            palette.fg_muted,
        ),
        TablePreview::Error(e) => (format!("(error: {e})"), palette.fg_error),
    };

    let body = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(6.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .text_aligned_full(
        body_text,
        12.0,
        body_color,
        Alignment::Start,
        false,
        Some("monospace".to_string()),
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(6.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .clip(true)
    .children(vec![header, body])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_campos_simples() {
        assert_eq!(parse_row("a,b,c", ','), vec!["a", "b", "c"]);
    }

    #[test]
    fn respeta_comillas_con_delimitador() {
        assert_eq!(
            parse_row(r#"x,"a,b",z"#, ','),
            vec!["x", "a,b", "z"]
        );
    }

    #[test]
    fn comilla_escapada() {
        assert_eq!(parse_row(r#""di ""hola""",y"#, ','), vec![r#"di "hola""#, "y"]);
    }

    #[test]
    fn render_alinea_y_cuenta() {
        let csv = "fecha,monto\n2026-01,10\n2026-02,200\n";
        match render(csv, ',') {
            TablePreview::Table { text, rows, cols } => {
                assert_eq!(rows, 3);
                assert_eq!(cols, 2);
                // Header + separador + filas.
                assert!(text.contains("fecha"));
                assert!(text.contains("─┼─"));
                assert!(text.contains(" │ "));
            }
            other => panic!("esperaba Table, obtuve {other:?}"),
        }
    }

    #[test]
    fn celda_larga_se_trunca() {
        let long = "z".repeat(MAX_CELL_W + 10);
        let p = pad(&long, MAX_CELL_W);
        assert!(p.ends_with('…'));
        assert_eq!(p.chars().count(), MAX_CELL_W);
    }
}
