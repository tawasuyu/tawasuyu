//! `Num` — un campo numérico COBOL en tiempo de ejecución.

use charka_bcd::{Decimal, Picture, Rounding};

/// Un campo numérico: un valor [`Decimal`] más la [`Picture`] que lo
/// conforma. Toda asignación pasa por la PICTURE — ese es el `MOVE` de
/// COBOL: el valor se ajusta a la escala y al tamaño declarados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Num {
    value: Decimal,
    pic: Picture,
}

impl Num {
    /// Campo nuevo en cero, con la PICTURE dada.
    pub fn new(pic: Picture) -> Self {
        Self {
            value: Decimal::zero(),
            pic,
        }
    }

    /// Campo con un `VALUE` inicial (el texto del literal). Un literal
    /// inválido deja el campo en cero.
    pub fn with_value(pic: Picture, literal: &str) -> Self {
        let mut n = Self::new(pic);
        if let Ok(d) = Decimal::parse(literal) {
            n.store(d);
        }
        n
    }

    /// El valor decimal actual.
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// La PICTURE del campo.
    pub fn picture(&self) -> Picture {
        self.pic
    }

    /// Asigna un valor conformándolo a la PICTURE: ajusta la escala
    /// truncando los dígitos fraccionarios sobrantes.
    pub fn store(&mut self, v: Decimal) {
        self.value = fit(v, &self.pic, Rounding::Truncate);
    }

    /// Como [`store`](Self::store) pero redondeando — el `ROUNDED`.
    pub fn store_rounded(&mut self, v: Decimal) {
        self.value = fit(v, &self.pic, Rounding::HalfUp);
    }

    /// Representación para `DISPLAY`: los dígitos del campo, rellenados
    /// con ceros a la izquierda hasta el total de la PICTURE; con un
    /// `-` adelante si el campo lleva signo y el valor es negativo.
    pub fn display(&self) -> String {
        let total = self.pic.total_digits() as usize;
        let abs = self.value.mantissa().unsigned_abs().to_string();
        let digits = if abs.len() >= total {
            abs[abs.len() - total..].to_string()
        } else {
            format!("{}{}", "0".repeat(total - abs.len()), abs)
        };
        if self.pic.signed && self.value.is_negative() {
            format!("-{digits}")
        } else {
            digits
        }
    }
}

/// Conforma un valor a una PICTURE. Si la parte entera no cabe (el
/// `ON SIZE ERROR` de COBOL) y ninguna cláusula lo captura, COBOL deja
/// los dígitos de bajo orden: reescalamos y enmascaramos.
fn fit(v: Decimal, pic: &Picture, rounding: Rounding) -> Decimal {
    if let Ok(d) = v.coerce(pic, rounding) {
        return d;
    }
    let r = v.rescale(pic.fraction_digits, rounding);
    let modulus = 10i128.pow(pic.total_digits() as u32);
    let mut m = r.mantissa() % modulus;
    if !pic.signed && m < 0 {
        m = -m;
    }
    Decimal::new(m, pic.fraction_digits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(s: &str) -> Picture {
        Picture::parse(s).expect("PICTURE válida")
    }

    fn dec(s: &str) -> Decimal {
        Decimal::parse(s).expect("decimal válido")
    }

    #[test]
    fn new_field_is_zero() {
        let n = Num::new(pic("9(5)"));
        assert!(n.value().is_zero());
        assert_eq!(n.display(), "00000");
    }

    #[test]
    fn with_value_initializes() {
        let n = Num::with_value(pic("9(3)"), "42");
        assert_eq!(n.display(), "042");
    }

    #[test]
    fn store_truncates_fraction() {
        let mut n = Num::new(pic("9(3)V99"));
        n.store(dec("12.3456"));
        assert_eq!(n.value(), dec("12.34"));
    }

    #[test]
    fn store_rounded_rounds_fraction() {
        let mut n = Num::new(pic("9(3)V99"));
        n.store_rounded(dec("12.3456"));
        assert_eq!(n.value(), dec("12.35"));
    }

    #[test]
    fn store_overflow_keeps_low_order_digits() {
        // 1234 no cabe en 9(3): COBOL conserva los 3 dígitos bajos.
        let mut n = Num::new(pic("9(3)"));
        n.store(dec("1234"));
        assert_eq!(n.display(), "234");
    }

    #[test]
    fn unsigned_field_stores_magnitude() {
        let mut n = Num::new(pic("9(3)"));
        n.store(dec("-7"));
        assert_eq!(n.display(), "007");
        assert!(!n.value().is_negative());
    }

    #[test]
    fn signed_field_keeps_sign_in_display() {
        let mut n = Num::new(pic("S9(3)"));
        n.store(dec("-7"));
        assert_eq!(n.display(), "-007");
    }

    #[test]
    fn display_includes_implied_fraction_digits() {
        // PIC 9(2)V99, valor 7.5 → dígitos 0750.
        let mut n = Num::new(pic("9(2)V99"));
        n.store(dec("7.5"));
        assert_eq!(n.display(), "0750");
    }
}
