//! Backend libp2p del handshake brahman: el mismo protocolo (Hello /
//! HelloAck / Ping / Pong / MatchEvent / Farewell, frames postcard
//! length-prefixed) ahora también viaja sobre streams libp2p de la
//! malla `brahman-net`.
//!
//! El servidor expone el bucle [`run_libp2p_accept_loop`] que acepta
//! streams del protocolo `BRAHMAN_HANDSHAKE_PROTOCOL` y los delega al
//! mismo `Server` que ya escucha por Unix socket — la `Session` es
//! genérica sobre el transporte, así que ambas vías comparten broker,
//! tablas de sesiones, push de MatchEvents, todo.
//!
//! El cliente se conecta vía [`connect_libp2p`]: abre un stream
//! libp2p hacia un `PeerId` ya conocido y arranca el handshake como
//! cualquier `Client`.
//!
//! Identidad: cada nodo libp2p tiene su `PeerId` (ed25519 derivado).
//! La identidad del Ente (Card.id ULID + futura firma) viaja en el
//! Hello, como en el path Unix. Trust remoto (verificación de firma
//! antes de aceptar el Hello) es Fase 3.
//!
//! Ejemplo (servidor — Arje):
//! ```ignore
//! let server = Arc::new(Server::bind("/run/brahman-init.sock", config)?);
//! let net = Arc::new(BrahmanNet::new()?);
//! net.listen("/ip4/0.0.0.0/tcp/4101".parse()?).await;
//!
//! tokio::spawn(brahman_handshake::network::run_libp2p_accept_loop(
//!     server.clone(),
//!     net.clone(),
//! ));
//! // Server::run sigue escuchando Unix en paralelo.
//! ```
//!
//! Ejemplo (cliente — sidecar de un Ente remoto):
//! ```ignore
//! let net = BrahmanNet::new()?;
//! net.dial(remote_multiaddr);
//! let mut client = brahman_handshake::network::connect_libp2p(
//!     &net, peer_id, my_card, None,
//! ).await?;
//! client.ping().await?;
//! ```

use std::sync::Arc;

use brahman_card::{Card, TypeRef, WitInterface};
use brahman_net::{BrahmanNet, Keypair, OpenStreamError, PeerId, Stream, StreamProtocol};

use crate::identity::SessionCert;
use futures::StreamExt;
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};
use tracing::{debug, warn};

use crate::client::{Client, ClientError};
use crate::server::Server;

/// Sub-protocolo del handshake brahman sobre streams libp2p.
pub const BRAHMAN_HANDSHAKE_PROTOCOL: StreamProtocol =
    StreamProtocol::new("/brahman/handshake/1.0.0");

/// Tipo del stream que ve la lógica del handshake una vez convertido
/// del mundo `futures::AsyncRead/Write` (libp2p) al mundo
/// `tokio::io::AsyncRead/Write` (resto del crate).
pub type LibP2pHandshakeStream = Compat<Stream>;

