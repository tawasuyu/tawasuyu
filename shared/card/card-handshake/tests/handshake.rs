//! Tests de integración: levanta server + client en el mismo proceso,
//! ejercita el round-trip completo del protocolo.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use brahman_broker::{Broker, BrokerConfig};
use brahman_card::{
    Card, CgroupSpec, Flow, Flows, NamespaceSet, Payload, ResourceLimits, SomaSpec, Supervision,
    TypeRef, CARD_SCHEMA_VERSION,
};
use brahman_handshake::{
    client::{Client, ClientError},
    codec::{read_frame, write_frame},
    messages::{Frame, HandshakeError, Hello, Ping},
    server::{Server, ServerConfig},
};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use ulid::Ulid;

fn sample_card(label: &str) -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        lineage: None,
        label: label.into(),
        provides: BTreeSet::new(),
        requires: BTreeSet::new(),
        soma: SomaSpec {
            cgroup: CgroupSpec {
                path: "ente.slice/test".into(),
                cpu_weight: None,
                io_weight: None,
            },
            namespaces: NamespaceSet::default(),
            rlimits: ResourceLimits::default(),
            cpu_affinity: None,
        },
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        ..Default::default()
    }
}

/// Genera una ruta de socket única bajo TMPDIR. No la creamos —
/// el server la creará al hacer bind.
fn sock_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "brahman-test-{}-{}-{}.sock",
        name,
        std::process::id(),
        Ulid::new()
    ))
}

