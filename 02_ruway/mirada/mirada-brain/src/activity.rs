//! Grafo de actividad — el linaje de procesos de las ventanas, en *constelaciones*.
//!
//! Una **constelación** es un grupo de ventanas emparentadas por linaje de
//! proceso: la terminal y el editor que lanzó desde ella, el navegador y su
//! ventana de descargas… La idea (del brainstorm de escritorio) es navegar y
//! agrupar por constelación —no por ventana suelta—, porque eso refleja en qué
//! está trabajando el usuario.
//!
//! El Cuerpo reporta, por cada ventana, su PID y la **cadena de ancestros** (los
//! PIDs del padre inmediato hacia la raíz) vía
//! [`BodyEvent::WindowLineage`](mirada_protocol::BodyEvent::WindowLineage). Este
//! módulo es pura teoría de grafos sobre esos datos: dos ventanas están en la
//! misma constelación si el proceso de una es **ancestro** del de la otra
//! (directa o transitivamente, encadenando por PIDs intermedios aunque no sean
//! ventanas). Es determinista y testeable sin compositor.

use std::collections::HashMap;

use mirada_layout::WindowId;

/// El linaje de proceso de una ventana: su PID y la cadena de PIDs ancestros
/// (padre inmediato primero), tal como la reporta el Cuerpo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lineage {
    pub pid: u32,
    pub ancestors: Vec<u32>,
}

/// El grafo de actividad: el linaje conocido de cada ventana. Las ventanas sin
/// linaje (el Cuerpo no pudo averiguar el PID) cuentan como constelación propia.
#[derive(Debug, Default, Clone)]
pub struct ActivityGraph {
    lineage: HashMap<WindowId, Lineage>,
}

impl ActivityGraph {
    /// Registra (o reemplaza) el linaje de una ventana.
    pub fn record(&mut self, id: WindowId, pid: u32, ancestors: Vec<u32>) {
        self.lineage.insert(id, Lineage { pid, ancestors });
    }

    /// Olvida el linaje de una ventana cerrada.
    pub fn forget(&mut self, id: WindowId) {
        self.lineage.remove(&id);
    }

    /// El linaje conocido de una ventana, si lo hay.
    pub fn lineage(&self, id: WindowId) -> Option<&Lineage> {
        self.lineage.get(&id)
    }

    /// `true` si las ventanas `a` y `b` están emparentadas por linaje: el PID de
    /// una aparece en la cadena de ancestros de la otra (una desciende de la
    /// otra). Sin linaje de alguna → no emparentadas.
    fn related(&self, a: WindowId, b: WindowId) -> bool {
        let (Some(la), Some(lb)) = (self.lineage.get(&a), self.lineage.get(&b)) else {
            return false;
        };
        la.ancestors.contains(&lb.pid) || lb.ancestors.contains(&la.pid)
    }

    /// Particiona `ids` en constelaciones: componentes conexas bajo la relación
    /// "una desciende de la otra". El orden de las constelaciones y de sus
    /// miembros respeta el de `ids` (determinista). Una ventana sin parentesco
    /// con ninguna otra es su propia constelación de un elemento.
    pub fn constellations(&self, ids: &[WindowId]) -> Vec<Vec<WindowId>> {
        let n = ids.len();
        // Union-find sencillo (n = ventanas de un escritorio, pequeño).
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for i in 0..n {
            for j in (i + 1)..n {
                if self.related(ids[i], ids[j]) {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
        // Agrupa por raíz, conservando el orden de primera aparición.
        let mut order: Vec<usize> = Vec::new();
        let mut groups: HashMap<usize, Vec<WindowId>> = HashMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            if !groups.contains_key(&r) {
                order.push(r);
            }
            groups.entry(r).or_default().push(ids[i]);
        }
        order.into_iter().map(|r| groups.remove(&r).unwrap()).collect()
    }

    /// La constelación de `id` dentro de `ids` (incluye a `id`). Si `id` no está
    /// en `ids`, devuelve vacío.
    pub fn constellation_of(&self, id: WindowId, ids: &[WindowId]) -> Vec<WindowId> {
        self.constellations(ids)
            .into_iter()
            .find(|c| c.contains(&id))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Terminal (pid 100) → shell intermedio (101) → editor GUI (pid 102): el
    /// editor lista 101 y 100 como ancestros, así que cae en la constelación de
    /// la terminal aunque el shell no tenga ventana.
    fn graph() -> ActivityGraph {
        let mut g = ActivityGraph::default();
        g.record(1, 100, vec![1]); // terminal, hija de init
        g.record(2, 102, vec![101, 100, 1]); // editor lanzado desde la terminal
        g.record(3, 500, vec![1]); // navegador, sin parentesco
        g.record(4, 540, vec![500, 1]); // descargas del navegador
        g
    }

    #[test]
    fn descendants_share_a_constellation_through_intermediate_pids() {
        let g = graph();
        let cs = g.constellations(&[1, 2, 3, 4]);
        // Dos constelaciones: {terminal, editor} y {navegador, descargas}.
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0], vec![1, 2]);
        assert_eq!(cs[1], vec![3, 4]);
    }

    #[test]
    fn constellation_of_returns_the_whole_lineage_group() {
        let g = graph();
        assert_eq!(g.constellation_of(2, &[1, 2, 3, 4]), vec![1, 2]);
        assert_eq!(g.constellation_of(4, &[1, 2, 3, 4]), vec![3, 4]);
    }

    #[test]
    fn a_window_without_lineage_is_its_own_constellation() {
        let mut g = graph();
        g.record(5, 9000, vec![]); // sin ancestros conocidos
        let cs = g.constellations(&[1, 2, 5]);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0], vec![1, 2]);
        assert_eq!(cs[1], vec![5]);
        // Y una ventana totalmente desconocida también va sola.
        assert_eq!(g.constellation_of(99, &[1, 2, 5]), Vec::<WindowId>::new());
    }

    #[test]
    fn forgetting_a_window_breaks_its_links() {
        let mut g = graph();
        g.forget(2); // se cierra el editor
        // La terminal queda sola; el editor ya no aporta el puente.
        assert_eq!(g.constellation_of(1, &[1, 3, 4]), vec![1]);
    }

    #[test]
    fn unrelated_windows_each_stand_alone() {
        let mut g = ActivityGraph::default();
        g.record(1, 10, vec![1]);
        g.record(2, 20, vec![1]); // mismo abuelo (init) pero sin descendencia mutua
        let cs = g.constellations(&[1, 2]);
        assert_eq!(cs.len(), 2); // compartir init no los une
    }
}
