//! Emisión de los statements del PROCEDURE: cada [`Stmt`] se traduce a
//! una o varias líneas de código Rust sobre `charka-runtime`.

use charka_ir::{Operand, Perform, PerformControl, PerformTarget, Stmt};

use crate::emit::Emitter;
use crate::expr::{
    emit_cond, emit_expr, figurative_fill, operand_decimal, operand_display, operand_str,
};
use crate::sym::{paragraph_method, FieldKind, Symbols};

/// Emite un statement.
pub(crate) fn emit_stmt(em: &mut Emitter, sym: &Symbols, stmt: &Stmt) {
    match stmt {
        Stmt::Move { from, to } => emit_move(em, sym, from, to),
        Stmt::Display { items } => emit_display(em, sym, items),
        Stmt::Accept { into } => {
            em.line(&format!(
                "// charka: ACCEPT {into} — entrada interactiva no soportada en v1"
            ));
        }
        Stmt::Compute {
            targets,
            rounded,
            expr,
        } => {
            let value = emit_expr(sym, expr);
            for t in targets {
                emit_store(em, sym, t, &value, *rounded);
            }
        }
        Stmt::Add {
            addends,
            to,
            giving,
            rounded,
        } => emit_add(em, sym, addends, to, giving, *rounded),
        Stmt::Subtract {
            amounts,
            from,
            giving,
            rounded,
        } => emit_subtract(em, sym, amounts, from, giving, *rounded),
        Stmt::Multiply {
            left,
            by,
            giving,
            rounded,
        } => emit_multiply(em, sym, left, by, giving, *rounded),
        Stmt::Divide {
            left,
            right,
            by_form,
            giving,
            rounded,
        } => emit_divide(em, sym, left, right, *by_form, giving, *rounded),
        Stmt::If {
            cond,
            then_branch,
            else_branch,
        } => {
            em.line(&format!("if {} {{", emit_cond(sym, cond)));
            em.indent();
            emit_block(em, sym, then_branch);
            em.dedent();
            if else_branch.is_empty() {
                em.line("}");
            } else {
                em.line("} else {");
                em.indent();
                emit_block(em, sym, else_branch);
                em.dedent();
                em.line("}");
            }
        }
        Stmt::Perform(p) => emit_perform(em, sym, p),
        Stmt::GoTo { target } => {
            em.line(&format!(
                "self.{}(); return; // charka: GO TO (aproximado)",
                paragraph_method(target)
            ));
        }
        Stmt::StopRun | Stmt::Goback => em.line("std::process::exit(0);"),
        Stmt::Exit => em.line("return;"),
        Stmt::Continue => em.line("// CONTINUE"),
        Stmt::Unknown { verb, .. } => {
            em.line(&format!("// charka: verbo no transpilado — {verb}"));
        }
    }
}

/// Emite una secuencia de statements (un cuerpo de bloque).
fn emit_block(em: &mut Emitter, sym: &Symbols, stmts: &[Stmt]) {
    for s in stmts {
        emit_stmt(em, sym, s);
    }
}

/// Almacena un valor `Decimal` (texto de expresión) en un campo.
fn emit_store(em: &mut Emitter, sym: &Symbols, name: &str, value: &str, rounded: bool) {
    match sym.lookup(name) {
        Some(f) => match f.kind {
            FieldKind::Num { .. } => {
                let method = if rounded { "store_rounded" } else { "store" };
                em.line(&format!("self.{}.{method}({value});", f.ident));
            }
            FieldKind::Text { .. } => {
                em.line(&format!(
                    "self.{}.store(({value}).to_string().as_str());",
                    f.ident
                ));
            }
        },
        None => em.line(&format!("// charka: destino no resuelto — {name}")),
    }
}

