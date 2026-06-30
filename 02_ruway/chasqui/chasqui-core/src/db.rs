//! DB de Mónadas y archivos. Backend dual:
//!
//! - **Memoria** (default, cache): `BTreeMap<Id, T>` para reads O(log n).
//! - **Persistencia** (opcional): sled-backed write-through. Si se abre
//!   con `MonadDb::open(path)`, cada `insert_*` escribe a sled además
//!   de la cache. Reads siempre vienen de la cache.
//!
//! Wire format: JSON via serde_json. Los manifestos son chicos y
//! ocasionalmente inspeccionables a mano (`sled-cli`); JSON gana sobre
//! postcard en debuggability.

use std::collections::BTreeMap;
use std::path::Path;

use chasqui_card::{FileEntry, FileId, MonadId, MonadManifest};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MonadDbError {
    #[error("sled: {0}")]
    Sled(#[from] sled::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("ULID inválido en clave: {0}")]
    BadKey(String),
}

const TREE_FILES: &str = "files";
const TREE_MONADS: &str = "monads";

/// Store de Mónadas + archivos. Cache en memoria + persistencia
/// opcional sled.
pub struct MonadDb {
    files: BTreeMap<FileId, FileEntry>,
    monads: BTreeMap<MonadId, MonadManifest>,
    persistence: Option<sled::Db>,
}

impl Default for MonadDb {
    fn default() -> Self {
        Self::new()
    }
}

impl MonadDb {
    /// Store en memoria pura (sin persistencia). El estado se pierde al salir.
    pub fn new() -> Self {
        Self {
            files: BTreeMap::new(),
            monads: BTreeMap::new(),
            persistence: None,
        }
    }

    /// Abre (o crea) un store sled-backed en `path`. Carga el contenido
    /// existente a la cache antes de devolver.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MonadDbError> {
        let db = sled::open(path)?;
        let mut files = BTreeMap::new();
        let mut monads = BTreeMap::new();

        let files_tree = db.open_tree(TREE_FILES)?;
        for kv in files_tree.iter() {
            let (k, v) = kv?;
            let id = decode_key(&k)?;
            let entry: FileEntry = serde_json::from_slice(&v)?;
            files.insert(id, entry);
        }
        let monads_tree = db.open_tree(TREE_MONADS)?;
        for kv in monads_tree.iter() {
            let (k, v) = kv?;
            let id = decode_key(&k)?;
            let monad: MonadManifest = serde_json::from_slice(&v)?;
            monads.insert(id, monad);
        }

        Ok(Self {
            files,
            monads,
            persistence: Some(db),
        })
    }

    /// `true` si tiene backend persistente.
    pub fn is_persistent(&self) -> bool {
        self.persistence.is_some()
    }

    // ---- Files ----

    pub fn insert_file(&mut self, file: FileEntry) -> Option<FileEntry> {
        if let Some(db) = &self.persistence {
            // Write-through: si falla el persist, lo logeamos pero la
            // memoria queda actualizada. Filosofía: cache nunca miente
            // sobre el último estado conocido en este proceso.
            if let Err(e) = persist_file(db, &file) {
                eprintln!("[MonadDb] persist file falló: {e}");
            }
        }
        self.files.insert(file.id, file)
    }

    pub fn ingest_files(&mut self, files: Vec<FileEntry>) {
        for f in files {
            self.insert_file(f);
        }
    }

    pub fn file(&self, id: FileId) -> Option<&FileEntry> {
        self.files.get(&id)
    }

    pub fn files(&self) -> impl Iterator<Item = &FileEntry> + '_ {
        self.files.values()
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    // ---- Monads ----

    pub fn insert_monad(&mut self, monad: MonadManifest) -> Option<MonadManifest> {
        if let Some(db) = &self.persistence {
            if let Err(e) = persist_monad(db, &monad) {
                eprintln!("[MonadDb] persist monad falló: {e}");
            }
        }
        self.monads.insert(monad.id, monad)
    }

    pub fn replace_monads(&mut self, monads: Vec<MonadManifest>) {
        // Si hay persistencia, limpiar tree antes de insertar.
        if let Some(db) = &self.persistence {
            if let Ok(tree) = db.open_tree(TREE_MONADS) {
                let _ = tree.clear();
            }
        }
        self.monads.clear();
        for m in monads {
            self.insert_monad(m);
        }
    }

    pub fn monad(&self, id: MonadId) -> Option<&MonadManifest> {
        self.monads.get(&id)
    }

    /// Elimina una Mónada del store (cache + persistencia). Devuelve el
    /// manifiesto removido, o `None` si no existía. No toca a quién la
    /// referenciaba como sub-Mónada — eso es responsabilidad de la capa
    /// de edición (`crate::edit`), que mantiene la coherencia del grafo.
    pub fn remove_monad(&mut self, id: MonadId) -> Option<MonadManifest> {
        if let Some(db) = &self.persistence {
            if let Ok(tree) = db.open_tree(TREE_MONADS) {
                let _ = tree.remove(id.to_string().as_bytes());
            }
        }
        self.monads.remove(&id)
    }

    pub fn monads(&self) -> impl Iterator<Item = &MonadManifest> + '_ {
        self.monads.values()
    }

    pub fn monad_count(&self) -> usize {
        self.monads.len()
    }

    /// Resuelve los archivos miembros de una Mónada como referencias.
    /// Skipea silenciosamente IDs que ya no estén en la tabla `files`.
    pub fn resolve_members(&self, monad_id: MonadId) -> Vec<&FileEntry> {
        match self.monads.get(&monad_id) {
            Some(m) => m.members.iter().filter_map(|id| self.files.get(id)).collect(),
            None => Vec::new(),
        }
    }
}

