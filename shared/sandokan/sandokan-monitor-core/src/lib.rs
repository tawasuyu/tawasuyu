//! `sandokan-monitor-core` — la **cara de sólo-lectura** del plano de control.
//!
//! El process monitor de tawasuyu observa las unidades vivas por el **mismo
//! contrato** que las controla: [`sandokan_core::Engine`]. No mira `/proc` ni
//! el card store crudo — eso volvería a tener dos fuentes de verdad (la del
//! control y la de la observación), justo el duplicado que el SDD elimina.
//! Ver `shared/sandokan/SDD.md` §6.
//!
//! [`observe`] toma cualquier `&dyn Engine` (in-process `LocalEngine`, remoto
//! `DaemonEngine`/`RemoteEngine` — da igual el transporte) y produce un
//! [`MonitorSnapshot`]: por cada unidad, su estado de ciclo de vida y una
//! telemetría puntual. Un frontend (p.ej. `arje-card-llimphi`) lo pinta.

#![forbid(unsafe_code)]

use sandokan_core::{Engine, EngineError, TelemetryFrame};
use sandokan_lifecycle::LifecycleState;
use ulid::Ulid;

/// Observación de una unidad viva: identidad + estado + telemetría puntual.
#[derive(Debug, Clone)]
pub struct UnitObservation {
    pub card_id: Ulid,
    pub label: String,
    /// Estado de ciclo de vida según el Engine (`status`).
    pub state: LifecycleState,
    /// Telemetría puntual (mem/cpu/threads). `None` si el Engine no la pudo
    /// dar en este instante (la unidad pudo salir entre `list` y `telemetry`).
    pub telemetry: Option<TelemetryFrame>,
    /// Restarts acumulados que el supervisor aplicó (del `TelemetryFrame`).
    /// `0` si no hay telemetría o el Engine no los trackea (SDD §6 Fase 2).
    pub restarts: u32,
}

/// Snapshot del plano de control en un instante: las unidades observadas.
#[derive(Debug, Clone, Default)]
pub struct MonitorSnapshot {
    pub units: Vec<UnitObservation>,
}

impl MonitorSnapshot {
    pub fn len(&self) -> usize {
        self.units.len()
    }
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
    /// Cuántas unidades están corriendo ahora mismo.
    pub fn running(&self) -> usize {
        self.units
            .iter()
            .filter(|u| matches!(u.state, LifecycleState::Running))
            .count()
    }
}

/// Observa un Engine y arma el snapshot. `list()` es la única llamada que, si
/// falla, aborta la observación (sin lista no hay nada que mirar). El
/// `status`/`telemetry` por unidad degradan: una unidad que desaparece entre
/// `list` y la consulta no tumba el snapshot (`status` → `Failed`,
/// `telemetry` → `None`).
pub async fn observe(engine: &dyn Engine) -> Result<MonitorSnapshot, EngineError> {
    let handles = engine.list().await?;
    let mut units = Vec::with_capacity(handles.len());
    for h in handles {
        let state = engine.status(h.card_id).await.unwrap_or_else(|e| {
            LifecycleState::Failed {
                reason: format!("status indisponible: {e}"),
            }
        });
        let telemetry = engine.telemetry(h.card_id).await.ok();
        let restarts = telemetry.as_ref().map(|t| t.restarts).unwrap_or(0);
        units.push(UnitObservation {
            card_id: h.card_id,
            label: h.label,
            state,
            telemetry,
            restarts,
        });
    }
    Ok(MonitorSnapshot { units })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sandokan_core::{ExecHandle, Intent};
    use std::time::{Duration, SystemTime};

    /// Engine mock: una lista fija de unidades, estados y telemetría
    /// inyectables, y un flag para fallar `list` (probar el abort).
    struct MockEngine {
        handles: Vec<ExecHandle>,
        state: LifecycleState,
        telemetry_ok: bool,
        list_fails: bool,
    }

    #[async_trait]
    impl Engine for MockEngine {
        async fn run(&self, _intent: Intent) -> Result<ExecHandle, EngineError> {
            unreachable!("el monitor no arranca nada")
        }
        async fn stop(&self, _id: Ulid, _grace: Duration) -> Result<(), EngineError> {
            unreachable!("el monitor no para nada")
        }
        async fn list(&self) -> Result<Vec<ExecHandle>, EngineError> {
            if self.list_fails {
                return Err(EngineError::Transport("mock caído".into()));
            }
            Ok(self.handles.clone())
        }
        async fn status(&self, _id: Ulid) -> Result<LifecycleState, EngineError> {
            Ok(self.state.clone())
        }
        async fn telemetry(&self, id: Ulid) -> Result<TelemetryFrame, EngineError> {
            if self.telemetry_ok {
                Ok(TelemetryFrame {
                    card_id: id,
                    at: SystemTime::UNIX_EPOCH,
                    mem_bytes: 1024,
                    nproc: 2,
                    cpu_pct: 3.5,
                    restarts: 5,
                })
            } else {
                Err(EngineError::NotFound(id))
            }
        }
    }

    fn handle(label: &str) -> ExecHandle {
        ExecHandle {
            card_id: Ulid::new(),
            label: label.into(),
            started_at: SystemTime::UNIX_EPOCH,
        }
    }

    #[tokio::test]
    async fn observe_arma_snapshot_con_estado_y_telemetria() {
        let eng = MockEngine {
            handles: vec![handle("a"), handle("b")],
            state: LifecycleState::Running,
            telemetry_ok: true,
            list_fails: false,
        };
        let snap = observe(&eng).await.unwrap();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap.running(), 2);
        assert!(snap.units[0].telemetry.is_some());
        assert_eq!(snap.units[0].telemetry.as_ref().unwrap().mem_bytes, 1024);
        assert_eq!(snap.units[0].restarts, 5); // surfaceado del frame
    }

    #[tokio::test]
    async fn telemetria_indisponible_degrada_a_none() {
        let eng = MockEngine {
            handles: vec![handle("a")],
            state: LifecycleState::Running,
            telemetry_ok: false,
            list_fails: false,
        };
        let snap = observe(&eng).await.unwrap();
        assert_eq!(snap.len(), 1);
        assert!(snap.units[0].telemetry.is_none()); // no tumba el snapshot
    }

    #[tokio::test]
    async fn list_caido_aborta_la_observacion() {
        let eng = MockEngine {
            handles: vec![],
            state: LifecycleState::Running,
            telemetry_ok: true,
            list_fails: true,
        };
        assert!(observe(&eng).await.is_err());
    }
}
