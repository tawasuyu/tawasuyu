//! Test E2E de Fase 2: discovery remoto vía DHT.
//!
//! Pipeline:
//! 1. **Provider node (A)**: arma server con `BrahmanNet` configurado;
//!    listen TCP; un cliente local registra una Card con un output
//!    flow. El server llama `announce_outputs` automáticamente, lo
//!    que hace `start_providing` en el DHT bajo la key derivada del
//!    flow.
//! 2. **Consumer node (B)**: arma su propio `BrahmanNet`; dial-ea al
//!    multiaddr del provider para que ambos se conozcan vía Identify
//!    (esto popula sus respectivos routing tables de Kademlia).
//! 3. **B llama `find_remote_providers(flow_name, type)`**: la query
//!    DHT propaga vía Kad, y eventually el provider responde con su
//!    `PeerId`.
//! 4. **Verificación**: el `PeerId` que B descubre coincide con el
//!    de A.
//!
//! Notas:
//! - Kademlia replication factor por defecto es 20; con 2 nodos no
//!   hay propagación material — A es el único provider, B llega a A
//!   vía la conexión directa establecida en step 2 y obtiene el record
//!   del store local de A.
//! - El test usa flow `monad-list:json` por familiaridad (es el flow
//!   real que `chasqui daemon` declara). Sirve también como prueba de
//!   que el sistema completo (daemon + DHT) funcionaría con cero
//!   cambios en la Card.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chasqui_broker::{Broker, BrokerConfig};
use card_core::{
    ulid::Ulid, Card, CardKind, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef,
    CARD_SCHEMA_VERSION,
};
use card_handshake::network::{find_remote_providers, run_libp2p_accept_loop};
use card_handshake::server::{Server, ServerConfig};
use card_net::{BrahmanNet, Multiaddr, Protocol};
use tempfile::TempDir;
use tokio::sync::Mutex;

fn provider_card(label: &str, flow_name: &str, type_name: &str) -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: label.into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        permissions: Default::default(),
        soma: Default::default(),
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
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dht_discovery_finds_remote_provider() {
    // ---- Node A (provider): server + libp2p net + Card con output ----
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
                net: Some(a_net.clone()), // ← clave Fase 2: anuncia al DHT
                policy: None,
            },
        )
        .unwrap(),
    );

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let a_addr = a_net.listen(listen_addr).await;
    let mut a_full_addr = a_addr.clone();
    a_full_addr.push(Protocol::P2p(a_peer));

    tokio::spawn(run_libp2p_accept_loop(a_server.clone(), a_net.clone()));

    // Unix accept loop: necesario para que Client::connect al socket
    // local no cuelgue (Server no se auto-accepta; el caller arma el
    // loop). Cada session entrante corre en su propia task.
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

    // Registrar la Card local en A con un flow output.
    let card = provider_card("test.engine_remote", "monad-list", "json");
    let mut local_client = card_handshake::client::Client::connect(&a_unix, card)
        .await
        .expect("registro local en A");

    // ---- Node B (consumer): otro net que dial-a a A ----
    let b_net = BrahmanNet::new().unwrap();
    b_net.dial(a_full_addr.clone());

    // Esperar a que la conexión se establezca y Identify popule el
    // routing table de Kad. En localhost con 2 peers, ~250ms es de
    // sobra; sumamos margen para CI.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // ---- Discovery: B busca providers de "monad-list:json" ----
    let providers = find_remote_providers(
        &b_net,
        "monad-list",
        &TypeRef::Primitive {
            name: "json".into(),
        },
    )
    .await;

    assert!(
        providers.contains(&a_peer),
        "B debería descubrir a A vía DHT. Encontrados: {:?}, esperado: {}",
        providers,
        a_peer
    );

    // Sanidad: el cliente local sigue vivo durante todo el test (lo
    // que mantiene la Card registrada y por tanto el record DHT vivo).
    local_client.farewell().await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dht_discovery_negative_unknown_flow() {
    // Mismo setup que el test happy-path, pero B busca un flow que A
    // NO ofrece. Debe devolver lista vacía dentro del timeout
    // razonable (no colgarse).
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
                broker: Some(a_broker),
                net: Some(a_net.clone()),
                policy: None,
            },
        )
        .unwrap(),
    );

    let a_addr = a_net.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let mut a_full = a_addr.clone();
    a_full.push(Protocol::P2p(a_peer));

    tokio::spawn(run_libp2p_accept_loop(a_server.clone(), a_net.clone()));

    // Unix accept loop: necesario para que Client::connect al socket
    // local no cuelgue (Server no se auto-accepta; el caller arma el
    // loop). Cada session entrante corre en su propia task.
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

    let card = provider_card("test.engine_other", "monad-list", "json");
    let mut local = card_handshake::client::Client::connect(&a_unix, card)
        .await
        .unwrap();

    let b_net = BrahmanNet::new().unwrap();
    b_net.dial(a_full);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Buscamos un flow que NADIE anunció.
    let providers = find_remote_providers(
        &b_net,
        "flow-que-no-existe",
        &TypeRef::Primitive {
            name: "json".into(),
        },
    )
    .await;

    assert!(
        providers.is_empty(),
        "no debería haber providers para un flow inexistente, got: {:?}",
        providers
    );

    local.farewell().await.ok();
}

/// stop_providing test: A registra Card con flow X, B descubre a A.
/// El cliente local de A hace farewell → cleanup llama
/// withdraw_outputs → A se quita del provider local store. Una nueva
/// query desde B (que rutea por A, único peer en el DHT) ya no debe
/// listarlo.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dht_discovery_withdraws_on_session_cleanup() {
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
                broker: Some(a_broker),
                net: Some(a_net.clone()),
                policy: None,
            },
        )
        .unwrap(),
    );
    let sessions = a_server.sessions();

    let a_addr = a_net.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
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

    // Card con un flow output anunciable.
    let card = provider_card("test.withdraws", "monad-list", "json");
    let local = card_handshake::client::Client::connect(&a_unix, card)
        .await
        .expect("registro local en A");

    let b_net = BrahmanNet::new().unwrap();
    b_net.dial(a_full);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Confirmación previa: A es discoverable.
    let before = find_remote_providers(
        &b_net,
        "monad-list",
        &TypeRef::Primitive {
            name: "json".into(),
        },
    )
    .await;
    assert!(
        before.contains(&a_peer),
        "antes del farewell A debería ser discoverable. got: {:?}",
        before
    );

    // Farewell del cliente local → server.cleanup → withdraw_outputs.
    local.farewell().await.ok();

    // Esperamos a que la sesión salga del registro de A (señal de
    // que cleanup completó).
    let mut waited = 0;
    while !sessions.lock().await.is_empty() && waited < 50 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        waited += 1;
    }
    assert!(
        sessions.lock().await.is_empty(),
        "sesión debería estar removida tras farewell"
    );

    // Pequeño margen extra para que el Command::StopProviding lo
    // procese el swarm task (no es await-able desde fuera).
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Nueva query: A ya no debería listarse como provider.
    let after = find_remote_providers(
        &b_net,
        "monad-list",
        &TypeRef::Primitive {
            name: "json".into(),
        },
    )
    .await;
    assert!(
        !after.contains(&a_peer),
        "tras farewell + withdraw_outputs, A NO debería ser discoverable. got: {:?}",
        after
    );
}
