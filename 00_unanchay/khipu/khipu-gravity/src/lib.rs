//! `badu-gravity` — la gravedad semántica de las notas.
//!
//! Cada nota tiene un vector semántico (lo produce `verbo`; aquí entra
//! ya calculado, sin acoplar a ningún backend). La afinidad entre dos
//! notas es la similitud coseno de sus vectores; con eso, este crate:
//!
//! - encuentra los **vecinos** más afines de una nota;
//! - agrupa las notas en **clústeres** por encima de un umbral;
//! - calcula un **layout 2D** donde las notas afines se atraen y todas
//!   se repelen — la «gravedad» literal de la lente espacial de badu.
//!
//! Todo es determinista: posiciones iniciales fijas, sin RNG, iteración
//! en orden estable.

#![forbid(unsafe_code)]

use badu_core::NoteId;
use serde::{Deserialize, Serialize};

/// Una nube de notas con su vector semántico — el dominio de la gravedad.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticField {
    /// `(id, vector)` en orden de inserción.
    entries: Vec<(NoteId, Vec<f32>)>,
}

/// Posición 2D resultante de una nota tras el layout por gravedad.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NotePlacement {
    pub id: NoteId,
    pub x: f32,
    pub y: f32,
}

/// Parámetros del layout dirigido por fuerzas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GravityConfig {
    /// Pasos de relajación.
    pub iterations: usize,
    /// Fuerza de atracción entre notas afines.
    pub attraction: f32,
    /// Fuerza de repulsión entre todo par de notas.
    pub repulsion: f32,
    /// Radio del círculo de posiciones iniciales.
    pub radius: f32,
    /// Fracción de la fuerza neta que se aplica por paso (amortiguación).
    pub step: f32,
}

impl Default for GravityConfig {
    fn default() -> Self {
        Self {
            iterations: 120,
            attraction: 0.02,
            repulsion: 800.0,
            radius: 240.0,
            step: 0.85,
        }
    }
}

/// Similitud coseno de dos vectores. `None` si difieren de largo.
fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return Some(0.0);
    }
    Some((dot / (na * nb)).clamp(-1.0, 1.0))
}

impl SemanticField {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cantidad de notas en el campo.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Inserta o reemplaza el vector de una nota.
    pub fn insert(&mut self, id: NoteId, vector: Vec<f32>) {
        if let Some(slot) = self.entries.iter_mut().find(|(eid, _)| *eid == id) {
            slot.1 = vector;
        } else {
            self.entries.push((id, vector));
        }
    }

    fn vector_of(&self, id: NoteId) -> Option<&[f32]> {
        self.entries
            .iter()
            .find(|(eid, _)| *eid == id)
            .map(|(_, v)| v.as_slice())
    }

    /// Afinidad (similitud coseno) entre dos notas. `None` si alguna no
    /// existe o los vectores difieren de largo.
    pub fn affinity(&self, a: NoteId, b: NoteId) -> Option<f32> {
        cosine(self.vector_of(a)?, self.vector_of(b)?)
    }

