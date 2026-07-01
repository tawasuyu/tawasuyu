//! `ecuacion` — **motor de Leyes autorables**: una Ley deja de ser un autómata
//! cableado en Rust y pasa a ser un **sistema de ecuaciones de actualización sobre
//! los campos de estado de la celda**, escrito por el usuario y **compilado** una vez
//! a opcodes para correr en el loop caliente.
//!
//! El modelo: cada material lleva un vector chico de **campos** escalares
//! ([`FieldDef`]: `cantidad`, `energía`, `temperatura`, `edad`…). Una Ley es una lista
//! de asignaciones `Δcampoₖ/dt = fₖ(campos, vecinos, params)` — un [`Program`]. Cada
//! tick, el motor integra `campo' = clamp(campo + dt·fₖ, min, max)` sobre toda la
//! rejilla.
//!
//! **Determinista** (regla §1.5 de la sim): se evalúa estilo **Jacobi** — `fₖ` lee
//! sólo el estado del tick anterior (buffer `cur`) y escribe en `next`, así el
//! resultado no depende del orden de barrido. Mismas condiciones → misma evolución,
//! sin azar y sin sesgo de recorrido (a diferencia de Gauss–Seidel).
//!
//! Esto **generaliza** la ley `Crecer` (un campo `altura` que sube) y abre reacción‑
//! difusión (Gray–Scott), difusión de calor, fuego, erosión… con la misma máquina.
//! NO reemplaza a [`WaterSim`](crate::WaterSim): ese es un autómata que *mueve celdas*
//! (conserva masa), otra discretización, y sigue como fast‑path nativo aparte.
//!
//! El mismo AST [`Expr`] alimenta las **dos vistas de autoría**: la barra de fórmula
//! (parser [`Expr::parse`] ↔ printer [`Expr::to_source`]) y, a futuro, el grafo de
//! nodos. Ambas serializan a este `Expr`, así se editan indistintamente.

use serde::{Deserialize, Serialize};

/// Índice de un campo de estado dentro del material (posición en su `Vec<FieldDef>`).
pub type FieldId = u16;
/// Índice de un parámetro del material (los sliders que ya expone la UI).
pub type ParamId = u16;

/// Un **campo de estado** de la celda: un escalar con nombre, valor inicial y rango.
/// El material declara los suyos; las ecuaciones los leen y escriben por nombre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub init: f32,
    pub min: f32,
    pub max: f32,
}

impl FieldDef {
    pub fn new(name: impl Into<String>, init: f32, min: f32, max: f32) -> Self {
        Self { name: name.into(), init, min, max }
    }
}

/// Dirección de un vecino de 6‑conectividad (caras del cubo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    Abajo,
    Arriba,
    Este,
    Oeste,
    Norte,
    Sur,
}

impl Dir {
    /// Desplazamiento `(dx,dy,dz)` de la dirección.
    #[inline]
    pub fn delta(self) -> (i32, i32, i32) {
        match self {
            Dir::Abajo => (0, -1, 0),
            Dir::Arriba => (0, 1, 0),
            Dir::Este => (1, 0, 0),
            Dir::Oeste => (-1, 0, 0),
            Dir::Norte => (0, 0, 1),
            Dir::Sur => (0, 0, -1),
        }
    }
    fn nombre(self) -> &'static str {
        match self {
            Dir::Abajo => "abajo",
            Dir::Arriba => "arriba",
            Dir::Este => "este",
            Dir::Oeste => "oeste",
            Dir::Norte => "norte",
            Dir::Sur => "sur",
        }
    }
    fn de_nombre(s: &str) -> Option<Dir> {
        Some(match s {
            "abajo" => Dir::Abajo,
            "arriba" => Dir::Arriba,
            "este" => Dir::Este,
            "oeste" => Dir::Oeste,
            "norte" => Dir::Norte,
            "sur" => Dir::Sur,
            _ => return None,
        })
    }
}

/// Reducción sobre los 6 vecinos de una celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reduce {
    Promedio,
    Suma,
    Min,
    Max,
}

