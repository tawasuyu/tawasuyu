//! El intérprete del IR: la ejecución «sombra» del programa COBOL.
//!
//! Ejecuta el [`Ir`] directamente sobre los tipos de `charka-runtime`,
//! sin compilar nada. Es una segunda ruta de ejecución, independiente
//! del código que emite `charka-codegen` — eso es lo que lo hace un
//! validador: si el intérprete y el transpilado divergen, hay un bug.

use std::collections::HashMap;

use charka_ir::{
    BinOp, CmpOp, Cond, Expr, Figurative, Ir, Operand, Perform, PerformControl, PerformTarget, Stmt,
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
        Self {
            ir,
            fields: build_fields(&ir.data),
            para_index,
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
                        let cur = self.field_value(t);
                        self.store(t, cur.add(&sum), *rounded);
                    }
                } else {
                    let base = match to.first() {
                        Some(first) => sum.add(&self.field_value(first)),
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
                        let cur = self.field_value(t);
                        self.store(t, cur.sub(&sum), *rounded);
                    }
                } else {
                    let minuend = from
                        .first()
                        .map(|f| self.field_value(f))
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
                    if let Operand::Data(name) = by {
                        self.store(name, value, *rounded);
                    }
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
                    if let Operand::Data(name) = right {
                        let v = divide(num, den, self.target_scale(name));
                        self.store(name, v, *rounded);
                    }
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
                let start = self.eval_decimal(from);
                self.store(var, start, false);
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
                    let next = self.field_value(var).add(&self.eval_decimal(by));
                    self.store(var, next, false);
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

    /// `MOVE from` a un solo campo destino.
    fn do_move(&mut self, from: &Operand, target: &str) {
        let key = target.to_uppercase();
        match self.fields.get(&key) {
            Some(Cell::Num(_)) => {
                let v = self.eval_decimal(from);
                if let Some(Cell::Num(n)) = self.fields.get_mut(&key) {
                    n.store(v);
                }
            }
            Some(Cell::Text(_)) => {
                if let Operand::Figurative(fig) = from {
                    let ch = figurative_fill(*fig);
                    if let Some(Cell::Text(t)) = self.fields.get_mut(&key) {
                        t.fill(ch);
                    }
                } else {
                    let s = self.eval_text(from);
                    if let Some(Cell::Text(t)) = self.fields.get_mut(&key) {
                        t.store(&s);
                    }
                }
            }
            None => {}
        }
    }

    /// Almacena un valor en un campo, conformándolo a su tipo.
    fn store(&mut self, name: &str, value: Decimal, rounded: bool) {
        match self.fields.get_mut(&name.to_uppercase()) {
            Some(Cell::Num(n)) => {
                if rounded {
                    n.store_rounded(value);
                } else {
                    n.store(value);
                }
            }
            Some(Cell::Text(t)) => t.store(&value.to_string()),
            None => {}
        }
    }

    // ── Evaluación ────────────────────────────────────────────────

    fn eval_decimal(&self, op: &Operand) -> Decimal {
        match op {
            Operand::Num(n) => Decimal::parse(n).unwrap_or_else(|_| Decimal::zero()),
            Operand::Str(s) => Decimal::parse(s).unwrap_or_else(|_| Decimal::zero()),
            Operand::Figurative(_) => Decimal::zero(),
            Operand::Data(name) => match self.fields.get(&name.to_uppercase()) {
                Some(Cell::Num(n)) => n.value(),
                Some(Cell::Text(t)) => {
                    Decimal::parse(t.as_str().trim()).unwrap_or_else(|_| Decimal::zero())
                }
                None => Decimal::zero(),
            },
        }
    }

    fn eval_text(&self, op: &Operand) -> String {
        match op {
            Operand::Str(s) => s.clone(),
            Operand::Num(n) => n.clone(),
            Operand::Figurative(f) => figurative_text(*f).to_string(),
            Operand::Data(name) => match self.fields.get(&name.to_uppercase()) {
                Some(Cell::Num(n)) => n.display(),
                Some(Cell::Text(t)) => t.display(),
                None => String::new(),
            },
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
            Cond::Named(_) => false, // nombres de condición (88): no soportado
            Cond::Not(inner) => !self.eval_cond(inner),
            Cond::And(a, b) => self.eval_cond(a) && self.eval_cond(b),
            Cond::Or(a, b) => self.eval_cond(a) || self.eval_cond(b),
        }
    }

    fn is_text(&self, op: &Operand) -> bool {
        match op {
            Operand::Str(_) => true,
            Operand::Data(name) => {
                matches!(self.fields.get(&name.to_uppercase()), Some(Cell::Text(_)))
            }
            _ => false,
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

    /// El valor actual de un campo por nombre.
    fn field_value(&self, name: &str) -> Decimal {
        match self.fields.get(&name.to_uppercase()) {
            Some(Cell::Num(n)) => n.value(),
            Some(Cell::Text(t)) => {
                Decimal::parse(t.as_str().trim()).unwrap_or_else(|_| Decimal::zero())
            }
            None => Decimal::zero(),
        }
    }

    /// Los dígitos fraccionarios de un campo numérico destino.
    fn target_scale(&self, name: &str) -> u8 {
        match self.fields.get(&name.to_uppercase()) {
            Some(Cell::Num(n)) => n.picture().fraction_digits,
            _ => 4,
        }
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
