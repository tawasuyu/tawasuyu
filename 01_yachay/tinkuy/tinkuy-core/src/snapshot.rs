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

/// Error de [`Snapshot::restore_into`]. Sólo dos casos: header faltante o
/// payload incoherente con el `n` declarado en el header.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreError {
    /// Menos de 8 bytes en el buffer — header `u64 n` no cabe.
    HeaderFaltante,
    /// El payload no mide exactamente `n × 11 × 4` bytes, con `n` el
    /// header. Diferencia en cualquier dirección abrota.
    PayloadIncoherente { esperado: usize, encontrado: usize },
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

    /// Inverso de [`Snapshot::capture`]: parsea `bytes` y vuelca el estado en
    /// `world`. Las arrays SoA quedan exactamente del largo `n` del header.
    /// `ax_prev`/`ay_prev`/`az_prev` se zeran (no viajan en el snapshot;
    /// el paso siguiente de Velocity-Verlet usará `a = 0` para el half-step
    /// — equivalente a haber recapturado el estado desde cero).
    ///
    /// Tras restaurar, `Snapshot::capture(world).cid` es bit-idéntico al
    /// CID del snapshot original — useful para rewind reproducible.
    pub fn restore_into(bytes: &[u8], world: &mut World) -> Result<(), RestoreError> {
        if bytes.len() < 8 {
            return Err(RestoreError::HeaderFaltante);
        }
        let mut n_buf = [0u8; 8];
        n_buf.copy_from_slice(&bytes[..8]);
        let n = u64::from_le_bytes(n_buf) as usize;
        let esperado = n * 11 * 4 + 8;
        if bytes.len() != esperado {
            return Err(RestoreError::PayloadIncoherente {
                esperado,
                encontrado: bytes.len(),
            });
        }
        // 11 bloques de f32 contiguos tras el header.
        let payload: &[f32] = bytemuck::cast_slice(&bytes[8..]);
        let mut off = 0usize;
        let mut take = |dst: &mut Vec<f32>| {
            let slice = &payload[off..off + n];
            dst.clear();
            dst.extend_from_slice(slice);
            off += n;
        };
        take(&mut world.xs.0);   take(&mut world.ys.0);   take(&mut world.zs.0);
        take(&mut world.vxs.0);  take(&mut world.vys.0);  take(&mut world.vzs.0);
        take(&mut world.axs.0);  take(&mut world.ays.0);  take(&mut world.azs.0);
        take(&mut world.masses.0);
        take(&mut world.charges.0);
        // ax_prev no viajan en el snapshot; quedan en cero del largo correcto.
        let zero = |dst: &mut Vec<f32>| {
            dst.clear();
            dst.resize(n, 0.0);
        };
        zero(&mut world.axs_prev.0);
        zero(&mut world.ays_prev.0);
        zero(&mut world.azs_prev.0);
        world.set_len_for_restore(n);
        Ok(())
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
    fn restore_into_round_trip_idempotente() {
        let mut a = World::with_capacity(8);
        a.spawn([1.0, 2.0, 3.0], [0.1, -0.2, 0.3], 1.0,  1.0);
        a.spawn([4.0, 5.0, 6.0], [0.0,  0.0, 0.0], 2.0, -1.0);
        a.spawn([7.5, 8.0, 9.0], [-0.4, 0.5, 0.6], 3.0,  0.5);
        // Snapshot inicial.
        let snap = Snapshot::capture(&a);
        let cid_orig = snap.cid;
        // Mutamos el mundo (avanzamos posiciones ficticiamente).
        for i in 0..a.len() {
            a.xs.0[i] += 100.0;
            a.vys.0[i] -= 1.0;
        }
        // Restauramos a otra arena (b) y a la misma (a).
        let mut b = World::with_capacity(8);
        Snapshot::restore_into(&snap.bytes, &mut b).unwrap();
        Snapshot::restore_into(&snap.bytes, &mut a).unwrap();
        assert_eq!(Snapshot::capture(&a).cid, cid_orig);
        assert_eq!(Snapshot::capture(&b).cid, cid_orig);
    }

    #[test]
    fn restore_into_header_faltante() {
        let mut w = World::with_capacity(2);
        let err = Snapshot::restore_into(&[0u8; 4], &mut w).unwrap_err();
        assert_eq!(err, RestoreError::HeaderFaltante);
    }

    #[test]
    fn restore_into_payload_incoherente() {
        let mut w = World::with_capacity(2);
        // Header dice n=3 → esperado = 3*44 + 8 = 140 bytes. Pasamos sólo 8 + 4.
        let mut buf = Vec::new();
        buf.extend_from_slice(&3u64.to_le_bytes());
        buf.extend_from_slice(&[0u8; 4]);
        let err = Snapshot::restore_into(&buf, &mut w).unwrap_err();
        match err {
            RestoreError::PayloadIncoherente { esperado, encontrado } => {
                assert_eq!(esperado, 3 * 11 * 4 + 8);
                assert_eq!(encontrado, 12);
            }
            other => panic!("error inesperado: {:?}", other),
        }
    }

    #[test]
    fn restore_into_zera_ax_prev() {
        let mut w = World::with_capacity(4);
        w.spawn([1.0; 3], [0.0; 3], 1.0, 0.0);
        // Simulamos un step previo dejando ax_prev distinto de cero.
        w.axs_prev.0[0] = 7.0;
        let snap = Snapshot::capture(&w);
        // Antes de restaurar, ensuciamos ax_prev de nuevo.
        w.axs_prev.0[0] = 99.0;
        Snapshot::restore_into(&snap.bytes, &mut w).unwrap();
        assert_eq!(w.axs_prev.0[0], 0.0);
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
