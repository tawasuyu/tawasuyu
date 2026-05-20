//! `Decimal` — un número decimal de punto fijo, exacto.
//!
//! COBOL no calcula en binario flotante: sus campos numéricos son
//! decimales de precisión fija. Un `Decimal` se guarda como una mantisa
//! entera (`i128`) y una escala (cuántos de sus dígitos son
//! fraccionarios) — `valor = mantisa / 10^escala`. La aritmética es
//! exacta; la pérdida de precisión sólo ocurre, explícita, al ajustar a
//! la escala de un campo receptor.
//!
//! Dominio: hasta 38 dígitos significativos (el rango de `i128`).

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::picture::Picture;
use crate::BcdError;

/// Modo de redondeo al perder dígitos fraccionarios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rounding {
    /// Descartar los dígitos sobrantes — el comportamiento por defecto
    /// de COBOL.
    Truncate,
    /// Redondear al más cercano, la mitad alejándose de cero — la
    /// opción `ROUNDED` de COBOL.
    HalfUp,
}

/// `10^n` como `i128`. Pánico si `n` excede el rango (n > 38).
fn pow10(n: u32) -> i128 {
    10i128.pow(n)
}

/// Cuántos dígitos decimales tiene `n` (`0` tiene cero dígitos).
fn digit_count(mut n: u128) -> u32 {
    let mut d = 0;
    while n > 0 {
        n /= 10;
        d += 1;
    }
    d
}

/// Un decimal de punto fijo exacto: `mantissa / 10^scale`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Decimal {
    mantissa: i128,
    scale: u8,
}

impl Decimal {
    /// Construye desde mantisa y escala crudas.
    pub fn new(mantissa: i128, scale: u8) -> Self {
        Self { mantissa, scale }
    }

    /// El cero (escala 0).
    pub fn zero() -> Self {
        Self { mantissa: 0, scale: 0 }
    }

    /// Un entero como decimal de escala 0.
    pub fn from_integer(value: i128) -> Self {
        Self { mantissa: value, scale: 0 }
    }

    /// Mantisa cruda.
    pub fn mantissa(&self) -> i128 {
        self.mantissa
    }

    /// Escala — cantidad de dígitos fraccionarios.
    pub fn scale(&self) -> u8 {
        self.scale
    }

    pub fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    pub fn is_negative(&self) -> bool {
        self.mantissa < 0
    }

    /// Parsea un literal numérico — `"123.45"`, `"-7"`, `"+0.001"`, `".5"`.
    pub fn parse(src: &str) -> Result<Decimal, BcdError> {
        let t = src.trim();
        let bad = || BcdError::BadNumber(src.to_string());
        let (neg, rest) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        if !rest.bytes().any(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let (int_str, frac_str) = rest.split_once('.').unwrap_or((rest, ""));
        let int_str = if int_str.is_empty() { "0" } else { int_str };
        if !int_str.bytes().all(|b| b.is_ascii_digit())
            || !frac_str.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(bad());
        }
        let mut mantissa: i128 =
            format!("{int_str}{frac_str}").parse().map_err(|_| bad())?;
        if neg {
            mantissa = -mantissa;
        }
        Ok(Decimal { mantissa, scale: frac_str.len() as u8 })
    }

    /// Devuelve el mismo valor expresado en `target_scale`. Subir de
    /// escala es exacto; bajar pierde dígitos según `rounding`.
    pub fn rescale(&self, target_scale: u8, rounding: Rounding) -> Decimal {
        match target_scale.cmp(&self.scale) {
            Ordering::Equal => *self,
            Ordering::Greater => {
                let factor = pow10((target_scale - self.scale) as u32);
                Decimal { mantissa: self.mantissa * factor, scale: target_scale }
            }
            Ordering::Less => {
                let divisor = pow10((self.scale - target_scale) as u32);
                let q = self.mantissa / divisor;
                let r = self.mantissa % divisor;
                let m = match rounding {
                    Rounding::Truncate => q,
                    Rounding::HalfUp => {
                        if r.unsigned_abs() * 2 >= divisor.unsigned_abs() {
                            q + self.mantissa.signum()
                        } else {
                            q
                        }
                    }
                };
                Decimal { mantissa: m, scale: target_scale }
            }
        }
    }

    /// Lleva dos decimales a una escala común (la mayor, sin pérdida).
    fn aligned(&self, other: &Decimal) -> (i128, i128, u8) {
        let s = self.scale.max(other.scale);
        (
            self.rescale(s, Rounding::Truncate).mantissa,
            other.rescale(s, Rounding::Truncate).mantissa,
            s,
        )
    }

