//! `llimphi-motion` — animaciones simples sobre el bucle Elm de Llimphi.
//!
//! Llimphi es Elm puro: `update(msg) -> model`. Para animar un valor en
//! el tiempo (un alpha que sube de 0 a 1, una posición que se desliza)
//! la app guarda un [`Tween`] en su modelo y pide al `Handle` que le
//! dispatchee un `Msg::Tick` periódicamente (cada ~16 ms) hasta que la
//! animación termine. Cada `update` lee `tween.value()` y la `view` la
//! pinta.
//!
//! Esta crate es deliberadamente chiquita:
//! - [`Lerp`] — interpolación lineal genérica (impls para `f32`,
//!   `(f32, f32)` y `Color`).
//! - [`Tween`] — interpolación temporizada con easing entre dos valores.
//! - [`animate`] — helper que arranca un loop de ticks autosuficiente
//!   sobre un `Handle`.
//!
//! Las duraciones y easings canónicos viven en [`llimphi_theme::motion`].
//!
//! ## Patrón típico
//!
//! ```ignore
//! use llimphi_motion::{Tween, animate};
//! use llimphi_theme::motion;
//!
//! enum Msg { ToastShow, Tick, ToastHidden }
//! struct Model { toast_alpha: Tween<f32> }
//!
//! // update:
//! Msg::ToastShow => {
//!     model.toast_alpha = Tween::new(0.0, 1.0, motion::NORMAL, motion::ease_out_cubic);
//!     animate(handle, motion::NORMAL, || Msg::Tick);
//!     model
//! }
//! Msg::Tick => {
//!     // El loop interno terminará solo cuando el tween esté done;
//!     // la `view` ya lee el alpha actual sin más.
//!     model
//! }
//!
//! // view:
//! toast_view().alpha(model.toast_alpha.value())
//! ```

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

pub use llimphi_theme::motion;
pub use llimphi_theme::Color;
use llimphi_ui::Handle;

/// Interpolación lineal genérica entre `self` y `other` con factor `t`
/// en `[0.0, 1.0]`. Cada impl decide cómo combinar componentes; los
/// callers pasan `t` ya con el easing aplicado.
pub trait Lerp: Copy {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for f64 {
    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t as f64
    }
}

impl Lerp for (f32, f32) {
    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        (self.0.lerp(other.0, t), self.1.lerp(other.1, t))
    }
}

impl Lerp for (f64, f64) {
    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        (self.0.lerp(other.0, t), self.1.lerp(other.1, t))
    }
}

impl Lerp for Color {
    /// Interpolación componente a componente sobre los 4 canales RGBA
    /// en espacio sRGB lineal-asumido. No es colorimetricamente correcto
    /// (debería ser oklab), pero para fades de alpha/tinte de UI es
    /// indistinguible y mucho más barato.
    #[inline]
    fn lerp(self, other: Self, t: f32) -> Self {
        let a = self.components;
        let b = other.components;
        Color {
            components: [
                a[0].lerp(b[0], t),
                a[1].lerp(b[1], t),
                a[2].lerp(b[2], t),
                a[3].lerp(b[3], t),
            ],
            ..self
        }
    }
}

/// Animación temporizada de un valor `T: Lerp` entre `from` y `to`.
///
/// El tween es **observable**: la app llama [`Tween::value`] desde su
/// `view` y obtiene el valor interpolado para el frame actual. No tiene
/// estado mutable: el tiempo se mide contra un `Instant` de inicio, así
/// que el mismo `Tween` puede ser leído desde múltiples lugares sin
/// que se desincronice.
#[derive(Debug, Clone, Copy)]
pub struct Tween<T: Lerp> {
    pub from: T,
    pub to: T,
    started: Instant,
    pub duration: Duration,
    /// Función de easing aplicada a `t ∈ [0, 1]` antes de interpolar.
    /// Las canónicas viven en [`llimphi_theme::motion`].
    pub easing: fn(f32) -> f32,
}

impl<T: Lerp> Tween<T> {
    /// Arranca el tween *ahora*. La primera lectura siguiente devuelve
    /// `from`; cuando hayan pasado `duration` segundos, devuelve `to`.
    pub fn new(from: T, to: T, duration: Duration, easing: fn(f32) -> f32) -> Self {
        Self {
            from,
            to,
            started: Instant::now(),
            duration,
            easing,
        }
    }

