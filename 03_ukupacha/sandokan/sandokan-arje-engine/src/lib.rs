//! `sandokan-arje-engine` — el `Engine` del host hablado sobre `arje-bus`.
//!
//! Implementa [`sandokan_core::Engine`] traduciendo cada método al protocolo
//! de `arje-bus` que atiende arje-zero (PID 1). Es el **transporte único de
//! control en Linux** (SDD §5 Fase 2-3): en vez de que sandokan tenga su
//! propio socket (`sandokan-daemon`) en paralelo al del init, el `Engine`
//! delegado viaja por el bus que el init ya sirve y que `arje-systemd1-compat`
//! ya usa.
//!
//! ## Compromisos de mapeo (documentados, no accidentales)
//!
//! El contrato `Engine` y el protocolo del bus no son isomorfos. Donde el bus
//! no da un dato, el bridge usa un valor honesto y lo deja anotado:
//!
//! | Engine | arje-bus | compromiso |
//! |---|---|---|
//! | `run` | `RunCard{card: WireCard}` | la Card viaja **entera por el wire**; no necesita existir en el store. arje la encarna con el bridge como `requester` (exige `Capability::Spawn`, hereda sus caps — sin escalada). |
//! | `stop` | `KillEnte{SIGTERM\|SIGKILL}` | `grace==0` → SIGKILL; si no, SIGTERM (el bus no escala SIGTERM→SIGKILL). |
//! | `list` | `ListEntes` | `started_at` no lo da el bus → `now()` aproximado. |
//! | `status` | `EnteStatus` | sólo `Running`/`Gone`; `Gone` → `NotFound` (arje-zero no retiene exit codes). |
//! | `telemetry` | `EnteTelemetry` | `cpu_pct = 0.0` (el bus da RSS + hilos, no CPU). |
//! | `set_cpu_weight` | `SetCpuWeight{cgroup_path,weight}` | arje-zero escribe `cpu.weight` del slice (SDD §8 capa 1). |
//! | `freeze` | `Freeze{cgroup_path,frozen}` | arje-zero escribe `cgroup.freeze` (freezer v2). |
//!
//! `run`/`stop`/`set_cpu_weight`/`freeze` requieren que el bridge corra como un
//! Ente autenticado (igual que `arje-systemd1-compat`); `list`/`status`/
//! `telemetry` son anónimos.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use arje_bus::{BusClient, BusRequest, BusResponse, EnteInfo, Liveness, ResourceSample};
use async_trait::async_trait;
use sandokan_core::{Engine, EngineError, ExecHandle, Intent, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
use ulid::Ulid;

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;

/// `Engine` que delega en arje-zero por su socket de bus.
pub struct ArjeEngine {
    sock: PathBuf,
}

impl ArjeEngine {
    /// Bridge contra el bus en `sock`.
    pub fn new(sock: impl Into<PathBuf>) -> Self {
        Self { sock: sock.into() }
    }

    /// Bridge contra el bus apuntado por `$ENTE_BUS_SOCK`.
    pub fn from_env() -> Result<Self, EngineError> {
        let p = std::env::var(arje_bus::ENV_BUS_SOCK)
            .map_err(|_| EngineError::Transport(format!("{} no definido", arje_bus::ENV_BUS_SOCK)))?;
        Ok(Self::new(p))
    }

    /// ¿Hay un init escuchando en el bus? Sonda barata (un `ListEntes`).
    /// La usa `sandokan::auto` para preferir el Engine de sistema.
    pub async fn is_reachable(&self) -> bool {
        self.list().await.is_ok()
    }

    /// Una request-response contra el bus. Un fallo de conexión/transporte es
    /// `EngineError::Transport`; la semántica de la respuesta la mapea el caller.
    async fn call(&self, req: BusRequest) -> Result<BusResponse, EngineError> {
        let mut client = BusClient::connect(&self.sock)
            .await
            .map_err(|e| EngineError::Transport(e.to_string()))?;
        client
            .call(req)
            .await
            .map_err(|e| EngineError::Transport(e.to_string()))
    }
}

// ---- Mapeos puros (testables sin bus) ----

/// `EnteInfo` del bus → `ExecHandle` del contrato. `started_at` no viaja por el
/// bus; el caller pasa un instante de referencia (típicamente `now`).
fn ente_to_handle(e: EnteInfo, started_at: SystemTime) -> ExecHandle {
    ExecHandle {
        card_id: e.id,
        label: e.label,
        started_at,
    }
}

/// `Liveness` → `LifecycleState`. `Gone` es `None` (el caller lo vuelve
/// `NotFound`): arje-zero no guarda exit codes, así que un Ente ausente no es
/// `Exited{code}` ni `Killed`, simplemente ya no está.
fn liveness_to_state(l: Liveness) -> Option<LifecycleState> {
    match l {
        Liveness::Running { .. } => Some(LifecycleState::Running),
        Liveness::Gone => None,
        Liveness::Parked { reason } => Some(LifecycleState::Parked { reason }),
    }
}

/// `ResourceSample` del bus → `TelemetryFrame`. `cpu_pct` queda en 0.0 (el bus
/// no muestrea CPU).
fn sample_to_frame(card_id: Ulid, at: SystemTime, s: ResourceSample) -> TelemetryFrame {
    TelemetryFrame {
        card_id,
        at,
        mem_bytes: s.mem_bytes,
        nproc: s.nproc,
        cpu_pct: 0.0,
        restarts: s.restarts,
    }
}

#[async_trait]
impl Engine for ArjeEngine {
    async fn run(&self, intent: Intent) -> Result<ExecHandle, EngineError> {
        // La Card viaja entera por el wire (RunCard): no necesita existir en el
        // store. arje la encarna con el bridge como requester (exige Spawn).
        let label = intent.card.label.clone();
        let card_id = intent.card_id();
        let card = card_core::WireCard::from(intent.card);
        match self.call(BusRequest::RunCard { card }).await? {
            BusResponse::Ok => Ok(ExecHandle {
                card_id,
                label,
                started_at: SystemTime::now(),
            }),
            BusResponse::Error(e) => Err(EngineError::Incarnate(e)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a RunCard: {other:?}"
            ))),
        }
    }

    async fn stop(&self, card_id: Ulid, grace: Duration) -> Result<(), EngineError> {
        let signal = if grace.is_zero() { SIGKILL } else { SIGTERM };
        match self
            .call(BusRequest::KillEnte {
                target: card_id,
                signal,
            })
            .await?
        {
            BusResponse::Ok => Ok(()),
            // arje-zero responde Error tanto si el Ente no existe como si no es
            // matable (Virtual/Wasm); conservamos su detalle.
            BusResponse::Error(e) => Err(EngineError::Transport(e)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a KillEnte: {other:?}"
            ))),
        }
    }

    async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
        match self.call(BusRequest::ListEntes).await? {
            BusResponse::Entes(entes) => {
                let now = SystemTime::now();
                Ok(entes.into_iter().map(|e| ente_to_handle(e, now)).collect())
            }
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a ListEntes: {other:?}"
            ))),
        }
    }

    async fn status(&self, card_id: Ulid) -> Result<LifecycleState, EngineError> {
        match self.call(BusRequest::EnteStatus { target: card_id }).await? {
            BusResponse::Status(l) => liveness_to_state(l).ok_or(EngineError::NotFound(card_id)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a EnteStatus: {other:?}"
            ))),
        }
    }

    async fn telemetry(&self, card_id: Ulid) -> Result<TelemetryFrame, EngineError> {
        match self
            .call(BusRequest::EnteTelemetry { target: card_id })
            .await?
        {
            BusResponse::Telemetry(s) => Ok(sample_to_frame(card_id, SystemTime::now(), s)),
            // arje-zero responde Error cuando el Ente no vive o no tiene proceso.
            BusResponse::Error(_) => Err(EngineError::NotFound(card_id)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a EnteTelemetry: {other:?}"
            ))),
        }
    }

    async fn set_cpu_weight(&self, cgroup_path: String, weight: u32) -> Result<(), EngineError> {
        // SDD §8 capa 1: el verbo de prioridad viaja por el bus hasta arje-zero,
        // que escribe `cpu.weight` del slice. Antes caía en el default
        // `Unsupported` —declarado en el contrato, pero no cableado—.
        match self
            .call(BusRequest::SetCpuWeight { cgroup_path, weight })
            .await?
        {
            BusResponse::Ok => Ok(()),
            BusResponse::Error(e) => Err(EngineError::Cgroup(e)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a SetCpuWeight: {other:?}"
            ))),
        }
    }

    async fn freeze(&self, cgroup_path: String, frozen: bool) -> Result<(), EngineError> {
        match self.call(BusRequest::Freeze { cgroup_path, frozen }).await? {
            BusResponse::Ok => Ok(()),
            BusResponse::Error(e) => Err(EngineError::Cgroup(e)),
            other => Err(EngineError::Transport(format!(
                "respuesta inesperada a Freeze: {other:?}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveness_running_es_running_gone_es_none() {
        assert!(matches!(
            liveness_to_state(Liveness::Running { pid: Some(7) }),
            Some(LifecycleState::Running)
        ));
        assert!(liveness_to_state(Liveness::Gone).is_none());
        // Parked viaja con su motivo → LifecycleState::Parked (visible en el monitor).
        assert!(matches!(
            liveness_to_state(Liveness::Parked { reason: "falta el piso".into() }),
            Some(LifecycleState::Parked { .. })
        ));
    }

    #[test]
    fn sample_a_frame_preserva_recursos_y_cpu_cero() {
        let id = Ulid::new();
        let f = sample_to_frame(
            id,
            SystemTime::UNIX_EPOCH,
            ResourceSample {
                mem_bytes: 4096,
                nproc: 3,
                restarts: 2,
            },
        );
        assert_eq!(f.card_id, id);
        assert_eq!(f.mem_bytes, 4096);
        assert_eq!(f.nproc, 3);
        assert_eq!(f.cpu_pct, 0.0); // el bus no muestrea CPU
        assert_eq!(f.restarts, 2);
    }

    #[test]
    fn ente_a_handle_copia_identidad() {
        let id = Ulid::new();
        let h = ente_to_handle(
            EnteInfo {
                id,
                label: "demo".into(),
                provides: vec![],
                pid: Some(42),
            },
            SystemTime::UNIX_EPOCH,
        );
        assert_eq!(h.card_id, id);
        assert_eq!(h.label, "demo");
    }
}
