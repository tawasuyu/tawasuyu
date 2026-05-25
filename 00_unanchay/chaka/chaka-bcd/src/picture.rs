//! La cláusula `PICTURE` — la forma declarada de un campo numérico COBOL.
//!
//! Sólo el subconjunto numérico: `9` (dígito), `V` (punto decimal
//! implícito), `S` (signo), y la repetición `9(n)`. Lo de edición
//! (`Z`, `*`, `,`, `.`, `$`, `B`…) es presentación y se trata aparte.

use serde::{Deserialize, Serialize};

use crate::BcdError;

/// La forma de un campo numérico: cuántos dígitos enteros, cuántos
/// fraccionarios y si admite signo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    pub integer_digits: u8,
    pub fraction_digits: u8,
    pub signed: bool,
}

impl Picture {
    /// Construye una `Picture` directa.
    pub fn new(integer_digits: u8, fraction_digits: u8, signed: bool) -> Self {
        Self { integer_digits, fraction_digits, signed }
    }

    /// Dígitos totales del campo (enteros + fraccionarios).
    pub fn total_digits(&self) -> u8 {
        self.integer_digits + self.fraction_digits
    }

    /// Parsea una cláusula PICTURE — `"9(5)V99"`, `"S9(3)"`, `"9999"`.
    /// Acepta el prefijo `PIC ` / `PICTURE ` opcional.
    pub fn parse(src: &str) -> Result<Picture, BcdError> {
        let up = src.trim().to_ascii_uppercase();
        let body = up
            .strip_prefix("PICTURE ")
            .or_else(|| up.strip_prefix("PIC "))
            .unwrap_or(&up)
            .trim();

        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        let mut signed = false;
        let mut integer_digits: u32 = 0;
        let mut fraction_digits: u32 = 0;
        let mut seen_v = false;

        // El signo, si lo hay, va primero.
        if chars.first() == Some(&'S') {
            signed = true;
            i = 1;
        }

        let bad = || BcdError::BadPicture(src.to_string());

        while i < chars.len() {
            match chars[i] {
                'V' => {
                    if seen_v {
                        return Err(bad()); // dos puntos decimales
                    }
                    seen_v = true;
                    i += 1;
                }
                '9' => {
                    // Cuenta este 9 y un posible '(n)' que lo siga.
                    let mut count: u32 = 1;
                    i += 1;
                    if chars.get(i) == Some(&'(') {
                        i += 1;
                        let start = i;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                        if start == i || chars.get(i) != Some(&')') {
                            return Err(bad());
                        }
                        let n: u32 = chars[start..i]
                            .iter()
                            .collect::<String>()
                            .parse()
                            .map_err(|_| bad())?;
                        count = n;
                        i += 1; // consume ')'
                    }
                    if seen_v {
                        fraction_digits += count;
                    } else {
                        integer_digits += count;
                    }
                }
                _ => return Err(bad()),
            }
        }

        let total = integer_digits + fraction_digits;
        if total == 0 || total > 38 {
            // i128 soporta 38 dígitos decimales.
            return Err(bad());
        }
        Ok(Picture {
            integer_digits: integer_digits as u8,
            fraction_digits: fraction_digits as u8,
            signed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_integer_and_fraction() {
        let p = Picture::parse("9(5)V99").unwrap();
        assert_eq!(p, Picture::new(5, 2, false));
    }

    #[test]
    fn parses_signed() {
        let p = Picture::parse("S9(3)").unwrap();
        assert_eq!(p, Picture::new(3, 0, true));
        assert!(p.signed);
    }

    #[test]
    fn parses_repeated_nines() {
        assert_eq!(Picture::parse("9999V9").unwrap(), Picture::new(4, 1, false));
    }

    #[test]
    fn accepts_pic_prefix() {
        assert_eq!(Picture::parse("PIC 9(2)").unwrap(), Picture::new(2, 0, false));
        assert_eq!(Picture::parse("PICTURE S9V9").unwrap(), Picture::new(1, 1, true));
    }

    #[test]
    fn rejects_garbage_and_double_v() {
        assert!(Picture::parse("X(3)").is_err());
        assert!(Picture::parse("9V9V9").is_err());
        assert!(Picture::parse("").is_err());
        assert!(Picture::parse("9(").is_err());
    }

    #[test]
    fn total_digits_sums_both_parts() {
        assert_eq!(Picture::parse("9(7)V999").unwrap().total_digits(), 10);
    }
}