fn emit_move(em: &mut Emitter, sym: &Symbols, from: &Operand, to: &[String]) {
    for t in to {
        match sym.lookup(t) {
            Some(f) => match f.kind {
                FieldKind::Num { .. } => {
                    em.line(&format!(
                        "self.{}.store({});",
                        f.ident,
                        operand_decimal(sym, from)
                    ));
                }
                FieldKind::Text { .. } => {
                    if let Operand::Figurative(fig) = from {
                        em.line(&format!(
                            "self.{}.fill('{}');",
                            f.ident,
                            figurative_fill(*fig)
                        ));
                    } else {
                        em.line(&format!(
                            "self.{}.store({});",
                            f.ident,
                            operand_str(sym, from)
                        ));
                    }
                }
            },
            None => em.line(&format!("// charka: destino MOVE no resuelto — {t}")),
        }
    }
}

fn emit_display(em: &mut Emitter, sym: &Symbols, items: &[Operand]) {
    if items.is_empty() {
        em.line("println!();");
        return;
    }
    let placeholders = "{}".repeat(items.len());
    let args: Vec<String> = items.iter().map(|o| operand_display(sym, o)).collect();
    em.line(&format!(
        "println!(\"{placeholders}\", {});",
        args.join(", ")
    ));
}

/// La suma de una lista de operandos, encadenando `.add`.
fn fold_sum(sym: &Symbols, ops: &[Operand]) -> String {
    let mut it = ops.iter();
    let Some(first) = it.next() else {
        return "Decimal::zero()".to_string();
    };
    let mut acc = operand_decimal(sym, first);
    for o in it {
        acc = format!("({acc}).add(&({}))", operand_decimal(sym, o));
    }
    acc
}

fn emit_add(
    em: &mut Emitter,
    sym: &Symbols,
    addends: &[Operand],
    to: &[String],
    giving: &[String],
    rounded: bool,
) {
    let sum = fold_sum(sym, addends);
    if !giving.is_empty() {
        let base = match to.first() {
            Some(first) => format!(
                "({sum}).add(&({}))",
                operand_decimal(sym, &Operand::Data(first.clone()))
            ),
            None => sum,
        };
        for g in giving {
            emit_store(em, sym, g, &base, rounded);
        }
    } else {
        for t in to {
            emit_inplace(em, sym, t, "add", &sum, rounded);
        }
    }
}

fn emit_subtract(
    em: &mut Emitter,
    sym: &Symbols,
    amounts: &[Operand],
    from: &[String],
    giving: &[String],
    rounded: bool,
) {
    let sum = fold_sum(sym, amounts);
    if !giving.is_empty() {
        let minuend = from
            .first()
            .map(|f| operand_decimal(sym, &Operand::Data(f.clone())))
            .unwrap_or_else(|| "Decimal::zero()".to_string());
        let value = format!("({minuend}).sub(&({sum}))");
        for g in giving {
            emit_store(em, sym, g, &value, rounded);
        }
    } else {
        for t in from {
            emit_inplace(em, sym, t, "sub", &sum, rounded);
        }
    }
}

fn emit_multiply(
    em: &mut Emitter,
    sym: &Symbols,
    left: &Operand,
    by: &Operand,
    giving: &[String],
    rounded: bool,
) {
    let l = operand_decimal(sym, left);
    if !giving.is_empty() {
        let value = format!("({l}).mul(&({}))", operand_decimal(sym, by));
        for g in giving {
            emit_store(em, sym, g, &value, rounded);
        }
    } else if let Operand::Data(name) = by {
        // `MULTIPLY a BY b` sin GIVING: b queda con a*b.
        emit_inplace(em, sym, name, "mul", &l, rounded);
    } else {
        em.line("// charka: MULTIPLY sin destino claro");
    }
}

