//! Parseo de expresiones aritméticas (`COMPUTE`) y de condiciones
//! (`IF`, `PERFORM UNTIL`).

use chaka_parser::TokenKind;

use crate::ast::{BinOp, CmpOp, Cond, Expr, Operand};
use crate::cursor::{parse_operand, Cursor};
use crate::kw::is_boundary;

// ── Expresiones ───────────────────────────────────────────────────

/// Parsea una expresión aritmética con precedencia y paréntesis.
pub(crate) fn parse_expr(c: &mut Cursor) -> Expr {
    parse_bin(c, 0)
}

/// Trepa por precedencia: `min_prec` es la mínima precedencia que este
/// nivel acepta seguir consumiendo.
fn parse_bin(c: &mut Cursor, min_prec: u8) -> Expr {
    let mut lhs = parse_unary(c);
    while let Some((op, prec, right_assoc)) = peek_binop(c) {
        if prec < min_prec {
            break;
        }
        c.bump();
        let next_min = if right_assoc { prec } else { prec + 1 };
        let rhs = parse_bin(c, next_min);
        lhs = Expr::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        };
    }
    lhs
}

/// Negación o signo unario delante de un primario.
fn parse_unary(c: &mut Cursor) -> Expr {
    if c.eat_sym("-") {
        return Expr::Neg(Box::new(parse_unary(c)));
    }
    if c.eat_sym("+") {
        return parse_unary(c);
    }
    parse_primary(c)
}

/// Un primario: un paréntesis o un operando hoja.
fn parse_primary(c: &mut Cursor) -> Expr {
    if c.eat_sym("(") {
        let e = parse_bin(c, 0);
        c.eat_sym(")");
        return e;
    }
    // No consumir un verbo o conector: la expresión terminó.
    if c.done() || c.peek_word().map(|w| is_boundary(&w)).unwrap_or(false) {
        return Expr::Operand(Operand::Num("0".into()));
    }
    Expr::Operand(parse_operand(c))
}

/// El operador binario en el token actual, con su precedencia y si es
/// asociativo a derecha. `**` es la única potencia, asociativa a der.
fn peek_binop(c: &Cursor) -> Option<(BinOp, u8, bool)> {
    let t = c.peek()?;
    if t.kind != TokenKind::Symbol {
        return None;
    }
    match t.text.as_str() {
        "+" => Some((BinOp::Add, 1, false)),
        "-" => Some((BinOp::Sub, 1, false)),
        "*" => Some((BinOp::Mul, 2, false)),
        "/" => Some((BinOp::Div, 2, false)),
        "**" => Some((BinOp::Pow, 3, true)),
        _ => None,
    }
}

// ── Condiciones ───────────────────────────────────────────────────

/// Parsea una condición: comparaciones unidas por `AND`/`OR`/`NOT`.
pub(crate) fn parse_cond(c: &mut Cursor) -> Cond {
    parse_or(c)
}

fn parse_or(c: &mut Cursor) -> Cond {
    let mut lhs = parse_and(c);
    while c.eat_word("OR") {
        let rhs = parse_and(c);
        lhs = Cond::Or(Box::new(lhs), Box::new(rhs));
    }
    lhs
}

fn parse_and(c: &mut Cursor) -> Cond {
    let mut lhs = parse_not(c);
    while c.eat_word("AND") {
        let rhs = parse_not(c);
        lhs = Cond::And(Box::new(lhs), Box::new(rhs));
    }
    lhs
}

fn parse_not(c: &mut Cursor) -> Cond {
    if c.eat_word("NOT") {
        return Cond::Not(Box::new(parse_not(c)));
    }
    parse_cond_primary(c)
}

/// Un primario de condición: un paréntesis, una comparación, o un dato
/// suelto (un nombre de condición de nivel 88).
fn parse_cond_primary(c: &mut Cursor) -> Cond {
    if c.eat_sym("(") {
        let inner = parse_or(c);
        c.eat_sym(")");
        return inner;
    }
    if c.done() || c.peek_word().map(|w| is_boundary(&w)).unwrap_or(false) {
        return Cond::Named(String::new());
    }
    let lhs = parse_operand(c);
    match parse_cmp_op(c) {
        Some(op) => {
            let rhs = parse_operand(c);
            Cond::Compare { lhs, op, rhs }
        }
        None => match lhs {
            Operand::Data(n) => Cond::Named(n),
            // Un literal solo como condición es raro; se degrada a "≠ 0".
            other => Cond::Compare {
                lhs: other,
                op: CmpOp::Ne,
                rhs: Operand::Num("0".into()),
            },
        },
    }
}

/// Lee un operador relacional (forma símbolo o forma palabra). Si no
/// hay ninguno, rebobina el cursor y devuelve `None`.
fn parse_cmp_op(c: &mut Cursor) -> Option<CmpOp> {
    let save = c.pos;
    c.eat_word("IS");
    let negated = c.eat_word("NOT");
    if let Some(op) = cmp_core(c) {
        return Some(if negated { negate(op) } else { op });
    }
    c.pos = save;
    None
}

/// El núcleo del comparador, sin el `IS`/`NOT` opcionales.
fn cmp_core(c: &mut Cursor) -> Option<CmpOp> {
    if c.eat_sym("<>") {
        return Some(CmpOp::Ne);
    }
    if c.eat_sym("<=") {
        return Some(CmpOp::Le);
    }
    if c.eat_sym(">=") {
        return Some(CmpOp::Ge);
    }
    if c.eat_sym("=") {
        return Some(CmpOp::Eq);
    }
    if c.eat_sym("<") {
        return Some(CmpOp::Lt);
    }
    if c.eat_sym(">") {
        return Some(CmpOp::Gt);
    }
    if c.eat_word("EQUAL") || c.eat_word("EQUALS") {
        c.eat_word("TO");
        return Some(CmpOp::Eq);
    }
    if c.eat_word("GREATER") {
        c.eat_word("THAN");
        if c.eat_word("OR") {
            c.eat_word("EQUAL");
            c.eat_word("TO");
            return Some(CmpOp::Ge);
        }
        return Some(CmpOp::Gt);
    }
    if c.eat_word("LESS") {
        c.eat_word("THAN");
        if c.eat_word("OR") {
            c.eat_word("EQUAL");
            c.eat_word("TO");
            return Some(CmpOp::Le);
        }
        return Some(CmpOp::Lt);
    }
    None
}

/// El comparador opuesto — para resolver `NOT` delante de un relacional.
fn negate(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Eq => CmpOp::Ne,
        CmpOp::Ne => CmpOp::Eq,
        CmpOp::Lt => CmpOp::Ge,
        CmpOp::Ge => CmpOp::Lt,
        CmpOp::Gt => CmpOp::Le,
        CmpOp::Le => CmpOp::Gt,
    }
}
