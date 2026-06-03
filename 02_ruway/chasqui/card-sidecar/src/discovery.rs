//! `brahman-sidecar::discovery` — API reusable para que un módulo
//! consumer encuentre proveedores vivos vía broker, sin hardcodear
//! sockets ni reimplementar el patrón a mano.
//!
//! Es la generalización de `discover_producer_socket` del CLI
//! `chasqui attract --remote`: declarás el `TypeRef` que querés
//! consumir y el broker te empuja un `MatchEvent::Available` con el
//! `producer_service_socket` del primer proveedor matched.
//!
//! Pipeline:
//! 1. `build_consumer_card(label, flow_name, type_name)` arma una
//!    Card mínima (Ente, Oneshot, Virtual) con un input flow.
//! 2. `await_provider(card, timeout)` se conecta al brahman-init,
//!    espera hasta `timeout` por `MatchEvent::Available`, devuelve
//!    el socket del proveedor electo, y envía Farewell.
//! 3. Para mundos blocking (CLIs, tests, std-thread loops) hay
//!    `await_provider_blocking` que arma su propio runtime
//!    `current_thread`.
//!
//! Quién elige al proveedor es el broker, no este módulo. Si el
//! broker tiene `priority_contexts` activo, podés cambiar de
//! proveedor sin tocar el consumer; el matching dinámico se respeta.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use card_core::{
    Card, CardKind, Flow, Flows, Lifecycle, Payload, Priority, Supervision, TypeRef, WitInterface,
};
use card_net::{BrahmanNet, PeerId};
use card_handshake::client::{Client, ClientError};
use card_handshake::messages::MatchEventKind;
use card_handshake::network::{connect_libp2p, LibP2pHandshakeStream, NetworkError};
use card_handshake::transport;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum ConsumerError {
    #[error("no se pudo conectar al init en {socket}: {source}")]
    Connect {
        socket: PathBuf,
        #[source]
        source: ClientError,
    },
    #[error("error en cliente brahman: {0}")]
    Client(#[from] ClientError),
    #[error("timeout {timeout:?} sin proveedor disponible para flow '{flow}' (type '{type_ref}')")]
    NoProvider {
        flow: String,
        type_ref: String,
        timeout: Duration,
    },
    #[error("no se pudo crear runtime tokio: {0}")]
    Runtime(String),
    #[error("la Card consumer no declara ningún input flow para resolver")]
    NoInputFlow,
    #[error(
        "el DHT no anuncia proveedores para flow '{flow}' (type '{type_ref}')"
    )]
    NoRemoteProviders { flow: String, type_ref: String },
    #[error(
        "todos los {} peers remotos fallaron al conectar; último error: {source}",
        peers.len()
    )]
    AllPeersFailed {
        peers: Vec<PeerId>,
        #[source]
        source: NetworkError,
    },
}

/// Construye una Card mínima de consumer que declara un input flow
/// con el `TypeRef::Primitive { name }` solicitado. Usá esto para
/// el caso común; si necesitás algo más rico (output flows,
/// permissions, references) construí la Card a mano y pasala a
/// [`await_provider`] directamente.
pub fn build_consumer_card(
    consumer_label: impl Into<String>,
    flow_name: impl Into<String>,
    type_name: impl Into<String>,
) -> Card {
    Card {
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::Oneshot,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        flow: Flows {
            input: vec![Flow {
                name: flow_name.into(),
                ty: TypeRef::Primitive {
                    name: type_name.into(),
                },
                pin_to: None,
            }],
            output: vec![],
        },
        ..Card::new(consumer_label)
    }
}

