//! `KnownPeers` — el "authorized_keys" del daemon shuma.
//!
//! Formato: un archivo de texto, una línea por pubkey hex (con `#` para
//! comentarios y líneas vacías ignoradas). Mismo espíritu que
//! `~/.ssh/authorized_keys` pero a propósito incompatible: no es SSH y
//! no queremos que un copy/paste accidental cruce dominios.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::identity::{KeypairError, PublicKey};

/// Allowlist de pubkeys que el servidor acepta.
#[derive(Debug, Clone, Default)]
pub struct KnownPeers {
    keys: HashSet<PublicKey>,
}

impl KnownPeers {
    /// Vacío — útil para tests y para el primer arranque.
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` si `key` está en la allowlist.
    pub fn contains(&self, key: &PublicKey) -> bool {
        self.keys.contains(key)
    }

    /// Añade una clave a la allowlist. `true` si era nueva.
    pub fn add(&mut self, key: PublicKey) -> bool {
        self.keys.insert(key)
    }

    /// Quita una clave. `true` si existía.
    pub fn remove(&mut self, key: &PublicKey) -> bool {
        self.keys.remove(key)
    }

    /// Cantidad de claves confiables.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` si no hay ninguna clave confiable.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Itera las claves en orden no especificado.
    pub fn iter(&self) -> impl Iterator<Item = &PublicKey> {
        self.keys.iter()
    }

    /// Path canónico: `~/.config/shuma/known_peers.txt`. `None` si el
    /// SO no expone un directorio de configuración.
    pub fn default_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "shuma")
            .map(|d| d.config_dir().join("known_peers.txt"))
    }

    /// Carga el archivo si existe, o devuelve un set vacío. Líneas que
    /// empiezan con `#`, líneas en blanco y líneas con hex mal formado
    /// se ignoran silenciosamente — *no* abortamos por una entrada
    /// vieja o tipeada a mano.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, KeypairError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = fs::read_to_string(path)
            .map_err(|e| KeypairError::Io(path.to_path_buf(), e))?;
        let mut set = HashSet::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            // Permitimos `<hex> [comentario opcional]`.
            let token = trimmed.split_whitespace().next().unwrap_or("");
            if let Ok(k) = PublicKey::from_hex(token) {
                set.insert(k);
            }
        }
        Ok(Self { keys: set })
    }

    /// Guarda el set en `path` (`0600` en Unix), una clave por línea
    /// con encabezado humano. Crea el padre si falta. Reescritura
    /// atómica (escribe `<path>.tmp` y `rename`).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), KeypairError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| KeypairError::Io(parent.to_path_buf(), e))?;
        }
        let tmp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|s| s.to_str()).unwrap_or("txt")
        ));
        let mut keys: Vec<&PublicKey> = self.keys.iter().collect();
        keys.sort_by(|a, b| a.0.cmp(&b.0));
        let mut buf = String::new();
        buf.push_str("# shuma-link known peers — una pubkey X25519 hex por línea.\n");
        buf.push_str("# Líneas en blanco y `#` se ignoran al cargar.\n");
        for k in keys {
            buf.push_str(&k.to_hex());
            buf.push('\n');
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| KeypairError::Io(tmp.clone(), e))?;
            f.write_all(buf.as_bytes())
                .map_err(|e| KeypairError::Io(tmp.clone(), e))?;
            f.flush().map_err(|e| KeypairError::Io(tmp.clone(), e))?;
        }
        #[cfg(not(unix))]
        {
            fs::write(&tmp, buf.as_bytes())
                .map_err(|e| KeypairError::Io(tmp.clone(), e))?;
        }
        fs::rename(&tmp, path).map_err(|e| KeypairError::Io(path.to_path_buf(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Keypair;
    use tempfile::tempdir;

    #[test]
    fn add_contains_remove() {
        let mut p = KnownPeers::new();
        let k = Keypair::generate().unwrap().public();
        assert!(!p.contains(&k));
        assert!(p.add(k));
        assert!(p.contains(&k));
        assert!(!p.add(k)); // segundo add: no es nuevo
        assert_eq!(p.len(), 1);
        assert!(p.remove(&k));
        assert!(!p.contains(&k));
    }

    #[test]
    fn round_trip_through_disk() {
        let d = tempdir().unwrap();
        let path = d.path().join("known.txt");
        let mut p = KnownPeers::new();
        let a = Keypair::generate().unwrap().public();
        let b = Keypair::generate().unwrap().public();
        p.add(a);
        p.add(b);
        p.save(&path).unwrap();
        let back = KnownPeers::load(&path).unwrap();
        assert!(back.contains(&a));
        assert!(back.contains(&b));
        assert_eq!(back.len(), 2);
    }

    #[test]
    fn missing_file_loads_empty() {
        let d = tempdir().unwrap();
        let back = KnownPeers::load(d.path().join("nope.txt")).unwrap();
        assert!(back.is_empty());
    }

    #[test]
    fn comments_and_blanks_are_skipped() {
        let d = tempdir().unwrap();
        let path = d.path().join("known.txt");
        let k = Keypair::generate().unwrap().public();
        let body = format!(
            "# comentario\n\n  # otro\n{}  # nota al lado\n",
            k.to_hex()
        );
        std::fs::write(&path, body).unwrap();
        let p = KnownPeers::load(&path).unwrap();
        assert_eq!(p.len(), 1);
        assert!(p.contains(&k));
    }

    #[test]
    fn malformed_lines_are_silently_dropped() {
        let d = tempdir().unwrap();
        let path = d.path().join("known.txt");
        let k = Keypair::generate().unwrap().public();
        let body = format!("not-hex-at-all\n{}\nfaa\n", k.to_hex());
        std::fs::write(&path, body).unwrap();
        let p = KnownPeers::load(&path).unwrap();
        assert_eq!(p.len(), 1, "sólo la línea bien formada debe contar");
    }
}
