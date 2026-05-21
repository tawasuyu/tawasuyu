//! El intérprete del IR: la ejecución «sombra» del programa COBOL.
//!
//! Ejecuta el [`Ir`] directamente sobre los tipos de `charka-runtime`,
//! sin compilar nada. Es una segunda ruta de ejecución, independiente
//! del código que emite `charka-codegen` — eso es lo que lo hace un
//! validador: si el intérprete y el transpilado divergen, hay un bug.

use std::collections::HashMap;

use charka_ir::{
    BinOp, CmpOp, Cond, ConditionName, Expr, Figurative, InspectOp, Ir, Operand, Perform,
    PerformControl, PerformTarget, Stmt,
};
use charka_runtime::{cobol_text_cmp, Decimal, Rounding};

use crate::field::{build_fields, Cell};

/// Tope de pasos: corta los bucles que no terminan (un `PERFORM UNTIL`
/// con una condición que nunca se cumple) en vez de colgarse.
const STEP_BUDGET: u64 = 5_000_000;

/// Escala intermedia de la división dentro de una expresión.
const DIV_SCALE: u8 = 9;

/// El resultado de ejecutar un statement: cómo sigue el control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flow {
    /// Sigue con el statement siguiente.
    Normal,
    /// Sale del párrafo actual (`EXIT`).
    Exit,
    /// Termina el programa (`STOP RUN`, `GOBACK`, tope de pasos).
    Stop,
}

/// La máquina sombra: el estado y el motor de ejecución.
pub(crate) struct Machine<'a> {
    ir: &'a Ir,
    fields: HashMap<String, Cell>,
    para_index: HashMap<String, usize>,
    conditions: HashMap<String, ConditionName>,
    pub output: Vec<String>,
    budget: u64,
    pub step_limit_hit: bool,
    pub stopped: bool,
}

impl<'a> Machine<'a> {
    /// Prepara la máquina: aplana los datos e indexa los párrafos.
    pub(crate) fn new(ir: &'a Ir) -> Self {
        let mut para_index = HashMap::new();
        for (i, proc) in ir.procedures.iter().enumerate() {
            para_index.entry(proc.name.to_uppercase()).or_insert(i);
        }
        let conditions = ir
            .model
            .conditions
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        Self {
            ir,
            fields: build_fields(&ir.model),
            para_index,
            conditions,
            output: Vec::new(),
            budget: STEP_BUDGET,
            step_limit_hit: false,
            stopped: false,
        }
    }

    /// Corre el programa: encadena los párrafos en orden (el «caer» de
    /// COBOL) hasta un `STOP RUN` o el final.
    pub(crate) fn run(&mut self) {
        let ir = self.ir;
        for i in 0..ir.procedures.len() {
            if let Flow::Stop = self.exec_block(&ir.procedures[i].body) {
                self.stopped = true;
                break;
            }
        }
    }

    // ── Ejecución ─────────────────────────────────────────────────

    /// Consume un paso del presupuesto. `true` si se agotó.
    fn tick(&mut self) -> bool {
        if self.budget == 0 {
            self.step_limit_hit = true;
            return true;
        }
        self.budget -= 1;
        false
    }

    fn exec_block(&mut self, stmts: &'a [Stmt]) -> Flow {
        for s in stmts {
            match self.exec_stmt(s) {
                Flow::Normal => {}
                other => return other,
            }
        }
        Flow::Normal
    }

