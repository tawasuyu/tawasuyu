//! Servidor de handshake. Listener Unix socket → sesiones por conexión.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use brahman_broker::{Broker, Endpoint};
use brahman_card::{Card, ResolvedCard, WitInterface, CARD_SCHEMA_VERSION};
use brahman_net::{BrahmanNet, PeerId};
use tokio::io::{split, AsyncRead, AsyncWrite, WriteHalf};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, warn};
use ulid::Ulid;

use crate::codec::{read_frame, write_frame};
use crate::messages::{
    Farewell, Frame, HandshakeError, Hello, HelloAck, MatchEvent, MatchEventKind, Ping, Pong,
    SessionId,
};

/// Tabla de sesiones vivas indexada por `SessionId`.
pub type SessionRegistry = Arc<Mutex<HashMap<SessionId, ResolvedCard>>>;

/// Broker compartido (opcional) que el servidor mantiene en sincronía con
/// el ciclo de vida de las sesiones.
pub type SharedBroker = Arc<Mutex<Broker>>;

/// Tabla de canales push por sesión: el server inyecta frames hacia el
/// cliente (p. ej. `MatchEvent`) sin requerir que el cliente haga request.
type SessionTxTable = Arc<Mutex<HashMap<SessionId, mpsc::Sender<Frame>>>>;

/// Por sesión, último match conocido por nombre de input. Se usa para
/// emitir diffs (Available/Lost) en lugar del estado completo.
type LastMatches = Arc<Mutex<HashMap<SessionId, HashMap<String, Endpoint>>>>;

/// Capacidad del canal push por sesión. Si se llena (cliente lento), los
/// eventos extra se descartan — el cliente puede re-consultar el estado.
const PUSH_CHANNEL_CAPACITY: usize = 32;

/// Configuración del servidor.
#[derive(Clone, Default)]
pub struct ServerConfig {
    /// `true` si el Init está atado al servidor (se reporta en `HelloAck`).
    pub init_attached: bool,
    /// Broker compartido. Si está presente, el servidor llama
    /// `register` tras un Hello aceptado y `unregister` al cerrar la
    /// sesión (Farewell o EOF). Si es `None`, el broker no se usa.
    pub broker: Option<SharedBroker>,
    /// Capa P2P compartida. Si está presente, cada Card registrada
    /// con outputs se anuncia automáticamente al DHT vía
    /// [`brahman_handshake::network::announce_outputs`], permitiendo
    /// que un consumer remoto los descubra con
    /// [`brahman_handshake::network::find_remote_providers`]. Si es
    /// `None`, el server queda "ciego al DHT" — sólo matchea sesiones
    /// locales (lo cual es correcto cuando no hay conectividad o no
    /// se desea exponer al exterior).
    pub net: Option<Arc<BrahmanNet>>,
    /// Política de admisión de peers libp2p (allow + deny + hot
    /// reload opcional). Si está presente, el trust gate del path
    /// libp2p evalúa cada `peer_id` (ya autenticado por Noise)
    /// contra esta política. `None` → modo totalmente abierto
    /// (cualquier peer Ed25519-válido pasa). El path Unix la ignora.
    pub policy: Option<crate::peer_policy::PeerPolicy>,
}

// Manual Debug porque BrahmanNet no implementa Debug (libp2p Swarm
// no es Debug). Sólo loggeamos los campos relevantes para tracing.
impl std::fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerConfig")
            .field("init_attached", &self.init_attached)
            .field("broker", &self.broker.as_ref().map(|_| "<broker>"))
            .field("net", &self.net.as_ref().map(|_| "<net>"))
            .field("policy", &self.policy.as_ref().map(|p| p.sizes()))
            .finish()
    }
}

/// Servidor de handshake escuchando en un Unix socket.
pub struct Server {
    listener: UnixListener,
    socket_path: PathBuf,
    sessions: SessionRegistry,
    push_table: SessionTxTable,
    last_matches: LastMatches,
    config: ServerConfig,
}

