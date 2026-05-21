//! Parseo de los statements del PROCEDURE division. COBOL no separa
//! statements con un símbolo: cada uno termina donde empieza el verbo
//! del siguiente, por eso las listas de operandos se cortan al ver una
//! palabra "frontera" (ver [`crate::kw`]).

use charka_parser::TokenKind;

use crate::ast::{InspectOp, Operand, Perform, PerformControl, PerformTarget, Stmt, WhenBranch};
use crate::cursor::{parse_operand, Cursor};
use crate::expr::{parse_cond, parse_expr};
use crate::kw::{is_boundary, is_terminator, is_verb};

/// Parsea statements hasta agotar los tokens o toparse con una palabra
/// de `stops` (los terminadores del bloque que llama).
pub(crate) fn parse_statements(c: &mut Cursor, stops: &[&str]) -> Vec<Stmt> {
    let mut out = Vec::new();
    while !c.done() {
        if let Some(w) = c.peek_word() {
            if stops.contains(&w.as_str()) {
                break;
            }
        }
        out.push(parse_one_stmt(c, stops));
    }
    out
}

/// Parsea un statement: despacha por el verbo. Todo parser consume al
/// menos un token, así que el bucle de [`parse_statements`] progresa.
fn parse_one_stmt(c: &mut Cursor, stops: &[&str]) -> Stmt {
    match c.peek_word().unwrap_or_default().as_str() {
        "MOVE" => parse_move(c),
        "DISPLAY" => parse_display(c),
        "ACCEPT" => parse_accept(c),
        "COMPUTE" => parse_compute(c),
        "ADD" => parse_add(c),
        "SUBTRACT" => parse_subtract(c),
        "MULTIPLY" => parse_multiply(c),
        "DIVIDE" => parse_divide(c),
        "IF" => parse_if(c),
        "EVALUATE" => parse_evaluate(c),
        "STRING" => parse_string(c),
        "UNSTRING" => parse_unstring(c),
        "INSPECT" => parse_inspect(c),
        "PERFORM" => parse_perform(c),
        "GO" => parse_goto(c),
        "STOP" => parse_stop(c),
        "GOBACK" => {
            c.bump();
            Stmt::Goback
        }
        "EXIT" => parse_exit(c),
        "CONTINUE" => {
            c.bump();
            Stmt::Continue
        }
        _ => parse_unknown(c, stops),
    }
}

// ── Listas ────────────────────────────────────────────────────────

/// Lee una lista de destinos de dato (separados por comas opcionales),
/// hasta una palabra frontera. Cada destino puede llevar subíndice de
/// tabla. Consume las apariciones de `ROUNDED`.
fn parse_targets(c: &mut Cursor, rounded: &mut bool) -> Vec<Operand> {
    let mut targets = Vec::new();
    loop {
        c.eat_sym(",");
        if c.eat_word("ROUNDED") {
            *rounded = true;
            continue;
        }
        match c.peek_word() {
            Some(w) if !is_boundary(&w) => targets.push(parse_operand(c)),
            _ => break,
        }
    }
    targets
}

/// Lee una lista de operandos hasta una palabra frontera.
fn parse_operand_list(c: &mut Cursor) -> Vec<Operand> {
    let mut ops = Vec::new();
    loop {
        c.eat_sym(",");
        if c.done() {
            break;
        }
        if let Some(w) = c.peek_word() {
            if is_boundary(&w) {
                break;
            }
        }
        let is_start = matches!(
            c.peek().map(|t| t.kind),
            Some(TokenKind::Number | TokenKind::String | TokenKind::Word)
        ) || c.at_sym("-")
            || c.at_sym("+");
        if !is_start {
            break;
        }
        ops.push(parse_operand(c));
    }
    ops
}

/// Salta los tokens de una cláusula que la v1 no modela, hasta el
/// siguiente verbo o terminador de ámbito.
fn skip_to_stmt_boundary(c: &mut Cursor) {
    while !c.done() {
        if let Some(w) = c.peek_word() {
            if is_verb(&w) || is_terminator(&w) {
                break;
            }
        }
        c.bump();
    }
}

/// ¿El token actual puede iniciar un operando?
fn is_operand_start(c: &Cursor) -> bool {
    match c.peek().map(|t| t.kind) {
        Some(TokenKind::Number | TokenKind::String) => true,
        Some(TokenKind::Word) => c.peek_word().map(|w| !is_boundary(&w)).unwrap_or(false),
        Some(TokenKind::Symbol) => c.at_sym("-") || c.at_sym("+"),
        _ => false,
    }
}