    fn exec_stmt(&mut self, stmt: &'a Stmt) -> Flow {
        if self.tick() {
            return Flow::Stop;
        }
        match stmt {
            Stmt::Move { from, to } => {
                for t in to {
                    self.do_move(from, t);
                }
                Flow::Normal
            }
            Stmt::Display { items } => {
                let line: String = items.iter().map(|o| self.eval_text(o)).collect();
                self.output.push(line);
                Flow::Normal
            }
            Stmt::Accept { .. } => Flow::Normal, // sin entrada: deja el campo igual
            Stmt::Compute {
                targets,
                rounded,
                expr,
            } => {
                let value = self.eval_expr(expr);
                for t in targets {
                    self.store(t, value, *rounded);
                }
                Flow::Normal
            }
            Stmt::Add {
                addends,
                to,
                giving,
                rounded,
            } => {
                let sum = self.fold_sum(addends);
                if giving.is_empty() {
                    for t in to {
                        let cur = self.eval_decimal(t);
                        self.store(t, cur.add(&sum), *rounded);
                    }
                } else {
                    let base = match to.first() {
                        Some(first) => sum.add(&self.eval_decimal(first)),
                        None => sum,
                    };
                    for g in giving {
                        self.store(g, base, *rounded);
                    }
                }
                Flow::Normal
            }
            Stmt::Subtract {
                amounts,
                from,
                giving,
                rounded,
            } => {
                let sum = self.fold_sum(amounts);
                if giving.is_empty() {
                    for t in from {
                        let cur = self.eval_decimal(t);
                        self.store(t, cur.sub(&sum), *rounded);
                    }
                } else {
                    let minuend = from
                        .first()
                        .map(|f| self.eval_decimal(f))
                        .unwrap_or_else(Decimal::zero);
                    let value = minuend.sub(&sum);
                    for g in giving {
                        self.store(g, value, *rounded);
                    }
                }
                Flow::Normal
            }
            Stmt::Multiply {
                left,
                by,
                giving,
                rounded,
            } => {
                let value = self.eval_decimal(left).mul(&self.eval_decimal(by));
                if giving.is_empty() {
                    // `MULTIPLY a BY b` sin GIVING: b queda con a*b.
                    self.store(by, value, *rounded);
                } else {
                    for g in giving {
                        self.store(g, value, *rounded);
                    }
                }
                Flow::Normal
            }
            Stmt::Divide {
                left,
                right,
                by_form,
                giving,
                rounded,
            } => {
                let (num, den) = if *by_form {
                    (self.eval_decimal(left), self.eval_decimal(right))
                } else {
                    (self.eval_decimal(right), self.eval_decimal(left))
                };
                if giving.is_empty() {
                    // `DIVIDE a INTO b` sin GIVING: b queda con b/a.
                    let v = divide(num, den, self.target_scale(right));
                    self.store(right, v, *rounded);
                } else {
                    for g in giving {
                        let v = divide(num, den, self.target_scale(g));
                        self.store(g, v, *rounded);
                    }
                }
                Flow::Normal
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                if self.eval_cond(cond) {
                    self.exec_block(then_branch)
                } else {
                    self.exec_block(else_branch)
                }
            }
            Stmt::Evaluate {
                subject,
                whens,
                other,
            } => {
                for branch in whens {
                    if branch
                        .values
                        .iter()
                        .any(|v| self.operands_equal(subject, v))
                    {
                        return self.exec_block(&branch.body);
                    }
                }
                self.exec_block(other)
            }
            Stmt::StringConcat { sources, into } => {
                let s: String = sources.iter().map(|o| self.eval_text(o)).collect();
                self.store_text(into, &s);
                Flow::Normal
            }
            Stmt::Unstring {
                source,
                delimiter,
                into,
            } => {
                let src = self.eval_text(source);
                let delim = self.eval_text(delimiter);
                let parts: Vec<String> = if delim.is_empty() {
                    vec![src]
                } else {
                    src.split(delim.as_str()).map(|p| p.to_string()).collect()
                };
                for (i, target) in into.iter().enumerate() {
                    let piece = parts.get(i).cloned().unwrap_or_default();
                    self.store_text(target, &piece);
                }
                Flow::Normal
            }
            Stmt::Inspect { target, op } => {
                match op {
                    InspectOp::TallyingForAll { counter, search } => {
                        let hay = self.eval_text(target);
                        let needle = self.eval_text(search);
                        let n = if needle.is_empty() {
                            0
                        } else {
                            hay.matches(needle.as_str()).count()
                        };
                        let cur = self.eval_decimal(counter);
                        self.store(counter, cur.add(&Decimal::from_integer(n as i128)), false);
                    }
                    InspectOp::ReplacingAll { from, to } => {
                        let hay = self.eval_text(target);
                        let f = self.eval_text(from);
                        let t = self.eval_text(to);
                        let new = if f.is_empty() {
                            hay
                        } else {
                            hay.replace(f.as_str(), t.as_str())
                        };
                        self.store_text(target, &new);
                    }
                }
                Flow::Normal
            }
            Stmt::Perform(p) => self.exec_perform(p),
            Stmt::GoTo { target } => {
                // Aproximación: ejecuta el destino y sale del párrafo.
                match self.run_paragraph(target) {
                    Flow::Stop => Flow::Stop,
                    _ => Flow::Exit,
                }
            }
            Stmt::StopRun | Stmt::Goback => Flow::Stop,
            Stmt::Exit => Flow::Exit,
            Stmt::Continue => Flow::Normal,
            Stmt::Unknown { .. } => Flow::Normal, // verbo no soportado: se omite
        }
    }