impl Server {
    /// Crea el listener en `path`. Si el archivo existe, lo elimina (asume
    /// que es un socket stale de una sesión previa).
    pub fn bind(path: impl Into<PathBuf>, config: ServerConfig) -> std::io::Result<Self> {
        let socket_path = path.into();
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }
        if let Some(parent) = socket_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let listener = UnixListener::bind(&socket_path)?;
        Ok(Self {
            listener,
            socket_path,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            push_table: Arc::new(Mutex::new(HashMap::new())),
            last_matches: Arc::new(Mutex::new(HashMap::new())),
            config,
        })
    }

    /// Devuelve la ruta del socket (útil para clientes en el mismo proceso).
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Vista compartida del registro de sesiones — útil para el Init/Admin
    /// para inspeccionar quién está conectado.
    pub fn sessions(&self) -> SessionRegistry {
        self.sessions.clone()
    }

    /// Acepta UNA conexión Unix, devuelve la `Session` lista para `handle()`.
    /// No corre el handler — eso es responsabilidad del llamante.
    /// Path Unix → `expected_peer = None` (firma del Hello opcional;
    /// SO_PEERCRED del kernel ya autentica al cliente).
    pub async fn accept_one(&self) -> std::io::Result<Session<UnixStream>> {
        let (stream, _addr) = self.listener.accept().await?;
        Ok(self.session_from_stream(stream))
    }

    /// Construye una `Session` a partir de un stream arbitrario que
    /// implemente `AsyncRead + AsyncWrite + Unpin + Send`. Path
    /// agnóstico al transport (Unix, in-memory, etc.) — `expected_peer`
    /// queda en `None`, así que la firma del Hello es opcional.
    pub fn session_from_stream<S>(&self, stream: S) -> Session<S>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.session_from_stream_inner(stream, None)
    }

    /// Variante para conexiones libp2p: el `peer_id` viene autenticado
    /// por Noise. La sesión exige firma del Hello cuya public key
    /// derive a este `peer_id` exacto. Ver
    /// [`super::network::run_libp2p_accept_loop`].
    pub fn session_from_libp2p_stream<S>(
        &self,
        stream: S,
        peer: PeerId,
    ) -> Session<S>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        self.session_from_stream_inner(stream, Some(peer))
    }

    fn session_from_stream_inner<S>(
        &self,
        stream: S,
        expected_peer: Option<PeerId>,
    ) -> Session<S>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Session {
            stream,
            sessions: self.sessions.clone(),
            push_table: self.push_table.clone(),
            last_matches: self.last_matches.clone(),
            config: self.config.clone(),
            expected_peer,
        }
    }

    /// Loop de aceptación: cada conexión se despacha en una task separada.
    /// Vive hasta que el listener falle o el caller drop el future.
    pub async fn run(self) -> std::io::Result<()> {
        loop {
            let session = self.accept_one().await?;
            tokio::spawn(async move {
                if let Err(e) = session.handle().await {
                    warn!(error = %e, "session terminó con error");
                }
            });
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Limpieza best-effort del socket. Si falla, log y seguir.
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %self.socket_path.display(), error = %e, "no se pudo limpiar socket");
            }
        }
    }
}

/// Conexión individual aceptada por el servidor. Genérica sobre el
/// transport — funciona indistinguiblemente sobre `UnixStream` (modo
/// local), libp2p stream wrapped con `tokio_util::compat`, in-memory
/// duplex (tests), etc.
pub struct Session<S> {
    stream: S,
    sessions: SessionRegistry,
    push_table: SessionTxTable,
    last_matches: LastMatches,
    config: ServerConfig,
    /// Si está set, el path es libp2p y `do_handshake` exige firma
    /// del Hello cuya public key derive a este `peer_id`. Si es
    /// `None`, el path es Unix/in-memory y la firma es opcional
    /// (pero si está, se verifica anyway por defensa en profundidad).
    expected_peer: Option<PeerId>,
}

impl<S> Session<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    /// Procesa la conexión hasta `Farewell` o EOF.
    ///
    /// Estructura: handshake (sobre el stream entero) → split en
    /// halves (read + write) → reader loop principal + writer task
    /// que drena el push channel. Garantiza cleanup (sessions + broker
    /// + canales) sin importar la rama de salida.
    ///
    /// El split es necesario para soportar streams `!Sync` como
    /// `libp2p::Stream`: `tokio::select!` sobre `&mut self.stream`
    /// requeriría `S: Sync`. Con `tokio::io::split` cada mitad va a
    /// su propia task, eliminando el requirement y permitiendo que
    /// la misma `Session` sirva indistinta sobre Unix socket o
    /// stream libp2p remoto.
    pub async fn handle(self) -> std::io::Result<()> {
        let Self {
            mut stream,
            sessions,
            push_table,
            last_matches,
            config,
            expected_peer,
        } = self;

        let session_id = match do_handshake(&mut stream, &config, &sessions, expected_peer).await?
        {
            Some(id) => id,
            None => return Ok(()), // Hello rechazado, no se registró nada
        };

        let result = run_post_handshake(
            stream,
            session_id,
            sessions.clone(),
            push_table.clone(),
            last_matches.clone(),
            config.clone(),
        )
        .await;

        cleanup(
            session_id,
            &sessions,
            &push_table,
            &last_matches,
            &config,
        )
        .await;

        result
    }

}