impl Reduce {
    fn nombre(self) -> &'static str {
        match self {
            Reduce::Promedio => "avg",
            Reduce::Suma => "sum6",
            Reduce::Min => "min6",
            Reduce::Max => "max6",
        }
    }
}

/// Operador unario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnOp {
    Neg,
    Abs,
    Exp,
    Sqrt,
}

/// Operador binario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Min,
    Max,
    /// `a > b → 1.0`, si no `0.0` (umbral).
    Gt,
    /// `a < b → 1.0`, si no `0.0`.
    Lt,
}

/// **Árbol de la ecuación**: la expresión que da el lado derecho de un `Δcampo/dt`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expr {
    Const(f32),
    /// Valor del campo en esta celda.
    Field(FieldId),
    /// Parámetro del material (slider).
    Param(ParamId),
    /// Paso de tiempo `dt`.
    Dt,
    /// Laplaciano discreto 6‑conexo del campo: `Σvecinos − 6·centro`.
    Laplacian(FieldId),
    /// Reducción sobre los 6 vecinos.
    Vecinos(Reduce, FieldId),
    /// Valor del campo en un vecino concreto.
    Dir(Dir, FieldId),
    Un(UnOp, Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// `clamp(x, lo, hi)`.
    Clamp(Box<Expr>, Box<Expr>, Box<Expr>),
}

/// Una **asignación**: `Δcampo/dt = expr`. El motor integra `campo += dt·expr`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assign {
    pub campo: FieldId,
    pub expr: Expr,
}

// =============================================================================
//  Compilación a opcodes (stack machine)
// =============================================================================

/// Opcode de la máquina de pila que ejecuta una ecuación por celda (compilado una
/// vez al editar la Ley; corre en el loop caliente sin recorrer el árbol).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Const(f32),
    Field(FieldId),
    Param(ParamId),
    Dt,
    Laplacian(FieldId),
    Vecinos(Reduce, FieldId),
    Dir(Dir, FieldId),
    Un(UnOp),
    Bin(BinOp),
    Clamp,
}

/// Ecuación compilada de un campo: `(campo destino, opcodes en RPN)`.
#[derive(Debug, Clone)]
struct Compiled {
    campo: FieldId,
    ops: Vec<Op>,
}

fn compilar(expr: &Expr, out: &mut Vec<Op>) {
    match expr {
        Expr::Const(c) => out.push(Op::Const(*c)),
        Expr::Field(f) => out.push(Op::Field(*f)),
        Expr::Param(p) => out.push(Op::Param(*p)),
        Expr::Dt => out.push(Op::Dt),
        Expr::Laplacian(f) => out.push(Op::Laplacian(*f)),
        Expr::Vecinos(r, f) => out.push(Op::Vecinos(*r, *f)),
        Expr::Dir(d, f) => out.push(Op::Dir(*d, *f)),
        Expr::Un(op, a) => {
            compilar(a, out);
            out.push(Op::Un(*op));
        }
        Expr::Bin(op, a, b) => {
            compilar(a, out);
            compilar(b, out);
            out.push(Op::Bin(*op));
        }
        Expr::Clamp(x, lo, hi) => {
            compilar(x, out);
            compilar(lo, out);
            compilar(hi, out);
            out.push(Op::Clamp);
        }
    }
}

/// Un **programa de Ley compilado**: las ecuaciones de todos los campos, listas para
/// correr. Se obtiene con [`Program::compile`] a partir de las [`Assign`] autoradas.
#[derive(Debug, Clone)]
pub struct Program {
    ecuaciones: Vec<Compiled>,
}

impl Program {
    /// Compila la lista de asignaciones (una Ley) a opcodes.
    pub fn compile(asignaciones: &[Assign]) -> Program {
        let ecuaciones = asignaciones
            .iter()
            .map(|a| {
                let mut ops = Vec::new();
                compilar(&a.expr, &mut ops);
                Compiled { campo: a.campo, ops }
            })
            .collect();
        Program { ecuaciones }
    }