/// Conecta al brahman-init, registra `consumer_card`, espera el
/// primer `MatchEvent::Available` y devuelve el `producer_service_socket`
/// que el broker emitió. Cierra la sesión con Farewell antes de
/// retornar (best-effort).
///
/// La `consumer_card` debe declarar al menos un `flow.input`; si no,
/// el broker no puede hacer matching y el await siempre dará timeout.
pub async fn await_provider(
    consumer_card: Card,
    timeout: Duration,
) -> Result<PathBuf, ConsumerError> {
    let init_path = transport::default_socket_path();

    // Capturamos descriptor para el mensaje de error antes de mover
    // la card al cliente.
    let (flow_name, type_ref_name) = describe_first_input(&consumer_card);

    let mut client = Client::connect(&init_path, consumer_card)
        .await
        .map_err(|source| ConsumerError::Connect {
            socket: init_path.clone(),
            source,
        })?;

    let deadline = Instant::now() + timeout;
    let socket = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break None;
        }
        match client.await_event(remaining).await? {
            Some(ev) if ev.kind == MatchEventKind::Available => {
                break ev.producer_service_socket;
            }
            Some(_) => continue, // Lost u otros: seguir esperando hasta el deadline
            None => break None,
        }
    };

    let _ = client.farewell().await; // best-effort cleanup

    socket.ok_or(ConsumerError::NoProvider {
        flow: flow_name,
        type_ref: type_ref_name,
        timeout,
    })
}

/// Wrapper bloqueante de [`await_provider`]. Crea un runtime tokio
/// `current_thread` efímero y bloquea el thread llamador. Útil para
/// CLIs, tests y módulos std-thread (p. ej. el frontend GPUI antes
/// de tener su propio runtime async).
pub fn await_provider_blocking(
    consumer_card: Card,
    timeout: Duration,
) -> Result<PathBuf, ConsumerError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| ConsumerError::Runtime(e.to_string()))?;

    rt.block_on(await_provider(consumer_card, timeout))
}

// =====================================================================
// Discovery local-first / remoto (DHT) — capa de consumidor
// =====================================================================

/// Discovery DHT crudo, reexportado desde `card-handshake` para que un
/// consumidor que ya usa `card-sidecar` pueda buscar proveedores remotos
/// por `(flow_name, TypeRef)` sin alcanzar las internas del handshake.
pub use card_handshake::network::find_remote_providers;

/// Dónde vive el proveedor elegido para un flow.
///
/// El broker local (vía `await_provider`) entrega un socket Unix listo
/// para consumir; el DHT remoto entrega `PeerId`s que el caller debe
/// dial-ar y sub-handshakear con `card_handshake::network::connect_libp2p`.
#[derive(Debug, Clone)]
pub enum ProviderLocation {
    /// Proveedor local: socket de servicio resuelto por el broker del init.
    Local(PathBuf),
    /// Proveedores remotos anunciados en el DHT (peers a dial-ar).
    Remote(Vec<PeerId>),
}

/// Resuelve un proveedor para el **primer input flow** de `consumer_card`,
/// **local primero, DHT remoto como fallback**.
///
/// 1. Pregunta al broker del init local (`await_provider`) hasta
///    `local_timeout`. Si hay match → [`ProviderLocation::Local`].
/// 2. Si no hay proveedor local (timeout) ni init local alcanzable
///    (socket ausente), consulta el DHT vía `net` y devuelve
///    [`ProviderLocation::Remote`] con los `PeerId`s que anuncian el flow
///    (lista posiblemente vacía si nadie lo provee en la malla).
///
/// Esto es el cableado que faltaba entre el discovery local (broker) y el
/// discovery remoto (DHT de `card-net`, ya cableado en el handshake): un
/// consumidor pide "un proveedor para el tipo X" y obtiene el más cercano
/// disponible sin saber a priori si vive en esta máquina o en la red.
pub async fn resolve_provider(
    consumer_card: Card,
    net: &BrahmanNet,
    local_timeout: Duration,
) -> Result<ProviderLocation, ConsumerError> {
    // Necesitamos el flow a resolver antes de mover la card al broker.
    let (flow_name, ty) = match consumer_card.flow.input.first() {
        Some(f) => (f.name.clone(), f.ty.clone()),
        None => return Err(ConsumerError::NoInputFlow),
    };

    // 1) Local primero.
    match await_provider(consumer_card, local_timeout).await {
        Ok(socket) => return Ok(ProviderLocation::Local(socket)),
        // Sin proveedor local (timeout) o sin init local alcanzable:
        // caemos al DHT. Cualquier otro error sí se propaga.
        Err(ConsumerError::NoProvider { .. }) | Err(ConsumerError::Connect { .. }) => {}
        Err(other) => return Err(other),
    }

    // 2) Fallback remoto por DHT.
    let peers = find_remote_providers(net, &flow_name, &ty).await;
    Ok(ProviderLocation::Remote(peers))
}