    /// Suma exacta.
    pub fn add(&self, other: &Decimal) -> Decimal {
        let (a, b, s) = self.aligned(other);
        Decimal { mantissa: a + b, scale: s }
    }

    /// Resta exacta.
    pub fn sub(&self, other: &Decimal) -> Decimal {
        let (a, b, s) = self.aligned(other);
        Decimal { mantissa: a - b, scale: s }
    }

    /// Producto exacto — la escala del resultado es la suma de escalas.
    pub fn mul(&self, other: &Decimal) -> Decimal {
        Decimal {
            mantissa: self.mantissa * other.mantissa,
            scale: self.scale.saturating_add(other.scale),
        }
    }

    /// División con la escala del resultado fijada de antemano (como en
    /// COBOL, donde el campo receptor define la precisión). Error si el
    /// divisor es cero o si un producto intermedio se sale de `i128`.
    pub fn div(
        &self,
        other: &Decimal,
        result_scale: u8,
        rounding: Rounding,
    ) -> Result<Decimal, BcdError> {
        if other.mantissa == 0 {
            return Err(BcdError::DivByZero);
        }
        let num_pow = other.scale as u32 + result_scale as u32;
        let numerator = self
            .mantissa
            .checked_mul(10i128.checked_pow(num_pow).ok_or(BcdError::Overflow)?)
            .ok_or(BcdError::Overflow)?;
        let denominator = other
            .mantissa
            .checked_mul(10i128.checked_pow(self.scale as u32).ok_or(BcdError::Overflow)?)
            .ok_or(BcdError::Overflow)?;
        let q = numerator / denominator;
        let r = numerator % denominator;
        let m = match rounding {
            Rounding::Truncate => q,
            Rounding::HalfUp => {
                if r.unsigned_abs() * 2 >= denominator.unsigned_abs() {
                    q + numerator.signum() * denominator.signum()
                } else {
                    q
                }
            }
        };
        Ok(Decimal { mantissa: m, scale: result_scale })
    }

    /// `true` si el valor entra en el campo `pic` sin perder dígitos
    /// enteros (la parte fraccionaria se ajusta, no desborda).
    pub fn fits(&self, pic: &Picture) -> bool {
        let r = self.rescale(pic.fraction_digits, Rounding::Truncate);
        let int_part = r.mantissa.unsigned_abs() / 10u128.pow(pic.fraction_digits as u32);
        digit_count(int_part) <= pic.integer_digits as u32
    }

    /// Almacena el valor en un campo `pic` — la operación `MOVE` de
    /// COBOL. Ajusta la escala con `rounding`; un campo sin signo guarda
    /// la magnitud; si la parte entera no cabe devuelve [`BcdError::Overflow`]
    /// (el `ON SIZE ERROR` de COBOL).
    pub fn coerce(&self, pic: &Picture, rounding: Rounding) -> Result<Decimal, BcdError> {
        let mut r = self.rescale(pic.fraction_digits, rounding);
        if !pic.signed && r.mantissa < 0 {
            r.mantissa = -r.mantissa;
        }
        let int_part = r.mantissa.unsigned_abs() / 10u128.pow(pic.fraction_digits as u32);
        if digit_count(int_part) > pic.integer_digits as u32 {
            return Err(BcdError::Overflow);
        }
        Ok(r)
    }
}

