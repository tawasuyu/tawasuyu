//! Snapshot del estado completo. Serializa SoA en bloque, hashea con BLAKE3
//! → CID que Akasha/AoE entienden nativamente.
//!
//! Formato del blob (little-endian):
//!   - u64  : n (número de partículas vivas)
//!   - n×f32 cada uno de: xs, ys, zs, vxs, vys, vzs, axs, ays, azs, masses, charges
//!
//! Determinismo: dos `World` con la misma secuencia de `spawn` y los mismos
//! valores numéricos producen el mismo CID, byte por byte.

use crate::ecs::World;

pub struct Snapshot {
    pub bytes: Vec<u8>,
    pub cid: [u8; 32],
}

impl Snapshot {
    pub fn capture(world: &World) -> Self {
        let n = world.len();
        let mut bytes = Vec::with_capacity(n * 11 * 4 + 8);
        bytes.extend_from_slice(&(n as u64).to_le_bytes());
        let push = |bytes: &mut Vec<u8>, arr: &[f32]| {
            bytes.extend_from_slice(bytemuck::cast_slice(&arr[..n]));
        };
        push(&mut bytes, &world.xs.0);   push(&mut bytes, &world.ys.0);   push(&mut bytes, &world.zs.0);
        push(&mut bytes, &world.vxs.0);  push(&mut bytes, &world.vys.0);  push(&mut bytes, &world.vzs.0);
        push(&mut bytes, &world.axs.0);  push(&mut bytes, &world.ays.0);  push(&mut bytes, &world.azs.0);
        push(&mut bytes, &world.masses.0);
        push(&mut bytes, &world.charges.0);
        let cid = blake3::hash(&bytes).into();
        Self { bytes, cid }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_world_produces_same_cid() {
        let mut a = World::with_capacity(4);
        let mut b = World::with_capacity(4);
        for w in [&mut a, &mut b] {
            w.spawn([1.0, 2.0, 3.0], [0.1, 0.2, 0.3], 1.0,  1.0);
            w.spawn([4.0, 5.0, 6.0], [0.0, 0.0, 0.0], 2.0, -1.0);
        }
        assert_eq!(Snapshot::capture(&a).cid, Snapshot::capture(&b).cid);
    }

    #[test]
    fn perturbed_world_produces_different_cid() {
        let mut a = World::with_capacity(2);
        let mut b = World::with_capacity(2);
        // Perturbación mayor que el epsilon f32 a 3.0 (~9.5e-7); si fuera
        // menor, ambos valores redondearían al mismo bit-pattern y el CID
        // coincidiría — efecto deseable y testeable aparte.
        a.spawn([1.0, 2.0, 3.0],   [0.; 3], 1.0, 0.0);
        b.spawn([1.0, 2.0, 3.001], [0.; 3], 1.0, 0.0);
        assert_ne!(Snapshot::capture(&a).cid, Snapshot::capture(&b).cid);
    }
}
