//! Seek de transporte sobre cualquier [`Seekable`] — la matemática que
//! antes vivía repetida en `media-app` (extraída 2026-06-12 para que
//! `media-module` y futuros frontends la compartan).
//!
//! Tres movimientos canónicos:
//!
//! - [`by_wrapped`]: salto **relativo** con wrap módulo duration (saltar
//!   atrás desde el segundo 2 cae al final, como un dial).
//! - [`to_fraction`]: salto **absoluto** a una fracción `0..=1` de la
//!   duración — lo que dispara un click en el timeline.
//! - [`to_pos`]: salto **absoluto** a una posición, clampeada a la
//!   duración — lo que usa un "resume" del historial.
//!
//! Las fuentes infinitas (tono, stream en vivo: `duration() == None`)
//! degradan igual que siempre: `by_wrapped` opera módulo 1 s (no-op
//! práctico), `to_fraction` queda en 0 y `to_pos` pasa el valor crudo
//! (la fuente clampea por contrato del trait).
//!
//! El re-anclaje A/V tras el salto (resetear [`crate::sync`], forzar el
//! present del frame destino) es responsabilidad del **pipeline del
//! host** — acá sólo se mueve la posición.

use std::time::Duration;

use crate::Seekable;

/// Mueve la posición en `delta_secs` (negativo = atrás) con wrap módulo
/// duration.
pub fn by_wrapped(src: &mut dyn Seekable, delta_secs: i64) {
    let dur = src.duration().unwrap_or(Duration::from_secs(1));
    let dur_s = dur.as_secs_f64().max(0.001);
    let cur_s = src.position().as_secs_f64();
    let new_s = (cur_s + delta_secs as f64).rem_euclid(dur_s);
    src.seek_to(Duration::from_secs_f64(new_s));
}

/// Salta a la posición absoluta `fraction` (`0..=1`, se clampea) de la
/// duración total.
pub fn to_fraction(src: &mut dyn Seekable, fraction: f32) {
    let dur_s = src.duration().unwrap_or(Duration::ZERO).as_secs_f64();
    let f = fraction.clamp(0.0, 1.0) as f64;
    src.seek_to(Duration::from_secs_f64(dur_s * f));
}

/// Salta a una posición absoluta, clampeada a la duración (para resume).
pub fn to_pos(src: &mut dyn Seekable, pos: Duration) {
    let dur = src.duration().unwrap_or(Duration::ZERO);
    let target = if dur.is_zero() { pos } else { pos.min(dur) };
    src.seek_to(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        pos: Duration,
        dur: Option<Duration>,
    }

    impl Seekable for Fake {
        fn position(&self) -> Duration {
            self.pos
        }
        fn duration(&self) -> Option<Duration> {
            self.dur
        }
        fn seek_to(&mut self, pos: Duration) {
            self.pos = pos;
        }
    }

    #[test]
    fn by_wrapped_envuelve_hacia_atras() {
        let mut f = Fake { pos: Duration::from_secs(2), dur: Some(Duration::from_secs(60)) };
        by_wrapped(&mut f, -5);
        assert_eq!(f.pos.as_secs(), 57);
    }

    #[test]
    fn by_wrapped_envuelve_hacia_adelante() {
        let mut f = Fake { pos: Duration::from_secs(58), dur: Some(Duration::from_secs(60)) };
        by_wrapped(&mut f, 5);
        assert_eq!(f.pos.as_secs(), 3);
    }

    #[test]
    fn to_fraction_clampa_y_escala() {
        let mut f = Fake { pos: Duration::ZERO, dur: Some(Duration::from_secs(100)) };
        to_fraction(&mut f, 0.5);
        assert_eq!(f.pos.as_secs(), 50);
        to_fraction(&mut f, 7.0);
        assert_eq!(f.pos.as_secs(), 100);
        to_fraction(&mut f, -1.0);
        assert_eq!(f.pos.as_secs(), 0);
    }

    #[test]
    fn to_pos_clampa_a_duracion() {
        let mut f = Fake { pos: Duration::ZERO, dur: Some(Duration::from_secs(30)) };
        to_pos(&mut f, Duration::from_secs(90));
        assert_eq!(f.pos.as_secs(), 30);
        // Sin duración (stream): pasa el valor crudo.
        let mut inf = Fake { pos: Duration::ZERO, dur: None };
        to_pos(&mut inf, Duration::from_secs(90));
        assert_eq!(inf.pos.as_secs(), 90);
    }
}
