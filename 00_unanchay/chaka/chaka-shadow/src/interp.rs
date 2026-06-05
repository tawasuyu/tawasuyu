//! El intérprete del IR: la ejecución «sombra» del programa COBOL.
//!
//! Ejecuta el [`Ir`] directamente sobre los tipos de `chaka-runtime`,
//! sin compilar nada. Es una segunda ruta de ejecución, independiente
//! del código que emite `chaka-codegen` — eso es lo que lo hace un
//! validador: si el intérprete y el transpilado divergen, hay un bug.

use std::collections::HashMap;

use chaka_ir::{
    AccessMode, BinOp, CmpOp, Cond, ConditionName, Expr, Figurative, FileMode, FileOrg, InspectOp,
    Ir, Operand, Perform, PerformControl, PerformTarget, SearchBranch, Stmt, WhenTest,
};
use chaka_runtime::{
    cobol_text_cmp, format_edited, CobFile, Decimal, Num, Organization, Rounding, StartCmp, Text,
};

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
    files: HashMap<String, CobFile>,
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
        let files = ir
            .files
            .iter()
            .map(|f| {
                let org = match f.organization {
                    FileOrg::Indexed => Organization::Indexed,
                    FileOrg::Relative => Organization::Relative,
                    _ => Organization::LineSequential,
                };
                (f.name.to_uppercase(), CobFile::with_org(&f.path, org))
            })
            .collect();
        Self {
            ir,
            fields: build_fields(&ir.model),
            para_index,
            conditions,
            files,
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
                    if branch.tests.iter().any(|t| self.when_test(subject, t)) {
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
                    InspectOp::TallyingForLeading { counter, search } => {
                        let hay = self.eval_text(target);
                        let needle = self.eval_text(search);
                        let n = if needle.is_empty() {
                            0
                        } else {
                            let mut count: usize = 0;
                            let mut rest = hay.as_str();
                            while rest.starts_with(needle.as_str()) {
                                count += 1;
                                rest = &rest[needle.len()..];
                            }
                            count
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
                    InspectOp::Converting { from, to } => {
                        let hay = self.eval_text(target);
                        let from_chars: Vec<char> = self.eval_text(from).chars().collect();
                        let to_chars: Vec<char> = self.eval_text(to).chars().collect();
                        let new: String = hay
                            .chars()
                            .map(|c| match from_chars.iter().position(|&f| f == c) {
                                Some(i) => to_chars.get(i).copied().unwrap_or(c),
                                None => c,
                            })
                            .collect();
                        self.store_text(target, &new);
                    }
                }
                Flow::Normal
            }
            Stmt::Initialize { targets } => {
                for t in targets {
                    match t {
                        Operand::Data(name) => {
                            match self.ir.model.group(name).map(|g| g.members.clone()) {
                                Some(members) => {
                                    for m in &members {
                                        self.reset_field(m);
                                    }
                                }
                                None => self.reset_field(name),
                            }
                        }
                        Operand::Indexed { .. } => self.reset_element(t),
                        _ => {}
                    }
                }
                Flow::Normal
            }
            Stmt::SetTrue { conditions } => {
                for name in conditions {
                    if let Some(cn) = self.conditions.get(&name.to_uppercase()).cloned() {
                        self.do_move(&cn.value, &Operand::Data(cn.parent));
                    }
                }
                Flow::Normal
            }
            Stmt::SetTo { targets, value } => {
                for t in targets {
                    self.do_move(value, &Operand::Data(t.to_uppercase()));
                }
                Flow::Normal
            }
            Stmt::SetAdjust { targets, by, up } => {
                let delta = self.eval_decimal(by);
                for t in targets {
                    let target = Operand::Data(t.to_uppercase());
                    let cur = self.eval_decimal(&target);
                    let new = if *up { cur.add(&delta) } else { cur.sub(&delta) };
                    self.store(&target, new, false);
                }
                Flow::Normal
            }
            Stmt::Open { mode, files } => {
                for f in files {
                    if let Some(cf) = self.files.get_mut(&f.to_uppercase()) {
                        match mode {
                            FileMode::Input => cf.open_input(),
                            FileMode::Output => cf.open_output(),
                            FileMode::IO => cf.open_io(),
                            FileMode::Extend => cf.open_extend(),
                        }
                    }
                }
                Flow::Normal
            }
            Stmt::Close { files } => {
                for f in files {
                    if let Some(cf) = self.files.get_mut(&f.to_uppercase()) {
                        cf.close();
                    }
                }
                Flow::Normal
            }
            Stmt::Read {
                file,
                key,
                at_end,
                not_at_end,
                invalid_key,
                not_invalid_key,
            } => {
                let meta = self.ir.files.iter().find(|f| f.name.eq_ignore_ascii_case(file));
                let record = meta.map(|m| m.record.clone()).unwrap_or_default();
                let org = meta.map(|m| m.organization).unwrap_or(FileOrg::LineSequential);
                let access = meta.map(|m| m.access).unwrap_or(AccessMode::Sequential);
                let keyed = matches!(org, FileOrg::Indexed | FileOrg::Relative);
                let random = keyed
                    && (key.is_some()
                        || !invalid_key.is_empty()
                        || !not_invalid_key.is_empty()
                        || access == AccessMode::Random);
                if random {
                    let key_text = self.key_text(file, key.as_deref());
                    let rec = self
                        .files
                        .get_mut(&file.to_uppercase())
                        .and_then(|cf| cf.read_keyed(&key_text));
                    match rec {
                        Some(text) => {
                            self.store_record(&record, &text);
                            self.exec_block(not_invalid_key)
                        }
                        None => self.exec_block(invalid_key),
                    }
                } else {
                    let line = self
                        .files
                        .get_mut(&file.to_uppercase())
                        .and_then(|cf| cf.read());
                    match line {
                        Some(text) => {
                            self.store_record(&record, &text);
                            self.exec_block(not_at_end)
                        }
                        None => self.exec_block(at_end),
                    }
                }
            }
            Stmt::Write {
                record,
                from,
                invalid_key,
                not_invalid_key,
            } => {
                if let Some(src) = from {
                    let text = self.eval_text(src);
                    self.store_record(record, &text);
                }
                self.exec_keyed_write(record, /* is_rewrite = */ false, invalid_key, not_invalid_key)
            }
            Stmt::Perform(p) => self.exec_perform(p),
            Stmt::Search {
                table,
                varying,
                at_end,
                whens,
            } => self.exec_search(table, varying, at_end, whens),
            Stmt::Rewrite {
                record,
                from,
                invalid_key,
                not_invalid_key,
            } => {
                if let Some(src) = from {
                    let text = self.eval_text(src);
                    self.store_record(record, &text);
                }
                self.exec_keyed_write(record, /* is_rewrite = */ true, invalid_key, not_invalid_key)
            }
            Stmt::Delete {
                file,
                invalid_key,
                not_invalid_key,
            } => {
                let keyed = self.file_is_keyed(file);
                if keyed {
                    let key_text = self.key_text(file, None);
                    let ok = self
                        .files
                        .get_mut(&file.to_uppercase())
                        .is_some_and(|cf| cf.delete_keyed(&key_text));
                    if ok {
                        self.exec_block(not_invalid_key)
                    } else {
                        self.exec_block(invalid_key)
                    }
                } else {
                    self.exec_block(not_invalid_key)
                }
            }
            Stmt::Start {
                file,
                key,
                cmp,
                invalid_key,
                not_invalid_key,
            } => {
                let keyed = self.file_is_keyed(file);
                if keyed {
                    let key_text = self.key_text(file, key.as_deref());
                    let rt_cmp = match cmp {
                        chaka_ir::StartCmp::Eq => StartCmp::Eq,
                        chaka_ir::StartCmp::Gt => StartCmp::Gt,
                        chaka_ir::StartCmp::Ge => StartCmp::Ge,
                        chaka_ir::StartCmp::Lt => StartCmp::Lt,
                        chaka_ir::StartCmp::Le => StartCmp::Le,
                    };
                    let ok = self
                        .files
                        .get_mut(&file.to_uppercase())
                        .is_some_and(|cf| cf.start(&key_text, rt_cmp));
                    if ok {
                        self.exec_block(not_invalid_key)
                    } else {
                        self.exec_block(invalid_key)
                    }
                } else {
                    self.exec_block(not_invalid_key)
                }
            }
            Stmt::Sort {
                using, giving, ..
            } => {
                self.exec_sort_or_merge(using, giving, /* do_sort = */ true);
                Flow::Normal
            }
            Stmt::Merge {
                using, giving, ..
            } => {
                // En la v1 MERGE re-ordena las líneas. Si las entradas
                // ya estaban ordenadas, el resultado es idéntico.
                self.exec_sort_or_merge(using, giving, /* do_sort = */ true);
                Flow::Normal
            }
            Stmt::Call {
                program,
                on_overflow,
                not_on_overflow,
                ..
            } => {
                // Aproximación v1: si el nombre del sub-programa coincide
                // con un párrafo del mismo programa, lo ejecuta. Si no,
                // se considera fallo y dispara `ON OVERFLOW`.
                let target = self.eval_text(program).trim().to_uppercase();
                if self.para_index.contains_key(&target) {
                    if let Flow::Stop = self.run_paragraph(&target) {
                        return Flow::Stop;
                    }
                    self.exec_block(not_on_overflow)
                } else {
                    self.exec_block(on_overflow)
                }
            }
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

    /// Cuerpo común de `SORT` y `MERGE`: concatena las líneas de los
    /// ficheros `using` (los abre/cierra para leer), opcionalmente las
    /// ordena, y las vuelca a cada fichero de `giving`.
    fn exec_sort_or_merge(&mut self, using: &[String], giving: &[String], do_sort: bool) {
        let mut lines: Vec<String> = Vec::new();
        for name in using {
            let key = name.to_uppercase();
            if let Some(cf) = self.files.get_mut(&key) {
                cf.open_input();
                while let Some(l) = cf.read() {
                    lines.push(l);
                }
                cf.close();
            }
        }
        if do_sort {
            lines.sort();
        }
        for name in giving {
            let key = name.to_uppercase();
            if let Some(cf) = self.files.get_mut(&key) {
                cf.open_output();
                for l in &lines {
                    cf.write(l);
                }
                cf.close();
            }
        }
    }

    fn exec_search(
        &mut self,
        table: &str,
        varying: &str,
        at_end: &'a [Stmt],
        whens: &'a [SearchBranch],
    ) -> Flow {
        if varying.is_empty() {
            // Sin índice explícito la v1 no puede iterar — corre `AT END`.
            return self.exec_block(at_end);
        }
        let limit = self
            .ir
            .model
            .field(table)
            .and_then(|f| f.occurs)
            .unwrap_or(0) as i128;
        let var_op = Operand::Data(varying.to_uppercase());
        loop {
            if self.tick() {
                return Flow::Stop;
            }
            let idx = self.eval_decimal(&var_op).mantissa();
            if idx < 1 || idx > limit {
                return self.exec_block(at_end);
            }
            for branch in whens {
                if self.eval_cond(&branch.cond) {
                    return self.exec_block(&branch.body);
                }
            }
            let next = idx + 1;
            self.store(&var_op, Decimal::from_integer(next), false);
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
            PerformTarget::Paragraph { name, thru } => {
                self.run_paragraph_range(name, thru.as_deref())
            }
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

    /// Ejecuta el rango de párrafos de `name` a `thru` inclusive (el
    /// `PERFORM name THRU thru`); sólo `name` si `thru` es `None`.
    fn run_paragraph_range(&mut self, name: &str, thru: Option<&str>) -> Flow {
        let Some(&start) = self.para_index.get(&name.to_uppercase()) else {
            return Flow::Normal;
        };
        let end = match thru {
            Some(t) => self
                .para_index
                .get(&t.to_uppercase())
                .copied()
                .unwrap_or(start),
            None => start,
        };
        let (lo, hi) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let ir = self.ir;
        for i in lo..=hi {
            if let Flow::Stop = self.exec_block(&ir.procedures[i].body) {
                return Flow::Stop;
            }
        }
        Flow::Normal
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
        // Un destino con PICTURE de edición formatea el valor numérico.
        if let Operand::Data(name) = target {
            if let Some(pic) = self.ir.model.field(name).and_then(|f| f.edit.clone()) {
                let value = self.eval_decimal(from);
                let text = format_edited(value, &pic);
                self.store_text(target, &text);
                return;
            }
        }
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

    /// Resetea un campo completo (escalar o tabla) a su valor por
    /// defecto: 0 si es numérico, espacios si es alfanumérico.
    fn reset_field(&mut self, name: &str) {
        match self.fields.get_mut(&name.to_uppercase()) {
            Some(Cell::Num(arr)) => {
                for n in arr.iter_mut() {
                    *n = Num::new(n.picture());
                }
            }
            Some(Cell::Text(arr)) => {
                for t in arr.iter_mut() {
                    *t = Text::new(t.len());
                }
            }
            None => {}
        }
    }

    /// Resetea un solo elemento de tabla a su valor por defecto.
    fn reset_element(&mut self, op: &Operand) {
        let Some((key, idx)) = self.resolve(op) else {
            return;
        };
        match self.fields.get_mut(&key) {
            Some(Cell::Num(arr)) => {
                if let Some(n) = arr.get_mut(idx) {
                    *n = Num::new(n.picture());
                }
            }
            Some(Cell::Text(arr)) => {
                if let Some(t) = arr.get_mut(idx) {
                    *t = Text::new(t.len());
                }
            }
            None => {}
        }
    }

    /// Almacena un texto en un destino, conformándolo a su tipo.
    // ── Registros de fichero (elementales o de grupo) ─────────────

    /// El texto completo de un registro: el campo si es elemental, o la
    /// concatenación del display de sus miembros si es un grupo.
    fn record_text(&self, record: &str) -> String {
        match self.ir.model.group(record) {
            Some(g) => g
                .members
                .iter()
                .map(|m| self.eval_text(&Operand::Data(m.clone())))
                .collect(),
            None => self.eval_text(&Operand::Data(record.to_string())),
        }
    }

    /// Distribuye `text` en un registro: lo asigna entero si es
    /// elemental, o lo trocea por el ancho de cada miembro si es un
    /// grupo (un registro de longitud fija).
    fn store_record(&mut self, record: &str, text: &str) {
        let Some(members) = self.ir.model.group(record).map(|g| g.members.clone()) else {
            self.store_text(&Operand::Data(record.to_string()), text);
            return;
        };
        let chars: Vec<char> = text.chars().collect();
        let mut off = 0usize;
        for m in &members {
            let w = self.field_width(m);
            let slice: String = chars
                .get(off..off + w)
                .map(|s| s.iter().collect())
                .unwrap_or_else(|| {
                    chars
                        .get(off..)
                        .map(|s| s.iter().collect())
                        .unwrap_or_default()
                });
            self.store_text(&Operand::Data(m.clone()), &slice);
            off += w;
        }
    }

    /// El ancho en caracteres de un campo elemental (su tamaño en un
    /// registro de longitud fija).
    fn field_width(&self, name: &str) -> usize {
        match self.ir.model.field(name).map(|f| f.kind) {
            Some(chaka_ir::FieldKind::Text { len }) => len,
            Some(chaka_ir::FieldKind::Num { int, frac, .. }) => int as usize + frac as usize,
            None => 0,
        }
    }

    /// El texto de la clave de un fichero: el campo `KEY` explícito, o la
    /// `RECORD`/`RELATIVE KEY` por defecto según su organización.
    fn key_text(&self, file: &str, explicit: Option<&str>) -> String {
        let key_name = explicit.map(|s| s.to_string()).or_else(|| {
            self.ir
                .files
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(file))
                .and_then(|e| match e.organization {
                    FileOrg::Indexed => e.record_key.clone(),
                    FileOrg::Relative => e.relative_key.clone(),
                    _ => None,
                })
        });
        match key_name {
            Some(n) => self.eval_text(&Operand::Data(n)),
            None => String::new(),
        }
    }

    /// ¿Es un fichero por clave (indexado o relativo)?
    fn file_is_keyed(&self, file: &str) -> bool {
        self.ir
            .files
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(file))
            .is_some_and(|f| matches!(f.organization, FileOrg::Indexed | FileOrg::Relative))
    }

    /// Cuerpo común de `WRITE`/`REWRITE`: escribe el registro en su
    /// fichero. Sobre un fichero por clave usa `write_keyed`/`rewrite_keyed`
    /// (cuyo `bool` decide la rama `INVALID KEY`); sobre line-sequential,
    /// un `write` simple seguido de `NOT INVALID KEY`.
    fn exec_keyed_write(
        &mut self,
        record: &str,
        is_rewrite: bool,
        invalid_key: &'a [Stmt],
        not_invalid_key: &'a [Stmt],
    ) -> Flow {
        let (file_name, keyed) =
            match self.ir.files.iter().find(|f| f.record.eq_ignore_ascii_case(record)) {
                Some(m) => (
                    m.name.to_uppercase(),
                    matches!(m.organization, FileOrg::Indexed | FileOrg::Relative),
                ),
                None => return Flow::Normal,
            };
        let line = self.record_text(record);
        if keyed {
            let key_text = self.key_text(&file_name, None);
            let ok = self.files.get_mut(&file_name).is_some_and(|cf| {
                if is_rewrite {
                    cf.rewrite_keyed(&key_text, &line)
                } else {
                    cf.write_keyed(&key_text, &line)
                }
            });
            if ok {
                self.exec_block(not_invalid_key)
            } else {
                self.exec_block(invalid_key)
            }
        } else {
            if let Some(cf) = self.files.get_mut(&file_name) {
                cf.write(&line);
            }
            self.exec_block(not_invalid_key)
        }
    }

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

    /// ¿Se cumple una prueba `WHEN` para el sujeto dado?
    fn when_test(&self, subject: &Operand, test: &WhenTest) -> bool {
        match test {
            WhenTest::Value(v) => self.operands_equal(subject, v),
            WhenTest::Range(lo, hi) => {
                let s = self.eval_decimal(subject);
                s >= self.eval_decimal(lo) && s <= self.eval_decimal(hi)
            }
            WhenTest::Cond(cond) => self.eval_cond(cond),
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
