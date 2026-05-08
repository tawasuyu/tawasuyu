//! Invariantes del `SledMstStore`. La propiedad clave: el `Mst`
//! reconstruido desde disco produce el mismo `root_hash` que el `Mst`
//! que insertamos — la estructura es derivable solo de las claves.

use minga_core::{ContentHash, Mst};
use minga_store::SledMstStore;
use tempfile::TempDir;

fn open_store(path: &std::path::Path) -> (sled::Db, SledMstStore) {
    let db = sled::open(path).unwrap();
    let store = SledMstStore::open_tree(&db, "mst").unwrap();
    (db, store)
}

fn ch(seed: u64) -> ContentHash {
    let h = blake3::hash(&seed.to_le_bytes());
    ContentHash(*h.as_bytes())
}

#[test]
fn insert_and_contains() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let h = ch(1);
    assert!(!store.contains(&h).unwrap());
    assert!(store.insert(h).unwrap());
    assert!(store.contains(&h).unwrap());

    // Idempotencia: re-insertar devuelve false.
    assert!(!store.insert(h).unwrap());
    assert_eq!(store.len(), 1);
}

#[test]
fn iter_returns_sorted_keys() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let hashes: Vec<ContentHash> = (0..32u64).map(ch).collect();
    for h in &hashes {
        store.insert(*h).unwrap();
    }

    let collected: Vec<ContentHash> = store.iter().map(|r| r.unwrap()).collect();
    let mut sorted = hashes.clone();
    sorted.sort();
    assert_eq!(collected, sorted);
}

#[test]
fn root_hash_matches_in_memory_mst() {
    // La propiedad fundacional: persistir solo las claves y reconstruir
    // el árbol da exactamente el mismo `root_hash` que un `Mst`
    // construido en memoria con las mismas claves.
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let mut in_memory = Mst::new();
    for i in 0..50u64 {
        let h = ch(i);
        store.insert(h).unwrap();
        in_memory.insert(h);
    }

    let reconstructed = store.to_in_memory().unwrap();
    assert_eq!(reconstructed.root_hash(), in_memory.root_hash());
    assert_eq!(reconstructed.len(), in_memory.len());
}

#[test]
fn data_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    let hashes: Vec<ContentHash> = (0..20u64).map(ch).collect();

    let target_root_hash;
    {
        let (db, store) = open_store(path);
        for h in &hashes {
            store.insert(*h).unwrap();
        }
        target_root_hash = store.to_in_memory().unwrap().root_hash();
        store.flush().unwrap();
        drop(store);
        drop(db);
    }
    {
        let (_db, store) = open_store(path);
        let reconstructed = store.to_in_memory().unwrap();
        assert_eq!(reconstructed.root_hash(), target_root_hash);
        assert_eq!(reconstructed.len(), 20);
    }
}

#[test]
fn order_independent_persistence() {
    // Insertar las mismas claves en orden distinto produce el mismo
    // `root_hash`. Equivalencia con la garantía del MST in-memory.
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    let hashes: Vec<ContentHash> = (0..30u64).map(ch).collect();

    let (_db1, s1) = open_store(dir1.path());
    for h in &hashes {
        s1.insert(*h).unwrap();
    }

    let (_db2, s2) = open_store(dir2.path());
    for h in hashes.iter().rev() {
        s2.insert(*h).unwrap();
    }

    assert_eq!(
        s1.to_in_memory().unwrap().root_hash(),
        s2.to_in_memory().unwrap().root_hash()
    );
}
