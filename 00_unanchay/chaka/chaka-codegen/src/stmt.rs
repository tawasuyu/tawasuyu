//! Emisión de los statements del PROCEDURE: cada [`Stmt`] se traduce a
//! una o varias líneas de código Rust sobre `charka-runtime`.

use charka_ir::{
    CmpOp, Cond, InspectOp, Operand, Perform, PerformControl, PerformTarget, Stmt, WhenBranch,
    WhenTest,
};

use crate::emit::Emitter;
use crate::expr::{
    emit_cond, emit_expr, field_ref, figurative_fill, operand_decimal, operand_display, operand_str,
};
use crate::sym::{paragraph_method, FieldKind, Symbols};

/// Emite un statement.
pub(crate) fn emit_stmt(em: &mut Emitter, sym: &Symbols, stmt: &Stmt) {
    match stmt {
        Stmt::Move { from, to } => emit_move(em, sym, from, to),
        Stmt::Display { items } => emit_display(em, sym, items),
        Stmt::Accept { .. } => {
            em.line("// charka: ACCEPT — entrada interactiva no soportada en v1");
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
        Stmt::Evaluate {
            subject,
            whens,
            other,
        } => emit_evaluate(em, sym, subject, whens, other),
        Stmt::StringConcat { sources, into } => emit_string(em, sym, sources, into),
        Stmt::Unstring {
            source,
            delimiter,
            into,
        } => emit_unstring(em, sym, source, delimiter, into),
        Stmt::Inspect { target, op } => emit_inspect(em, sym, target, op),
        Stmt::Initialize { targets } => emit_initialize(em, sym, targets),
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

/// Almacena un valor `Decimal` (texto de expresión) en un destino —
/// un dato escalar o un elemento de tabla.
fn emit_store(em: &mut Emitter, sym: &Symbols, target: &Operand, value: &str, rounded: bool) {
    match field_ref(sym, target) {
        Some((lref, FieldKind::Num { .. })) => {
            let method = if rounded { "store_rounded" } else { "store" };
            em.line(&format!("{lref}.{method}({value});"));
        }
        Some((lref, FieldKind::Text { .. })) => {
            em.line(&format!("{lref}.store(({value}).to_string().as_str());"));
        }
        None => em.line("// charka: destino no resuelto"),
    }
}

fn emit_move(em: &mut Emitter, sym: &Symbols, from: &Operand, to: &[Operand]) {
    for t in to {
        match field_ref(sym, t) {
            Some((lref, FieldKind::Num { .. })) => {
                em.line(&format!("{lref}.store({});", operand_decimal(sym, from)));
            }
            Some((lref, FieldKind::Text { .. })) => {
                if let Operand::Figurative(fig) = from {
                    em.line(&format!("{lref}.fill('{}');", figurative_fill(*fig)));
                } else {
                    em.line(&format!("{lref}.store({});", operand_str(sym, from)));
                }
            }
            None => em.line("// charka: destino MOVE no resuelto"),
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
    to: &[Operand],
    giving: &[Operand],
    rounded: bool,
) {
    let sum = fold_sum(sym, addends);
    if !giving.is_empty() {
        let base = match to.first() {
            Some(first) => format!("({sum}).add(&({}))", operand_decimal(sym, first)),
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
    from: &[Operand],
    giving: &[Operand],
    rounded: bool,
) {
    let sum = fold_sum(sym, amounts);
    if !giving.is_empty() {
        let minuend = from
            .first()
            .map(|f| operand_decimal(sym, f))
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
    giving: &[Operand],
    rounded: bool,
) {
    let l = operand_decimal(sym, left);
    if giving.is_empty() {
        // `MULTIPLY a BY b` sin GIVING: b queda con a*b.
        emit_inplace(em, sym, by, "mul", &l, rounded);
    } else {
        let value = format!("({l}).mul(&({}))", operand_decimal(sym, by));
        for g in giving {
            emit_store(em, sym, g, &value, rounded);
        }
    }
}

fn emit_divide(
    em: &mut Emitter,
    sym: &Symbols,
    left: &Operand,
    right: &Operand,
    by_form: bool,
    giving: &[Operand],
    rounded: bool,
) {
    // `a BY b` → a/b; `a INTO b` → b/a.
    let (num, den) = if by_form {
        (operand_decimal(sym, left), operand_decimal(sym, right))
    } else {
        (operand_decimal(sym, right), operand_decimal(sym, left))
    };
    let div = |scale: u8| {
        format!(
            "({num}).div(&({den}), {scale}, Rounding::Truncate).unwrap_or_else(|_| Decimal::zero())"
        )
    };
    if giving.is_empty() {
        // `DIVIDE a INTO b` sin GIVING: b queda con b/a.
        let value = div(target_scale(sym, right));
        emit_store(em, sym, right, &value, rounded);
    } else {
        for g in giving {
            let value = div(target_scale(sym, g));
            emit_store(em, sym, g, &value, rounded);
        }
    }
}

/// Emite una operación aritmética en el lugar: `target = target <op> rhs`.
fn emit_inplace(
    em: &mut Emitter,
    sym: &Symbols,
    target: &Operand,
    op: &str,
    rhs: &str,
    rounded: bool,
) {
    match field_ref(sym, target) {
        Some((lref, FieldKind::Num { .. })) => {
            let method = if rounded { "store_rounded" } else { "store" };
            em.line(&format!("{lref}.{method}({lref}.value().{op}(&({rhs})));"));
        }
        _ => em.line("// charka: destino aritmético no resuelto"),
    }
}

/// La escala de redondeo de un destino numérico (sus dígitos
/// fraccionarios), o 4 por defecto.
fn target_scale(sym: &Symbols, op: &Operand) -> u8 {
    match field_ref(sym, op).map(|(_, k)| k) {
        Some(FieldKind::Num { frac, .. }) => frac,
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

/// Emite un `EVALUATE` como una cadena `if / else if / else`.
fn emit_evaluate(
    em: &mut Emitter,
    sym: &Symbols,
    subject: &Operand,
    whens: &[WhenBranch],
    other: &[Stmt],
) {
    if whens.is_empty() {
        if !other.is_empty() {
            em.line("{");
            em.indent();
            emit_block(em, sym, other);
            em.dedent();
            em.line("}");
        }
        return;
    }
    for (i, branch) in whens.iter().enumerate() {
        let cond = branch_condition(sym, subject, branch);
        if i == 0 {
            em.line(&format!("if {cond} {{"));
        } else {
            em.line(&format!("}} else if {cond} {{"));
        }
        em.indent();
        emit_block(em, sym, &branch.body);
        em.dedent();
    }
    if other.is_empty() {
        em.line("}");
    } else {
        em.line("} else {");
        em.indent();
        emit_block(em, sym, other);
        em.dedent();
        em.line("}");
    }
}

/// La condición de una rama `WHEN`: pasa si **alguna** de sus pruebas
/// se cumple.
fn branch_condition(sym: &Symbols, subject: &Operand, branch: &WhenBranch) -> String {
    if branch.tests.is_empty() {
        return "false".to_string();
    }
    branch
        .tests
        .iter()
        .map(|t| format!("({})", test_condition(sym, subject, t)))
        .collect::<Vec<_>>()
        .join(" || ")
}

/// Traduce una prueba `WHEN` a una expresión Rust de tipo `bool`.
fn test_condition(sym: &Symbols, subject: &Operand, test: &WhenTest) -> String {
    let compare = |op: CmpOp, rhs: &Operand| {
        emit_cond(
            sym,
            &Cond::Compare {
                lhs: subject.clone(),
                op,
                rhs: rhs.clone(),
            },
        )
    };
    match test {
        WhenTest::Value(v) => compare(CmpOp::Eq, v),
        WhenTest::Range(lo, hi) => {
            format!(
                "({}) && ({})",
                compare(CmpOp::Ge, lo),
                compare(CmpOp::Le, hi)
            )
        }
        WhenTest::Cond(cond) => emit_cond(sym, cond),
    }
}

/// Almacena una expresión `&str` en un destino: directo si es de
/// texto, parseado a `Decimal` si es numérico.
fn emit_store_text(em: &mut Emitter, sym: &Symbols, target: &Operand, text: &str) {
    match field_ref(sym, target) {
        Some((lref, FieldKind::Text { .. })) => {
            em.line(&format!("{lref}.store({text});"));
        }
        Some((lref, FieldKind::Num { .. })) => {
            em.line(&format!(
                "{lref}.store(Decimal::parse(({text}).trim())\
                 .unwrap_or_else(|_| Decimal::zero()));"
            ));
        }
        None => em.line("// charka: destino no resuelto"),
    }
}

/// `STRING` — concatena el texto de las fuentes en el destino.
fn emit_string(em: &mut Emitter, sym: &Symbols, sources: &[Operand], into: &Operand) {
    let fmt = "{}".repeat(sources.len());
    let args: Vec<String> = sources.iter().map(|s| operand_display(sym, s)).collect();
    let concat = format!("&format!(\"{fmt}\", {})", args.join(", "));
    emit_store_text(em, sym, into, &concat);
}

/// `UNSTRING` — parte el texto de la fuente y reparte los trozos.
fn emit_unstring(
    em: &mut Emitter,
    sym: &Symbols,
    source: &Operand,
    delimiter: &Operand,
    into: &[Operand],
) {
    em.line("{");
    em.indent();
    em.line(&format!(
        "let __src = ({}).to_string();",
        operand_display(sym, source)
    ));
    em.line(&format!(
        "let __delim = ({}).to_string();",
        operand_display(sym, delimiter)
    ));
    em.line("let mut __it = __src.split(__delim.as_str());");
    for t in into {
        emit_store_text(em, sym, t, "__it.next().unwrap_or(\"\")");
    }
    em.dedent();
    em.line("}");
}

/// `INSPECT` — cuenta (`TALLYING`) o reemplaza (`REPLACING`).
fn emit_inspect(em: &mut Emitter, sym: &Symbols, target: &Operand, op: &InspectOp) {
    match op {
        InspectOp::TallyingForAll { counter, search } => {
            em.line("{");
            em.indent();
            em.line(&format!(
                "let __n = ({}).matches({}).count() as i128;",
                operand_display(sym, target),
                operand_str(sym, search)
            ));
            match field_ref(sym, counter) {
                Some((lref, FieldKind::Num { .. })) => em.line(&format!(
                    "{lref}.store({lref}.value().add(&Decimal::from_integer(__n)));"
                )),
                _ => em.line("// charka: contador INSPECT no resuelto"),
            }
            em.dedent();
            em.line("}");
        }
        InspectOp::ReplacingAll { from, to } => {
            let replaced = format!(
                "({}).replace({}, {})",
                operand_display(sym, target),
                operand_str(sym, from),
                operand_str(sym, to)
            );
            emit_store_text(em, sym, target, &format!("{replaced}.as_str()"));
        }
    }
}

/// `INITIALIZE` — pone cada destino (o los miembros de un grupo) en su
/// valor por defecto.
fn emit_initialize(em: &mut Emitter, sym: &Symbols, targets: &[Operand]) {
    for t in targets {
        match t {
            Operand::Data(name) => match sym.group(name) {
                Some(members) => {
                    for m in members {
                        emit_reset(em, sym, m);
                    }
                }
                None => emit_reset(em, sym, name),
            },
            Operand::Indexed { .. } => emit_reset_element(em, sym, t),
            _ => {}
        }
    }
}

/// Resetea un campo completo (escalar o tabla entera).
fn emit_reset(em: &mut Emitter, sym: &Symbols, name: &str) {
    let Some(f) = sym.lookup(name) else {
        em.line(&format!("// charka: INITIALIZE de {name} no resuelto"));
        return;
    };
    let reset = match f.kind {
        FieldKind::Num { .. } => "store(Decimal::zero())",
        FieldKind::Text { .. } => "fill(' ')",
    };
    match f.occurs {
        None => em.line(&format!("self.{}.{reset};", f.ident)),
        Some(_) => {
            em.line(&format!("for __e in self.{}.iter_mut() {{", f.ident));
            em.indent();
            em.line(&format!("__e.{reset};"));
            em.dedent();
            em.line("}");
        }
    }
}

/// Resetea un solo elemento de tabla (`INITIALIZE ELEM(I)`).
fn emit_reset_element(em: &mut Emitter, sym: &Symbols, op: &Operand) {
    match field_ref(sym, op) {
        Some((lref, FieldKind::Num { .. })) => em.line(&format!("{lref}.store(Decimal::zero());")),
        Some((lref, FieldKind::Text { .. })) => em.line(&format!("{lref}.fill(' ');")),
        None => em.line("// charka: INITIALIZE no resuelto"),
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
            let var_op = Operand::Data(var.clone());
            emit_store(em, sym, &var_op, &operand_decimal(sym, from), false);
            em.line(&format!("while !({}) {{", emit_cond(sym, until)));
            em.indent();
            emit_body(em, sym);
            emit_inplace(em, sym, &var_op, "add", &operand_decimal(sym, by), false);
            em.dedent();
            em.line("}");
        }
    }
}
