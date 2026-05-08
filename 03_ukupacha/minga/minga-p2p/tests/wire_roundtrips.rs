//! Tests de roundtrip de serialización para `Message`.

use minga_core::{Attestation, ContentHash, Keypair, NodeProbe, StoredNode};
use minga_p2p::Message;

fn roundtrip(msg: &Message) {
    let bytes = msg.encode();
    let decoded = Message::decode(&bytes).unwrap();
    assert_eq!(msg, &decoded);
}

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

#[test]
fn hello_roundtrip() {
    let k = kp(1);
    let root = ContentHash([42; 32]);
    let sig = k.sign(root.as_bytes());
    let msg = Message::Hello {
        peer_did: k.did(),
        root_subtree_hash: root,
        signature: sig,
    };
    roundtrip(&msg);
}

#[test]
fn probe_req_roundtrip() {
    roundtrip(&Message::ProbeReq {
        subtree_hash: ContentHash([5; 32]),
    });
}

#[test]
fn probe_res_with_probe_roundtrip() {
    let msg = Message::ProbeRes {
        subtree_hash: ContentHash([7; 32]),
        probe: Some(NodeProbe {
            level: 2,
            keys: vec![ContentHash([1; 32])],
            child_hashes: vec![ContentHash([10; 32]), ContentHash([20; 32])],
        }),
    };
    roundtrip(&msg);
}

#[test]
fn probe_res_empty_roundtrip() {
    roundtrip(&Message::ProbeRes {
        subtree_hash: ContentHash([7; 32]),
        probe: None,
    });
}

#[test]
fn fetch_roundtrip() {
    roundtrip(&Message::Fetch {
        hash: ContentHash([3; 32]),
    });
}

#[test]
fn deliver_roundtrip() {
    let stored = StoredNode {
        kind: "function_item".to_string(),
        field_name: Some("body".to_string()),
        leaf_text: None,
        children: vec![ContentHash([1; 32]), ContentHash([2; 32])],
    };
    roundtrip(&Message::Deliver {
        hash: ContentHash([99; 32]),
        stored,
    });
}

#[test]
fn attest_push_roundtrip() {
    let alice = kp(10);
    let bob = kp(20);
    let attestations = vec![
        Attestation::create(&alice, ContentHash([1; 32])),
        Attestation::create(&bob, ContentHash([2; 32])),
    ];
    roundtrip(&Message::AttestPush { attestations });
}

#[test]
fn done_roundtrip() {
    roundtrip(&Message::Done);
}

#[test]
fn malformed_bytes_decode_to_error() {
    let bogus = vec![0xFFu8; 100];
    assert!(Message::decode(&bogus).is_err());
}

#[test]
fn empty_bytes_decode_to_error() {
    assert!(Message::decode(&[]).is_err());
}

#[test]
fn message_decode_after_encode_preserves_signatures() {
    // El roundtrip de un Hello debe preservar la firma de modo que la
    // verificación criptográfica del receptor siga funcionando.
    let k = kp(33);
    let root = ContentHash([55; 32]);
    let sig = k.sign(root.as_bytes());
    let original = Message::Hello {
        peer_did: k.did(),
        root_subtree_hash: root,
        signature: sig,
    };
    let bytes = original.encode();
    let decoded = Message::decode(&bytes).unwrap();
    let Message::Hello {
        peer_did,
        root_subtree_hash,
        signature,
    } = decoded
    else {
        panic!("variante incorrecta tras decode");
    };
    assert!(peer_did.verify(root_subtree_hash.as_bytes(), &signature));
}
