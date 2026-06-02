//! Escenarios iniciales del mundo — condiciones de borde de la simulación.
//!
//! Unifica el setup que vivía duplicado en `tinkuy-sim` (binario CLI) y
//! `tinkuy-llimphi` (app gráfica): mismo lattice cúbico, mismas velocidades
//! térmicas, misma resta de drift del centro de masas y misma grilla de
//! celdas (regla #2 — el dominio vive en el core, los frontends sólo lo
//! parametrizan). El PRNG `SplitMix64` también estaba calcado en ambos.

use crate::observables::total_momentum;
use crate::{Grid3D, World};

/// PRNG SplitMix64 — determinista, reproducible, sin dependencias. Mismo
/// stream que usaban sim y llimphi para sembrar velocidades térmicas.
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniforme en [-1, 1].
    pub fn next_centered(&mut self) -> f32 {
        let bits = self.next_u64();
        (bits as i64 as f64 / i64::MAX as f64) as f32
    }
}

/// Siembra un lattice cúbico de `side³` partículas separadas `spacing` en
/// cada eje, con velocidades térmicas `U(-1,1)·√temp_init`, el drift del
/// centro de masas removido (Σp = 0 al arranque, sin rotación espuria por
/// sesgo del RNG), y una grilla de celdas de lado `cutoff` (que garantiza
/// que los vecinos quepan en 27 celdas). Devuelve `(World, Grid3D,
/// bounds_min, bounds_max)`.
///
/// El margen `+ cutoff` en el lado de la caja evita self-overlap en el
/// borde periódico. Es el setup canónico de tinkuy — antes duplicado en
/// `tinkuy-sim::init_world` y `tinkuy-llimphi::init_world`.
pub fn lattice_cubica(
    side: usize,
    spacing: f32,
    cutoff: f32,
    seed: u64,
    temp_init: f32,
) -> (World, Grid3D, [f32; 3], [f32; 3]) {
    let n_actual = side * side * side;
    let l = side as f32 * spacing + cutoff; // margen para evitar self-overlap

    let bounds_min = [0.0; 3];
    let bounds_max = [l, l, l];

    let mut w = World::with_capacity(n_actual);
    let mut rng = SplitMix64::new(seed);

    // velocidades térmicas ~ U(-1,1) escaladas para que ⟨KE⟩ ≈ (3/2)N·kB·T0.
    // En unidades reducidas, σ_v = √T0.
    let vscale = temp_init.sqrt();

    let half = spacing * 0.5;
    for k in 0..side {
        for j in 0..side {
            for i in 0..side {
                let x = i as f32 * spacing + half + (cutoff * 0.5);
                let y = j as f32 * spacing + half + (cutoff * 0.5);
                let z = k as f32 * spacing + half + (cutoff * 0.5);
                let vx = rng.next_centered() * vscale;
                let vy = rng.next_centered() * vscale;
                let vz = rng.next_centered() * vscale;
                w.spawn([x, y, z], [vx, vy, vz], 1.0, 0.0);
            }
        }
    }

    // Sustrae drift del centro de masas: garantiza Σp = 0 (sin rotación
    // espuria del sistema entero por sesgo del RNG).
    let [px, py, pz] = total_momentum(&w);
    let m_total = n_actual as f64; // todas masa 1.0
    let dvx = (px / m_total) as f32;
    let dvy = (py / m_total) as f32;
    let dvz = (pz / m_total) as f32;
    for i in 0..n_actual {
        w.vxs.0[i] -= dvx;
        w.vys.0[i] -= dvy;
        w.vzs.0[i] -= dvz;
    }

    // Grilla: cell_size = cutoff garantiza que vecinos quepan en 27 celdas.
    let dims_x = ((l / cutoff).ceil() as u32).max(3);
    let mut g = Grid3D::new(bounds_min, cutoff, [dims_x; 3], n_actual);
    g.rebuild(&w);

    (w, g, bounds_min, bounds_max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_centrado_en_rango() {
        let mut rng = SplitMix64::new(0xC0FFEE);
        for _ in 0..10_000 {
            let v = rng.next_centered();
            assert!((-1.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn lattice_cuenta_y_momento_cero() {
        let side = 4;
        let (w, _g, bmin, bmax) = lattice_cubica(side, 1.5, 2.5, 0xC0FFEE, 0.5);
        assert_eq!(w.len(), side * side * side);
        // Σp ≈ 0 tras restar el drift del CM.
        let [px, py, pz] = total_momentum(&w);
        assert!(px.abs() < 1e-3 && py.abs() < 1e-3 && pz.abs() < 1e-3);
        // La caja arranca en el origen y es cúbica.
        assert_eq!(bmin, [0.0; 3]);
        assert!(bmax[0] == bmax[1] && bmax[1] == bmax[2] && bmax[0] > 0.0);
    }

    #[test]
    fn lattice_es_determinista_por_seed() {
        let a = lattice_cubica(3, 1.5, 2.5, 42, 0.5).0;
        let b = lattice_cubica(3, 1.5, 2.5, 42, 0.5).0;
        let c = lattice_cubica(3, 1.5, 2.5, 43, 0.5).0;
        assert_eq!(a.vxs.0, b.vxs.0, "misma seed ⇒ mismo estado");
        assert_ne!(a.vxs.0, c.vxs.0, "seed distinta ⇒ estado distinto");
    }
}
