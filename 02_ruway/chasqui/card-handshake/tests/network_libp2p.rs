//! Test E2E: handshake brahman remoto sobre libp2p stream.
//!
//! Pipeline:
//! 1. Server: bind Unix socket (necesario aunque no lo use el cliente);
//!    crear `BrahmanNet` y escuchar en `/ip4/127.0.0.1/tcp/0`;
//!    montar `run_libp2p_accept_loop`.
//! 2. Client: crear su propio `BrahmanNet`; dial al multiaddr del
//!    server; `connect_libp2p` con su Card; `ping`; `farewell`.
//! 3. Verificar: el server registró la sesión; sessions.len() == 1
//!    durante la sesión, == 0 después del farewell.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chasqui_broker::{Broker, BrokerConfig};
use card_core::{
    ulid::Ulid, Card, CardKind, Lifecycle, Payload, Priority, Supervision,
    CARD_SCHEMA_VERSION,
};
use card_handshake::identity::{Identity, DEFAULT_SESSION_TTL};
use card_handshake::network::{connect_libp2p, connect_libp2p_with_cert, run_libp2p_accept_loop};
use card_handshake::peer_policy::PeerPolicy;
use card_handshake::server::{Server, ServerConfig};
use card_net::{BrahmanNet, Keypair, Multiaddr, PeerId, Protocol};
use tempfile::TempDir;
use tokio::sync::Mutex;

fn sample_card(label: &str) -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: label.into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        permissions: Default::default(),
        soma: Default::default(),
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::default(),
        priority: Priority::default(),
        kind: CardKind::Ente,
        ..Default::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libp2p_handshake_roundtrip() {
    // ---- Server side ----
    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");

    let broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: Some(broker.clone()),
                net: None,
                policy: None,
            },
        )
        .unwrap(),
    );
    let sessions = server.sessions();

    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer_id = server_net.peer_id;

    // Listen on a random TCP port.
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let actual_addr = server_net.listen(listen_addr).await;
    // Inject the libp2p PeerId into the multiaddr so the client knows
    // who to dial.
    let mut full_addr = actual_addr.clone();
    full_addr.push(Protocol::P2p(server_peer_id));

    // Spawn the libp2p accept loop.
    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    // ---- Client side ----
    let client_net = BrahmanNet::new().unwrap();
    client_net.dial(full_addr.clone());

    // Pequeña espera para que el dial conecte. En un entorno real el
    // caller usaría un mecanismo de barrier, pero para tests un sleep
    // corto es suficiente y deterministic en localhost.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let card = sample_card("test.remote_ente");
    let client_kp = client_net.keypair();
    let mut client = connect_libp2p(&client_net, server_peer_id, card, None, &client_kp)
        .await
        .expect("handshake remoto debería completar");

    // Verificación: el server vio la sesión.
    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 1, "una sesión registrada");
        let resolved = s.values().next().unwrap();
        assert_eq!(resolved.card.label, "test.remote_ente");
    }

    // Ping roundtrip.
    let ts = client.ping().await.expect("ping debería responder");
    assert!(ts > 0, "timestamp del Pong > 0");

    // Farewell limpio.
    client.farewell().await.expect("farewell debería completar");

    // Tras el farewell, el cleanup remueve la sesión.
    // Damos un tick para que el handler procese el frame.
    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 0, "sesión removida tras farewell");
    }

    // peer_id no usado aquí, pero validamos que la API existe.
    let _ = PeerId::random();
}

/// Fase 3 negativo: el cliente intenta firmar el Hello con una keypair
/// distinta a la del peer libp2p. El server (que verifica que la
/// public key del Hello derive al peer_id autenticado por Noise) debe
/// rechazar con `Unauthorized`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libp2p_handshake_rejects_mismatched_signing_key() {
    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");

    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: None,
                net: None,
                policy: None,
            },
        )
        .unwrap(),
    );
    let sessions = server.sessions();

    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer = server_net.peer_id;
    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let actual = server_net.listen(listen_addr).await;
    let mut full = actual.clone();
    full.push(Protocol::P2p(server_peer));

    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    let client_net = BrahmanNet::new().unwrap();
    client_net.dial(full);
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Keypair fraudulenta: NO es la del client_net.
    let evil_keypair = Keypair::generate_ed25519();

    let card = sample_card("test.evil");
    let result = connect_libp2p(&client_net, server_peer, card, None, &evil_keypair).await;

    assert!(
        result.is_err(),
        "handshake con keypair fraudulenta debe fallar"
    );

    // Sanidad: ninguna sesión registrada.
    let s = sessions.lock().await;
    assert_eq!(s.len(), 0, "no debería haber sesión registrada");
}

