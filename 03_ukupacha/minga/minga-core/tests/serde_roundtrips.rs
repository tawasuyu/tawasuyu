//! Tests de roundtrip de serialización para los tipos de wire.
//!
//! Cualquier tipo que cruce la red debe (a) (de)serializar bit-a-bit
//! igual sobre postcard, y (b) preservar todos sus invariantes
//! semánticos tras un viaje. Estos tests son la red de seguridad
//! contra cambios de schema accidentales que romperían la
//! compatibilidad on-the-wire.

use minga_core::{Attestation, ContentHash, Keypair, NodeProbe, Signature, StoredNode};

fn roundtrip<T: serde::Serialize + for<'a> serde::Deserialize<'a> + PartialEq + std::fmt::Debug>(
    value: &T,
) {
    let bytes = postcard::to_allocvec(value).unwrap();
    let decoded: T = postcard::from_bytes(&bytes).unwrap();
    assert_eq!(value, &decoded);
}

#[test]
fn content_hash_roundtrip() {
    let h = ContentHash([42; 32]);
    roundtrip(&h);

    // Codifica como exactamente 32 bytes (transparent sobre [u8; 32]).
    let bytes = postcard::to_allocvec(&h).unwrap();
    assert_eq!(bytes.len(), 32);
    assert_eq!(bytes, vec![42u8; 32]);
}

#[test]
fn did_roundtrip() {
    let kp = Keypair::from_seed(&[7; 32]);
    let did = kp.did();
    roundtrip(&did);
    let bytes = postcard::to_allocvec(&did).unwrap();
    assert_eq!(bytes.len(), 32);
}

#[test]
fn signature_roundtrip() {
    let kp = Keypair::from_seed(&[3; 32]);
    let sig = kp.sign(b"mensaje");
    roundtrip(&sig);
    // 64 bytes Ed25519 + cualquier overhead transparent.
    let bytes = postcard::to_allocvec(&sig).unwrap();
    assert_eq!(bytes.len(), 64);
}

#[test]
fn signature_roundtrip_preserves_verify() {
    let kp = Keypair::from_seed(&[9; 32]);
    let msg = b"el mensaje original";
    let sig = kp.sign(msg);

    let bytes = postcard::to_allocvec(&sig).unwrap();
    let decoded: Signature = postcard::from_bytes(&bytes).unwrap();

    // El predicado criptográfico se preserva exactamente.
    assert!(kp.did().verify(msg, &decoded));
}

#[test]
fn stored_node_roundtrip() {
    let s = StoredNode {
        kind: "function_item".to_string(),
        field_name: Some("body".to_string()),
        leaf_text: None,
        children: vec![ContentHash([1; 32]), ContentHash([2; 32])],
    };
    roundtrip(&s);
}

#[test]
fn stored_node_with_leaf_roundtrip() {
    let s = StoredNode {
        kind: "integer_literal".to_string(),
        field_name: None,
        leaf_text: Some(b"42".to_vec()),
        children: Vec::new(),
    };
    roundtrip(&s);
}

#[test]
fn attestation_roundtrip() {
    let kp = Keypair::from_seed(&[5; 32]);
    let att = Attestation::create(&kp, ContentHash([99; 32]));
    roundtrip(&att);
}

#[test]
fn attestation_roundtrip_preserves_verify() {
    let kp = Keypair::from_seed(&[11; 32]);
    let att = Attestation::create(&kp, ContentHash([77; 32]));

    let bytes = postcard::to_allocvec(&att).unwrap();
    let decoded: Attestation = postcard::from_bytes(&bytes).unwrap();

    assert!(decoded.verify());
}

#[test]
fn node_probe_roundtrip() {
    let probe = NodeProbe {
        level: 3,
        keys: vec![ContentHash([1; 32]), ContentHash([2; 32])],
        child_hashes: vec![
            ContentHash([10; 32]),
            ContentHash([20; 32]),
            ContentHash([30; 32]),
        ],
    };
    roundtrip(&probe);
}

#[test]
fn empty_collections_serialize_compactly() {
    // postcard codifica longitudes con varint. Vec vacío = 1 byte (longitud 0).
    let probe = NodeProbe {
        level: 0,
        keys: Vec::new(),
        child_hashes: Vec::new(),
    };
    let bytes = postcard::to_allocvec(&probe).unwrap();
    // postcard varint: u32(0) = 1 byte, vec_len(0) = 1 byte ×2 = 3 bytes total.
    assert_eq!(bytes.len(), 3);
}

#[test]
fn malformed_bytes_fail_decode() {
    let bogus = vec![0xFFu8; 100];
    let result: Result<Attestation, _> = postcard::from_bytes(&bogus);
    assert!(result.is_err());
}
