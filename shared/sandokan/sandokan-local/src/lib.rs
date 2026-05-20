//! sandokan-local — `LocalEngine`, orquestación in-process.
//!
//! Primera implementación del trait [`Engine`]: encarna Cards en el
//! mismo host vía `arje-incarnate`, mantiene un registro de las
//! entidades activas y hace reaping perezoso (waitpid WNOHANG en cada
//! consulta) para detectar salidas sin un task de fondo dedicado.
//!
//! `DaemonEngine` y `RemoteEngine` (transportes) se construirán sobre
//! este mismo contrato en crates separados.

mod proc;

use arje_incarnate::{Incarnator, IncarnatorConfig};
use async_trait::async_trait;
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use ulid::Ulid;

/// Una entidad encarnada y su estado conocido.
struct Entity {
    handle: ExecHandle,
    pid: i32,
    state: LifecycleState,
}

/// Orquestador in-process. Encarna Cards localmente y trackea su lifecycle.
pub struct LocalEngine {
    base_cfg: IncarnatorConfig,
    registry: Mutex<HashMap<Ulid, Entity>>,
}

impl LocalEngine {
    /// Crea un engine con configuración de incarnación por defecto.
    pub fn new() -> Self {
        Self::with_config(IncarnatorConfig::default())
    }

    /// Crea un engine con una `IncarnatorConfig` explícita (bus socket,
    /// env extra, strict_caps).
    pub fn with_config(cfg: IncarnatorConfig) -> Self {
        Self {
            base_cfg: cfg,
            registry: Mutex::new(HashMap::new()),
        }
    }

    /// Marca el estado de una entidad (best-effort; ignora lock envenenado).
    fn mark(&self, card_id: Ulid, state: LifecycleState) {
        if let Ok(mut reg) = self.registry.lock() {
            if let Some(ent) = reg.get_mut(&card_id) {
                ent.state = state;
            }
        }
    }
}

impl Default for LocalEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Reaping no-bloqueante de un pid. `Some(estado terminal)` si la entidad
/// transicionó; `None` si sigue viva.
fn reap(pid: i32) -> Option<LifecycleState> {
    match waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG)) {
        Ok(WaitStatus::Exited(_, code)) => Some(LifecycleState::Exited { code }),
        Ok(WaitStatus::Signaled(_, _, _)) => Some(LifecycleState::Killed),
        Ok(WaitStatus::StillAlive) => None,
        // Stopped / Continued / PtraceEvent: la entidad sigue presente.
        Ok(_) => None,
        Err(nix::errno::Errno::ECHILD) => {
            // No es (ya) hijo reapable. Si procfs no lo tiene, terminó.
            if proc::proc_exists(pid) {
                None
            } else {
                Some(LifecycleState::Exited { code: -1 })
            }
        }
        Err(_) => None,
    }
}

#[async_trait]
impl Engine for LocalEngine {
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError> {
        let card_id = intent.card_id();
        let label = intent.card.label.clone();

        // El env del contexto se mergea sobre el del engine base.
        let mut cfg = self.base_cfg.clone();
        cfg.extra_env.extend(intent.context.env.clone());

        // NOTA v1: `IsolationLevel` es advisory. None/Standard encarnan
        // según `Card.soma`; Sealed (rootfs aislado vía pivot_root +
        // OverlayFS) queda reservado para cuando el Intent transporte una
        // spec de rootfs — `arje-incarnate` ya expone los builders.
        let incarnator = Incarnator::new(cfg);
        let outcome = incarnator
            .incarnate(&intent.card)
            .map_err(|e| EngineError::Incarnate(e.to_string()))?;

        let handle = ExecHandle {
            card_id,
            label,
            started_at: SystemTime::now(),
        };
        let mut reg = self.registry.lock().expect("registry lock");
        reg.insert(
            card_id,
            Entity {
                handle: handle.clone(),
                pid: outcome.pid.as_raw(),
                state: LifecycleState::Running,
            },
        );
        Ok(handle)
    }

    async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        let pid = {
            let reg = self.registry.lock().expect("registry lock");
            reg.get(&card_id)
                .map(|e| e.pid)
                .ok_or(EngineError::NotFound(card_id))?
        };
        let npid = Pid::from_raw(pid);

        // SIGTERM + período de gracia: damos chance a un cierre ordenado.
        if grace > Duration::ZERO {
            let _ = kill(npid, Signal::SIGTERM);
            let deadline = Instant::now() + grace;
            loop {
                if reap(pid).is_some() {
                    self.mark(card_id, LifecycleState::Killed);
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }

        // SIGKILL + reap bloqueante de lo que haya quedado.
        let _ = kill(npid, Signal::SIGKILL);
        let _ = waitpid(npid, None);
        self.mark(card_id, LifecycleState::Killed);
        Ok(())
    }

    async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
        let mut reg = self.registry.lock().expect("registry lock");
        let mut out = Vec::new();
        for ent in reg.values_mut() {
            if !ent.state.is_terminal() {
                if let Some(new_state) = reap(ent.pid) {
                    ent.state = new_state;
                }
            }
            if !ent.state.is_terminal() {
                out.push(ent.handle.clone());
            }
        }
        Ok(out)
    }

    async fn status(&self, card_id: Ulid) -> Result<LifecycleState, EngineError> {
        let mut reg = self.registry.lock().expect("registry lock");
        let ent = reg
            .get_mut(&card_id)
            .ok_or(EngineError::NotFound(card_id))?;
        if !ent.state.is_terminal() {
            if let Some(new_state) = reap(ent.pid) {
                ent.state = new_state;
            }
        }
        Ok(ent.state.clone())
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        let pid = {
            let reg = self.registry.lock().expect("registry lock");
            reg.get(&card_id)
                .map(|e| e.pid)
                .ok_or(EngineError::NotFound(card_id))?
        };
        Ok(TelemetryFrame {
            card_id,
            at: SystemTime::now(),
            mem_bytes: proc::read_mem_bytes(pid),
            nproc: proc::read_thread_count(pid),
            // v1: CPU% requiere dos samples espaciados — pendiente.
            cpu_pct: 0.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_engine_lists_nothing() {
        let e = LocalEngine::new();
        assert!(e.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unknown_card_is_not_found() {
        let e = LocalEngine::new();
        let id = Ulid::new();
        assert!(matches!(e.status(id).await, Err(EngineError::NotFound(_))));
        assert!(matches!(
            e.stop(id, Duration::ZERO).await,
            Err(EngineError::NotFound(_))
        ));
        assert!(matches!(
            e.telemetry(id).await,
            Err(EngineError::NotFound(_))
        ));
    }
}
