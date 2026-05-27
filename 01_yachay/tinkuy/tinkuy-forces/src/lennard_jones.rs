//! Lennard-Jones 12-6 con cutoff.
//!
//! V(r) = 4ε [ (σ/r)¹² − (σ/r)⁶ ]
//! F_i = −∇_i V = (24ε/r²)[2(σ/r)¹² − (σ/r)⁶] · (r_i − r_j)
//!
//! Convención: el kernel **acumula** en `world.axs/ays/azs` (`+=`). Para
//! superponer varias fuerzas en el mismo step, el caller invoca primero
//! [`clear_accelerations`] (o lo hace una vez al inicio del step). El
//! integrador de tinkuy-core no limpia automáticamente porque el contrato de
//! `compute_forces` es opaco.
//!
//! Precondición: `grid.cell_size >= cutoff`. La grilla garantiza así que
//! todas las interacciones dentro del radio caen en las 27 celdas vecinas.

use rayon::prelude::*;
use tinkuy_core::{Grid3D, World};

pub struct LjParams {
    pub epsilon: f32,
    pub sigma:   f32,
    pub cutoff:  f32,
}

/// Limpia `axs/ays/azs[..n]` a cero. Llamar una vez al inicio de cada step
/// si se van a superponer varios kernels de fuerza.
pub fn clear_accelerations(world: &mut World) {
    let n = world.len();
    for v in [
        &mut world.axs.0, &mut world.ays.0, &mut world.azs.0,
    ] {
        for k in 0..n { v[k] = 0.0; }
    }
}

/// Newtype Send+Sync sobre *mut f32 (mismo patrón que en `integrator.rs`).
#[derive(Copy, Clone)]
#[repr(transparent)]
struct SyncPtr(*mut f32);
// SAFETY: cada partícula `i` solo es escrita por el worker que itera sobre `i`;
// no hay aliasing mutable simultáneo.
unsafe impl Send for SyncPtr {}
unsafe impl Sync for SyncPtr {}