    /// Cantidad de ecuaciones (campos que la Ley modifica).
    pub fn len(&self) -> usize {
        self.ecuaciones.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ecuaciones.is_empty()
    }
}

// =============================================================================
//  Motor de campo
// =============================================================================

/// **Motor de campos**: mantiene el estado escalar por celda de todos los campos de
/// un material y avanza la simulación aplicando un [`Program`] estilo Jacobi.
///
/// Layout de memoria **campo‑mayor**: los `ncells` valores de un campo son
/// contiguos (`buf[f*ncells + i]`), así muestrear vecinos de un campo es cache‑friendly.
pub struct FieldEngine {
    dim: [u32; 3],
    ncells: usize,
    defs: Vec<FieldDef>,
    cur: Vec<f32>,
    next: Vec<f32>,
}

impl FieldEngine {
    /// Crea el motor con los campos dados, cada celda inicializada a `FieldDef::init`.
    pub fn new(dim: [u32; 3], defs: Vec<FieldDef>) -> Self {
        let ncells = (dim[0] as usize) * (dim[1] as usize) * (dim[2] as usize);
        let nf = defs.len();
        let mut cur = vec![0.0f32; nf * ncells];
        for (f, d) in defs.iter().enumerate() {
            for v in cur[f * ncells..(f + 1) * ncells].iter_mut() {
                *v = d.init;
            }
        }
        let next = cur.clone();
        Self { dim, ncells, defs, cur, next }
    }

    pub fn dim(&self) -> [u32; 3] {
        self.dim
    }
    pub fn fields(&self) -> &[FieldDef] {
        &self.defs
    }

    #[inline]
    fn idx(&self, x: u32, y: u32, z: u32) -> usize {
        (x as usize) + (y as usize) * (self.dim[0] as usize)
            + (z as usize) * (self.dim[0] as usize) * (self.dim[1] as usize)
    }

    /// Lee el campo `f` en `(x,y,z)`.
    #[inline]
    pub fn get(&self, f: FieldId, x: u32, y: u32, z: u32) -> f32 {
        self.cur[(f as usize) * self.ncells + self.idx(x, y, z)]
    }

    /// Fija el campo `f` en `(x,y,z)` (clampeado al rango del campo). Útil para
    /// sembrar condiciones iniciales (una gota, un foco de calor…).
    pub fn set(&mut self, f: FieldId, x: u32, y: u32, z: u32, v: f32) {
        let d = &self.defs[f as usize];
        let vi = v.clamp(d.min, d.max);
        let i = (f as usize) * self.ncells + self.idx(x, y, z);
        self.cur[i] = vi;
    }

    /// Muestra el campo `f` en `(x,y,z)` con **borde Neumann** (fuera de la rejilla =
    /// el valor de la celda misma → sin flujo artificial en el borde).
    #[inline]
    fn sample(&self, f: usize, x: i32, y: i32, z: i32) -> f32 {
        let xc = x.clamp(0, self.dim[0] as i32 - 1) as u32;
        let yc = y.clamp(0, self.dim[1] as i32 - 1) as u32;
        let zc = z.clamp(0, self.dim[2] as i32 - 1) as u32;
        self.cur[f * self.ncells + self.idx(xc, yc, zc)]
    }

