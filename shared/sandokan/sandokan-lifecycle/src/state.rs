//! Máquina de estados del ciclo de vida de una entidad supervisada.

use serde::{Deserialize, Serialize};

/// Estado de una entidad supervisada (proceso, workspace, sandbox, ...).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LifecycleState {
    /// Creada, aún no arrancó.
    Pending,
    /// En ejecución.
    Running,
    /// Salió por sí misma con un código de salida.
    Exited { code: i32 },
    /// Falló (no llegó a correr, o crasheó de forma no capturable).
    Failed { reason: String },
    /// Terminada por el supervisor (SIGKILL / quota / drain).
    Killed,
}

impl LifecycleState {
    /// `true` si el estado es terminal (no habrá más transiciones sin
    /// un restart explícito).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            LifecycleState::Exited { .. }
                | LifecycleState::Failed { .. }
                | LifecycleState::Killed
        )
    }

    /// `true` si el estado terminal cuenta como fallo (dispara restart
    /// si la política lo permite). `Exited { code: 0 }` NO es fallo.
    pub fn is_failure(&self) -> bool {
        match self {
            LifecycleState::Exited { code } => *code != 0,
            LifecycleState::Failed { .. } => true,
            LifecycleState::Killed => false, // kill deliberado, no fallo
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_detection() {
        assert!(!LifecycleState::Pending.is_terminal());
        assert!(!LifecycleState::Running.is_terminal());
        assert!(LifecycleState::Exited { code: 0 }.is_terminal());
        assert!(LifecycleState::Killed.is_terminal());
        assert!(LifecycleState::Failed { reason: "x".into() }.is_terminal());
    }

    #[test]
    fn failure_semantics() {
        assert!(!LifecycleState::Exited { code: 0 }.is_failure());
        assert!(LifecycleState::Exited { code: 1 }.is_failure());
        assert!(LifecycleState::Failed { reason: "x".into() }.is_failure());
        assert!(!LifecycleState::Killed.is_failure());
        assert!(!LifecycleState::Running.is_failure());
    }
}
