//! Observables físicos del `World`: energía cinética, momentum total,
//! temperatura cinética. Funciones puras, sin mutación.
//!
//! Las reducciones son escalares serialmente — el coste es O(N) y dominado
//! por bandwidth de memoria; paralelizar con rayon añade overhead que solo
//! amortiza por encima de ~100k partículas (lo dejamos para cuando midamos).

use crate::ecs::World;

/// Energía cinética total: Σ ½·m_i·|v_i|². Acumulada en f64 para mantener
/// precisión con muchos términos.
#[inline]
pub fn kinetic_energy(world: &World) -> f64 {
    let n = world.len();
    let mut sum = 0.0f64;
    for i in 0..n {
        let m = world.masses.0[i] as f64;
        let vx = world.vxs.0[i] as f64;
        let vy = world.vys.0[i] as f64;
        let vz = world.vzs.0[i] as f64;
        sum += 0.5 * m * (vx * vx + vy * vy + vz * vz);
    }
    sum
}

/// Momentum total Σ m_i·v_i como vector (3 componentes en f64).
#[inline]
pub fn total_momentum(world: &World) -> [f64; 3] {
    let n = world.len();
    let mut px = 0.0f64;
    let mut py = 0.0f64;
    let mut pz = 0.0f64;
    for i in 0..n {
        let m = world.masses.0[i] as f64;
        px += m * world.vxs.0[i] as f64;
        py += m * world.vys.0[i] as f64;
        pz += m * world.vzs.0[i] as f64;
    }
    [px, py, pz]
}

/// Temperatura cinética según el teorema de equipartición:
///   T = 2·KE / (3·N·kB)
/// `kb` se pasa explícito; el caller elige unidades (en unidades reducidas LJ
/// suele ser 1.0).
#[inline]
pub fn temperature(world: &World, kb: f64) -> f64 {
    let n = world.len();
    if n == 0 || kb == 0.0 { return 0.0; }
    2.0 * kinetic_energy(world) / (3.0 * n as f64 * kb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinetic_energy_of_stationary_world_is_zero() {
        let mut w = World::with_capacity(3);
        for _ in 0..3 { w.spawn([0.; 3], [0.; 3], 1.0, 0.0); }
        assert_eq!(kinetic_energy(&w), 0.0);
    }

    #[test]
    fn kinetic_energy_matches_analytical() {
        let mut w = World::with_capacity(1);
        w.spawn([0.; 3], [3.0, 4.0, 0.0], 2.0, 0.0); // |v|=5, KE = ½·2·25 = 25
        let ke = kinetic_energy(&w);
        assert!((ke - 25.0).abs() < 1e-10, "KE={ke}");
    }

    #[test]
    fn momentum_sums_correctly() {
        let mut w = World::with_capacity(2);
        w.spawn([0.; 3], [ 1.0, 0.0, 0.0], 2.0, 0.0);
        w.spawn([0.; 3], [-1.0, 0.0, 0.0], 3.0, 0.0);
        let [px, py, pz] = total_momentum(&w);
        assert!((px - (-1.0)).abs() < 1e-10, "px={px}"); // 2·1 + 3·(-1) = -1
        assert_eq!(py, 0.0);
        assert_eq!(pz, 0.0);
    }

    #[test]
    fn temperature_equipartition() {
        // 1 partícula con KE=25 → T = 2·25 / (3·1·1) = 50/3 ≈ 16.6667
        let mut w = World::with_capacity(1);
        w.spawn([0.; 3], [3.0, 4.0, 0.0], 2.0, 0.0);
        let t = temperature(&w, 1.0);
        assert!((t - 50.0 / 3.0).abs() < 1e-9, "T={t}");
    }
}
