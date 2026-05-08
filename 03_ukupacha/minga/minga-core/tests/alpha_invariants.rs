//! Invariantes del hash α-equivalente.
//!
//! El hash α debe ser estable bajo renombre de variables ligadas y romper
//! con cualquier cambio que afecte la *intención* del término: nombre de
//! la función, tipos en la firma, posición de argumentos, identidad de
//! variables libres.

use minga_core::{alpha::hash_node_alpha, parse};

#[test]
fn alpha_param_rename_invariant() {
    let a = parse::rust("fn f(x: i32) -> i32 { x + 1 }").unwrap();
    let b = parse::rust("fn f(y: i32) -> i32 { y + 1 }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_let_rename_invariant() {
    let a = parse::rust("fn f() -> i32 { let x = 1; x + 2 }").unwrap();
    let b = parse::rust("fn f() -> i32 { let y = 1; y + 2 }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_param_swap_with_rename_invariant() {
    let a = parse::rust("fn f(x: i32, y: i32) -> i32 { x - y }").unwrap();
    let b = parse::rust("fn f(a: i32, b: i32) -> i32 { a - b }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_shadowing_let_invariant() {
    let a = parse::rust("fn f() -> i32 { let x = 1; let x = x + 1; x }").unwrap();
    let b = parse::rust("fn f() -> i32 { let a = 1; let b = a + 1; b }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_function_name_matters() {
    let a = parse::rust("fn f(x: i32) -> i32 { x }").unwrap();
    let b = parse::rust("fn g(x: i32) -> i32 { x }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_signature_type_matters() {
    let a = parse::rust("fn f(x: i32) -> i32 { x }").unwrap();
    let b = parse::rust("fn f(x: i64) -> i64 { x }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_body_change_matters() {
    let a = parse::rust("fn f(x: i32) -> i32 { x + 1 }").unwrap();
    let b = parse::rust("fn f(x: i32) -> i32 { x + 2 }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_free_variable_identity_matters() {
    let a = parse::rust("fn f() { foo() }").unwrap();
    let b = parse::rust("fn f() { bar() }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_distinguishes_bound_vs_free() {
    // En el primero `x` es parámetro (ligado); en el segundo `x` es libre.
    let a = parse::rust("fn f(x: i32) -> i32 { x }").unwrap();
    let b = parse::rust("fn f() -> i32 { x }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_param_order_matters() {
    let a = parse::rust("fn f(x: i32, y: i32) -> i32 { x - y }").unwrap();
    let b = parse::rust("fn f(x: i32, y: i32) -> i32 { y - x }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_diverges_from_structural_under_rename() {
    // Bajo renombre, el hash estructural rompe pero el α se conserva. Esto
    // demuestra que α añade poder discriminatorio en una dimensión nueva
    // (intención) ortogonal a la sintaxis.
    use minga_core::cas::hash_node;
    let a = parse::rust("fn f(x: i32) -> i32 { x + 1 }").unwrap();
    let b = parse::rust("fn f(z: i32) -> i32 { z + 1 }").unwrap();
    assert_ne!(hash_node(&a), hash_node(&b));
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_closure_param_rename_invariant() {
    let a = parse::rust("fn f() -> i32 { let g = |x: i32| x + 1; g(0) }").unwrap();
    let b = parse::rust("fn f() -> i32 { let g = |y: i32| y + 1; g(0) }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_closure_captures_outer_binding() {
    // El cierre captura `z` (renombrable) del entorno; renombrar tanto el
    // exterior como el parámetro debe seguir produciendo el mismo hash.
    let a = parse::rust("fn f() -> i32 { let z = 1; let g = |x: i32| x + z; g(0) }").unwrap();
    let b = parse::rust("fn f() -> i32 { let q = 1; let g = |y: i32| y + q; g(0) }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_closure_distinguishes_captured_vs_free() {
    // En el primero `z` es ligado en el scope exterior (parámetro de `f`);
    // en el segundo `z` es libre. Aunque la forma del cierre coincide,
    // la identidad del término difiere.
    let a = parse::rust("fn f(z: i32) -> i32 { let g = |x: i32| x + z; g(0) }").unwrap();
    let b = parse::rust("fn f() -> i32 { let g = |x: i32| x + z; g(0) }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_for_loop_var_rename_invariant() {
    let a = parse::rust("fn f(v: Vec<i32>) -> i32 { let mut s = 0; for x in v { s += x } s }")
        .unwrap();
    let b = parse::rust("fn f(v: Vec<i32>) -> i32 { let mut s = 0; for y in v { s += y } s }")
        .unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_tuple_destructure_rename_invariant() {
    let a = parse::rust("fn f() -> i32 { let (a, b) = (1, 2); a + b }").unwrap();
    let b = parse::rust("fn f() -> i32 { let (x, y) = (1, 2); x + y }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_tuple_destructure_position_matters() {
    // (a, b) y (a, b) pero el cuerpo usa b - a vs a - b: distintos.
    let a = parse::rust("fn f() -> i32 { let (x, y) = (1, 2); x - y }").unwrap();
    let b = parse::rust("fn f() -> i32 { let (x, y) = (1, 2); y - x }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_mut_pattern_rename_invariant() {
    let a = parse::rust("fn f() -> i32 { let mut x = 1; x += 2; x }").unwrap();
    let b = parse::rust("fn f() -> i32 { let mut z = 1; z += 2; z }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_simple_arm_rename_invariant() {
    let a = parse::rust("fn f(v: i32) -> i32 { match v { x => x + 1, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { y => y + 1, _ => 0 } }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_arms_have_independent_scope() {
    // Arm 1 introduce `x`; arm 2 introduce `y`. Ambos renombrables sin
    // afectarse mutuamente.
    let a = parse::rust("fn f(v: i32) -> i32 { match v { x => x, y => y + 1, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { a => a, b => b + 1, _ => 0 } }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_constructor_distinguishes_arms() {
    // Some vs Ok: distintos constructores; el hash debe reflejarlo.
    let a =
        parse::rust("fn f(v: Option<i32>) -> i32 { match v { Some(x) => x, _ => 0 } }").unwrap();
    let b =
        parse::rust("fn f(v: Result<i32, ()>) -> i32 { match v { Ok(x) => x, _ => 0 } }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_tuple_struct_binder_rename_invariant() {
    let a =
        parse::rust("fn f(v: Option<i32>) -> i32 { match v { Some(x) => x + 1, None => 0 } }")
            .unwrap();
    let b =
        parse::rust("fn f(v: Option<i32>) -> i32 { match v { Some(y) => y + 1, None => 0 } }")
            .unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_struct_pattern_rename_invariant() {
    let a = parse::rust(
        "struct P{x:i32,y:i32} fn f(p: P) -> i32 { match p { P { x: a, y: b } => a + b } }",
    )
    .unwrap();
    let b = parse::rust(
        "struct P{x:i32,y:i32} fn f(p: P) -> i32 { match p { P { x: c, y: d } => c + d } }",
    )
    .unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_struct_pattern_field_name_matters() {
    // Renombrar el campo (la "x" antes del `:`) cambia la identidad: es
    // parte de la firma del struct, no un binder.
    let a = parse::rust(
        "struct P{x:i32,y:i32} fn f(p: P) -> i32 { match p { P { x: a, y: b } => a + b } }",
    )
    .unwrap();
    let b = parse::rust(
        "struct P{x:i32,y:i32} fn f(p: P) -> i32 { match p { P { y: a, x: b } => a + b } }",
    )
    .unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_guard_binder_rename_invariant() {
    let a = parse::rust("fn f(v: i32) -> i32 { match v { x if x > 0 => x, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { y if y > 0 => y, _ => 0 } }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_guard_op_distinguishes() {
    let a = parse::rust("fn f(v: i32) -> i32 { match v { x if x > 0 => x, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { x if x < 0 => x, _ => 0 } }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_captured_pattern_rename_invariant() {
    let a = parse::rust("fn f(v: i32) -> i32 { match v { n @ 1..=5 => n, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { m @ 1..=5 => m, _ => 0 } }").unwrap();
    assert_eq!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_captured_range_changes_hash() {
    let a = parse::rust("fn f(v: i32) -> i32 { match v { n @ 1..=5 => n, _ => 0 } }").unwrap();
    let b = parse::rust("fn f(v: i32) -> i32 { match v { n @ 1..=9 => n, _ => 0 } }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}

#[test]
fn alpha_match_constructor_vs_binder() {
    // En el primero, `None` es discriminator (mayúscula); en el segundo,
    // `x` es un catch-all binder. Estructural y semánticamente distintos.
    let a =
        parse::rust("fn f(v: Option<i32>) -> i32 { match v { None => 0, Some(z) => z } }").unwrap();
    let b =
        parse::rust("fn f(v: Option<i32>) -> i32 { match v { x => 0, Some(z) => z } }").unwrap();
    assert_ne!(hash_node_alpha(&a), hash_node_alpha(&b));
}