    /// Evalúa una ecuación compilada en la celda `(x,y,z)` sobre el buffer `cur`.
    fn eval(&self, ops: &[Op], params: &[f32], dt: f32, x: i32, y: i32, z: i32, stack: &mut Vec<f32>) -> f32 {
        stack.clear();
        for op in ops {
            match *op {
                Op::Const(c) => stack.push(c),
                Op::Field(f) => stack.push(self.sample(f as usize, x, y, z)),
                Op::Param(p) => stack.push(params.get(p as usize).copied().unwrap_or(0.0)),
                Op::Dt => stack.push(dt),
                Op::Laplacian(f) => {
                    let f = f as usize;
                    let c = self.sample(f, x, y, z);
                    let s = self.sample(f, x + 1, y, z)
                        + self.sample(f, x - 1, y, z)
                        + self.sample(f, x, y + 1, z)
                        + self.sample(f, x, y - 1, z)
                        + self.sample(f, x, y, z + 1)
                        + self.sample(f, x, y, z - 1);
                    stack.push(s - 6.0 * c);
                }
                Op::Vecinos(r, f) => {
                    let f = f as usize;
                    let n = [
                        self.sample(f, x + 1, y, z),
                        self.sample(f, x - 1, y, z),
                        self.sample(f, x, y + 1, z),
                        self.sample(f, x, y - 1, z),
                        self.sample(f, x, y, z + 1),
                        self.sample(f, x, y, z - 1),
                    ];
                    let v = match r {
                        Reduce::Suma => n.iter().sum(),
                        Reduce::Promedio => n.iter().sum::<f32>() / 6.0,
                        Reduce::Min => n.iter().copied().fold(f32::INFINITY, f32::min),
                        Reduce::Max => n.iter().copied().fold(f32::NEG_INFINITY, f32::max),
                    };
                    stack.push(v);
                }
                Op::Dir(d, f) => {
                    let (dx, dy, dz) = d.delta();
                    stack.push(self.sample(f as usize, x + dx, y + dy, z + dz));
                }
                Op::Un(u) => {
                    let a = stack.pop().unwrap_or(0.0);
                    stack.push(match u {
                        UnOp::Neg => -a,
                        UnOp::Abs => a.abs(),
                        UnOp::Exp => a.exp(),
                        UnOp::Sqrt => a.max(0.0).sqrt(),
                    });
                }
                Op::Bin(b) => {
                    let rhs = stack.pop().unwrap_or(0.0);
                    let lhs = stack.pop().unwrap_or(0.0);
                    stack.push(match b {
                        BinOp::Add => lhs + rhs,
                        BinOp::Sub => lhs - rhs,
                        BinOp::Mul => lhs * rhs,
                        BinOp::Div => {
                            if rhs == 0.0 {
                                0.0
                            } else {
                                lhs / rhs
                            }
                        }
                        BinOp::Min => lhs.min(rhs),
                        BinOp::Max => lhs.max(rhs),
                        BinOp::Gt => (lhs > rhs) as i32 as f32,
                        BinOp::Lt => (lhs < rhs) as i32 as f32,
                    });
                }
                Op::Clamp => {
                    let hi = stack.pop().unwrap_or(0.0);
                    let lo = stack.pop().unwrap_or(0.0);
                    let xv = stack.pop().unwrap_or(0.0);
                    stack.push(xv.clamp(lo.min(hi), lo.max(hi)));
                }
            }
        }
        stack.pop().unwrap_or(0.0)
    }

    /// Avanza **un tick** integrando `campo += dt·f` para cada ecuación, estilo Jacobi
    /// (todas leen el estado anterior). Determinista: independiente del orden de barrido.
    pub fn step(&mut self, program: &Program, params: &[f32], dt: f32) {
        // `next` parte del estado actual: los campos sin ecuación quedan intactos.
        self.next.copy_from_slice(&self.cur);
        let (dx, dy, dz) = (self.dim[0] as i32, self.dim[1] as i32, self.dim[2] as i32);
        let mut stack: Vec<f32> = Vec::with_capacity(16);
        for ec in &program.ecuaciones {
            let f = ec.campo as usize;
            let d = &self.defs[f];
            let base = f * self.ncells;
            for z in 0..dz {
                for y in 0..dy {
                    for x in 0..dx {
                        let i = self.idx(x as u32, y as u32, z as u32);
                        let cur_v = self.cur[base + i];
                        let rate = self.eval(&ec.ops, params, dt, x, y, z, &mut stack);
                        self.next[base + i] = (cur_v + dt * rate).clamp(d.min, d.max);
                    }
                }
            }
        }
        std::mem::swap(&mut self.cur, &mut self.next);
    }

    /// Suma de todos los valores de un campo (para tests de conservación).
    pub fn total(&self, f: FieldId) -> f32 {
        let base = (f as usize) * self.ncells;
        self.cur[base..base + self.ncells].iter().sum()
    }
}

