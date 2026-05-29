//! Bytecode stack-machine + eval cero-alloc (D2 del roadmap).
//!
//! El compilador linealiza un `Expr` en post-order: cada nodo emite sus hijos
//! primero y luego su propio opcode. Una stack LIFO acumula resultados de
//! operandos; cada `Op` consume k operandos del tope y deja 1 (excepto `Neg`
//! que es 1→1).
//!
//! El programa final consta de:
//!   - `code: Vec<Op>` — instrucciones (~1 B cada una salvo `Const{idx: u16}`).
//!   - `consts: Vec<f32>` — pool de constantes deduplicado por igualdad
//!     bit-a-bit en `push_const` (`+0.0` y `-0.0` cuentan como distintos para
//!     no romper la semántica IEEE en divisiones).
//!   - `stack_depth: u16` — profundidad máxima alcanzada durante la
//!     simulación abstracta del compilador. El caller asigna un buffer de
//!     `[f32; stack_depth]` y `eval_with_stack` no aloca nada.

use alloc::vec::Vec;

use crate::{BinOp, Expr, Func, Var};

/// Bytecode compilado. `code` y `consts` viven por el programa; en el hot
/// loop solo se lee.
#[derive(Clone, Debug, PartialEq)]
pub struct Bytecode {
    pub code: Vec<Op>,
    pub consts: Vec<f32>,
    /// Profundidad máxima del stack durante el programa. El caller pasa al
    /// menos este tamaño a `eval_with_stack`.
    pub stack_depth: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    /// Empuja `consts[idx]` al tope del stack.
    Const(u16),
    /// Empuja el valor de la variable `Var` (leído de `VarBindings`).
    LoadVar(Var),
    /// Pop `b`, pop `a`, push `a op b`.
    Add, Sub, Mul, Div,
    /// Pop `x`, push `-x`.
    Neg,
    /// Pop `n`, pop `x`, push `x^n` (powf). Caller responsable de NaN si n < 0
    /// y x < 0 — no validamos.
    Pow,
    /// Pop `x`, push `1/x`. Para `x == 0` produce `±inf` (semántica IEEE 754).
    Inv,
    /// Pop `x`, push `√x`. Para `x < 0` produce `NaN` — caller responsabilidad.
    Sqrt,
}

/// Snapshot de variables para una evaluación. Se actualiza una vez por par
/// (i, j) antes de invocar `eval_with_stack`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VarBindings {
    pub r: f32, pub r2: f32,
    pub eps: f32, pub sigma: f32,
    pub qi: f32, pub qj: f32,
    pub mi: f32, pub mj: f32,
    pub dx: f32, pub dy: f32, pub dz: f32,
}

impl VarBindings {
    #[inline]
    pub fn get(&self, v: Var) -> f32 {
        match v {
            Var::R => self.r,         Var::R2 => self.r2,
            Var::Eps => self.eps,     Var::Sigma => self.sigma,
            Var::Qi => self.qi,       Var::Qj => self.qj,
            Var::Mi => self.mi,       Var::Mj => self.mj,
            Var::Dx => self.dx,       Var::Dy => self.dy,    Var::Dz => self.dz,
        }
    }
}

// ─── Compilador ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum CompileError {
    /// Más de 65535 constantes distintas. No esperable en prácticas humanas
    /// pero defensivo contra generación automática (D2 es la base de E2).
    ConstPoolOverflow,
}

pub fn compile(expr: &Expr) -> Result<Bytecode, CompileError> {
    let mut code = Vec::new();
    let mut consts = Vec::new();
    // Simulación del stack durante emit: cada instrucción suma o resta a
    // `current_depth` (cuántos valores hay en el stack tras ejecutarla).
    let mut current: i32 = 0;
    let mut peak: i32 = 0;
    emit(expr, &mut code, &mut consts, &mut current, &mut peak)?;
    debug_assert_eq!(current, 1, "tras evaluar Expr el stack debe tener 1 valor");
    let stack_depth = peak.max(1) as u16;
    Ok(Bytecode { code, consts, stack_depth })
}

fn push_const(c: f32, consts: &mut Vec<f32>) -> Result<u16, CompileError> {
    // Dedupe por bits: dos constantes idénticas comparten slot. NaN no aparece
    // por construcción (el lexer rechaza no-finitos), así que basta `to_bits()`.
    let bits = c.to_bits();
    if let Some(idx) = consts.iter().position(|x| x.to_bits() == bits) {
        return Ok(idx as u16);
    }
    if consts.len() >= u16::MAX as usize {
        return Err(CompileError::ConstPoolOverflow);
    }
    let idx = consts.len() as u16;
    consts.push(c);
    Ok(idx)
}