/// Errores específicos del backend libp2p.
#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("abrir stream libp2p: {0}")]
    OpenStream(#[from] OpenStreamError),

    #[error("handshake: {0}")]
    Handshake(#[from] ClientError),

    #[error("aceptar stream libp2p: {0}")]
    AcceptStream(String),
}

/// Loop de aceptación de streams libp2p del protocolo handshake.
/// Cada stream entrante se construye como `Session` reutilizando las
/// tablas compartidas del `Server`, así que conviven con sesiones
/// Unix indistinguibles.
///
/// Vive hasta que el control libp2p se cierre o el caller drop el
/// future. Errores por sesión se loggean (no tumban el loop).
pub async fn run_libp2p_accept_loop(
    server: Arc<Server>,
    net: Arc<BrahmanNet>,
) -> Result<(), NetworkError> {
    let mut control = net.control.clone();
    let mut incoming = control
        .accept(BRAHMAN_HANDSHAKE_PROTOCOL)
        .map_err(|e| NetworkError::AcceptStream(e.to_string()))?;

    while let Some((peer, stream)) = incoming.next().await {
        let server = server.clone();
        // .compat() debe pasar al spawn ADENTRO; si lo hacemos afuera
        // y capturamos `Compat<Stream>` en la closure, el future
        // resultante hereda traits que dyn AsyncReadWrite no satisface
        // (compatibility con thread-safety de tokio::spawn).
        tokio::spawn(handle_libp2p_session(server, stream, peer));
    }

    Ok(())
}

async fn handle_libp2p_session(
    server: Arc<Server>,
    stream: Stream,
    peer: PeerId,
) {
    // session_from_libp2p_stream propaga el peer_id al `do_handshake`,
    // que exige firma del Hello cuya public key derive a este peer.
    let session = server.session_from_libp2p_stream(stream.compat(), peer);
    if let Err(e) = session.handle().await {
        warn!(
            target: "brahman_handshake::network",
            peer = %peer,
            error = %e,
            "sesión libp2p terminó con error"
        );
    }
}

/// Conecta como cliente a un Ente remoto vía libp2p y completa el
/// handshake **firmado** con `keypair`. Requiere que `net` ya tenga
/// conexión (o pueda dial-ar) al `peer`; típicamente el caller hace
/// `net.dial(multiaddr)` antes.
///
/// La `keypair` debe ser la misma que la del nodo libp2p (la que
/// pasaste a [`brahman_net::BrahmanNet::with_keypair`]). Si no coincide
/// con el `peer_id` autenticado por Noise, el server rechaza el Hello
/// con `Unauthorized`.
///
/// Devuelve un `Client` típico — los métodos `ping`, `await_event`,
/// `farewell` funcionan idéntico al path Unix. El stream subyacente
/// es libp2p convertido vía `tokio_util::compat`.
pub async fn connect_libp2p(
    net: &BrahmanNet,
    peer: PeerId,
    card: Card,
    wit: Option<WitInterface>,
    keypair: &Keypair,
) -> Result<Client<LibP2pHandshakeStream>, NetworkError> {
    let mut control = net.control.clone();
    let stream = control
        .open_stream(peer, BRAHMAN_HANDSHAKE_PROTOCOL)
        .await?;
    let client = Client::connect_with_stream_signed(stream.compat(), card, wit, keypair).await?;
    Ok(client)
}

/// Igual que `connect_libp2p` pero adjunta un `SessionCert` al Hello.
/// El server, al verificar el cert, evalúa la política de admisión
/// contra el `master_peer_id` derivado — no contra el `peer_id`
/// libp2p. Esto permite **rotar** la session keypair sin perder
/// reconocimiento en allowlists remotas.
///
/// El `keypair` debe ser la session libp2p (la que firma la conexión
/// Noise); el `cert` debe haber sido emitido por una identity master
/// para esa misma session pubkey (ver
/// [`crate::identity::Identity::issue_session_cert`]).
pub async fn connect_libp2p_with_cert(
    net: &BrahmanNet,
    peer: PeerId,
    card: Card,
    wit: Option<WitInterface>,
    session_keypair: &Keypair,
    cert: SessionCert,
) -> Result<Client<LibP2pHandshakeStream>, NetworkError> {
    let mut control = net.control.clone();
    let stream = control
        .open_stream(peer, BRAHMAN_HANDSHAKE_PROTOCOL)
        .await?;
    let client = Client::connect_with_stream_signed_with_cert(
        stream.compat(),
        card,
        wit,
        session_keypair,
        cert,
    )
    .await?;
    Ok(client)
}

// =====================================================================
// Discovery remoto via DHT — Fase 2
// =====================================================================
//
// Cuando un Ente registra una Card con outputs en el Init local, el
// Init anuncia al DHT (`net.start_providing(key)`) bajo una key
// derivada de `(flow_name, TypeRef)`. Cualquier nodo conectado al
// mismo DHT puede consultar `find_remote_providers(flow_name, type)`
// y obtener la lista de `PeerId`s que dijeron proveer ese flow.
//
// La key es **estable y libre de colisiones** entre versiones del
// monorepo: usa blake3 sobre un canon textual `brahman-flow|{name}|{type_canon}`.
// Cambiar la canonicalización rompe el discovery cross-version, así
// que cualquier modificación requiere bump de versión documentado.

/// Prefijo de namespace para todas las keys DHT del subprotocolo
/// brahman. Discrimina contra otros usos del mismo DHT (sync minga,
/// futuros) — protege contra colisiones accidentales.
const FLOW_KEY_PREFIX: &str = "brahman-flow|v1|";

/// Canonicaliza un `TypeRef` a string estable. Cambios aquí rompen
/// la compatibilidad de discovery cross-version; bump documentado
/// en `FLOW_KEY_PREFIX` al modificar.
fn canonicalize_type(t: &TypeRef) -> String {
    match t {
        TypeRef::Primitive { name } => format!("prim:{}", name),
        TypeRef::Wit {
            package,
            interface,
            name,
        } => format!(
            "wit:{}#{}#{}",
            package,
            interface.as_deref().unwrap_or(""),
            name
        ),
    }
}

/// Deriva la key del DHT para un `(flow_name, type_ref)` específico.
/// blake3-32B determinístico — la misma tupla en cualquier máquina
/// produce la misma key.
pub fn flow_dht_key(flow_name: &str, type_ref: &TypeRef) -> [u8; 32] {
    let canon = format!(
        "{}{}|{}",
        FLOW_KEY_PREFIX,
        flow_name,
        canonicalize_type(type_ref)
    );
    *blake3::hash(canon.as_bytes()).as_bytes()
}

/// Anuncia al DHT que este nodo provee cada output flow declarado
/// en `card`. Llamarlo tras `register_session` propaga la
/// disponibilidad a todos los peers que comparten DHT con éste.
///
/// Idempotente: re-anunciar la misma key actualiza el TTL del record
/// en el DHT. Best-effort: si `start_providing` falla por falta de
/// peers cercanos (DHT vacío), el record vive en el store local
/// hasta que llegue una conexión.
pub fn announce_outputs(net: &BrahmanNet, card: &Card) {
    for flow in &card.flow.output {
        let key = flow_dht_key(&flow.name, &flow.ty);
        debug!(
            target: "brahman_handshake::network",
            flow = %flow.name,
            "announce_output → DHT"
        );
        net.start_providing(&key);
    }
}

/// Retira los anuncios DHT previos de [`announce_outputs`] para esta
/// `card`. Llamado desde `cleanup` cuando una sesión cierra (Farewell,
/// EOF, error). El record local se borra al instante; copias
/// replicadas en peers remotos expiran por TTL natural de kad.
pub fn withdraw_outputs(net: &BrahmanNet, card: &Card) {
    for flow in &card.flow.output {
        let key = flow_dht_key(&flow.name, &flow.ty);
        debug!(
            target: "brahman_handshake::network",
            flow = %flow.name,
            "withdraw_output → DHT (stop_providing)"
        );
        net.stop_providing(&key);
    }
}

/// Consulta el DHT por peers que han anunciado proveer el flow
/// `(flow_name, type_ref)`. Devuelve la lista resuelta de `PeerId`s.
/// Lista vacía si nadie anuncia, si la query timeout-ea, o si el
/// DHT no ha encontrado providers.
///
/// Para cada `PeerId` devuelto, el caller puede luego dial-ar al
/// peer (a sus addrs conocidas vía Identify) y abrir un sub-handshake
/// remoto con [`connect_libp2p`].
pub async fn find_remote_providers(
    net: &BrahmanNet,
    flow_name: &str,
    type_ref: &TypeRef,
) -> Vec<PeerId> {
    let key = flow_dht_key(flow_name, type_ref);
    net.find_providers(&key).await
}
