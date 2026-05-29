//! Optimizaciones sobre el AST antes de compilar (D4 del roadmap).
//!
//! El optimizador hace dos pases acoplados:
//!
//!   1. **Constant folding**: combina operaciones cuyo árbol entero es
//!      constante. Por ejemplo `4 * 6 - 2` se reduce a `Num(22.0)` antes de
//!      pasar al bytecode. Sólo se foldean operaciones que el VM también
//!      sabría evaluar (potencias enteras 0..=12, suma/resta/producto/cociente
//!      sobre f32 finitos). Para divisiones por cero y sqrt/inv de números
//!      negativos: se preserva el nodo sin foldear — esos casos los detecta
//!      el caller en runtime con la semántica IEEE.
//!
//!   2. **Simplificación algebraica**: reescrituras locales que reducen
//!      profundidad de stack y número de ops:
//!        x + 0 → x          0 + x → x
//!        x - 0 → x          0 - x → -x
//!        x * 1 → x          1 * x → x
//!        x * 0 → 0          0 * x → 0
//!        x / 1 → x
//!        pow(x, 0) → 1      pow(x, 1) → x
//!        inv(inv(x)) → x    -(-x) → x
//!        inv(num) → 1/num   (cuando num != 0)
//!
//! Ambos pases se aplican hasta fix-point: tras una reescritura local pueden
//! aparecer nuevas oportunidades (p.ej. `inv(pow(x, 0))` → `inv(1)` → ...).
//!
//! La función `optimize` retorna un AST equivalente al original bajo la
//! semántica del VM con la salvedad que toda discrepancia numérica viene del
//! mismo pipeline `pow_int` que la VM (cero divergencia salvo errores f32
//! de orden epsilon).

use alloc::boxed::Box;

use crate::{BinOp, Expr, Func};

/// Punto de entrada: aplica fold + simplify a fix-point.
pub fn optimize(e: Expr) -> Expr {
    let mut prev = e;
    loop {
        let next = optimize_once(prev.clone());
        if next == prev { break next; }
        prev = next;
    }
}

fn optimize_once(e: Expr) -> Expr {
    match e {
        Expr::Num(_) | Expr::Var(_) => e,

        Expr::Neg(inner) => {
            let inner = optimize_once(*inner);
            match inner {
                Expr::Num(v) => Expr::Num(-v),
                Expr::Neg(x) => *x, // -(-x) → x
                other => Expr::Neg(Box::new(other)),
            }
        }

        Expr::Bin(op, a, b) => {
            let a = optimize_once(*a);
            let b = optimize_once(*b);
            simplify_bin(op, a, b)
        }

        Expr::Call(f, args) => {
            let args: alloc::vec::Vec<Expr> =
                args.into_iter().map(optimize_once).collect();
            simplify_call(f, args)
        }
    }
}

fn simplify_bin(op: BinOp, a: Expr, b: Expr) -> Expr {
    // Fold puro: ambas hojas son números.
    if let (Expr::Num(x), Expr::Num(y)) = (&a, &b) {
        if let Some(v) = fold_bin(op, *x, *y) {
            return Expr::Num(v);
        }
    }
    // Simplificaciones algebraicas. Cada brazo cubre un patrón concreto;
    // mantenemos `match` exhaustivo por operador para que el lector vea
    // las reescrituras agrupadas.
    match (op, &a, &b) {
        // Add
        (BinOp::Add, Expr::Num(z), _) if *z == 0.0 => b,
        (BinOp::Add, _, Expr::Num(z)) if *z == 0.0 => a,
        // Sub
        (BinOp::Sub, _, Expr::Num(z)) if *z == 0.0 => a,
        (BinOp::Sub, Expr::Num(z), _) if *z == 0.0 => Expr::Neg(Box::new(b)),
        // Mul
        (BinOp::Mul, Expr::Num(o), _) if *o == 1.0 => b,
        (BinOp::Mul, _, Expr::Num(o)) if *o == 1.0 => a,
        (BinOp::Mul, Expr::Num(z), _) if *z == 0.0 => Expr::Num(0.0),
        (BinOp::Mul, _, Expr::Num(z)) if *z == 0.0 => Expr::Num(0.0),
        // Div
        (BinOp::Div, _, Expr::Num(o)) if *o == 1.0 => a,
        // 0/x: no foldear; mantener la división porque si x=0 el comportamiento
        // IEEE NaN debe preservarse en runtime.
        _ => Expr::Bin(op, Box::new(a), Box::new(b)),
    }
}

