//! iniy-graph — grafo de implicaciones sobre el corpus de aserciones.
//!
//! Cada nodo es una aserción; cada arista es una relación NLI (entailment o
//! contradiction) por encima de un umbral. Permite consultas: pares más
//! contradictorios, componentes conexas, caminos de implicación, y métricas
//! geométricas (densidad, asortatividad, curvatura discreta).

use iniy_core::{Asercion, AsercionId, ClaseNli, Implicacion};
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

pub struct GrafoCreencias {
    grafo: DiGraph<AsercionId, Implicacion>,
    indice: HashMap<AsercionId, NodeIndex>,
}

impl Default for GrafoCreencias {
    fn default() -> Self {
        Self::nuevo()
    }
}

impl GrafoCreencias {
    pub fn nuevo() -> Self {
        Self { grafo: DiGraph::new(), indice: HashMap::new() }
    }

    pub fn agregar_asercion(&mut self, asercion: &Asercion) -> NodeIndex {
        if let Some(&idx) = self.indice.get(&asercion.id) {
            return idx;
        }
        let idx = self.grafo.add_node(asercion.id);
        self.indice.insert(asercion.id, idx);
        idx
    }

    pub fn agregar_implicacion(&mut self, imp: Implicacion) {
        let (Some(&p), Some(&h)) = (self.indice.get(&imp.premisa), self.indice.get(&imp.hipotesis)) else {
            return;
        };
        self.grafo.add_edge(p, h, imp);
    }

    /// Top-N pares más contradictorios (mayor score de contradicción).
    pub fn top_contradicciones(&self, n: usize) -> Vec<&Implicacion> {
        let mut todas: Vec<&Implicacion> = self
            .grafo
            .edge_weights()
            .filter(|imp| imp.relacion.dominante() == ClaseNli::Contradiction)
            .collect();
        todas.sort_by(|a, b| b.relacion.contradiction.partial_cmp(&a.relacion.contradiction).unwrap_or(std::cmp::Ordering::Equal));
        todas.truncate(n);
        todas
    }

    pub fn cantidad_aserciones(&self) -> usize {
        self.grafo.node_count()
    }

    pub fn cantidad_implicaciones(&self) -> usize {
        self.grafo.edge_count()
    }
}