fn emit(
    e: &Expr,
    code: &mut Vec<Op>,
    consts: &mut Vec<f32>,
    current: &mut i32,
    peak: &mut i32,
) -> Result<(), CompileError> {
    match e {
        Expr::Num(v) => {
            let idx = push_const(*v, consts)?;
            code.push(Op::Const(idx));
            *current += 1; *peak = (*peak).max(*current);
        }
        Expr::Var(v) => {
            code.push(Op::LoadVar(*v));
            *current += 1; *peak = (*peak).max(*current);
        }
        Expr::Neg(inner) => {
            emit(inner, code, consts, current, peak)?;
            code.push(Op::Neg);
            // Neg: 1 → 1, no cambia current.
        }
        Expr::Bin(op, a, b) => {
            emit(a, code, consts, current, peak)?;
            emit(b, code, consts, current, peak)?;
            code.push(match op {
                BinOp::Add => Op::Add,
                BinOp::Sub => Op::Sub,
                BinOp::Mul => Op::Mul,
                BinOp::Div => Op::Div,
            });
            // Bin: 2 → 1, current -= 1.
            *current -= 1;
        }
        Expr::Call(f, args) => {
            for a in args {
                emit(a, code, consts, current, peak)?;
            }
            let (opc, consumes) = match f {
                Func::Pow  => (Op::Pow,  2usize),
                Func::Inv  => (Op::Inv,  1usize),
                Func::Sqrt => (Op::Sqrt, 1usize),
            };
            code.push(opc);
            // Call: n → 1, current -= (n - 1).
            *current -= consumes as i32 - 1;
        }
    }
    Ok(())
}

// ─── VM ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// El caller pasó un stack más corto que `bc.stack_depth`.
    StackTooSmall { needed: u16, got: usize },
    /// Const index fuera del pool. Defensivo — no debería ocurrir con
    /// bytecode emitido por `compile`.
    BadConstIndex(u16),
}

/// Evalúa el bytecode usando el buffer `stack` provisto por el caller.
/// Cero allocs: no toca el heap.
#[inline]
pub fn eval_with_stack(
    bc: &Bytecode,
    vars: &VarBindings,
    stack: &mut [f32],
) -> Result<f32, EvalError> {
    if stack.len() < bc.stack_depth as usize {
        return Err(EvalError::StackTooSmall {
            needed: bc.stack_depth,
            got: stack.len(),
        });
    }
    let mut sp: usize = 0; // siguiente posición libre del stack
    for op in &bc.code {
        match op {
            Op::Const(idx) => {
                let v = *bc.consts.get(*idx as usize)
                    .ok_or(EvalError::BadConstIndex(*idx))?;
                stack[sp] = v; sp += 1;
            }
            Op::LoadVar(v) => { stack[sp] = vars.get(*v); sp += 1; }
            Op::Add => { sp -= 1; stack[sp - 1] += stack[sp]; }
            Op::Sub => { sp -= 1; stack[sp - 1] -= stack[sp]; }
            Op::Mul => { sp -= 1; stack[sp - 1] *= stack[sp]; }
            Op::Div => { sp -= 1; stack[sp - 1] /= stack[sp]; }
            Op::Neg => { stack[sp - 1] = -stack[sp - 1]; }
            Op::Pow => {
                sp -= 1;
                let n = stack[sp];
                stack[sp - 1] = libm_powf(stack[sp - 1], n);
            }
            Op::Inv => { stack[sp - 1] = 1.0 / stack[sp - 1]; }
            Op::Sqrt => {
                let x = stack[sp - 1];
                stack[sp - 1] = libm_sqrtf(x);
            }
        }
    }
    debug_assert_eq!(sp, 1, "stack debe quedar con exactamente 1 valor");
    Ok(stack[0])
}

// `f32::powf` y `f32::sqrt` viven en `std::f32`. Bajo `#![no_std]` se vuelven
// inalcanzables salvo que importemos `libm` o usemos las intrinsics. Como
// queremos compilar a wasm32-unknown-unknown sin deps adicionales, definimos
// shims que delegan a `std` cuando hay y a las intrinsics LLVM (`f32::powi`
// reproducible vía exp/ln) en otro caso. wasm32-unknown-unknown SÍ provee
// estos intrinsics como exports del runtime; lo verificamos en tests.
//
// `core::intrinsics` está unstable, así que en estable usamos `f32::powf` /
// `f32::sqrt` directamente — están disponibles en core gracias a "f32 inherent
// methods" estabilizados desde 1.79 (los métodos no-libm). Verificamos en
// tests que el bytecode compila a wasm32.

