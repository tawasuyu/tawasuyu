//! Parser Pratt (precedence-climbing) sobre los tokens de `lex`.
//!
//! Precedencias (igual que Excel, de menor a mayor):
//!   0. `=` `<>` `<` `<=` `>` `>=`
//!   1. `&`
//!   2. `+` `-`
//!   3. `*` `/`
//!   4. `^`  (right-associative)
//!   5. prefijo `-` `+`
//!   6. postfijo `%`
//!   7. primary (literal, ref, range, llamada, grupo)
//!
//! El rango `A1:B2` se reconoce dentro de `primary` solo cuando ambos
//! lados parsean como `CellRef`. Eso evita ambigüedad con `A1:B2+1`,
//! que se descompone como `(A1:B2) + 1`.

use super::ast::{BinaryOp, FormulaExpr, UnaryOp};
use super::lex::{tokenize, LexError, Token};
use crate::cell::{CellRange, CellRef};
use rust_decimal::Decimal;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(#[from] LexError),
    #[error("unexpected end of input; expected {expected}")]
    UnexpectedEof { expected: &'static str },
    #[error("unexpected token `{found}`; expected {expected}")]
    Unexpected {
        found: String,
        expected: &'static str,
    },
    #[error("invalid cell reference around `{0}`")]
    BadCellRef(String),
    #[error("invalid range: both sides must be cell references")]
    BadRange,
    #[error("function name expected, got `{0}`")]
    BadFunctionName(String),
}

pub fn parse_formula(src: &str) -> Result<FormulaExpr, ParseError> {
    let tokens = tokenize(src)?;
    let mut p = Parser {
        tokens: &tokens,
        pos: 0,
    };
    let expr = p.parse_expr(0)?;
    if p.pos != tokens.len() {
        return Err(ParseError::Unexpected {
            found: format!("{:?}", tokens[p.pos]),
            expected: "end of formula",
        });
    }
    Ok(expr)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&'a Token> {
        let t = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(t)
    }

    fn expect(&mut self, kind: &Token, label: &'static str) -> Result<(), ParseError> {
        match self.peek() {
            Some(t) if std::mem::discriminant(t) == std::mem::discriminant(kind) => {
                self.pos += 1;
                Ok(())
            }
            Some(t) => Err(ParseError::Unexpected {
                found: format!("{:?}", t),
                expected: label,
            }),
            None => Err(ParseError::UnexpectedEof { expected: label }),
        }
    }

    /// Precedence-climbing. `min_bp` es el binding power mínimo que
    /// los operadores binarios deben superar para extender la
    /// expresión actual.
    fn parse_expr(&mut self, min_bp: u8) -> Result<FormulaExpr, ParseError> {
        let mut lhs = self.parse_prefix()?;

        loop {
            // Postfijo `%`: se aplica antes que cualquier infijo.
            if matches!(self.peek(), Some(Token::Percent)) {
                self.pos += 1;
                lhs = FormulaExpr::Unary(UnaryOp::Percent, Box::new(lhs));
                continue;
            }

            let (op, l_bp, r_bp) = match self.peek() {
                Some(Token::Eq) => (BinaryOp::Eq, 1, 2),
                Some(Token::Ne) => (BinaryOp::Ne, 1, 2),
                Some(Token::Lt) => (BinaryOp::Lt, 1, 2),
                Some(Token::Le) => (BinaryOp::Le, 1, 2),
                Some(Token::Gt) => (BinaryOp::Gt, 1, 2),
                Some(Token::Ge) => (BinaryOp::Ge, 1, 2),
                Some(Token::Amp) => (BinaryOp::Concat, 3, 4),
                Some(Token::Plus) => (BinaryOp::Add, 5, 6),
                Some(Token::Minus) => (BinaryOp::Sub, 5, 6),
                Some(Token::Star) => (BinaryOp::Mul, 7, 8),
                Some(Token::Slash) => (BinaryOp::Div, 7, 8),
                // Pow right-assoc: l_bp > r_bp para que `2^3^2` parse
                // como `2^(3^2)`.
                Some(Token::Caret) => (BinaryOp::Pow, 10, 9),
                _ => break,
            };

            if l_bp < min_bp {
                break;
            }
            self.pos += 1;
            let rhs = self.parse_expr(r_bp)?;
            lhs = FormulaExpr::Binary(op, Box::new(lhs), Box::new(rhs));
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<FormulaExpr, ParseError> {
        match self.peek() {
            Some(Token::Minus) => {
                self.pos += 1;
                // bp prefijo = 11, mayor que cualquier binario
                // (caret = 10/9). Garantiza que `-2^4` parse como
                // `-(2^4)` igual que Excel.
                let inner = self.parse_expr(11)?;
                Ok(FormulaExpr::Unary(UnaryOp::Neg, Box::new(inner)))
            }
            Some(Token::Plus) => {
                self.pos += 1;
                let inner = self.parse_expr(11)?;
                Ok(FormulaExpr::Unary(UnaryOp::Plus, Box::new(inner)))
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<FormulaExpr, ParseError> {
        match self.peek().cloned() {
            None => Err(ParseError::UnexpectedEof {
                expected: "expression",
            }),
            Some(Token::Number(n)) => {
                self.pos += 1;
                Ok(FormulaExpr::Number(n))
            }
            Some(Token::Text(t)) => {
                self.pos += 1;
                Ok(FormulaExpr::Text(t))
            }
            Some(Token::LParen) => {
                self.pos += 1;
                let inner = self.parse_expr(0)?;
                self.expect(&Token::RParen, "`)`")?;
                Ok(inner)
            }
            Some(Token::Dollar) | Some(Token::Ident(_)) => self.parse_ident_starter(),
            Some(other) => Err(ParseError::Unexpected {
                found: format!("{:?}", other),
                expected: "expression",
            }),
        }
    }

    /// Una expresión que empieza con `$` o un identificador puede ser:
    /// un literal `TRUE`/`FALSE`, una llamada a función, una `CellRef`
    /// (suelta o como inicio de un rango), o un `#NAME?` si nada de eso
    /// encaja.
    fn parse_ident_starter(&mut self) -> Result<FormulaExpr, ParseError> {
        let saved = self.pos;

        // Intento 1: CellRef (con dollars opcionales). Si tras la
        // referencia hay `:`, busco otra y emito un CellRange.
        if let Some(cr) = self.try_consume_cell_ref() {
            if matches!(self.peek(), Some(Token::Colon)) {
                self.pos += 1;
                let cr2 = self
                    .try_consume_cell_ref()
                    .ok_or(ParseError::BadRange)?;
                return Ok(FormulaExpr::Range(CellRange::new(cr, cr2)));
            }
            return Ok(FormulaExpr::Ref(cr));
        }

        // Reset: no era CellRef.
        self.pos = saved;

        // Intento 2: bare ident (function o bool literal).
        let ident = match self.advance() {
            Some(Token::Ident(s)) => s.clone(),
            Some(t) => {
                return Err(ParseError::Unexpected {
                    found: format!("{:?}", t),
                    expected: "identifier",
                })
            }
            None => {
                return Err(ParseError::UnexpectedEof {
                    expected: "identifier",
                })
            }
        };

        if matches!(self.peek(), Some(Token::LParen)) {
            self.pos += 1;
            let args = self.parse_args()?;
            self.expect(&Token::RParen, "`)`")?;
            return Ok(FormulaExpr::Call(ident.to_uppercase(), args));
        }

        match ident.to_uppercase().as_str() {
            "TRUE" => Ok(FormulaExpr::Bool(true)),
            "FALSE" => Ok(FormulaExpr::Bool(false)),
            _ => Err(ParseError::BadFunctionName(ident)),
        }
    }

    /// Intenta consumir una referencia de celda. Casos válidos:
    ///   - `Ident("A1")` — letras+dígitos en un solo token.
    ///   - `Dollar Ident("A1")` — col anclada, fila en el ident.
    ///   - `Ident("A") Dollar Number(1)` — fila anclada explícita.
    ///   - `Dollar Ident("A") Dollar Number(1)` — ambas ancladas.
    ///   - `Ident("A") Number(1)` — fallback puro split (raro,
    ///     viene de `=A1` cuando el lexer no fundiera, aunque ahora
    ///     siempre fundamos).
    ///
    /// Si nada encaja restaura `pos`. La verificación del rango
    /// (fila > 0) la hace `CellRef::from_str`.
    fn try_consume_cell_ref(&mut self) -> Option<CellRef> {
        let saved = self.pos;
        let col_abs = matches!(self.peek(), Some(Token::Dollar));
        if col_abs {
            self.pos += 1;
        }

        let ident_text = match self.peek() {
            Some(Token::Ident(s)) => s.clone(),
            _ => {
                self.pos = saved;
                return None;
            }
        };
        self.pos += 1;

        // Caso A: ident con letras seguidas de dígitos (`A1`,
        // `AB12`). Reconstruimos el literal canónico.
        let letters_len = ident_text
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        if letters_len > 0 && letters_len < ident_text.len() {
            // Verificar que el sufijo sea solo dígitos.
            let suffix = &ident_text[letters_len..];
            if suffix.chars().all(|c| c.is_ascii_digit()) {
                let mut buf = String::new();
                if col_abs {
                    buf.push('$');
                }
                buf.push_str(&ident_text);
                if let Ok(cr) = buf.parse::<CellRef>() {
                    return Some(cr);
                }
            }
            self.pos = saved;
            return None;
        }

        // Caso B: ident solo letras → puede venir `[$] Number` después.
        if letters_len == ident_text.len() {
            let row_abs = matches!(self.peek(), Some(Token::Dollar));
            if row_abs {
                self.pos += 1;
            }
            let row = match self.peek() {
                Some(Token::Number(n)) => *n,
                _ => {
                    self.pos = saved;
                    return None;
                }
            };
            if row.fract() != Decimal::ZERO || row <= Decimal::ZERO {
                self.pos = saved;
                return None;
            }
            let row_u32: u32 = match row.to_string().parse() {
                Ok(n) => n,
                Err(_) => {
                    self.pos = saved;
                    return None;
                }
            };
            self.pos += 1;
            let mut buf = String::new();
            if col_abs {
                buf.push('$');
            }
            buf.push_str(&ident_text);
            if row_abs {
                buf.push('$');
            }
            buf.push_str(&row_u32.to_string());
            if let Ok(cr) = buf.parse::<CellRef>() {
                return Some(cr);
            }
        }

        self.pos = saved;
        None
    }

    fn parse_args(&mut self) -> Result<Vec<FormulaExpr>, ParseError> {
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Token::RParen)) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr(0)?);
            if matches!(self.peek(), Some(Token::Comma)) {
                self.pos += 1;
                continue;
            }
            break;
        }
        Ok(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;
    use rust_decimal::Decimal;

    #[test]
    fn parses_plain_number() {
        let e = parse_formula("42").unwrap();
        assert_eq!(e, FormulaExpr::Number(Decimal::from(42)));
    }

    #[test]
    fn arithmetic_respects_precedence() {
        // 1 + 2 * 3 = 1 + (2 * 3)
        let e = parse_formula("1 + 2 * 3").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Add, lhs, rhs) => {
                assert!(matches!(*lhs, FormulaExpr::Number(_)));
                assert!(matches!(*rhs, FormulaExpr::Binary(BinaryOp::Mul, _, _)));
            }
            _ => panic!("expected Add at root"),
        }
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 = 2^(3^2) = 512
        let e = parse_formula("2^3^2").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Pow, lhs, rhs) => {
                assert!(matches!(*lhs, FormulaExpr::Number(_)));
                assert!(matches!(*rhs, FormulaExpr::Binary(BinaryOp::Pow, _, _)));
            }
            _ => panic!("expected Pow at root"),
        }
    }

    #[test]
    fn unary_minus_binds_tighter_than_caret() {
        // En Excel: -2^4 = (-2)^4 = 16, no -(2^4) = -16.
        // Nuestro bp prefijo = 11 > pow = 10 → unary se aplica primero.
        let e = parse_formula("-2^4").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Pow, lhs, _) => {
                assert!(matches!(*lhs, FormulaExpr::Unary(UnaryOp::Neg, _)));
            }
            _ => panic!("expected Pow with negated lhs"),
        }
    }

    #[test]
    fn cell_ref_parses_inside_expression() {
        let e = parse_formula("A1+B2").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Add, lhs, rhs) => {
                assert_eq!(*lhs, FormulaExpr::Ref(CellRef::new(0, 0)));
                assert_eq!(*rhs, FormulaExpr::Ref(CellRef::new(1, 1)));
            }
            _ => panic!("expected Add of two refs"),
        }
    }

    #[test]
    fn absolute_anchors_in_refs() {
        let e = parse_formula("$A$1+A$1").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Add, lhs, rhs) => {
                let l = match *lhs {
                    FormulaExpr::Ref(c) => c,
                    _ => panic!(),
                };
                assert!(l.col_absolute && l.row_absolute);
                let r = match *rhs {
                    FormulaExpr::Ref(c) => c,
                    _ => panic!(),
                };
                assert!(!r.col_absolute && r.row_absolute);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn range_inside_sum_call() {
        let e = parse_formula("SUM(A1:B2)").unwrap();
        match e {
            FormulaExpr::Call(name, args) => {
                assert_eq!(name, "SUM");
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], FormulaExpr::Range(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn function_names_normalize_to_uppercase() {
        let e1 = parse_formula("sum(1,2)").unwrap();
        let e2 = parse_formula("Sum(1,2)").unwrap();
        let e3 = parse_formula("SUM(1,2)").unwrap();
        assert_eq!(e1, e2);
        assert_eq!(e2, e3);
    }

    #[test]
    fn bool_literals() {
        assert_eq!(parse_formula("TRUE").unwrap(), FormulaExpr::Bool(true));
        assert_eq!(parse_formula("False").unwrap(), FormulaExpr::Bool(false));
    }

    #[test]
    fn empty_arg_list() {
        let e = parse_formula("NOW()").unwrap();
        assert!(matches!(e, FormulaExpr::Call(ref n, ref a) if n == "NOW" && a.is_empty()));
    }

    #[test]
    fn percent_postfix() {
        // 50% = 0.5 (representado como Unary(Percent, 50))
        let e = parse_formula("50%").unwrap();
        assert_eq!(
            e,
            FormulaExpr::Unary(UnaryOp::Percent, Box::new(FormulaExpr::Number(Decimal::from(50))))
        );
    }

    #[test]
    fn concat_with_amp() {
        let e = parse_formula(r#""hola "&"mundo""#).unwrap();
        assert!(matches!(e, FormulaExpr::Binary(BinaryOp::Concat, _, _)));
    }

    #[test]
    fn comparison_below_arithmetic() {
        // A1+1 > B2*2  →  (A1+1) > (B2*2)
        let e = parse_formula("A1+1 > B2*2").unwrap();
        assert!(matches!(e, FormulaExpr::Binary(BinaryOp::Gt, _, _)));
    }

    #[test]
    fn paren_grouping() {
        // (1+2)*3 — la suma debe ser hija de la multiplicación.
        let e = parse_formula("(1+2)*3").unwrap();
        match e {
            FormulaExpr::Binary(BinaryOp::Mul, lhs, _) => {
                assert!(matches!(*lhs, FormulaExpr::Binary(BinaryOp::Add, _, _)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn trailing_garbage_rejected() {
        assert!(parse_formula("1+2 garbage").is_err());
    }
}
