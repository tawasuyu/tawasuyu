//! Codec packed-decimal (COBOL `COMP-3`).
//!
//! En el formato packed cada dígito ocupa 4 bits (un nibble); el último
//! nibble es el signo (`0xC` positivo, `0xD` negativo, `0xF` sin signo).
//! Si la cuenta de dígitos del campo es par, los nibbles se rellenan con
//! un `0` al inicio para que el total (dígitos + signo) sea impar y
//! quepa exacto en bytes enteros.
//!
//! Por ejemplo `+12345` con `PIC S9(5) COMP-3` ocupa 3 bytes: `12 34 5C`.
//! `-12` con `PIC S9(2) COMP-3` ocupa 2 bytes: `01 2D`.

use crate::picture::Picture;
use crate::{BcdError, Decimal, Rounding};

/// Nibble de signo para un valor positivo.
const SIGN_POSITIVE: u8 = 0xC;
/// Nibble de signo para un valor negativo.
const SIGN_NEGATIVE: u8 = 0xD;
/// Nibble de signo para un campo sin signo declarado (`S` ausente).
const SIGN_UNSIGNED: u8 = 0xF;

/// Cuántos bytes ocupa un campo packed con la PICTURE dada.
pub fn packed_size(pic: &Picture) -> usize {
    (pic.total_digits() as usize + 2) / 2
}

/// Empaqueta un `Decimal` según la PICTURE indicada (lo conforma a su
/// escala con truncamiento). Devuelve el vector de bytes packed.
pub fn pack(value: &Decimal, pic: &Picture) -> Vec<u8> {
    pack_with_rounding(value, pic, Rounding::Truncate)
}

/// Igual que [`pack`] pero permite escoger el modo de redondeo al ajustar
/// el valor a la escala de la PICTURE.
pub fn pack_with_rounding(value: &Decimal, pic: &Picture, rounding: Rounding) -> Vec<u8> {
    let scaled = value.rescale(pic.fraction_digits, rounding);
    let total = pic.total_digits() as usize;
    let modulus = 10i128.pow(total as u32);
    let mut mantissa = scaled.mantissa();
    let negative = pic.signed && mantissa < 0;
    if mantissa < 0 {
        mantissa = -mantissa;
    }
    mantissa %= modulus;

    // Construye la lista de dígitos rellena con ceros a la izquierda.
    let digits_str = format!("{:0>width$}", mantissa, width = total);
    let mut nibbles: Vec<u8> = digits_str
        .bytes()
        .map(|b| b - b'0')
        .collect();

    // Si la cuenta de dígitos es par, añade un nibble cero al inicio
    // para que `dígitos + signo` sea impar — y se acomode en bytes.
    if nibbles.len() % 2 == 0 {
        nibbles.insert(0, 0);
    }

    // Nibble de signo al final.
    let sign = if !pic.signed {
        SIGN_UNSIGNED
    } else if negative {
        SIGN_NEGATIVE
    } else {
        SIGN_POSITIVE
    };
    nibbles.push(sign);

    nibbles
        .chunks(2)
        .map(|c| (c[0] << 4) | c[1])
        .collect()
}

