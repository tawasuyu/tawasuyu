//! Grilla regular 3D + transferencia lock-free entre celdas.
//!
//! La transferencia **no** usa colas atómicas. Patrón:
//!   1. Cada worker (rayon) recibe un rango disjunto de partículas. Mientras
//!      integra, escribe en su `Outbox` local (sin sincronización).
//!   2. Tras la barrera implícita del `par_iter`, `merge_transfers` aplica
//!      todos los outboxes en orden determinista: outbox[0], outbox[1], …
//!   3. La asignación entidad→celda se reescribe in-place sobre listas
//!      intrusivas (heads/next), O(1) por inserción/eliminación.

use crate::ecs::World;

/// Coordenada lineal de celda en la grilla `(i + j*nx + k*nx*ny)`.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct CellId(pub u32);

/// Mensaje de transferencia: la entidad `entity_idx` ya no pertenece a su
/// celda actual y debe reubicarse en `dst`. Se aplica en `merge_transfers`.
#[derive(Copy, Clone, Debug)]
pub struct Transfer {
    pub entity_idx: u32,
    pub dst: CellId,
}

/// Outbox worker-local. **Sin atómicos, sin locks**. Solo el worker que la
/// posee la escribe; el merge la consume en serie.
#[derive(Default, Debug, Clone)]
pub struct Outbox(pub Vec<Transfer>);

impl Outbox {
    pub fn with_capacity(cap: usize) -> Self { Self(Vec::with_capacity(cap)) }
    #[inline] pub fn push(&mut self, t: Transfer) { self.0.push(t); }
    #[inline] pub fn clear(&mut self) { self.0.clear(); }
    #[inline] pub fn len(&self) -> usize { self.0.len() }
    #[inline] pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

pub struct Grid3D {
    pub origin: [f32; 3],
    pub cell_size: f32,
    pub dims: [u32; 3], // (nx, ny, nz)
    /// Cabeza de lista intrusiva: índice de la primera entidad en cada celda
    /// (`u32::MAX` = vacía). Coste de inserción/eliminación: O(1).
    pub heads: Vec<u32>,
    /// Próximo nodo en la lista intrusiva por entidad.
    pub next: Vec<u32>,
    /// Celda actual de cada entidad (cache de lookup espacial).
    pub cell_of: Vec<CellId>,
}

impl Grid3D {
    pub fn new(origin: [f32; 3], cell_size: f32, dims: [u32; 3], capacity: usize) -> Self {
        assert!(cell_size > 0.0, "cell_size debe ser > 0");
        assert!(dims[0] > 0 && dims[1] > 0 && dims[2] > 0, "dims > 0");
        let total = (dims[0] as usize) * (dims[1] as usize) * (dims[2] as usize);
        Self {
            origin,
            cell_size,
            dims,
            heads: vec![u32::MAX; total],
            next: vec![u32::MAX; capacity],
            cell_of: vec![CellId(0); capacity],
        }
    }

    #[inline]
    pub fn total_cells(&self) -> usize {
        (self.dims[0] as usize) * (self.dims[1] as usize) * (self.dims[2] as usize)
    }

    /// Mapea una posición del mundo a la celda que la contiene, clamping
    /// contra los bordes de la grilla.
    #[inline]
    pub fn cell_of_pos(&self, x: f32, y: f32, z: f32) -> CellId {
        let inv = 1.0 / self.cell_size;
        let i = (((x - self.origin[0]) * inv) as i32)
            .clamp(0, self.dims[0] as i32 - 1) as u32;
        let j = (((y - self.origin[1]) * inv) as i32)
            .clamp(0, self.dims[1] as i32 - 1) as u32;
        let k = (((z - self.origin[2]) * inv) as i32)
            .clamp(0, self.dims[2] as i32 - 1) as u32;
        CellId(i + j * self.dims[0] + k * self.dims[0] * self.dims[1])
    }