/// Allowlist gate: A configura `allowlist = [client_authorized_peer]`.
/// Un cliente con peer_id en la lista pasa el handshake; otro con
/// peer_id distinto es rechazado con `Unauthorized` ANTES de la
/// verificación de firma (la allowlist se chequea primero, es más
/// barata).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libp2p_handshake_allowlist_admits_listed_rejects_others() {
    // Pre-generamos las dos identidades cliente para que A pueda
    // construir la allowlist conociendo cuál es la "permitida".
    let allowed_kp = Keypair::generate_ed25519();
    let allowed_peer = allowed_kp.public().to_peer_id();
    let denied_kp = Keypair::generate_ed25519();
    // (denied_peer no se necesita para la lista — sólo para clarity)
    let _ = denied_kp.public().to_peer_id();

    // ---- Server con allowlist activa ----
    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");
    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: None,
                net: None,
                policy: Some(PeerPolicy::from_sets(
                    Some([allowed_peer].into_iter().collect()),
                    std::collections::BTreeSet::new(),
                )),
            },
        )
        .unwrap(),
    );
    let sessions = server.sessions();

    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer = server_net.peer_id;
    let actual = server_net.listen("/ip4/127.0.0.1/tcp/0".parse().unwrap()).await;
    let mut full = actual.clone();
    full.push(Protocol::P2p(server_peer));

    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    // ---- Cliente PERMITIDO ----
    let allowed_net = BrahmanNet::with_keypair(allowed_kp.clone()).unwrap();
    allowed_net.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let card_ok = sample_card("test.allowed");
    let mut allowed_client = connect_libp2p(&allowed_net, server_peer, card_ok, None, &allowed_kp)
        .await
        .expect("peer en allowlist debe pasar");

    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 1, "sesión del peer permitido registrada");
    }
    allowed_client.farewell().await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ---- Cliente DENEGADO ----
    let denied_net = BrahmanNet::with_keypair(denied_kp.clone()).unwrap();
    denied_net.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let card_no = sample_card("test.denied");
    let result = connect_libp2p(&denied_net, server_peer, card_no, None, &denied_kp).await;

    assert!(
        result.is_err(),
        "peer fuera de allowlist debe ser rechazado, got: {:?}",
        result.is_ok()
    );
    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 0, "ninguna sesión adicional registrada tras intento denegado");
    }
}

/// Denylist gate: A configura `policy` con un peer en la denylist.
/// Modo abierto para todo lo demás (sin allowlist), pero el peer
/// baneado es rechazado aún teniendo Ed25519 válida y peer_id que
/// derivaría limpio del Noise handshake.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libp2p_handshake_denylist_blocks_listed_peer() {
    let banned_kp = Keypair::generate_ed25519();
    let banned_peer = banned_kp.public().to_peer_id();
    let other_kp = Keypair::generate_ed25519();

    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");
    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: None,
                net: None,
                policy: Some(PeerPolicy::from_sets(
                    None, // sin allowlist (abierto)
                    [banned_peer].into_iter().collect(),
                )),
            },
        )
        .unwrap(),
    );
    let sessions = server.sessions();

    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer = server_net.peer_id;
    let actual = server_net
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;
    let mut full = actual.clone();
    full.push(Protocol::P2p(server_peer));

    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    // Cliente baneado: connect debe fallar.
    let banned_net = BrahmanNet::with_keypair(banned_kp.clone()).unwrap();
    banned_net.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let card_x = sample_card("test.banned");
    let result = connect_libp2p(&banned_net, server_peer, card_x, None, &banned_kp).await;
    assert!(
        result.is_err(),
        "peer en denylist debe ser rechazado, got Ok"
    );
    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 0, "el peer baneado no debería tener sesión");
    }

    // Cliente no-baneado pasa.
    let other_net = BrahmanNet::with_keypair(other_kp.clone()).unwrap();
    other_net.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let card_ok = sample_card("test.other");
    let mut other_client = connect_libp2p(&other_net, server_peer, card_ok, None, &other_kp)
        .await
        .expect("peer fuera de denylist debe pasar");
    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 1, "sesión del peer no-baneado registrada");
    }
    other_client.farewell().await.ok();
}

/// Swarm-level deny via `PeerPolicy::attach_to_net`: cuando la deny
/// se aplica al swarm vía `block_list`, el peer baneado es rechazado
/// en el dial — la conexión TCP/Noise nunca completa, así que el
/// cliente nunca llega siquiera a mandar el Hello. Más eficiente que
/// el handshake-level deny.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn swarm_level_deny_blocks_before_noise() {
    let banned_kp = Keypair::generate_ed25519();
    let banned_peer = banned_kp.public().to_peer_id();

    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");
    let policy = card_handshake::peer_policy::PeerPolicy::from_sets(
        None,
        [banned_peer].into_iter().collect(),
    );
    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: None,
                net: None,
                policy: Some(policy.clone()),
            },
        )
        .unwrap(),
    );
    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer = server_net.peer_id;

    // ATTACH: la deny se proyecta al swarm. Es lo nuevo de este
    // commit — sin esta llamada, el deny seguiría aplicando sólo
    // al nivel de handshake brahman (lo que también funciona pero
    // gasta un round-trip Noise).
    policy.attach_to_net(server_net.clone());

    let actual = server_net
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;
    let mut full = actual.clone();
    full.push(Protocol::P2p(server_peer));

    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    // Cliente baneado intenta dial + handshake. Con swarm-level
    // deny, la conexión libp2p ni siquiera completa: `connect_libp2p`
    // falla con error de open_stream (peer inalcanzable / connection
    // refused) en lugar del Unauthorized del handshake-level path.
    let banned_net = BrahmanNet::with_keypair(banned_kp.clone()).unwrap();
    banned_net.dial(full.clone());

    let card = sample_card("test.swarm_banned");
    // Timeout corto: si el block falla, el handshake completaría
    // rápido en localhost. Si funciona, debería fallar el dial casi
    // instantáneo o colgarse hasta el timeout.
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        connect_libp2p(&banned_net, server_peer, card, None, &banned_kp),
    )
    .await;

    match result {
        Ok(Ok(_)) => panic!("peer baneado a nivel swarm NO debería completar handshake"),
        Ok(Err(e)) => {
            // Esperado: error de transporte/stream, no de handshake.
            tracing::info!(error = %e, "swarm-level deny rechazó como esperado");
        }
        Err(_) => {
            // También aceptable: timeout porque el dial nunca completa.
            tracing::info!("swarm-level deny → connect timeout (también OK)");
        }
    }
}

