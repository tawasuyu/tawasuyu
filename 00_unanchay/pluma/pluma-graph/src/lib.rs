//! `pluma_app-graph` — el grafo narrativo (DAG de `NarrativeAtom`s).
//!
//! Mantiene los átomos + una adjacency list `dependencia → dependientes`.
//! Cuando un átomo muta, [`NarrativeGraph::propagate_mutation`] marca en
//! cascada a todo descendiente como `PendingEvaluation` — la "onda de
//! choque lógica" de la spec. Agnóstico de UI: devuelve los ids
//! afectados; el front-end decide cuándo re-renderizar.

#![forbid(unsafe_code)]

use pluma_core::{CoherenceState, NarrativeAtom};
use std::collections::{HashMap, HashSet, VecDeque};
use uuid::Uuid;

/// El documento como grafo dirigido acíclico de átomos narrativos.
#[derive(Debug, Default)]
pub struct NarrativeGraph {
    nodes: HashMap<Uuid, NarrativeAtom>,
    /// `dependencia → [átomos que dependen de ella]`.
    adjacency: HashMap<Uuid, Vec<Uuid>>,
}

impl NarrativeGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn contains(&self, id: Uuid) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn get(&self, id: Uuid) -> Option<&NarrativeAtom> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut NarrativeAtom> {
        self.nodes.get_mut(&id)
    }

    /// Itera todos los átomos del grafo (orden no determinista).
    pub fn atoms(&self) -> impl Iterator<Item = &NarrativeAtom> {
        self.nodes.values()
    }

    /// Construye un grafo desde una colección de átomos.
    pub fn from_atoms(atoms: impl IntoIterator<Item = NarrativeAtom>) -> Self {
        let mut g = Self::new();
        for a in atoms {
            g.insert(a);
        }
        g
    }

    /// Inserta un átomo y conecta las aristas desde sus dependencias.
    pub fn insert(&mut self, atom: NarrativeAtom) {
        let id = atom.id;
        for &dep in &atom.dependencies {
            let children = self.adjacency.entry(dep).or_default();
            if !children.contains(&id) {
                children.push(id);
            }
        }
        self.nodes.insert(id, atom);
    }

    /// Dependientes directos de `id`.
    pub fn dependents(&self, id: Uuid) -> &[Uuid] {
        self.adjacency.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Propaga una mutación: marca `PendingEvaluation` en TODO descendiente
    /// transitivo de `origin` (BFS sobre la adjacency). Devuelve los ids
    /// afectados — el caller (front-end) decide cuándo re-renderizar.
    ///
    /// `origin` mismo no se marca (es la fuente; ya se sabe que cambió).
    pub fn propagate_mutation(&mut self, origin: Uuid) -> Vec<Uuid> {
        let mut affected = Vec::new();
        let mut seen: HashSet<Uuid> = HashSet::new();
        let mut queue: VecDeque<Uuid> = VecDeque::new();
        queue.push_back(origin);
        seen.insert(origin);

        while let Some(current) = queue.pop_front() {
            let children: Vec<Uuid> = self
                .adjacency
                .get(&current)
                .cloned()
                .unwrap_or_default();
            for child in children {
                if seen.insert(child) {
                    if let Some(node) = self.nodes.get_mut(&child) {
                        node.coherence = CoherenceState::PendingEvaluation;
                    }
                    affected.push(child);
                    queue.push_back(child);
                }
            }
        }
        affected
    }

    /// Orden topológico de los átomos (dependencias antes que dependientes).
    /// `None` si el grafo tiene un ciclo (no es un DAG válido).
    pub fn topological_order(&self) -> Option<Vec<Uuid>> {
        let mut indeg: HashMap<Uuid, usize> = self.nodes.keys().map(|&k| (k, 0)).collect();
        for atom in self.nodes.values() {
            for &dep in &atom.dependencies {
                if self.nodes.contains_key(&dep) {
                    *indeg.entry(atom.id).or_insert(0) += 1;
                }
            }
        }
        let mut queue: VecDeque<Uuid> =
            indeg.iter().filter(|(_, &d)| d == 0).map(|(&k, _)| k).collect();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(u) = queue.pop_front() {
            order.push(u);
            for &child in self.dependents(u) {
                if let Some(d) = indeg.get_mut(&child) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
        if order.len() == self.nodes.len() {
            Some(order)
        } else {
            None // quedó un ciclo
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construye una cadena a → b → c y devuelve sus ids.
    fn chain() -> (NarrativeGraph, Uuid, Uuid, Uuid) {
        let mut g = NarrativeGraph::new();
        let a = NarrativeAtom::new("a", "main");
        let ( a_id,) = (a.id,);
        let b = NarrativeAtom::new("b", "main").depends_on(a_id);
        let b_id = b.id;
        let c = NarrativeAtom::new("c", "main").depends_on(b_id);
        let c_id = c.id;
        g.insert(a);
        g.insert(b);
        g.insert(c);
        (g, a_id, b_id, c_id)
    }

    #[test]
    fn insert_wires_adjacency() {
        let (g, a, b, c) = chain();
        assert_eq!(g.len(), 3);
        assert_eq!(g.dependents(a), &[b]);
        assert_eq!(g.dependents(b), &[c]);
        assert!(g.dependents(c).is_empty());
    }

    #[test]
    fn propagate_marks_all_descendants_pending() {
        let (mut g, a, b, c) = chain();
        let affected = g.propagate_mutation(a);
        assert_eq!(affected.len(), 2);
        assert!(affected.contains(&b) && affected.contains(&c));
        assert_eq!(g.get(b).unwrap().coherence, CoherenceState::PendingEvaluation);
        assert_eq!(g.get(c).unwrap().coherence, CoherenceState::PendingEvaluation);
        // El origen NO se marca.
        assert_eq!(g.get(a).unwrap().coherence, CoherenceState::Valid);
    }

    #[test]
    fn propagate_from_leaf_affects_nothing() {
        let (mut g, _a, _b, c) = chain();
        assert!(g.propagate_mutation(c).is_empty());
    }

    #[test]
    fn topological_order_respects_dependencies() {
        let (g, a, b, c) = chain();
        let order = g.topological_order().expect("es un DAG");
        let pos = |id: Uuid| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(a) < pos(b));
        assert!(pos(b) < pos(c));
    }

    #[test]
    fn cycle_has_no_topological_order() {
        // a depende de b, b depende de a.
        let mut g = NarrativeGraph::new();
        let a = NarrativeAtom::new("a", "main");
        let b = NarrativeAtom::new("b", "main");
        let (a_id, b_id) = (a.id, b.id);
        let a = a.depends_on(b_id);
        let b = b.depends_on(a_id);
        g.insert(a);
        g.insert(b);
        assert!(g.topological_order().is_none());
    }
}