    /// Tween que ya está terminado y siempre devuelve el mismo valor.
    /// Útil para inicializar un campo de modelo antes de cualquier animación.
    pub fn idle(value: T) -> Self {
        Self {
            from: value,
            to: value,
            started: Instant::now() - Duration::from_secs(1),
            duration: Duration::from_millis(1),
            easing: motion::linear,
        }
    }

    /// Progreso normalizado en `[0.0, 1.0]`, ya con easing aplicado.
    pub fn progress(&self) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let elapsed = self.started.elapsed().as_secs_f32();
        let t = (elapsed / self.duration.as_secs_f32()).clamp(0.0, 1.0);
        (self.easing)(t)
    }

    /// Valor actual interpolado.
    pub fn value(&self) -> T {
        self.from.lerp(self.to, self.progress())
    }

    /// `true` si la animación ya completó su `duration`.
    pub fn done(&self) -> bool {
        self.started.elapsed() >= self.duration
    }
}

/// Lanza un loop de ticks de animación que dispara `make_msg()` a ~60 Hz
/// durante `duration`, y se autodetiene cuando termina. El callback no
/// hace falta que verifique el tiempo: la app lee `tween.value()` y el
/// hilo interno se encarga de los frames.
///
/// Cada tick dispatcha un `Msg` al `update` — la app no tiene que hacer
/// nada en ese update salvo, eventualmente, leer el `Tween` cuya
/// `progress()` cambió desde la última lectura. La `view` luego se
/// repinta con el valor interpolado del frame.
///
/// **Detención**: el hilo de ticks vive `duration + 32ms` (un frame
/// extra de gracia para que el último tick caiga *después* del tope
/// del tween y la `view` final pinte el valor `to`). No hace falta
/// cancelar manualmente. Para tweens encadenados (A → B → C) la app
/// llama `animate()` de nuevo desde el `update` cuando el tween anterior
/// termina.
///
/// Internamente usa un hilo dedicado (no `spawn_periodic`, que es
/// infinito) y dispatcha vía `Handle::dispatch` clonado.
pub fn animate<F, Msg>(handle: &Handle<Msg>, duration: Duration, make_msg: F)
where
    F: Fn() -> Msg + Send + Sync + 'static,
    Msg: Clone + Send + 'static,
{
    let frame = Duration::from_millis(16);
    let total = duration + Duration::from_millis(32);
    let handle = handle.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        while start.elapsed() <= total {
            handle.dispatch(make_msg());
            std::thread::sleep(frame);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_f32_endpoints() {
        assert!((0.0_f32.lerp(10.0, 0.0) - 0.0).abs() < 1e-6);
        assert!((0.0_f32.lerp(10.0, 1.0) - 10.0).abs() < 1e-6);
        assert!((0.0_f32.lerp(10.0, 0.5) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn lerp_tuple_componentwise() {
        let p = (0.0_f32, 100.0).lerp((10.0, 0.0), 0.5);
        assert!((p.0 - 5.0).abs() < 1e-6);
        assert!((p.1 - 50.0).abs() < 1e-6);
    }

    #[test]
    fn lerp_color_endpoints() {
        let a = Color::from_rgba8(0, 0, 0, 0);
        let b = Color::from_rgba8(255, 255, 255, 255);
        let mid = a.lerp(b, 0.5);
        let [r, g, bl, al] = mid.components;
        assert!((r - 0.5).abs() < 1e-3);
        assert!((g - 0.5).abs() < 1e-3);
        assert!((bl - 0.5).abs() < 1e-3);
        assert!((al - 0.5).abs() < 1e-3);
    }

    #[test]
    fn tween_idle_returns_constant_value() {
        let t = Tween::idle(42.0_f32);
        assert!((t.value() - 42.0).abs() < 1e-6);
        assert!(t.done());
    }

    #[test]
    fn tween_zero_duration_immediately_done() {
        let t = Tween::new(0.0_f32, 1.0, Duration::ZERO, motion::linear);
        assert!((t.progress() - 1.0).abs() < 1e-6);
        assert!((t.value() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn tween_progress_clamps_after_duration() {
        let t = Tween::new(0.0_f32, 10.0, Duration::from_millis(1), motion::linear);
        std::thread::sleep(Duration::from_millis(10));
        assert!((t.progress() - 1.0).abs() < 1e-6);
        assert!((t.value() - 10.0).abs() < 1e-6);
    }
}
