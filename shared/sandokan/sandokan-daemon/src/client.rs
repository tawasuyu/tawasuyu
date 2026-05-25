//! `DaemonEngine` — cliente que implementa `Engine` sobre el wire.

use crate::protocol::{read_frame, write_frame, DaemonRequest, DaemonResponse};
use async_trait::async_trait;
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;
use ulid::Ulid;

/// Engine que delega cada operación a un daemon vía Unix socket.
///
/// Stateless: cada llamada abre una conexión, hace un round-trip y la
/// cierra. Simple y robusto; si el daemon no está, las llamadas fallan
/// con `EngineError::Transport`.
pub struct DaemonEngine {
    socket_path: PathBuf,
}

impl DaemonEngine {
    /// Crea un cliente apuntando al socket dado.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self { socket_path: socket_path.into() }
    }

    /// `true` si el socket existe y acepta conexiones ahora mismo.
    pub async fn is_reachable(&self) -> bool {
        UnixStream::connect(&self.socket_path).await.is_ok()
    }

    /// Un round-trip: conecta, envía el request, lee el response.
    async fn roundtrip(&self, req: DaemonRequest) -> Result<DaemonResponse, EngineError> {
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| EngineError::Transport(format!("connect: {e}")))?;
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
    EngineError::Transport("respuesta del daemon no coincide con el request".into())
}

#[async_trait]
impl Engine for DaemonEngine {
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
