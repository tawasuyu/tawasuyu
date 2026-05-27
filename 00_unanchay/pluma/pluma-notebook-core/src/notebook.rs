//! El notebook — celdas en orden de presentación + un DAG de dependencias.
//!
//! Un notebook tiene dos estructuras a la vez: el **orden de
//! presentación** (la lista de celdas tal como se leen) y el **DAG de
//! dependencias** (qué celda necesita el resultado de cuál). La
//! ejecución sigue el DAG; el digest Merkle certifica que dos corridas
//! del mismo notebook producen lo mismo.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::cell::{Cell, CellId, CellKind, CellOutput, CellState, Position};

/// Un notebook reproducible.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Notebook {
    /// Celdas en orden de presentación.
    cells: Vec<Cell>,
    next_id: CellId,
}

impl Notebook {
    pub fn new() -> Self {
        Self { cells: Vec::new(), next_id: 1 }
    }

    /// Añade una celda al final, sin dependencias y en estado `Stale`.
    /// Devuelve su id.
    pub fn push(&mut self, kind: CellKind, source: impl Into<String>) -> CellId {
        let id = self.next_id;
        self.next_id += 1;
        self.cells.push(Cell {
            id,
            kind,
            source: source.into(),
            depends_on: Vec::new(),
            state: CellState::Stale,
            position: None,
            last_output: None,
        });
        id
    }

    /// Reemplaza la última salida de una celda. Editar la fuente o cambiar
    /// una dependencia no la borra automáticamente — el último output
    /// queda visible mientras la celda está `Stale`, hasta la próxima
    /// corrida. `false` si la celda no existe.
    pub fn set_last_output(&mut self, id: CellId, out: Option<CellOutput>) -> bool {
        match self.cell_mut(id) {
            Some(c) => {
                c.last_output = out;
                true
            }
            None => false,
        }
    }

    /// Coloca una celda en el canvas espacial. `None` la devuelve al modo
    /// puramente lineal. No toca el estado de la celda (la posición es
    /// presentación, no contenido) y por tanto no afecta el digest.
    pub fn set_position(&mut self, id: CellId, pos: Option<Position>) -> bool {
        match self.cell_mut(id) {
            Some(c) => {
                c.position = pos;
                true
            }
            None => false,
        }
    }

