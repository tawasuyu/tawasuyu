//! iniy-graph — grafo de implicaciones sobre el corpus de aserciones.
//!
//! Cada nodo es una aserción; cada arista es una relación NLI (entailment o
//! contradiction) entre dos aserciones. Encima del grafo viven:
//! - Consultas analíticas: top-N pares contradictorios.
//! - Propagación de creencias: dada una opinión sobre A, derivar opiniones
//!   inducidas sobre todo lo que A implica o contradice transitivamente,
//!   con descuento de Jøsang por el score NLI de cada arista.

use iniy_core::{Asercion, AsercionId, ClaseNli, Implicacion, Opinion};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use std::collections::{HashMap, VecDeque};

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

    /// Propaga `op_inicial` desde `origen` por el grafo, descontando por el
    /// score NLI de cada arista y fusionando cuando dos rutas convergen sobre
    /// la misma aserción destino. Aristas tratadas como simétricas (las
    /// implicaciones del corpus pueden venir indexadas en cualquier orden).
    ///
    /// Algoritmo: BFS por niveles. En cada nivel, cada nodo nuevo recibe la
    /// opinión derivada del nodo del nivel anterior con mejor score, y se
    /// fusiona con cualquier opinión preexistente del mismo nivel. Se detiene
    /// cuando ya no hay nodos nuevos por explorar (no propaga al ya-visitado
    /// — evita loops y mantiene el costo O(V+E)).
    ///
    /// Devuelve un mapa `AsercionId -> Opinion` que incluye al `origen` (con
    /// `op_inicial`) y a cada aserción alcanzable que no fue neutral en alguna
    /// arista del camino.
    pub fn propagar(&self, origen: AsercionId, op_inicial: Opinion) -> HashMap<AsercionId, Opinion> {
        let mut salida: HashMap<AsercionId, Opinion> = HashMap::new();
        let Some(&origen_idx) = self.indice.get(&origen) else {
            salida.insert(origen, op_inicial);
            return salida;
        };
        salida.insert(origen, op_inicial);

        let mut visitados: HashMap<NodeIndex, Opinion> = HashMap::new();
        visitados.insert(origen_idx, op_inicial);
        let mut cola: VecDeque<NodeIndex> = VecDeque::from([origen_idx]);

        while let Some(actual) = cola.pop_front() {
            let op_actual = *visitados.get(&actual).expect("invariante de BFS");
            // Vecinos en ambas direcciones (out + in): la implicación
            // premisa→hipótesis se considera bi-direccionable a nivel de
            // propagación de creencia (apoyar A apoya B y viceversa, con
            // descuento por el score).
            for dir in [Direction::Outgoing, Direction::Incoming] {
                for eref in self.grafo.edges_directed(actual, dir) {
                    let vecino = if dir == Direction::Outgoing { eref.target() } else { eref.source() };
                    if visitados.contains_key(&vecino) {
                        continue;
                    }
                    let rel = &eref.weight().relacion;
                    let op_derivada = if rel.entailment >= rel.contradiction && rel.entailment > 0.0 {
                        op_actual.descontar(rel.entailment)
                    } else if rel.contradiction > 0.0 {
                        op_actual.invertir().descontar(rel.contradiction)
                    } else {
                        continue;
                    };
                    visitados.insert(vecino, op_derivada);
                    salida.insert(self.grafo[vecino], op_derivada);
                    cola.push_back(vecino);
                }
            }
        }
        salida
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{AsercionId, ChunkId, DocId, Opinion, RelacionNli};

    fn aserc(t: &str) -> Asercion {
        Asercion {
            id: AsercionId::nuevo(),
            doc_id: DocId::nuevo(),
            chunk_id: ChunkId::nuevo(),
            texto: t.into(),
            opinion_autoral: Opinion::nueva(0.5, 0.2, 0.3, 0.5).unwrap(),
        }
    }

    #[test]
    fn propagar_origen_sin_aristas_solo_devuelve_a_si_mismo() {
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("a");
        g.agregar_asercion(&a);
        let op = Opinion::nueva(0.9, 0.05, 0.05, 0.5).unwrap();
        let map = g.propagar(a.id, op);
        assert_eq!(map.len(), 1);
        assert!((map[&a.id].creencia - 0.9).abs() < 1e-5);
    }

    #[test]
    fn propagar_entailment_descuenta_creencia() {
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("A");
        let b = aserc("B");
        g.agregar_asercion(&a);
        g.agregar_asercion(&b);
        g.agregar_implicacion(Implicacion {
            premisa: a.id,
            hipotesis: b.id,
            relacion: RelacionNli { entailment: 0.8, contradiction: 0.0, neutral: 0.2 },
        });
        let op = Opinion::dogmatica_si();
        let map = g.propagar(a.id, op);
        let op_b = map[&b.id];
        // Entailment 0.8: creencia degradada 0.8 * 1.0 = 0.8, resto a u.
        assert!((op_b.creencia - 0.8).abs() < 1e-5);
        assert!((op_b.incertidumbre - 0.2).abs() < 1e-5);
    }

    #[test]
    fn propagar_contradiction_invierte_y_descuenta() {
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("A");
        let b = aserc("B");
        g.agregar_asercion(&a);
        g.agregar_asercion(&b);
        g.agregar_implicacion(Implicacion {
            premisa: a.id,
            hipotesis: b.id,
            relacion: RelacionNli { entailment: 0.0, contradiction: 0.7, neutral: 0.3 },
        });
        let op = Opinion::dogmatica_si();
        let map = g.propagar(a.id, op);
        let op_b = map[&b.id];
        // dogmatica_si invertida = dogmatica_no, descontada por 0.7:
        // d = 1.0 * 0.7 = 0.7, b = 0, u = 0.3.
        assert!((op_b.descreencia - 0.7).abs() < 1e-5);
        assert!((op_b.incertidumbre - 0.3).abs() < 1e-5);
        assert!(op_b.creencia < 1e-5);
    }

    #[test]
    fn propagar_dos_saltos_compone_descuento() {
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("A");
        let b = aserc("B");
        let c = aserc("C");
        g.agregar_asercion(&a);
        g.agregar_asercion(&b);
        g.agregar_asercion(&c);
        g.agregar_implicacion(Implicacion {
            premisa: a.id, hipotesis: b.id,
            relacion: RelacionNli { entailment: 0.8, contradiction: 0.0, neutral: 0.2 },
        });
        g.agregar_implicacion(Implicacion {
            premisa: b.id, hipotesis: c.id,
            relacion: RelacionNli { entailment: 0.5, contradiction: 0.0, neutral: 0.5 },
        });
        let map = g.propagar(a.id, Opinion::dogmatica_si());
        let op_c = map[&c.id];
        // creencia tras dos descuentos: 1.0 * 0.8 * 0.5 = 0.4
        assert!((op_c.creencia - 0.4).abs() < 1e-5);
    }

    #[test]
    fn propagar_aristas_neutras_no_recorre() {
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("A");
        let b = aserc("B");
        g.agregar_asercion(&a);
        g.agregar_asercion(&b);
        g.agregar_implicacion(Implicacion {
            premisa: a.id, hipotesis: b.id,
            relacion: RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 },
        });
        let map = g.propagar(a.id, Opinion::dogmatica_si());
        assert!(!map.contains_key(&b.id));
    }
}
