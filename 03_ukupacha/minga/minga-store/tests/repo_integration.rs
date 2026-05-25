//! Test de integración del `PersistentRepo`: los tres stores conviven
//! en una misma `sled::Db`, escritos en una sesión y recuperados
//! intactos en la siguiente.

use minga_core::{parse, Attestation, ContentHash, Keypair};
use minga_store::PersistentRepo;
use tempfile::TempDir;

#[test]
fn three_stores_persist_together_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    let alice = Keypair::from_seed(&[1; 32]);

    // ── Sesión 1: poblamos el repo ──────────────────────────────────
    let function_hash;
    let target_root_hash;
    {
        let repo = PersistentRepo::open(path).unwrap();
        let n = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
        function_hash = repo.nodes.put(&n).unwrap();
        repo.mst.insert(function_hash).unwrap();
        repo.attestations
            .add(Attestation::create(&alice, function_hash))
            .unwrap();

        target_root_hash = repo.mst.to_in_memory().unwrap().root_hash();
        repo.flush().unwrap();
    }

    // ── Sesión 2: reabrimos y verificamos integridad ────────────────
    {
        let repo = PersistentRepo::open(path).unwrap();

        // Nodo recuperable.
        let stored = repo.nodes.get(&function_hash).unwrap().unwrap();
        assert_eq!(stored.kind, "source_file");

        // Reconstrucción completa idéntica al original.
        let reconstructed = repo.nodes.reconstruct(&function_hash).unwrap().unwrap();
        let original = parse::rust("fn add(x: i32, y: i32) -> i32 { x + y }").unwrap();
        assert_eq!(reconstructed, original);

        // MST: misma raíz tras reconstruir.
        assert_eq!(
            repo.mst.to_in_memory().unwrap().root_hash(),
            target_root_hash
        );

        // Atestación: sigue ahí, sigue verificable.
        let authors = repo.attestations.authors_of(&function_hash).unwrap();
        assert_eq!(authors, vec![alice.did()]);
        let atts = repo.attestations.get(&function_hash).unwrap();
        assert!(atts[0].verify());
    }
}

#[test]
fn repo_supports_multiple_functions_and_authors() {
    let dir = TempDir::new().unwrap();
    let repo = PersistentRepo::open(dir.path()).unwrap();

    let alice = Keypair::from_seed(&[1; 32]);
    let bob = Keypair::from_seed(&[2; 32]);

    let mut hashes: Vec<ContentHash> = Vec::new();
    for src in &[
        "fn one() -> i32 { 1 }",
        "fn two() -> i32 { 2 }",
        "fn three(x: i32) -> i32 { x + 1 }",
    ] {
        let n = parse::rust(src).unwrap();
        let h = repo.nodes.put(&n).unwrap();
        repo.mst.insert(h).unwrap();
        hashes.push(h);
    }

    // Alice firma las tres; Bob firma solo la primera.
    for h in &hashes {
        repo.attestations
            .add(Attestation::create(&alice, *h))
            .unwrap();
    }
    repo.attestations
        .add(Attestation::create(&bob, hashes[0]))
        .unwrap();

    repo.flush().unwrap();

    assert_eq!(repo.mst.len(), 3);
    assert_eq!(repo.attestations.len(), 4);

    // La función firmada por ambos tiene dos autores.
    let authors_first = repo.attestations.authors_of(&hashes[0]).unwrap();
    assert_eq!(authors_first.len(), 2);
    assert!(authors_first.contains(&alice.did()));
    assert!(authors_first.contains(&bob.did()));

    // Las otras dos solo tienen a Alice.
    assert_eq!(
        repo.attestations.authors_of(&hashes[1]).unwrap(),
        vec![alice.did()]
    );
}
