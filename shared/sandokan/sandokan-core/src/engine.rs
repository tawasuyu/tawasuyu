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

    /// Reescribe `cpu.weight` de un cgroup **ya existente** (reweight en
    /// caliente, sin reencarnar). `cgroup_path` se direcciona igual que
    /// `CgroupSpec.path` (relativo → bajo el cgroup actual). Pensado para
    /// deprioritizar/priorizar todo un subárbol — el slice de un contexto
    /// `pacha` — de una sola escritura (el peso es jerárquico). El default
    /// rebota `Unsupported` para engines que no tocan cgroups (remoto/mock).
    async fn set_cpu_weight(&self, cgroup_path: String, weight: u32) -> Result<(), EngineError> {
        let _ = (cgroup_path, weight);
        Err(EngineError::Unsupported("set_cpu_weight".into()))
    }

    /// Congela (`true`) o descongela (`false`) un cgroup vía el freezer v2
    /// (`cgroup.freeze`). Jerárquico: gobierna todo el subárbol → SIGSTOP de
    /// grupo conservando la RAM. Default `Unsupported`.
    async fn freeze(&self, cgroup_path: String, frozen: bool) -> Result<(), EngineError> {
        let _ = (cgroup_path, frozen);
        Err(EngineError::Unsupported("freeze".into()))
    }

    /// Fija `memory.max` (tope **duro**: el kernel OOM-matea el grupo al
    /// cruzarlo) de un cgroup ya existente. `bytes == u64::MAX` = sin tope.
    /// Jerárquico como el resto de los knobs de cgroup. Default `Unsupported`.
    async fn set_memory_max(&self, cgroup_path: String, bytes: u64) -> Result<(), EngineError> {
        let _ = (cgroup_path, bytes);
        Err(EngineError::Unsupported("set_memory_max".into()))
    }

    /// Fija `memory.high` (tope **blando**: el kernel estrangula y recupera
    /// memoria sin matar) de un cgroup ya existente. `bytes == u64::MAX` = sin
    /// tope. Ideal para deprioritizar un slice sin arriesgar OOM. Default
    /// `Unsupported`.
    async fn set_memory_high(&self, cgroup_path: String, bytes: u64) -> Result<(), EngineError> {
        let _ = (cgroup_path, bytes);
        Err(EngineError::Unsupported("set_memory_high".into()))
    }

    /// Fija `io.weight` (peso de I/O por defecto, `1..=10000`, 100 = neutro) de
    /// un cgroup ya existente — el análogo de `set_cpu_weight` para disco.
    /// Jerárquico. Default `Unsupported`.
    async fn set_io_weight(&self, cgroup_path: String, weight: u32) -> Result<(), EngineError> {
        let _ = (cgroup_path, weight);
        Err(EngineError::Unsupported("set_io_weight".into()))
    }

    /// Reinicia una unidad: la detiene (con `grace`, igual que `stop`) y la
    /// vuelve a encarnar con la **misma Card** (mismo `card_id`). Para
    /// `LocalEngine` es stop→run del intent guardado; para un supervisor
    /// (arje-zero) el reinicio puede delegarse en su `RestartPolicy`. Default
    /// `Unsupported` para engines que no retienen el intent (remoto/mock) o
    /// cuyo reinicio vive en otra capa. Pensado para que una regla de métrica o
    /// del cerebro recicle una unidad colgada sin matarla a mano.
    async fn restart(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        let _ = (card_id, grace);
        Err(EngineError::Unsupported("restart".into()))
    }
}
