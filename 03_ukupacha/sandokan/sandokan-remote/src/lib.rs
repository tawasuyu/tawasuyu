//! `sandokan-remote` — `RemoteEngine`: orquesta en un host remoto.
//!
//! Misma técnica que `DaemonEngine` pero el transporte es un canal SSH
//! `direct-streamlocal` hacia el `sandokan.sock` del host remoto. El wire
//! es idéntico (postcard length-prefixed) — sólo cambia el túnel, así
//! que el código de protocolo se reusa tal cual.
//!
//! Requiere que el host remoto corra `sandokan daemon` escuchando en
//! `remote_socket`.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use ssh::{SshConfig, SshSession};
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_daemon::{read_frame, write_frame, DaemonRequest, DaemonResponse};
use sandokan_lifecycle::LifecycleState;
use std::time::Duration;
use ulid::Ulid;

/// Engine que delega a un daemon sandokan en un host remoto, tunelando
/// el wire sobre SSH. La sesión SSH maestra se mantiene; cada operación
/// abre un canal `direct-streamlocal` nuevo (multiplexado, barato).
pub struct RemoteEngine {
    session: SshSession,
    remote_socket: String,
}

impl RemoteEngine {
    /// Conecta por SSH al host y prepara el túnel al socket del daemon.
    pub async fn connect(
        ssh: &SshConfig,
        remote_socket: impl Into<String>,
    ) -> Result<Self, EngineError> {
        let session = SshSession::connect(ssh)
            .await
            .map_err(|e| EngineError::Transport(format!("ssh connect: {e}")))?;
        Ok(Self { session, remote_socket: remote_socket.into() })
    }

    /// Construye un `RemoteEngine` sobre una `SshSession` ya establecida
    /// (permite compartir la conexión maestra con otros consumidores).
    pub fn with_session(session: SshSession, remote_socket: impl Into<String>) -> Self {
        Self { session, remote_socket: remote_socket.into() }
    }

    async fn roundtrip(&self, req: DaemonRequest) -> Result<DaemonResponse, EngineError> {
        let mut stream = self
            .session
            .forward_unix(&self.remote_socket)
            .await
            .map_err(|e| EngineError::Transport(format!("ssh forward: {e}")))?;
        write_frame(&mut stream, &req)
            .await
            .map_err(|e| EngineError::Transport(format!("send: {e}")))?;
        read_frame::<_, DaemonResponse>(&mut stream)
            .await
            .map_err(|e| EngineError::Transport(format!("recv: {e}")))
    }
}

/// Un response que no corresponde al request enviado.
fn mismatch() -> EngineError {
    EngineError::Transport("respuesta remota no coincide con el request".into())
}

#[async_trait]
impl Engine for RemoteEngine {
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError> {
        match self.roundtrip(DaemonRequest::Run(intent)).await? {
            DaemonResponse::Ran(h) => Ok(h),
            DaemonResponse::Err(e) => Err(e),
            _ => Err(mismatch()),
        }
    }

    async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        let req = DaemonRequest::Stop {
            card_id,
            grace_ms: grace.as_millis() as u64,
        };
        match self.roundtrip(req).await? {
            DaemonResponse::Stopped => Ok(()),
            DaemonResponse::Err(e) => Err(e),
            _ => Err(mismatch()),
        }
    }

    async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
        match self.roundtrip(DaemonRequest::List).await? {
            DaemonResponse::Listed(v) => Ok(v),
            DaemonResponse::Err(e) => Err(e),
            _ => Err(mismatch()),
        }
    }

    async fn status(&self, card_id: Ulid) -> Result<LifecycleState, EngineError> {
        match self.roundtrip(DaemonRequest::Status { card_id }).await? {
            DaemonResponse::Status(s) => Ok(s),
            DaemonResponse::Err(e) => Err(e),
            _ => Err(mismatch()),
        }
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        match self.roundtrip(DaemonRequest::Telemetry { card_id }).await? {
            DaemonResponse::Telemetry(t) => Ok(t),
            DaemonResponse::Err(e) => Err(e),
            _ => Err(mismatch()),
        }
    }
}
