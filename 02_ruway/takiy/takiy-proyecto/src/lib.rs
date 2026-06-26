//! `takiy-proyecto` — un proyecto de takiy con **versionado DAG**.
//!
//! Mismo patrón que `pluma-proyecto` (commits content-addressed con
//! padres, dedup por contenido, historia topo-ordenada, guardar/abrir a
//! un archivo), pero el documento versionado es un único [`Score`] en
//! lugar del haz multilienzo de pluma. Reusa la sustancia compartida:
//! `format::hash` (BLAKE3) + `postcard` para direccionar por contenido.
//!
//! Un proyecto tiene una **working copy** (`trabajo: Score`) que el editor
//! muta libremente; `push` sella un snapshot inmutable (un [`Commit`] que
//! apunta al hash del `Score` y a su padre). `checkout` trae un commit
//! viejo a la working copy. `historia` da el grafo de versiones para
//! pintarlo. Todo el DAG cabe en un solo archivo `.takiyproj`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use takiy_core::Score;

/// Hash content-addressed (BLAKE3, 32 bytes) — el mismo de `shared/format`.
pub type Hash = format::Hash;

/// Un nodo del DAG de versiones: apunta al `Score` sellado y a su(s)
/// padre(s). `padres` vacío = commit raíz; uno = lineal; ≥2 = merge
/// (no hay merge todavía, pero el formato lo admite, como en pluma).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commit {
    /// Hash del `Score` (blob postcard) que este commit congela.
    pub score: Hash,
    /// Padres en el DAG.
    pub padres: Vec<Hash>,
    /// Autor del snapshot.
    pub autor: String,
    /// Marca de tiempo (segundos Unix) — la pasa el llamador (no se mira
    /// el reloj acá, para que los tests sean deterministas).
    pub timestamp: u64,
    /// Mensaje de la versión.
    pub mensaje: String,
}