    fn exec_perform(&mut self, p: &'a Perform) -> Flow {
        match &p.control {
            PerformControl::Once => self.run_target(&p.target),
            PerformControl::Times(n) => {
                let count = self.count_of(n);
                for _ in 0..count {
                    if self.tick() {
                        return Flow::Stop;
                    }
                    if let Flow::Stop = self.run_target(&p.target) {
                        return Flow::Stop;
                    }
                }
                Flow::Normal
            }
            PerformControl::Until(cond) => loop {
                if self.tick() {
                    return Flow::Stop;
                }
                if self.eval_cond(cond) {
                    return Flow::Normal;
                }
                if let Flow::Stop = self.run_target(&p.target) {
                    return Flow::Stop;
                }
            },
            PerformControl::Varying {
                var,
                from,
                by,
                until,
            } => {
                let var_op = Operand::Data(var.clone());
                let start = self.eval_decimal(from);
                self.store(&var_op, start, false);
                loop {
                    if self.tick() {
                        return Flow::Stop;
                    }
                    if self.eval_cond(until) {
                        return Flow::Normal;
                    }
                    if let Flow::Stop = self.run_target(&p.target) {
                        return Flow::Stop;
                    }
                    let next = self.eval_decimal(&var_op).add(&self.eval_decimal(by));
                    self.store(&var_op, next, false);
                }
            }
        }
    }

    /// Ejecuta una vez el cuerpo de un `PERFORM`. Un `EXIT` dentro de
    /// él termina esa pasada, no el programa.
    fn run_target(&mut self, target: &'a PerformTarget) -> Flow {
        let flow = match target {
            PerformTarget::Paragraph { name, .. } => self.run_paragraph(name),
            PerformTarget::Inline(body) => self.exec_block(body),
        };
        match flow {
            Flow::Stop => Flow::Stop,
            _ => Flow::Normal,
        }
    }

    fn run_paragraph(&mut self, name: &str) -> Flow {
        let Some(&idx) = self.para_index.get(&name.to_uppercase()) else {
            return Flow::Normal;
        };
        let ir = self.ir;
        match self.exec_block(&ir.procedures[idx].body) {
            Flow::Stop => Flow::Stop,
            _ => Flow::Normal,
        }
    }

    /// Resuelve una referencia a dato (escalar o elemento de tabla) a
    /// su nombre y un índice 0-based. `None` si no es una referencia.
    fn resolve(&self, op: &Operand) -> Option<(String, usize)> {
        match op {
            Operand::Data(name) => Some((name.to_uppercase(), 0)),
            Operand::Indexed { name, index } => {
                // El subíndice de COBOL es 1-based.
                let i = self
                    .eval_decimal(index)
                    .rescale(0, Rounding::Truncate)
                    .mantissa();
                let idx = if i < 1 { 0 } else { (i - 1) as usize };
                Some((name.to_uppercase(), idx))
            }
            _ => None,
        }
    }

    /// `MOVE from` a un solo destino (escalar o elemento de tabla).
    fn do_move(&mut self, from: &Operand, target: &Operand) {
        let Some((key, idx)) = self.resolve(target) else {
            return;
        };
        let is_num = matches!(self.fields.get(&key), Some(Cell::Num(_)));
        if is_num {
            let v = self.eval_decimal(from);
            if let Some(Cell::Num(arr)) = self.fields.get_mut(&key) {
                if let Some(n) = arr.get_mut(idx) {
                    n.store(v);
                }
            }
        } else if let Operand::Figurative(fig) = from {
            let ch = figurative_fill(*fig);
            if let Some(Cell::Text(arr)) = self.fields.get_mut(&key) {
                if let Some(t) = arr.get_mut(idx) {
                    t.fill(ch);
                }
            }
        } else {
            let s = self.eval_text(from);
            if let Some(Cell::Text(arr)) = self.fields.get_mut(&key) {
                if let Some(t) = arr.get_mut(idx) {
                    t.store(&s);
                }
            }
        }
    }

