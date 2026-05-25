//! Test de integración real con libp2p.
//!
//! Dos `LibP2pNode`s independientes en localhost:
//! - cada uno con su propia identidad libp2p,
//! - conectados por TCP (con cifrado Noise + multiplexado Yamux),
//! - intercambiando una sesión completa de sync vía bidirectional
//!   streams sobre el protocolo `/minga/sync/1.0.0`.
//!
//! Lo único que el wire añade respecto al harness in-memory es el
//! transporte. La lógica del protocolo y el state machine son los
//! mismos — eso es exactamente lo que queríamos demostrar.

use std::time::Duration;

use futures::StreamExt;
use minga_core::{parse, ContentHash, Keypair, MemStore, Mst, NodeStore};
use minga_p2p::{run_sync_async, LibP2pNode, SyncSession, SYNC_PROTOCOL};
use tokio_util::compat::FuturesAsyncReadCompatExt;

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
async fn libp2p_sync_two_peers_over_tcp() {
    let node_a = LibP2pNode::new().unwrap();
    let node_b = LibP2pNode::new().unwrap();
    let peer_b = node_b.peer_id;

    // Solo B necesita escuchar; A inicia el dial.
    let addr_b = node_b
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;

    // B acepta streams del protocolo Minga en una tarea.
    let only_b_sources = &["fn from_b(x: i32) -> i32 { x + 1 }"];
    let (mst_b, store_b, _) = build_repo(only_b_sources);
    let session_b = SyncSession::without_attestations(mst_b, store_b, kp(2));

    let mut control_b = node_b.control.clone();
    let task_b = tokio::spawn(async move {
        let mut incoming = control_b.accept(SYNC_PROTOCOL).unwrap();
        let (_peer, stream) = incoming.next().await.expect("incoming stream");
        run_sync_async(session_b, stream.compat()).await
    });

    // A dializa B y abre stream. Reintenta hasta que la conexión esté
    // arriba (puede tardar unos ms el handshake Noise+Yamux).
    node_a.dial(addr_b);
    let mut control_a = node_a.control.clone();
    let stream_a = {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match control_a.open_stream(peer_b, SYNC_PROTOCOL).await {
                Ok(s) => break s,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("no se pudo abrir stream tras 5s: {e:?}"),
            }
        }
    };

    let only_a_sources = &["fn from_a() -> i32 { 0 }"];
    let (mst_a, store_a, _) = build_repo(only_a_sources);
    let session_a = SyncSession::without_attestations(mst_a, store_a, kp(1));

    let task_a = tokio::spawn(async move { run_sync_async(session_a, stream_a.compat()).await });

    let result_a = task_a.await.expect("task A").expect("sync A");
    let result_b = task_b.await.expect("task B").expect("sync B");

    // Convergencia tras viajar sobre TCP real.
    assert_eq!(result_a.mst().root_hash(), result_b.mst().root_hash());
    assert_eq!(result_a.mst().len(), 2);
    assert_eq!(result_b.mst().len(), 2);

    // Cada peer terminó con la identidad libp2p del otro autenticada.
    // (Las identidades libp2p no son las mismas que los DIDs Minga —
    // las primeras autentican el canal, los segundos firman contenido.)
    assert!(result_a.peer_did().is_some());
    assert!(result_b.peer_did().is_some());
}

#[tokio::test]
async fn libp2p_sync_with_attestations() {
    use minga_core::{Attestation, AttestationStore};

    let node_a = LibP2pNode::new().unwrap();
    let node_b = LibP2pNode::new().unwrap();
    let peer_b = node_b.peer_id;

    let addr_b = node_b
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;

    let kp_a = kp(10);
    let kp_b = kp(20);

    let (mst_a, store_a, roots_a) = build_repo(&["fn signed_by_a() -> i32 { 1 }"]);
    let (mst_b, store_b, roots_b) = build_repo(&["fn signed_by_b() -> i32 { 2 }"]);

    let mut atts_a = AttestationStore::new();
    atts_a.add(Attestation::create(&kp_a, roots_a[0])).unwrap();

    let mut atts_b = AttestationStore::new();
    atts_b.add(Attestation::create(&kp_b, roots_b[0])).unwrap();

    let session_a = SyncSession::new(mst_a, store_a, atts_a, kp_a.clone());
    let session_b = SyncSession::new(mst_b, store_b, atts_b, kp_b.clone());

    let mut control_b = node_b.control.clone();
    let task_b = tokio::spawn(async move {
        let mut incoming = control_b.accept(SYNC_PROTOCOL).unwrap();
        let (_peer, stream) = incoming.next().await.expect("incoming stream");
        run_sync_async(session_b, stream.compat()).await
    });

    node_a.dial(addr_b);
    let mut control_a = node_a.control.clone();
    let stream_a = {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match control_a.open_stream(peer_b, SYNC_PROTOCOL).await {
                Ok(s) => break s,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => panic!("no se pudo abrir stream: {e:?}"),
            }
        }
    };

    let task_a = tokio::spawn(async move { run_sync_async(session_a, stream_a.compat()).await });

    let result_a = task_a.await.unwrap().unwrap();
    let result_b = task_b.await.unwrap().unwrap();

    // Atestaciones cruzaron criptográficamente verificadas.
    assert_eq!(
        result_a.attestations().authors_of(&roots_b[0]),
        vec![kp_b.did()]
    );
    assert_eq!(
        result_b.attestations().authors_of(&roots_a[0]),
        vec![kp_a.did()]
    );
}
