//! Tests de Provider Records vía Kademlia DHT.
//!
//! Discovery a nivel de **contenido**: en lugar de "¿quién está
//! cerca?", la pregunta es "¿quién tiene el hash X?". Cuando un peer
//! ingresa contenido, se anuncia como provider; otros peers consultan
//! el DHT para encontrar a quién dial directamente.

use std::time::Duration;

use minga_core::{parse, AttestationStore, ContentHash, Keypair, MemStore, Mst};
use minga_p2p::{LibP2pNode, MingaPeer};

fn kp(seed: u8) -> Keypair {
    Keypair::from_seed(&[seed; 32])
}

#[tokio::test]
async fn provider_announce_and_lookup_two_nodes() {
    let a = LibP2pNode::new().unwrap();
    let b = LibP2pNode::new().unwrap();

    let addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    // A conoce a B y dializa para establecer conexión Kad.
    a.add_dht_peer(b.peer_id, addr_b.clone());
    a.dial(addr_b);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // A anuncia que tiene `content`.
    let content = ContentHash([0x42; 32]);
    a.start_providing(&content.0);

    // Margen para que el ADD_PROVIDER se replique a B.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // B consulta — debe encontrar A.
    let providers = b.find_providers(&content.0).await;
    assert!(
        providers.iter().any(|p| *p == a.peer_id),
        "B debe descubrir a A como provider, obtuvo: {:?}",
        providers
    );
}

#[tokio::test]
async fn provider_lookup_returns_empty_for_unknown_content() {
    let a = LibP2pNode::new().unwrap();
    let b = LibP2pNode::new().unwrap();

    let addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    a.add_dht_peer(b.peer_id, addr_b.clone());
    a.dial(addr_b);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Nadie ha anunciado este hash.
    let unknown = ContentHash([0xFF; 32]);
    let providers = b.find_providers(&unknown.0).await;
    assert!(providers.is_empty());
}

#[tokio::test]
async fn minga_peer_ingest_auto_announces_provider() {
    // El test de integración del flujo "fase de salida al mundo real":
    // un peer hace ingest de un archivo y, sin acción adicional, otro
    // peer puede descubrirlo vía DHT como provider.

    let a_kp = kp(1);
    let b_kp = kp(2);

    let a = MingaPeer::new(a_kp, Mst::new(), MemStore::new(), AttestationStore::new()).unwrap();
    let b = MingaPeer::new(b_kp, Mst::new(), MemStore::new(), AttestationStore::new()).unwrap();

    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let _addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    // Conectar B a A vía Kad (rendezvous bidireccional).
    a.add_dht_peer(b.peer_id(), _addr_b);
    b.add_dht_peer(a.peer_id(), addr_a.clone());
    b.dial(addr_a);

    tokio::time::sleep(Duration::from_millis(300)).await;

    // A ingresa una función. Esto debe anunciarla automáticamente.
    let n = parse::rust("fn discover_me() -> i32 { 7 }").unwrap();
    let h = a.ingest(&n).await;

    // Margen para la replicación del provider record.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // B busca quién tiene `h` y debe encontrar A.
    let providers = b.find_providers(h).await;
    assert!(
        providers.iter().any(|p| *p == a.peer_id()),
        "B debe descubrir a A como provider del contenido recién ingerido. Obtuvo: {:?}",
        providers,
    );
}

#[tokio::test]
async fn minga_peer_announce_all_roots_publishes_each_alpha() {
    // Tras ingerir múltiples raíces con dialect, `announce_all_roots`
    // republica todos los α-hashes en el DHT. Útil al arrancar un
    // `listen` sobre un repo existente: las raíces vuelven a ser
    // descubribles sin re-ingerir cada archivo.
    use minga_core::parse::Dialect;

    let a_kp = kp(11);
    let b_kp = kp(12);

    let a = MingaPeer::new(a_kp, Mst::new(), MemStore::new(), AttestationStore::new()).unwrap();
    let b = MingaPeer::new(b_kp, Mst::new(), MemStore::new(), AttestationStore::new()).unwrap();

    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    a.add_dht_peer(b.peer_id(), addr_b);
    b.add_dht_peer(a.peer_id(), addr_a.clone());
    b.dial(addr_a);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A ingresa dos raíces distintas con dialect.
    let n1 = parse::rust("fn alpha_one() -> i32 { 1 }").unwrap();
    let (alpha1, _) = a.ingest_with_dialect(&n1, Dialect::Rust).await;
    let n2 = parse::rust("fn alpha_two() -> i32 { 2 }").unwrap();
    let (alpha2, _) = a.ingest_with_dialect(&n2, Dialect::Rust).await;

    // Re-anuncia (idempotente). Devuelve 2.
    let announced = a.announce_all_roots().await;
    assert_eq!(announced, 2);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // B busca cada α-hash y debe encontrar A.
    let p1 = b.find_providers(alpha1).await;
    let p2 = b.find_providers(alpha2).await;
    assert!(p1.iter().any(|p| *p == a.peer_id()));
    assert!(p2.iter().any(|p| *p == a.peer_id()));
}
