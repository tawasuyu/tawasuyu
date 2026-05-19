//! Servidor admin: emite un `StatusSnapshot` JSON por conexión y cierra.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use brahman_broker::Broker;
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;
use tracing::warn;

use crate::snapshot::StatusSnapshot;

/// Configuración del servidor admin.
#[derive(Debug, Clone, Default)]
pub struct AdminConfig {
    /// `true` si el Init está atado al servidor que aloja este admin.
    pub init_attached: bool,
    /// Contexto operativo del broker, espejado en el snapshot.
    pub current_context: Option<String>,
}

/// Servidor admin escuchando en un Unix socket.
pub struct AdminServer {
    listener: UnixListener,
    socket_path: PathBuf,
    broker: Arc<Mutex<Broker>>,
    config: AdminConfig,
}

impl AdminServer {
    /// Crea el listener. Si `path` existe, lo elimina (asume socket stale).
    pub fn bind(
        path: impl Into<PathBuf>,
        broker: Arc<Mutex<Broker>>,
        config: AdminConfig,
    ) -> std::io::Result<Self> {
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
            broker,
            config,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Loop de aceptación: cada conexión recibe un snapshot y se cierra.
    pub async fn run(self) -> std::io::Result<()> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            let broker = self.broker.clone();
            let config = self.config.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, broker, config).await {
                    warn!(error = %e, "admin conn falló");
                }
            });
        }
    }
}

impl Drop for AdminServer {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                warn!(path = %self.socket_path.display(), error = %e, "no se pudo limpiar admin socket");
            }
        }
    }
}

async fn handle_conn(
    mut stream: UnixStream,
    broker: Arc<Mutex<Broker>>,
    config: AdminConfig,
) -> std::io::Result<()> {
    let snapshot = build_snapshot(&broker, &config).await;
    let mut json = serde_json::to_string(&snapshot)?;
    json.push('\n');
    stream.write_all(json.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn build_snapshot(broker: &Arc<Mutex<Broker>>, config: &AdminConfig) -> StatusSnapshot {
    let b = broker.lock().await;
    let sessions: Vec<_> = b.cards().cloned().collect();
    let matches = b.all_matches();
    StatusSnapshot {
        server_version: crate::ADMIN_VERSION.to_string(),
        protocol_version: brahman_card::PROTOCOL_VERSION.to_string(),
        init_attached: config.init_attached,
        current_context: config.current_context.clone(),
        sessions,
        matches,
    }
}