    /// Reconstruye listas intrusivas desde cero. O(N). Llamar tras `spawn`
    /// masivos o tras cargar un Snapshot.
    pub fn rebuild(&mut self, world: &World) {
        self.heads.fill(u32::MAX);
        if self.next.len() < world.len() {
            self.next.resize(world.len(), u32::MAX);
            self.cell_of.resize(world.len(), CellId(0));
        }
        for i in 0..world.len() {
            let c = self.cell_of_pos(world.xs.0[i], world.ys.0[i], world.zs.0[i]);
            self.cell_of[i] = c;
            self.next[i] = self.heads[c.0 as usize];
            self.heads[c.0 as usize] = i as u32;
        }
    }

    /// Aplica todos los outboxes en orden determinista. Llamar al final de
    /// cada substep.
    pub fn merge_transfers(&mut self, outboxes: &mut [Outbox]) {
        for ob in outboxes.iter_mut() {
            for t in ob.0.drain(..) {
                let i = t.entity_idx as usize;
                let old = self.cell_of[i];
                if old == t.dst { continue; }
                self.unlink(i, old);
                self.link(i, t.dst);
                self.cell_of[i] = t.dst;
            }
        }
    }

    #[inline]
    fn link(&mut self, entity_idx: usize, cell: CellId) {
        self.next[entity_idx] = self.heads[cell.0 as usize];
        self.heads[cell.0 as usize] = entity_idx as u32;
    }

    fn unlink(&mut self, entity_idx: usize, cell: CellId) {
        let head_slot = &mut self.heads[cell.0 as usize];
        if *head_slot == entity_idx as u32 {
            *head_slot = self.next[entity_idx];
            return;
        }
        let mut cur = *head_slot;
        while cur != u32::MAX {
            let nxt = self.next[cur as usize];
            if nxt == entity_idx as u32 {
                self.next[cur as usize] = self.next[entity_idx];
                return;
            }
            cur = nxt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_of_pos_clamps_and_indexes() {
        let g = Grid3D::new([0.0; 3], 1.0, [4, 4, 4], 0);
        assert_eq!(g.cell_of_pos(0.5, 0.5, 0.5).0, 0);
        assert_eq!(g.cell_of_pos(1.5, 0.5, 0.5).0, 1);
        assert_eq!(g.cell_of_pos(0.5, 1.5, 0.5).0, 4);
        assert_eq!(g.cell_of_pos(0.5, 0.5, 1.5).0, 16);
        // clamp
        assert_eq!(g.cell_of_pos(-9.0, 0.5, 0.5).0, 0);
        assert_eq!(g.cell_of_pos(99.0, 99.0, 99.0).0, 4 * 4 * 4 - 1);
    }

    #[test]
    fn rebuild_then_merge_transfer_preserves_membership() {
        let mut w = World::with_capacity(2);
        let _a = w.spawn([0.5, 0.5, 0.5], [0.; 3], 1.0, 0.0); // celda 0
        let _b = w.spawn([1.5, 0.5, 0.5], [0.; 3], 1.0, 0.0); // celda 1

        let mut g = Grid3D::new([0.; 3], 1.0, [4, 4, 4], w.len());
        g.rebuild(&w);
        assert_eq!(g.cell_of[0], CellId(0));
        assert_eq!(g.cell_of[1], CellId(1));
        assert_eq!(g.heads[0], 0);
        assert_eq!(g.heads[1], 1);

        // mueve la entidad 0 a la celda 5 (1,1,0) vía outbox
        let mut outs = vec![Outbox::default(), Outbox::default()];
        outs[0].push(Transfer { entity_idx: 0, dst: CellId(5) });
        g.merge_transfers(&mut outs);
        assert_eq!(g.cell_of[0], CellId(5));
        assert_eq!(g.heads[0], u32::MAX, "celda 0 debe quedar vacía");
        assert_eq!(g.heads[5], 0, "celda 5 debe tener a la entidad 0");
        assert!(outs[0].is_empty(), "outbox debe haberse drenado");
    }
}
