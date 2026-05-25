//! Uniform grid para hit-test de nodos móviles.

use crate::buffers::NodeBuffer;
use std::collections::HashMap;

/// Grid de celdas cuadradas. `rebuild` lo repuebla; `query` busca el
/// nodo cuyo radio cubre un punto.
pub struct SpatialHash {
    cell: f32,
    map: HashMap<(i32, i32), Vec<usize>>,
}

impl SpatialHash {
    /// `cell_size` conviene ~2× el radio típico de nodo.
    pub fn new(cell_size: f32) -> Self {
        Self { cell: cell_size.max(1.0), map: HashMap::new() }
    }

    fn cell_of(&self, x: f32, y: f32) -> (i32, i32) {
        ((x / self.cell).floor() as i32, (y / self.cell).floor() as i32)
    }

    /// Repuebla el grid con las posiciones actuales de los nodos.
    pub fn rebuild(&mut self, nodes: &NodeBuffer) {
        self.map.clear();
        for i in 0..nodes.len() {
            let (x, y) = nodes.pos(i);
            self.map.entry(self.cell_of(x, y)).or_default().push(i);
        }
    }

    /// Devuelve el nodo más cercano a `(x,y)` cuyo radio lo cubre, o
    /// `None`. Revisa la celda del punto y sus 8 vecinas.
    pub fn query(&self, nodes: &NodeBuffer, x: f32, y: f32) -> Option<usize> {
        let (cx, cy) = self.cell_of(x, y);
        let mut best: Option<(usize, f32)> = None;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(bucket) = self.map.get(&(cx + dx, cy + dy)) {
                    for &i in bucket {
                        let (nx, ny) = nodes.pos(i);
                        let r = nodes.radius(i);
                        let d2 = (nx - x).powi(2) + (ny - y).powi(2);
                        if d2 <= r * r && best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
                            best = Some((i, d2));
                        }
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_hits_node_under_point() {
        let mut nb = NodeBuffer::new();
        nb.push(10.0, 10.0, 5.0);
        nb.push(100.0, 100.0, 8.0);
        let mut sh = SpatialHash::new(20.0);
        sh.rebuild(&nb);
        assert_eq!(sh.query(&nb, 12.0, 11.0), Some(0));
        assert_eq!(sh.query(&nb, 103.0, 98.0), Some(1));
    }

    #[test]
    fn query_misses_empty_space() {
        let mut nb = NodeBuffer::new();
        nb.push(10.0, 10.0, 5.0);
        let mut sh = SpatialHash::new(20.0);
        sh.rebuild(&nb);
        assert_eq!(sh.query(&nb, 500.0, 500.0), None);
        // fuera del radio pero misma celda
        assert_eq!(sh.query(&nb, 18.0, 18.0), None);
    }
}