    /// Almacena un valor en un destino, conformándolo a su tipo.
    fn store(&mut self, target: &Operand, value: Decimal, rounded: bool) {
        let Some((key, idx)) = self.resolve(target) else {
            return;
        };
        match self.fields.get_mut(&key) {
            Some(Cell::Num(arr)) => {
                if let Some(n) = arr.get_mut(idx) {
                    if rounded {
                        n.store_rounded(value);
                    } else {
                        n.store(value);
                    }
                }
            }
            Some(Cell::Text(arr)) => {
                if let Some(t) = arr.get_mut(idx) {
                    t.store(&value.to_string());
                }
            }
            None => {}
        }
    }

    /// Almacena un texto en un destino, conformándolo a su tipo.
    fn store_text(&mut self, target: &Operand, text: &str) {
        let Some((key, idx)) = self.resolve(target) else {
            return;
        };
        match self.fields.get_mut(&key) {
            Some(Cell::Text(arr)) => {
                if let Some(t) = arr.get_mut(idx) {
                    t.store(text);
                }
            }
            Some(Cell::Num(arr)) => {
                if let Some(n) = arr.get_mut(idx) {
                    n.store(Decimal::parse(text.trim()).unwrap_or_else(|_| Decimal::zero()));
                }
            }
            None => {}
        }
    }

    // ── Evaluación ────────────────────────────────────────────────

    fn eval_decimal(&self, op: &Operand) -> Decimal {
        match op {
            Operand::Num(n) => Decimal::parse(n).unwrap_or_else(|_| Decimal::zero()),
            Operand::Str(s) => Decimal::parse(s).unwrap_or_else(|_| Decimal::zero()),
            Operand::Figurative(_) => Decimal::zero(),
            Operand::Data(_) | Operand::Indexed { .. } => {
                let Some((key, idx)) = self.resolve(op) else {
                    return Decimal::zero();
                };
                match self.fields.get(&key) {
                    Some(Cell::Num(arr)) => arr
                        .get(idx)
                        .map(|n| n.value())
                        .unwrap_or_else(Decimal::zero),
                    Some(Cell::Text(arr)) => arr
                        .get(idx)
                        .and_then(|t| Decimal::parse(t.as_str().trim()).ok())
                        .unwrap_or_else(Decimal::zero),
                    None => Decimal::zero(),
                }
            }
        }
    }

    fn eval_text(&self, op: &Operand) -> String {
        match op {
            Operand::Str(s) => s.clone(),
            Operand::Num(n) => n.clone(),
            Operand::Figurative(f) => figurative_text(*f).to_string(),
            Operand::Data(_) | Operand::Indexed { .. } => {
                let Some((key, idx)) = self.resolve(op) else {
                    return String::new();
                };
                match self.fields.get(&key) {
                    Some(Cell::Num(arr)) => arr.get(idx).map(|n| n.display()).unwrap_or_default(),
                    Some(Cell::Text(arr)) => arr.get(idx).map(|t| t.display()).unwrap_or_default(),
                    None => String::new(),
                }
            }
        }
    }

    fn eval_expr(&self, e: &Expr) -> Decimal {
        match e {
            Expr::Operand(op) => self.eval_decimal(op),
            Expr::Neg(inner) => Decimal::zero().sub(&self.eval_expr(inner)),
            Expr::Binary { op, lhs, rhs } => {
                let l = self.eval_expr(lhs);
                let r = self.eval_expr(rhs);
                match op {
                    BinOp::Add => l.add(&r),
                    BinOp::Sub => l.sub(&r),
                    BinOp::Mul => l.mul(&r),
                    BinOp::Div => divide(l, r, DIV_SCALE),
                    BinOp::Pow => pow(&l, &r),
                }
            }
        }
    }

