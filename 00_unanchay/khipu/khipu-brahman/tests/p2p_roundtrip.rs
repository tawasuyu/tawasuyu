//! Integración: transferir un sobre khipu entre dos nodos libp2p reales en
//! localhost. Espeja el molde de `minga-p2p`/`agora-net-brahman`: un nodo
//! escucha y sirve, el otro dial-ea, abre stream y jala; el sobre que llega
//! se verifica con `khipu_share::open`.

use std::time::{Duration, Instant};

use agora_core::Keypair;
use card_net::Multiaddr;
use khipu_brahman::KhipuNode;
use khipu_share::{open, seal, SharedNote};

/// Camino que usa la app: `listen_str` da la dirección para compartir y
/// `fetch_addr_str` la consume (dial + reintento + fetch).
#[tokio::test]
async fn jalar_por_direccion_str_como_la_app() {
    let autor = Keypair::from_seed([32u8; 32]);
    let sobre = seal(
        &autor,
        vec![SharedNote {
            title: "via str".into(),
            body: "listen_str + fetch_addr_str".into(),
            tags: vec![],
        }],
        1,
    )
    .unwrap();
    let bytes = sobre.to_bytes().unwrap();

    let server = KhipuNode::standalone().unwrap();
    let client = KhipuNode::standalone().unwrap();
    let dial = server.listen_str("/ip4/127.0.0.1/tcp/0").await.unwrap();
    let _serve = server.run_serve(move || Some(bytes.clone()));

    // fetch_addr_str ya reintenta internamente mientras se conecta.
    let recibido = client.fetch_addr_str(&dial).await.expect("fetch por str");
    let bundle = open(&recibido).expect("verifica tras el viaje");
    assert_eq!(bundle.notes[0].title, "via str");
}

/// NAT traversal: A reserva un circuito en un relay público y sirve;
/// B le jala el cuaderno *a través del relay* (Circuit Relay v2), sin
/// dirección directa a A. Verifica la maquinaria relay/dcutr de card-net.
#[tokio::test]
async fn jalar_a_traves_de_un_relay() {
    let autor = Keypair::from_seed([33u8; 32]);
    let sobre = seal(
        &autor,
        vec![SharedNote {
            title: "relay".into(),
            body: "viajó por un circuito relay".into(),
            tags: vec![],
        }],
        1,
    )
    .unwrap();
    let bytes = sobre.to_bytes().unwrap();

    // Relay público.
    let relay = KhipuNode::standalone().unwrap();
    let relay_addr = relay.listen_str("/ip4/127.0.0.1/tcp/0").await.unwrap();

    // A: sirve y reserva un circuito en el relay (su dirección pasa a ser
    // `…/p2p/<relay>/p2p-circuit/p2p/<A>`).
    let a = KhipuNode::standalone().unwrap();
    let _serve = a.run_serve(move || Some(bytes.clone()));
    // A se conecta al relay; el intercambio identify le confirma al relay
    // su dirección externa (para que la incluya en la reserva). Recién
    // entonces reservamos el circuito.
    a.dial_str(&relay_addr).unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let circuit = format!("{relay_addr}/p2p-circuit");
    let a_addr = tokio::time::timeout(Duration::from_secs(15), a.listen_str(&circuit))
        .await
        .expect("la reserva del circuito no debería colgar")
        .unwrap();
    assert!(a_addr.contains("p2p-circuit"), "A debe anunciarse vía circuito");

    // B jala el cuaderno de A a través del relay.
    let b = KhipuNode::standalone().unwrap();
    let recibido = tokio::time::timeout(Duration::from_secs(25), b.fetch_addr_str(&a_addr))
        .await
        .expect("el fetch por relay no debería colgar")
        .expect("recibir vía relay");
    let bundle = open(&recibido).expect("verifica tras el viaje por relay");
    assert_eq!(bundle.notes[0].title, "relay");
}

#[tokio::test]
async fn jalar_un_sobre_entre_dos_nodos_libp2p() {
    // Sellar el cuaderno a servir.
    let autor = Keypair::from_seed([31u8; 32]);
    let sobre = seal(
        &autor,
        vec![SharedNote {
            title: "P2P".into(),
            body: "viajó por libp2p".into(),
            tags: vec!["brahman".into()],
        }],
        1,
    )
    .unwrap();
    let bytes = sobre.to_bytes().unwrap();

    // Dos nodos en localhost.
    let server = KhipuNode::standalone().unwrap();
    let client = KhipuNode::standalone().unwrap();
    let server_pid = server.peer_id();

    let addr = server.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let _serve = server.run_serve(move || Some(bytes.clone()));

    // El cliente dial-ea por multiaddr + peer-id.
    let dial: Multiaddr = format!("{addr}/p2p/{server_pid}").parse().unwrap();
    client.dial(dial);

    // Reintentar el fetch hasta que la conexión esté lista.
    let deadline = Instant::now() + Duration::from_secs(8);
    let recibido = loop {
        match client.fetch(server_pid).await {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => panic!("fetch falló: {e}"),
        }
    };

    // Idéntico bit a bit y verificable.
    assert_eq!(recibido, sobre);
    let bundle = open(&recibido).expect("firma válida tras el viaje libp2p");
    assert_eq!(bundle.notes[0].title, "P2P");
    assert_eq!(bundle.author, autor.public_key());
}
