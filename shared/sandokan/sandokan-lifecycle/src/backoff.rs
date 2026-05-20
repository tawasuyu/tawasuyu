//! Backoff exponencial con tope.

use std::time::Duration;

/// Calculador de backoff exponencial. Cada `next_delay()` devuelve el
/// delay actual y luego lo duplica, hasta saturar en `max`.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    current: Duration,
}

impl Backoff {
    /// Crea un backoff que arranca en `base` y satura en `max`.
    /// Si `base > max`, `base` se clampa a `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        let base = base.min(max);
        Self { base, max, current: base }
    }

    /// Devuelve el delay actual y escala el siguiente (×2, capeado a `max`).
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        delay
    }

    /// Vuelve al delay base (tras un éxito).
    pub fn reset(&mut self) {
        self.current = self.base;
    }

    /// Delay que devolvería el próximo `next_delay()` sin consumirlo.
    pub fn peek(&self) -> Duration {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalates_then_caps() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_millis(800));
        assert_eq!(b.next_delay(), Duration::from_millis(100));
        assert_eq!(b.next_delay(), Duration::from_millis(200));
        assert_eq!(b.next_delay(), Duration::from_millis(400));
        assert_eq!(b.next_delay(), Duration::from_millis(800));
        assert_eq!(b.next_delay(), Duration::from_millis(800)); // capeado
    }

    #[test]
    fn reset_returns_to_base() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(30));
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(100));
    }

    #[test]
    fn base_clamped_to_max() {
        let mut b = Backoff::new(Duration::from_secs(10), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }
}
