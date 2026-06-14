//! `CellRef` y `CellRange` — direcciones en una hoja.
//!
//! Convención A1: la columna es base-26 sin cero (A..Z, AA..AZ, BA..),
//! la fila es 1-indexada. Por dentro almacenamos ambos como `u32`
//! 0-indexados — la conversión queda localizada en `parse`/`to_string`.
//!
//! Soportamos los cuatro modos de anclaje (`A1`, `$A1`, `A$1`, `$A$1`)
//! porque son lo que el usuario espera al copiar/pegar una fórmula:
//! un `$` ancla esa coordenada al copiar. El motor de evaluación los
//! resuelve igual; el anclaje solo importa al reescribir fórmulas
//! durante un fill/copy.
//!
//! `CellRange` es siempre rectangular `start..=end` con coordenadas ya
//! normalizadas (top-left + bottom-right). `B5:A1` se reescribe a
//! `A1:B5` al parsear — es lo que Excel hace internamente.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// Una referencia de celda. Identidad = `(col, row)` solamente; los
/// flags de anclaje (`col_absolute`, `row_absolute`) son metadata de
/// notación que afectan SOLO al `Display` y al `shift` de fill/copy
/// — no a la resolución en el HashMap del sheet, ni al `Eq`/`Hash`.
/// Esto significa que `A1`, `$A1`, `A$1` y `$A$1` apuntan a la misma
/// celda; sólo cambia cómo se reescribe la fórmula al copiarla.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CellRef {
    pub col: u32,
    pub row: u32,
    #[serde(default)]
    pub col_absolute: bool,
    #[serde(default)]
    pub row_absolute: bool,
}

impl PartialEq for CellRef {
    fn eq(&self, other: &Self) -> bool {
        self.col == other.col && self.row == other.row
    }
}

impl Eq for CellRef {}

impl std::hash::Hash for CellRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.col.hash(state);
        self.row.hash(state);
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CellRefError {
    #[error("empty cell reference")]
    Empty,
    #[error("missing column letters")]
    MissingColumn,
    #[error("missing row number")]
    MissingRow,
    #[error("invalid character `{0}` in cell reference")]
    InvalidChar(char),
    #[error("row out of range (must be >= 1)")]
    RowZero,
    #[error("trailing input after cell reference: `{0}`")]
    Trailing(String),
}

impl CellRef {
    pub const fn new(col: u32, row: u32) -> Self {
        Self {
            col,
            row,
            col_absolute: false,
            row_absolute: false,
        }
    }

    /// Convierte un índice 0-based de columna a las letras A1: `0 → "A"`,
    /// `25 → "Z"`, `26 → "AA"`, `701 → "ZZ"`, `702 → "AAA"`.
    pub fn col_label(mut col: u32) -> String {
        let mut buf = Vec::new();
        // Base-26 desplazado: cada dígito ocupa el rango 1..=26 (no
        // 0..=25), por lo que restamos 1 antes de dividir.
        loop {
            buf.push(b'A' + (col % 26) as u8);
            if col < 26 {
                break;
            }
            col = col / 26 - 1;
        }
        buf.reverse();
        String::from_utf8(buf).unwrap()
    }

    /// Parser del literal `[$]COL[$]ROW`. Devuelve `(CellRef, resto)` —
    /// permite a los callers (parser de fórmulas, parser de rangos)
    /// consumir el prefijo y seguir.
    pub fn parse_prefix(input: &str) -> Result<(Self, &str), CellRefError> {
        if input.is_empty() {
            return Err(CellRefError::Empty);
        }
        let bytes = input.as_bytes();
        let mut i = 0;

        let col_absolute = bytes.get(i) == Some(&b'$');
        if col_absolute {
            i += 1;
        }

        let col_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == col_start {
            return Err(CellRefError::MissingColumn);
        }
        let col_letters = &input[col_start..i];
        let col = decode_col(col_letters)?;

        let row_absolute = bytes.get(i) == Some(&b'$');
        if row_absolute {
            i += 1;
        }

        let row_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == row_start {
            return Err(CellRefError::MissingRow);
        }
        let row_str = &input[row_start..i];
        let row: u32 = row_str.parse().map_err(|_| CellRefError::MissingRow)?;
        if row == 0 {
            return Err(CellRefError::RowZero);
        }

        Ok((
            Self {
                col,
                row: row - 1,
                col_absolute,
                row_absolute,
            },
            &input[i..],
        ))
    }
}

fn decode_col(letters: &str) -> Result<u32, CellRefError> {
    // Base-26 desplazado inverso. Cada letra contribuye `(L - 'A' + 1) *
    // 26^k`, y al final restamos 1 para volver al espacio 0-based.
    let mut total: u32 = 0;
    for c in letters.chars() {
        let upper = c.to_ascii_uppercase();
        if !upper.is_ascii_uppercase() {
            return Err(CellRefError::InvalidChar(c));
        }
        total = total * 26 + (upper as u32 - 'A' as u32 + 1);
    }
    Ok(total - 1)
}