#[tokio::test]
async fn full_handshake_roundtrip() {
    let path = sock_path("happy");
    let server = Server::bind(&path, ServerConfig { init_attached: true, broker: None, net: None, policy: None }).unwrap();

    let session_handle = tokio::spawn({
        async move {
            let session = server.accept_one().await.unwrap();
            session.handle().await.unwrap();
        }
    });

    let mut client = Client::connect(&path, sample_card("alpha")).await.unwrap();
    assert!(client.server_info().init_attached);
    assert_eq!(
        client.server_info().protocol_version,
        brahman_card::PROTOCOL_VERSION
    );

    let mut last = 0u64;
    for _ in 0..3 {
        let ts = client.ping().await.unwrap();
        assert!(ts >= last);
        last = ts;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    client.farewell().await.unwrap();

    tokio::time::timeout(Duration::from_secs(2), session_handle)
        .await
        .expect("server hung after farewell")
        .unwrap();
}

#[tokio::test]
async fn list_sessions_returns_currently_registered() {
    // Levantamos un server con broker (requerido para que el registro
    // pase por el path real) y conectamos 3 clientes. El último pide
    // ListSessions y debe ver a los 2 anteriores + a sí mismo.
    let path = sock_path("listsess");
    let broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let server = Server::bind(
        &path,
        ServerConfig {
            init_attached: true,
            broker: Some(broker),
            net: None,
            policy: None,
        },
    )
    .unwrap();

    // Una task accept loop genérica para los 3 clientes.
    let server_handle = tokio::spawn(async move {
        for _ in 0..3 {
            let session = server.accept_one().await.unwrap();
            tokio::spawn(async move {
                let _ = session.handle().await;
            });
        }
        // Mantener el server vivo para que las sesiones puedan
        // mantenerse abiertas mientras el observer pregunta.
        std::future::pending::<()>().await;
    });

    let mut alpha = Client::connect(&path, sample_card("producer-alpha"))
        .await
        .unwrap();
    let mut beta = Client::connect(&path, sample_card("producer-beta"))
        .await
        .unwrap();
    // observer es el que va a preguntar.
    let mut observer = Client::connect(&path, sample_card("observer"))
        .await
        .unwrap();

    let list = observer.list_sessions().await.unwrap();
    assert_eq!(list.entries.len(), 3, "deberían verse 3 sesiones activas");

    let labels: BTreeSet<&str> = list.entries.iter().map(|e| e.label.as_str()).collect();
    assert!(labels.contains("producer-alpha"));
    assert!(labels.contains("producer-beta"));
    assert!(labels.contains("observer"));

    // schema_version + conscious sanity en la propia entry del observer.
    let me = list
        .entries
        .iter()
        .find(|e| e.label == "observer")
        .unwrap();
    assert_eq!(me.schema_version, brahman_card::CARD_SCHEMA_VERSION);
    assert!(!me.conscious, "observer no envió WIT — debería ser agnostic");

    alpha.farewell().await.unwrap();
    beta.farewell().await.unwrap();
    observer.farewell().await.unwrap();
    server_handle.abort();
}

#[tokio::test]
async fn rejects_invalid_card_client_side() {
    let path = sock_path("invalid");
    let server = Server::bind(&path, ServerConfig::default()).unwrap();
    let session_handle = tokio::spawn(async move {
        // No esperamos que el server complete: el cliente corta antes.
        let _ = tokio::time::timeout(Duration::from_secs(1), async move {
            let session = server.accept_one().await.unwrap();
            session.handle().await.unwrap();
        })
        .await;
    });

    let mut bad = sample_card("placeholder");
    bad.label = String::new();
    let err = Client::connect(&path, bad).await.unwrap_err();
    assert!(matches!(err, ClientError::InvalidCard(_)));

    session_handle.abort();
}

#[tokio::test]
async fn server_rejects_protocol_mismatch() {
    let path = sock_path("mismatch");
    let server = Server::bind(&path, ServerConfig::default()).unwrap();
    let session_handle = tokio::spawn(async move {
        let session = server.accept_one().await.unwrap();
        session.handle().await.unwrap();
    });

    let mut stream = UnixStream::connect(&path).await.unwrap();
    let hello = Hello {
        schema_version: CARD_SCHEMA_VERSION,
        protocol_version: "999.0.0".into(),
        card: sample_card("future-module").into(),
        wit: None,
        signature: None,
        identity_cert: None,
    };
    write_frame(&mut stream, &Frame::Hello(hello)).await.unwrap();

    match read_frame(&mut stream).await.unwrap() {
        Frame::Error(HandshakeError::ProtocolMismatch(_)) => {}
        other => panic!("esperado ProtocolMismatch, got {other:?}"),
    }

    tokio::time::timeout(Duration::from_secs(2), session_handle)
        .await
        .expect("server hung after rejecting")
        .unwrap();
}

// =====================================================================
// Integración handshake ↔ broker
// =====================================================================

fn card_with_flows(label: &str, input: Vec<Flow>, output: Vec<Flow>) -> Card {
    Card {
        schema_version: CARD_SCHEMA_VERSION,
        id: Ulid::new(),
        label: label.into(),
        soma: SomaSpec {
            cgroup: CgroupSpec {
                path: "ente.slice/test".into(),
                cpu_weight: None,
                io_weight: None,
            },
            namespaces: NamespaceSet::default(),
            rlimits: ResourceLimits::default(),
            cpu_affinity: None,
        },
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        flow: Flows { input, output },
        ..Default::default()
    }
}

fn flow(name: &str, ty: TypeRef) -> Flow {
    Flow {
        name: name.into(),
        ty,
        pin_to: None,
    }
}

/// Espera hasta que `broker.len() >= n` o timeout.
async fn wait_for_broker_len(broker: &Arc<Mutex<Broker>>, n: usize) {
    for _ in 0..50 {
        if broker.lock().await.len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("broker no alcanzó {n} entradas en 500ms");
}

#[tokio::test]
async fn broker_registers_and_unregisters_with_session() {
    let path = sock_path("broker-lifecycle");
    let broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let server = Server::bind(
        &path,
        ServerConfig {
            init_attached: false,
            broker: Some(broker.clone()),
            net: None,
            policy: None,
        },
    )
    .unwrap();

    let session_handle = tokio::spawn(async move {
        let session = server.accept_one().await.unwrap();
        session.handle().await.unwrap();
    });

    let mut client = Client::connect(&path, sample_card("alpha")).await.unwrap();
    let session_id = client.session();

    // Tras el handshake, la Card debe estar registrada en el broker.
    wait_for_broker_len(&broker, 1).await;
    {
        let b = broker.lock().await;
        assert_eq!(b.len(), 1);
        assert!(b.sessions().any(|s| s == session_id));
    }

    client.farewell().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), session_handle)
        .await
        .expect("server colgó tras farewell")
        .unwrap();

    // Tras el cleanup, el broker queda vacío.
    {
        let b = broker.lock().await;
        assert_eq!(b.len(), 0);
    }
}

