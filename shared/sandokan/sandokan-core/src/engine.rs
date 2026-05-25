//! El trait `Engine` — contrato uniforme del orquestador.

use crate::error::EngineError;
use crate::event::TelemetryFrame;
use crate::intent::{ExecHandle, Intent};
use async_trait::async_trait;
use sandokan_lifecycle::LifecycleState;
use std::time::Duration;
use ulid::Ulid;

/// El orquestador. Tres implementaciones lo cumplen de forma intercambiable:
///
/// - `LocalEngine` — encarna in-process (`arje-incarnate` + `arje-brain-rules`).
/// - `DaemonEngine` — delega a otro proceso vía Unix socket (postcard).
/// - `RemoteEngine` — delega a otro host vía `brahman-ssh-multiplex`.
///
/// Un helper `Engine::auto()` (en `sandokan-local`) prueba si hay un
/// daemon escuchando y elige `DaemonEngine`; si no, `LocalEngine`.
///
/// El contrato es poll-based (sin streams) para que las tres impls lo
/// cumplan uniformemente sin complejidad de `Stream` sobre trait objects.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Encarna una intención. Devuelve un handle a la entidad corriendo.
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError>;

    /// Detiene una entidad con período de gracia (SIGTERM → espera →
    /// SIGKILL). `grace == 0` = kill inmediato.
    async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError>;

    /// Lista las entidades actualmente activas.
    async fn list(&self) -> Result<Vec<ExecHandle>, EngineError>;

    /// Estado actual de una entidad.
    async fn status(&self, card_id: Ulid) -> Result<LifecycleState, EngineError>;

    /// Telemetría puntual de una entidad.
    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError>;
}
