//! Renderizado `FormulaExpr → String`. La salida es canónica:
//! `parse(render(expr)) == expr` para cualquier expr bien formada.
//! Esto es lo que permite que fill/copy genere fórmulas nuevas y la
//! UI las muestre exactamente como las pintaría el motor.
//!
//! Reglas de paréntesis: se emiten cuando un operando interno tiene
//! precedencia estrictamente menor que la del padre, o cuando tiene
//! la misma precedencia pero está del lado "equivocado" de un
//! operador asimétrico (caret right-assoc; resto left-assoc). El
//! resultado mantiene la semántica sin paréntesis redundantes.

use super::ast::{BinaryOp, FormulaExpr, UnaryOp};

pub fn render(expr: &FormulaExpr) -> String {
    let mut buf = String::new();
    write_expr(expr, &mut buf, 0, false);
    buf
}

/// `min_prec` = precedencia desde el contexto del padre. `is_right` =
/// si el nodo actual es el operando derecho de un binario (importa
/// para left-associatividad).
fn write_expr(expr: &FormulaExpr, buf: &mut String, min_prec: u8, is_right: bool) {
    match expr {
        FormulaExpr::Number(n) => {
            buf.push_str(&n.normalize().to_string());
        }
        FormulaExpr::Text(s) => {
            buf.push('"');
            // Escape de comilla = doble comilla.
            for c in s.chars() {
                if c == '"' {
                    buf.push_str("\"\"");
                } else {
                    buf.push(c);
                }
            }
            buf.push('"');
        }
        FormulaExpr::Bool(true) => buf.push_str("TRUE"),
        FormulaExpr::Bool(false) => buf.push_str("FALSE"),
        FormulaExpr::Ref(c) => buf.push_str(&c.to_string()),
        FormulaExpr::Range(r) => buf.push_str(&r.to_string()),
        FormulaExpr::ErrorLiteral(e) => buf.push_str(e.token()),
        FormulaExpr::Unary(op, inner) => match op {
            UnaryOp::Neg => {
                let need_paren = min_prec > 11;
                if need_paren {
                    buf.push('(');
                }
                buf.push('-');
                write_expr(inner, buf, 11, false);
                if need_paren {
                    buf.push(')');
                }
            }
            UnaryOp::Plus => {
                buf.push('+');
                write_expr(inner, buf, 11, false);
            }
            UnaryOp::Percent => {
                write_expr(inner, buf, 12, false);
                buf.push('%');
            }
        },
        FormulaExpr::Binary(op, lhs, rhs) => {
            let (l_bp, r_bp, sym, prec) = bin_info(*op);
            // ^ es right-assoc: en `a^b^c` el rhs es `b^c` (parse
            // como child con r_bp más bajo). Para render, necesito
            // saber si lhs/rhs requieren paréntesis comparando con bp.
            let need_paren = prec < min_prec
                || (prec == min_prec && is_right && !is_right_assoc(*op));
            if need_paren {
                buf.push('(');
            }
            write_expr(lhs, buf, l_bp, false);
            buf.push_str(sym);
            write_expr(rhs, buf, r_bp, true);
            if need_paren {
                buf.push(')');
            }
        }
        FormulaExpr::Call(name, args) => {
            buf.push_str(name);
            buf.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    buf.push_str(", ");
                }
                write_expr(a, buf, 0, false);
            }
            buf.push(')');
        }
    }
}

/// (l_bp, r_bp, símbolo, prec del operador). l_bp / r_bp son los
/// binding powers que `parse_expr` usa al descender; el `prec` es la
/// precedencia visible para decidir paréntesis en render (= l_bp).
fn bin_info(op: BinaryOp) -> (u8, u8, &'static str, u8) {
    match op {
        BinaryOp::Eq => (1, 2, "=", 1),
        BinaryOp::Ne => (1, 2, "<>", 1),
        BinaryOp::Lt => (1, 2, "<", 1),
        BinaryOp::Le => (1, 2, "<=", 1),
        BinaryOp::Gt => (1, 2, ">", 1),
        BinaryOp::Ge => (1, 2, ">=", 1),
        BinaryOp::Concat => (3, 4, "&", 3),
        BinaryOp::Add => (5, 6, "+", 5),
        BinaryOp::Sub => (5, 6, "-", 5),
        BinaryOp::Mul => (7, 8, "*", 7),
        BinaryOp::Div => (7, 8, "/", 7),
        BinaryOp::Pow => (10, 9, "^", 10),
    }
}

fn is_right_assoc(op: BinaryOp) -> bool {
    matches!(op, BinaryOp::Pow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formula::compile;

    fn roundtrip(src: &str) {
        let expr = compile(src).unwrap();
        let rendered = render(&expr);
        let reparsed = compile(&rendered).unwrap_or_else(|e| {
            panic!("render produjo `{rendered}` que NO re-parsea: {e}")
        });
        assert_eq!(
            expr, reparsed,
            "round-trip diverge: src=`{src}` rendered=`{rendered}`"
        );
    }

    #[test]
    fn simple_arithmetic_round_trip() {
        roundtrip("1+2*3");
        roundtrip("(1+2)*3");
        roundtrip("2^3^2");
    }

    #[test]
    fn refs_and_ranges_round_trip() {
        roundtrip("A1+B2");
        roundtrip("$A$1+A$1+$A1+A1");
        roundtrip("SUM(A1:B10)");
    }

    #[test]
    fn strings_with_quotes_round_trip() {
        roundtrip(r#"="he said ""hi""""#);
    }

    #[test]
    fn unicode_round_trip() {
        roundtrip(r#"=CONCAT("café", "ñandú")"#);
    }

    #[test]
    fn errors_round_trip() {
        // El motor de fill emite estos literales; deben parsear de vuelta.
        roundtrip("=#REF!");
        roundtrip("=#REF!+1");
        roundtrip("=IFERROR(A1, #N/A)");
    }

    #[test]
    fn unary_minus_with_pow_preserves_excel_order() {
        // En Excel `-2^4` = `(-2)^4` = 16. El parser ya lo agrupa así
        // (bp prefijo 11 > pow 10); render debe reproducir lo mismo.
        let expr = compile("-2^4").unwrap();
        let rendered = render(&expr);
        // No exigimos paréntesis literales, sólo que reparse = expr.
        let re = compile(&rendered).unwrap();
        assert_eq!(expr, re);
    }
}
