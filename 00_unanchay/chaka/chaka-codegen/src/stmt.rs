//! Emisión de los statements del PROCEDURE: cada [`Stmt`] se traduce a
//! una o varias líneas de código Rust sobre `chaka-runtime`.

use chaka_ir::{
    CmpOp, Cond, FileMode, InspectOp, Operand, Perform, PerformControl, PerformTarget,
    SearchBranch, Stmt, WhenBranch, WhenTest,
};

use crate::emit::Emitter;
use crate::expr::{
    emit_cond, emit_expr, field_ref, figurative_fill, operand_decimal, operand_display,
    operand_str, rust_str,
};
use crate::sym::{paragraph_method, FieldKind, Symbols};

/// Emite un statement.
pub(crate) fn emit_stmt(em: &mut Emitter, sym: &Symbols, stmt: &Stmt) {
    match stmt {
        Stmt::Move { from, to } => emit_move(em, sym, from, to),
        Stmt::Display { items } => emit_display(em, sym, items),
        Stmt::Accept { .. } => {
            em.line("// chaka: ACCEPT — entrada interactiva no soportada en v1");
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
        Stmt::SetTrue { conditions } => emit_set_true(em, sym, conditions),
        Stmt::SetTo { targets, value } => {
            for t in targets {
                let target = Operand::Data(t.to_uppercase());
                emit_move(em, sym, value, std::slice::from_ref(&target));
            }
        }
        Stmt::SetAdjust { targets, by, up } => {
            let delta = operand_decimal(sym, by);
            for t in targets {
                let target = Operand::Data(t.to_uppercase());
                let op = if *up { "add" } else { "sub" };
                emit_inplace(em, sym, &target, op, &delta, false);
            }
        }
        Stmt::Open { mode, files } => emit_open(em, sym, *mode, files),
        Stmt::Close { files } => emit_close(em, sym, files),
        Stmt::Read {
            file,
            key,
            at_end,
            not_at_end,
            invalid_key,
            not_invalid_key,
        } => emit_read(
            em,
            sym,
            file,
            key.as_deref(),
            at_end,
            not_at_end,
            invalid_key,
            not_invalid_key,
        ),
        Stmt::Write {
            record,
            from,
            invalid_key,
            not_invalid_key,
        } => emit_write(em, sym, record, from.as_ref(), invalid_key, not_invalid_key),
        Stmt::Perform(p) => emit_perform(em, sym, p),
        Stmt::Search {
            table,
            varying,
            at_end,
            whens,
        } => emit_search(em, sym, table, varying, at_end, whens),
        Stmt::Sort { using, giving, .. } => emit_sort_or_merge(em, sym, using, giving, true),
        Stmt::Merge { using, giving, .. } => emit_sort_or_merge(em, sym, using, giving, true),
        Stmt::Rewrite {
            record,
            from,
            invalid_key,
            not_invalid_key,
        } => emit_rewrite(em, sym, record, from.as_ref(), invalid_key, not_invalid_key),
        Stmt::Delete {
            file,
            invalid_key,
            not_invalid_key,
        } => emit_delete(em, sym, file, invalid_key, not_invalid_key),
        Stmt::Start {
            file,
            key,
            cmp,
            invalid_key,
            not_invalid_key,
        } => emit_start(em, sym, file, key.as_deref(), *cmp, invalid_key, not_invalid_key),
        Stmt::Call {
            program,
            on_overflow,
            not_on_overflow,
            ..
        } => emit_call(em, sym, program, on_overflow, not_on_overflow),
        Stmt::GoTo { target } => {
            em.line(&format!(
                "self.{}(); return; // chaka: GO TO (aproximado)",
                paragraph_method(target)
            ));
        }
        Stmt::StopRun | Stmt::Goback => em.line("std::process::exit(0);"),
        Stmt::Exit => em.line("return;"),
        Stmt::Continue => em.line("// CONTINUE"),
        Stmt::Unknown { verb, .. } => {
            em.line(&format!("// chaka: verbo no transpilado — {verb}"));
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
        None => em.line("// chaka: destino no resuelto"),
    }
}

fn emit_move(em: &mut Emitter, sym: &Symbols, from: &Operand, to: &[Operand]) {
    for t in to {
        // Un destino con PICTURE de edición formatea el valor numérico.
        if let Operand::Data(name) = t {
            if let Some(pic) = sym.lookup(name).and_then(|f| f.edit.clone()) {
                let ident = sym
                    .lookup(name)
                    .map(|f| f.ident.clone())
                    .unwrap_or_default();
                em.line(&format!(
                    "self.{ident}.store(&format_edited({}, {}));",
                    operand_decimal(sym, from),
                    rust_str(&pic)
                ));
                continue;
            }
        }
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
            None => em.line("// chaka: destino MOVE no resuelto"),
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
        _ => em.line("// chaka: destino aritmético no resuelto"),
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
        None => em.line("// chaka: destino no resuelto"),
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

/// `INSPECT` — cuenta (`TALLYING`) o reemplaza (`REPLACING`/`CONVERTING`).
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
                _ => em.line("// chaka: contador INSPECT no resuelto"),
            }
            em.dedent();
            em.line("}");
        }
        InspectOp::TallyingForLeading { counter, search } => {
            em.line("{");
            em.indent();
            em.line(&format!(
                "let __hay = ({}).to_string();",
                operand_display(sym, target)
            ));
            em.line(&format!(
                "let __needle = ({}).to_string();",
                operand_display(sym, search)
            ));
            em.line("let mut __n: i128 = 0;");
            em.line("if !__needle.is_empty() {");
            em.indent();
            em.line("let mut __rest = __hay.as_str();");
            em.line("while __rest.starts_with(__needle.as_str()) {");
            em.indent();
            em.line("__n += 1;");
            em.line("__rest = &__rest[__needle.len()..];");
            em.dedent();
            em.line("}");
            em.dedent();
            em.line("}");
            match field_ref(sym, counter) {
                Some((lref, FieldKind::Num { .. })) => em.line(&format!(
                    "{lref}.store({lref}.value().add(&Decimal::from_integer(__n)));"
                )),
                _ => em.line("// chaka: contador INSPECT no resuelto"),
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
        InspectOp::Converting { from, to } => {
            em.line("{");
            em.indent();
            em.line(&format!(
                "let __from: Vec<char> = ({}).chars().collect();",
                operand_display(sym, from)
            ));
            em.line(&format!(
                "let __to: Vec<char> = ({}).chars().collect();",
                operand_display(sym, to)
            ));
            em.line(&format!(
                "let __conv: String = ({}).chars().map(|c| match __from.iter().position(|&f| f == c) {{ Some(i) => __to.get(i).copied().unwrap_or(c), None => c }}).collect();",
                operand_display(sym, target)
            ));
            emit_store_text(em, sym, target, "__conv.as_str()");
            em.dedent();
            em.line("}");
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

/// `OPEN {INPUT|OUTPUT} files...`
fn emit_open(em: &mut Emitter, sym: &Symbols, mode: FileMode, files: &[String]) {
    let method = match mode {
        FileMode::Input => "open_input",
        FileMode::Output => "open_output",
        FileMode::IO => "open_io",
        FileMode::Extend => "open_extend",
    };
    for f in files {
        match sym.file(f) {
            Some(fs) => em.line(&format!("self.{}.{method}();", fs.ident)),
            None => em.line("// chaka: OPEN de fichero no resuelto"),
        }
    }
}

/// `CLOSE files...`
fn emit_close(em: &mut Emitter, sym: &Symbols, files: &[String]) {
    for f in files {
        match sym.file(f) {
            Some(fs) => em.line(&format!("self.{}.close();", fs.ident)),
            None => em.line("// chaka: CLOSE de fichero no resuelto"),
        }
    }
}

/// `READ file [AT END ...] [NOT AT END ...]` — lee la línea siguiente
/// en el registro del fichero.
/// `READ file ...`. Sobre un fichero por clave con ramas `INVALID KEY`,
/// clave explícita o acceso aleatorio, es una lectura directa
/// (`read_keyed`); en cualquier otro caso es secuencial (`read`).
#[allow(clippy::too_many_arguments)]
fn emit_read(
    em: &mut Emitter,
    sym: &Symbols,
    file: &str,
    key: Option<&str>,
    at_end: &[Stmt],
    not_at_end: &[Stmt],
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    let Some(fs) = sym.file(file) else {
        em.line("// chaka: READ de fichero no resuelto");
        return;
    };
    let random = fs.is_keyed()
        && (key.is_some()
            || !invalid_key.is_empty()
            || !not_invalid_key.is_empty()
            || fs.access == chaka_ir::AccessMode::Random);
    if random {
        let key_expr = key_value(sym, fs, key).unwrap_or_else(|| "String::new()".to_string());
        em.line(&format!(
            "match self.{}.read_keyed(&{key_expr}) {{",
            fs.ident
        ));
        em.indent();
        em.line("Some(__line) => {");
        em.indent();
        emit_store_record(em, sym, &fs.record, "__line.as_str()");
        emit_block(em, sym, not_invalid_key);
        em.dedent();
        em.line("}");
        em.line("None => {");
        em.indent();
        emit_block(em, sym, invalid_key);
        em.dedent();
        em.line("}");
        em.dedent();
        em.line("}");
        return;
    }
    // Lectura secuencial (line-sequential, o `READ NEXT` de un keyed).
    em.line(&format!("match self.{}.read() {{", fs.ident));
    em.indent();
    em.line("Some(__line) => {");
    em.indent();
    emit_store_record(em, sym, &fs.record, "__line.as_str()");
    emit_block(em, sym, not_at_end);
    em.dedent();
    em.line("}");
    em.line("None => {");
    em.indent();
    emit_block(em, sym, at_end);
    em.dedent();
    em.line("}");
    em.dedent();
    em.line("}");
}

/// `WRITE record [FROM from] [INVALID KEY ...]`. Sobre un fichero por
/// clave usa `write_keyed` (que devuelve `false` ante clave duplicada,
/// disparando `INVALID KEY`); sobre line-sequential, un `write` simple.
fn emit_write(
    em: &mut Emitter,
    sym: &Symbols,
    record: &str,
    from: Option<&Operand>,
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    if let Some(src) = from {
        let value = operand_str(sym, src);
        emit_store_record(em, sym, record, &value);
    }
    let (Some(fs), Some(val)) = (sym.file_of_record(record), record_value(sym, record)) else {
        em.line("// chaka: WRITE de registro no resuelto");
        return;
    };
    if fs.is_keyed() {
        let key_expr = key_value(sym, fs, None).unwrap_or_else(|| "String::new()".to_string());
        emit_keyed_result(
            em,
            sym,
            &format!("self.{}.write_keyed(&{key_expr}, &{val})", fs.ident),
            invalid_key,
            not_invalid_key,
        );
    } else {
        em.line(&format!("self.{}.write(&{val});", fs.ident));
        emit_block(em, sym, not_invalid_key);
    }
}

/// `REWRITE record [FROM from] [INVALID KEY ...]`. Por clave usa
/// `rewrite_keyed` (`false` si la clave no existe); en line-sequential
/// se comporta como un `WRITE` y dispara siempre `NOT INVALID KEY`.
fn emit_rewrite(
    em: &mut Emitter,
    sym: &Symbols,
    record: &str,
    from: Option<&Operand>,
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    if let Some(src) = from {
        let value = operand_str(sym, src);
        emit_store_record(em, sym, record, &value);
    }
    let (Some(fs), Some(val)) = (sym.file_of_record(record), record_value(sym, record)) else {
        em.line("// chaka: REWRITE de registro no resuelto");
        return;
    };
    if fs.is_keyed() {
        let key_expr = key_value(sym, fs, None).unwrap_or_else(|| "String::new()".to_string());
        emit_keyed_result(
            em,
            sym,
            &format!("self.{}.rewrite_keyed(&{key_expr}, &{val})", fs.ident),
            invalid_key,
            not_invalid_key,
        );
    } else {
        em.line(&format!("self.{}.write(&{val});", fs.ident));
        emit_block(em, sym, not_invalid_key);
    }
}

/// `DELETE file [INVALID KEY ...]`. Por clave usa `delete_keyed`
/// (`false` si la clave no existe); en line-sequential es un no-op que
/// dispara `NOT INVALID KEY`.
fn emit_delete(
    em: &mut Emitter,
    sym: &Symbols,
    file: &str,
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    let Some(fs) = sym.file(file) else {
        em.line("// chaka: DELETE de fichero no resuelto");
        return;
    };
    if fs.is_keyed() {
        let key_expr = key_value(sym, fs, None).unwrap_or_else(|| "String::new()".to_string());
        emit_keyed_result(
            em,
            sym,
            &format!("self.{}.delete_keyed(&{key_expr})", fs.ident),
            invalid_key,
            not_invalid_key,
        );
    } else {
        em.line("// chaka: DELETE — no-op en line-sequential");
        emit_block(em, sym, not_invalid_key);
    }
}

/// `START file [KEY op k] [INVALID KEY ...]`. Por clave posiciona el
/// cursor con `start` (`false` si no hay registro que satisfaga); en
/// line-sequential es un no-op que dispara `NOT INVALID KEY`.
fn emit_start(
    em: &mut Emitter,
    sym: &Symbols,
    file: &str,
    key: Option<&str>,
    cmp: chaka_ir::StartCmp,
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    let Some(fs) = sym.file(file) else {
        em.line("// chaka: START de fichero no resuelto");
        return;
    };
    if fs.is_keyed() {
        let key_expr = key_value(sym, fs, key).unwrap_or_else(|| "String::new()".to_string());
        let cmp_variant = match cmp {
            chaka_ir::StartCmp::Eq => "Eq",
            chaka_ir::StartCmp::Gt => "Gt",
            chaka_ir::StartCmp::Ge => "Ge",
            chaka_ir::StartCmp::Lt => "Lt",
            chaka_ir::StartCmp::Le => "Le",
        };
        emit_keyed_result(
            em,
            sym,
            &format!(
                "self.{}.start(&{key_expr}, StartCmp::{cmp_variant})",
                fs.ident
            ),
            invalid_key,
            not_invalid_key,
        );
    } else {
        em.line("// chaka: START — no-op (acceso secuencial)");
        emit_block(em, sym, not_invalid_key);
    }
}

/// Emite `if <cond> { not_invalid_key } else { invalid_key }` para una
/// operación por clave que devuelve `bool` (true = éxito).
fn emit_keyed_result(
    em: &mut Emitter,
    sym: &Symbols,
    cond: &str,
    invalid_key: &[Stmt],
    not_invalid_key: &[Stmt],
) {
    em.line(&format!("if {cond} {{"));
    em.indent();
    emit_block(em, sym, not_invalid_key);
    em.dedent();
    em.line("} else {");
    em.indent();
    emit_block(em, sym, invalid_key);
    em.dedent();
    em.line("}");
}

/// La expresión Rust (`String`) con el valor de la clave de un fichero:
/// el `display()` del campo `KEY` explícito, o de la `RECORD`/`RELATIVE
/// KEY` por defecto. `None` si no resuelve a ningún campo.
fn key_value(sym: &Symbols, fs: &crate::sym::FileSym, explicit: Option<&str>) -> Option<String> {
    let name = explicit.or_else(|| fs.key_name())?;
    sym.lookup(name).map(|f| format!("self.{}.display()", f.ident))
}

/// El ancho en caracteres del registro que ocupa un campo elemental —
/// su tamaño dentro de un registro de longitud fija. Para un numérico
/// es el total de dígitos de la PICTURE (sin signo de presentación).
fn field_width(kind: &FieldKind) -> usize {
    match kind {
        FieldKind::Text { len } => *len,
        FieldKind::Num { int, frac, .. } => *int as usize + *frac as usize,
    }
}

/// Una expresión Rust de tipo `String` con el valor completo de un
/// registro: el `display()` del campo si es elemental, o la
/// concatenación del `display()` de sus miembros si es un grupo. `None`
/// si el nombre no resuelve a ningún dato.
fn record_value(sym: &Symbols, record: &str) -> Option<String> {
    if let Some(f) = sym.lookup(record) {
        return Some(format!("self.{}.display()", f.ident));
    }
    let members = sym.group(record)?;
    let parts: Vec<String> = members
        .iter()
        .filter_map(|m| sym.lookup(m))
        .map(|f| format!("self.{}.display()", f.ident))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let fmt = "{}".repeat(parts.len());
    Some(format!("format!(\"{fmt}\", {})", parts.join(", ")))
}

/// Distribuye el texto `value` (una expresión Rust `&str`) en el
/// registro `record`: lo asigna entero si es elemental, o lo trocea por
/// el ancho de cada miembro si es un grupo (un registro de longitud
/// fija). El bloque se emite con su propio scope.
fn emit_store_record(em: &mut Emitter, sym: &Symbols, record: &str, value: &str) {
    if let Some(f) = sym.lookup(record) {
        emit_store_slice(em, &f.ident, f.kind, value);
        return;
    }
    let Some(members) = sym.group(record) else {
        em.line("// chaka: registro no resuelto");
        return;
    };
    let fields: Vec<(String, FieldKind)> = members
        .iter()
        .filter_map(|m| sym.lookup(m).map(|f| (f.ident.clone(), f.kind)))
        .collect();
    if fields.is_empty() {
        em.line("// chaka: registro de grupo vacío");
        return;
    }
    let total: usize = fields.iter().map(|(_, k)| field_width(k)).sum();
    em.line("{");
    em.indent();
    em.line(&format!("let mut __rec: Vec<char> = ({value}).chars().collect();"));
    em.line(&format!("while __rec.len() < {total} {{ __rec.push(' '); }}"));
    let mut off = 0usize;
    for (ident, kind) in &fields {
        let w = field_width(kind);
        em.line(&format!(
            "let __f: String = __rec[{off}..{}].iter().collect();",
            off + w
        ));
        emit_store_slice(em, ident, *kind, "__f.as_str()");
        off += w;
    }
    em.dedent();
    em.line("}");
}

/// Asigna `value` (expresión `&str`) a un campo elemental, parseándolo a
/// `Decimal` si el campo es numérico.
fn emit_store_slice(em: &mut Emitter, ident: &str, kind: FieldKind, value: &str) {
    match kind {
        FieldKind::Text { .. } => em.line(&format!("self.{ident}.store({value});")),
        FieldKind::Num { .. } => em.line(&format!(
            "self.{ident}.store(Decimal::parse(({value}).trim()).unwrap_or_else(|_| Decimal::zero()));"
        )),
    }
}

/// `SET cond... TO TRUE` — asigna a cada dato padre el valor que hace
/// verdadero su nombre de condición (nivel 88).
fn emit_set_true(em: &mut Emitter, sym: &Symbols, conditions: &[String]) {
    for name in conditions {
        match sym.condition(name) {
            Some(cn) => {
                let target = Operand::Data(cn.parent.clone());
                let value = cn.value.clone();
                emit_move(em, sym, &value, std::slice::from_ref(&target));
            }
            None => em.line(&format!("// chaka: condición 88 no resuelta — {name}")),
        }
    }
}

/// Resetea un campo completo (escalar o tabla entera).
fn emit_reset(em: &mut Emitter, sym: &Symbols, name: &str) {
    let Some(f) = sym.lookup(name) else {
        em.line(&format!("// chaka: INITIALIZE de {name} no resuelto"));
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
        None => em.line("// chaka: INITIALIZE no resuelto"),
    }
}

/// `SORT` y `MERGE` — lee todas las líneas de los `using`, las ordena
/// si toca, y las vuelca a cada `giving`. Las claves `ON KEY` no se
/// honran en la v1: se ordena por línea completa.
fn emit_sort_or_merge(
    em: &mut Emitter,
    sym: &Symbols,
    using: &[String],
    giving: &[String],
    do_sort: bool,
) {
    em.line("{");
    em.indent();
    em.line("let mut __lines: Vec<String> = Vec::new();");
    for name in using {
        match sym.file(name) {
            Some(fs) => {
                em.line(&format!("self.{}.open_input();", fs.ident));
                em.line(&format!("while let Some(__l) = self.{}.read() {{", fs.ident));
                em.indent();
                em.line("__lines.push(__l);");
                em.dedent();
                em.line("}");
                em.line(&format!("self.{}.close();", fs.ident));
            }
            None => em.line(&format!("// chaka: SORT/MERGE — USING {name} no resuelto")),
        }
    }
    if do_sort {
        em.line("__lines.sort();");
    }
    for name in giving {
        match sym.file(name) {
            Some(fs) => {
                em.line(&format!("self.{}.open_output();", fs.ident));
                em.line("for __l in &__lines {");
                em.indent();
                em.line(&format!("self.{}.write(__l);", fs.ident));
                em.dedent();
                em.line("}");
                em.line(&format!("self.{}.close();", fs.ident));
            }
            None => em.line(&format!("// chaka: SORT/MERGE — GIVING {name} no resuelto")),
        }
    }
    em.dedent();
    em.line("}");
}

/// `SEARCH` — búsqueda lineal: incrementa `varying` hasta agotar la
/// tabla (`OCCURS n`), evalúa cada `WHEN` en cada vuelta. Si ninguna
/// dispara, ejecuta `AT END`.
fn emit_search(
    em: &mut Emitter,
    sym: &Symbols,
    table: &str,
    varying: &str,
    at_end: &[Stmt],
    whens: &[SearchBranch],
) {
    let var_op = Operand::Data(varying.to_uppercase());
    let limit = sym
        .lookup(table)
        .and_then(|f| f.occurs)
        .unwrap_or(0);
    if varying.is_empty() || limit == 0 {
        em.line("// chaka: SEARCH sin VARYING o tabla sin OCCURS — corre AT END");
        emit_block(em, sym, at_end);
        return;
    }
    em.line("'search: loop {");
    em.indent();
    em.line(&format!(
        "let __idx = {}.mantissa();",
        operand_decimal(sym, &var_op)
    ));
    em.line(&format!("if __idx < 1 || __idx > {limit}i128 {{"));
    em.indent();
    emit_block(em, sym, at_end);
    em.line("break 'search;");
    em.dedent();
    em.line("}");
    for branch in whens {
        em.line(&format!("if {} {{", emit_cond(sym, &branch.cond)));
        em.indent();
        emit_block(em, sym, &branch.body);
        em.line("break 'search;");
        em.dedent();
        em.line("}");
    }
    emit_inplace(em, sym, &var_op, "add", "Decimal::from_integer(1)", false);
    em.dedent();
    em.line("}");
}

/// `CALL` — aproximación v1: si el nombre del sub-programa es un
/// literal y coincide con un párrafo del mismo programa, lo invoca y
/// emite la rama `NOT ON OVERFLOW`; en otro caso, emite la rama
/// `ON OVERFLOW` (sub-programa externo no soportado en la v1).
fn emit_call(
    em: &mut Emitter,
    sym: &Symbols,
    program: &Operand,
    on_overflow: &[Stmt],
    not_on_overflow: &[Stmt],
) {
    let resolved = match program {
        Operand::Str(s) => sym
            .paragraphs
            .iter()
            .find(|(cobol, _)| cobol == &s.to_uppercase())
            .map(|(_, m)| m.clone()),
        _ => None,
    };
    match resolved {
        Some(method) => {
            em.line(&format!("self.{method}();"));
            if !not_on_overflow.is_empty() {
                emit_block(em, sym, not_on_overflow);
            }
        }
        None => {
            em.line("// chaka: CALL — sub-programa externo no resuelto en la v1");
            if !on_overflow.is_empty() {
                emit_block(em, sym, on_overflow);
            }
        }
    }
}

fn emit_perform(em: &mut Emitter, sym: &Symbols, p: &Perform) {
    // Emite el "cuerpo": la llamada al párrafo o el bloque en línea.
    let emit_body = |em: &mut Emitter, sym: &Symbols| match &p.target {
        PerformTarget::Paragraph { name, thru } => {
            for m in sym.paragraph_range(name, thru.as_deref()) {
                em.line(&format!("self.{m}();"));
            }
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