#[cfg(feature = "cpu")]
pub fn lennard_jones(world: &mut World, grid: &Grid3D, p: &LjParams) {
    let n = world.len();
    if n == 0 { return; }
    debug_assert!(
        grid.cell_size >= p.cutoff,
        "cell_size ({}) < cutoff ({}) — vecinos posiblemente perdidos",
        grid.cell_size, p.cutoff
    );

    let xs = world.xs.0.as_slice();
    let ys = world.ys.0.as_slice();
    let zs = world.zs.0.as_slice();
    let masses = world.masses.0.as_slice();
    let cell_of = grid.cell_of.as_slice();

    let ax_p = SyncPtr(world.axs.0.as_mut_ptr());
    let ay_p = SyncPtr(world.ays.0.as_mut_ptr());
    let az_p = SyncPtr(world.azs.0.as_mut_ptr());

    let cutoff2 = p.cutoff * p.cutoff;
    let sigma2  = p.sigma * p.sigma;
    let eps     = p.epsilon;

    (0..n).into_par_iter().with_min_len(64).for_each(|i| {
        // Disjoint-capture (Rust 2021): forzar captura por struct entero.
        let _ = (&ax_p, &ay_p, &az_p);

        let xi = xs[i]; let yi = ys[i]; let zi = zs[i];
        let mut fx = 0.0f32;
        let mut fy = 0.0f32;
        let mut fz = 0.0f32;

        grid.for_each_neighbor(cell_of[i], |j| {
            if j == i { return; }
            // Vector de j a i (sign-correct para LJ).
            let dx = xi - xs[j];
            let dy = yi - ys[j];
            let dz = zi - zs[j];
            let r2 = dx * dx + dy * dy + dz * dz;
            if r2 > cutoff2 || r2 < 1.0e-12 { return; }
            let inv_r2 = 1.0 / r2;
            let sr2  = sigma2 * inv_r2;
            let sr6  = sr2 * sr2 * sr2;
            let sr12 = sr6 * sr6;
            let f_over_r = 24.0 * eps * inv_r2 * (2.0 * sr12 - sr6);
            fx += f_over_r * dx;
            fy += f_over_r * dy;
            fz += f_over_r * dz;
        });

        let m = masses[i];
        let inv_m = if m > 0.0 { 1.0 / m } else { 0.0 };
        // SAFETY: índice `i` único por iteración; nadie más escribe aquí.
        unsafe {
            *ax_p.0.add(i) += fx * inv_m;
            *ay_p.0.add(i) += fy * inv_m;
            *az_p.0.add(i) += fz * inv_m;
        }
    });
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    fn rebuild_grid(world: &World, cell_size: f32, dims: [u32; 3]) -> Grid3D {
        let mut g = Grid3D::new([-50.0; 3], cell_size, dims, world.len());
        g.rebuild(world);
        g
    }

    #[test]
    fn pair_at_minimum_has_near_zero_force() {
        // r_min = σ · 2^(1/6) ≈ 1.122462 σ
        let r_min = 1.122_462_05_f32;
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0], [0.; 3], 1.0, 0.0);
        w.spawn([r_min, 0.0, 0.0], [0.; 3], 1.0, 0.0);
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        lennard_jones(&mut w, &g, &LjParams { epsilon: 1.0, sigma: 1.0, cutoff: 2.5 });
        // |F| debería ser O(1e-4) o menor por errores numéricos de r_min.
        let fmag = (w.axs.0[0].powi(2) + w.ays.0[0].powi(2) + w.azs.0[0].powi(2)).sqrt();
        assert!(fmag < 1e-3, "F en el mínimo no es despreciable: {}", fmag);
    }

    #[test]
    fn pair_close_is_repulsive() {
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0],   [0.; 3], 1.0, 0.0);
        w.spawn([0.9, 0.0, 0.0],   [0.; 3], 1.0, 0.0); // r < σ
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        lennard_jones(&mut w, &g, &LjParams { epsilon: 1.0, sigma: 1.0, cutoff: 2.5 });
        // i=0 debe ser empujado a -x (alejándose de i=1 que está a +x).
        assert!(w.axs.0[0] < 0.0, "F_0_x debería ser negativo: {}", w.axs.0[0]);
        // Por Newton-3, i=1 debe ser empujado a +x.
        assert!(w.axs.0[1] > 0.0, "F_1_x debería ser positivo: {}", w.axs.0[1]);
        // Magnitudes ~ iguales (double-counting con par perfecto).
        assert!((w.axs.0[0] + w.axs.0[1]).abs() < 1e-3, "Newton-3 roto: F0+F1 = {}", w.axs.0[0] + w.axs.0[1]);
    }

    #[test]
    fn pair_beyond_cutoff_has_zero_force() {
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0], [0.; 3], 1.0, 0.0);
        w.spawn([2.6, 0.0, 0.0], [0.; 3], 1.0, 0.0); // r > 2.5
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        lennard_jones(&mut w, &g, &LjParams { epsilon: 1.0, sigma: 1.0, cutoff: 2.5 });
        assert_eq!(w.axs.0[0], 0.0);
        assert_eq!(w.axs.0[1], 0.0);
    }

    #[test]
    fn momentum_is_conserved_in_random_cloud() {
        // Newton-3 ⇒ Σ F_i = 0 (porque cada interacción i↔j contribuye F y −F).
        // Acumulamos m_i · a_i en lugar de a_i para descontar la división por m.
        let mut w = World::with_capacity(64);
        // Spawn determinista (sin RNG): grilla 4×4×4 dentro del cutoff.
        for k in 0..4 { for j in 0..4 { for i in 0..4 {
            w.spawn(
                [i as f32 * 1.1, j as f32 * 1.1, k as f32 * 1.1],
                [0.; 3], 1.0, 0.0,
            );
        }}}
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        lennard_jones(&mut w, &g, &LjParams { epsilon: 1.0, sigma: 1.0, cutoff: 2.5 });
        let mut sx = 0.0f64; let mut sy = 0.0f64; let mut sz = 0.0f64;
        for i in 0..w.len() {
            sx += w.axs.0[i] as f64 * w.masses.0[i] as f64;
            sy += w.ays.0[i] as f64 * w.masses.0[i] as f64;
            sz += w.azs.0[i] as f64 * w.masses.0[i] as f64;
        }
        let drift = (sx*sx + sy*sy + sz*sz).sqrt();
        assert!(drift < 1e-2, "momentum drift = {} (esperado ~0 por Newton-3)", drift);
    }
}
