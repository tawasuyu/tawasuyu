//! `SledAlphaPathsStore`: índice inverso α-hash → paths conocidos.
//!
//! El `SledPathHistoryStore` indexa por path (clave directa); para
//! consultar "qué paths existen para esta raíz" había que escanear todo
//! ese tree y reconstruir el reverso en RAM (lo que hacía `cmd_roots`).
//! Cuando el repo crece a millones de paths esa pasada se vuelve
//! prohibitiva.
//!
//! Layout: clave `[α (32 bytes)] [path bytes]`, valor `ts_secs` u64
//! big-endian. Como la α va primero, una iteración con prefijo `α` da
//! todos los paths de esa raíz sin tocar el resto del tree. Los paths
//! quedan ordenados lexicográficamente — irrelevante para el caso de
//! uso pero estable.
//!
//! Este store es **local** al peer, igual que `path_history` y
//! `attestation_timestamps`: los paths no viajan por wire.

use minga_core::ContentHash;
use sled::{Db, Tree};

use crate::error::StoreError;

pub struct SledAlphaPathsStore {
    tree: Tree,
}

impl SledAlphaPathsStore {
    pub fn open_tree(db: &Db, name: &str) -> Result<Self, StoreError> {
        Ok(Self {
            tree: db.open_tree(name)?,
        })
    }

    /// Registra (o actualiza con un timestamp más reciente) la
    /// asociación α → path. Idempotente: re-registrar con ts más viejo
    /// no pisa.
    pub fn record(&self, alpha: ContentHash, path: &str, ts_secs: u64) -> Result<(), StoreError> {
        let key = compose_key(&alpha, path);
        if let Some(prev) = self.tree.get(&key)? {
            if prev.len() == 8 {
                let mut buf = [0u8; 8];
                buf.copy_from_slice(&prev);
                if u64::from_be_bytes(buf) >= ts_secs {
                    return Ok(());
                }
            }
        }
        self.tree.insert(key, ts_secs.to_be_bytes().to_vec())?;
        Ok(())
    }

    /// Lista los paths registrados para `alpha`, junto con su ts. Vec
    /// vacío si la raíz no tiene paths locales.
    pub fn paths_for(&self, alpha: &ContentHash) -> Result<Vec<(String, u64)>, StoreError> {
        let mut out = Vec::new();
        for entry in self.tree.scan_prefix(alpha.0) {
            let (k, v) = entry?;
            if k.len() < 32 || v.len() != 8 {
                continue;
            }
            let path = String::from_utf8(k[32..].to_vec())
                .map_err(|_| StoreError::HashMismatch)?;
            let mut ts = [0u8; 8];
            ts.copy_from_slice(&v);
            out.push((path, u64::from_be_bytes(ts)));
        }
        Ok(out)
    }

    /// Devuelve el path con timestamp más reciente para `alpha`, o
    /// `None`. Conveniencia para callers que sólo quieren "el path
    /// canónico" (típicamente, el último usado).
    pub fn most_recent_path(&self, alpha: &ContentHash) -> Result<Option<String>, StoreError> {
        let mut best: Option<(String, u64)> = None;
        for (p, ts) in self.paths_for(alpha)? {
            match &best {
                Some((_, bts)) if *bts >= ts => {}
                _ => best = Some((p, ts)),
            }
        }
        Ok(best.map(|(p, _)| p))
    }

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
}

fn compose_key(alpha: &ContentHash, path: &str) -> Vec<u8> {
    let p = path.as_bytes();
    let mut k = Vec::with_capacity(32 + p.len());
    k.extend_from_slice(&alpha.0);
    k.extend_from_slice(p);
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_temp_db() -> (Db, TempDir) {
        let dir = TempDir::new().unwrap();
        let cfg = sled::Config::default().path(dir.path()).temporary(true);
        let db = cfg.open().unwrap();
        (db, dir)
    }

    #[test]
    fn record_and_query_single_alpha() {
        let (db, _d) = open_temp_db();
        let s = SledAlphaPathsStore::open_tree(&db, "alpha_paths").unwrap();
        let a = ContentHash([7u8; 32]);
        s.record(a, "src/main.rs", 100).unwrap();
        s.record(a, "lib/x.rs", 200).unwrap();
        let mut paths = s.paths_for(&a).unwrap();
        paths.sort();
        assert_eq!(
            paths,
            vec![("lib/x.rs".to_string(), 200), ("src/main.rs".to_string(), 100)]
        );
    }

    #[test]
    fn most_recent_returns_latest_ts() {
        let (db, _d) = open_temp_db();
        let s = SledAlphaPathsStore::open_tree(&db, "alpha_paths").unwrap();
        let a = ContentHash([9u8; 32]);
        s.record(a, "a", 10).unwrap();
        s.record(a, "b", 50).unwrap();
        s.record(a, "c", 20).unwrap();
        assert_eq!(s.most_recent_path(&a).unwrap(), Some("b".to_string()));
    }

    #[test]
    fn record_does_not_regress_timestamp() {
        let (db, _d) = open_temp_db();
        let s = SledAlphaPathsStore::open_tree(&db, "alpha_paths").unwrap();
        let a = ContentHash([1u8; 32]);
        s.record(a, "x", 100).unwrap();
        s.record(a, "x", 50).unwrap();
        let paths = s.paths_for(&a).unwrap();
        assert_eq!(paths, vec![("x".to_string(), 100)]);
    }

    #[test]
    fn scan_does_not_leak_across_alphas() {
        let (db, _d) = open_temp_db();
        let s = SledAlphaPathsStore::open_tree(&db, "alpha_paths").unwrap();
        let a = ContentHash([1u8; 32]);
        let b = ContentHash([2u8; 32]);
        s.record(a, "p", 1).unwrap();
        s.record(b, "p", 1).unwrap();
        assert_eq!(s.paths_for(&a).unwrap().len(), 1);
        assert_eq!(s.paths_for(&b).unwrap().len(), 1);
    }
}
