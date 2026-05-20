//! Time-to-live anclado a un `Instant`.

use std::time::{Duration, Instant};

/// Time-to-live. Marca un instante límite tras el cual una entidad
/// supervisada se considera vencida. Runtime-only (no serializable:
/// `Instant` no tiene representación estable).
#[derive(Debug, Clone, Copy)]
pub struct Ttl {
    deadline: Instant,
}

impl Ttl {
    /// TTL que vence `lifetime` después de ahora.
    pub fn new(lifetime: Duration) -> Self {
        Self { deadline: Instant::now() + lifetime }
    }

    /// TTL con un deadline absoluto explícito.
    pub fn from_deadline(deadline: Instant) -> Self {
        Self { deadline }
    }

    /// `true` si el deadline ya pasó.
    pub fn expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Tiempo restante hasta el deadline. `Duration::ZERO` si ya venció.
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// El instante límite.
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_ttl_not_expired() {
        let t = Ttl::new(Duration::from_secs(60));
        assert!(!t.expired());
        assert!(t.remaining() > Duration::from_secs(58));
    }

    #[test]
    fn past_deadline_is_expired() {
        let t = Ttl::from_deadline(Instant::now() - Duration::from_secs(1));
        assert!(t.expired());
        assert_eq!(t.remaining(), Duration::ZERO);
    }
}