/// Consume una `Card` a través de un proveedor remoto descubierto por DHT.
///
/// Recorre `peers` en orden, intentando `connect_libp2p` con cada uno hasta
/// que uno establezca handshake. La sesión devuelta es un `Client` idéntico
/// al del path Unix: `ping`, `await_event`, `farewell` funcionan igual; el
/// stream subyacente es libp2p convertido vía `tokio_util::compat`.
///
/// La keypair que se usa para firmar el handshake es la del propio `net`
/// (`BrahmanNet::keypair`) — la misma que autentica la conexión Noise, así
/// que el `peer_id` del Hello coincide con el del transporte y el server
/// no rechaza por `Unauthorized`.
///
/// Errores:
/// * [`ConsumerError::NoRemoteProviders`] si `peers` está vacío.
/// * [`ConsumerError::AllPeersFailed`] si ningún peer aceptó el handshake;
///   el `source` es el último `NetworkError` observado.
///
/// Atajo de uso típico tras `resolve_provider`:
/// ```ignore
/// match resolve_provider(card.clone(), &net, timeout).await? {
///     ProviderLocation::Local(socket) => {
///         Client::connect_with(&socket, card, wit).await?
///     }
///     ProviderLocation::Remote(peers) => {
///         consume_remote(&net, card, wit, peers).await?
///     }
/// };
/// ```
pub async fn consume_remote(
    net: &BrahmanNet,
    consumer_card: Card,
    wit: Option<WitInterface>,
    peers: Vec<PeerId>,
) -> Result<Client<LibP2pHandshakeStream>, ConsumerError> {
    if peers.is_empty() {
        let (flow, type_ref) = describe_first_input(&consumer_card);
        return Err(ConsumerError::NoRemoteProviders { flow, type_ref });
    }

    let keypair = net.keypair();
    let mut last_error: Option<NetworkError> = None;

    for peer in &peers {
        match connect_libp2p(net, *peer, consumer_card.clone(), wit.clone(), &keypair).await {
            Ok(client) => return Ok(client),
            Err(e) => {
                warn!(
                    target: "card_sidecar",
                    peer = %peer,
                    error = %e,
                    "dial al peer remoto falló, intento siguiente",
                );
                last_error = Some(e);
            }
        }
    }

    Err(ConsumerError::AllPeersFailed {
        peers,
        source: last_error.expect("el loop garantiza al menos un error si peers no era vacío"),
    })
}

/// Conecta al brahman-init con una Card observer (sin inputs ni
/// outputs) y pide la lista de sesiones activas. Útil para
/// herramientas de observabilidad (broker-explorer, CLIs).
///
/// El observer se identifica con `observer_label`. La sesión se
/// cierra con Farewell antes de retornar (best-effort).
pub async fn list_sessions(
    observer_label: impl Into<String>,
) -> Result<card_handshake::messages::SessionList, ConsumerError> {
    let init_path = transport::default_socket_path();
    // Card mínima sin flow.input/output: el observer no participa en
    // matching, sólo establece sesión para poder consultar.
    let card = Card {
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::Oneshot,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        flow: Flows {
            input: vec![],
            output: vec![],
        },
        ..Card::new(observer_label)
    };

    let mut client = Client::connect(&init_path, card)
        .await
        .map_err(|source| ConsumerError::Connect {
            socket: init_path.clone(),
            source,
        })?;

    let list = client.list_sessions().await?;
    let _ = client.farewell().await;
    Ok(list)
}