    /// Las `k` notas más afines a `id`, de mayor a menor afinidad.
    /// Empata por id ascendente para que el orden sea determinista.
    pub fn nearest(&self, id: NoteId, k: usize) -> Vec<(NoteId, f32)> {
        let Some(base) = self.vector_of(id) else {
            return Vec::new();
        };
        let mut scored: Vec<(NoteId, f32)> = self
            .entries
            .iter()
            .filter(|(eid, _)| *eid != id)
            .filter_map(|(eid, v)| cosine(base, v).map(|s| (*eid, s)))
            .collect();
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(core::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }

    /// Agrupa las notas en clústeres: dos notas quedan en el mismo grupo
    /// si su afinidad alcanza `threshold` (transitivamente). Cada
    /// clúster viene ordenado por id, y la lista de clústeres también.
    pub fn clusters(&self, threshold: f32) -> Vec<Vec<NoteId>> {
        let n = self.entries.len();
        let mut parent: Vec<usize> = (0..n).collect();

        fn find(parent: &mut [usize], i: usize) -> usize {
            let mut root = i;
            while parent[root] != root {
                root = parent[root];
            }
            let mut cur = i;
            while parent[cur] != root {
                let next = parent[cur];
                parent[cur] = root;
                cur = next;
            }
            root
        }

        for i in 0..n {
            for j in (i + 1)..n {
                let sim = cosine(&self.entries[i].1, &self.entries[j].1).unwrap_or(0.0);
                if sim >= threshold {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }

        let mut groups: std::collections::BTreeMap<usize, Vec<NoteId>> = Default::default();
        for i in 0..n {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().push(self.entries[i].0);
        }
        let mut out: Vec<Vec<NoteId>> = groups.into_values().collect();
        for c in &mut out {
            c.sort_unstable();
        }
        out.sort_by(|a, b| a.first().cmp(&b.first()));
        out
    }

    /// Layout 2D por gravedad: las notas afines se atraen, todas se
    /// repelen. Determinista — posiciones iniciales en círculo, sin RNG.
    pub fn gravity_layout(&self, cfg: &GravityConfig) -> Vec<NotePlacement> {
        let n = self.entries.len();
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![NotePlacement { id: self.entries[0].0, x: 0.0, y: 0.0 }];
        }

        // Posiciones iniciales repartidas en un círculo.
        let mut pos: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let a = core::f32::consts::TAU * i as f32 / n as f32;
                (cfg.radius * a.cos(), cfg.radius * a.sin())
            })
            .collect();

        // Afinidades precomputadas (no cambian entre pasos).
        let mut aff = vec![0.0f32; n * n];
        for i in 0..n {
            for j in (i + 1)..n {
                let s = cosine(&self.entries[i].1, &self.entries[j].1)
                    .unwrap_or(0.0)
                    .max(0.0);
                aff[i * n + j] = s;
                aff[j * n + i] = s;
            }
        }

        const EPS: f32 = 0.001;
        for _ in 0..cfg.iterations {
            let mut force = vec![(0.0f32, 0.0f32); n];
            for i in 0..n {
                for j in (i + 1)..n {
                    let dx = pos[j].0 - pos[i].0;
                    let dy = pos[j].1 - pos[i].1;
                    let dist = (dx * dx + dy * dy).sqrt().max(EPS);
                    let (ux, uy) = (dx / dist, dy / dist);
                    // Atracción crece con la distancia y la afinidad;
                    // repulsión cae con el cuadrado de la distancia.
                    let attract = cfg.attraction * aff[i * n + j] * dist;
                    let repel = cfg.repulsion / (dist * dist);
                    let net = attract - repel; // >0 → acercar
                    force[i].0 += net * ux;
                    force[i].1 += net * uy;
                    force[j].0 -= net * ux;
                    force[j].1 -= net * uy;
                }
            }
            for i in 0..n {
                pos[i].0 += force[i].0 * cfg.step;
                pos[i].1 += force[i].1 * cfg.step;
            }
        }

        self.entries
            .iter()
            .zip(pos)
            .map(|((id, _), (x, y))| NotePlacement { id: *id, x, y })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tres vectores: 1 y 2 casi paralelos, 3 ortogonal.
    fn field() -> SemanticField {
        let mut f = SemanticField::new();
        f.insert(1, vec![1.0, 0.0, 0.0]);
        f.insert(2, vec![0.9, 0.1, 0.0]);
        f.insert(3, vec![0.0, 0.0, 1.0]);
        f
    }

    #[test]
    fn affinity_is_high_for_aligned_vectors() {
        let f = field();
        let near = f.affinity(1, 2).unwrap();
        let far = f.affinity(1, 3).unwrap();
        assert!(near > 0.95);
        assert!(far.abs() < 1e-6);
        assert!(near > far);
    }

    #[test]
    fn affinity_missing_note_is_none() {
        assert!(field().affinity(1, 99).is_none());
    }

    #[test]
    fn nearest_ranks_by_affinity() {
        let f = field();
        let near = f.nearest(1, 2);
        assert_eq!(near[0].0, 2); // el más afín a 1
        assert_eq!(near.len(), 2);
        assert!(near[0].1 > near[1].1);
    }

    #[test]
    fn insert_replaces_existing_vector() {
        let mut f = SemanticField::new();
        f.insert(1, vec![1.0, 0.0]);
        f.insert(1, vec![0.0, 1.0]);
        assert_eq!(f.len(), 1);
        assert_eq!(f.vector_of(1), Some([0.0, 1.0].as_slice()));
    }

    #[test]
    fn clusters_group_affine_notes() {
        let f = field();
        // Umbral alto: 1 y 2 juntos, 3 solo.
        let cs = f.clusters(0.8);
        assert_eq!(cs, vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn low_threshold_merges_everything() {
        let cs = field().clusters(-1.0);
        assert_eq!(cs, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn gravity_layout_places_every_note() {
        let placements = field().gravity_layout(&GravityConfig::default());
        assert_eq!(placements.len(), 3);
        let ids: Vec<_> = placements.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn gravity_pulls_affine_notes_closer() {
        let f = field();
        let p = f.gravity_layout(&GravityConfig::default());
        let dist = |a: NoteId, b: NoteId| {
            let pa = p.iter().find(|x| x.id == a).unwrap();
            let pb = p.iter().find(|x| x.id == b).unwrap();
            ((pa.x - pb.x).powi(2) + (pa.y - pb.y).powi(2)).sqrt()
        };
        // Las notas afines (1,2) terminan más cerca que las disímiles (1,3).
        assert!(dist(1, 2) < dist(1, 3));
    }

    #[test]
    fn gravity_layout_is_deterministic() {
        let f = field();
        let a = f.gravity_layout(&GravityConfig::default());
        let b = f.gravity_layout(&GravityConfig::default());
        assert_eq!(a, b);
    }

    #[test]
    fn empty_and_single_fields_are_handled() {
        assert!(SemanticField::new().gravity_layout(&GravityConfig::default()).is_empty());
        let mut one = SemanticField::new();
        one.insert(7, vec![1.0, 1.0]);
        let p = one.gravity_layout(&GravityConfig::default());
        assert_eq!(p, vec![NotePlacement { id: 7, x: 0.0, y: 0.0 }]);
    }
}
