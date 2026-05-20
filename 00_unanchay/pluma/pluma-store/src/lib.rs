//! `fana-store` — persistencia del grafo narrativo.
//!
//! Store key-value embebido (`sled`): cada `NarrativeAtom` se guarda con
//! su `Uuid` como clave y su serialización `bincode` como valor. El
//! grafo completo se reconstruye leyendo todos los átomos y re-cableando
//! la adjacency desde sus `dependencies`.

#![forbid(unsafe_code)]

use fana_core::NarrativeAtom;
use fana_graph::NarrativeGraph;
use uuid::Uuid;

/// Falla de una operación de store.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sled: {0}")]
    Db(#[from] sled::Error),
    #[error("serialización: {0}")]
    Serde(#[from] bincode::Error),
}

/// Store del grafo narrativo sobre sled.
pub struct GraphStore {
    db: sled::Db,
}

impl GraphStore {
    /// Abre (o crea) el store en `path`.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, StoreError> {
        Ok(Self { db: sled::open(path)? })
    }

    /// Guarda (o reemplaza) un átomo.
    pub fn put_atom(&self, atom: &NarrativeAtom) -> Result<(), StoreError> {
        let bytes = bincode::serialize(atom)?;
        self.db.insert(atom.id.as_bytes(), bytes)?;
        Ok(())
    }

    /// Lee un átomo por id.
    pub fn get_atom(&self, id: Uuid) -> Result<Option<NarrativeAtom>, StoreError> {
        match self.db.get(id.as_bytes())? {
            Some(bytes) => Ok(Some(bincode::deserialize(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Elimina un átomo.
    pub fn remove_atom(&self, id: Uuid) -> Result<(), StoreError> {
        self.db.remove(id.as_bytes())?;
        Ok(())
    }

    /// Cantidad de átomos persistidos.
    pub fn len(&self) -> usize {
        self.db.len()
    }

    pub fn is_empty(&self) -> bool {
        self.db.is_empty()
    }

    /// Persiste el grafo completo (un `put` por átomo).
    pub fn save_graph(&self, graph: &NarrativeGraph) -> Result<(), StoreError> {
        for atom in graph.atoms() {
            self.put_atom(atom)?;
        }
        self.db.flush()?;
        Ok(())
    }

    /// Reconstruye el grafo leyendo todos los átomos persistidos.
    pub fn load_graph(&self) -> Result<NarrativeGraph, StoreError> {
        let mut atoms = Vec::with_capacity(self.db.len());
        for entry in self.db.iter() {
            let (_, bytes) = entry?;
            atoms.push(bincode::deserialize::<NarrativeAtom>(&bytes)?);
        }
        Ok(NarrativeGraph::from_atoms(atoms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (GraphStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = GraphStore::open(dir.path().join("fana.sled")).unwrap();
        (store, dir)
    }

    #[test]
    fn put_get_remove_roundtrip() {
        let (store, _d) = temp_store();
        let atom = NarrativeAtom::new("capítulo uno", "main");
        let id = atom.id;
        store.put_atom(&atom).unwrap();
        let loaded = store.get_atom(id).unwrap().expect("debe existir");
        assert_eq!(loaded.id, id);
        assert_eq!(*loaded.content, *atom.content);
        assert!(loaded.hash_matches());
        store.remove_atom(id).unwrap();
        assert!(store.get_atom(id).unwrap().is_none());
    }

    #[test]
    fn save_and_load_graph_preserves_structure() {
        let (store, _d) = temp_store();
        let a = NarrativeAtom::new("a", "main");
        let b = NarrativeAtom::new("b", "main").depends_on(a.id);
        let (a_id, b_id) = (a.id, b.id);
        let mut g = NarrativeGraph::new();
        g.insert(a);
        g.insert(b);
        store.save_graph(&g).unwrap();

        let reloaded = store.load_graph().unwrap();
        assert_eq!(reloaded.len(), 2);
        assert!(reloaded.contains(a_id) && reloaded.contains(b_id));
        // La adjacency se reconstruye desde las dependencies.
        assert_eq!(reloaded.dependents(a_id), &[b_id]);
    }
}