/// Errores del proyecto.
#[derive(Debug)]
pub enum ProyectoError {
    /// Falta un objeto referenciado por su hash (DAG incompleto).
    ObjetoFaltante(Hash),
    /// Falla de (de)serialización postcard.
    Serde(&'static str),
    /// Falla de E/S al guardar/abrir.
    Io(std::io::Error),
}

impl std::fmt::Display for ProyectoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ObjetoFaltante(_) => write!(f, "objeto faltante en el DAG"),
            Self::Serde(w) => write!(f, "serde: {w}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}
impl std::error::Error for ProyectoError {}
impl From<std::io::Error> for ProyectoError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Forma serializada del proyecto en disco (`.takiyproj`): el store de
/// objetos + las refs + la working copy actual.
#[derive(Serialize, Deserialize)]
struct Disco {
    nombre: String,
    objetos: BTreeMap<Hash, Vec<u8>>,
    head: Option<Hash>,
    /// `Score` de la working copy, serializado con postcard.
    trabajo: Vec<u8>,
}

/// Un proyecto de takiy: store content-addressed + HEAD + working copy.
#[derive(Clone)]
pub struct Proyecto {
    /// Nombre legible del proyecto.
    pub nombre: String,
    /// Store: hash → bytes postcard del objeto (commits y scores).
    objetos: BTreeMap<Hash, Vec<u8>>,
    /// Commit actual (la "rama" implícita). `None` = sin sellar todavía.
    head: Option<Hash>,
    /// Copia de trabajo editable — lo que el editor muta.
    trabajo: Score,
}

impl Proyecto {
    /// Proyecto nuevo con una working copy inicial (sin commits aún).
    pub fn nuevo(nombre: impl Into<String>, score: Score) -> Self {
        Self {
            nombre: nombre.into(),
            objetos: BTreeMap::new(),
            head: None,
            trabajo: score,
        }
    }

    /// La working copy (lo que se está editando).
    pub fn score(&self) -> &Score {
        &self.trabajo
    }

    /// Acceso mutable a la working copy.
    pub fn score_mut(&mut self) -> &mut Score {
        &mut self.trabajo
    }

    /// Reemplaza la working copy entera.
    pub fn set_score(&mut self, score: Score) {
        self.trabajo = score;
    }

    /// Commit actual (HEAD), si hay alguno sellado.
    pub fn head(&self) -> Option<Hash> {
        self.head
    }

    // ----- store content-addressed --------------------------------------

    fn put_bytes(&mut self, bytes: Vec<u8>) -> Hash {
        let h = format::hash(&bytes);
        self.objetos.entry(h).or_insert(bytes);
        h
    }

    fn put_obj<T: Serialize>(&mut self, obj: &T) -> Result<Hash, ProyectoError> {
        let bytes = postcard::to_allocvec(obj).map_err(|_| ProyectoError::Serde("serializar"))?;
        Ok(self.put_bytes(bytes))
    }

    fn get_obj<T: for<'de> Deserialize<'de>>(&self, h: &Hash) -> Result<T, ProyectoError> {
        let bytes = self.objetos.get(h).ok_or(ProyectoError::ObjetoFaltante(*h))?;
        postcard::from_bytes(bytes).map_err(|_| ProyectoError::Serde("deserializar"))
    }

    /// Lee un commit por hash.
    pub fn commit(&self, h: &Hash) -> Option<Commit> {
        self.get_obj(h).ok()
    }

    /// Lee el `Score` que un commit congela.
    pub fn score_de(&self, commit: &Commit) -> Option<Score> {
        self.get_obj(&commit.score).ok()
    }

    // ----- push / checkout ----------------------------------------------

    /// Sella la working copy como una versión nueva. Devuelve el hash del
    /// commit, o `None` si nada cambió respecto del HEAD (dedup por
    /// contenido: el `Score` produce el mismo hash).
    pub fn push(
        &mut self,
        autor: impl Into<String>,
        mensaje: impl Into<String>,
        timestamp: u64,
    ) -> Option<Hash> {
        let score_hash = self.put_obj(&self.trabajo.clone()).ok()?;
        // Dedup: si el HEAD ya congela este mismo Score, no-op.
        if let Some(p) = self.head {
            if let Some(c) = self.commit(&p) {
                if c.score == score_hash {
                    return None;
                }
            }
        }
        let commit = Commit {
            score: score_hash,
            padres: self.head.into_iter().collect(),
            autor: autor.into(),
            timestamp,
            mensaje: mensaje.into(),
        };
        let commit_hash = self.put_obj(&commit).ok()?;
        self.head = Some(commit_hash);
        Some(commit_hash)
    }

    /// Trae un commit viejo a la working copy y mueve el HEAD a él. La
    /// working copy previa NO se pierde si estaba sellada (sigue en el
    /// DAG); si tenía cambios sin sellar, sí se descartan.
    pub fn checkout(&mut self, commit: Hash) -> bool {
        let Some(c) = self.commit(&commit) else {
            return false;
        };
        let Some(score) = self.score_de(&c) else {
            return false;
        };
        self.trabajo = score;
        self.head = Some(commit);
        true
    }

    // ----- historia ------------------------------------------------------

    /// Commits alcanzables desde HEAD, en orden topológico (padres antes
    /// que hijos), con su hash. El grafo de versiones para pintar.
    pub fn historia(&self) -> Vec<(Hash, Commit)> {
        let mut visto: BTreeSet<Hash> = BTreeSet::new();
        let mut orden: Vec<Hash> = Vec::new();
        if let Some(h) = self.head {
            self.topo(h, &mut visto, &mut orden);
        }
        orden
            .into_iter()
            .filter_map(|h| self.commit(&h).map(|c| (h, c)))
            .collect()
    }

    fn topo(&self, h: Hash, visto: &mut BTreeSet<Hash>, orden: &mut Vec<Hash>) {
        if !visto.insert(h) {
            return;
        }
        if let Some(c) = self.commit(&h) {
            for p in &c.padres {
                self.topo(*p, visto, orden);
            }
        }
        orden.push(h);
    }

    /// Cantidad de versiones selladas.
    pub fn num_versiones(&self) -> usize {
        self.historia().len()
    }

    // ----- persistencia --------------------------------------------------

    /// Serializa el proyecto entero (DAG + working copy) a `path`.
    pub fn guardar(&self, path: impl AsRef<Path>) -> Result<(), ProyectoError> {
        let trabajo =
            postcard::to_allocvec(&self.trabajo).map_err(|_| ProyectoError::Serde("trabajo"))?;
        let disco = Disco {
            nombre: self.nombre.clone(),
            objetos: self.objetos.clone(),
            head: self.head,
            trabajo,
        };
        let bytes = postcard::to_allocvec(&disco).map_err(|_| ProyectoError::Serde("disco"))?;
        let p = path.as_ref();
        let tmp = p.with_extension("takiyproj.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, p)?;
        Ok(())
    }

    /// Abre un proyecto desde `path`.
    pub fn abrir(path: impl AsRef<Path>) -> Result<Proyecto, ProyectoError> {
        let bytes = std::fs::read(path.as_ref())?;
        let disco: Disco =
            postcard::from_bytes(&bytes).map_err(|_| ProyectoError::Serde("disco"))?;
        let trabajo: Score =
            postcard::from_bytes(&disco.trabajo).map_err(|_| ProyectoError::Serde("trabajo"))?;
        Ok(Proyecto {
            nombre: disco.nombre,
            objetos: disco.objetos,
            head: disco.head,
            trabajo,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use takiy_core::{Pitch, ScoreNote, Track};

    fn score_con(nota_beat: f32) -> Score {
        let mut s = Score::new(120.0);
        let mut t = Track::new("a");
        t.add(ScoreNote::new(Pitch::A4, nota_beat, 1.0, 100));
        s.add_track(t);
        s
    }

    #[test]
    fn push_seals_a_version_and_advances_head() {
        let mut p = Proyecto::nuevo("demo", score_con(0.0));
        assert!(p.head().is_none());
        let h = p.push("yo", "v1", 1).expect("primer commit");
        assert_eq!(p.head(), Some(h));
        assert_eq!(p.num_versiones(), 1);
    }

    #[test]
    fn push_dedups_when_nothing_changed() {
        let mut p = Proyecto::nuevo("demo", score_con(0.0));
        p.push("yo", "v1", 1).unwrap();
        // Sin cambios → no sella otra versión.
        assert!(p.push("yo", "otra vez", 2).is_none());
        assert_eq!(p.num_versiones(), 1);
    }

    #[test]
    fn history_is_linear_and_parented() {
        let mut p = Proyecto::nuevo("demo", score_con(0.0));
        let v1 = p.push("yo", "v1", 1).unwrap();
        // Cambiá la working copy y sellá otra.
        *p.score_mut() = score_con(2.0);
        let v2 = p.push("yo", "v2", 2).unwrap();
        let hist = p.historia();
        assert_eq!(hist.len(), 2);
        // Topo: el padre (v1) viene antes que el hijo (v2).
        assert_eq!(hist[0].0, v1);
        assert_eq!(hist[1].0, v2);
        assert_eq!(hist[1].1.padres, vec![v1]);
    }

    #[test]
    fn checkout_restores_an_old_score() {
        let mut p = Proyecto::nuevo("demo", score_con(0.0));
        let v1 = p.push("yo", "v1", 1).unwrap();
        *p.score_mut() = score_con(5.0);
        p.push("yo", "v2", 2).unwrap();
        // Volvé a v1: la working copy recupera la nota en beat 0.
        assert!(p.checkout(v1));
        assert_eq!(p.score().track(0).unwrap().notes()[0].start, 0.0);
        assert_eq!(p.head(), Some(v1));
    }

    #[test]
    fn save_and_open_roundtrip_preserves_dag_and_working_copy() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("takiy_proj_test_{}.takiyproj", std::process::id()));
        let mut p = Proyecto::nuevo("demo", score_con(0.0));
        let v1 = p.push("yo", "v1", 1).unwrap();
        *p.score_mut() = score_con(3.0);
        let v2 = p.push("yo", "v2", 2).unwrap();
        p.guardar(&path).unwrap();

        let q = Proyecto::abrir(&path).unwrap();
        assert_eq!(q.nombre, "demo");
        assert_eq!(q.head(), Some(v2));
        assert_eq!(q.num_versiones(), 2);
        assert!(q.commit(&v1).is_some());
        // working copy preservada (la nota en beat 3).
        assert_eq!(q.score().track(0).unwrap().notes()[0].start, 3.0);

        std::fs::remove_file(&path).ok();
    }
}