// ============================================================================
// Free functions (post-refactor): la lógica del post-handshake corre sobre
// halves del stream; no necesita más `&mut Session<S>` y por eso vive afuera.
// ============================================================================

async fn run_post_handshake<S>(
    stream: S,
    session_id: SessionId,
    sessions: SessionRegistry,
    push_table: SessionTxTable,
    last_matches: LastMatches,
    config: ServerConfig,
) -> std::io::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Canal por donde el server inyecta frames push (MatchEvent, etc.).
    let (tx, mut rx) = mpsc::channel::<Frame>(PUSH_CHANNEL_CAPACITY);
    push_table.lock().await.insert(session_id, tx);

    // Tras registrar el canal, recomputar matches y emitir diffs.
    broadcast_match_diffs(&push_table, &last_matches, &config).await;

    // Split: reader en el hilo principal, writer compartido bajo Mutex
    // entre la writer task (push channel) y el handler de inbound
    // (que escribe Pong/Error). Mutex serializa writes; es OK porque
    // la frecuencia de writes por sesión es baja.
    let (mut reader, writer) = split(stream);
    let writer = Arc::new(Mutex::new(writer));

    // Writer task: drena el push channel.
    let writer_for_push = writer.clone();
    let writer_task = tokio::spawn(async move {
        while let Some(frame) = rx.recv().await {
            let mut w = writer_for_push.lock().await;
            if write_frame(&mut *w, &frame).await.is_err() {
                break;
            }
        }
    });

    // Reader loop principal.
    let broker_for_loop = config.broker.clone();
    let result: std::io::Result<()> = loop {
        match read_frame(&mut reader).await {
            Ok(frame) => {
                match handle_inbound_frame(
                    session_id,
                    frame,
                    &writer,
                    &sessions,
                    broker_for_loop.as_ref(),
                )
                .await
                {
                    Ok(true) => continue,
                    Ok(false) => break Ok(()),
                    Err(e) => break Err(e),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                debug!(session = %session_id, "cliente cerró sin Farewell");
                break Ok(());
            }
            Err(e) => break Err(e),
        }
    };

    // Cerrar writer: drop nuestro Arc y abortar la task. La task
    // saldrá igual cuando rx se cierre por drop del último Sender,
    // pero abortarla es más rápido que esperar a que próximo recv()
    // observe el cierre.
    drop(writer);
    writer_task.abort();
    let _ = writer_task.await;

    result
}

async fn handle_inbound_frame<S>(
    session_id: SessionId,
    frame: Frame,
    writer: &Arc<Mutex<WriteHalf<S>>>,
    sessions: &SessionRegistry,
    broker_for_match: Option<&SharedBroker>,
) -> std::io::Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    match frame {
        Frame::Ping(Ping { session }) if session == session_id => {
            let pong = Pong {
                timestamp_ms: now_ms(),
            };
            let mut w = writer.lock().await;
            write_frame(&mut *w, &Frame::Pong(pong)).await?;
            Ok(true)
        }
        Frame::Ping(_) => {
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::Error(HandshakeError::Unauthorized(
                    "session-id no coincide".into(),
                )),
            )
            .await?;
            Ok(true)
        }
        Frame::Farewell(Farewell { session }) if session == session_id => Ok(false),
        Frame::Farewell(_) => {
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::Error(HandshakeError::Unauthorized(
                    "session-id no coincide".into(),
                )),
            )
            .await?;
            Ok(true)
        }
        Frame::ListSessions(crate::messages::ListSessions { session })
            if session == session_id =>
        {
            let list = build_session_list(sessions).await;
            let mut w = writer.lock().await;
            write_frame(&mut *w, &Frame::SessionList(list)).await?;
            Ok(true)
        }
        Frame::ListSessions(_) => {
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::Error(HandshakeError::Unauthorized(
                    "session-id no coincide".into(),
                )),
            )
            .await?;
            Ok(true)
        }
        Frame::ListMatches(crate::messages::ListMatches { session })
            if session == session_id =>
        {
            let matches = match &broker_for_match {
                Some(b) => b.lock().await.all_matches(),
                None => Vec::new(),
            };
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::MatchList(crate::messages::MatchList { matches }),
            )
            .await?;
            Ok(true)
        }
        Frame::ListMatches(_) => {
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::Error(HandshakeError::Unauthorized(
                    "session-id no coincide".into(),
                )),
            )
            .await?;
            Ok(true)
        }
        _ => {
            let mut w = writer.lock().await;
            write_frame(
                &mut *w,
                &Frame::Error(HandshakeError::Rejected(
                    "frame inesperado tras handshake".into(),
                )),
            )
            .await?;
            Ok(true)
        }
    }
}