/// Multi-key identity: la propiedad fundamental que cierra el
/// proyecto. El cliente B tiene una identity master estable; el
/// server A le permite el master_peer en allowlist. B se conecta con
/// **session1**; pasa. B "rota": genera **session2** distinta, emite
/// un nuevo cert con la misma identity, se conecta de nuevo. Pasa
/// también — sin que A toque su allowlist.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn identity_cert_allows_session_rotation_without_policy_change() {
    // Master de B (estable, persistente).
    let master_kp = Keypair::generate_ed25519();
    let master_peer = master_kp.public().to_peer_id();
    let identity = Identity::from_keypair(master_kp);

    // A configura policy: allowlist con master_peer (NO sessions).
    let tmp = TempDir::new().unwrap();
    let unix_socket = tmp.path().join("brahman-init.sock");
    let server = Arc::new(
        Server::bind(
            &unix_socket,
            ServerConfig {
                init_attached: true,
                broker: None,
                net: None,
                policy: Some(PeerPolicy::from_sets(
                    Some([master_peer].into_iter().collect()),
                    std::collections::BTreeSet::new(),
                )),
            },
        )
        .unwrap(),
    );
    let sessions = server.sessions();

    let server_net = Arc::new(BrahmanNet::new().unwrap());
    let server_peer = server_net.peer_id;
    let actual = server_net
        .listen("/ip4/127.0.0.1/tcp/0".parse().unwrap())
        .await;
    let mut full = actual.clone();
    full.push(Protocol::P2p(server_peer));

    tokio::spawn(run_libp2p_accept_loop(server.clone(), server_net.clone()));

    // ---- Conexión 1: session1 ----
    let session1_kp = Keypair::generate_ed25519();
    let cert1 = identity
        .issue_session_cert(&session1_kp, DEFAULT_SESSION_TTL)
        .unwrap();
    let net1 = BrahmanNet::with_keypair(session1_kp.clone()).unwrap();
    net1.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client1 = connect_libp2p_with_cert(
        &net1,
        server_peer,
        sample_card("test.session1"),
        None,
        &session1_kp,
        cert1,
    )
    .await
    .expect("session1 con cert válido del master allowlisted debe pasar");

    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 1, "session1 registrada");
    }
    client1.farewell().await.ok();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // ---- ROTACIÓN: session2 distinta, mismo master ----
    let session2_kp = Keypair::generate_ed25519();
    assert_ne!(
        session1_kp.public().to_peer_id(),
        session2_kp.public().to_peer_id(),
        "test inválido si las sessions son iguales"
    );
    let cert2 = identity
        .issue_session_cert(&session2_kp, DEFAULT_SESSION_TTL)
        .unwrap();

    let net2 = BrahmanNet::with_keypair(session2_kp.clone()).unwrap();
    net2.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut client2 = connect_libp2p_with_cert(
        &net2,
        server_peer,
        sample_card("test.session2"),
        None,
        &session2_kp,
        cert2,
    )
    .await
    .expect(
        "session2 (rotada) con cert del MISMO master debe pasar sin tocar allowlist",
    );

    {
        let s = sessions.lock().await;
        assert_eq!(s.len(), 1, "session2 registrada");
    }
    client2.farewell().await.ok();

    // Sanity: una session sin cert (path Fase 3) cuyo session_peer_id
    // NO está en la allowlist (porque la allowlist tiene master, no
    // sessions) DEBE ser rechazada.
    let session_other = Keypair::generate_ed25519();
    let net_other = BrahmanNet::with_keypair(session_other.clone()).unwrap();
    net_other.dial(full.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = connect_libp2p(
        &net_other,
        server_peer,
        sample_card("test.no_cert"),
        None,
        &session_other,
    )
    .await;
    assert!(
        result.is_err(),
        "sin cert, session_peer_id (no listado) debe ser rechazado"
    );
}
