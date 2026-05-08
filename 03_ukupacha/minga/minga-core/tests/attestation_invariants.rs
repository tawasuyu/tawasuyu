//! Invariantes de las atestaciones firmadas y del `AttestationStore`.
//!
//! La tesis del módulo: una atestación válida es una **prueba**
//! criptográfica de autoría, no una declaración. El store nunca
//! almacena pruebas falsas — cualquier intento de inyectar una firma
//! corrupta se rechaza al ingresar, no al consultar.

use minga_core::{Attestation, AttestationError, AttestationStore, ContentHash, Keypair};

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

fn ch(seed: u8) -> ContentHash {
    ContentHash([seed; 32])
}

#[test]
fn create_then_verify_succeeds() {
    let alice = kp(1);
    let att = Attestation::create(&alice, ch(7));
    assert!(att.verify());
    assert_eq!(att.author, alice.did());
    assert_eq!(att.content, ch(7));
}

#[test]
fn modifying_content_invalidates() {
    let alice = kp(1);
    let mut att = Attestation::create(&alice, ch(7));
    att.content = ch(8);
    assert!(!att.verify());
}

#[test]
fn modifying_signature_invalidates() {
    let alice = kp(1);
    let mut att = Attestation::create(&alice, ch(7));
    att.signature.0[0] ^= 0xFF;
    assert!(!att.verify());
}

#[test]
fn modifying_author_invalidates() {
    let alice = kp(1);
    let bob = kp(2);
    let mut att = Attestation::create(&alice, ch(7));
    att.author = bob.did();
    assert!(!att.verify());
}

#[test]
fn store_accepts_valid_attestation() {
    let alice = kp(1);
    let att = Attestation::create(&alice, ch(5));
    let mut store = AttestationStore::new();
    assert!(store.add(att.clone()).is_ok());
    assert_eq!(store.len(), 1);
    assert_eq!(store.get(&ch(5)), &[att][..]);
}

#[test]
fn store_rejects_invalid_signature() {
    let alice = kp(1);
    let mut att = Attestation::create(&alice, ch(5));
    att.signature.0[10] ^= 1;
    let mut store = AttestationStore::new();
    assert_eq!(store.add(att), Err(AttestationError::InvalidSignature));
    assert_eq!(store.len(), 0);
}

#[test]
fn store_rejects_swapped_content() {
    // Atestación creada para `ch(1)`, modificada para reclamar `ch(2)`.
    // La firma sigue siendo válida sobre `ch(1)` pero ahora el content
    // dice `ch(2)` — no verifica.
    let alice = kp(1);
    let mut att = Attestation::create(&alice, ch(1));
    att.content = ch(2);
    let mut store = AttestationStore::new();
    assert!(store.add(att).is_err());
}

#[test]
fn store_is_idempotent_for_same_author_content() {
    let alice = kp(1);
    let att = Attestation::create(&alice, ch(5));
    let mut store = AttestationStore::new();
    store.add(att.clone()).unwrap();
    store.add(att.clone()).unwrap();
    store.add(att).unwrap();
    assert_eq!(store.len(), 1);
}

#[test]
fn store_keeps_multiple_authors_per_content() {
    let alice = kp(1);
    let bob = kp(2);
    let carol = kp(3);
    let h = ch(99);
    let mut store = AttestationStore::new();
    store.add(Attestation::create(&alice, h)).unwrap();
    store.add(Attestation::create(&bob, h)).unwrap();
    store.add(Attestation::create(&carol, h)).unwrap();
    assert_eq!(store.len(), 3);
    assert_eq!(store.get(&h).len(), 3);

    let authors = store.authors_of(&h);
    assert_eq!(authors.len(), 3);
    assert!(authors.contains(&alice.did()));
    assert!(authors.contains(&bob.did()));
    assert!(authors.contains(&carol.did()));
}

#[test]
fn authors_of_for_unknown_content_is_empty() {
    let store = AttestationStore::new();
    assert!(store.authors_of(&ch(0)).is_empty());
    assert_eq!(store.get(&ch(0)).len(), 0);
}

#[test]
fn distinct_authors_distinct_signatures_same_content() {
    // Firmar el mismo `ContentHash` con dos llaves distintas produce
    // firmas distintas (Ed25519 es determinista por llave, así que la
    // diferencia viene de la llave, no de un nonce aleatorio).
    let alice = kp(1);
    let bob = kp(2);
    let h = ch(50);
    let a1 = Attestation::create(&alice, h);
    let a2 = Attestation::create(&bob, h);
    assert_ne!(a1.signature, a2.signature);
    assert_ne!(a1.author, a2.author);
    assert!(a1.verify());
    assert!(a2.verify());
}