fn fold_bin(op: BinOp, x: f32, y: f32) -> Option<f32> {
    let v = match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => {
            if y == 0.0 { return None; } // dejar div-by-zero para runtime
            x / y
        }
    };
    if v.is_finite() { Some(v) } else { None }
}

fn simplify_call(f: Func, args: alloc::vec::Vec<Expr>) -> Expr {
    match (f, args.as_slice()) {
        // pow(x, 0) → 1
        (Func::Pow, [_, Expr::Num(n)]) if *n == 0.0 => Expr::Num(1.0),
        // pow(x, 1) → x
        (Func::Pow, [x, Expr::Num(n)]) if *n == 1.0 => x.clone(),
        // pow(num_x, num_n) entero 0..=12 → fold
        (Func::Pow, [Expr::Num(x), Expr::Num(n)]) => {
            if *n >= 0.0 && *n <= 12.0 && (*n as i32) as f32 == *n {
                let mut acc = 1.0f32;
                let mut i = *n as u32;
                while i > 0 { acc *= *x; i -= 1; }
                Expr::Num(acc)
            } else {
                Expr::Call(f, args)
            }
        }
        // inv(inv(x)) → x
        (Func::Inv, [Expr::Call(Func::Inv, inner)]) if inner.len() == 1 => {
            inner[0].clone()
        }
        // inv(num) → 1/num para num != 0
        (Func::Inv, [Expr::Num(x)]) if *x != 0.0 => Expr::Num(1.0 / *x),
        // sqrt(num) para num >= 0: foldear con NR. Reusa el método del VM.
        (Func::Sqrt, [Expr::Num(x)]) if *x >= 0.0 => Expr::Num(nr_sqrt(*x)),
        _ => Expr::Call(f, args),
    }
}