#[tokio::test]
async fn broker_matches_two_live_modules() {
    let path = sock_path("broker-match");
    let broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let server = Server::bind(
        &path,
        ServerConfig {
            init_attached: false,
            broker: Some(broker.clone()),
            net: None,
            policy: None,
        },
    )
    .unwrap();

    // Server loop: usa la API run() para manejar accept+spawn.
    let server_handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    // Productor: emite "out" tipo string.
    let producer_card = card_with_flows(
        "dht",
        vec![],
        vec![flow(
            "out",
            TypeRef::Primitive {
                name: "string".into(),
            },
        )],
    );
    let mut producer = Client::connect(&path, producer_card).await.unwrap();
    wait_for_broker_len(&broker, 1).await;

    // Consumidor: pide "in" tipo string.
    let consumer_card = card_with_flows(
        "ui",
        vec![flow(
            "in",
            TypeRef::Primitive {
                name: "string".into(),
            },
        )],
        vec![],
    );
    let mut consumer = Client::connect(&path, consumer_card).await.unwrap();
    wait_for_broker_len(&broker, 2).await;

    // El broker debe encontrar el match consumer.in ← producer.out.
    let m = {
        let b = broker.lock().await;
        b.find_producer_for(consumer.session(), "in")
    }
    .expect("broker no encontró match");
    assert_eq!(m.consumer_label, "ui");
    assert_eq!(m.producer_label, "dht");
    assert_eq!(m.producer.flow_name, "out");

    // Cuando el productor se va, el match desaparece.
    producer.farewell().await.unwrap();
    for _ in 0..50 {
        if broker.lock().await.len() < 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    {
        let b = broker.lock().await;
        assert!(b.find_producer_for(consumer.session(), "in").is_none());
    }

    consumer.farewell().await.unwrap();
    server_handle.abort();
}

#[tokio::test]
async fn match_event_pushed_on_producer_arrival() {
    use brahman_handshake::messages::MatchEventKind;

    let path = sock_path("push-match");
    let broker = Arc::new(Mutex::new(Broker::new(BrokerConfig::default())));
    let server = Server::bind(
        &path,
        ServerConfig {
            init_attached: false,
            broker: Some(broker.clone()),
            net: None,
            policy: None,
        },
    )
    .unwrap();

    let server_handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    // El consumidor llega primero — sin productor, no hay match aún.
    let consumer_card = card_with_flows(
        "ui",
        vec![flow(
            "in",
            TypeRef::Primitive {
                name: "json".into(),
            },
        )],
        vec![],
    );
    let mut consumer = Client::connect(&path, consumer_card).await.unwrap();

    // No debería haber evento todavía.
    let no_event = consumer
        .await_event(Duration::from_millis(100))
        .await
        .unwrap();
    assert!(no_event.is_none(), "evento inesperado: {no_event:?}");

    // Llega el productor → consumer debe recibir Available.
    let producer_card = card_with_flows(
        "dht",
        vec![],
        vec![flow(
            "out",
            TypeRef::Primitive {
                name: "json".into(),
            },
        )],
    );
    let mut producer = Client::connect(&path, producer_card).await.unwrap();

    let ev = consumer
        .await_event(Duration::from_secs(2))
        .await
        .unwrap()
        .expect("Available no llegó");
    assert_eq!(ev.kind, MatchEventKind::Available);
    assert_eq!(ev.consumer_flow, "in");
    assert_eq!(ev.producer_label, "dht");
    assert_eq!(ev.producer_flow, "out");

    // El productor se va → consumer debe recibir Lost.
    producer.farewell().await.unwrap();
    let ev = consumer
        .await_event(Duration::from_secs(2))
        .await
        .unwrap()
        .expect("Lost no llegó");
    assert_eq!(ev.kind, MatchEventKind::Lost);
    assert_eq!(ev.consumer_flow, "in");

    consumer.farewell().await.unwrap();
    server_handle.abort();
}

#[tokio::test]
async fn ping_before_hello_rejected() {
    let path = sock_path("ping-no-hello");
    let server = Server::bind(&path, ServerConfig::default()).unwrap();
    let session_handle = tokio::spawn(async move {
        let session = server.accept_one().await.unwrap();
        session.handle().await.unwrap();
    });

    // Conectamos y mandamos un Ping sin haber saludado.
    let mut stream = UnixStream::connect(&path).await.unwrap();
    write_frame(
        &mut stream,
        &Frame::Ping(Ping {
            session: Ulid::new(),
        }),
    )
    .await
    .unwrap();

    match read_frame(&mut stream).await.unwrap() {
        Frame::Error(HandshakeError::Rejected(_)) => {}
        other => panic!("esperado Rejected, got {other:?}"),
    }

    tokio::time::timeout(Duration::from_secs(2), session_handle)
        .await
        .expect("server hung after rejecting")
        .unwrap();
}
