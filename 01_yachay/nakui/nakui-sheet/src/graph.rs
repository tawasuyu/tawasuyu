//! Grafo de dependencias entre celdas, construido y mutado en
//! caliente. El manifiesto Nakui no enumera 10 000 morfismos; este
//! grafo se alimenta de la tabla viva de celdas y reacciona a cada
//! `set_cell`.
//!
//! Convención de aristas: `dep → cell` significa que `cell` depende
//! de `dep`. Caminar las aristas hacia adelante desde `D` da el
//! conjunto de celdas que se contaminan cuando `D` cambia.
//!
//! `set_deps` reemplaza atómicamente las dependencias de una celda
//! tras detectar que la actualización NO introduce un ciclo. Si el
//! check de ciclo falla, el grafo queda exactamente como estaba.

use crate::cell::CellRef;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("cycle: cell {target} depends on itself through {chain:?}")]
pub struct CycleError {
    pub target: CellRef,
    pub chain: Vec<CellRef>,
}

#[derive(Debug, Default)]
pub struct SheetGraph {
    g: DiGraph<CellRef, ()>,
    nodes: HashMap<CellRef, NodeIndex>,
}

impl SheetGraph {
    pub fn new() -> Self {
        Self::default()
    }

    fn node(&mut self, c: CellRef) -> NodeIndex {
        if let Some(&idx) = self.nodes.get(&c) {
            return idx;
        }
        let idx = self.g.add_node(c);
        self.nodes.insert(c, idx);
        idx
    }

    fn node_opt(&self, c: CellRef) -> Option<NodeIndex> {
        self.nodes.get(&c).copied()
    }

    /// Aristas entrantes a `c` actualmente registradas (sus dependencias).
    pub fn deps_of(&self, c: CellRef) -> Vec<CellRef> {
        match self.node_opt(c) {
            None => Vec::new(),
            Some(idx) => self
                .g
                .edges_directed(idx, petgraph::Direction::Incoming)
                .map(|e| self.g[e.source()])
                .collect(),
        }
    }

    /// Reemplaza el conjunto de dependencias de `cell`. Si la nueva
    /// configuración introduce un ciclo (alguna dep es alcanzable
    /// HACIA ADELANTE desde `cell`), devuelve `CycleError` y deja el
    /// grafo sin tocar.
    pub fn set_deps(
        &mut self,
        cell: CellRef,
        new_deps: &[CellRef],
    ) -> Result<(), CycleError> {
        // Quitamos auto-referencias antes de cualquier chequeo: una
        // celda no depende de sí misma "por error" — eso ya es un
        // ciclo de longitud 1 y lo señalamos como tal.
        for d in new_deps {
            if *d == cell {
                return Err(CycleError {
                    target: cell,
                    chain: vec![cell],
                });
            }
        }

        // Check anticipado de ciclo: hay ciclo si alguna nueva dep `d`
        // ya es alcanzable hacia adelante desde `cell` en el grafo
        // actual. (Una arista nueva `d → cell` cerraría el camino.)
        // Si `cell` aún no existe como nodo no puede haber predecesores
        // suyos, así que no puede crear ciclo.
        if let Some(cell_idx) = self.node_opt(cell) {
            let new_dep_idxs: HashSet<NodeIndex> = new_deps
                .iter()
                .filter_map(|d| self.node_opt(*d))
                .collect();
            if !new_dep_idxs.is_empty() {
                if let Some(chain) = self.path_forward(cell_idx, &new_dep_idxs) {
                    return Err(CycleError {
                        target: cell,
                        chain: chain.into_iter().map(|i| self.g[i]).collect(),
                    });
                }
            }
        }

        // Sin ciclos: aplicar. Borramos entrantes viejas, agregamos
        // las nuevas. Las nodes faltantes se crean.
        let cell_idx = self.node(cell);
        let incoming: Vec<_> = self
            .g
            .edges_directed(cell_idx, petgraph::Direction::Incoming)
            .map(|e| e.id())
            .collect();
        for e in incoming {
            self.g.remove_edge(e);
        }
        for d in new_deps {
            let d_idx = self.node(*d);
            self.g.add_edge(d_idx, cell_idx, ());
        }
        Ok(())
    }

