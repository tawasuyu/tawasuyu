//! Mensajes del protocolo de handshake.
//!
//! Todos los mensajes que cruzan el wire son variantes de [`Frame`].

use std::path::PathBuf;

use brahman_broker::MatchStrategy;
use brahman_card::{TypeRef, WireCard, WitInterface};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Identificador de sesión emitido por el servidor en `HelloAck`.
pub type SessionId = Ulid;

/// Saludo inicial del módulo. Lleva la Card en forma `WireCard`
/// (postcard-friendly: sin extensiones JSON arbitrarias). El servidor
/// la convierte a `Card` para uso interno. Opcionalmente, una
/// `WitInterface` ya extraída — si está presente, el módulo es
/// "consciente" y el server lo registra como `ResolvedCard::from_conscious`.
///
/// **Firma (Fase 3, trust remoto)**: el campo `signature` es
/// obligatorio para conexiones libp2p (donde el server exige que la
/// public key derive al `peer_id` autenticado por Noise) y opcional
/// para Unix socket (donde SO_PEERCRED del kernel ya provee
/// autenticación). La firma cubre los bytes postcard de
/// `(WireCard, Option<WitInterface>)` — ver
/// [`HelloSignature::sign_payload`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// Versión del schema de Card que el cliente sigue.
    pub schema_version: u16,
    /// Versión del protocolo handshake del cliente.
    pub protocol_version: String,
    /// Tarjeta de Presentación, proyectada al wire.
    pub card: WireCard,
    /// Interfaz WIT extraída por el cliente (típicamente con
    /// `brahman-card-wit`). `None` si el módulo es agnóstico.
    #[serde(default)]
    pub wit: Option<WitInterface>,
    /// Firma Ed25519 sobre `(card, wit)`. Requerida para conexiones
    /// remotas (libp2p); opcional para Unix socket. Ver módulo
    /// [`super::signature`] para construcción y verificación.
    #[serde(default)]
    pub signature: Option<HelloSignature>,
    /// Cert opcional que vincula la session keypair (la que firma el
    /// Hello) a una **identity master** estable. Si está presente,
    /// la política de admisión se evalúa contra el `master_peer_id`
    /// derivado del cert — no contra el session peer_id. Esto permite
    /// rotar la session sin invalidar las allowlists remotas.
    ///
    /// Ver [`super::identity::SessionCert`] para shape y semantics.
    /// Si es `None`, fallback al modelo de Fase 3: la política
    /// evalúa el session peer_id directamente.
    #[serde(default)]
    pub identity_cert: Option<crate::identity::SessionCert>,
}

/// Firma de un Hello. La `public_key` viaja en el formato canónico
/// libp2p (protobuf) — el verificador la decodifica y compara su
/// `peer_id` derivado con la identidad libp2p autenticada por Noise.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelloSignature {
    /// Public key del firmante, encoded como `libp2p::identity::PublicKey::encode_protobuf()`.
    pub public_key: Vec<u8>,
    /// Bytes de la firma Ed25519 sobre el payload canonical.
    pub signature: Vec<u8>,
}

/// Respuesta del servidor a un `Hello` aceptado.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    /// Versión del crate del servidor.
    pub server_version: String,
    /// Versión del protocolo soportada por el servidor.
    pub protocol_version: String,
    /// Identificador de sesión asignado.
    pub session: SessionId,
    /// `true` si el Init está vinculado al servidor; `false` si el servidor
    /// corre standalone (modo degradado).
    pub init_attached: bool,
}

/// Latido del cliente. El servidor responde con [`Pong`] llevando su reloj.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ping {
    pub session: SessionId,
}

/// Respuesta a un `Ping` con timestamp del servidor (ms desde UNIX_EPOCH).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub timestamp_ms: u64,
}

/// Cierre cooperativo de la sesión por parte del cliente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Farewell {
    pub session: SessionId,
}

/// Errores del protocolo emitidos por el servidor.
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum HandshakeError {
    #[error("protocolo incompatible: {0}")]
    ProtocolMismatch(String),
    #[error("card inválida: {0}")]
    InvalidCard(String),
    #[error("schema de card incompatible: cliente={client}, servidor={server}")]
    SchemaMismatch { client: u16, server: u16 },
    #[error("sin autorización: {0}")]
    Unauthorized(String),
    #[error("capacidad requerida no satisfecha: {0}")]
    CapabilityUnmet(String),
    #[error("rechazado: {0}")]
    Rejected(String),
    #[error("error interno: {0}")]
    Internal(String),
}

