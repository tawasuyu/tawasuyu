//! Velocity-Verlet paralelo por rangos de partículas.
//!
//! Esquema simpléctico (conserva energía a largo plazo):
//!   1. v(t + dt/2) = v(t) + a(t) · dt/2     ← medio "kick"
//!   2. x(t + dt)   = x(t) + v(t + dt/2) · dt  ← "drift"
//!   3. a(t + dt)   = F(x(t + dt)) / m         ← kernel de fuerzas (externo)
//!   4. v(t + dt)   = v(t + dt/2) + a(t + dt) · dt/2  ← segundo medio kick
//!
//! Pasos 1+2 viven en [`kick_drift`], paso 4 en [`finish_kick`]. El paso 3 lo
//! provee el caller (futuro crate `tinkuy-forces`).

use crate::ecs::World;
use crate::grid::{Grid3D, Outbox, Transfer};

pub struct IntegratorParams {
    pub dt: f32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

/// Newtype `Send + Sync` sobre `*mut f32`. Necesario porque, con disjoint
/// capture (Rust 2021), el closure paralelo captura cada *campo* del struct
/// `Ptrs` por separado, no el struct entero — así que el `Sync` debe vivir en
/// el tipo del campo.
#[derive(Copy, Clone)]
#[repr(transparent)]
struct SyncPtr(*mut f32);
// SAFETY: cada worker accede solo a su rango disjunto [w*chunk, (w+1)*chunk);
// no hay aliasing mutable simultáneo entre workers. Es el mismo patrón
// interno de `rayon::slice::ParallelSliceMut`.
unsafe impl Send for SyncPtr {}
unsafe impl Sync for SyncPtr {}

#[derive(Copy, Clone)]
struct Ptrs {
    x:  SyncPtr, y:  SyncPtr, z:  SyncPtr,
    vx: SyncPtr, vy: SyncPtr, vz: SyncPtr,
}

/// Pasos 1+2 de Velocity-Verlet. Paralelo por **rango disjunto de partículas**;
/// emite transferencias entre celdas a través de outboxes worker-local.
///
/// Precondición: `outboxes.len() >= 1`. En la práctica use uno por hilo de
/// rayon (`rayon::current_num_threads()`). Los outboxes se vacían dentro de
/// `Grid3D::merge_transfers` que el caller debe invocar después.
#[cfg(feature = "cpu")]
pub fn kick_drift(
    world: &mut World,
    grid: &Grid3D,
    params: &IntegratorParams,
    outboxes: &mut [Outbox],
) {
    use rayon::prelude::*;

    let n = world.len();
    if n == 0 { return; }
    let n_workers = outboxes.len().max(1);
    let chunk = n.div_ceil(n_workers);
    let dt = params.dt;
    let half_dt = 0.5 * dt;

    // Cada worker escribe solo en `[w*chunk .. min(n, (w+1)*chunk))`.
    // Rangos disjuntos → sin aliasing mutable.
    let ptrs = Ptrs {
        x:  SyncPtr(world.xs.0.as_mut_ptr()),
        y:  SyncPtr(world.ys.0.as_mut_ptr()),
        z:  SyncPtr(world.zs.0.as_mut_ptr()),
        vx: SyncPtr(world.vxs.0.as_mut_ptr()),
        vy: SyncPtr(world.vys.0.as_mut_ptr()),
        vz: SyncPtr(world.vzs.0.as_mut_ptr()),
    };
    let axs = world.axs.0.as_slice();
    let ays = world.ays.0.as_slice();
    let azs = world.azs.0.as_slice();
    let cell_of = grid.cell_of.as_slice();

    outboxes.par_iter_mut().enumerate().for_each(|(w, ob)| {
        // Disjoint capture (Rust 2021): sin esto, el closure capturaría
        // `ptrs.x.0` (path profundo) como `*mut f32` no-Sync, en vez de `ptrs`
        // entero. Este `&ptrs` fija la ruta de captura al struct completo.
        let _ = &ptrs;
        let start = w * chunk;
        let end = (start + chunk).min(n);
        for i in start..end {
            // SAFETY: i ∈ [start, end) ⊂ [0, n) y este worker es el único
            // que escribe en este índice durante el par_iter.
            let (nx, ny, nz) = unsafe {
                let vx = *ptrs.vx.0.add(i) + axs[i] * half_dt;
                let vy = *ptrs.vy.0.add(i) + ays[i] * half_dt;
                let vz = *ptrs.vz.0.add(i) + azs[i] * half_dt;
                let nx = *ptrs.x.0.add(i) + vx * dt;
                let ny = *ptrs.y.0.add(i) + vy * dt;
                let nz = *ptrs.z.0.add(i) + vz * dt;
                *ptrs.vx.0.add(i) = vx;
                *ptrs.vy.0.add(i) = vy;
                *ptrs.vz.0.add(i) = vz;
                *ptrs.x.0.add(i)  = nx;
                *ptrs.y.0.add(i)  = ny;
                *ptrs.z.0.add(i)  = nz;
                (nx, ny, nz)
            };

            let new_cell = grid.cell_of_pos(nx, ny, nz);
            if new_cell != cell_of[i] {
                ob.push(Transfer { entity_idx: i as u32, dst: new_cell });
            }
        }
    });
}

/// Paso 4 de Velocity-Verlet: segundo medio kick con las aceleraciones nuevas.
/// Invocar DESPUÉS del kernel de fuerzas que rellenó `world.axs/ays/azs`.
#[cfg(feature = "cpu")]
pub fn finish_kick(world: &mut World, params: &IntegratorParams) {
    use rayon::prelude::*;
    let half_dt = 0.5 * params.dt;

    // Split-borrow: las tres parejas (vx,ax) (vy,ay) (vz,az) son campos
    // disjuntos del World; el borrow checker los acepta por separado.
    {
        let vxs = &mut world.vxs.0;
        let axs = &world.axs.0;
        vxs.par_iter_mut().zip(axs.par_iter()).for_each(|(v, a)| *v += a * half_dt);
    }
    {
        let vys = &mut world.vys.0;
        let ays = &world.ays.0;
        vys.par_iter_mut().zip(ays.par_iter()).for_each(|(v, a)| *v += a * half_dt);
    }
    {
        let vzs = &mut world.vzs.0;
        let azs = &world.azs.0;
        vzs.par_iter_mut().zip(azs.par_iter()).for_each(|(v, a)| *v += a * half_dt);
    }
}

/// Step completo de Velocity-Verlet. `compute_forces` recibe el `World` con
/// las posiciones ya integradas y debe rellenar `world.axs/ays/azs` con la
/// aceleración correspondiente a `x(t + dt)`.
pub fn velocity_verlet_step<F>(
    world: &mut World,
    grid: &mut Grid3D,
    params: &IntegratorParams,
    outboxes: &mut [Outbox],
    compute_forces: F,
) where
    F: FnOnce(&mut World, &Grid3D),
{
    // Guarda a(t) para diagnóstico/depuración (energía, temperatura).
    // Split borrow campo a campo: prev/cur son campos disjuntos del World.
    let n = world.len();
    {
        let (prev, cur) = (&mut world.axs_prev.0, &world.axs.0);
        prev[..n].copy_from_slice(&cur[..n]);
    }
    {
        let (prev, cur) = (&mut world.ays_prev.0, &world.ays.0);
        prev[..n].copy_from_slice(&cur[..n]);
    }
    {
        let (prev, cur) = (&mut world.azs_prev.0, &world.azs.0);
        prev[..n].copy_from_slice(&cur[..n]);
    }

    #[cfg(feature = "cpu")]
    {
        kick_drift(world, grid, params, outboxes);
        grid.merge_transfers(outboxes);
        compute_forces(world, grid);
        finish_kick(world, params);
    }
    #[cfg(not(feature = "cpu"))]
    {
        let _ = (world, grid, params, outboxes, compute_forces);
        unimplemented!("habilita feature `cpu` (rayon) o la futura `gpu`");
    }
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use crate::Snapshot;

    /// Caída libre: sin fuerzas, una partícula con velocidad constante
    /// recorre `v·dt` en cada step. Verifica la rama drift de Verlet.
    #[test]
    fn drift_only_advances_position_by_v_times_dt() {
        let mut w = World::with_capacity(1);
        w.spawn([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 1.0, 0.0);
        let mut g = Grid3D::new([-50.0; 3], 5.0, [20, 20, 20], w.len());
        g.rebuild(&w);
        let params = IntegratorParams {
            dt: 0.1, bounds_min: [-50.; 3], bounds_max: [50.; 3],
        };
        let mut outs = vec![Outbox::default(); 2];
        for _ in 0..10 {
            velocity_verlet_step(&mut w, &mut g, &params, &mut outs, |_, _| {});
        }
        assert!((w.xs.0[0] - 1.0).abs() < 1e-5, "x = {}", w.xs.0[0]);
        assert!(w.ys.0[0].abs() < 1e-6);
        assert!(w.zs.0[0].abs() < 1e-6);
    }

    /// Determinismo: dos corridas con la misma semilla de estado producen el
    /// mismo CID de Snapshot. Crítico para reproducibilidad y para que Wawa
    /// pueda cachear resultados por CID.
    #[test]
    fn run_is_deterministic_under_identical_inputs() {
        let make = || {
            let mut w = World::with_capacity(64);
            for i in 0..32 {
                let f = i as f32 * 0.1;
                w.spawn([f, f * 0.5, f * 0.25], [0.01, 0.0, 0.0], 1.0, 0.0);
            }
            w
        };
        let params = IntegratorParams {
            dt: 0.01, bounds_min: [-50.; 3], bounds_max: [50.; 3],
        };
        let run = |mut w: World| {
            let mut g = Grid3D::new([-50.; 3], 2.0, [50, 50, 50], w.len());
            g.rebuild(&w);
            let mut outs = vec![Outbox::default(); 4];
            for _ in 0..20 {
                velocity_verlet_step(&mut w, &mut g, &params, &mut outs, |_, _| {});
            }
            Snapshot::capture(&w).cid
        };
        assert_eq!(run(make()), run(make()));
    }
}