/// Lee un único nombre de dato, si lo hay y no es una palabra frontera.
fn parse_one_name(c: &mut Cursor) -> Option<String> {
    match c.peek_word() {
        Some(w) if !is_boundary(&w) => {
            c.bump();
            Some(w)
        }
        _ => None,
    }
}

// ── Statements ────────────────────────────────────────────────────

fn parse_move(c: &mut Cursor) -> Stmt {
    c.bump(); // MOVE
    c.eat_word("CORRESPONDING");
    c.eat_word("CORR");
    let from = parse_operand(c);
    c.eat_word("TO");
    let mut rounded = false;
    let to = parse_targets(c, &mut rounded);
    Stmt::Move { from, to }
}

fn parse_display(c: &mut Cursor) -> Stmt {
    c.bump(); // DISPLAY
    let items = parse_operand_list(c);
    skip_to_stmt_boundary(c); // p. ej. `WITH NO ADVANCING`, `UPON ...`
    Stmt::Display { items }
}

fn parse_accept(c: &mut Cursor) -> Stmt {
    c.bump(); // ACCEPT
    let into = if c.peek_word().map(|w| !is_boundary(&w)).unwrap_or(false) {
        parse_operand(c)
    } else {
        Operand::Data(String::new())
    };
    skip_to_stmt_boundary(c); // p. ej. `FROM DATE`
    Stmt::Accept { into }
}

fn parse_compute(c: &mut Cursor) -> Stmt {
    c.bump(); // COMPUTE
    let mut rounded = false;
    let targets = parse_targets(c, &mut rounded);
    if !c.eat_sym("=") {
        c.eat_word("EQUAL");
    }
    let expr = parse_expr(c);
    c.eat_word("END-COMPUTE");
    Stmt::Compute {
        targets,
        rounded,
        expr,
    }
}

fn parse_add(c: &mut Cursor) -> Stmt {
    c.bump(); // ADD
    c.eat_word("CORRESPONDING");
    c.eat_word("CORR");
    let addends = parse_operand_list(c);
    let mut rounded = false;
    let mut to = Vec::new();
    let mut giving = Vec::new();
    if c.eat_word("TO") {
        to = parse_targets(c, &mut rounded);
    }
    if c.eat_word("GIVING") {
        giving = parse_targets(c, &mut rounded);
    }
    c.eat_word("END-ADD");
    Stmt::Add {
        addends,
        to,
        giving,
        rounded,
    }
}

fn parse_subtract(c: &mut Cursor) -> Stmt {
    c.bump(); // SUBTRACT
    c.eat_word("CORRESPONDING");
    c.eat_word("CORR");
    let amounts = parse_operand_list(c);
    let mut rounded = false;
    let mut from = Vec::new();
    let mut giving = Vec::new();
    if c.eat_word("FROM") {
        from = parse_targets(c, &mut rounded);
    }
    if c.eat_word("GIVING") {
        giving = parse_targets(c, &mut rounded);
    }
    c.eat_word("END-SUBTRACT");
    Stmt::Subtract {
        amounts,
        from,
        giving,
        rounded,
    }
}

fn parse_multiply(c: &mut Cursor) -> Stmt {
    c.bump(); // MULTIPLY
    let left = parse_operand(c);
    c.eat_word("BY");
    let by = parse_operand(c);
    let mut rounded = false;
    let mut giving = Vec::new();
    if c.eat_word("GIVING") {
        giving = parse_targets(c, &mut rounded);
    } else if c.eat_word("ROUNDED") {
        rounded = true;
    }
    c.eat_word("END-MULTIPLY");
    Stmt::Multiply {
        left,
        by,
        giving,
        rounded,
    }
}

fn parse_divide(c: &mut Cursor) -> Stmt {
    c.bump(); // DIVIDE
    let left = parse_operand(c);
    let by_form = if c.eat_word("BY") {
        true
    } else {
        c.eat_word("INTO");
        false
    };
    let right = parse_operand(c);
    let mut rounded = false;
    let mut giving = Vec::new();
    if c.eat_word("GIVING") {
        giving = parse_targets(c, &mut rounded);
    } else if c.eat_word("ROUNDED") {
        rounded = true;
    }
    if c.eat_word("REMAINDER") {
        let _ = parse_targets(c, &mut rounded);
    }
    c.eat_word("END-DIVIDE");
    Stmt::Divide {
        left,
        right,
        by_form,
        giving,
        rounded,
    }
}

