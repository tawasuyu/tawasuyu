//! Migración perezosa de `alpha_paths`: un repo cuyo `path_history` ya
//! tiene entradas pero cuyo `alpha_paths` está vacío (caso real al
//! actualizar un repo viejo) debe reconstruir el reverse-index en el
//! primer `open`.

use minga_core::ContentHash;
use minga_store::{PersistentRepo, SledPathHistoryStore};
use tempfile::TempDir;

#[test]
fn open_rebuilds_alpha_paths_from_existing_history() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    let alpha1 = ContentHash([11u8; 32]);
    let alpha2 = ContentHash([22u8; 32]);

    // Sesión 1: simulamos un repo viejo populando sólo `path_history`
    // a nivel sled (sin pasar por `PersistentRepo::open`, que es
    // justamente quien dispara la migración).
    {
        let db = sled::open(path).unwrap();
        let paths = SledPathHistoryStore::open_tree(&db, "path_history").unwrap();
        paths.append("src/a.rs", alpha1, 100).unwrap();
        paths.append("src/a.rs", alpha2, 200).unwrap();
        paths.append("src/b.rs", alpha1, 150).unwrap();
        paths.flush().unwrap();
    }

    // Sesión 2: open dispara la migración.
    let repo = PersistentRepo::open(path).unwrap();
    assert!(!repo.alpha_paths.is_empty());

    let p1 = repo.alpha_paths.paths_for(&alpha1).unwrap();
    let mut p1_names: Vec<_> = p1.iter().map(|(p, _)| p.clone()).collect();
    p1_names.sort();
    assert_eq!(p1_names, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);

    let p2 = repo.alpha_paths.paths_for(&alpha2).unwrap();
    assert_eq!(p2, vec![("src/a.rs".to_string(), 200)]);

    // Idempotencia: el segundo open no duplica entradas.
    drop(repo);
    let repo2 = PersistentRepo::open(path).unwrap();
    assert_eq!(repo2.alpha_paths.paths_for(&alpha1).unwrap().len(), 2);
}
