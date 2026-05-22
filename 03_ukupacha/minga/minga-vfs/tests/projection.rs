//! Cobertura de la capa agnóstica del VFS (render + source) sobre
//! código real parseado con tree-sitter. No monta FUSE: verifica que
//! la proyección hash → contenido es correcta de punta a punta.

use minga_core::parse;
use minga_vfs::render::{render_sexp, render_source};
use minga_vfs::source::{reconstruct, MemSource, NodeSource};

#[test]
fn ingest_rust_then_reconstruct_is_lossless_at_the_ast_level() {
    let original = parse::rust("fn add(a: i32, b: i32) -> i32 { a + b }").unwrap();

    let mut src = MemSource::new();
    let hash = src.add_root(&original);

    // El árbol reconstruido desde el store debe ser idéntico bit a bit
    // al que se ingirió: el direccionamiento por contenido lo garantiza.
    let back = reconstruct(&src, &hash).expect("el hash recién insertado resuelve");
    assert_eq!(back, original);
}

#[test]
fn roots_lists_every_ingested_file() {
    let mut src = MemSource::new();
    let a = src.add_root(&parse::rust("fn a() {}").unwrap());
    let b = src.add_root(&parse::python("def b():\n    pass\n").unwrap());

    let roots = src.roots();
    assert_eq!(roots.len(), 2);
    assert!(roots.contains(&a));
    assert!(roots.contains(&b));
}

#[test]
fn source_view_recovers_rust_keywords_and_structure() {
    let node = parse::rust("fn main() { let x = 1; }").unwrap();
    let mut src = MemSource::new();
    let hash = src.add_root(&node);

    let rebuilt = reconstruct(&src, &hash).unwrap();
    let text = render_source(&rebuilt);

    for token in ["fn", "main", "let", "x", "1"] {
        assert!(text.contains(token), "falta `{token}` en:\n{text}");
    }
    // El cuerpo entre llaves debe quedar indentado en su propia línea.
    assert!(text.contains("\n    "), "cuerpo sin indentar:\n{text}");
}

#[test]
fn sexp_view_exposes_tree_sitter_node_kinds() {
    let node = parse::rust("fn main() {}").unwrap();
    let mut src = MemSource::new();
    let hash = src.add_root(&node);

    let rebuilt = reconstruct(&src, &hash).unwrap();
    let sexp = render_sexp(&rebuilt);

    assert!(sexp.contains("(source_file"), "raíz del árbol:\n{sexp}");
    assert!(sexp.contains("function_item"), "el ítem función:\n{sexp}");
}

#[test]
fn deduplicated_subtrees_share_one_node() {
    // Dos archivos con la misma función `helper` deben compartir el
    // subárbol en el store: ingerir el segundo no lo vuelve a guardar.
    let mut src = MemSource::new();
    let one = parse::rust("fn helper() { 42 }").unwrap();
    let two = parse::rust("fn helper() { 42 }").unwrap();

    let h1 = src.add_root(&one);
    let h2 = src.add_root(&two);

    // Estructura idéntica ⇒ mismo hash ⇒ una sola raíz.
    assert_eq!(h1, h2);
    assert_eq!(src.roots().len(), 1);
}
