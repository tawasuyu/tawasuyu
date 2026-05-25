//! Invariantes del direccionamiento por contenido semántico.
//!
//! Estos tests definen la *tesis matemática* del núcleo: qué cambios deben
//! preservar el hash y qué cambios deben romperlo. Si alguno falla, la
//! garantía fundacional de Minga está rota.

use minga_core::{cas::hash_node, parse};

#[test]
fn whitespace_invariant() {
    let a = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
    let b = parse::rust("fn add(x:i32,y:i32)->i32{x+y}").unwrap();
    let c = parse::rust("fn  add( x : i32 , y : i32 )\n  -> i32\n{\n    x + y\n}").unwrap();
    assert_eq!(hash_node(&a), hash_node(&b));
    assert_eq!(hash_node(&a), hash_node(&c));
}

#[test]
fn comment_invariant() {
    let a = parse::rust("fn f() { 1 + 2 }").unwrap();
    let b = parse::rust("fn f() { /* comentario */ 1 + 2 // cola\n }").unwrap();
    let c = parse::rust("// arriba\nfn f() {\n    // dentro\n    1 + 2\n}\n").unwrap();
    assert_eq!(hash_node(&a), hash_node(&b));
    assert_eq!(hash_node(&a), hash_node(&c));
}

#[test]
fn body_change_breaks_hash() {
    let a = parse::rust("fn f() { 1 + 2 }").unwrap();
    let b = parse::rust("fn f() { 1 + 3 }").unwrap();
    assert_ne!(hash_node(&a), hash_node(&b));
}

#[test]
fn rename_breaks_hash_for_now() {
    // Capa base: renombrar identificadores cambia el hash. La identidad
    // por intención (alpha-equivalencia: mismo cuerpo módulo nombres
    // ligados) es una capa superior que se construirá encima.
    let a = parse::rust("fn add(x: i32) -> i32 { x }").unwrap();
    let b = parse::rust("fn add(y: i32) -> i32 { y }").unwrap();
    assert_ne!(hash_node(&a), hash_node(&b));
}

#[test]
fn signature_change_breaks_hash() {
    let a = parse::rust("fn f(x: i32) -> i32 { x }").unwrap();
    let b = parse::rust("fn f(x: i64) -> i64 { x }").unwrap();
    assert_ne!(hash_node(&a), hash_node(&b));
}

#[test]
fn order_matters() {
    // Reordenar dos funciones top-level cambia el hash del archivo entero
    // (el árbol del source_file tiene hijos ordenados). El hash de cada
    // función individual debe permanecer estable.
    let file_a = parse::rust("fn a() {} fn b() {}").unwrap();
    let file_b = parse::rust("fn b() {} fn a() {}").unwrap();
    assert_ne!(hash_node(&file_a), hash_node(&file_b));

    // Pero las funciones individuales (segundo nivel) sí coinciden cruzadas:
    let fa = &file_a.children[0]; // fn a
    let fb_in_b = &file_b.children[1]; // fn a en file_b
    assert_eq!(hash_node(fa), hash_node(fb_in_b));
}