/// Desempaqueta un campo packed según la PICTURE indicada. Falla si el
/// número de bytes no concuerda con la PICTURE o si algún nibble de
/// dígito no es 0-9.
pub fn unpack(bytes: &[u8], pic: &Picture) -> Result<Decimal, BcdError> {
    let expected = packed_size(pic);
    if bytes.len() != expected {
        return Err(BcdError::BadNumber(format!(
            "packed: {} bytes, esperaba {}",
            bytes.len(),
            expected
        )));
    }
    let total_nibbles = expected * 2;
    let pad_nibble = total_nibbles - pic.total_digits() as usize - 1;

    let mut mantissa: i128 = 0;
    let mut sign: u8 = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        let hi = byte >> 4;
        let lo = byte & 0x0F;
        for (n, &nibble) in [hi, lo].iter().enumerate() {
            let pos = idx * 2 + n;
            if pos < pad_nibble {
                if nibble != 0 {
                    return Err(BcdError::BadNumber(format!(
                        "packed: nibble de relleno no nulo ({nibble:#x})"
                    )));
                }
                continue;
            }
            if pos == total_nibbles - 1 {
                sign = nibble;
                continue;
            }
            if nibble > 9 {
                return Err(BcdError::BadNumber(format!(
                    "packed: nibble inválido ({nibble:#x})"
                )));
            }
            mantissa = mantissa * 10 + nibble as i128;
        }
    }

    match sign {
        SIGN_NEGATIVE => mantissa = -mantissa,
        SIGN_POSITIVE | SIGN_UNSIGNED => {}
        // Algunos compiladores usan `0xA` (positivo alternativo) o
        // `0xB` (negativo alternativo); aceptamos los menos.
        0xA | 0xE => {}
        0xB => mantissa = -mantissa,
        _ => {
            return Err(BcdError::BadNumber(format!(
                "packed: nibble de signo inválido ({sign:#x})"
            )))
        }
    }

    Ok(Decimal::new(mantissa, pic.fraction_digits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(s: &str) -> Picture {
        Picture::parse(s).unwrap()
    }

    fn dec(s: &str) -> Decimal {
        Decimal::parse(s).unwrap()
    }

    #[test]
    fn size_rounds_up_to_full_bytes() {
        assert_eq!(packed_size(&pic("9(5)")), 3); // 5 dig + signo = 3 bytes
        assert_eq!(packed_size(&pic("9(4)")), 3); // 4 dig + relleno + signo
        assert_eq!(packed_size(&pic("9(1)")), 1); // 1 dig + signo
        assert_eq!(packed_size(&pic("9(2)V99")), 3); // 4 dig + relleno + signo
    }

    #[test]
    fn pack_positive_five_digit_field() {
        // +12345 PIC S9(5) → 12 34 5C
        let bytes = pack(&dec("12345"), &pic("S9(5)"));
        assert_eq!(bytes, vec![0x12, 0x34, 0x5C]);
    }

    #[test]
    fn pack_negative_with_signed_picture() {
        // -12 PIC S9(2) → 01 2D (signo D, relleno 0 al frente)
        let bytes = pack(&dec("-12"), &pic("S9(2)"));
        assert_eq!(bytes, vec![0x01, 0x2D]);
    }

    #[test]
    fn pack_unsigned_uses_f_sign() {
        // 12345 PIC 9(5) (sin signo) → 12 34 5F
        let bytes = pack(&dec("12345"), &pic("9(5)"));
        assert_eq!(bytes, vec![0x12, 0x34, 0x5F]);
    }

    #[test]
    fn pack_with_fraction_digits_keeps_scale() {
        // 12.34 PIC 9(2)V99 → mantissa 1234 → 01 23 4F
        let bytes = pack(&dec("12.34"), &pic("9(2)V99"));
        assert_eq!(bytes, vec![0x01, 0x23, 0x4F]);
    }

    #[test]
    fn pack_truncates_overflow_keeping_low_order() {
        // 123456 no cabe en PIC 9(5) → conserva los 5 bajos: 23456.
        let bytes = pack(&dec("123456"), &pic("9(5)"));
        assert_eq!(bytes, vec![0x23, 0x45, 0x6F]);
    }

    #[test]
    fn unpack_roundtrip_signed() {
        let p = pic("S9(5)");
        for val in &["12345", "-12345", "0", "-1", "99999"] {
            let d = dec(val);
            let bytes = pack(&d, &p);
            let back = unpack(&bytes, &p).unwrap();
            assert_eq!(back.mantissa(), d.mantissa(), "valor {val}");
            assert_eq!(back.scale(), d.scale());
        }
    }

    #[test]
    fn unpack_roundtrip_with_fraction() {
        let p = pic("S9(3)V99");
        let d = dec("-123.45");
        let bytes = pack(&d, &p);
        let back = unpack(&bytes, &p).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn unpack_rejects_invalid_size() {
        let err = unpack(&[0x12, 0x3C], &pic("S9(5)")).unwrap_err();
        assert!(matches!(err, BcdError::BadNumber(_)));
    }

    #[test]
    fn unpack_rejects_invalid_digit_nibble() {
        // 0xA en posición de dígito (no de signo): inválido.
        let err = unpack(&[0xA2, 0x3C], &pic("S9(3)")).unwrap_err();
        assert!(matches!(err, BcdError::BadNumber(_)));
    }
}
