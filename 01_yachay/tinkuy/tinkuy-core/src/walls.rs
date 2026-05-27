//! Reflective boundary conditions: partículas rebotan elásticamente en las
//! paredes del dominio. Conserva energía cinética exactamente (sólo cambia
//! signo de la componente normal).
//!
//! Llamar DESPUÉS del drift en cada step (idealmente entre `kick_drift` y
//! `merge_transfers`, así la grilla se actualiza con las posiciones ya
//! corregidas).

use crate::ecs::World;

#[inline]
pub fn reflect_walls(
    world: &mut World,
    bounds_min: [f32; 3],
    bounds_max: [f32; 3],
) {
    let n = world.len();
    for i in 0..n {
        // Eje X.
        if world.xs.0[i] < bounds_min[0] {
            world.xs.0[i] = 2.0 * bounds_min[0] - world.xs.0[i];
            world.vxs.0[i] = -world.vxs.0[i];
        } else if world.xs.0[i] > bounds_max[0] {
            world.xs.0[i] = 2.0 * bounds_max[0] - world.xs.0[i];
            world.vxs.0[i] = -world.vxs.0[i];
        }
        // Eje Y.
        if world.ys.0[i] < bounds_min[1] {
            world.ys.0[i] = 2.0 * bounds_min[1] - world.ys.0[i];
            world.vys.0[i] = -world.vys.0[i];
        } else if world.ys.0[i] > bounds_max[1] {
            world.ys.0[i] = 2.0 * bounds_max[1] - world.ys.0[i];
            world.vys.0[i] = -world.vys.0[i];
        }
        // Eje Z.
        if world.zs.0[i] < bounds_min[2] {
            world.zs.0[i] = 2.0 * bounds_min[2] - world.zs.0[i];
            world.vzs.0[i] = -world.vzs.0[i];
        } else if world.zs.0[i] > bounds_max[2] {
            world.zs.0[i] = 2.0 * bounds_max[2] - world.zs.0[i];
            world.vzs.0[i] = -world.vzs.0[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observables::kinetic_energy;

    #[test]
    fn particle_bounces_off_wall_and_preserves_speed() {
        let mut w = World::with_capacity(1);
        // partícula a x=10.5 con v=+1 — fuera del dominio [0,10]
        w.spawn([10.5, 5.0, 5.0], [1.0, 0.0, 0.0], 1.0, 0.0);
        let ke_before = kinetic_energy(&w);
        reflect_walls(&mut w, [0.; 3], [10., 10., 10.]);
        // Posición debe quedar reflejada: 2·10 − 10.5 = 9.5
        assert!((w.xs.0[0] - 9.5).abs() < 1e-6);
        // Velocidad debe invertirse: −1
        assert_eq!(w.vxs.0[0], -1.0);
        // Energía cinética conservada.
        let ke_after = kinetic_energy(&w);
        assert!((ke_after - ke_before).abs() < 1e-9);
    }

    #[test]
    fn particle_inside_bounds_is_untouched() {
        let mut w = World::with_capacity(1);
        w.spawn([5.0, 5.0, 5.0], [1.0, 2.0, -3.0], 1.0, 0.0);
        let before = (w.xs.0[0], w.ys.0[0], w.zs.0[0], w.vxs.0[0], w.vys.0[0], w.vzs.0[0]);
        reflect_walls(&mut w, [0.; 3], [10., 10., 10.]);
        let after = (w.xs.0[0], w.ys.0[0], w.zs.0[0], w.vxs.0[0], w.vys.0[0], w.vzs.0[0]);
        assert_eq!(before, after);
    }

    #[test]
    fn multi_axis_reflection_works() {
        let mut w = World::with_capacity(1);
        // Sale por +x y −y simultáneamente.
        w.spawn([11.0, -0.5, 5.0], [2.0, -1.0, 0.0], 1.0, 0.0);
        reflect_walls(&mut w, [0.; 3], [10., 10., 10.]);
        assert!((w.xs.0[0] - 9.0).abs() < 1e-6);
        assert!((w.ys.0[0] - 0.5).abs() < 1e-6);
        assert_eq!(w.vxs.0[0], -2.0);
        assert_eq!(w.vys.0[0],  1.0);
    }
}
