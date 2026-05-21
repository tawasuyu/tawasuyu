//! `charka-runtime` — el soporte de ejecución de los programas COBOL
//! transpilados.
//!
//! Lo que `charka-codegen` emite no es Rust autónomo: es Rust que
//! enlaza contra esta biblioteca. Aquí viven los tipos que dan a un
//! programa transpilado la semántica de COBOL en tiempo de ejecución:
//!
//! - [`Num`] — un campo numérico (`PIC 9(5)V99`): un [`Decimal`] de
//!   punto fijo conformado a su [`Picture`]. Toda asignación trunca a
//!   la escala y al tamaño declarados, como el `MOVE` de COBOL.
//! - [`Text`] — un campo alfanumérico (`PIC X(20)`) de longitud fija:
//!   toda asignación justifica a la izquierda y rellena o trunca.
//!
//! La aritmética decimal exacta la aporta `charka-bcd`, cuyos tipos
//! ([`Decimal`], [`Picture`], [`Rounding`]) se reexportan para que el
//! código generado sólo necesite `use charka_runtime::*;`.

#![forbid(unsafe_code)]

mod num;
mod text;

pub use charka_bcd::{Decimal, Picture, Rounding};
pub use num::Num;
pub use text::Text;

use std::cmp::Ordering;

/// Compara dos campos alfanuméricos con la semántica de COBOL: el más
/// corto se considera rellenado con espacios a la derecha, de modo que
/// `"AB"` y `"AB  "` son iguales.
pub fn cobol_text_cmp(a: &str, b: &str) -> Ordering {
    let n = a.chars().count().max(b.chars().count());
    let padded = |s: &str| -> Vec<char> {
        let mut v: Vec<char> = s.chars().collect();
        while v.len() < n {
            v.push(' ');
        }
        v
    };
    padded(a).cmp(&padded(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_cmp_orders_lexically() {
        assert_eq!(cobol_text_cmp("ABC", "ABD"), Ordering::Less);
        assert_eq!(cobol_text_cmp("ABD", "ABC"), Ordering::Greater);
        assert_eq!(cobol_text_cmp("ABC", "ABC"), Ordering::Equal);
    }

    #[test]
    fn text_cmp_pads_the_shorter_with_spaces() {
        assert_eq!(cobol_text_cmp("AB", "AB  "), Ordering::Equal);
        assert_eq!(cobol_text_cmp("AB", "ABC"), Ordering::Less);
    }

    #[test]
    fn fields_compose_for_generated_code() {
        // Un mini-programa transpilado a mano: WS-CT crece, WS-MSG fijo.
        let mut ws_ct = Num::with_value(Picture::new(3, 0, false), "0");
        ws_ct.store(ws_ct.value().add(&Decimal::from_integer(5)));
        assert_eq!(ws_ct.display(), "005");

        let mut ws_msg = Text::new(10);
        ws_msg.store("LISTO");
        assert_eq!(ws_msg.as_str(), "LISTO     ");
    }
}
