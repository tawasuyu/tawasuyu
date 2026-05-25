//! Invariantes del NodeStore.
//!
//! El almacén tiene tres responsabilidades cruzadas que deben sostenerse
//! simultáneamente:
//! 1. **Round-trip exacto**: lo que entró sale igual.
//! 2. **Hash estable**: el hash que devuelve `put` coincide con
//!    `cas::hash_node` del nodo original.
//! 3. **Deduplicación**: subárboles compartidos se almacenan una sola vez.

use minga_core::{
    cas::hash_node,
    parse,
    store::{MemStore, NodeStore},
};

#[test]
fn store_round_trip_preserves_tree() {
    let original = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
    let mut store = MemStore::new();
    let h = store.put(&original);
    let reconstructed = store.reconstruct(&h).unwrap();
    assert_eq!(reconstructed, original);
}

#[test]
fn store_hash_matches_cas() {
    let n = parse::rust("fn f() -> bool { true }").unwrap();
    let mut store = MemStore::new();
    let put_hash = store.put(&n);
    assert_eq!(put_hash, hash_node(&n));
}

#[test]
fn store_idempotent_put() {
    let n = parse::rust("fn f() { 1 + 2 + 3 }").unwrap();
    let mut store = MemStore::new();
    let h1 = store.put(&n);
    let len_after_first = store.len();
    let h2 = store.put(&n);
    let len_after_second = store.len();
    assert_eq!(h1, h2);
    assert_eq!(len_after_first, len_after_second);
}

#[test]
fn store_dedup_shared_subtree() {
    // Dos funciones con cuerpo idéntico: el subárbol del bloque y todos
    // sus descendientes deben aparecer una sola vez en el almacén.
    let a = parse::rust("fn alpha() -> i32 { 1 + 2 }").unwrap();
    let b = parse::rust("fn beta() -> i32 { 1 + 2 }").unwrap();

    let mut store = MemStore::new();
    let h_a = store.put(&a);
    let count_after_a = store.len();
    let h_b = store.put(&b);
    let count_after_b = store.len();

    assert_ne!(h_a, h_b, "los hashes raíz deben diferir (nombres distintos)");

    // Buscar el bloque del cuerpo en ambas y verificar mismo hash.
    let body_a = find_first_kind(&a, "block").unwrap();
    let body_b = find_first_kind(&b, "block").unwrap();
    assert_eq!(hash_node(body_a), hash_node(body_b));

    // Crecimiento esperado al añadir b: solo los nodos que difieren entre
    // las dos funciones (el `function_item` raíz, el identificador del
    // nombre `beta`, posiblemente algún wrapper). En cualquier caso,
    // estrictamente menos que duplicar el almacén.
    assert!(
        count_after_b < 2 * count_after_a,
        "dedup falló: {count_after_b} >= 2 * {count_after_a}"
    );
}

#[test]
fn store_subtree_resolvable_independently() {
    // El hash de cualquier subárbol debe poder reconstruirse aunque
    // hayamos pedido un árbol mayor que lo contiene.
    let n = parse::rust("fn f() -> i32 { let x = 7; x * 2 }").unwrap();
    let mut store = MemStore::new();
    store.put(&n);

    let block = find_first_kind(&n, "block").unwrap();
    let block_hash = hash_node(block);
    assert!(store.contains(&block_hash));
    let reconstructed_block = store.reconstruct(&block_hash).unwrap();
    assert_eq!(&reconstructed_block, block);
}

#[test]
fn store_unknown_hash_is_none() {
    let store = MemStore::new();
    let bogus = minga_core::ContentHash([0xAB; 32]);
    assert!(store.get(&bogus).is_none());
    assert!(store.reconstruct(&bogus).is_none());
}

#[test]
fn store_multiple_files_share_common_constants() {
    // Tres archivos con el literal "42" repetido: el nodo
    // `integer_literal` con texto "42" debe almacenarse una sola vez.
    let n1 = parse::rust("fn a() -> i32 { 42 }").unwrap();
    let n2 = parse::rust("fn b() -> i32 { 42 }").unwrap();
    let n3 = parse::rust("fn c() -> i32 { 42 }").unwrap();
    let mut store = MemStore::new();
    store.put(&n1);
    let after_one = store.len();
    store.put(&n2);
    store.put(&n3);
    let after_three = store.len();
    // Cota laxa: 3 archivos no triplican el almacén; comparten ~todos los
    // nodos del cuerpo (block, integer_literal "42").
    assert!(after_three < 3 * after_one);
}

fn find_first_kind<'a>(
    node: &'a minga_core::SemanticNode,
    kind: &str,
) -> Option<&'a minga_core::SemanticNode> {
    if node.kind == kind {
        return Some(node);
    }
    for c in &node.children {
        if let Some(f) = find_first_kind(c, kind) {
            return Some(f);
        }
    }
    None
}
