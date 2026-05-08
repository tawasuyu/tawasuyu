//! Tests del `run_sync_async` sobre canales async in-memory.
//!
//! Equivalentes a los del harness síncrono pero ejecutados sobre
//! `tokio::io::duplex` — la misma lógica protocolar viajando sobre
//! bytes serializados con postcard, encuadrados con length-prefix, y
//! transportados por una pipa async. Si esto pasa, lo único que falta
//! para el sync sobre TCP/QUIC/libp2p es enchufar el transporte real.

use minga_core::{parse, ContentHash, Keypair, MemStore, Mst, NodeStore};
use minga_p2p::{run_sync_async, SyncSession};

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

fn build_repo(sources: &[&str]) -> (Mst, MemStore, Vec<ContentHash>) {
    let mut mst = Mst::new();
    let mut store = MemStore::new();
    let mut roots = Vec::new();
    for src in sources {
        let n = parse::rust(src).unwrap();
        let h = store.put(&n);
        mst.insert(h);
        roots.push(h);
    }
    (mst, store, roots)
}

#[tokio::test]
async fn async_sync_identical_repos() {
    let sources = &["fn add(x: i32, y: i32) -> i32 { x + y }"];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(sources);

    let session_a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let session_b = SyncSession::without_attestations(mst_b, store_b, kp(2));

    let (a_stream, b_stream) = tokio::io::duplex(64 * 1024);

    let task_a = tokio::spawn(run_sync_async(session_a, a_stream));
    let task_b = tokio::spawn(run_sync_async(session_b, b_stream));

    let a = task_a.await.unwrap().unwrap();
    let b = task_b.await.unwrap().unwrap();

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
}

#[tokio::test]
async fn async_sync_one_empty_pulls_everything() {
    let sources = &["fn complex(x: i32) -> i32 { let y = x * 2; y + 1 }"];
    let (mst_a, store_a, _) = build_repo(sources);
    let (mst_b, store_b, _) = build_repo(&[]);
    let store_a_size = store_a.len();

    let session_a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let session_b = SyncSession::without_attestations(mst_b, store_b, kp(2));

    let (a_stream, b_stream) = tokio::io::duplex(64 * 1024);

    let task_a = tokio::spawn(run_sync_async(session_a, a_stream));
    let task_b = tokio::spawn(run_sync_async(session_b, b_stream));

    let a = task_a.await.unwrap().unwrap();
    let b = task_b.await.unwrap().unwrap();

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
    assert_eq!(a.store().len(), b.store().len());
    assert_eq!(b.store().len(), store_a_size);
}

#[tokio::test]
async fn async_sync_disjoint_sets_merge() {
    let only_a = &[
        "fn alpha() -> i32 { 1 }",
        "fn beta(x: i32) -> i32 { x + 1 }",
    ];
    let only_b = &[
        "fn gamma(y: i32) -> bool { y > 0 }",
        "fn delta() -> &'static str { \"hello\" }",
    ];

    let (mst_a, store_a, _) = build_repo(only_a);
    let (mst_b, store_b, _) = build_repo(only_b);

    let session_a = SyncSession::without_attestations(mst_a, store_a, kp(1));
    let session_b = SyncSession::without_attestations(mst_b, store_b, kp(2));

    let (a_stream, b_stream) = tokio::io::duplex(64 * 1024);

    let task_a = tokio::spawn(run_sync_async(session_a, a_stream));
    let task_b = tokio::spawn(run_sync_async(session_b, b_stream));

    let a = task_a.await.unwrap().unwrap();
    let b = task_b.await.unwrap().unwrap();

    assert_eq!(a.mst().root_hash(), b.mst().root_hash());
    assert_eq!(a.mst().len(), 4);
}

#[tokio::test]
async fn async_sync_propagates_authenticated_identity() {
    // Cada peer debe acabar conociendo el DID verificado del otro,
    // exactamente como en el harness síncrono.
    let kp_a = kp(10);
    let kp_b = kp(20);
    let did_a = kp_a.did();
    let did_b = kp_b.did();

    let session_a = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp_a);
    let session_b = SyncSession::without_attestations(Mst::new(), MemStore::new(), kp_b);

    let (a_stream, b_stream) = tokio::io::duplex(64 * 1024);

    let task_a = tokio::spawn(run_sync_async(session_a, a_stream));
    let task_b = tokio::spawn(run_sync_async(session_b, b_stream));

    let a = task_a.await.unwrap().unwrap();
    let b = task_b.await.unwrap().unwrap();

    assert_eq!(a.peer_did(), Some(did_b));
    assert_eq!(b.peer_did(), Some(did_a));
}

#[tokio::test]
async fn async_sync_propagates_attestations() {
    use minga_core::{Attestation, AttestationStore};

    let kp_a = kp(30);
    let kp_b = kp(40);

    let (mst_a, store_a, roots_a) = build_repo(&["fn from_a() -> i32 { 1 }"]);
    let (mst_b, store_b, roots_b) = build_repo(&["fn from_b() -> i32 { 2 }"]);

    let mut atts_a = AttestationStore::new();
    atts_a
        .add(Attestation::create(&kp_a, roots_a[0]))
        .unwrap();

    let mut atts_b = AttestationStore::new();
    atts_b
        .add(Attestation::create(&kp_b, roots_b[0]))
        .unwrap();

    let session_a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let session_b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());

    let (a_stream, b_stream) = tokio::io::duplex(128 * 1024);

    let task_a = tokio::spawn(run_sync_async(session_a, a_stream));
    let task_b = tokio::spawn(run_sync_async(session_b, b_stream));

    let a = task_a.await.unwrap().unwrap();
    let b = task_b.await.unwrap().unwrap();

    // Los DIDs y atestaciones cruzaron correctamente sobre el wire.
    assert_eq!(a.attestations().authors_of(&roots_a[0]), vec![kp_a.did()]);
    assert_eq!(a.attestations().authors_of(&roots_b[0]), vec![kp_b.did()]);
    assert_eq!(b.attestations().authors_of(&roots_a[0]), vec![kp_a.did()]);
    assert_eq!(b.attestations().authors_of(&roots_b[0]), vec![kp_b.did()]);
}
