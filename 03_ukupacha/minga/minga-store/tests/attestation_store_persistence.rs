//! Invariantes del `SledAttestationStore`.

use minga_core::{Attestation, AttestationError, ContentHash, Keypair};
use minga_store::{SledAttestationStore, StoreError};
use tempfile::TempDir;

fn open_store(path: &std::path::Path) -> (sled::Db, SledAttestationStore) {
    let db = sled::open(path).unwrap();
    let store = SledAttestationStore::open_tree(&db, "atts").unwrap();
    (db, store)
}

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

fn ch(seed: u8) -> ContentHash {
    ContentHash([seed; 32])
}

#[test]
fn add_then_get_roundtrips() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let alice = kp(1);
    let att = Attestation::create(&alice, ch(7));
    store.add(att.clone()).unwrap();

    let retrieved = store.get(&ch(7)).unwrap();
    assert_eq!(retrieved, vec![att]);
}

#[test]
fn invalid_signature_rejected() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let alice = kp(1);
    let mut att = Attestation::create(&alice, ch(7));
    att.signature.0[10] ^= 0xFF;

    let r = store.add(att);
    assert!(matches!(
        r,
        Err(StoreError::Attestation(AttestationError::InvalidSignature))
    ));
    assert_eq!(store.len(), 0);
}

#[test]
fn idempotent_per_author_and_content() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());

    let alice = kp(1);
    let att = Attestation::create(&alice, ch(5));
    store.add(att.clone()).unwrap();
    store.add(att.clone()).unwrap();
    store.add(att).unwrap();

    assert_eq!(store.len(), 1);
}

#[test]
fn multiple_authors_per_content() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let alice = kp(1);
    let bob = kp(2);
    let carol = kp(3);
    let h = ch(99);

    store.add(Attestation::create(&alice, h)).unwrap();
    store.add(Attestation::create(&bob, h)).unwrap();
    store.add(Attestation::create(&carol, h)).unwrap();

    assert_eq!(store.len(), 3);
    let authors = store.authors_of(&h).unwrap();
    assert_eq!(authors.len(), 3);
    assert!(authors.contains(&alice.did()));
    assert!(authors.contains(&bob.did()));
    assert!(authors.contains(&carol.did()));
}

#[test]
fn data_persists_across_reopen() {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    let alice = kp(42);
    let h = ch(11);

    {
        let (db, store) = open_store(path);
        store.add(Attestation::create(&alice, h)).unwrap();
        store.flush().unwrap();
        drop(store);
        drop(db);
    }
    {
        let (_db, store) = open_store(path);
        let authors = store.authors_of(&h).unwrap();
        assert_eq!(authors, vec![alice.did()]);
    }
}

#[test]
fn unknown_content_returns_empty() {
    let dir = TempDir::new().unwrap();
    let (_db, store) = open_store(dir.path());
    let authors = store.authors_of(&ch(0)).unwrap();
    assert!(authors.is_empty());
}
