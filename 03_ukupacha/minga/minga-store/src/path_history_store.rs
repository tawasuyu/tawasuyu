//! `SledPathHistoryStore`: historial cronológico de raíces ingeridas
//! desde cada path local.
//!
//! Para implementar `minga blame` necesitamos saber, dado un archivo,
//! la secuencia de α-hashes que fueron sus raíces a lo largo del
//! tiempo. Las atestaciones no llevan esta info (las firmas son sobre
//! contenido, no sobre path) y la transmisión por wire es por hash,
//! no por nombre — así que esta info es estrictamente **local** al
//! peer, como [`SledTimestampStore`].
//!
//! Layout: clave `path_bytes` (la cadena UTF-8 del path canonicalizado
//! por el caller), valor postcard-serializado `Vec<(ContentHash, u64)>`
//! en orden cronológico ascendente. Para cada ingesta sobre el mismo
//! path se re-serializa el vector entero — aceptable porque los
//! historiales son cortos (decenas de entradas como mucho).

use minga_core::ContentHash;
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledPathHistoryStore {
    tree: Tree,
}

impl SledPathHistoryStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    /// Anexa un par `(alpha, ts_secs)` al historial de `path`. Si el
    /// último α registrado en el historial es idéntico al recién
    /// ingerido (ingesta del mismo contenido sin cambios), NO se
    /// duplica — el historial sólo crece cuando el contenido cambia
    /// realmente. Esto evita ruido en `minga blame` cuando `watch`
    /// dispara múltiples eventos por una sola edición.
    pub fn append(
        &self,
        path: &str,
        alpha: ContentHash,
        ts_secs: u64,
    ) -> Result<(), StoreError> {
        let mut history = self.history(path)?;
        if let Some(last) = history.last() {
            if last.0 == alpha {
                return Ok(());
            }
        }
        history.push((alpha, ts_secs));
        let bytes = postcard::to_allocvec(&history)?;
        self.tree.insert(path.as_bytes(), bytes)?;
        Ok(())
    }

    /// Historial cronológico de `path`. Vec vacío si nunca se ingirió.
    pub fn history(&self, path: &str) -> Result<Vec<(ContentHash, u64)>, StoreError> {
        let Some(bytes) = self.tree.get(path.as_bytes())? else {
            return Ok(Vec::new());
        };
        Ok(postcard::from_bytes(&bytes)?)
    }

    /// Cuántos paths tienen historial registrado.
    pub fn len(&self) -> usize {
        self.tree.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    pub fn flush(&self) -> Result<(), StoreError> {
        self.tree.flush()?;
        Ok(())
    }

    /// Iterador sobre todos los paths con su historial completo.
    pub fn iter(
        &self,
    ) -> impl Iterator<Item = Result<(String, Vec<(ContentHash, u64)>), StoreError>> + '_ {
        self.tree.iter().map(|kv| {
            let (k, v) = kv?;
            let path = String::from_utf8(k.to_vec())
                .map_err(|_| StoreError::HashMismatch)?;
            let history: Vec<(ContentHash, u64)> = postcard::from_bytes(&v)?;
            Ok((path, history))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use minga_core::ContentHash;
    use tempfile::TempDir;

    fn open_temp_db() -> (sled::Db, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let cfg = sled::Config::default().path(dir.path()).temporary(true);
        let db = cfg.open().expect("sled open");
        (db, dir)
    }

    #[test]
    fn append_and_read_history() {
        let (db, _dir) = open_temp_db();
        let s = SledPathHistoryStore::open_tree(&db, "path_history").unwrap();
        let a1 = ContentHash([1u8; 32]);
        let a2 = ContentHash([2u8; 32]);
        s.append("src/main.rs", a1, 100).unwrap();
        s.append("src/main.rs", a2, 200).unwrap();
        let h = s.history("src/main.rs").unwrap();
        assert_eq!(h, vec![(a1, 100), (a2, 200)]);
    }

    #[test]
    fn append_skips_duplicate_tail() {
        // Re-ingerir el mismo α no debe duplicar la entrada.
        let (db, _dir) = open_temp_db();
        let s = SledPathHistoryStore::open_tree(&db, "path_history").unwrap();
        let a = ContentHash([42u8; 32]);
        s.append("x.rs", a, 1).unwrap();
        s.append("x.rs", a, 99).unwrap();
        let h = s.history("x.rs").unwrap();
        assert_eq!(h, vec![(a, 1)], "el segundo append debió ser no-op");
    }

    #[test]
    fn history_empty_for_unknown_path() {
        let (db, _dir) = open_temp_db();
        let s = SledPathHistoryStore::open_tree(&db, "path_history").unwrap();
        assert!(s.history("nope.rs").unwrap().is_empty());
    }
}