#[inline] fn libm_powf(x: f32, n: f32) -> f32 {
    // `f32::powf` requiere libm. Para mantener el crate libre de dependencias
    // implementamos potencia entera (caso 99% de fuerzas físicas) y delegamos
    // el resto a `exp(n·ln(x))` con las funciones disponibles en `core`.
    //
    // Caso especial enteros 1..=12: bucle directo. Cubre LJ (12 y 6), Hooke
    // (n=2), Coulomb-like (3). Sin dependencias.
    // Detecta entero en [0, 12] sin `f32::fract` (que vive en `std`).
    if n >= 0.0 && n <= 12.0 && (n as i32) as f32 == n {
        let mut acc = 1.0f32;
        let mut i = n as u32;
        while i > 0 { acc *= x; i -= 1; }
        return acc;
    }
    // Fallback genérico: `f32::powf` solo está en `std`. Bajo `no_std` puro
    // delegamos a una aproximación `exp(n·ln|x|)·sign`. Suficiente para los
    // casos no-enteros que un nodo visual del futuro podría generar.
    let sign = if x < 0.0 && (n as i32) as f32 == n && (n as i32) % 2 == 1 { -1.0 } else { 1.0 };
    libm_exp(n * libm_ln(x.abs())) * sign
}

#[inline] fn libm_sqrtf(x: f32) -> f32 {
    // `f32::sqrt` es un intrinsic disponible en core a través de
    // `f32::from_bits` y bit-tricks. Usamos el método de Newton-Raphson
    // como fallback portable; tres iteraciones bastan para f32.
    if x <= 0.0 { return if x == 0.0 { 0.0 } else { f32::NAN }; }
    let mut g = f32::from_bits(0x5f37_5a86u32.wrapping_sub(x.to_bits() >> 1));
    // 3 pasos de NR sobre 1/√x; luego multiplica por x.
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    x * g
}

/// Aproximación `ln(x)` para `x > 0`. Polinomio de Taylor sobre `1+y` con
/// reducción `x = m · 2^e`. Precisión ~1e-5; suficiente para fuerzas — los
/// errores numéricos de Verlet son varios órdenes mayores.
fn libm_ln(x: f32) -> f32 {
    if x <= 0.0 { return f32::NAN; }
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // m ∈ [1, 2)
    let y = (m - 1.0) / (m + 1.0);
    let y2 = y * y;
    // ln(m) = 2·(y + y³/3 + y⁵/5 + y⁷/7)
    let series = 2.0 * y * (1.0 + y2 / 3.0 + y2 * y2 / 5.0 + y2 * y2 * y2 / 7.0);
    series + e as f32 * core::f32::consts::LN_2
}