fn parse_if(c: &mut Cursor) -> Stmt {
    c.bump(); // IF
    let cond = parse_cond(c);
    c.eat_word("THEN");
    let then_branch = parse_statements(c, &["ELSE", "END-IF"]);
    let else_branch = if c.eat_word("ELSE") {
        parse_statements(c, &["END-IF"])
    } else {
        Vec::new()
    };
    c.eat_word("END-IF");
    Stmt::If {
        cond,
        then_branch,
        else_branch,
    }
}

fn parse_evaluate(c: &mut Cursor) -> Stmt {
    c.bump(); // EVALUATE
    let subject = parse_operand(c);
    let mut whens = Vec::new();
    let mut other = Vec::new();
    while !c.done() && !c.at_word("END-EVALUATE") {
        if !c.at_word("WHEN") {
            break; // algo inesperado dentro del EVALUATE: se corta
        }
        // Varios `WHEN` apilados comparten el mismo cuerpo.
        let mut values = Vec::new();
        let mut is_other = false;
        while c.eat_word("WHEN") {
            if c.eat_word("OTHER") {
                is_other = true;
            } else {
                values.push(parse_operand(c));
            }
        }
        let body = parse_statements(c, &["WHEN", "END-EVALUATE"]);
        if is_other {
            other = body;
        } else {
            whens.push(WhenBranch { values, body });
        }
    }
    c.eat_word("END-EVALUATE");
    Stmt::Evaluate {
        subject,
        whens,
        other,
    }
}

fn parse_string(c: &mut Cursor) -> Stmt {
    c.bump(); // STRING
    let mut sources = Vec::new();
    while !c.done() && !c.at_word("INTO") && !c.at_word("END-STRING") {
        if c.eat_word("DELIMITED") {
            c.eat_word("BY");
            if !c.eat_word("SIZE") {
                let _ = parse_operand(c); // delimitador: la v1 lo ignora
            }
        } else if is_operand_start(c) {
            sources.push(parse_operand(c));
        } else {
            break;
        }
    }
    c.eat_word("INTO");
    let into = parse_operand(c);
    skip_to_stmt_boundary(c); // p. ej. `WITH POINTER`, `ON OVERFLOW`
    c.eat_word("END-STRING");
    Stmt::StringConcat { sources, into }
}

fn parse_unstring(c: &mut Cursor) -> Stmt {
    c.bump(); // UNSTRING
    let source = parse_operand(c);
    let delimiter = if c.eat_word("DELIMITED") {
        c.eat_word("BY");
        c.eat_word("ALL");
        parse_operand(c)
    } else {
        Operand::Str(" ".to_string())
    };
    c.eat_word("INTO");
    let mut rounded = false;
    let into = parse_targets(c, &mut rounded);
    skip_to_stmt_boundary(c); // p. ej. `DELIMITER IN`, `COUNT IN`
    c.eat_word("END-UNSTRING");
    Stmt::Unstring {
        source,
        delimiter,
        into,
    }
}

fn parse_inspect(c: &mut Cursor) -> Stmt {
    c.bump(); // INSPECT
    let target = parse_operand(c);
    if c.eat_word("TALLYING") {
        let counter = parse_operand(c);
        c.eat_word("FOR");
        c.eat_word("ALL");
        let search = parse_operand(c);
        skip_to_stmt_boundary(c);
        Stmt::Inspect {
            target,
            op: InspectOp::TallyingForAll { counter, search },
        }
    } else if c.eat_word("REPLACING") {
        c.eat_word("ALL");
        let from = parse_operand(c);
        c.eat_word("BY");
        let to = parse_operand(c);
        skip_to_stmt_boundary(c);
        Stmt::Inspect {
            target,
            op: InspectOp::ReplacingAll { from, to },
        }
    } else {
        // Forma de INSPECT que la v1 no modela.
        skip_to_stmt_boundary(c);
        Stmt::Unknown {
            verb: "INSPECT".to_string(),
            tokens: Vec::new(),
        }
    }
}

