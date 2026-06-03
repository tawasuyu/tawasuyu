//! Integración del resolver `card_sidecar::discovery::resolve_provider`:
//! prueba el camino **local-first / remote-fallback**. Sin init local
//! alcanzable (socket bogus), el resolver debe caer al DHT y descubrir un
//! proveedor remoto que anunció su output flow.
//!
//! Reusa el harness de `card-handshake/tests/network_discovery.rs`: un Node A
//! con Server + `BrahmanNet` + Card-con-output (el Server llama
//! `announce_outputs` al registrar → `start_providing` en el DHT), y un Node B
//! que dial-a a A y resuelve vía el DHT compartido.

use std::sync::Arc;
use std::time::Duration;

use card_core::{
    Card, CardKind, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef,
};
use card_handshake::network::run_libp2p_accept_loop;
use card_handshake::server::{Server, ServerConfig};
use card_net::{BrahmanNet, Multiaddr, Protocol};
use card_sidecar::discovery::{
    build_consumer_card, consume_remote, resolve_provider, ConsumerError, ProviderLocation,
};
use chasqui_broker::{Broker, BrokerConfig};
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Card de proveedor con un único output flow `(flow_name, type_name)`.
fn provider_card(label: &str, flow_name: &str, type_name: &str) -> Card {
    Card {
        payload: Payload::Virtual,
        supervision: Supervision::Delegate,
        lifecycle: Lifecycle::Daemon,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        flow: Flows {
            input: vec![],
            output: vec![Flow {
                name: flow_name.into(),
                ty: TypeRef::Primitive {
                    name: type_name.into(),
                },
                pin_to: None,
            }],
        },
        ..Card::new(label)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resolve_provider_cae_a_dht_cuando_no_hay_local() {
    // Forzar miss local: que `await_provider` falle rápido por socket ausente
    // (nextest aísla cada test en su propio proceso, así que la env no se filtra).
    std::env::set_var("BRAHMAN_INIT_SOCKET", "/nonexistent/brahman-init-test.sock");

    // ---- Node A: Server + net + Card con output (anuncia al DHT) ----
    let tmp = TempDir::new().unwrap();
    let a_unix = tmp.path().join("a.sock");
    let a_broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let a_net = Arc::new(BrahmanNet::new().unwrap());
    let a_peer = a_net.peer_id;

    let a_server = Arc::new(
        Server::bind(
            &a_unix,
            ServerConfig {
                init_attached: true,
                broker: Some(a_broker.clone()),
                net: Some(a_net.clone()), // ← el Server anuncia los outputs al DHT
                policy: None,
            },
        )
        .unwrap(),
    );

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let a_addr = a_net.listen(listen_addr).await;
    let mut a_full = a_addr.clone();
    a_full.push(Protocol::P2p(a_peer));

    tokio::spawn(run_libp2p_accept_loop(a_server.clone(), a_net.clone()));
    {
        let s = a_server.clone();
        tokio::spawn(async move {
            loop {
                match s.accept_one().await {
                    Ok(session) => {
                        tokio::spawn(async move {
                            let _ = session.handle().await;
                        });
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // A registra una Card con output "monad-list":json → start_providing.
    let card = provider_card("test.engine_remote", "monad-list", "json");
    let mut local_client = card_handshake::client::Client::connect(&a_unix, card)
        .await
        .expect("registro local en A");

    // ---- Node B: net que dial-a a A y comparte DHT ----
    let b_net = BrahmanNet::new().unwrap();
    b_net.dial(a_full.clone());
    // Margen para que Identify popule la routing table de Kad.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // resolve_provider: sin init local (socket bogus) → fallback al DHT.
    let consumer = build_consumer_card("test.consumer", "monad-list", "json");
    let loc = resolve_provider(consumer, &b_net, Duration::from_millis(200))
        .await
        .expect("resolve_provider no debe errar");

    match loc {
        ProviderLocation::Remote(peers) => assert!(
            peers.contains(&a_peer),
            "el fallback DHT debería descubrir a A; encontrados: {:?}, esperado: {}",
            peers,
            a_peer
        ),
        ProviderLocation::Local(s) => {
            panic!("esperaba Remote (no hay init local), obtuve Local({:?})", s)
        }
    }

    local_client.farewell().await.ok();
}

/// End-to-end del follow-up de Nivel 3 (CIERRE §3.2): un nodo descubre por
/// DHT a un proveedor remoto y CONSUME — abre handshake firmado por libp2p,
/// pinea y se despide — sin un init local de por medio. Es el patrón de
/// `jalar_a_traves_de_un_relay` aplicado al consumer-side de chasqui.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn consume_remote_abre_handshake_firmado_via_libp2p() {
    // Mismo truco que el test de arriba: forzar miss del init local para
    // que el resolver caiga directo al DHT.
    std::env::set_var(
        "BRAHMAN_INIT_SOCKET",
        "/nonexistent/brahman-init-consume.sock",
    );

    // ---- Node A: Server + net; al registrar la Card con output, el
    //              Server llama `announce_outputs` y publica la key
    //              `(monad-list, json)` en el DHT.
    let tmp = TempDir::new().unwrap();
    let a_unix = tmp.path().join("a.sock");
    let a_broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let a_net = Arc::new(BrahmanNet::new().unwrap());
    let a_peer = a_net.peer_id;

    let a_server = Arc::new(
        Server::bind(
            &a_unix,
            ServerConfig {
                init_attached: true,
                broker: Some(a_broker.clone()),
                net: Some(a_net.clone()),
                policy: None,
            },
        )
        .unwrap(),
    );

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let a_addr = a_net.listen(listen_addr).await;
    let mut a_full = a_addr.clone();
    a_full.push(Protocol::P2p(a_peer));

    // A acepta tanto las sesiones libp2p (handshake firmado entrante) como
    // las locales por socket Unix (necesarias para registrar la Card que
    // dispara el `announce_outputs`).
    tokio::spawn(run_libp2p_accept_loop(a_server.clone(), a_net.clone()));
    {
        let s = a_server.clone();
        tokio::spawn(async move {
            loop {
                match s.accept_one().await {
                    Ok(session) => {
                        tokio::spawn(async move {
                            let _ = session.handle().await;
                        });
                    }
                    Err(_) => break,
                }
            }
        });
    }

    let provider = provider_card("test.engine_remote_consume", "monad-list", "json");
    let mut local_provider_client = card_handshake::client::Client::connect(&a_unix, provider)
        .await
        .expect("registro local del provider en A");

    // ---- Node B: net independiente; dial-a A para compartir DHT.
    let b_net = BrahmanNet::new().unwrap();
    b_net.dial(a_full.clone());
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Descubrir.
    let consumer = build_consumer_card("test.consumer_remote", "monad-list", "json");
    let loc = resolve_provider(consumer.clone(), &b_net, Duration::from_millis(200))
        .await
        .expect("resolve_provider no debe errar");

    let peers = match loc {
        ProviderLocation::Remote(peers) => {
            assert!(
                peers.contains(&a_peer),
                "el fallback DHT debería descubrir a A; encontrados: {:?}, esperado: {}",
                peers,
                a_peer
            );
            peers
        }
        ProviderLocation::Local(s) => {
            panic!("esperaba Remote (no hay init local), obtuve Local({:?})", s)
        }
    };

    // Consumir: abre handshake firmado por libp2p contra A y pinea.
    let mut remote = consume_remote(&b_net, consumer, None, peers)
        .await
        .expect("consume_remote debe abrir handshake firmado contra A");

    remote
        .ping()
        .await
        .expect("ping libp2p sobre la sesión consumer-remoto");

    // Cierre limpio de ambas sesiones.
    remote.farewell().await.ok();
    local_provider_client.farewell().await.ok();
}

/// Si el DHT no anuncia ningún proveedor para el flow pedido, `consume_remote`
/// no debe colgar ni intentar nada — debe fallar inmediato con
/// `NoRemoteProviders`. Este test no necesita Net A ni B, sólo un net solo y
/// una lista vacía de peers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn consume_remote_falla_inmediato_si_no_hay_peers() {
    let net = BrahmanNet::new().unwrap();
    let consumer = build_consumer_card("test.consumer_solo", "flow-fantasma", "json");

    let err = consume_remote(&net, consumer, None, vec![])
        .await
        .expect_err("vector vacío debe traducirse a NoRemoteProviders");

    match err {
        ConsumerError::NoRemoteProviders { flow, type_ref } => {
            assert_eq!(flow, "flow-fantasma");
            assert_eq!(type_ref, "json");
        }
        otro => panic!("esperaba NoRemoteProviders, obtuve {otro:?}"),
    }
}