/// Snapshot read-only de la `SessionRegistry` proyectado a la forma
/// de wire para el frame `SessionList`. Suelta el lock antes de
/// retornar para que el writer del frame no contenga el mutex.
async fn build_session_list(sessions: &SessionRegistry) -> crate::messages::SessionList {
    let table = sessions.lock().await;
    let entries = table
        .iter()
        .map(|(id, resolved)| crate::messages::SessionEntry {
            session: *id,
            label: resolved.card.label.clone(),
            schema_version: resolved.card.schema_version,
            outputs: resolved
                .card
                .flow
                .output
                .iter()
                .map(|f| f.name.clone())
                .collect(),
            inputs: resolved
                .card
                .flow
                .input
                .iter()
                .map(|f| f.name.clone())
                .collect(),
            conscious: resolved.wit.is_some(),
        })
        .collect();
    crate::messages::SessionList { entries }
}

/// Limpieza atómica de las vistas registradas + (si net activo) retiro
/// de anuncios DHT de los outputs de la Card. Se ejecuta tanto si la
/// sesión cierra por Farewell, EOF, o error. Tras desregistrar, emite
/// diffs a las sesiones que perdieron el match contra ésta.
async fn cleanup(
    session_id: SessionId,
    sessions: &SessionRegistry,
    push_table: &SessionTxTable,
    last_matches: &LastMatches,
    config: &ServerConfig,
) {
    // Tomamos la Card ANTES de borrarla — si net está configurado
    // necesitamos sus outputs para llamar withdraw_outputs. `remove`
    // devuelve el valor extraído.
    let removed_card = sessions.lock().await.remove(&session_id);
    push_table.lock().await.remove(&session_id);
    last_matches.lock().await.remove(&session_id);
    if let Some(broker) = &config.broker {
        broker.lock().await.unregister(session_id);
    }
    if let (Some(net), Some(resolved)) = (&config.net, removed_card) {
        crate::network::withdraw_outputs(net, &resolved.card);
    }
    broadcast_match_diffs(push_table, last_matches, config).await;
}

/// Recomputa los matches para todas las sesiones registradas y empuja
/// `MatchEvent::Available` / `MatchEvent::Lost` por las que cambiaron
/// respecto al último estado conocido.
async fn broadcast_match_diffs(
    push_table: &SessionTxTable,
    last_matches: &LastMatches,
    config: &ServerConfig,
) {
    let broker = match &config.broker {
        Some(b) => b,
        None => return,
    };

    let b = broker.lock().await;
    let push_table = push_table.lock().await;
    let mut last = last_matches.lock().await;

    debug!(
        target: "brahman_handshake::broadcast",
        cards = b.len(),
        push_subscribers = push_table.len(),
        "broadcast_match_diffs"
    );

    let cards: Vec<_> = b.cards().cloned().collect();

    for cons in &cards {
        let cons_session = cons.session;
        let tx = match push_table.get(&cons_session) {
            Some(tx) => tx,
            None => continue,
        };
        let cons_last = last.entry(cons_session).or_default();

        for input in &cons.inputs {
            let new_match = b.find_producer_for(cons_session, &input.name);
            let new_endpoint = new_match.as_ref().map(|m| m.producer.clone());
            let old_endpoint = cons_last.get(&input.name).cloned();

            if old_endpoint == new_endpoint {
                continue;
            }

            if let Some(m) = &new_match {
                let producer_service_socket = b
                    .cards()
                    .find(|c| c.session == m.producer.session)
                    .and_then(|c| c.service_socket.clone());
                let event = MatchEvent {
                    kind: MatchEventKind::Available,
                    consumer_flow: input.name.clone(),
                    producer_session: m.producer.session,
                    producer_label: m.producer_label.clone(),
                    producer_flow: m.producer.flow_name.clone(),
                    ty: m.ty.clone(),
                    via: m.via,
                    pinned: m.pinned,
                    producer_service_socket,
                };
                let send_res = tx.try_send(Frame::MatchEvent(event));
                debug!(
                    target: "brahman_handshake::broadcast",
                    consumer = %cons_session,
                    flow = %input.name,
                    producer = %m.producer_label,
                    result = ?send_res.as_ref().map(|_| "ok").unwrap_or_else(|e| match e {
                        tokio::sync::mpsc::error::TrySendError::Full(_) => "full",
                        tokio::sync::mpsc::error::TrySendError::Closed(_) => "closed",
                    }),
                    "Available pushed"
                );
            } else {
                let event = MatchEvent {
                    kind: MatchEventKind::Lost,
                    consumer_flow: input.name.clone(),
                    producer_session: Ulid::nil(),
                    producer_label: String::new(),
                    producer_flow: String::new(),
                    ty: input.ty.clone(),
                    via: brahman_broker::MatchStrategy::Exact,
                    pinned: false,
                    producer_service_socket: None,
                };
                let _ = tx.try_send(Frame::MatchEvent(event));
            }

            if let Some(ep) = new_endpoint {
                cons_last.insert(input.name.clone(), ep);
            } else {
                cons_last.remove(&input.name);
            }
        }
    }
}