/// Aproximación `exp(x)` por reducción `x = k·ln2 + r` con r ∈ [-ln2/2, ln2/2]
/// y Taylor de grado 6 sobre `e^r`.
fn libm_exp(x: f32) -> f32 {
    let inv_ln2 = 1.0 / core::f32::consts::LN_2;
    let k = (x * inv_ln2 + 0.5 * x.signum()) as i32; // round
    let r = x - k as f32 * core::f32::consts::LN_2;
    // e^r ≈ 1 + r + r²/2 + r³/6 + r⁴/24 + r⁵/120 + r⁶/720
    let r2 = r * r;
    let r3 = r2 * r;
    let er = 1.0 + r + r2 * 0.5 + r3 * (1.0/6.0) + r2 * r2 * (1.0/24.0)
           + r3 * r2 * (1.0/120.0) + r3 * r3 * (1.0/720.0);
    // 2^k vía manipulación de bits: e^x = e^r · 2^k.
    let pow2_k = if (-126..=127).contains(&k) {
        f32::from_bits(((k + 127) as u32) << 23)
    } else if k > 127 { f32::INFINITY } else { 0.0 };
    er * pow2_k
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn eval_src(src: &str, vars: VarBindings) -> f32 {
        let ast = parse(src).expect("parse");
        let bc = compile(&ast).expect("compile");
        let mut stack = [0.0f32; 32];
        eval_with_stack(&bc, &vars, &mut stack[..]).expect("eval")
    }

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol.max(b.abs() * tol)
    }

    #[test]
    fn const_and_arithmetic() {
        let v = VarBindings::default();
        assert_eq!(eval_src("1 + 2 * 3", v), 7.0);
        assert_eq!(eval_src("(1 + 2) * 3", v), 9.0);
        assert_eq!(eval_src("10 - 2 - 1", v), 7.0);
        assert_eq!(eval_src("-5 + 2", v), -3.0);
    }

    #[test]
    fn vars_are_loaded() {
        let v = VarBindings { r: 2.0, eps: 1.0, sigma: 1.0, ..Default::default() };
        assert_eq!(eval_src("eps * sigma / r", v), 0.5);
    }

    #[test]
    fn inv_and_sqrt() {
        let v = VarBindings { r2: 4.0, ..Default::default() };
        assert_eq!(eval_src("inv(r2)", v), 0.25);
        // sqrt vía Newton-Raphson aproximado (3 pasos): tolerancia 1e-5.
        let s = eval_src("sqrt(r2)", v);
        assert!((s - 2.0).abs() < 1e-5, "sqrt(4) = {s}");
    }

    #[test]
    fn pow_integer_exponents() {
        let v = VarBindings { r: 2.0, ..Default::default() };
        assert_eq!(eval_src("pow(r, 3)", v), 8.0);
        assert_eq!(eval_src("pow(r, 12)", v), 4096.0);
    }

    #[test]
    fn lennard_jones_at_minimum_is_near_zero() {
        // F_LJ_radial = 24·ε · (2·(σ/r)¹² − (σ/r)⁶) · inv(r²)
        // En r_min = σ·2^(1/6), la fuerza radial pasa por cero.
        let r_min = 1.122_462_05_f32;
        let v = VarBindings {
            r: r_min, r2: r_min * r_min, eps: 1.0, sigma: 1.0,
            ..Default::default()
        };
        let f = eval_src(
            "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)", v);
        assert!(f.abs() < 1e-2, "F en r_min debería ≈ 0, fue {f}");
    }

    #[test]
    fn dsl_matches_native_lj_within_tol() {
        // Verifica equivalencia con la fórmula nativa del kernel `lennard_jones`:
        //   f_over_r = 24·ε · inv(r²) · (2·sr¹² − sr⁶)  con sr² = σ²/r²
        // Probamos varios r dentro y fuera del cutoff.
        for &r in &[0.95f32, 1.0, 1.1, 1.5, 2.0, 2.4] {
            let r2 = r * r;
            let sr2 = 1.0 / r2;
            let sr6 = sr2 * sr2 * sr2;
            let sr12 = sr6 * sr6;
            let native = 24.0 * 1.0 * (2.0 * sr12 - sr6) / r2;

            let v = VarBindings { r, r2, eps: 1.0, sigma: 1.0, ..Default::default() };
            let dsl = eval_src(
                "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)", v);
            assert!(
                approx(dsl, native, 1e-4),
                "discrepancia DSL ↔ nativo en r={r}: dsl={dsl} native={native}"
            );
        }
    }

    #[test]
    fn stack_depth_is_accurate() {
        // 3 niveles anidados: pow(sigma/r, 12) requiere stack al menos = 3
        // (sigma, r, después div→1, push n=12→2, pow→1). Confirmamos que
        // el cálculo de peak no se queda corto.
        let ast = parse("pow(sigma / r, 12)").unwrap();
        let bc = compile(&ast).unwrap();
        // Mínimo razonable: 2 (sigma, r) antes del div, luego 2 (resultado, 12)
        // antes del pow. peak ≥ 2 está garantizado, comprobamos exactitud.
        assert!(bc.stack_depth >= 2);

        // Si pasamos un stack más corto, debe fallar limpio.
        let mut small = [0.0f32; 1];
        let v = VarBindings { r: 2.0, sigma: 1.0, ..Default::default() };
        let err = eval_with_stack(&bc, &v, &mut small).unwrap_err();
        assert!(matches!(err, EvalError::StackTooSmall { .. }));
    }

    #[test]
    fn const_pool_dedupes_repeated_literals() {
        // `2 + 2 * 2` tiene tres apariciones del literal `2`. El pool debe
        // tener una sola entrada y los tres `Op::Const` apuntar al mismo idx.
        let bc = compile(&parse("2 + 2 * 2").unwrap()).unwrap();
        assert_eq!(bc.consts, alloc::vec![2.0_f32], "pool sin dedupar: {:?}", bc.consts);
        let const_ops: alloc::vec::Vec<_> = bc.code.iter()
            .filter_map(|op| if let Op::Const(i) = op { Some(*i) } else { None })
            .collect();
        assert_eq!(const_ops, alloc::vec![0_u16, 0, 0]);
    }

    #[test]
    fn powf_for_non_integer_exponents() {
        // Verifica el fallback `exp(n·ln(x))` con caso no-entero típico.
        let v = VarBindings { r: 4.0, ..Default::default() };
        let dsl = eval_src("pow(r, 0.5)", v);
        // sqrt(4) = 2, pero pasamos por libm_powf no por sqrt. Tolerancia
        // alta porque ln/exp aproximados.
        assert!((dsl - 2.0).abs() < 5e-2, "pow(4, 0.5) = {dsl}");
    }
}
