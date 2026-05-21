//! `format_edited` — el formateo de un valor numérico según una
//! PICTURE de edición (`ZZ,ZZ9.99`).

use charka_bcd::{Decimal, Rounding};

/// Formatea `value` según una PICTURE de edición. Soporta `9` (dígito),
/// `Z` (dígito con supresión de ceros a la izquierda), `,` (coma de
/// millares, en blanco dentro de la zona suprimida), `.` (punto
/// decimal) y `B` (espacio). El signo se descarta.
pub fn format_edited(value: Decimal, pic: &str) -> String {
    let up = pic.to_uppercase();
    let (int_pic, frac_pic) = match up.split_once('.') {
        Some((a, b)) => (a, b),
        None => (up.as_str(), ""),
    };
    let count_digits = |s: &str| s.chars().filter(|c| *c == '9' || *c == 'Z').count();
    let int_digits = count_digits(int_pic);
    let frac_digits = count_digits(frac_pic);
    let total = int_digits + frac_digits;

    // Los dígitos del valor, con exactamente `frac_digits` decimales.
    let mantissa = value
        .rescale(frac_digits as u8, Rounding::Truncate)
        .mantissa()
        .unsigned_abs();
    let mut digits = mantissa.to_string();
    if digits.len() < total {
        digits = format!("{}{}", "0".repeat(total - digits.len()), digits);
    } else if digits.len() > total {
        digits = digits[digits.len() - total..].to_string();
    }
    let int_part = &digits.as_bytes()[..int_digits];
    let frac_part = &digits.as_bytes()[int_digits..];

    let mut out = String::new();
    let mut di = 0;
    let mut seen = false;
    for ch in int_pic.chars() {
        match ch {
            '9' => {
                out.push(int_part[di] as char);
                di += 1;
                seen = true;
            }
            'Z' => {
                let d = int_part[di];
                di += 1;
                if seen || d != b'0' {
                    out.push(d as char);
                    seen = true;
                } else {
                    out.push(' ');
                }
            }
            ',' => out.push(if seen { ',' } else { ' ' }),
            'B' => out.push(' '),
            other => out.push(other),
        }
    }
    if !frac_pic.is_empty() {
        out.push('.');
        let mut fi = 0;
        for ch in frac_pic.chars() {
            match ch {
                '9' | 'Z' => {
                    out.push(frac_part[fi] as char);
                    fi += 1;
                }
                other => out.push(other),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(s: &str) -> Decimal {
        Decimal::parse(s).unwrap()
    }

    #[test]
    fn suppresses_leading_zeros_and_inserts_commas() {
        assert_eq!(format_edited(dec("1234.5"), "Z,ZZ9.99"), "1,234.50");
        assert_eq!(format_edited(dec("7"), "Z,ZZ9.99"), "    7.00");
        assert_eq!(format_edited(dec("0"), "Z,ZZ9.99"), "    0.00");
    }

    #[test]
    fn comma_in_suppressed_zone_is_blank() {
        // El millar va en blanco si no hay dígito significativo antes.
        assert_eq!(format_edited(dec("42"), "ZZ,ZZ9"), "    42");
        assert_eq!(format_edited(dec("12345"), "ZZ,ZZ9"), "12,345");
    }

    #[test]
    fn nine_positions_always_show() {
        assert_eq!(format_edited(dec("0"), "999"), "000");
        assert_eq!(format_edited(dec("0"), "ZZZ"), "   ");
    }
}