/// Lee el Hello, valida (incluyendo firma cuando aplica), registra la
/// sesión y emite HelloAck.
async fn do_handshake<S>(
    stream: &mut S,
    config: &ServerConfig,
    sessions: &SessionRegistry,
    expected_peer: Option<PeerId>,
) -> std::io::Result<Option<SessionId>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let frame = read_frame(stream).await?;
    let hello = match frame {
        Frame::Hello(h) => h,
        _ => {
            write_frame(
                stream,
                &Frame::Error(HandshakeError::Rejected(
                    "primer frame debe ser Hello".into(),
                )),
            )
            .await?;
            return Ok(None);
        }
    };

    if let Some(err) = validate_hello(&hello) {
        write_frame(stream, &Frame::Error(err)).await?;
        return Ok(None);
    }

    // Identity cert (multi-key identity, opcional): si el cliente
    // adjuntó cert, la "identidad lógica" del peer es el master
    // derivado del cert (estable across rotaciones), no el session
    // peer_id (efímero). Sin cert, fallback al modelo de Fase 3
    // (logical = session). Esto permite migración gradual y
    // backwards compatibility con clientes que no usan identity.
    let logical_peer = if let (Some(session_peer), Some(cert)) =
        (expected_peer, &hello.identity_cert)
    {
        let session_pk_bytes: &[u8] = match &hello.signature {
            Some(sig) => &sig.public_key,
            None => {
                write_frame(
                    stream,
                    &Frame::Error(HandshakeError::Unauthorized(
                        "Hello con identity_cert requiere también signature".into(),
                    )),
                )
                .await?;
                return Ok(None);
            }
        };
        match cert.verify_against_session(session_pk_bytes) {
            Ok(master_peer) => {
                debug!(
                    session = %session_peer,
                    master = %master_peer,
                    "identity cert válido — policy se evalúa contra master_peer"
                );
                Some(master_peer)
            }
            Err(e) => {
                write_frame(
                    stream,
                    &Frame::Error(HandshakeError::Unauthorized(format!(
                        "identity cert inválido: {e}"
                    ))),
                )
                .await?;
                debug!(peer = %session_peer, error = %e, "cert rechazado");
                return Ok(None);
            }
        }
    } else {
        expected_peer
    };

    // Policy gate (path libp2p): si está configurada, el peer
    // autenticado debe pasar la política (deny first, luego allow).
    // El peer evaluado es `logical_peer`: master si hay cert,
    // session si no. Se chequea ANTES de la firma porque es
    // comparación O(log n) sin crypto. La política no se aplica
    // al path Unix (autenticación por SO_PEERCRED, no por PeerId).
    if let (Some(peer), Some(policy)) = (logical_peer, &config.policy) {
        let decision = policy.evaluate(&peer);
        if !decision.is_admitted() {
            write_frame(
                stream,
                &Frame::Error(HandshakeError::Unauthorized(format!(
                    "peer {peer}: {}",
                    decision.reason()
                ))),
            )
            .await?;
            debug!(peer = %peer, reason = decision.reason(), "rechazado por policy");
            return Ok(None);
        }
    }

    // Trust gate: en path libp2p (expected_peer = Some), exigir
    // firma cuya public key derive al peer autenticado por Noise.
    // En path Unix (expected_peer = None), si la firma viene se
    // verifica anyway por defensa en profundidad — no es un error
    // que esté ahí, pero si está debe ser válida.
    if let Some(peer) = expected_peer {
        let sig = match &hello.signature {
            Some(s) => s,
            None => {
                write_frame(
                    stream,
                    &Frame::Error(HandshakeError::Unauthorized(
                        "Hello sin firma en conexión remota libp2p".into(),
                    )),
                )
                .await?;
                return Ok(None);
            }
        };
        if let Err(e) = crate::signature::verify_hello(sig, &hello.card, &hello.wit, peer) {
            write_frame(
                stream,
                &Frame::Error(HandshakeError::Unauthorized(format!("firma inválida: {e}"))),
            )
            .await?;
            debug!(peer = %peer, error = %e, "firma rechazada");
            return Ok(None);
        }
    } else if let Some(sig) = &hello.signature {
        // Firma presente en path local: no exigida pero verificada.
        // Si está y no valida, es un signo de Hello mal-construido y
        // rechazamos por seguridad.
        // Para Unix no tenemos peer_id contra el cual comparar; se
        // verifica sólo la consistencia interna (firma sobre payload
        // con la public_key declarada).
        match brahman_net::PublicKey::try_decode_protobuf(&sig.public_key) {
            Ok(pk) => {
                let payload = match postcard::to_allocvec(&(
                    crate::signature::SIGNATURE_VERSION,
                    &hello.card,
                    &hello.wit,
                )) {
                    Ok(b) => b,
                    Err(_) => {
                        write_frame(
                            stream,
                            &Frame::Error(HandshakeError::Internal(
                                "no pude codificar payload para verificar firma".into(),
                            )),
                        )
                        .await?;
                        return Ok(None);
                    }
                };
                if !pk.verify(&payload, &sig.signature) {
                    write_frame(
                        stream,
                        &Frame::Error(HandshakeError::Unauthorized(
                            "firma del Hello presente pero inválida".into(),
                        )),
                    )
                    .await?;
                    return Ok(None);
                }
            }
            Err(e) => {
                write_frame(
                    stream,
                    &Frame::Error(HandshakeError::Unauthorized(format!(
                        "public_key inválida en firma: {e}"
                    ))),
                )
                .await?;
                return Ok(None);
            }
        }
    }

    let session_id = Ulid::new();
    let card: Card = hello.card.into();
    register_session(session_id, card, hello.wit, config, sessions).await;

    let ack = HelloAck {
        server_version: crate::HANDSHAKE_VERSION.to_string(),
        protocol_version: brahman_card::PROTOCOL_VERSION.to_string(),
        session: session_id,
        init_attached: config.init_attached,
    };
    write_frame(stream, &Frame::HelloAck(ack)).await?;
    debug!(session = %session_id, "handshake completado");
    Ok(Some(session_id))
}

