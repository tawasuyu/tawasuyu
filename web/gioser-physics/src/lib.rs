//! Resorte-amortiguador genérico de N dimensiones.
//!
//! Sirve para animaciones orgánicas: el "tilt" de la chacana hacia el mouse,
//! el pulso del sol, las transiciones de hover. La integración es semi-implícita
//! Euler — estable para `dt < 1/freq_hz`. Si `dt` real puede exceder ese límite,
//! el caller subdivide.
//!
//! `damping_ratio`:
//! - `1.0` crítico: settle en tiempo mínimo, sin overshoot.
//! - `0.7` sub-crítico: overshoot suave (≈4.6 %), se siente vivo.
//! - `< 0.5` muy oscilante (no recomendado fuera de FX).

#![no_std]

#[derive(Clone, Copy, Debug)]
pub struct SpringDamper<const N: usize> {
    pub position: [f32; N],
    pub velocity: [f32; N],
    pub target: [f32; N],
    /// Frecuencia natural en Hz.
    pub freq_hz: f32,
    /// 1.0 = crítico, < 1.0 = oscila.
    pub damping_ratio: f32,
}

impl<const N: usize> SpringDamper<N> {
    pub const fn new(freq_hz: f32, damping_ratio: f32) -> Self {
        Self {
            position: [0.0; N],
            velocity: [0.0; N],
            target: [0.0; N],
            freq_hz,
            damping_ratio,
        }
    }

    pub fn with_position(mut self, pos: [f32; N]) -> Self {
        self.position = pos;
        self.target = pos;
        self
    }

    pub fn set_target(&mut self, t: [f32; N]) {
        self.target = t;
    }

    /// Avanza la simulación. Caller suele pasarlo desde un `requestAnimationFrame`.
    pub fn step(&mut self, dt: f32) {
        // Tau = 2π. core::f32::consts::TAU está estable desde 1.47.
        let omega = core::f32::consts::TAU * self.freq_hz;
        let zeta = self.damping_ratio;
        let k = omega * omega;
        let c = 2.0 * zeta * omega;
        let mut i = 0;
        while i < N {
            let dx = self.position[i] - self.target[i];
            let a = -k * dx - c * self.velocity[i];
            self.velocity[i] += a * dt;
            self.position[i] += self.velocity[i] * dt;
            i += 1;
        }
    }

    /// `true` cuando el sistema está esencialmente parado en el target.
    pub fn at_rest(&self, eps_pos: f32, eps_vel: f32) -> bool {
        let mut i = 0;
        while i < N {
            if (self.position[i] - self.target[i]).abs() > eps_pos
                || self.velocity[i].abs() > eps_vel
            {
                return false;
            }
            i += 1;
        }
        true
    }
}

pub type SpringDamper1 = SpringDamper<1>;
pub type SpringDamper2 = SpringDamper<2>;
pub type SpringDamper3 = SpringDamper<3>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_at_target_when_critically_damped() {
        let mut s = SpringDamper2::new(3.0, 1.0);
        s.set_target([1.0, -0.5]);
        for _ in 0..300 {
            s.step(1.0 / 120.0);
        }
        assert!((s.position[0] - 1.0).abs() < 1e-3);
        assert!((s.position[1] + 0.5).abs() < 1e-3);
        assert!(s.at_rest(1e-3, 1e-3));
    }

    #[test]
    fn underdamped_overshoots() {
        let mut s = SpringDamper1::new(3.0, 0.3);
        s.set_target([1.0]);
        let mut peak = 0.0f32;
        for _ in 0..240 {
            s.step(1.0 / 240.0);
            if s.position[0] > peak {
                peak = s.position[0];
            }
        }
        assert!(peak > 1.0, "underdamped should overshoot, peak={}", peak);
    }

    #[test]
    fn at_rest_initially() {
        let s: SpringDamper2 = SpringDamper::new(2.0, 1.0);
        assert!(s.at_rest(1e-6, 1e-6));
    }
}
