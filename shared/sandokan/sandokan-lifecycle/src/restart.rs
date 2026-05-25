//! Política de restart con conteo + backoff exponencial.

use crate::backoff::Backoff;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Política declarativa de restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestartPolicy {
    /// Si reintentar tras una salida con fallo.
    pub on_failure: bool,
    /// Máximo de restarts. `0` = infinito.
    pub max_restarts: u32,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self { on_failure: false, max_restarts: 0 }
    }
}

/// Estado mutable de restart de una entidad supervisada. Combina la
/// política con un `Backoff` y el conteo de intentos consumidos.
#[derive(Debug, Clone)]
pub struct RestartTracker {
    policy: RestartPolicy,
    backoff: Backoff,
    count: u32,
}

impl RestartTracker {
    pub fn new(policy: RestartPolicy, backoff: Backoff) -> Self {
        Self { policy, backoff, count: 0 }
    }

    /// Registra un fallo. Devuelve `Some(delay)` con el backoff a esperar
    /// antes del próximo intento, o `None` si no se debe reintentar
    /// (política desactivada o `max_restarts` agotado).
    pub fn on_failure(&mut self) -> Option<Duration> {
        if !self.policy.on_failure {
            return None;
        }
        if self.policy.max_restarts != 0 && self.count >= self.policy.max_restarts {
            return None;
        }
        self.count += 1;
        Some(self.backoff.next_delay())
    }

    /// Registra un éxito: resetea conteo y backoff.
    pub fn on_success(&mut self) {
        self.count = 0;
        self.backoff.reset();
    }

    /// Cantidad de restarts consumidos.
    pub fn count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backoff() -> Backoff {
        Backoff::new(Duration::from_millis(100), Duration::from_secs(30))
    }

    #[test]
    fn disabled_policy_never_restarts() {
        let mut t = RestartTracker::new(
            RestartPolicy { on_failure: false, max_restarts: 0 },
            backoff(),
        );
        assert!(t.on_failure().is_none());
    }

    #[test]
    fn respects_max_restarts() {
        let mut t = RestartTracker::new(
            RestartPolicy { on_failure: true, max_restarts: 3 },
            backoff(),
        );
        assert!(t.on_failure().is_some());
        assert!(t.on_failure().is_some());
        assert!(t.on_failure().is_some());
        assert!(t.on_failure().is_none()); // 4º agota la cuota
        assert_eq!(t.count(), 3);
    }

    #[test]
    fn infinite_when_max_zero() {
        let mut t = RestartTracker::new(
            RestartPolicy { on_failure: true, max_restarts: 0 },
            backoff(),
        );
        for _ in 0..100 {
            assert!(t.on_failure().is_some());
        }
    }

    #[test]
    fn backoff_escalates_then_success_resets() {
        let mut t = RestartTracker::new(
            RestartPolicy { on_failure: true, max_restarts: 0 },
            backoff(),
        );
        assert_eq!(t.on_failure(), Some(Duration::from_millis(100)));
        assert_eq!(t.on_failure(), Some(Duration::from_millis(200)));
        t.on_success();
        assert_eq!(t.count(), 0);
        assert_eq!(t.on_failure(), Some(Duration::from_millis(100)));
    }
}
