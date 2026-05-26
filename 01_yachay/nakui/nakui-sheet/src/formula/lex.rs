//! Lexer mínimo para fórmulas Excel.
//!
//! Decisión: NO emitimos un token `CellRef` desde el lexer. Las
//! referencias y rangos se reconocen en el parser, donde tras ver un
//! identificador inspeccionamos si parsea como `CellRef`, si hay `(`
//! detrás (función), o si hay `:` (rango). Esto evita reglas
//! ambiguas a nivel léxico (`A1` vs `SIN`).

use rust_decimal::Decimal;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Number(Decimal),
    /// Texto literal entre comillas dobles, ya sin las comillas y con
    /// `""` → `"` decodificado.
    Text(String),
    /// Identificador (funciones, `TRUE`/`FALSE`, o el prefijo
    /// alfabético de una `CellRef` posible). Mayúsculas preservadas
    /// para la decodificación posterior — el parser normaliza.
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Amp,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LParen,
    RParen,
    Comma,
    Colon,
    Dollar,
}

#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unterminated string literal starting at position {0}")]
    UnterminatedString(usize),
    #[error("invalid number `{0}` at position {1}")]
    InvalidNumber(String, usize),
    #[error("unexpected character `{0}` at position {1}")]
    UnexpectedChar(char, usize),
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, LexError> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];

        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Números: dígitos + opcional `.` + dígitos. No soportamos
        // notación científica intencionalmente — las hojas de
        // contabilidad no la necesitan, y omitirla evita ambigüedades
        // con tokens tipo `E5` (referencia a celda E5 vs exponente).
        if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(|b| b.is_ascii_digit()))
        {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text = &src[start..i];
            let num = Decimal::from_str(text)
                .map_err(|_| LexError::InvalidNumber(text.to_string(), start))?;
            tokens.push(Token::Number(num));
            continue;
        }

        // Texto entre comillas. `""` dentro escapa a una comilla.
        // Iteramos por chars (no bytes) para que UTF-8 multi-byte
        // (`é`, `ñ`, emoji) llegue intacto al string final.
        if c == b'"' {
            let start = i;
            i += 1;
            let mut buf = String::new();
            let tail = &src[i..];
            let mut iter = tail.char_indices();
            loop {
                match iter.next() {
                    None => return Err(LexError::UnterminatedString(start)),
                    Some((off, '"')) => {
                        // Pico siguiente para decidir escape vs cierre.
                        let after = i + off + 1;
                        if src.as_bytes().get(after) == Some(&b'"') {
                            buf.push('"');
                            // Avanzamos el char_indices saltando la
                            // segunda comilla; reconstruimos el iter
                            // desde la posición correcta.
                            let new_tail = &src[after + 1..];
                            i = after + 1;
                            iter = new_tail.char_indices();
                            continue;
                        }
                        i = after;
                        break;
                    }
                    Some((_, ch)) => buf.push(ch),
                }
            }
            tokens.push(Token::Text(buf));
            continue;
        }

        // Identificadores: comienzan con letra o `_`, continúan con
        // letras, dígitos y `_`. Las referencias de celda (`A1`,
        // `AB12`) caen aquí — el parser las reconoce.
        if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
            {
                i += 1;
            }
            tokens.push(Token::Ident(src[start..i].to_string()));
            continue;
        }

        // Operadores. Los de dos chars (`<=`, `>=`, `<>`) van primero.
        match c {
            b'<' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    tokens.push(Token::Le);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'>') {
                    tokens.push(Token::Ne);
                    i += 2;
                } else {
                    tokens.push(Token::Lt);
                    i += 1;
                }
            }
            b'>' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    tokens.push(Token::Gt);
                    i += 1;
                }
            }
            b'=' => {
                tokens.push(Token::Eq);
                i += 1;
            }
            b'+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            b'-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            b'*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            b'/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            b'^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            b'%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            b'&' => {
                tokens.push(Token::Amp);
                i += 1;
            }
            b'(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            b')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            b',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            b':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            b'$' => {
                tokens.push(Token::Dollar);
                i += 1;
            }
            other => return Err(LexError::UnexpectedChar(other as char, i)),
        }
    }

    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizes_basic_arithmetic() {
        let toks = tokenize("1 + 2.5 * 3").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Number(Decimal::from(1)),
                Token::Plus,
                Token::Number(Decimal::from_str("2.5").unwrap()),
                Token::Star,
                Token::Number(Decimal::from(3)),
            ]
        );
    }

    #[test]
    fn recognizes_double_char_operators() {
        let toks = tokenize("a<=b >= c <>d").unwrap();
        let kinds: Vec<_> = toks
            .iter()
            .filter_map(|t| match t {
                Token::Le | Token::Ge | Token::Ne | Token::Lt | Token::Gt => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec![Token::Le, Token::Ge, Token::Ne]);
    }

    #[test]
    fn strings_with_escaped_quotes() {
        // Excel: `""` dentro de "..." es el escape de una comilla.
        // Input: "he said ""hi"""  → resultado: he said "hi"
        let toks = tokenize("\"he said \"\"hi\"\"\"").unwrap();
        assert_eq!(toks, vec![Token::Text("he said \"hi\"".into())]);
    }

    #[test]
    fn strings_preserve_utf8_multibyte() {
        let toks = tokenize("\"café ñandú\"").unwrap();
        assert_eq!(toks, vec![Token::Text("café ñandú".into())]);
    }

    #[test]
    fn unterminated_string_errors_with_position() {
        let err = tokenize(r#"1 + "open"#).unwrap_err();
        assert_eq!(err, LexError::UnterminatedString(4));
    }

    #[test]
    fn cell_refs_emerge_as_single_idents() {
        // Decisión: idents incluyen dígitos (`MY_FN2`, `A1`). La
        // diferenciación CellRef-vs-función la hace el parser
        // mirando el patrón de letras+dígitos.
        let toks = tokenize("SUM(A1:B10)").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Ident("SUM".into()),
                Token::LParen,
                Token::Ident("A1".into()),
                Token::Colon,
                Token::Ident("B10".into()),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn dollar_anchors_preserved_for_parser() {
        let toks = tokenize("$A$1").unwrap();
        assert_eq!(
            toks,
            vec![
                Token::Dollar,
                Token::Ident("A".into()),
                Token::Dollar,
                Token::Number(Decimal::from(1)),
            ]
        );
    }

    #[test]
    fn leading_decimal_point() {
        let toks = tokenize(".5").unwrap();
        assert_eq!(toks, vec![Token::Number(Decimal::from_str(".5").unwrap())]);
    }
}
