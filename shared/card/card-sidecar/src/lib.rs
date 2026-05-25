//! `brahman-sidecar` — boilerplate del cliente brahman extraído.
//!
//! Cualquier módulo que quiera presentarse al Init brahman pero que tenga
//! su propio runtime (GPUI, current_thread tokio, std-thread loop, etc.)
//! puede llamar [`spawn`] con su [`brahman_card::Card`]. Eso arma un
//! thread aparte con un runtime tokio current_thread, conecta al Init,
//! y mantiene la sesión viva con pings periódicos.
//!
//! Si el Init no está disponible, el thread loggea y termina — el módulo
//! sigue funcionando standalone.
//!
//! Errores de conexión / ping se loggean vía `tracing::warn!`. Si querés
//! capturar la salida del thread (por ejemplo para test), usá
//! [`spawn_with_handle`] que devuelve un `JoinHandle`.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

pub mod discovery;
pub use discovery::{
    await_provider, await_provider_blocking, build_consumer_card, list_matches,
    list_matches_blocking, list_sessions, list_sessions_blocking, ConsumerError,
};

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use brahman_card::{ulid::Ulid, Card, WitInterface};
use brahman_handshake::{client::Client, transport};
use tokio::task::AbortHandle;
use tracing::{info, warn};

/// Período entre pings al Init.
pub const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(30);

/// Configuración del sidecar.
#[derive(Debug, Clone)]
pub struct SidecarConfig {
    /// Card que se presenta al Init.
    pub card: Card,
    /// WIT interface opcional. Si es `Some`, el módulo se registra como
    /// "consciente" (`ResolvedCard::from_conscious`).
    pub wit: Option<WitInterface>,
    /// Período entre pings.
    pub ping_interval: Duration,
}

impl SidecarConfig {
    /// Configuración agnóstica con defaults razonables (sin WIT, ping 30s).
    pub fn new(card: Card) -> Self {
        Self {
            card,
            wit: None,
            ping_interval: DEFAULT_PING_INTERVAL,
        }
    }

    /// Configuración consciente con WIT extraída.
    pub fn with_wit(mut self, wit: WitInterface) -> Self {
        self.wit = Some(wit);
        self
    }
}

/// Spawn fire-and-forget agnóstico. Para módulos conscientes usá
/// [`spawn_conscious`] o [`spawn_with_handle`] con un `SidecarConfig`
/// personalizado.
pub fn spawn(card: Card) {
    if let Err(e) = spawn_with_handle(SidecarConfig::new(card)) {
        warn!(error = %e, "no se pudo spawnear el sidecar brahman");
    }
}

/// Spawn fire-and-forget con WIT. Idéntico a [`spawn`] pero el módulo se
/// registra como consciente en el broker.
pub fn spawn_conscious(card: Card, wit: WitInterface) {
    if let Err(e) = spawn_with_handle(SidecarConfig::new(card).with_wit(wit)) {
        warn!(error = %e, "no se pudo spawnear el sidecar brahman");
    }
}

/// Spawn devolviendo el `JoinHandle` para tests o cleanup explícito.
pub fn spawn_with_handle(config: SidecarConfig) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("brahman-sidecar".into())
        .spawn(move || run_thread(config))
}

// =====================================================================
// SidecarPool — un solo runtime tokio compartido por N sesiones
// =====================================================================

/// Pool consolidado: un único thread con un runtime tokio
/// `current_thread` que hostea N tasks de sidecar simultáneas.
///
/// Para módulos con muchas sesiones (p. ej. `chasqui daemon` publicando
/// 50+ Mónadas), evita el costo de tener un thread+runtime por cada
/// sesión.
///
/// **API**:
/// - `SidecarPool::new()` crea el pool (spawn del thread runtime).
/// - `pool.spawn(card)` añade una sesión sin WIT.
/// - `pool.spawn_conscious(card, wit)` añade una sesión con WIT.
/// - `pool.spawn_with_config(config)` para configuración custom.
///
/// El pool se mantiene vivo mientras exista. Si el `SidecarPool`
/// se dropea, el thread interno termina y todas las sesiones cierran.
pub struct SidecarPool {
    handle: tokio::runtime::Handle,
    /// Sesiones vivas indexadas por `Card.id`. Permite que un nuevo
    /// `spawn` con el mismo id aborte la sesión previa — útil cuando
    /// un módulo (p. ej. `chasqui daemon`) re-publica una Mónada cuya
    /// composición cambió.
    sessions: Arc<Mutex<HashMap<Ulid, AbortHandle>>>,
    _thread: JoinHandle<()>,
}