// =============================================================================
//  Parser texto → Expr  y  printer Expr → texto  (base de las dos vistas)
// =============================================================================

/// Tabla de símbolos: resuelve nombres de campo y de parámetro a sus ids. La comparte
/// el editor de la Ley (viene de los `FieldDef` del material y sus params).
#[derive(Debug, Clone, Default)]
pub struct Symbols {
    pub campos: Vec<String>,
    pub params: Vec<String>,
}

impl Symbols {
    fn campo_id(&self, n: &str) -> Option<FieldId> {
        self.campos.iter().position(|c| c == n).map(|i| i as FieldId)
    }
    fn param_id(&self, n: &str) -> Option<ParamId> {
        self.params.iter().position(|p| p == n).map(|i| i as ParamId)
    }
    fn campo_nombre(&self, id: FieldId) -> String {
        self.campos.get(id as usize).cloned().unwrap_or_else(|| format!("campo{id}"))
    }
    fn param_nombre(&self, id: ParamId) -> String {
        self.params.get(id as usize).cloned().unwrap_or_else(|| format!("param{id}"))
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f32),
    Ident(String),
    LParen,
    RParen,
    Comma,
    Plus,
    Minus,
    Star,
    Slash,
    Lt,
    Gt,
}

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                toks.push(Tok::Comma);
                i += 1;
            }
            '+' => {
                toks.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                toks.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Star);
                i += 1;
            }
            '/' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '<' => {
                toks.push(Tok::Lt);
                i += 1;
            }
            '>' => {
                toks.push(Tok::Gt);
                i += 1;
            }
            _ if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
                    i += 1;
                }
                let txt: String = bytes[start..i].iter().collect();
                let n: f32 = txt.parse().map_err(|_| format!("número inválido: {txt}"))?;
                toks.push(Tok::Num(n));
            }
            _ if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
                toks.push(Tok::Ident(bytes[start..i].iter().collect()));
            }
            _ => return Err(format!("carácter inesperado: {c}")),
        }
    }
    Ok(toks)
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    sym: &'a Symbols,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat(&mut self, t: &Tok) -> Result<(), String> {
        if self.peek() == Some(t) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("se esperaba {t:?}, hay {:?}", self.peek()))
        }
    }

    // suma/resta (precedencia baja)
    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_term()?;
        while let Some(t) = self.peek() {
            let op = match t {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    // mult/div (precedencia alta)
    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_factor()?;
        while let Some(t) = self.peek() {
            let op = match t {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_factor()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_factor(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Minus) => Ok(Expr::Un(UnOp::Neg, Box::new(self.parse_factor()?))),
            Some(Tok::Num(n)) => Ok(Expr::Const(n)),
            Some(Tok::LParen) => {
                let e = self.parse_expr()?;
                self.eat(&Tok::RParen)?;
                Ok(e)
            }
            Some(Tok::Ident(name)) => {
                // ¿llamada a función?
                if self.peek() == Some(&Tok::LParen) {
                    self.pos += 1;
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        args.push(self.parse_expr()?);
                        while self.peek() == Some(&Tok::Comma) {
                            self.pos += 1;
                            args.push(self.parse_expr()?);
                        }
                    }
                    self.eat(&Tok::RParen)?;
                    self.build_call(&name, args)
                } else {
                    // constante especial, campo o parámetro
                    if name == "dt" {
                        return Ok(Expr::Dt);
                    }
                    if let Some(id) = self.sym.campo_id(&name) {
                        Ok(Expr::Field(id))
                    } else if let Some(id) = self.sym.param_id(&name) {
                        Ok(Expr::Param(id))
                    } else {
                        Err(format!("símbolo desconocido: {name}"))
                    }
                }
            }
            other => Err(format!("token inesperado: {other:?}")),
        }
    }

    /// Resuelve una llamada `nombre(args)` cuyo primer arg suele ser un campo.
    fn build_call(&self, name: &str, args: Vec<Expr>) -> Result<Expr, String> {
        // funciones sobre un campo (arg = nombre de campo → id)
        let campo_arg = |args: &[Expr]| -> Result<FieldId, String> {
            match args.first() {
                Some(Expr::Field(f)) => Ok(*f),
                _ => Err(format!("{name}(...) espera un campo como argumento")),
            }
        };
        match name {
            "lap" => Ok(Expr::Laplacian(campo_arg(&args)?)),
            "avg" => Ok(Expr::Vecinos(Reduce::Promedio, campo_arg(&args)?)),
            "sum6" => Ok(Expr::Vecinos(Reduce::Suma, campo_arg(&args)?)),
            "min6" => Ok(Expr::Vecinos(Reduce::Min, campo_arg(&args)?)),
            "max6" => Ok(Expr::Vecinos(Reduce::Max, campo_arg(&args)?)),
            "abs" => Ok(Expr::Un(UnOp::Abs, Box::new(single(name, args)?))),
            "exp" => Ok(Expr::Un(UnOp::Exp, Box::new(single(name, args)?))),
            "sqrt" => Ok(Expr::Un(UnOp::Sqrt, Box::new(single(name, args)?))),
            "min" => {
                let (a, b) = pair(name, args)?;
                Ok(Expr::Bin(BinOp::Min, Box::new(a), Box::new(b)))
            }
            "max" => {
                let (a, b) = pair(name, args)?;
                Ok(Expr::Bin(BinOp::Max, Box::new(a), Box::new(b)))
            }
            "clamp" => {
                if args.len() != 3 {
                    return Err("clamp(x, lo, hi) espera 3 argumentos".into());
                }
                let mut it = args.into_iter();
                Ok(Expr::Clamp(
                    Box::new(it.next().unwrap()),
                    Box::new(it.next().unwrap()),
                    Box::new(it.next().unwrap()),
                ))
            }
            _ => {
                if let Some(d) = Dir::de_nombre(name) {
                    Ok(Expr::Dir(d, campo_arg(&args)?))
                } else {
                    Err(format!("función desconocida: {name}"))
                }
            }
        }
    }
}