fn emit_divide(
    em: &mut Emitter,
    sym: &Symbols,
    left: &Operand,
    right: &Operand,
    by_form: bool,
    giving: &[String],
    rounded: bool,
) {
    // `a BY b` → a/b; `a INTO b` → b/a.
    let (num, den) = if by_form {
        (operand_decimal(sym, left), operand_decimal(sym, right))
    } else {
        (operand_decimal(sym, right), operand_decimal(sym, left))
    };
    if !giving.is_empty() {
        for g in giving {
            let value = format!(
                "({num}).div(&({den}), {}, Rounding::Truncate).unwrap_or_else(|_| Decimal::zero())",
                target_scale(sym, g)
            );
            emit_store(em, sym, g, &value, rounded);
        }
    } else if let Operand::Data(name) = right {
        // `DIVIDE a INTO b` sin GIVING: b queda con b/a.
        let value = format!(
            "({num}).div(&({den}), {}, Rounding::Truncate).unwrap_or_else(|_| Decimal::zero())",
            target_scale(sym, name)
        );
        emit_store(em, sym, name, &value, rounded);
    } else {
        em.line("// charka: DIVIDE sin destino claro");
    }
}

/// Emite una operación aritmética en el lugar: `t = t <op> rhs`.
fn emit_inplace(em: &mut Emitter, sym: &Symbols, name: &str, op: &str, rhs: &str, rounded: bool) {
    match sym.lookup(name) {
        Some(f) if matches!(f.kind, FieldKind::Num { .. }) => {
            let method = if rounded { "store_rounded" } else { "store" };
            em.line(&format!(
                "self.{0}.{method}(self.{0}.value().{op}(&({rhs})));",
                f.ident
            ));
        }
        _ => em.line(&format!(
            "// charka: destino aritmético no resuelto — {name}"
        )),
    }
}

/// La escala de redondeo de un destino numérico (sus dígitos
/// fraccionarios), o 4 por defecto.
fn target_scale(sym: &Symbols, name: &str) -> u8 {
    match sym.lookup(name).map(|f| &f.kind) {
        Some(FieldKind::Num { frac, .. }) => *frac,
        _ => 4,
    }
}

/// Una expresión `usize` para el número de repeticiones de un `PERFORM`.
fn count_expr(sym: &Symbols, op: &Operand) -> String {
    match op {
        Operand::Num(n) => match n.trim_start_matches('+').parse::<i128>() {
            Ok(v) if v >= 0 => format!("{v}usize"),
            _ => "0usize".to_string(),
        },
        _ => format!(
            "(({}).rescale(0, Rounding::Truncate).mantissa().max(0) as usize)",
            operand_decimal(sym, op)
        ),
    }
}

fn emit_perform(em: &mut Emitter, sym: &Symbols, p: &Perform) {
    // Emite el "cuerpo": la llamada al párrafo o el bloque en línea.
    let emit_body = |em: &mut Emitter, sym: &Symbols| match &p.target {
        PerformTarget::Paragraph { name, thru } => {
            let note = thru
                .as_ref()
                .map(|t| format!(" // charka: THRU {t} — rango no soportado"))
                .unwrap_or_default();
            em.line(&format!("self.{}();{note}", paragraph_method(name)));
        }
        PerformTarget::Inline(body) => emit_block(em, sym, body),
    };

    match &p.control {
        PerformControl::Once => {
            if matches!(p.target, PerformTarget::Inline(_)) {
                em.line("{");
                em.indent();
                emit_body(em, sym);
                em.dedent();
                em.line("}");
            } else {
                emit_body(em, sym);
            }
        }
        PerformControl::Times(n) => {
            em.line(&format!("for _ in 0..{} {{", count_expr(sym, n)));
            em.indent();
            emit_body(em, sym);
            em.dedent();
            em.line("}");
        }
        PerformControl::Until(cond) => {
            em.line(&format!("while !({}) {{", emit_cond(sym, cond)));
            em.indent();
            emit_body(em, sym);
            em.dedent();
            em.line("}");
        }
        PerformControl::Varying {
            var,
            from,
            by,
            until,
        } => {
            // var = from; mientras no se cumpla `until`: cuerpo; var += by.
            emit_store(em, sym, var, &operand_decimal(sym, from), false);
            em.line(&format!("while !({}) {{", emit_cond(sym, until)));
            em.indent();
            emit_body(em, sym);
            emit_inplace(em, sym, var, "add", &operand_decimal(sym, by), false);
            em.dedent();
            em.line("}");
        }
    }
}
