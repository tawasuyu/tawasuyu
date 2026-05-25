//! Observabilidad: eventos de ciclo de vida y frames de telemetría.

use sandokan_lifecycle::LifecycleState;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use ulid::Ulid;

/// Un evento en la vida de una entidad encarnada. El orquestador los
/// emite; los consumidores (shells, paneles) reaccionan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifecycleEvent {
    /// La entidad fue encarnada. `pid` es `None` si no aplica (Wasm, virtual).
    Spawned { card_id: Ulid, pid: Option<i32> },
    /// El estado de la entidad cambió.
    StateChanged { card_id: Ulid, state: LifecycleState },
    /// La entidad terminó (estado terminal).
    Exited { card_id: Ulid, state: LifecycleState },
}

impl LifecycleEvent {
    /// El `card_id` al que refiere el evento.
    pub fn card_id(&self) -> Ulid {
        match self {
            LifecycleEvent::Spawned { card_id, .. }
            | LifecycleEvent::StateChanged { card_id, .. }
            | LifecycleEvent::Exited { card_id, .. } => *card_id,
        }
    }
}

/// Una medición puntual de recursos de una entidad. Los campos se
/// inlinean (en vez de reusar `sandokan_lifecycle::ResourceUsage`) para
/// que el frame sea un wire type estable e independiente.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryFrame {
    pub card_id: Ulid,
    pub at: SystemTime,
    pub mem_bytes: u64,
    pub nproc: u32,
    /// Porcentaje de CPU (100.0 = 1 core saturado).
    pub cpu_pct: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_card_id_accessor() {
        let id = Ulid::new();
        let ev = LifecycleEvent::Exited {
            card_id: id,
            state: LifecycleState::Exited { code: 0 },
        };
        assert_eq!(ev.card_id(), id);
    }
}
