//! Estado del **hero de lock**: el progreso temporal de la transición soñada
//! (la pantalla viva haciendo **zoom-in** —creciendo hacia el usuario— al
//! bloquear, antes de revelar el greeter). El `target` lo decide el llamador
//! ([`mirada_layout::zoom_in_rect`]); `LockHero` sólo interpola `full → target`.
//!
//! Esta parte es **pura y testeable**: sólo el reloj de progreso `0→1` y el rect
//! interpolado. La captura congelada del output (una `GlesTexture`) y el render
//! escalado viven en el backend DRM, que lee este progreso cada frame. La
//! geometría compartida (rect destino + easing) la aporta
//! [`mirada_layout::hero`], así el greeter y el compositor coinciden en el
//! aterrizaje.

use mirada_layout::geometry::Rect;

/// Duración por defecto del encogido (segundos).
pub(crate) const HERO_SECS: f32 = 0.42;

/// El progreso del hero: cuánto lleva (`t ∈ [0,1]`), su duración y el rect
/// destino (el thumbnail de aterrizaje en coordenadas de la salida).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LockHero {
    t: f32,
    dur: f32,
    target: Rect,
}

impl LockHero {
    /// Arranca un hero hacia `target` (el [`mirada_layout::landing_rect`] de la
    /// salida) con la duración por defecto.
    pub(crate) fn new(target: Rect) -> Self {
        Self { t: 0.0, dur: HERO_SECS.max(0.001), target }
    }

    /// Avanza el progreso `dt` segundos. Devuelve `true` cuando el hero terminó
    /// (`t` llegó a `1.0`) — el backend entonces lo descarta y revela el greeter.
    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        self.t = (self.t + dt.max(0.0) / self.dur).min(1.0);
        self.done()
    }

    /// `true` si el encogido completó.
    pub(crate) fn done(&self) -> bool {
        self.t >= 1.0
    }

    /// El rect de la captura este frame: de `full` (pantalla completa) al
    /// thumbnail destino, con el easing suave de [`mirada_layout::hero`].
    pub(crate) fn rect(&self, full: Rect) -> Rect {
        mirada_layout::hero_rect(full, self.target, self.t)
    }

    /// Opacidad de un velo oscuro detrás de la captura, que sube `0→1` con el
    /// progreso (la sesión que se va cediendo paso al greeter).
    pub(crate) fn veil_alpha(&self) -> f32 {
        mirada_layout::hero::ease_in_out(self.t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }

    #[test]
    fn arranca_en_pantalla_completa() {
        let h = LockHero::new(mirada_layout::landing_rect(1920, 1080));
        assert_eq!(h.rect(full()), full());
        assert!(!h.done());
        assert_eq!(h.veil_alpha(), 0.0);
    }

    #[test]
    fn avanza_y_termina_en_el_target() {
        let target = mirada_layout::landing_rect(1920, 1080);
        let mut h = LockHero::new(target);
        // A mitad de duración no terminó y el rect va achicándose.
        assert!(!h.advance(HERO_SECS * 0.5));
        let mid = h.rect(full());
        assert!(mid.w < 1920 && mid.w > target.w);
        // Pasada la duración total, termina y aterriza exacto en el target.
        assert!(h.advance(HERO_SECS));
        assert!(h.done());
        assert_eq!(h.rect(full()), target);
        assert_eq!(h.veil_alpha(), 1.0);
    }

    #[test]
    fn dt_negativo_o_cero_no_rompe() {
        let mut h = LockHero::new(mirada_layout::landing_rect(800, 600));
        assert!(!h.advance(0.0));
        assert!(!h.advance(-1.0));
        assert_eq!(h.rect(Rect::new(0, 0, 800, 600)), Rect::new(0, 0, 800, 600));
    }
}
