//! `SheetValue` — el valor canónico de una celda evaluada.
//!
//! Excel/Sheets mete números, texto, booleanos y errores en el mismo
//! enum dinámico; replicamos esa forma porque las fórmulas naturalmente
//! cruzan tipos (`IF(A1>0, "ok", 42)` es válido). La diferencia clave es
//! que los números viven como `rust_decimal::Decimal` — 96 bits de
//! mantissa + escala explícita — y no como `f64`. Eso elimina los
//! errores de redondeo que hacen que `0.1 + 0.2 != 0.3` en hojas
//! financieras.
//!
//! Los errores son valores de primera clase (`#DIV/0!`, `#REF!`...): se
//! propagan por las fórmulas sin abortar la evaluación. Esto es lo que
//! permite que una hoja con un error en `B5` siga renderizando el resto
//! sin caerse.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SheetError {
    #[error("#DIV/0!")]
    DivZero,
    #[error("#VALUE!")]
    Value,
    #[error("#REF!")]
    Ref,
    #[error("#NAME?")]
    Name,
    #[error("#N/A")]
    NotApplicable,
    #[error("#NUM!")]
    Num,
    #[error("#CYCLE!")]
    Cycle,
    #[error("#PARSE!")]
    Parse,
}

impl SheetError {
    /// Token corto que se muestra en la celda (lo que Excel pinta).
    pub fn token(&self) -> &'static str {
        match self {
            Self::DivZero => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::NotApplicable => "#N/A",
            Self::Num => "#NUM!",
            Self::Cycle => "#CYCLE!",
            Self::Parse => "#PARSE!",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum SheetValue {
    /// Celda sin contenido. Semánticamente distinto de `Number(0)` y de
    /// `Text("")`: las funciones agregadas la ignoran (`SUM` la salta),
    /// mientras que `0` cuenta y `""` rompe un `SUM` con `#VALUE!`.
    Empty,
    Number(Decimal),
    Text(String),
    Bool(bool),
    Error(SheetError),
}

impl SheetValue {
    pub fn from_int(n: i64) -> Self {
        Self::Number(Decimal::from(n))
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    /// Coerción numérica al estilo Excel: `Empty` → `0`, `Bool(true)` →
    /// `1`, `Bool(false)` → `0`, `Text` parseable → su número, errores
    /// se propagan. Devuelve `Err(SheetError)` cuando la coerción es
    /// imposible — el caller decide si ese error mata la fórmula o se
    /// envuelve en un `SheetValue::Error`.
    pub fn to_number(&self) -> Result<Decimal, SheetError> {
        match self {
            Self::Number(d) => Ok(*d),
            Self::Empty => Ok(Decimal::ZERO),
            Self::Bool(true) => Ok(Decimal::ONE),
            Self::Bool(false) => Ok(Decimal::ZERO),
            Self::Text(s) => s.parse::<Decimal>().map_err(|_| SheetError::Value),
            Self::Error(e) => Err(e.clone()),
        }
    }

    /// Coerción booleana al estilo Excel: número no-cero → `true`,
    /// `0` → `false`, `Empty` → `false`. El texto NO coerce a bool en
    /// Excel — devuelve `#VALUE!`.
    pub fn to_bool(&self) -> Result<bool, SheetError> {
        match self {
            Self::Bool(b) => Ok(*b),
            Self::Number(d) => Ok(!d.is_zero()),
            Self::Empty => Ok(false),
            Self::Text(_) => Err(SheetError::Value),
            Self::Error(e) => Err(e.clone()),
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Number(d) => d.normalize().to_string(),
            Self::Text(s) => s.clone(),
            Self::Bool(true) => "TRUE".into(),
            Self::Bool(false) => "FALSE".into(),
            Self::Error(e) => e.token().to_string(),
        }
    }

    /// Como `to_display_string`, pero respeta un [`CellFormat`]. Texto
    /// y booleanos ignoran el format (no son numéricos); empty,
    /// number y error sí responden — number aplica el formato
    /// numérico, empty queda vacío, error muestra su token.
    pub fn to_formatted_string(&self, fmt: &CellFormat) -> String {
        match self {
            Self::Number(d) => fmt.format_number(*d),
            _ => self.to_display_string(),
        }
    }
}

/// Formato de display de una celda. Es metadata visual — no cambia
/// el valor almacenado, solo cómo se pinta. `General` (default)
/// usa el `to_display_string` natural; los otros son opt-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellFormat {
    /// Sin formato — muestra el value tal cual.
    General,
    /// Número con un número fijo de decimales y separador de miles
    /// (`1,234.50`).
    Number { decimals: u8 },
    /// Moneda con prefijo (`$1,234.50` o `€1.234,50` — el símbolo
    /// queda al gusto del usuario). Sin convertir entre monedas;
    /// es solo cosmético.
    Currency { symbol: String, decimals: u8 },
    /// Porcentaje: multiplica el valor por 100 al display
    /// (`0.5` → `50.00%`).
    Percent { decimals: u8 },
}

impl Default for CellFormat {
    fn default() -> Self {
        Self::General
    }
}

impl CellFormat {
    pub fn format_number(&self, n: rust_decimal::Decimal) -> String {
        match self {
            Self::General => n.normalize().to_string(),
            Self::Number { decimals } => format_with_separators(n, *decimals, ""),
            Self::Currency { symbol, decimals } => {
                let body = format_with_separators(n, *decimals, "");
                if n.is_sign_negative() {
                    // Estilo Excel: el símbolo va después del menos.
                    // "−$1,234.50" en vez de "$−1,234.50".
                    let abs = body.trim_start_matches('-');
                    format!("-{symbol}{abs}")
                } else {
                    format!("{symbol}{body}")
                }
            }
            Self::Percent { decimals } => {
                let scaled = n * rust_decimal::Decimal::from(100);
                format_with_separators(scaled, *decimals, "") + "%"
            }
        }
    }
}

/// Formatea un `Decimal` con N decimales fijos y separador de miles
/// (`,`). Sin localización (que es scope creep — primero hacemos
/// que funcione, luego internacionalizamos).
fn format_with_separators(
    n: rust_decimal::Decimal,
    decimals: u8,
    _locale: &str,
) -> String {
    let rounded = n.round_dp(decimals as u32);
    let s = format!("{rounded:.*}", decimals as usize);
    // Insertar separadores de miles en la parte entera.
    let (sign, body) = if let Some(stripped) = s.strip_prefix('-') {
        ("-", stripped)
    } else {
        ("", s.as_str())
    };
    let (int_part, frac_part) = match body.find('.') {
        Some(idx) => (&body[..idx], &body[idx..]),
        None => (body, ""),
    };
    let mut out = String::new();
    out.push_str(sign);
    let bytes = int_part.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out.push_str(frac_part);
    out
}

impl From<Decimal> for SheetValue {
    fn from(d: Decimal) -> Self {
        Self::Number(d)
    }
}

impl From<i64> for SheetValue {
    fn from(n: i64) -> Self {
        Self::Number(Decimal::from(n))
    }
}

impl From<bool> for SheetValue {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}

impl From<String> for SheetValue {
    fn from(s: String) -> Self {
        Self::Text(s)
    }
}

impl From<&str> for SheetValue {
    fn from(s: &str) -> Self {
        Self::Text(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn exact_decimal_no_float_drift() {
        let a = SheetValue::Number(Decimal::from_str("0.1").unwrap());
        let b = SheetValue::Number(Decimal::from_str("0.2").unwrap());
        let sum = a.to_number().unwrap() + b.to_number().unwrap();
        assert_eq!(sum, Decimal::from_str("0.3").unwrap());
    }

    #[test]
    fn empty_coerces_to_zero_in_arithmetic() {
        assert_eq!(SheetValue::Empty.to_number().unwrap(), Decimal::ZERO);
    }

    #[test]
    fn bool_coerces_numerically() {
        assert_eq!(SheetValue::Bool(true).to_number().unwrap(), Decimal::ONE);
        assert_eq!(SheetValue::Bool(false).to_number().unwrap(), Decimal::ZERO);
    }

    #[test]
    fn text_parseable_coerces_to_number() {
        assert_eq!(
            SheetValue::Text("42.5".into()).to_number().unwrap(),
            Decimal::from_str("42.5").unwrap()
        );
    }

    #[test]
    fn text_unparseable_yields_value_error() {
        assert_eq!(
            SheetValue::Text("hola".into()).to_number(),
            Err(SheetError::Value)
        );
    }

    #[test]
    fn errors_propagate_through_coercion() {
        let v = SheetValue::Error(SheetError::DivZero);
        assert_eq!(v.to_number(), Err(SheetError::DivZero));
        assert_eq!(v.to_bool(), Err(SheetError::DivZero));
    }

    #[test]
    fn text_does_not_coerce_to_bool() {
        assert_eq!(
            SheetValue::Text("true".into()).to_bool(),
            Err(SheetError::Value)
        );
    }

    #[test]
    fn error_tokens_match_excel_conventions() {
        assert_eq!(SheetError::DivZero.token(), "#DIV/0!");
        assert_eq!(SheetError::Ref.token(), "#REF!");
        assert_eq!(SheetError::NotApplicable.token(), "#N/A");
    }

    #[test]
    fn display_strings_strip_decimal_trailing_zeros() {
        // `normalize` elimina ceros sobrantes: 1.50 → 1.5, 5.00 → 5.
        let v = SheetValue::Number(Decimal::from_str("1.50").unwrap());
        assert_eq!(v.to_display_string(), "1.5");
    }

    #[test]
    fn number_format_with_decimals_and_separators() {
        let v = SheetValue::Number(Decimal::from_str("1234567.5").unwrap());
        let fmt = CellFormat::Number { decimals: 2 };
        assert_eq!(v.to_formatted_string(&fmt), "1,234,567.50");
    }

    #[test]
    fn number_format_rounds() {
        let v = SheetValue::Number(Decimal::from_str("3.456").unwrap());
        let fmt = CellFormat::Number { decimals: 2 };
        // banker's rounding de Decimal: 3.456 → 3.46
        assert_eq!(v.to_formatted_string(&fmt), "3.46");
    }

    #[test]
    fn currency_format_uses_symbol_and_sign() {
        let v = SheetValue::Number(Decimal::from_str("1234.5").unwrap());
        let fmt = CellFormat::Currency {
            symbol: "$".into(),
            decimals: 2,
        };
        assert_eq!(v.to_formatted_string(&fmt), "$1,234.50");
        let neg = SheetValue::Number(Decimal::from_str("-99").unwrap());
        assert_eq!(neg.to_formatted_string(&fmt), "-$99.00");
    }

    #[test]
    fn percent_format_multiplies_by_hundred() {
        let v = SheetValue::Number(Decimal::from_str("0.5").unwrap());
        let fmt = CellFormat::Percent { decimals: 1 };
        assert_eq!(v.to_formatted_string(&fmt), "50.0%");
    }

    #[test]
    fn general_format_uses_natural_display() {
        let v = SheetValue::Number(Decimal::from_str("1.50").unwrap());
        assert_eq!(v.to_formatted_string(&CellFormat::General), "1.5");
    }

    #[test]
    fn non_numeric_values_ignore_format() {
        let v = SheetValue::Text("hola".into());
        let fmt = CellFormat::Currency {
            symbol: "$".into(),
            decimals: 2,
        };
        assert_eq!(v.to_formatted_string(&fmt), "hola");
    }
}
