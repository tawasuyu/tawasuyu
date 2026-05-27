//! ECS minimalista, SoA puro, handle generacional.
//!
//! Las arrays calientes (posiciones, velocidades, aceleraciones, masa, carga)
//! están **alineadas a 64 B** y mantienen la misma longitud `len`. Cualquier
//! sistema (integrador, kernel de fuerzas) opera sobre slices `&[f32]`
//! paralelos sin punteros cruzados — caché-hit cercano al 100 % y SIMD
//! trivial sobre cada eje.

use bytemuck::{Pod, Zeroable};

/// Handle generacional. 8 B, `Copy`, hash trivial.
///
/// El campo `gen` se incrementa al despawnear/respawnear un slot, así que un
/// handle viejo se invalida automáticamente y `World::alive` lo detecta.
#[repr(C)]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug, Pod, Zeroable)]
pub struct EntityHandle {
    pub idx: u32,
    pub gen: u32,
}

/// Wrapper que alinea el **header** (struct Vec en sí) a 64 B, evitando
/// false-sharing entre headers de campos adyacentes.
///
/// **Limitación conocida**: NO alinea la allocation del heap que Vec apunta —
/// eso seguirá con la alineación natural del global allocator (`align_of::<T>`).
/// Para SIMD wide en B4 migraremos a `aligned-vec::AVec` (alineación de heap
/// configurable).
#[repr(C, align(64))]
pub struct Aligned64<T>(pub T);

pub struct World {
    // --- posición (SoA por eje) ---
    pub xs: Aligned64<Vec<f32>>,
    pub ys: Aligned64<Vec<f32>>,
    pub zs: Aligned64<Vec<f32>>,

    // --- velocidad ---
    pub vxs: Aligned64<Vec<f32>>,
    pub vys: Aligned64<Vec<f32>>,
    pub vzs: Aligned64<Vec<f32>>,

    // --- aceleración (recalculada cada step; doble buffer para diagnóstico) ---
    pub axs: Aligned64<Vec<f32>>,
    pub ays: Aligned64<Vec<f32>>,
    pub azs: Aligned64<Vec<f32>>,
    pub axs_prev: Aligned64<Vec<f32>>,
    pub ays_prev: Aligned64<Vec<f32>>,
    pub azs_prev: Aligned64<Vec<f32>>,

    // --- escalares por partícula ---
    pub masses:  Aligned64<Vec<f32>>,
    pub charges: Aligned64<Vec<f32>>,

    // --- generación por slot (para handle generacional) ---
    generations: Vec<u32>,
    free_slots:  Vec<u32>,

    len: usize,
}

impl World {
    pub fn with_capacity(cap: usize) -> Self {
        let cap = (cap + 15) & !15;
        let mk = || Aligned64(Vec::with_capacity(cap));
        Self {
            xs: mk(), ys: mk(), zs: mk(),
            vxs: mk(), vys: mk(), vzs: mk(),
            axs: mk(), ays: mk(), azs: mk(),
            axs_prev: mk(), ays_prev: mk(), azs_prev: mk(),
            masses: mk(), charges: mk(),
            generations: Vec::with_capacity(cap),
            free_slots:  Vec::new(),
            len: 0,
        }
    }

    #[inline] pub fn len(&self) -> usize { self.len }
    #[inline] pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn spawn(
        &mut self,
        pos: [f32; 3], vel: [f32; 3],
        mass: f32, charge: f32,
    ) -> EntityHandle {
        if let Some(idx) = self.free_slots.pop() {
            let i = idx as usize;
            self.xs.0[i] = pos[0]; self.ys.0[i] = pos[1]; self.zs.0[i] = pos[2];
            self.vxs.0[i] = vel[0]; self.vys.0[i] = vel[1]; self.vzs.0[i] = vel[2];
            self.axs.0[i] = 0.0; self.ays.0[i] = 0.0; self.azs.0[i] = 0.0;
            self.axs_prev.0[i] = 0.0; self.ays_prev.0[i] = 0.0; self.azs_prev.0[i] = 0.0;
            self.masses.0[i] = mass;
            self.charges.0[i] = charge;
            self.generations[i] = self.generations[i].wrapping_add(1);
            EntityHandle { idx, gen: self.generations[i] }
        } else {
            let idx = self.len as u32;
            self.xs.0.push(pos[0]); self.ys.0.push(pos[1]); self.zs.0.push(pos[2]);
            self.vxs.0.push(vel[0]); self.vys.0.push(vel[1]); self.vzs.0.push(vel[2]);
            self.axs.0.push(0.0); self.ays.0.push(0.0); self.azs.0.push(0.0);
            self.axs_prev.0.push(0.0); self.ays_prev.0.push(0.0); self.azs_prev.0.push(0.0);
            self.masses.0.push(mass);
            self.charges.0.push(charge);
            self.generations.push(1);
            self.len += 1;
            EntityHandle { idx, gen: 1 }
        }
    }

    #[inline]
    pub fn alive(&self, h: EntityHandle) -> bool {
        (h.idx as usize) < self.generations.len()
            && self.generations[h.idx as usize] == h.gen
    }

    pub fn despawn(&mut self, h: EntityHandle) {
        if !self.alive(h) { return; }
        let i = h.idx as usize;
        self.masses.0[i] = 0.0;
        self.charges.0[i] = 0.0;
        self.vxs.0[i] = 0.0; self.vys.0[i] = 0.0; self.vzs.0[i] = 0.0;
        self.free_slots.push(h.idx);
        self.generations[i] = self.generations[i].wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_assigns_distinct_handles() {
        let mut w = World::with_capacity(8);
        let a = w.spawn([1.0, 2.0, 3.0], [0.; 3], 1.0, -1.0);
        let b = w.spawn([4.0, 5.0, 6.0], [0.; 3], 2.0,  0.0);
        assert_ne!(a, b);
        assert_eq!(w.len(), 2);
        assert_eq!(w.xs.0[a.idx as usize], 1.0);
        assert_eq!(w.ys.0[b.idx as usize], 5.0);
        // Alineación del header del Vec (no del heap). El heap requiere
        // aligned-vec, que llegará en B4 junto con std::simd.
        let header_ptr = (&w.xs as *const _) as usize;
        assert_eq!(header_ptr & 63, 0, "header de xs no está alineado a 64 B");
    }

    #[test]
    fn despawn_recycles_slot_with_new_generation() {
        let mut w = World::with_capacity(4);
        let h = w.spawn([0.; 3], [0.; 3], 1.0, 0.0);
        w.despawn(h);
        assert!(!w.alive(h));
        let h2 = w.spawn([1.; 3], [0.; 3], 1.0, 0.0);
        assert_eq!(h.idx, h2.idx, "slot debe reciclarse");
        assert_ne!(h.gen, h2.gen, "generación debe avanzar");
        assert!(!w.alive(h));
        assert!( w.alive(h2));
    }
}