async fn register_session(
    session_id: SessionId,
    card: Card,
    wit: Option<WitInterface>,
    config: &ServerConfig,
    sessions: &SessionRegistry,
) {
    if let Some(broker) = &config.broker {
        broker
            .lock()
            .await
            .register(session_id, &card, wit.clone());
    }
    // Si el server tiene net configurado, anunciar los outputs al
    // DHT para que peers remotos puedan descubrirlos. Idempotente
    // y best-effort — fallos de Kad no propagan al handshake.
    if let Some(net) = &config.net {
        crate::network::announce_outputs(net, &card);
    }
    let resolved = match wit {
        Some(w) => ResolvedCard::from_conscious(card, w),
        None => ResolvedCard::from_agnostic(card),
    };
    sessions.lock().await.insert(session_id, resolved);
}

fn validate_hello(hello: &Hello) -> Option<HandshakeError> {
    if hello.schema_version != CARD_SCHEMA_VERSION {
        return Some(HandshakeError::SchemaMismatch {
            client: hello.schema_version,
            server: CARD_SCHEMA_VERSION,
        });
    }
    if hello.protocol_version != brahman_card::PROTOCOL_VERSION {
        return Some(HandshakeError::ProtocolMismatch(format!(
            "cliente={}, servidor={}",
            hello.protocol_version,
            brahman_card::PROTOCOL_VERSION
        )));
    }
    let as_card: Card = Card::from(hello.card.clone());
    if let Err(e) = as_card.validate() {
        return Some(HandshakeError::InvalidCard(e.to_string()));
    }
    None
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
