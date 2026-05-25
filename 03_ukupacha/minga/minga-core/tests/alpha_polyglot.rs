//! α-equivalencia para Python, TypeScript, JavaScript, Go.
//!
//! Mismas propiedades que `alpha_invariants.rs` para Rust:
//! - Renombre de variables ligadas → mismo hash.
//! - Cambio de estructura / nombres libres → hash distinto.

use minga_core::alpha::hash_alpha_with;
use minga_core::parse::Dialect;

fn h(d: Dialect, src: &str) -> minga_core::cas::ContentHash {
    let n = d.parse(src).expect("parse OK");
    hash_alpha_with(d, &n)
}

// ============================================================================
// Python
// ============================================================================

#[test]
fn python_def_param_rename_invariant() {
    let a = h(Dialect::Python, "def f(x):\n    return x + 1\n");
    let b = h(Dialect::Python, "def f(y):\n    return y + 1\n");
    assert_eq!(a, b);
}

#[test]
fn python_def_function_name_matters() {
    let a = h(Dialect::Python, "def f(x):\n    return x\n");
    let b = h(Dialect::Python, "def g(x):\n    return x\n");
    assert_ne!(a, b, "el nombre de la función NO es α-anónimo");
}

#[test]
fn python_lambda_rename_invariant() {
    let a = h(Dialect::Python, "f = lambda x: x + 1\n");
    let b = h(Dialect::Python, "f = lambda y: y + 1\n");
    assert_eq!(a, b);
}

#[test]
fn python_for_loop_rename_invariant() {
    let a = h(
        Dialect::Python,
        "for x in xs:\n    print(x)\n",
    );
    let b = h(
        Dialect::Python,
        "for y in xs:\n    print(y)\n",
    );
    assert_eq!(a, b);
}

#[test]
fn python_for_iterable_name_matters() {
    let a = h(
        Dialect::Python,
        "for x in xs:\n    print(x)\n",
    );
    let b = h(
        Dialect::Python,
        "for x in ys:\n    print(x)\n",
    );
    assert_ne!(a, b, "el iterable es variable libre, su nombre importa");
}

#[test]
fn python_list_comprehension_rename_invariant() {
    let a = h(Dialect::Python, "result = [x*2 for x in xs]\n");
    let b = h(Dialect::Python, "result = [y*2 for y in xs]\n");
    assert_eq!(a, b);
}

#[test]
fn python_nested_comprehension_rename_invariant() {
    // Doble for_in_clause: x e y son binders.
    let a = h(
        Dialect::Python,
        "result = [(x, y) for x in xs for y in ys]\n",
    );
    let b = h(
        Dialect::Python,
        "result = [(a, b) for a in xs for b in ys]\n",
    );
    assert_eq!(a, b);
}

#[test]
fn python_with_statement_rename_invariant() {
    let a = h(
        Dialect::Python,
        "with open(p) as f:\n    f.read()\n",
    );
    let b = h(
        Dialect::Python,
        "with open(p) as g:\n    g.read()\n",
    );
    assert_eq!(a, b);
}

#[test]
fn python_lambda_does_not_collide_with_unrelated() {
    let plus = h(Dialect::Python, "f = lambda x: x + 1\n");
    let minus = h(Dialect::Python, "f = lambda x: x - 1\n");
    assert_ne!(plus, minus, "operación distinta debe dar hash distinto");
}

// ============================================================================
// JavaScript / TypeScript (mismo profile)
// ============================================================================

#[test]
fn js_function_rename_invariant() {
    let a = h(Dialect::JavaScript, "function f(x) { return x + 1; }");
    let b = h(Dialect::JavaScript, "function f(y) { return y + 1; }");
    assert_eq!(a, b);
}

#[test]
fn js_function_name_matters() {
    let a = h(Dialect::JavaScript, "function f(x) { return x; }");
    let b = h(Dialect::JavaScript, "function g(x) { return x; }");
    assert_ne!(a, b);
}

#[test]
fn js_arrow_function_rename_invariant() {
    let a = h(Dialect::JavaScript, "const f = (x) => x + 1;");
    let b = h(Dialect::JavaScript, "const f = (y) => y + 1;");
    assert_eq!(a, b);
}

#[test]
fn js_arrow_shorthand_rename_invariant() {
    // `x => ...` (sin paréntesis) — single identifier.
    let a = h(Dialect::JavaScript, "const f = x => x + 1;");
    let b = h(Dialect::JavaScript, "const f = y => y + 1;");
    assert_eq!(a, b);
}