/// Newton-Raphson para `sqrt` — mismo método que el VM, para coherencia bit-a-bit
/// entre el fold y el eval runtime.
fn nr_sqrt(x: f32) -> f32 {
    if x == 0.0 { return 0.0; }
    let mut g = f32::from_bits(0x5f37_5a86u32.wrapping_sub(x.to_bits() >> 1));
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    g = g * (1.5 - 0.5 * x * g * g);
    x * g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, Var};
    use alloc::vec;

    fn opt(src: &str) -> Expr {
        optimize(parse(src).unwrap())
    }

    #[test]
    fn constant_arithmetic_folds_to_single_num() {
        assert_eq!(opt("1 + 2 * 3"), Expr::Num(7.0));
        assert_eq!(opt("(1 + 2) * 3"), Expr::Num(9.0));
        assert_eq!(opt("10 - 2 - 1"), Expr::Num(7.0));
        assert_eq!(opt("-5 + 2"), Expr::Num(-3.0));
    }

    #[test]
    fn add_zero_drops_to_other_operand() {
        assert_eq!(opt("r + 0"), Expr::Var(Var::R));
        assert_eq!(opt("0 + r"), Expr::Var(Var::R));
    }

    #[test]
    fn mul_zero_collapses_to_zero() {
        assert_eq!(opt("r * 0"), Expr::Num(0.0));
        assert_eq!(opt("0 * r2"), Expr::Num(0.0));
    }

    #[test]
    fn mul_one_drops_to_other_operand() {
        assert_eq!(opt("r2 * 1"), Expr::Var(Var::R2));
        assert_eq!(opt("1 * sigma"), Expr::Var(Var::Sigma));
    }

    #[test]
    fn div_one_drops_to_dividend() {
        assert_eq!(opt("r / 1"), Expr::Var(Var::R));
    }

    #[test]
    fn pow_zero_one_special_cases() {
        assert_eq!(opt("pow(r, 0)"), Expr::Num(1.0));
        assert_eq!(opt("pow(r, 1)"), Expr::Var(Var::R));
    }

    #[test]
    fn pow_constant_integer_folds() {
        assert_eq!(opt("pow(2, 12)"), Expr::Num(4096.0));
        assert_eq!(opt("pow(3, 3)"), Expr::Num(27.0));
    }

    #[test]
    fn double_inv_eliminates() {
        assert_eq!(opt("inv(inv(r2))"), Expr::Var(Var::R2));
    }

    #[test]
    fn double_neg_eliminates() {
        // -(-(r)) → r
        assert_eq!(opt("-(-r)"), Expr::Var(Var::R));
    }

    #[test]
    fn lj_partial_fold_collapses_constants() {
        // 24·eps·(2·(σ/r)^12 − (σ/r)^6) · inv(r²)
        // El subnodo `24` queda como Num, `2` queda como Num. Lo importante:
        // el AST resultante debe seguir siendo equivalente al original.
        let opt_ast = opt("24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)");
        // Verificación blanda: el AST se reduce vs el original (no debe crecer)
        // y permanece evaluable. La equivalencia numérica se prueba en
        // `optimize_preserves_lj_numeric`.
        // Forma raíz: una mul; rhs es inv(r2).
        if let Expr::Bin(BinOp::Mul, _, rhs) = &opt_ast {
            assert_eq!(**rhs, Expr::Call(Func::Inv, vec![Expr::Var(Var::R2)]));
        } else {
            panic!("forma inesperada: {opt_ast:?}");
        }
    }

    #[test]
    fn optimize_is_idempotent() {
        // Aplicar dos veces debe dar el mismo AST: el fix-point ya se alcanzó.
        let once = opt("24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)");
        let twice = optimize(once.clone());
        assert_eq!(once, twice);
    }

    #[test]
    fn optimize_preserves_lj_numeric() {
        // Mismo `eval` antes y después de optimize.
        use crate::{compile, eval_with_stack, VarBindings};
        let src = "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)";
        let raw = parse(src).unwrap();
        let opt = optimize(raw.clone());
        let bc_raw = compile(&raw).unwrap();
        let bc_opt = compile(&opt).unwrap();

        for &r in &[0.95f32, 1.0, 1.122_46, 1.5, 2.0, 2.4] {
            let r2 = r * r;
            let v = VarBindings { r, r2, eps: 1.0, sigma: 1.0, ..Default::default() };
            let mut s1 = [0.0; 32];
            let mut s2 = [0.0; 32];
            let a = eval_with_stack(&bc_raw, &v, &mut s1).unwrap();
            let b = eval_with_stack(&bc_opt, &v, &mut s2).unwrap();
            // Identidad bit-a-bit no garantizada por el orden de fold de pow_int
            // (asociatividad de productos f32 puede divergir); tolerancia laxa.
            assert!((a - b).abs() <= a.abs().max(1.0) * 1e-3, "Δ en r={r}: a={a} b={b}");
        }
    }

    #[test]
    fn optimize_reduces_op_count() {
        use crate::compile;
        let src = "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)";
        let raw_ast = parse(src).unwrap();
        let opt_ast = optimize(raw_ast.clone());
        let bc_raw = compile(&raw_ast).unwrap();
        let bc_opt = compile(&opt_ast).unwrap();
        // Optimizado nunca puede tener MÁS ops que el crudo.
        assert!(
            bc_opt.code.len() <= bc_raw.code.len(),
            "optimize creció ops: {} → {}",
            bc_raw.code.len(), bc_opt.code.len()
        );
    }

    /// Defensa contra regresión: que el optimize no infle el pool de
    /// constantes en ninguna de las fórmulas canónicas del bench. Si un día
    /// alguien agrega una regla que rompe esto (porque pierde el dedupe o
    /// genera nuevos `Num` no consolidados), el test grita.
    #[test]
    fn optimize_never_grows_const_pool_on_canonical_formulas() {
        use crate::compile;
        let cases: &[(&str, &str)] = &[
            ("lj",      "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)"),
            ("coulomb", "qi * qj * inv(r2) * sqrt(r2)"),
            ("hooke",   "-100.0 * (r - 1.5)"),
        ];
        for (name, src) in cases {
            let raw = parse(src).unwrap();
            let opt = optimize(raw.clone());
            let bc_raw = compile(&raw).unwrap();
            let bc_opt = compile(&opt).unwrap();
            assert!(
                bc_opt.consts.len() <= bc_raw.consts.len(),
                "{name}: optimize creció pool de consts {} → {}",
                bc_raw.consts.len(), bc_opt.consts.len()
            );
            assert!(
                bc_opt.code.len() <= bc_raw.code.len(),
                "{name}: optimize creció code {} → {}",
                bc_raw.code.len(), bc_opt.code.len()
            );
        }
    }
}
