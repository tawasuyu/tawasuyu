//! Topología del fractal: índice de hijos por lineage y orden topológico
//! para shutdown.

use super::EnteGraph;
use std::collections::BTreeSet;
use ulid::Ulid;

impl EnteGraph {
    /// DFS post-order desde la Semilla. Hojas primero, raíz al final.
    /// Garantiza que SIGTERM va a un padre sólo cuando sus hijos ya recibieron
    /// la señal (evita orfandad transitoria que confunda Restart supervisors).
    pub(in crate::graph) fn topo_order(&self) -> Vec<Ulid> {
        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        self.dfs_post(self.seed.id, &mut visited, &mut order);
        // Entes encarnados sin lineage hacia el seed (no debería pasar pero
        // protege contra grafos huérfanos): añadirlos al final.
        for id in self.incarnated.keys() {
            if !visited.contains(id) {
                self.dfs_post(*id, &mut visited, &mut order);
            }
        }
        order
    }

    fn dfs_post(&self, node: Ulid, visited: &mut BTreeSet<Ulid>, order: &mut Vec<Ulid>) {
        if !visited.insert(node) { return; }
        if let Some(children) = self.children.get(&node) {
            for c in children.clone() {
                self.dfs_post(c, visited, order);
            }
        }
        order.push(node);
    }
}
