//! Estado del **hero de lock/deslock**: el progreso temporal de la transición
//! soñada. Al **bloquear**, la pantalla viva se *encoge* hasta el thumbnail de la
//! sesión (zoom-out) con el velo subiendo. Al **desbloquear**, la misma captura
//! congelada *crece* de vuelta desde el thumbnail hasta la pantalla completa
//! (zoom-in) con el velo bajando. Dos direcciones sobre el mismo mecanismo.
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

/// El progreso del hero: cuánto lleva (`t ∈ [0,1]`), su duración, el rect del
/// thumbnail (`target`) y la **dirección**. En **lock** encoge `full → target`
/// con el velo subiendo; en **deslock** crece `target → full` con el velo
/// bajando (el zoom-in de vuelta a la sesión viva). Mismo mecanismo, invertido.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LockHero {
    t: f32,
    dur: f32,
    target: Rect,
    /// `false` = lock (encoge, velo sube). `true` = deslock (crece, velo baja).
    unlock: bool,
}

impl LockHero {
    /// Hero de **lock**: encoge de pantalla completa a `target` (el
    /// [`mirada_layout::landing_rect`] de la salida), con la duración por defecto.
    pub(crate) fn new(target: Rect) -> Self {
        Self { t: 0.0, dur: HERO_SECS.max(0.001), target, unlock: false }
    }

    /// Hero de **deslock**: crece de `target` (el thumbnail) a pantalla completa
    /// — el zoom-in de vuelta a la sesión al desbloquear. Reusa la captura
    /// congelada que el lock dejó retenida.
    pub(crate) fn new_unlock(target: Rect) -> Self {
        Self { t: 0.0, dur: HERO_SECS.max(0.001), target, unlock: true }
    }

    /// `true` si es un hero de deslock (crece), no de lock (encoge). El backend
    /// lo usa para saber si al terminar debe descartar la captura o retenerla.
    pub(crate) fn is_unlock(&self) -> bool {
        self.unlock
    }

    /// Avanza el progreso `dt` segundos. Devuelve `true` cuando el hero terminó
    /// (`t` llegó a `1.0`) — el backend entonces lo descarta.
    pub(crate) fn advance(&mut self, dt: f32) -> bool {
        self.t = (self.t + dt.max(0.0) / self.dur).min(1.0);
        self.done()
    }

    /// `true` si la transición completó.
    pub(crate) fn done(&self) -> bool {
        self.t >= 1.0
    }

    /// El rect de la captura este frame, con el easing suave de
    /// [`mirada_layout::hero`]. Lock: `full → target` (encoge). Deslock:
    /// `target → full` (crece).
    pub(crate) fn rect(&self, full: Rect) -> Rect {
        if self.unlock {
            mirada_layout::hero_rect(self.target, full, self.t)
        } else {
            mirada_layout::hero_rect(full, self.target, self.t)
        }
    }

    /// Opacidad del velo oscuro detrás de la captura. Lock: sube `0→1` (la sesión
    /// cede paso al greeter). Deslock: baja `1→0` (la sesión vuelve).
    pub(crate) fn veil_alpha(&self) -> f32 {
        let e = mirada_layout::hero::ease_in_out(self.t);
        if self.unlock {
            1.0 - e
        } else {
            e
        }
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
    fn deslock_crece_del_thumbnail_a_full_y_el_velo_baja() {
        let target = mirada_layout::landing_rect(1920, 1080);
        let mut h = LockHero::new_unlock(target);
        assert!(h.is_unlock());
        // Arranca en el thumbnail con el velo lleno (la sesión aún tapada).
        assert_eq!(h.rect(full()), target);
        assert_eq!(h.veil_alpha(), 1.0);
        // A mitad, el rect creció (entre target y full) y el velo bajó.
        assert!(!h.advance(HERO_SECS * 0.5));
        let mid = h.rect(full());
        assert!(mid.w > target.w && mid.w < 1920, "crece hacia full");
        assert!(h.veil_alpha() < 1.0 && h.veil_alpha() > 0.0);
        // Al terminar: pantalla completa y velo en cero (sesión revelada).
        assert!(h.advance(HERO_SECS));
        assert_eq!(h.rect(full()), full());
        assert_eq!(h.veil_alpha(), 0.0);
    }

    #[test]
    fn dt_negativo_o_cero_no_rompe() {
        let mut h = LockHero::new(mirada_layout::landing_rect(800, 600));
        assert!(!h.advance(0.0));
        assert!(!h.advance(-1.0));
        assert_eq!(h.rect(Rect::new(0, 0, 800, 600)), Rect::new(0, 0, 800, 600));
    }
}