fn persist_file(db: &sled::Db, f: &FileEntry) -> Result<(), MonadDbError> {
    let tree = db.open_tree(TREE_FILES)?;
    let key = f.id.to_string();
    let val = serde_json::to_vec(f)?;
    tree.insert(key.as_bytes(), val)?;
    Ok(())
}

fn persist_monad(db: &sled::Db, m: &MonadManifest) -> Result<(), MonadDbError> {
    let tree = db.open_tree(TREE_MONADS)?;
    let key = m.id.to_string();
    let val = serde_json::to_vec(m)?;
    tree.insert(key.as_bytes(), val)?;
    Ok(())
}

fn decode_key(k: &[u8]) -> Result<ulid::Ulid, MonadDbError> {
    let s = std::str::from_utf8(k).map_err(|_| MonadDbError::BadKey(format!("{:?}", k)))?;
    ulid::Ulid::from_string(s).map_err(|_| MonadDbError::BadKey(s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chasqui_card::Lens;
    use ulid::Ulid;

    fn mk_file(path: &str) -> FileEntry {
        FileEntry {
            id: FileId::from(Ulid::new()),
            path: std::path::PathBuf::from(path),
            content_hash: None,
            size: 100,
            mtime_ms: 0,
            extension: Some("rs".into()),
        }
    }

    #[test]
    fn ingest_and_lookup() {
        let mut db = MonadDb::new();
        let f1 = mk_file("/a/x.rs");
        let f2 = mk_file("/a/y.rs");
        let id1 = f1.id;
        db.ingest_files(vec![f1, f2]);
        assert_eq!(db.file_count(), 2);
        assert!(db.file(id1).is_some());
        assert!(!db.is_persistent());
    }

    #[test]
    fn resolve_members_filters_missing() {
        let mut db = MonadDb::new();
        let f1 = mk_file("/x/a.rs");
        let id1 = f1.id;
        db.insert_file(f1);

        let mut m = MonadManifest::new("test");
        m.members.insert(id1);
        m.members.insert(FileId::from(Ulid::new())); // miembro fantasma
        m.dominant_lens = Lens::Code;
        m.touch();

        let mid = m.id;
        db.insert_monad(m);

        let resolved = db.resolve_members(mid);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].id, id1);
    }

    #[test]
    fn replace_monads_clears_old() {
        let mut db = MonadDb::new();
        let mut m1 = MonadManifest::new("a");
        m1.members.insert(FileId::from(Ulid::new()));
        m1.touch();
        db.insert_monad(m1);
        assert_eq!(db.monad_count(), 1);

        let mut m2 = MonadManifest::new("b");
        m2.members.insert(FileId::from(Ulid::new()));
        m2.touch();
        db.replace_monads(vec![m2]);
        assert_eq!(db.monad_count(), 1);
        assert!(db.monads().next().unwrap().label == "b");
    }

    #[test]
    fn persistence_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dbpath = tmp.path().join("monads.sled");

        // Escribimos algunos datos
        {
            let mut db = MonadDb::open(&dbpath).expect("open");
            assert!(db.is_persistent());
            let f = mk_file("/persist/a.rs");
            let fid = f.id;
            db.insert_file(f);

            let mut m = MonadManifest::new("persist-test");
            m.members.insert(fid);
            m.dominant_lens = Lens::Code;
            m.touch();
            db.insert_monad(m);
        }

        // Reabrimos y verificamos que están
        let db = MonadDb::open(&dbpath).expect("reopen");
        assert_eq!(db.file_count(), 1);
        assert_eq!(db.monad_count(), 1);
        let m = db.monads().next().unwrap();
        assert_eq!(m.label, "persist-test");
        assert_eq!(m.cardinality, 1);
    }

    #[test]
    fn replace_monads_purges_persistent_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let dbpath = tmp.path().join("replace.sled");

        {
            let mut db = MonadDb::open(&dbpath).unwrap();
            let mut m1 = MonadManifest::new("old");
            m1.members.insert(FileId::from(Ulid::new()));
            m1.touch();
            db.insert_monad(m1);
        }

        // Reabrir, replace, verificar
        {
            let mut db = MonadDb::open(&dbpath).unwrap();
            assert_eq!(db.monad_count(), 1);
            let mut m2 = MonadManifest::new("new");
            m2.members.insert(FileId::from(Ulid::new()));
            m2.touch();
            db.replace_monads(vec![m2]);
            assert_eq!(db.monad_count(), 1);
        }

        // Tercera apertura: sólo "new" sobrevive
        let db = MonadDb::open(&dbpath).unwrap();
        assert_eq!(db.monad_count(), 1);
        assert_eq!(db.monads().next().unwrap().label, "new");
    }
}
