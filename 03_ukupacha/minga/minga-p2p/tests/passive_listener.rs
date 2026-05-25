//! Tests del passive listener.
//!
//! Un peer "always-on" que acepta sincronizaciones continuamente:
//! cada peer entrante mergea sus contribuciones al estado compartido.
//! El test demuestra que dos peers consecutivos (B luego C) se
//! sincronizan independientemente con A, y A acaba con la unión de
//! ambos estados.

use std::time::Duration;

use minga_core::{parse, AttestationStore, Keypair, MemStore, Mst, NodeStore};
use minga_p2p::MingaPeer;

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

fn singleton_repo(src: &str) -> (Mst, MemStore, minga_core::ContentHash) {
    let mut mst = Mst::new();
    let mut store = MemStore::new();
    let h = store.put(&parse::rust(src).unwrap());
    mst.insert(h);
    (mst, store, h)
}

async fn sync_with_retry(peer: &MingaPeer, target: libp2p::PeerId) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if peer.sync_with(target).await.is_ok() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!("sync no completó en 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn passive_listener_serves_two_consecutive_peers() {
    // ── Peer A: vacío, escucha pasivamente ─────────────────────────
    let a = MingaPeer::new(
        kp(1),
        Mst::new(),
        MemStore::new(),
        AttestationStore::new(),
    )
    .unwrap();
    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let _accept = a.run_passive_accept();

    // ── Peer B: tiene función X. Sincroniza con A ─────────────────
    let (mst_b, store_b, h_x) = singleton_repo("fn x() -> i32 { 1 }");
    let b = MingaPeer::new(kp(2), mst_b, store_b, AttestationStore::new()).unwrap();

    b.dial(addr_a.clone());
    sync_with_retry(&b, a.peer_id()).await;

    // A debe haber absorbido X.
    let (mst_a_mid, _, _) = a.snapshot().await;
    assert!(mst_a_mid.contains(&h_x), "A no aprendió X de B");

    // ── Peer C: tiene función Y. Sincroniza con A ─────────────────
    let (mst_c, store_c, h_y) = singleton_repo("fn y(z: i32) -> i32 { z * 2 }");
    let c = MingaPeer::new(kp(3), mst_c, store_c, AttestationStore::new()).unwrap();

    c.dial(addr_a.clone());
    sync_with_retry(&c, a.peer_id()).await;

    // ── Verificación: A acumuló X (de B) e Y (de C) ──────────────
    let (mst_a_final, _, _) = a.snapshot().await;
    assert!(mst_a_final.contains(&h_x), "A perdió X");
    assert!(mst_a_final.contains(&h_y), "A no aprendió Y");
    assert_eq!(mst_a_final.len(), 2);

    // C también tiene ambas: la suya y X que recibió de A durante el sync.
    let (mst_c_final, _, _) = c.snapshot().await;
    assert!(mst_c_final.contains(&h_x), "C no recibió X transitivamente");
    assert!(mst_c_final.contains(&h_y));
    assert_eq!(mst_c_final.len(), 2);
}

#[tokio::test]
async fn passive_listener_propagates_attestations() {
    use minga_core::Attestation;

    let kp_a = kp(10);
    let kp_b = kp(20);
    let kp_c = kp(30);

    // A pasivo, sin contenido.
    let a = MingaPeer::new(
        kp_a.clone(),
        Mst::new(),
        MemStore::new(),
        AttestationStore::new(),
    )
    .unwrap();
    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let _accept = a.run_passive_accept();

    // B con contenido firmado por kp_b.
    let (mst_b, store_b, h_b) = singleton_repo("fn from_b() -> i32 { 1 }");
    let mut atts_b = AttestationStore::new();
    atts_b.add(Attestation::create(&kp_b, h_b)).unwrap();
    let b = MingaPeer::new(kp_b.clone(), mst_b, store_b, atts_b).unwrap();
    b.dial(addr_a.clone());
    sync_with_retry(&b, a.peer_id()).await;

    // C con contenido firmado por kp_c. Sincroniza con A: aprende
    // tanto el contenido de B como su atestación.
    let (mst_c, store_c, h_c) = singleton_repo("fn from_c() -> i32 { 2 }");
    let mut atts_c = AttestationStore::new();
    atts_c.add(Attestation::create(&kp_c, h_c)).unwrap();
    let c = MingaPeer::new(kp_c.clone(), mst_c, store_c, atts_c).unwrap();
    c.dial(addr_a.clone());
    sync_with_retry(&c, a.peer_id()).await;

    // C ahora ve la atestación de B sobre h_b — sin haber hablado
    // nunca con B directamente. La transitividad funciona.
    let (_, _, atts_c_final) = c.snapshot().await;
    let authors_b = atts_c_final.authors_of(&h_b);
    assert_eq!(authors_b, vec![kp_b.did()]);

    // Y C tiene su propia atestación intacta.
    let authors_c = atts_c_final.authors_of(&h_c);
    assert_eq!(authors_c, vec![kp_c.did()]);
}