    /// Posición de una celda en el canvas, si la tiene.
    pub fn position(&self, id: CellId) -> Option<Position> {
        self.cell(id).and_then(|c| c.position)
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Celdas en orden de presentación.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn cell(&self, id: CellId) -> Option<&Cell> {
        self.cells.iter().find(|c| c.id == id)
    }

    fn cell_mut(&mut self, id: CellId) -> Option<&mut Cell> {
        self.cells.iter_mut().find(|c| c.id == id)
    }

    /// `true` si `a` depende —directa o transitivamente— de `b`.
    fn depends_transitively(&self, a: CellId, b: CellId) -> bool {
        let mut seen: BTreeSet<CellId> = BTreeSet::new();
        let mut queue: VecDeque<CellId> = VecDeque::from([a]);
        while let Some(cur) = queue.pop_front() {
            let Some(cell) = self.cell(cur) else { continue };
            for &dep in &cell.depends_on {
                if dep == b {
                    return true;
                }
                if seen.insert(dep) {
                    queue.push_back(dep);
                }
            }
        }
        false
    }

    /// Declara que `cell` depende de `dep`. Rechaza (devuelve `false`)
    /// si alguna celda no existe o si la arista crearía un ciclo.
    pub fn add_dependency(&mut self, cell: CellId, dep: CellId) -> bool {
        if cell == dep || self.cell(cell).is_none() || self.cell(dep).is_none() {
            return false;
        }
        // Si `dep` ya depende de `cell`, esta arista cerraría un ciclo.
        if self.depends_transitively(dep, cell) {
            return false;
        }
        let c = self.cell_mut(cell).expect("verificado");
        if !c.depends_on.contains(&dep) {
            c.depends_on.push(dep);
        }
        true
    }

    /// Reemplaza la fuente de una celda: la marca `Stale` y propaga la
    /// obsolescencia a todas sus dependientes. Devuelve los ids marcados
    /// (sin contar la celda misma). `false` si la celda no existe.
    pub fn set_source(&mut self, id: CellId, source: impl Into<String>) -> bool {
        let Some(c) = self.cell_mut(id) else {
            return false;
        };
        c.source = source.into();
        c.state = CellState::Stale;
        self.propagate_stale(id);
        true
    }

    /// Marca el estado de una celda. `false` si no existe.
    pub fn set_state(&mut self, id: CellId, state: CellState) -> bool {
        match self.cell_mut(id) {
            Some(c) => {
                c.state = state;
                true
            }
            None => false,
        }
    }

    /// Dependientes directos de `id`.
    pub fn dependents(&self, id: CellId) -> Vec<CellId> {
        self.cells
            .iter()
            .filter(|c| c.depends_on.contains(&id))
            .map(|c| c.id)
            .collect()
    }

    /// Dependientes transitivos de `root` (sin incluir a `root`). Es el
    /// cono de obsolescencia: las celdas que necesitan recomputarse si
    /// `root` cambia. Útil para minimizar la recomputación reactiva.
    pub fn dependents_transitive(&self, root: CellId) -> Vec<CellId> {
        let mut out: Vec<CellId> = Vec::new();
        let mut seen: BTreeSet<CellId> = BTreeSet::from([root]);
        let mut queue: VecDeque<CellId> = VecDeque::from([root]);
        while let Some(cur) = queue.pop_front() {
            for child in self.dependents(cur) {
                if seen.insert(child) {
                    out.push(child);
                    queue.push_back(child);
                }
            }
        }
        out
    }

    /// Marca `Stale` a todo dependiente transitivo de `id`. Devuelve los
    /// ids afectados.
    pub fn propagate_stale(&mut self, id: CellId) -> Vec<CellId> {
        let mut affected: Vec<CellId> = Vec::new();
        let mut seen: BTreeSet<CellId> = BTreeSet::from([id]);
        let mut queue: VecDeque<CellId> = VecDeque::from([id]);
        while let Some(cur) = queue.pop_front() {
            for child in self.dependents(cur) {
                if seen.insert(child) {
                    if let Some(c) = self.cell_mut(child) {
                        c.state = CellState::Stale;
                    }
                    affected.push(child);
                    queue.push_back(child);
                }
            }
        }
        affected
    }

    /// Orden topológico de ejecución (dependencias antes que
    /// dependientes). `None` si el DAG tiene un ciclo.
    pub fn execution_order(&self) -> Option<Vec<CellId>> {
        let mut indeg: BTreeMap<CellId, usize> =
            self.cells.iter().map(|c| (c.id, 0usize)).collect();
        for c in &self.cells {
            for &dep in &c.depends_on {
                if self.cell(dep).is_some() {
                    *indeg.get_mut(&c.id).unwrap() += 1;
                }
            }
        }
        let mut queue: VecDeque<CellId> =
            indeg.iter().filter(|(_, &d)| d == 0).map(|(&k, _)| k).collect();
        let mut order: Vec<CellId> = Vec::with_capacity(self.cells.len());
        while let Some(u) = queue.pop_front() {
            order.push(u);
            for child in self.dependents(u) {
                if let Some(d) = indeg.get_mut(&child) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(child);
                    }
                }
            }
        }
        (order.len() == self.cells.len()).then_some(order)
    }

    /// Digest Merkle de cada celda: `blake3(content_hash ‖ digests de las
    /// dependencias)`. Captura la celda y todo su linaje — dos notebooks
    /// con los mismos digests producen, reproduciblemente, lo mismo.
    /// `None` si hay un ciclo.
    fn all_digests(&self) -> Option<BTreeMap<CellId, [u8; 32]>> {
        let order = self.execution_order()?;
        let mut digests: BTreeMap<CellId, [u8; 32]> = BTreeMap::new();
        for id in order {
            let cell = self.cell(id).expect("del orden");
            let mut h = blake3::Hasher::new();
            h.update(&cell.content_hash());
            // Dependencias ordenadas → el digest no depende del orden de
            // declaración.
            let mut deps = cell.depends_on.clone();
            deps.sort_unstable();
            for dep in deps {
                if let Some(d) = digests.get(&dep) {
                    h.update(d);
                }
            }
            digests.insert(id, *h.finalize().as_bytes());
        }
        Some(digests)
    }

    /// Digest reproducible de una celda concreta.
    pub fn digest(&self, id: CellId) -> Option<[u8; 32]> {
        self.all_digests()?.get(&id).copied()
    }

    /// Digest reproducible del notebook entero: `blake3` de los digests
    /// de todas las celdas en orden de id. Dos notebooks con el mismo
    /// digest son reproduciblemente equivalentes. `None` si hay ciclo.
    pub fn notebook_digest(&self) -> Option<[u8; 32]> {
        let digests = self.all_digests()?;
        let mut h = blake3::Hasher::new();
        for d in digests.values() {
            h.update(d);
        }
        Some(*h.finalize().as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn code(nb: &mut Notebook, src: &str) -> CellId {
        nb.push(CellKind::Code { language: "rust".into() }, src)
    }

    /// Notebook a → b → c (cada uno depende del anterior).
    fn chain() -> (Notebook, CellId, CellId, CellId) {
        let mut nb = Notebook::new();
        let a = code(&mut nb, "let x = 1;");
        let b = code(&mut nb, "let y = x + 1;");
        let c = code(&mut nb, "println!(\"{y}\");");
        nb.add_dependency(b, a);
        nb.add_dependency(c, b);
        (nb, a, b, c)
    }

    #[test]
    fn push_keeps_display_order() {
        let (nb, a, b, c) = chain();
        let ids: Vec<_> = nb.cells().iter().map(|x| x.id).collect();
        assert_eq!(ids, vec![a, b, c]);
    }

    #[test]
    fn execution_order_respects_dependencies() {
        let (nb, a, b, c) = chain();
        assert_eq!(nb.execution_order(), Some(vec![a, b, c]));
    }

    #[test]
    fn add_dependency_rejects_cycles() {
        let (mut nb, a, _b, c) = chain();
        // a depender de c cerraría el ciclo a→b→c→a.
        assert!(!nb.add_dependency(a, c));
        assert!(nb.execution_order().is_some());
    }

    #[test]
    fn add_dependency_rejects_self_and_missing() {
        let (mut nb, a, ..) = chain();
        assert!(!nb.add_dependency(a, a));
        assert!(!nb.add_dependency(a, 999));
    }

    #[test]
    fn editing_a_cell_propagates_staleness() {
        let (mut nb, a, b, c) = chain();
        for id in [a, b, c] {
            nb.set_state(id, CellState::Fresh);
        }
        nb.set_source(a, "let x = 42;");
        // La celda editada y sus descendientes quedan Stale.
        assert_eq!(nb.cell(a).unwrap().state, CellState::Stale);
        assert_eq!(nb.cell(b).unwrap().state, CellState::Stale);
        assert_eq!(nb.cell(c).unwrap().state, CellState::Stale);
    }

    #[test]
    fn editing_a_leaf_does_not_stale_its_ancestors() {
        let (mut nb, a, b, c) = chain();
        for id in [a, b, c] {
            nb.set_state(id, CellState::Fresh);
        }
        nb.set_source(c, "println!(\"fin\");");
        assert_eq!(nb.cell(a).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(b).unwrap().state, CellState::Fresh);
        assert_eq!(nb.cell(c).unwrap().state, CellState::Stale);
    }

    #[test]
    fn notebook_digest_is_stable_across_calls() {
        let (nb, ..) = chain();
        assert_eq!(nb.notebook_digest(), nb.notebook_digest());
    }

    #[test]
    fn editing_a_source_changes_the_digest() {
        let (mut nb, a, ..) = chain();
        let before = nb.notebook_digest();
        nb.set_source(a, "let x = 999;");
        assert_ne!(before, nb.notebook_digest());
    }

    #[test]
    fn cell_digest_reflects_upstream_changes() {
        // Cambiar `a` cambia el digest de `c` (su descendiente).
        let (mut nb, a, _b, c) = chain();
        let c_before = nb.digest(c);
        nb.set_source(a, "let x = 7;");
        assert_ne!(c_before, nb.digest(c));
    }

    #[test]
    fn dependency_order_does_not_affect_digest() {
        // Dos celdas con las mismas dos dependencias, declaradas en
        // distinto orden, dan el mismo digest.
        let mut x = Notebook::new();
        let xa = code(&mut x, "a");
        let xb = code(&mut x, "b");
        let xc = code(&mut x, "c");
        x.add_dependency(xc, xa);
        x.add_dependency(xc, xb);

        let mut y = Notebook::new();
        let ya = code(&mut y, "a");
        let yb = code(&mut y, "b");
        let yc = code(&mut y, "c");
        y.add_dependency(yc, yb);
        y.add_dependency(yc, ya);

        assert_eq!(x.digest(xc), y.digest(yc));
    }

    #[test]
    fn embed_cells_carry_their_module() {
        let mut nb = Notebook::new();
        let id = nb.push(CellKind::Embed { module: "dominium".into() }, "preset: caos");
        assert!(matches!(
            &nb.cell(id).unwrap().kind,
            CellKind::Embed { module } if module == "dominium"
        ));
    }

    #[test]
    fn dependents_transitive_walks_the_cone() {
        // a → b → c, d (suelta)
        let mut nb = Notebook::new();
        let a = code(&mut nb, "a");
        let b = code(&mut nb, "b");
        let c = code(&mut nb, "c");
        let _d = code(&mut nb, "d");
        nb.add_dependency(b, a);
        nb.add_dependency(c, b);

        let cono = nb.dependents_transitive(a);
        assert_eq!(cono, vec![b, c]); // ni a ni d
    }

    #[test]
    fn dependents_transitive_of_leaf_is_empty() {
        let (nb, _a, _b, c) = chain();
        assert!(nb.dependents_transitive(c).is_empty());
    }

    #[test]
    fn position_round_trips_and_is_optional() {
        let mut nb = Notebook::new();
        let id = nb.push(CellKind::Markdown, "x");
        assert_eq!(nb.position(id), None);
        nb.set_position(id, Some(Position::new(12.5, -3.0)));
        assert_eq!(nb.position(id), Some(Position::new(12.5, -3.0)));
        nb.set_position(id, None);
        assert_eq!(nb.position(id), None);
    }

    #[test]
    fn position_does_not_affect_digest() {
        let (mut nb, a, ..) = chain();
        let antes = nb.notebook_digest();
        nb.set_position(a, Some(Position::new(100.0, 200.0)));
        assert_eq!(antes, nb.notebook_digest());
    }
}