#[test]
fn js_let_const_rename_invariant() {
    let a = h(Dialect::JavaScript, "function f() { const x = 1; return x + 2; }");
    let b = h(Dialect::JavaScript, "function f() { const y = 1; return y + 2; }");
    assert_eq!(a, b);
}

#[test]
fn js_for_of_rename_invariant() {
    let a = h(
        Dialect::JavaScript,
        "function f() { for (const x of xs) { use(x); } }",
    );
    let b = h(
        Dialect::JavaScript,
        "function f() { for (const y of xs) { use(y); } }",
    );
    assert_eq!(a, b);
}

#[test]
fn js_for_classic_rename_invariant() {
    let a = h(
        Dialect::JavaScript,
        "function f() { for (let i = 0; i < n; i++) { use(i); } }",
    );
    let b = h(
        Dialect::JavaScript,
        "function f() { for (let j = 0; j < n; j++) { use(j); } }",
    );
    assert_eq!(a, b);
}

#[test]
fn js_catch_rename_invariant() {
    let a = h(
        Dialect::JavaScript,
        "function f() { try { x(); } catch (e) { log(e); } }",
    );
    let b = h(
        Dialect::JavaScript,
        "function f() { try { x(); } catch (err) { log(err); } }",
    );
    assert_eq!(a, b);
}

#[test]
fn ts_typed_param_rename_invariant() {
    // El TIPO afecta el hash, pero el nombre del parámetro no.
    let a = h(
        Dialect::TypeScript,
        "function f(x: number): number { return x + 1; }",
    );
    let b = h(
        Dialect::TypeScript,
        "function f(y: number): number { return y + 1; }",
    );
    assert_eq!(a, b);
}

#[test]
fn ts_typed_param_type_matters() {
    let int_v = h(
        Dialect::TypeScript,
        "function f(x: number): number { return x; }",
    );
    let str_v = h(
        Dialect::TypeScript,
        "function f(x: string): string { return x; }",
    );
    assert_ne!(int_v, str_v, "el tipo afecta semántica");
}

// ============================================================================
// Go
// ============================================================================

#[test]
fn go_function_rename_invariant() {
    let a = h(
        Dialect::Go,
        "package main\nfunc add(a, b int) int { return a + b }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nfunc add(x, y int) int { return x + y }\n",
    );
    assert_eq!(a, b);
}

#[test]
fn go_function_name_matters() {
    let a = h(
        Dialect::Go,
        "package main\nfunc add(a, b int) int { return a + b }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nfunc sub(a, b int) int { return a + b }\n",
    );
    assert_ne!(a, b);
}

#[test]
fn go_short_var_decl_rename_invariant() {
    let a = h(
        Dialect::Go,
        "package main\nfunc main() { x := compute(); use(x) }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nfunc main() { y := compute(); use(y) }\n",
    );
    assert_eq!(a, b);
}

#[test]
fn go_range_clause_rename_invariant() {
    let a = h(
        Dialect::Go,
        "package main\nfunc main() { for k, v := range m { use(k, v) } }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nfunc main() { for x, y := range m { use(x, y) } }\n",
    );
    assert_eq!(a, b);
}

#[test]
fn go_if_init_rename_invariant() {
    let a = h(
        Dialect::Go,
        "package main\nfunc main() { if x := lookup(); x > 0 { use(x) } }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nfunc main() { if y := lookup(); y > 0 { use(y) } }\n",
    );
    assert_eq!(a, b);
}

#[test]
fn go_func_literal_closure_rename_invariant() {
    let a = h(
        Dialect::Go,
        "package main\nvar f = func(x int) int { return x + 1 }\n",
    );
    let b = h(
        Dialect::Go,
        "package main\nvar f = func(y int) int { return y + 1 }\n",
    );
    assert_eq!(a, b);
}

// ============================================================================
// Cross-language sanity
// ============================================================================

#[test]
fn structurally_similar_programs_in_different_languages_have_distinct_hashes() {
    // `def f(x): return x+1` en Python vs `function f(x){return x+1}` en JS.
    // Mismo "shape" en idea pero distintas gramáticas → distintos kinds →
    // distintos hashes. Importante para evitar colisiones cross-language.
    let py = h(Dialect::Python, "def f(x):\n    return x + 1\n");
    let js = h(Dialect::JavaScript, "function f(x) { return x + 1; }");
    assert_ne!(py, js);
}
