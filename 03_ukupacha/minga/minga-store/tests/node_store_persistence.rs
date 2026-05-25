//! Invariantes del `SledNodeStore`. Cubre:
//! - Round-trip estructural (lo que entra sale igual).
//! - Hash consistente con `cas::hash_node`.
//! - Idempotencia.
//! - Persistencia tras cerrar y reabrir el DB.
//! - Rechazo de `put_chunked` con hash inconsistente.

use minga_core::{cas::hash_components, hash_node, parse, ContentHash, StoredNode};
use minga_store::{SledNodeStore, StoreError};
use tempfile::TempDir;

fn open_store(path: &std::path::Path) -> (sled::Db, SledNodeStore) {
    let db = sled::open(path).unwrap();
    let store = SledNodeStore::open_tree(&db, "nodes").unwrap();
    (db, store)
}

#[test]
fn round_trip_preserves_tree() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let original = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
    let h = store.put(&original).unwrap();
    let reconstructed = store.reconstruct(&h).unwrap().unwrap();
    assert_eq!(original, reconstructed);
}

#[test]
fn put_hash_matches_cas() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let n = parse::rust("fn f() -> bool { true }").unwrap();
    let h_via_put = store.put(&n).unwrap();
    let h_via_cas = hash_node(&n);
    assert_eq!(h_via_put, h_via_cas);
}

#[test]
fn put_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let n = parse::rust("fn f() { 1 + 2 + 3 }").unwrap();
    let h1 = store.put(&n).unwrap();
    let len_after_first = store.len();
    let h2 = store.put(&n).unwrap();
    let len_after_second = store.len();
    assert_eq!(h1, h2);
    assert_eq!(len_after_first, len_after_second);
}

#[test]
fn data_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    let original = parse::rust("fn squared(n: i32) -> i32 { n * n }").unwrap();
    let h;
    {
        let (db, store) = open_store(path);
        h = store.put(&original).unwrap();
        store.flush().unwrap();
        drop(store);
        drop(db);
    }
    {
        let (_db, store) = open_store(path);
        let reconstructed = store.reconstruct(&h).unwrap().unwrap();
        assert_eq!(reconstructed, original);
    }
}

#[test]
fn shared_subtrees_dedup() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let a = parse::rust("fn alpha() -> i32 { 1 + 2 }").unwrap();
    let b = parse::rust("fn beta() -> i32 { 1 + 2 }").unwrap();

    store.put(&a).unwrap();
    let count_after_a = store.len();
    store.put(&b).unwrap();
    let count_after_b = store.len();

    // El cuerpo `{ 1 + 2 }` y subnodos son idénticos: comparten
    // entrada en sled. Crecimiento estricto pero menor que duplicar.
    assert!(
        count_after_b < 2 * count_after_a,
        "dedup falló: {} >= 2 * {}",
        count_after_b,
        count_after_a
    );
}

#[test]
fn put_chunked_rejects_hash_mismatch() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let stored = StoredNode {
        kind: "function_item".to_string(),
        field_name: None,
        leaf_text: None,
        children: Vec::new(),
    };
    let bogus_hash = ContentHash([0xAB; 32]);

    let result = store.put_chunked(bogus_hash, &stored);
    assert!(matches!(result, Err(StoreError::HashMismatch)));
    assert!(!store.contains(&bogus_hash).unwrap());
}

#[test]
fn put_chunked_accepts_correct_hash() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let stored = StoredNode {
        kind: "integer_literal".to_string(),
        field_name: None,
        leaf_text: Some(b"42".to_vec()),
        children: Vec::new(),
    };
    let real_hash = hash_components(
        &stored.kind,
        stored.field_name.as_deref(),
        stored.leaf_text.as_deref(),
        &stored.children,
    );

    store.put_chunked(real_hash, &stored).unwrap();
    assert!(store.contains(&real_hash).unwrap());
    let retrieved = store.get(&real_hash).unwrap().unwrap();
    assert_eq!(retrieved, stored);
}

#[test]
fn unknown_hash_returns_none() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let bogus = ContentHash([0xFE; 32]);
    assert_eq!(store.get(&bogus).unwrap(), None);
    assert_eq!(store.reconstruct(&bogus).unwrap(), None);
    assert!(!store.contains(&bogus).unwrap());
}
