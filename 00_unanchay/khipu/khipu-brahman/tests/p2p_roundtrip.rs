//! Integración: transferir un sobre khipu entre dos nodos libp2p reales en
//! localhost. Espeja el molde de `minga-p2p`/`agora-net-brahman`: un nodo
//! escucha y sirve, el otro dial-ea, abre stream y jala; el sobre que llega
//! se verifica con `khipu_share::open`.

use std::time::{Duration, Instant};

use agora_core::Keypair;
use card_net::Multiaddr;
use khipu_brahman::KhipuNode;
use khipu_share::{open, seal, SharedNote};

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
