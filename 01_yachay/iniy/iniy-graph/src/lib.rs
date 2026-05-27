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
use std::collections::{HashMap, HashSet};

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

    /// Propaga `op_inicial` desde `origen` por el grafo NLI, descontando por
    /// el score de cada arista y FUSIONANDO opiniones cuando múltiples rutas
    /// convergen sobre la misma aserción destino.
    ///
    /// Algoritmo: BFS por niveles. La opinión de un nodo `n` en nivel `k` se
    /// computa como `Opinion::fusionar_muchas` de las opiniones derivadas
    /// desde TODOS sus vecinos que estén en niveles `< k`. Un nodo solo entra
    /// a la salida si tiene al menos una derivación no neutral.
    ///
    /// Esto convierte rutas convergentes en evidencia acumulada: si A → B y
    /// A → C → B con scores suficientes, B fusiona las dos opiniones (con
    /// menor incertidumbre que cualquiera de las dos sola).
    ///
    /// Aristas se tratan como simétricas para propagación: una arista NLI
    /// premisa→hipótesis representa una relación que la creencia respeta en
    /// ambas direcciones (apoyar A apoya B; descreer A descree B).
    pub fn propagar(&self, origen: AsercionId, op_inicial: Opinion) -> HashMap<AsercionId, Opinion> {
        let mut salida: HashMap<AsercionId, Opinion> = HashMap::new();
        salida.insert(origen, op_inicial);
        let Some(&origen_idx) = self.indice.get(&origen) else {
            return salida;
        };

        // ops[idx] = opinión derivada del nodo `idx`. nivel[idx] = nivel BFS.
        let mut ops: HashMap<NodeIndex, Opinion> = HashMap::new();
        let mut nivel: HashMap<NodeIndex, usize> = HashMap::new();
        ops.insert(origen_idx, op_inicial);
        nivel.insert(origen_idx, 0);

        let mut frontera: HashSet<NodeIndex> = HashSet::from([origen_idx]);
        let mut k: usize = 0;

        while !frontera.is_empty() {
            // Candidatos para nivel k+1 = vecinos no visitados de cualquier nodo en la frontera.
            let mut candidatos: HashSet<NodeIndex> = HashSet::new();
            for &actual in &frontera {
                for dir in [Direction::Outgoing, Direction::Incoming] {
                    for eref in self.grafo.edges_directed(actual, dir) {
                        let vecino = if dir == Direction::Outgoing { eref.target() } else { eref.source() };
                        if !nivel.contains_key(&vecino) {
                            candidatos.insert(vecino);
                        }
                    }
                }
            }
            if candidatos.is_empty() {
                break;
            }
            k += 1;
            let mut nueva_frontera: HashSet<NodeIndex> = HashSet::new();
            for cand in candidatos {
                // Recolectar derivaciones desde TODOS los predecesores de niveles < k.
                let mut derivadas: Vec<Opinion> = Vec::new();
                for dir in [Direction::Outgoing, Direction::Incoming] {
                    for eref in self.grafo.edges_directed(cand, dir) {
                        let vecino = if dir == Direction::Outgoing { eref.target() } else { eref.source() };
                        let Some(&niv_vecino) = nivel.get(&vecino) else { continue; };
                        if niv_vecino >= k {
                            continue;
                        }
                        let op_vecino = ops[&vecino];
                        let rel = &eref.weight().relacion;
                        let derivada = if rel.entailment >= rel.contradiction && rel.entailment > 0.0 {
                            op_vecino.descontar(rel.entailment)
                        } else if rel.contradiction > 0.0 {
                            op_vecino.invertir().descontar(rel.contradiction)
                        } else {
                            continue;
                        };
                        derivadas.push(derivada);
                    }
                }
                if derivadas.is_empty() {
                    // El candidato no tiene aristas no-neutrales hacia el cono
                    // explorado; no se propaga.
                    continue;
                }
                let op_cand = Opinion::fusionar_muchas(&derivadas);
                ops.insert(cand, op_cand);
                nivel.insert(cand, k);
                salida.insert(self.grafo[cand], op_cand);
                nueva_frontera.insert(cand);
            }
            frontera = nueva_frontera;
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
    fn propagar_dos_caminos_convergentes_fusiona_bajando_incertidumbre() {
        // Topología: A → B (e=0.6), A → C (e=0.6), B → D (e=0.6), C → D (e=0.6).
        // D recibe dos rutas paralelas desde A. Cada una sola da
        // creencia 0.6 * 0.6 = 0.36 con u alta. La fusión debe bajar la
        // incertidumbre respecto a una sola ruta.
        let mut g = GrafoCreencias::nuevo();
        let a = aserc("A");
        let b = aserc("B");
        let c = aserc("C");
        let d = aserc("D");
        g.agregar_asercion(&a);
        g.agregar_asercion(&b);
        g.agregar_asercion(&c);
        g.agregar_asercion(&d);
        let e = |p, h, val| Implicacion { premisa: p, hipotesis: h,
            relacion: RelacionNli { entailment: val, contradiction: 0.0, neutral: 1.0 - val } };
        g.agregar_implicacion(e(a.id, b.id, 0.6));
        g.agregar_implicacion(e(a.id, c.id, 0.6));
        g.agregar_implicacion(e(b.id, d.id, 0.6));
        g.agregar_implicacion(e(c.id, d.id, 0.6));
        let map = g.propagar(a.id, Opinion::dogmatica_si());
        // D debe estar presente y su u < u de una sola ruta (que sería 0.64).
        let op_d = map[&d.id];
        assert!(op_d.creencia > 0.3, "D recibió creencia: {op_d:?}");
        assert!(op_d.incertidumbre < 0.64, "fusión debe bajar u, got: {}", op_d.incertidumbre);
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