impl PartialEq for Decimal {
    /// Dos decimales son iguales si representan el mismo valor — `1.0`
    /// es igual a `1.00`.
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, b, _) = self.aligned(other);
        a.cmp(&b)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mantissa < 0 {
            write!(f, "-")?;
        }
        let abs = self.mantissa.unsigned_abs();
        if self.scale == 0 {
            return write!(f, "{abs}");
        }
        let s = self.scale as usize;
        let mut digits = abs.to_string();
        if digits.len() <= s {
            // Rellena para que haya al menos un dígito entero (`0.xx`).
            digits = format!("{}{}", "0".repeat(s - digits.len() + 1), digits);
        }
        let (int_part, frac_part) = digits.split_at(digits.len() - s);
        write!(f, "{int_part}.{frac_part}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse(s).unwrap()
    }

    #[test]
    fn parse_and_display_roundtrip() {
        assert_eq!(d("123.45").to_string(), "123.45");
        assert_eq!(d("-7").to_string(), "-7");
        assert_eq!(d("0.001").to_string(), "0.001");
        assert_eq!(d(".5").to_string(), "0.5");
        assert_eq!(d("+42").to_string(), "42");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Decimal::parse("abc").is_err());
        assert!(Decimal::parse("").is_err());
        assert!(Decimal::parse("1.2.3").is_err());
        assert!(Decimal::parse("-").is_err());
    }

    #[test]
    fn addition_aligns_scales() {
        // 1.5 + 2.25 = 3.75
        assert_eq!(d("1.5").add(&d("2.25")), d("3.75"));
        // 100 + 0.001 = 100.001
        assert_eq!(d("100").add(&d("0.001")), d("100.001"));
    }

    #[test]
    fn subtraction_is_exact() {
        assert_eq!(d("10.00").sub(&d("3.33")), d("6.67"));
    }

    #[test]
    fn multiplication_sums_scales() {
        // 1.5 * 1.5 = 2.25
        let p = d("1.5").mul(&d("1.5"));
        assert_eq!(p, d("2.25"));
        assert_eq!(p.scale(), 2);
    }

    #[test]
    fn equality_ignores_trailing_zeros() {
        assert_eq!(d("1.0"), d("1.00"));
        assert_eq!(d("2"), d("2.000"));
    }

    #[test]
    fn ordering_compares_values() {
        assert!(d("1.9") < d("1.91"));
        assert!(d("-5") < d("0.0"));
        assert!(d("100.0") > d("99.99"));
    }

    #[test]
    fn rescale_truncates_toward_zero() {
        assert_eq!(d("1.999").rescale(2, Rounding::Truncate), d("1.99"));
        assert_eq!(d("-1.999").rescale(2, Rounding::Truncate), d("-1.99"));
    }

    #[test]
    fn rescale_half_up_rounds_away_from_zero() {
        assert_eq!(d("1.995").rescale(2, Rounding::HalfUp), d("2.00"));
        assert_eq!(d("-1.995").rescale(2, Rounding::HalfUp), d("-2.00"));
        assert_eq!(d("1.994").rescale(2, Rounding::HalfUp), d("1.99"));
    }

    #[test]
    fn division_respects_result_scale() {
        // 10 / 3 a 4 decimales, truncado.
        let q = d("10").div(&d("3"), 4, Rounding::Truncate).unwrap();
        assert_eq!(q, d("3.3333"));
        // Redondeado.
        let r = d("10").div(&d("3"), 4, Rounding::HalfUp).unwrap();
        assert_eq!(r, d("3.3333"));
        // 7 / 8 = 0.875 exacto.
        assert_eq!(d("7").div(&d("8"), 3, Rounding::Truncate).unwrap(), d("0.875"));
    }

    #[test]
    fn division_by_zero_errors() {
        assert_eq!(d("1").div(&d("0"), 2, Rounding::Truncate), Err(BcdError::DivByZero));
    }

    #[test]
    fn coerce_into_picture_adjusts_scale() {
        let pic = Picture::parse("9(5)V99").unwrap();
        let stored = d("12.5").coerce(&pic, Rounding::Truncate).unwrap();
        assert_eq!(stored.scale(), 2);
        assert_eq!(stored, d("12.50"));
    }

    #[test]
    fn coerce_overflow_is_a_size_error() {
        let pic = Picture::parse("9(3)").unwrap(); // máx 999
        assert_eq!(d("1000").coerce(&pic, Rounding::Truncate), Err(BcdError::Overflow));
        assert!(d("999").coerce(&pic, Rounding::Truncate).is_ok());
    }

    #[test]
    fn coerce_rounding_can_trigger_overflow() {
        // 999.6 redondeado a entero → 1000, que ya no cabe en 9(3).
        let pic = Picture::parse("9(3)").unwrap();
        assert_eq!(d("999.6").coerce(&pic, Rounding::HalfUp), Err(BcdError::Overflow));
    }

    #[test]
    fn unsigned_picture_stores_magnitude() {
        let pic = Picture::parse("9(4)V99").unwrap(); // sin S
        let stored = d("-12.34").coerce(&pic, Rounding::Truncate).unwrap();
        assert!(!stored.is_negative());
        assert_eq!(stored, d("12.34"));
    }

    #[test]
    fn signed_picture_keeps_the_sign() {
        let pic = Picture::parse("S9(4)V99").unwrap();
        let stored = d("-12.34").coerce(&pic, Rounding::Truncate).unwrap();
        assert!(stored.is_negative());
    }
}