/// Wrapper bloqueante de [`list_sessions`]. Idéntico patrón a
/// `await_provider_blocking`: runtime current_thread efímero.
pub fn list_sessions_blocking(
    observer_label: impl Into<String>,
) -> Result<card_handshake::messages::SessionList, ConsumerError> {
    let label = observer_label.into();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| ConsumerError::Runtime(e.to_string()))?;
    rt.block_on(list_sessions(label))
}

/// Análogo a `list_sessions` pero pide los matches activos del
/// broker. La Card observer es la misma forma minimalista (sin
/// flow.input/output) — el endpoint no requiere participar en
/// matching.
pub async fn list_matches(
    observer_label: impl Into<String>,
) -> Result<card_handshake::messages::MatchList, ConsumerError> {
    let init_path = transport::default_socket_path();
    let card = Card {
        payload: Payload::Virtual,
        supervision: Supervision::OneShot,
        lifecycle: Lifecycle::Oneshot,
        priority: Priority::Normal,
        kind: CardKind::Ente,
        flow: Flows {
            input: vec![],
            output: vec![],
        },
        ..Card::new(observer_label)
    };

    let mut client = Client::connect(&init_path, card)
        .await
        .map_err(|source| ConsumerError::Connect {
            socket: init_path.clone(),
            source,
        })?;

    let list = client.list_matches().await?;
    let _ = client.farewell().await;
    Ok(list)
}

/// Wrapper bloqueante de [`list_matches`].
pub fn list_matches_blocking(
    observer_label: impl Into<String>,
) -> Result<card_handshake::messages::MatchList, ConsumerError> {
    let label = observer_label.into();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| ConsumerError::Runtime(e.to_string()))?;
    rt.block_on(list_matches(label))
}

fn describe_first_input(card: &Card) -> (String, String) {
    match card.flow.input.first() {
        Some(flow) => {
            let type_name = match &flow.ty {
                TypeRef::Primitive { name } => name.clone(),
                TypeRef::Wit { package, name, .. } => format!("{package}#{name}"),
            };
            (flow.name.clone(), type_name)
        }
        None => ("(sin input)".into(), "(sin tipo)".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use card_core::ulid::Ulid;

    #[test]
    fn builder_sets_input_flow_with_primitive_type() {
        let c = build_consumer_card("chasqui.cli", "embed-result", "json");
        assert_eq!(c.label, "chasqui.cli");
        assert_eq!(c.kind, CardKind::Ente);
        assert!(matches!(c.lifecycle, Lifecycle::Oneshot));
        assert!(matches!(c.supervision, Supervision::OneShot));
        assert_eq!(c.flow.input.len(), 1);
        let f = &c.flow.input[0];
        assert_eq!(f.name, "embed-result");
        match &f.ty {
            TypeRef::Primitive { name } => assert_eq!(name, "json"),
            _ => panic!("expected primitive type"),
        }
        assert!(c.flow.output.is_empty());
        // El builder asigna un id real (no nil) — fundamental para que
        // el broker no colisione con otros consumers.
        assert!(c.id != Ulid::nil(), "consumer card id no debe ser nil");
    }

    #[test]
    fn builder_assigns_distinct_ids_per_call() {
        let a = build_consumer_card("a", "f", "t");
        let b = build_consumer_card("a", "f", "t");
        assert_ne!(a.id, b.id, "cada Card debería tener id propio");
    }

    #[test]
    fn describe_falls_back_when_no_input_flow() {
        let mut c = build_consumer_card("x", "f", "t");
        c.flow.input.clear();
        let (flow, ty) = describe_first_input(&c);
        assert_eq!(flow, "(sin input)");
        assert_eq!(ty, "(sin tipo)");
    }

    #[test]
    fn describe_formats_wit_type() {
        let mut c = build_consumer_card("x", "f", "t");
        c.flow.input[0].ty = TypeRef::Wit {
            package: "brahman:dht".into(),
            interface: None,
            name: "entity-result".into(),
        };
        let (_, ty) = describe_first_input(&c);
        assert_eq!(ty, "brahman:dht#entity-result");
    }
}