fn single(name: &str, args: Vec<Expr>) -> Result<Expr, String> {
    if args.len() != 1 {
        return Err(format!("{name}(x) espera 1 argumento"));
    }
    Ok(args.into_iter().next().unwrap())
}
fn pair(name: &str, args: Vec<Expr>) -> Result<(Expr, Expr), String> {
    if args.len() != 2 {
        return Err(format!("{name}(a, b) espera 2 argumentos"));
    }
    let mut it = args.into_iter();
    Ok((it.next().unwrap(), it.next().unwrap()))
}

impl Expr {
    /// Parsea una fórmula de texto a un [`Expr`], resolviendo campos y params por
    /// nombre con la tabla `sym`. Ej: `"grav*abajo(agua) + horiz*(avg(agua) - agua)"`.
    pub fn parse(src: &str, sym: &Symbols) -> Result<Expr, String> {
        let toks = lex(src)?;
        let mut p = Parser { toks, pos: 0, sym };
        let e = p.parse_expr()?;
        if p.pos != p.toks.len() {
            return Err(format!("sobra entrada tras la expresión (token {:?})", p.peek()));
        }
        Ok(e)
    }

    /// Imprime el `Expr` como fórmula de texto (inversa de [`parse`](Self::parse)),
    /// usando los nombres de `sym`. Base de la barra de fórmula y del round‑trip.
    pub fn to_source(&self, sym: &Symbols) -> String {
        self.print(sym, 0)
    }