/// Notificación push del server al consumer: un match disponible o perdido.
///
/// El server emite `Available` cuando un productor empieza a satisfacer un
/// `flow.input` del consumer (ya sea porque el productor acaba de
/// registrarse, o porque cambió el match anterior). Emite `Lost` cuando
/// el productor previo dejó de satisfacer el input (desregistro o
/// cambio de match).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchEvent {
    pub kind: MatchEventKind,
    /// Nombre del input del consumer al que aplica el evento.
    pub consumer_flow: String,
    /// Sesión y label del productor (en `Lost` puede ser nil/vacío).
    pub producer_session: SessionId,
    pub producer_label: String,
    pub producer_flow: String,
    /// Tipo del flujo matcheado.
    pub ty: TypeRef,
    /// Estrategia que ganó (relevante en `Available`).
    pub via: MatchStrategy,
    /// `true` si fue resuelto por `pin_to`.
    pub pinned: bool,
    /// Socket de servicio (data plane) que declaró el productor.
    /// Si está presente, el consumer puede conectar directo sin
    /// pasar por discovery adicional. `None` si el productor no
    /// declaró service_socket en su Card.
    #[serde(default)]
    pub producer_service_socket: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchEventKind {
    Available,
    Lost,
}

/// Pedido de listado de sesiones activas registradas en el broker. La
/// `session` es el id propio del que pregunta — el server lo valida
/// contra la sesión actual de la conexión, mismo patrón que `Ping`.
///
/// Pensado para herramientas de observabilidad (broker-explorer y
/// CLIs de diagnóstico). No expone secrets: sólo metadata pública
/// que el módulo ya anunció en su `Hello`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSessions {
    pub session: SessionId,
}

/// Una entrada en la respuesta a `ListSessions`. Slim por diseño —
/// el observer arma la UI con esto sin tener que abrir conexiones
/// adicionales por sesión.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub session: SessionId,
    /// Label declarado en `WireCard.label` — el "nombre humano" del
    /// módulo.
    pub label: String,
    /// Versión del schema de Card que el módulo declaró.
    pub schema_version: u16,
    /// Nombres de los `flow.output` que la Card declara producir.
    pub outputs: Vec<String>,
    /// Nombres de los `flow.input` que la Card declara consumir.
    pub inputs: Vec<String>,
    /// `true` si el módulo se anunció como "consciente" (trajo
    /// `WitInterface` extraída en el Hello).
    pub conscious: bool,
}

/// Respuesta a `ListSessions`. El orden no está garantizado — los
/// clientes que necesiten estabilidad pueden ordenar por `session`
/// (Ulid es ordenable temporal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionList {
    pub entries: Vec<SessionEntry>,
}

/// Pedido del listado de matches actuales del broker. La `session`
/// se valida igual que `ListSessions`. Si el server no tiene broker
/// configurado, devuelve la lista vacía (no es un error — refleja
/// que no hay matching activo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMatches {
    pub session: SessionId,
}

/// Respuesta a `ListMatches` con el snapshot de matches consumidor↔productor
/// actualmente computados por el broker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchList {
    pub matches: Vec<brahman_broker::Match>,
}

/// Frame único de wire — discriminada por variante. Cada conexión es un
/// stream de frames.
///
/// Direcciones:
/// - Cliente → Server: `Hello`, `Ping`, `Farewell`, `ListSessions`,
///   `ListMatches`.
/// - Server → Cliente: `HelloAck`, `Pong`, `Error`, `MatchEvent`,
///   `SessionList`, `MatchList`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Frame {
    Hello(Hello),
    HelloAck(HelloAck),
    Ping(Ping),
    Pong(Pong),
    Farewell(Farewell),
    Error(HandshakeError),
    MatchEvent(MatchEvent),
    ListSessions(ListSessions),
    SessionList(SessionList),
    ListMatches(ListMatches),
    MatchList(MatchList),
}