fn parse_perform(c: &mut Cursor) -> Stmt {
    c.bump(); // PERFORM

    // `PERFORM VARYING ... ... END-PERFORM` — cuerpo en línea.
    if c.eat_word("VARYING") {
        let control = parse_varying(c);
        let body = parse_statements(c, &["END-PERFORM"]);
        c.eat_word("END-PERFORM");
        return inline_perform(body, control);
    }

    // `PERFORM UNTIL cond ... END-PERFORM` — cuerpo en línea.
    if c.eat_word("UNTIL") {
        let cond = parse_cond(c);
        let body = parse_statements(c, &["END-PERFORM"]);
        c.eat_word("END-PERFORM");
        return inline_perform(body, PerformControl::Until(cond));
    }

    // `PERFORM n TIMES ... END-PERFORM` — cuerpo en línea.
    if matches!(c.peek().map(|t| t.kind), Some(TokenKind::Number)) {
        let n = parse_operand(c);
        c.eat_word("TIMES");
        let body = parse_statements(c, &["END-PERFORM"]);
        c.eat_word("END-PERFORM");
        return inline_perform(body, PerformControl::Times(n));
    }

    // `PERFORM ... END-PERFORM` — cuerpo en línea, una vez.
    if c.peek_word().map(|w| is_verb(&w)).unwrap_or(false) {
        let body = parse_statements(c, &["END-PERFORM"]);
        c.eat_word("END-PERFORM");
        return inline_perform(body, PerformControl::Once);
    }

    // `PERFORM PARA [THRU PARA2] [VARYING ... | n TIMES | UNTIL cond]`.
    let Some(name) = parse_one_name(c) else {
        // Forma no reconocida tras `PERFORM`: perform vacío.
        return inline_perform(Vec::new(), PerformControl::Once);
    };
    let thru = if c.eat_word("THRU") || c.eat_word("THROUGH") {
        parse_one_name(c)
    } else {
        None
    };
    let control = if c.eat_word("VARYING") {
        parse_varying(c)
    } else if c.eat_word("UNTIL") {
        PerformControl::Until(parse_cond(c))
    } else if at_count(c) {
        let n = parse_operand(c);
        c.eat_word("TIMES");
        PerformControl::Times(n)
    } else {
        PerformControl::Once
    };
    Stmt::Perform(Perform {
        target: PerformTarget::Paragraph { name, thru },
        control,
    })
}

/// Arma un `PERFORM` con cuerpo en línea.
fn inline_perform(body: Vec<Stmt>, control: PerformControl) -> Stmt {
    Stmt::Perform(Perform {
        target: PerformTarget::Inline(body),
        control,
    })
}

/// Parsea la cláusula `VARYING var FROM x BY y UNTIL cond`, ya
/// consumida la palabra `VARYING`.
fn parse_varying(c: &mut Cursor) -> PerformControl {
    let var = parse_one_name(c).unwrap_or_default();
    c.eat_word("FROM");
    let from = parse_operand(c);
    c.eat_word("BY");
    let by = parse_operand(c);
    c.eat_word("UNTIL");
    let until = parse_cond(c);
    PerformControl::Varying {
        var,
        from,
        by,
        until,
    }
}

/// ¿El cursor está sobre `<operando> TIMES`?
fn at_count(c: &Cursor) -> bool {
    match c.peek().map(|t| t.kind) {
        Some(TokenKind::Number) => true,
        Some(TokenKind::Word) => {
            let w = c.peek_word().unwrap_or_default();
            !is_boundary(&w) && c.word_at(1).as_deref() == Some("TIMES")
        }
        _ => false,
    }
}

fn parse_goto(c: &mut Cursor) -> Stmt {
    c.bump(); // GO
    c.eat_word("TO");
    Stmt::GoTo {
        target: parse_one_name(c).unwrap_or_default(),
    }
}

fn parse_stop(c: &mut Cursor) -> Stmt {
    c.bump(); // STOP
    c.eat_word("RUN");
    Stmt::StopRun
}

fn parse_exit(c: &mut Cursor) -> Stmt {
    c.bump(); // EXIT
    c.eat_word("PROGRAM");
    c.eat_word("PARAGRAPH");
    c.eat_word("PERFORM");
    c.eat_word("SECTION");
    Stmt::Exit
}

/// Verbo no soportado: conserva el verbo y sus tokens hasta el próximo
/// statement (otro verbo), terminador de ámbito o tope del bloque.
fn parse_unknown(c: &mut Cursor, stops: &[&str]) -> Stmt {
    let verb = c.peek_word().unwrap_or_default();
    let mut tokens = Vec::new();
    if let Some(t) = c.bump() {
        tokens.push(t);
    }
    while !c.done() {
        if let Some(w) = c.peek_word() {
            if stops.contains(&w.as_str()) || is_verb(&w) || is_terminator(&w) {
                break;
            }
        }
        if let Some(t) = c.bump() {
            tokens.push(t);
        }
    }
    Stmt::Unknown { verb, tokens }
}