    // prec: 0 top, 1 suma/cmp, 2 mult, 3 unario/atómico — para poner paréntesis mínimos
    fn print(&self, sym: &Symbols, prec: u8) -> String {
        match self {
            Expr::Const(c) => fmt_num(*c),
            Expr::Field(f) => sym.campo_nombre(*f),
            Expr::Param(p) => sym.param_nombre(*p),
            Expr::Dt => "dt".to_string(),
            Expr::Laplacian(f) => format!("lap({})", sym.campo_nombre(*f)),
            Expr::Vecinos(r, f) => format!("{}({})", r.nombre(), sym.campo_nombre(*f)),
            Expr::Dir(d, f) => format!("{}({})", d.nombre(), sym.campo_nombre(*f)),
            Expr::Un(UnOp::Neg, a) => {
                let s = format!("-{}", a.print(sym, 3));
                paren_if(prec > 1, s)
            }
            Expr::Un(op, a) => {
                let name = match op {
                    UnOp::Abs => "abs",
                    UnOp::Exp => "exp",
                    UnOp::Sqrt => "sqrt",
                    UnOp::Neg => unreachable!(),
                };
                format!("{name}({})", a.print(sym, 0))
            }
            Expr::Bin(op, a, b) => {
                let (sym_op, my_prec) = match op {
                    BinOp::Add => ("+", 1),
                    BinOp::Sub => ("-", 1),
                    BinOp::Lt => ("<", 1),
                    BinOp::Gt => (">", 1),
                    BinOp::Mul => ("*", 2),
                    BinOp::Div => ("/", 2),
                    // min/max/... como funciones
                    BinOp::Min => {
                        return format!("min({}, {})", a.print(sym, 0), b.print(sym, 0));
                    }
                    BinOp::Max => {
                        return format!("max({}, {})", a.print(sym, 0), b.print(sym, 0));
                    }
                };
                let s = format!("{} {sym_op} {}", a.print(sym, my_prec), b.print(sym, my_prec + 1));
                paren_if(prec > my_prec, s)
            }
            Expr::Clamp(x, lo, hi) => format!(
                "clamp({}, {}, {})",
                x.print(sym, 0),
                lo.print(sym, 0),
                hi.print(sym, 0)
            ),
        }
    }
}

