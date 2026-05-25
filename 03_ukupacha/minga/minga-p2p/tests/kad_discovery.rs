//! Tests de descubrimiento vía Kademlia DHT.

use std::time::Duration;

use minga_core::{parse, AttestationStore, Keypair, MemStore, Mst, NodeStore};
use minga_p2p::{LibP2pNode, MingaPeer};

#[tokio::test]
async fn identify_auto_populates_kad_routing_table() {
    // Sin `add_dht_peer` manual: solo dial. Identify intercambia
    // direcciones automáticamente y poblamos Kad con ellas. Tras
    // unos cientos de ms, A puede consultar B vía DHT.
    let a = LibP2pNode::new().unwrap();
    let b = LibP2pNode::new().unwrap();

    let addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    a.dial(addr_b);

    // Margen para handshake Noise + Yamux + Identify.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let result = a.find_closest_peers(b.peer_id).await;
    assert!(
        result.iter().any(|p| p.peer_id == b.peer_id),
        "tras Identify, B debe estar en el routing de A. Obtuvo: {:?}",
        result.iter().map(|p| p.peer_id).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn kad_two_node_basic_discovery() {
    // A escucha. B dializa, añade A al routing table de Kad.
    // Tras el handshake Kad, B puede consultar el DHT y encontrar A.
    let a = LibP2pNode::new().unwrap();
    let b = LibP2pNode::new().unwrap();

    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    b.add_dht_peer(a.peer_id, addr_a.clone());
    b.dial(addr_a.clone());

    // Damos margen para handshake Noise+Yamux+Kad.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let result = b.find_closest_peers(a.peer_id).await;
    assert!(
        result.iter().any(|p| p.peer_id == a.peer_id),
        "B debe encontrar A vía DHT, obtuvo {:?}",
        result
    );
}

#[tokio::test]
async fn kad_three_node_discovery_via_rendezvous() {
    // Test canónico de descubrimiento DHT:
    // - A es un peer "rendezvous" que pre-conoce a B y C (en una red
    //   real, A los aprendería de los handshakes Kad cuando B y C se
    //   conectan; aquí lo seedeamos explícitamente para no depender
    //   de timing de propagación).
    // - B solo conoce a A.
    // - B pregunta al DHT por C: la query va a A, A responde con C,
    //   B aprende la dirección de C sin haberle hablado nunca.
    //
    // Este es exactamente el patrón de IPFS, libp2p bootstrap nodes
    // y cualquier P2P descentralizado real.

    let a = LibP2pNode::new().unwrap(); // rendezvous
    let b = LibP2pNode::new().unwrap();
    let c = LibP2pNode::new().unwrap();

    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let addr_b = b.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let addr_c = c.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    // A (el rendezvous) tiene a B y C en su routing table.
    a.add_dht_peer(b.peer_id, addr_b);
    a.add_dht_peer(c.peer_id, addr_c);

    // B solo conoce a A.
    b.add_dht_peer(a.peer_id, addr_a.clone());
    b.dial(addr_a.clone());

    // Margen para que la conexión Kad B↔A se establezca.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // B pregunta al DHT por C. Su routing table solo tiene A; la
    // query va a A; A responde con C de su table. B descubre.
    let result = b.find_closest_peers(c.peer_id).await;
    assert!(
        result.iter().any(|p| p.peer_id == c.peer_id),
        "B debe descubrir C vía A; obtuvo: {:?}",
        result.iter().map(|p| p.peer_id).collect::<Vec<_>>()
    );

    // Y la dirección de C debe haber viajado en el resultado, así
    // que B podría dialarlo directamente sin pasar por A.
    let c_entry = result.iter().find(|p| p.peer_id == c.peer_id).unwrap();
    assert!(!c_entry.addrs.is_empty(), "C debe venir con address resoluble");
}

#[tokio::test]
async fn kad_discovery_then_sync() {
    // Cierre del bucle: B descubre C vía DHT a través de A, y luego
    // sincroniza directamente con C. Discovery + transport + sync
    // protocolar autenticado, todo end-to-end sobre red real.

    fn singleton(seed: u8, src: &str) -> MingaPeer {
        let mut mst = Mst::new();
        let mut store = MemStore::new();
        let h = store.put(&parse::rust(src).unwrap());
        mst.insert(h);
        MingaPeer::new(
            Keypair::from_seed(&[seed; 32]),
            mst,
            store,
            AttestationStore::new(),
        )
        .unwrap()
    }

    // A: rendezvous puro, solo Kad (no MingaPeer, no necesita estado).
    let a = LibP2pNode::new().unwrap();
    let addr_a = a.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;

    // C: tiene una función que B querrá. Pasivo para aceptar el sync.
    let c = singleton(3, "fn from_c(x: i32) -> i32 { x + 100 }");
    let addr_c = c.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let _accept_c = c.run_passive_accept();

    // A pre-conoce a C en su routing table (rendezvous comportándose
    // como tal).
    a.add_dht_peer(c.peer_id(), addr_c);

    // B: tiene su propia función. Solo conoce A.
    let b = singleton(2, "fn from_b() -> i32 { 0 }");
    b.add_dht_peer(a.peer_id, addr_a.clone());
    b.dial(addr_a.clone());

    tokio::time::sleep(Duration::from_millis(300)).await;

    // B descubre a C vía DHT.
    let discovered = b.find_closest_peers(c.peer_id()).await;
    let c_entry = discovered
        .iter()
        .find(|p| p.peer_id == c.peer_id())
        .unwrap_or_else(|| {
            panic!(
                "B no descubrió C; encontró: {:?}",
                discovered.iter().map(|p| p.peer_id).collect::<Vec<_>>()
            )
        });

    // B usa la dirección descubierta para dial directo y sync.
    let addr_c_via_dht = c_entry.addrs[0].clone();
    b.dial(addr_c_via_dht);

    // Reintentamos sync hasta que la conexión esté arriba.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if b.sync_with(c.peer_id()).await.is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!("sync no completó en 5s");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Tras el sync, B y C tienen el mismo MST (unión). El merge de
    // C sucede en su task de accept (paralela a B); esperamos a que
    // ese merge se vea reflejado en su state.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        let (mst_b, _, _) = b.snapshot().await;
        let (mst_c, _, _) = c.snapshot().await;
        if mst_b.root_hash() == mst_c.root_hash() && mst_b.len() == 2 {
            break;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "no convergencia tras 2s: |B|={}, |C|={}",
                mst_b.len(),
                mst_c.len()
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