impl SidecarPool {
    /// Crea un pool nuevo. Bloquea hasta que el runtime esté listo.
    pub fn new() -> std::io::Result<Self> {
        let (handle_tx, handle_rx) = mpsc::sync_channel::<tokio::runtime::Handle>(0);
        let thread = std::thread::Builder::new()
            .name("brahman-sidecar-pool".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_io()
                    .enable_time()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        warn!(error = %e, "tokio runtime falló — pool muerto");
                        return;
                    }
                };
                if handle_tx.send(rt.handle().clone()).is_err() {
                    return;
                }
                // Mantenemos el runtime vivo mientras existan tasks.
                rt.block_on(std::future::pending::<()>());
            })?;
        let handle = handle_rx
            .recv()
            .map_err(|_| std::io::Error::other("pool runtime no respondió"))?;
        Ok(Self {
            handle,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            _thread: thread,
        })
    }

    /// Añade una sesión agnóstica al pool (sin WIT).
    pub fn spawn(&self, card: Card) {
        self.spawn_with_config(SidecarConfig::new(card));
    }

    /// Añade una sesión consciente (con WIT) al pool.
    pub fn spawn_conscious(&self, card: Card, wit: WitInterface) {
        self.spawn_with_config(SidecarConfig::new(card).with_wit(wit));
    }

    /// Añade una sesión con configuración custom.
    ///
    /// Si ya existía una sesión para el mismo `Card.id`, la previa
    /// se aborta antes de spawnear la nueva. Esto hace `spawn`
    /// idempotente respecto al id: re-publicar una Mónada cuya
    /// composición cambió "refresca" la sesión en el broker.
    pub fn spawn_with_config(&self, config: SidecarConfig) {
        let card_id = config.card.id;
        let join = self.handle.spawn(run_client(config));
        let abort = join.abort_handle();
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(prev) = sessions.insert(card_id, abort) {
                prev.abort();
            }
        }
    }

    /// Cierra explícitamente la sesión asociada a `card_id`. No-op si
    /// no había sesión registrada.
    pub fn drop_session(&self, card_id: Ulid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(abort) = sessions.remove(&card_id) {
                abort.abort();
            }
        }
    }

    /// Cantidad actual de sesiones vivas (estimada — puede haber
    /// drift transitorio entre abort y limpieza).
    pub fn live_sessions(&self) -> usize {
        self.sessions.lock().map(|s| s.len()).unwrap_or(0)
    }
}

impl Default for SidecarPool {
    fn default() -> Self {
        Self::new().expect("falló SidecarPool::new")
    }
}

fn run_thread(config: SidecarConfig) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!(error = %e, "tokio runtime falló");
            return;
        }
    };
    rt.block_on(run_client(config));
}

/// Bucle async del sidecar. Público para que `SidecarPool` lo use vía
/// `handle.spawn(run_client(...))` desde código externo al runtime.
pub async fn run_client(config: SidecarConfig) {
    let path = transport::default_socket_path();
    let conscious = config.wit.is_some();
    let mut client = match Client::connect_with(&path, config.card, config.wit).await {
        Ok(c) => {
            info!(
                target: "brahman_sidecar",
                session = %c.session(),
                init_attached = c.server_info().init_attached,
                server = %c.server_info().server_version,
                conscious,
                "attached"
            );
            c
        }
        Err(e) => {
            warn!(
                target: "brahman_sidecar",
                error = %e,
                socket = %path.display(),
                "no conectado"
            );
            return;
        }
    };

    loop {
        tokio::time::sleep(config.ping_interval).await;
        if let Err(e) = client.ping().await {
            warn!(target: "brahman_sidecar", error = %e, "ping falló — terminando sidecar");
            return;
        }
    }
}
