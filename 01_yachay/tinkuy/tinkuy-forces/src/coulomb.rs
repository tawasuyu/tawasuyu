//! Coulomb electrostática con cutoff.
//!
//! F_i = k_e · q_i · q_j / r² · r̂_{ij}
//!     = k_e · q_i · q_j · (r_i − r_j) / r³
//!
//! Requiere `sqrt` (potencia r³ impar, no se evita con álgebra). Mismo patrón
//! que [`super::lennard_jones`]: paralelo por partícula, acumulación `+=`,
//! double-counting (sin atómicos).
//!
//! Unidades: `ke` se pasa explícito para que el caller decida sistema (Gauss,
//! SI, atómicas). No hay default mágico.

#[cfg(feature = "cpu")]
use rayon::prelude::*;
use tinkuy_core::{Grid3D, World};

pub struct CoulombParams {
    /// Constante de Coulomb en las unidades elegidas por el caller.
    pub ke: f32,
    pub cutoff: f32,
}

#[cfg(feature = "cpu")]
#[derive(Copy, Clone)]
#[repr(transparent)]
struct SyncPtr(*mut f32);
#[cfg(feature = "cpu")]
unsafe impl Send for SyncPtr {}
#[cfg(feature = "cpu")]
unsafe impl Sync for SyncPtr {}

#[cfg(feature = "cpu")]
pub fn coulomb(world: &mut World, grid: &Grid3D, p: &CoulombParams) {
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
    let masses  = world.masses.0.as_slice();
    let charges = world.charges.0.as_slice();
    let cell_of = grid.cell_of.as_slice();

    let ax_p = SyncPtr(world.axs.0.as_mut_ptr());
    let ay_p = SyncPtr(world.ays.0.as_mut_ptr());
    let az_p = SyncPtr(world.azs.0.as_mut_ptr());

    let cutoff2 = p.cutoff * p.cutoff;
    let ke = p.ke;

    (0..n).into_par_iter().with_min_len(64).for_each(|i| {
        let _ = (&ax_p, &ay_p, &az_p);

        let xi = xs[i]; let yi = ys[i]; let zi = zs[i];
        let qi = charges[i];
        if qi == 0.0 { return; } // partícula neutra: no acumula nada por aquí

        let mut fx = 0.0f32;
        let mut fy = 0.0f32;
        let mut fz = 0.0f32;

        grid.for_each_neighbor(cell_of[i], |j| {
            if j == i { return; }
            let qj = charges[j];
            if qj == 0.0 { return; }
            let dx = xi - xs[j];
            let dy = yi - ys[j];
            let dz = zi - zs[j];
            let r2 = dx * dx + dy * dy + dz * dz;
            if r2 > cutoff2 || r2 < 1.0e-12 { return; }
            // F_over_r = ke · qi · qj / r³ ; vector F = F_over_r · (r_i − r_j)
            let inv_r = 1.0 / r2.sqrt();
            let inv_r3 = inv_r * inv_r * inv_r;
            let k = ke * qi * qj * inv_r3;
            fx += k * dx;
            fy += k * dy;
            fz += k * dz;
        });

        let m = masses[i];
        let inv_m = if m > 0.0 { 1.0 / m } else { 0.0 };
        unsafe {
            *ax_p.0.add(i) += fx * inv_m;
            *ay_p.0.add(i) += fy * inv_m;
            *az_p.0.add(i) += fz * inv_m;
        }
    });
}

// Variante single-thread para Wawa / wasm32. Misma semántica que la rama `cpu`.
#[cfg(all(feature = "wasm", not(feature = "cpu")))]
pub fn coulomb(world: &mut World, grid: &Grid3D, p: &CoulombParams) {
    let n = world.len();
    if n == 0 { return; }
    debug_assert!(
        grid.cell_size >= p.cutoff,
        "cell_size ({}) < cutoff ({}) — vecinos posiblemente perdidos",
        grid.cell_size, p.cutoff
    );

    let cutoff2 = p.cutoff * p.cutoff;
    let ke = p.ke;

    for i in 0..n {
        let qi = world.charges.0[i];
        if qi == 0.0 { continue; }
        let xi = world.xs.0[i];
        let yi = world.ys.0[i];
        let zi = world.zs.0[i];
        let cell_i = grid.cell_of[i];
        let mut fx = 0.0f32;
        let mut fy = 0.0f32;
        let mut fz = 0.0f32;
        grid.for_each_neighbor(cell_i, |j| {
            if j == i { return; }
            let qj = world.charges.0[j];
            if qj == 0.0 { return; }
            let dx = xi - world.xs.0[j];
            let dy = yi - world.ys.0[j];
            let dz = zi - world.zs.0[j];
            let r2 = dx * dx + dy * dy + dz * dz;
            if r2 > cutoff2 || r2 < 1.0e-12 { return; }
            let inv_r = 1.0 / r2.sqrt();
            let inv_r3 = inv_r * inv_r * inv_r;
            let k = ke * qi * qj * inv_r3;
            fx += k * dx;
            fy += k * dy;
            fz += k * dz;
        });
        let m = world.masses.0[i];
        let inv_m = if m > 0.0 { 1.0 / m } else { 0.0 };
        world.axs.0[i] += fx * inv_m;
        world.ays.0[i] += fy * inv_m;
        world.azs.0[i] += fz * inv_m;
    }
}