fn fmt_num(c: f32) -> String {
    if c.fract() == 0.0 && c.abs() < 1e7 {
        format!("{}", c as i64)
    } else {
        let s = format!("{c}");
        s
    }
}
fn paren_if(cond: bool, s: String) -> String {
    if cond {
        format!("({s})")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(campos: &[&str], params: &[&str]) -> Symbols {
        Symbols {
            campos: campos.iter().map(|s| s.to_string()).collect(),
            params: params.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn round_trip_parse_print() {
        let s = sym(&["agua"], &["grav", "horiz"]);
        let casos = [
            "grav * abajo(agua) + horiz * (avg(agua) - agua)",
            "lap(agua) - agua * agua",
            "clamp(agua + dt, 0, 1)",
            "min(agua, 1) + max(agua, 0)",
        ];
        for c in casos {
            let e = Expr::parse(c, &s).expect("parsea");
            let printed = e.to_source(&s);
            let e2 = Expr::parse(&printed, &s).expect("re-parsea");
            assert_eq!(e, e2, "round-trip estable para: {c}  →  {printed}");
        }
    }

    #[test]
    fn parse_errores_utiles() {
        let s = sym(&["agua"], &[]);
        assert!(Expr::parse("desconocido", &s).is_err());
        assert!(Expr::parse("agua +", &s).is_err());
        assert!(Expr::parse("lap()", &s).is_err());
        assert!(Expr::parse("agua ) (", &s).is_err());
    }

    #[test]
    fn difusion_de_calor_conserva_y_converge() {
        // Un campo `t` con ecuación de calor: dt/dt = k*lap(t). Neumann conserva el
        // total y converge a uniforme.
        let s = sym(&["t"], &["k"]);
        let e = Expr::parse("k * lap(t)", &s).unwrap();
        let prog = Program::compile(&[Assign { campo: 0, expr: e }]);
        let mut eng = FieldEngine::new([8, 1, 8], vec![FieldDef::new("t", 0.0, 0.0, 1000.0)]);
        eng.set(0, 4, 0, 4, 100.0); // foco de calor
        let total0 = eng.total(0);
        for _ in 0..200 {
            eng.step(&prog, &[0.1], 1.0);
        }
        let total1 = eng.total(0);
        assert!((total0 - total1).abs() < 1e-2, "conserva calor: {total0} vs {total1}");
        // convergió: el foco se enfrió y el borde se calentó (menos varianza).
        let centro = eng.get(0, 4, 0, 4);
        let borde = eng.get(0, 0, 0, 0);
        assert!(centro < 50.0, "el foco se difundió (centro={centro})");
        assert!(borde > 0.1, "el calor llegó al borde (borde={borde})");
    }

    #[test]
    fn crecer_como_campo() {
        // La ley Crecer generalizada: la altura sube a `vel` hasta un tope. Ecuación:
        // dh/dt = vel * (h < tope).  (umbral → 0/1)
        let s = sym(&["h"], &["vel", "tope"]);
        let e = Expr::parse("vel * (h < tope)", &s).unwrap();
        let prog = Program::compile(&[Assign { campo: 0, expr: e }]);
        let mut eng = FieldEngine::new([1, 1, 1], vec![FieldDef::new("h", 0.0, 0.0, 100.0)]);
        for _ in 0..20 {
            eng.step(&prog, &[1.0, 5.0], 1.0);
        }
        // sube a razón 1/tick, se detiene al llegar a tope=5.
        let h = eng.get(0, 0, 0, 0);
        assert!((h - 5.0).abs() < 1.5, "creció y se detuvo cerca del tope: {h}");
    }

    #[test]
    fn reaccion_difusion_gray_scott_hace_patron() {
        // Gray–Scott: dos campos u,v.
        //   du/dt = Du*lap(u) - u*v*v + F*(1-u)
        //   dv/dt = Dv*lap(v) + u*v*v - (F+k)*v
        let s = sym(&["u", "v"], &["Du", "Dv", "F", "k"]);
        let eu = Expr::parse("Du * lap(u) - u * v * v + F * (1 - u)", &s).unwrap();
        let ev = Expr::parse("Dv * lap(v) + u * v * v - (F + k) * v", &s).unwrap();
        let prog = Program::compile(&[
            Assign { campo: 0, expr: eu },
            Assign { campo: 1, expr: ev },
        ]);
        let mut eng = FieldEngine::new(
            [32, 1, 32],
            vec![
                FieldDef::new("u", 1.0, 0.0, 1.0),
                FieldDef::new("v", 0.0, 0.0, 1.0),
            ],
        );
        // sembrar una mancha de v en el centro
        for z in 14..18 {
            for x in 14..18 {
                eng.set(1, x, 0, z, 0.5);
                eng.set(0, x, 0, z, 0.25);
            }
        }
        let params = [0.16, 0.08, 0.06, 0.062];
        for _ in 0..400 {
            eng.step(&prog, &params, 1.0);
        }
        // Hubo estructura: v tiene varianza espacial (no todo cero ni todo igual).
        let mut vmin = f32::INFINITY;
        let mut vmax = f32::NEG_INFINITY;
        for z in 0..32 {
            for x in 0..32 {
                let v = eng.get(1, x, 0, z);
                vmin = vmin.min(v);
                vmax = vmax.max(v);
                assert!(v.is_finite(), "estable numéricamente");
            }
        }
        assert!(vmax - vmin > 0.05, "reacción-difusión formó patrón (rango v={})", vmax - vmin);
    }

    #[test]
    fn determinismo_independiente_del_orden() {
        // Dos corridas idénticas dan exactamente lo mismo (Jacobi, sin azar).
        let s = sym(&["t"], &["k"]);
        let e = Expr::parse("k * lap(t)", &s).unwrap();
        let prog = Program::compile(&[Assign { campo: 0, expr: e }]);
        let run = || {
            let mut eng = FieldEngine::new([6, 1, 6], vec![FieldDef::new("t", 0.0, 0.0, 100.0)]);
            eng.set(0, 1, 0, 2, 50.0);
            eng.set(0, 4, 0, 3, 30.0);
            for _ in 0..50 {
                eng.step(&prog, &[0.2], 1.0);
            }
            (0..36).map(|i| eng.cur[i]).collect::<Vec<_>>()
        };
        assert_eq!(run(), run(), "misma evolución bit a bit");
    }
}