impl FromStr for CellRef {
    type Err = CellRefError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (cr, rest) = Self::parse_prefix(s)?;
        if !rest.is_empty() {
            return Err(CellRefError::Trailing(rest.to_string()));
        }
        Ok(cr)
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.col_absolute {
            f.write_str("$")?;
        }
        f.write_str(&Self::col_label(self.col))?;
        if self.row_absolute {
            f.write_str("$")?;
        }
        write!(f, "{}", self.row + 1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CellRangeError {
    #[error("missing `:` in range")]
    MissingColon,
    #[error("start cell: {0}")]
    Start(CellRefError),
    #[error("end cell: {0}")]
    End(CellRefError),
}

impl CellRange {
    /// Construye un rango normalizado (top-left + bottom-right
    /// garantizados). Útil cuando el caller ya tiene `CellRef`s.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        let (c1, c2) = (a.col.min(b.col), a.col.max(b.col));
        let (r1, r2) = (a.row.min(b.row), a.row.max(b.row));
        Self {
            start: CellRef {
                col: c1,
                row: r1,
                col_absolute: a.col_absolute,
                row_absolute: a.row_absolute,
            },
            end: CellRef {
                col: c2,
                row: r2,
                col_absolute: b.col_absolute,
                row_absolute: b.row_absolute,
            },
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = CellRef> + '_ {
        (self.start.row..=self.end.row).flat_map(move |row| {
            (self.start.col..=self.end.col).map(move |col| CellRef::new(col, row))
        })
    }

    pub fn cell_count(&self) -> usize {
        let cols = (self.end.col - self.start.col + 1) as usize;
        let rows = (self.end.row - self.start.row + 1) as usize;
        cols * rows
    }
}

impl FromStr for CellRange {
    type Err = CellRangeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (colon_idx, _) = s
            .char_indices()
            .find(|(_, c)| *c == ':')
            .ok_or(CellRangeError::MissingColon)?;
        let left = &s[..colon_idx];
        let right = &s[colon_idx + 1..];
        let a = CellRef::from_str(left).map_err(CellRangeError::Start)?;
        let b = CellRef::from_str(right).map_err(CellRangeError::End)?;
        Ok(Self::new(a, b))
    }
}

impl fmt::Display for CellRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn col_label_roundtrip_through_alphabet() {
        for col in 0..=701u32 {
            let label = CellRef::col_label(col);
            assert_eq!(decode_col(&label).unwrap(), col, "label = {}", label);
        }
        assert_eq!(CellRef::col_label(0), "A");
        assert_eq!(CellRef::col_label(25), "Z");
        assert_eq!(CellRef::col_label(26), "AA");
        assert_eq!(CellRef::col_label(701), "ZZ");
        assert_eq!(CellRef::col_label(702), "AAA");
    }

    #[test]
    fn parses_plain_relative() {
        let cr: CellRef = "B5".parse().unwrap();
        assert_eq!(cr, CellRef::new(1, 4));
        assert!(!cr.col_absolute);
        assert!(!cr.row_absolute);
    }

    #[test]
    fn parses_all_four_anchor_modes() {
        let cases = [
            ("A1", false, false),
            ("$A1", true, false),
            ("A$1", false, true),
            ("$A$1", true, true),
        ];
        for (input, ca, ra) in cases {
            let cr: CellRef = input.parse().unwrap();
            assert_eq!(cr.col, 0);
            assert_eq!(cr.row, 0);
            assert_eq!(cr.col_absolute, ca, "input={}", input);
            assert_eq!(cr.row_absolute, ra, "input={}", input);
            assert_eq!(cr.to_string(), input);
        }
    }

    #[test]
    fn lowercase_letters_normalize_to_uppercase() {
        let cr: CellRef = "ab10".parse().unwrap();
        assert_eq!(cr.to_string(), "AB10");
    }

    #[test]
    fn row_zero_rejected() {
        assert_eq!("A0".parse::<CellRef>(), Err(CellRefError::RowZero));
    }

    #[test]
    fn missing_pieces_rejected() {
        assert_eq!("5".parse::<CellRef>(), Err(CellRefError::MissingColumn));
        assert_eq!("A".parse::<CellRef>(), Err(CellRefError::MissingRow));
    }

    #[test]
    fn trailing_garbage_rejected() {
        assert!(matches!(
            "A1+B2".parse::<CellRef>(),
            Err(CellRefError::Trailing(_))
        ));
    }

    #[test]
    fn parse_prefix_returns_remaining_input() {
        let (cr, rest) = CellRef::parse_prefix("AB12:CD34").unwrap();
        assert_eq!(cr, CellRef::new(27, 11));
        assert_eq!(rest, ":CD34");
    }

    #[test]
    fn range_normalizes_to_top_left_first() {
        // El usuario escribe B5:A1, lo guardamos como A1:B5.
        let r: CellRange = "B5:A1".parse().unwrap();
        assert_eq!(r.start, CellRef::new(0, 0));
        assert_eq!(r.end, CellRef::new(1, 4));
    }

    #[test]
    fn range_iter_walks_row_major() {
        let r: CellRange = "A1:B2".parse().unwrap();
        let cells: Vec<_> = r.iter().map(|c| c.to_string()).collect();
        assert_eq!(cells, vec!["A1", "B1", "A2", "B2"]);
    }

    #[test]
    fn range_cell_count_matches_iteration() {
        let r: CellRange = "A1:C10".parse().unwrap();
        assert_eq!(r.cell_count(), 30);
        assert_eq!(r.iter().count(), 30);
    }
}