#[cfg(all(test, any(feature = "cpu", feature = "wasm")))]
mod tests {
    use super::*;
    use crate::lennard_jones::clear_accelerations;

    fn rebuild_grid(world: &World, cell_size: f32, dims: [u32; 3]) -> Grid3D {
        let mut g = Grid3D::new([-50.0; 3], cell_size, dims, world.len());
        g.rebuild(world);
        g
    }

    #[test]
    fn opposite_charges_attract() {
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0], [0.; 3], 1.0,  1.0);  // +
        w.spawn([2.0, 0.0, 0.0], [0.; 3], 1.0, -1.0);  // −
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        coulomb(&mut w, &g, &CoulombParams { ke: 1.0, cutoff: 2.5 });
        // i=0 (+) sentirse atraído hacia i=1 (en +x) ⇒ a_x[0] > 0.
        assert!(w.axs.0[0] > 0.0, "atracción rota: a_0_x = {}", w.axs.0[0]);
        assert!(w.axs.0[1] < 0.0, "atracción rota: a_1_x = {}", w.axs.0[1]);
    }

    #[test]
    fn same_charges_repel() {
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0], [0.; 3], 1.0, 1.0);
        w.spawn([2.0, 0.0, 0.0], [0.; 3], 1.0, 1.0);
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        coulomb(&mut w, &g, &CoulombParams { ke: 1.0, cutoff: 2.5 });
        assert!(w.axs.0[0] < 0.0, "repulsión rota: a_0_x = {}", w.axs.0[0]);
        assert!(w.axs.0[1] > 0.0, "repulsión rota: a_1_x = {}", w.axs.0[1]);
    }

    #[test]
    fn coulomb_momentum_conserved() {
        // Plasma neutro: 32 cargas + y 32 cargas − colocadas en una grilla 4×4×4.
        let mut w = World::with_capacity(64);
        let mut idx = 0;
        for k in 0..4 { for j in 0..4 { for i in 0..4 {
            let q = if idx % 2 == 0 { 1.0 } else { -1.0 };
            w.spawn(
                [i as f32 * 1.2, j as f32 * 1.2, k as f32 * 1.2],
                [0.; 3], 1.0, q,
            );
            idx += 1;
        }}}
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        coulomb(&mut w, &g, &CoulombParams { ke: 1.0, cutoff: 2.5 });
        let mut sx = 0.0f64; let mut sy = 0.0f64; let mut sz = 0.0f64;
        for i in 0..w.len() {
            sx += w.axs.0[i] as f64 * w.masses.0[i] as f64;
            sy += w.ays.0[i] as f64 * w.masses.0[i] as f64;
            sz += w.azs.0[i] as f64 * w.masses.0[i] as f64;
        }
        let drift = (sx*sx + sy*sy + sz*sz).sqrt();
        assert!(drift < 1e-2, "momentum drift = {}", drift);
    }

    #[test]
    fn neutral_particle_feels_nothing() {
        let mut w = World::with_capacity(2);
        w.spawn([0.0, 0.0, 0.0], [0.; 3], 1.0, 0.0);  // neutra
        w.spawn([1.5, 0.0, 0.0], [0.; 3], 1.0, 1.0);  // cargada
        let g = rebuild_grid(&w, 3.0, [40, 40, 40]);
        clear_accelerations(&mut w);
        coulomb(&mut w, &g, &CoulombParams { ke: 1.0, cutoff: 2.5 });
        assert_eq!(w.axs.0[0], 0.0);
        assert_eq!(w.axs.0[1], 0.0); // y por simetría qi·qj=0
    }
}
