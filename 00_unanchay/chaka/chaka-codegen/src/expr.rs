//! Emisión de expresiones y condiciones: cada nodo del IR se convierte
//! en un fragmento de código Rust (un `String`).

use charka_ir::{BinOp, CmpOp, Cond, Expr, Figurative, Operand};

use crate::sym::{FieldKind, Symbols};

/// Un literal de texto Rust con las comillas y los escapes adecuados.
pub(crate) fn rust_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// El texto que representa una constante figurativa.
pub(crate) fn figurative_text(f: Figurative) -> &'static str {
    match f {
        Figurative::Zero => "0",
        Figurative::Space => " ",
        Figurative::Quote => "\"",
        Figurative::HighValue | Figurative::LowValue | Figurative::Null => "",
    }
}

/// El carácter de relleno de una figurativa, para `Text::fill`.
pub(crate) fn figurative_fill(f: Figurative) -> char {
    match f {
        Figurative::Zero => '0',
        Figurative::Quote => '"',
        _ => ' ',
    }
}

/// Un operando como expresión de tipo `Decimal`.
pub(crate) fn operand_decimal(sym: &Symbols, op: &Operand) -> String {
    match op {
        Operand::Num(n) => format!("dec({})", rust_str(n)),
        Operand::Str(s) => format!(
            "Decimal::parse({}).unwrap_or_else(|_| Decimal::zero())",
            rust_str(s)
        ),
        Operand::Figurative(_) => "Decimal::zero()".to_string(),
        Operand::Data(name) => match sym.lookup(name) {
            Some(f) => match f.kind {
                FieldKind::Num { .. } => format!("self.{}.value()", f.ident),
                FieldKind::Text { .. } => format!(
                    "Decimal::parse(self.{}.display().trim()).unwrap_or_else(|_| Decimal::zero())",
                    f.ident
                ),
            },
            None => format!("Decimal::zero() /* charka: dato no resuelto {name} */"),
        },
    }
}

/// Un operando como expresión de tipo `&str` (para texto).
pub(crate) fn operand_str(sym: &Symbols, op: &Operand) -> String {
    match op {
        Operand::Str(s) => rust_str(s),
        Operand::Num(n) => rust_str(n),
        Operand::Figurative(f) => rust_str(figurative_text(*f)),
        Operand::Data(name) => match sym.lookup(name) {
            Some(f) => format!("self.{}.display().as_str()", f.ident),
            None => format!("\"\" /* charka: dato no resuelto {name} */"),
        },
    }
}

/// Un operando como expresión que implementa `Display` (para `DISPLAY`).
pub(crate) fn operand_display(sym: &Symbols, op: &Operand) -> String {
    match op {
        Operand::Str(s) => rust_str(s),
        Operand::Num(n) => rust_str(n),
        Operand::Figurative(f) => rust_str(figurative_text(*f)),
        Operand::Data(name) => match sym.lookup(name) {
            Some(f) => format!("self.{}.display()", f.ident),
            None => format!("\"\" /* charka: dato no resuelto {name} */"),
        },
    }
}

/// Emite una expresión aritmética como código Rust de tipo `Decimal`.
pub(crate) fn emit_expr(sym: &Symbols, e: &Expr) -> String {
    match e {
        Expr::Operand(op) => operand_decimal(sym, op),
        Expr::Neg(inner) => format!("Decimal::zero().sub(&({}))", emit_expr(sym, inner)),
        Expr::Binary { op, lhs, rhs } => {
            let l = emit_expr(sym, lhs);
            let r = emit_expr(sym, rhs);
            match op {
                BinOp::Add => format!("({l}).add(&({r}))"),
                BinOp::Sub => format!("({l}).sub(&({r}))"),
                BinOp::Mul => format!("({l}).mul(&({r}))"),
                BinOp::Div => format!(
                    "({l}).div(&({r}), 9, Rounding::Truncate).unwrap_or_else(|_| Decimal::zero())"
                ),
                BinOp::Pow => "Decimal::zero() /* charka: ** no soportado */".to_string(),
            }
        }
    }
}

/// Emite una condición como código Rust de tipo `bool`.
pub(crate) fn emit_cond(sym: &Symbols, c: &Cond) -> String {
    match c {
        Cond::Compare { lhs, op, rhs } => emit_compare(sym, lhs, *op, rhs),
        Cond::Named(name) => {
            format!("false /* charka: condición 88 no soportada: {name} */")
        }
        Cond::Not(inner) => format!("!({})", emit_cond(sym, inner)),
        Cond::And(a, b) => format!("({}) && ({})", emit_cond(sym, a), emit_cond(sym, b)),
        Cond::Or(a, b) => format!("({}) || ({})", emit_cond(sym, a), emit_cond(sym, b)),
    }
}

/// Emite una comparación: numérica si ambos lados lo son, alfanumérica
/// si alguno es texto.
fn emit_compare(sym: &Symbols, lhs: &Operand, op: CmpOp, rhs: &Operand) -> String {
    if is_text_operand(sym, lhs) || is_text_operand(sym, rhs) {
        let method = match op {
            CmpOp::Eq => "is_eq",
            CmpOp::Ne => "is_ne",
            CmpOp::Lt => "is_lt",
            CmpOp::Gt => "is_gt",
            CmpOp::Le => "is_le",
            CmpOp::Ge => "is_ge",
        };
        format!(
            "cobol_text_cmp({}, {}).{method}()",
            operand_str(sym, lhs),
            operand_str(sym, rhs)
        )
    } else {
        let rust_op = match op {
            CmpOp::Eq => "==",
            CmpOp::Ne => "!=",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
        };
        format!(
            "({}) {rust_op} ({})",
            operand_decimal(sym, lhs),
            operand_decimal(sym, rhs)
        )
    }
}

/// ¿El operando es alfanumérico?
fn is_text_operand(sym: &Symbols, op: &Operand) -> bool {
    match op {
        Operand::Str(_) => true,
        Operand::Data(name) => matches!(
            sym.lookup(name).map(|f| &f.kind),
            Some(FieldKind::Text { .. })
        ),
        _ => false,
    }
}