    fn eval_cond(&self, c: &Cond) -> bool {
        match c {
            Cond::Compare { lhs, op, rhs } => {
                let ord = if self.is_text(lhs) || self.is_text(rhs) {
                    cobol_text_cmp(&self.eval_text(lhs), &self.eval_text(rhs))
                } else {
                    self.eval_decimal(lhs).cmp(&self.eval_decimal(rhs))
                };
                match op {
                    CmpOp::Eq => ord.is_eq(),
                    CmpOp::Ne => ord.is_ne(),
                    CmpOp::Lt => ord.is_lt(),
                    CmpOp::Gt => ord.is_gt(),
                    CmpOp::Le => ord.is_le(),
                    CmpOp::Ge => ord.is_ge(),
                }
            }
            Cond::Named(name) => match self.conditions.get(&name.to_uppercase()) {
                // Un nombre de condición (88): el dato padre igual al
                // valor que la hace verdadera.
                Some(cn) => self.operands_equal(&Operand::Data(cn.parent.clone()), &cn.value),
                None => false,
            },
            Cond::Not(inner) => !self.eval_cond(inner),
            Cond::And(a, b) => self.eval_cond(a) && self.eval_cond(b),
            Cond::Or(a, b) => self.eval_cond(a) || self.eval_cond(b),
        }
    }

    fn is_text(&self, op: &Operand) -> bool {
        match op {
            Operand::Str(_) => true,
            Operand::Data(_) | Operand::Indexed { .. } => match self.resolve(op) {
                Some((key, _)) => matches!(self.fields.get(&key), Some(Cell::Text(_))),
                None => false,
            },
            _ => false,
        }
    }

    /// ¿Son iguales dos operandos? (Para las ramas `WHEN` del `EVALUATE`.)
    fn operands_equal(&self, a: &Operand, b: &Operand) -> bool {
        if self.is_text(a) || self.is_text(b) {
            cobol_text_cmp(&self.eval_text(a), &self.eval_text(b)).is_eq()
        } else {
            self.eval_decimal(a) == self.eval_decimal(b)
        }
    }

    /// La suma de una lista de operandos.
    fn fold_sum(&self, ops: &[Operand]) -> Decimal {
        let mut acc = Decimal::zero();
        for o in ops {
            acc = acc.add(&self.eval_decimal(o));
        }
        acc
    }

    /// Los dígitos fraccionarios de un destino numérico.
    fn target_scale(&self, op: &Operand) -> u8 {
        if let Some((key, idx)) = self.resolve(op) {
            if let Some(Cell::Num(arr)) = self.fields.get(&key) {
                if let Some(n) = arr.get(idx) {
                    return n.picture().fraction_digits;
                }
            }
        }
        4
    }

    /// El número de repeticiones de un `PERFORM ... TIMES`.
    fn count_of(&self, op: &Operand) -> usize {
        let m = self
            .eval_decimal(op)
            .rescale(0, Rounding::Truncate)
            .mantissa();
        if m < 0 {
            0
        } else {
            m as usize
        }
    }
}

/// División con escala fija; una división por cero da cero.
fn divide(num: Decimal, den: Decimal, scale: u8) -> Decimal {
    num.div(&den, scale, Rounding::Truncate)
        .unwrap_or_else(|_| Decimal::zero())
}

/// Potencia con exponente entero no negativo; en otro caso da 1.
fn pow(base: &Decimal, exp: &Decimal) -> Decimal {
    let e = exp.rescale(0, Rounding::Truncate).mantissa();
    if !(0..=256).contains(&e) {
        return Decimal::from_integer(1);
    }
    let mut acc = Decimal::from_integer(1);
    for _ in 0..e {
        acc = acc.mul(base);
    }
    acc
}

/// El texto que representa una constante figurativa.
fn figurative_text(f: Figurative) -> &'static str {
    match f {
        Figurative::Zero => "0",
        Figurative::Space => " ",
        Figurative::Quote => "\"",
        Figurative::HighValue | Figurative::LowValue | Figurative::Null => "",
    }
}

/// El carácter de relleno de una figurativa, para `Text::fill`.
fn figurative_fill(f: Figurative) -> char {
    match f {
        Figurative::Zero => '0',
        Figurative::Quote => '"',
        _ => ' ',
    }
}