    /// BFS hacia adelante desde `from` buscando cualquier `target`.
    /// Devuelve el camino `[from, ..., target]` si existe.
    fn path_forward(
        &self,
        from: NodeIndex,
        targets: &HashSet<NodeIndex>,
    ) -> Option<Vec<NodeIndex>> {
        let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        queue.push_back(from);
        let mut seen = HashSet::new();
        seen.insert(from);
        while let Some(n) = queue.pop_front() {
            if targets.contains(&n) && n != from {
                // Reconstruye el camino.
                let mut path = vec![n];
                let mut cur = n;
                while let Some(&p) = parents.get(&cur) {
                    path.push(p);
                    cur = p;
                    if cur == from {
                        break;
                    }
                }
                path.reverse();
                return Some(path);
            }
            for e in self.g.edges_directed(n, petgraph::Direction::Outgoing) {
                let t = e.target();
                if seen.insert(t) {
                    parents.insert(t, n);
                    queue.push_back(t);
                }
            }
        }
        None
    }

    /// Devuelve el conjunto de celdas alcanzables hacia adelante
    /// desde cualquier `seed` (incluyéndolas), en orden topológico.
    /// Esto es exactamente "lo que hay que recalcular" cuando los
    /// `seeds` se acaban de modificar.
    ///
    /// Solo recorremos el subgrafo afectado — si una hoja tiene un
    /// millón de celdas y cambia una, el coste es proporcional a las
    /// downstream, no a la hoja entera.
    pub fn downstream_topo(&self, seeds: &[CellRef]) -> Vec<CellRef> {
        // 1. BFS hacia adelante para recolectar el set.
        let mut set: HashSet<NodeIndex> = HashSet::new();
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        for s in seeds {
            if let Some(idx) = self.node_opt(*s) {
                if set.insert(idx) {
                    queue.push_back(idx);
                }
            }
        }
        while let Some(n) = queue.pop_front() {
            for e in self.g.edges_directed(n, petgraph::Direction::Outgoing) {
                let t = e.target();
                if set.insert(t) {
                    queue.push_back(t);
                }
            }
        }

        // 2. Kahn restringido al subset. Para cada nodo calculamos su
        //    in-degree DENTRO del subset (las dependencias externas no
        //    cuentan — sus valores ya están y no necesitan recálculo).
        let mut indeg: HashMap<NodeIndex, usize> = HashMap::new();
        for &n in &set {
            let d = self
                .g
                .edges_directed(n, petgraph::Direction::Incoming)
                .filter(|e| set.contains(&e.source()))
                .count();
            indeg.insert(n, d);
        }
        let mut ready: VecDeque<NodeIndex> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(n, _)| *n)
            .collect();
        let mut out = Vec::new();
        while let Some(n) = ready.pop_front() {
            out.push(self.g[n]);
            for e in self.g.edges_directed(n, petgraph::Direction::Outgoing) {
                let t = e.target();
                if let Some(d) = indeg.get_mut(&t) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push_back(t);
                    }
                }
            }
        }
        out
    }

    /// Número de celdas registradas (con o sin dependencias).
    pub fn node_count(&self) -> usize {
        self.g.node_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellRef;

    fn cr(s: &str) -> CellRef {
        s.parse().unwrap()
    }

    #[test]
    fn linear_chain_topo_order() {
        let mut g = SheetGraph::new();
        // C1 = B1+1, B1 = A1+1. Esperamos topo A1 → B1 → C1.
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        g.set_deps(cr("C1"), &[cr("B1")]).unwrap();
        let order = g.downstream_topo(&[cr("A1")]);
        assert_eq!(order, vec![cr("A1"), cr("B1"), cr("C1")]);
    }

    #[test]
    fn diamond_topo_resolves_consistently() {
        let mut g = SheetGraph::new();
        // D = B + C, B = A, C = A. Topo: A primero, luego B y C
        // (en cualquier orden), luego D.
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        g.set_deps(cr("C1"), &[cr("A1")]).unwrap();
        g.set_deps(cr("D1"), &[cr("B1"), cr("C1")]).unwrap();
        let order = g.downstream_topo(&[cr("A1")]);
        assert_eq!(order.first(), Some(&cr("A1")));
        assert_eq!(order.last(), Some(&cr("D1")));
        assert_eq!(order.len(), 4);
    }

    #[test]
    fn downstream_only_visits_affected_subgraph() {
        let mut g = SheetGraph::new();
        // Dos cadenas independientes: A→B→C y X→Y→Z.
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        g.set_deps(cr("C1"), &[cr("B1")]).unwrap();
        g.set_deps(cr("Y1"), &[cr("X1")]).unwrap();
        g.set_deps(cr("Z1"), &[cr("Y1")]).unwrap();
        let touched = g.downstream_topo(&[cr("X1")]);
        // Solo la cadena de X. No tocamos A/B/C.
        assert_eq!(
            touched.iter().copied().collect::<HashSet<_>>(),
            [cr("X1"), cr("Y1"), cr("Z1")].into_iter().collect()
        );
    }

    #[test]
    fn cycle_self_reference_rejected() {
        let mut g = SheetGraph::new();
        let err = g.set_deps(cr("A1"), &[cr("A1")]).unwrap_err();
        assert_eq!(err.target, cr("A1"));
        assert_eq!(g.node_count(), 0, "rejected change should not mutate graph");
    }

    #[test]
    fn cycle_through_intermediate_rejected() {
        let mut g = SheetGraph::new();
        // A→B→C ya existe. Intentar C ← A crearía A→B→C→A.
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        g.set_deps(cr("C1"), &[cr("B1")]).unwrap();
        let err = g.set_deps(cr("A1"), &[cr("C1")]).unwrap_err();
        assert_eq!(err.target, cr("A1"));
        // El chain devuelto debe contener a C1 → ... → A1 (o
        // equivalente).
        assert!(err.chain.contains(&cr("C1")));
    }

    #[test]
    fn cycle_rejection_leaves_graph_unchanged() {
        let mut g = SheetGraph::new();
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        let before = g.deps_of(cr("B1"));
        let _ = g.set_deps(cr("A1"), &[cr("B1")]);
        let after = g.deps_of(cr("B1"));
        assert_eq!(before, after);
        assert!(g.deps_of(cr("A1")).is_empty());
    }

    #[test]
    fn set_deps_replaces_old_dependencies() {
        let mut g = SheetGraph::new();
        g.set_deps(cr("B1"), &[cr("A1")]).unwrap();
        assert_eq!(g.deps_of(cr("B1")), vec![cr("A1")]);
        g.set_deps(cr("B1"), &[cr("C1")]).unwrap();
        assert_eq!(g.deps_of(cr("B1")), vec![cr("C1")]);
        // A1 ya no tiene B1 como dependiente.
        let touched = g.downstream_topo(&[cr("A1")]);
        assert_eq!(touched, vec![cr("A1")]);
    }

    #[test]
    fn isolated_cell_in_seed_appears_alone() {
        let g = SheetGraph::new();
        // Celda nunca registrada: no aparece. Recalcular un nodo
        // desconocido es no-op.
        assert!(g.downstream_topo(&[cr("Z9")]).is_empty());
    }

    #[test]
    fn multiple_seeds_merge_and_topo_holds() {
        let mut g = SheetGraph::new();
        // X→Y, A→Y. Si los seeds son [X, A], Y aparece después.
        g.set_deps(cr("Y1"), &[cr("X1"), cr("A1")]).unwrap();
        let order = g.downstream_topo(&[cr("X1"), cr("A1")]);
        assert_eq!(order.last(), Some(&cr("Y1")));
    }
}
