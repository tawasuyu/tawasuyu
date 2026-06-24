//! Direccionamiento por contenido (CAS) — **vendorizado** de
//! `hammer-core::hash` (`/mnt/vvv/hammer/crates/hammer-core/src/hash.rs`).
//!
//! Copiamos el tipo en vez de depender del crate para no acoplar los builds
//! de los dos repos (decisión "híbrido": formato idéntico, sin dep de
//! compilación). El **formato del hash es el mismo** (`b3:<hex>` sobre BLAKE3
//! con length-prefijado) para que un día converjan bajo el CAS unificado
//! (ADR 0007). Si cambia el algoritmo allá, hay que espejarlo acá.

use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Hash BLAKE3 de un artefacto, con prefijo legible `b3:`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactHash(String);

impl ArtifactHash {
    /// Construye desde una representación hex ya calculada.
    pub fn from_hex(hex: impl Into<String>) -> Self {
        ArtifactHash(format!("b3:{}", hex.into()))
    }

    /// Hashea un conjunto ordenado de entradas. El llamador es responsable de
    /// pasarlas en orden canónico y estable.
    pub fn of_inputs(inputs: &[&[u8]]) -> Self {
        let mut hasher = blake3::Hasher::new();
        for chunk in inputs {
            // length-prefijado: evita colisiones por concatenación ambigua.
            hasher.update(&(chunk.len() as u64).to_le_bytes());
            hasher.update(chunk);
        }
        ArtifactHash(format!("b3:{}", hasher.finalize().to_hex()))
    }

    /// Hash de un bloque de bytes — el caso del binario de una app.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        ArtifactHash(format!("b3:{}", blake3::hash(bytes).to_hex()))
    }

    /// Hash de un archivo en disco (lee su contenido).
    pub fn of_file(path: &Path) -> std::io::Result<Self> {
        Ok(Self::of_bytes(&std::fs::read(path)?))
    }

    /// Forma corta para directorios de un store CAS: `<hash-sin-prefijo>-<name>`.
    pub fn store_dir_name(&self, name: &str) -> String {
        let bare = self.0.strip_prefix("b3:").unwrap_or(&self.0);
        format!("{bare}-{name}")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Hash de **contenido** de un árbol de archivos: BLAKE3 determinista sobre
    /// los bytes reales (rutas relativas ordenadas + tipo + bit de ejecución +
    /// contenido / target de symlink). Compatible con `hammer::of_tree`.
    pub fn of_tree(root: &Path) -> std::io::Result<ArtifactHash> {
        let mut rels: Vec<PathBuf> = Vec::new();
        collect_rel(root, Path::new(""), &mut rels)?;
        rels.sort();

        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hammer-tree-v1");
        for rel in &rels {
            let abs = root.join(rel);
            let meta = std::fs::symlink_metadata(&abs)?;
            let relb = rel.as_os_str().as_bytes();
            hasher.update(&(relb.len() as u64).to_le_bytes());
            hasher.update(relb);

            let ft = meta.file_type();
            if ft.is_symlink() {
                let tgt = std::fs::read_link(&abs)?;
                let t = tgt.as_os_str().as_bytes();
                hasher.update(b"L");
                hasher.update(&(t.len() as u64).to_le_bytes());
                hasher.update(t);
            } else if ft.is_dir() {
                hasher.update(b"D");
            } else {
                let exec = meta.permissions().mode() & 0o111 != 0;
                hasher.update(if exec { b"Fx" } else { b"F0" });
                let bytes = std::fs::read(&abs)?;
                hasher.update(&(bytes.len() as u64).to_le_bytes());
                hasher.update(&bytes);
            }
        }
        Ok(ArtifactHash(format!("b3:{}", hasher.finalize().to_hex())))
    }
}

fn collect_rel(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(root.join(rel))? {
        let entry = entry?;
        let child = rel.join(entry.file_name());
        let is_dir = entry.file_type()?.is_dir();
        out.push(child.clone());
        if is_dir {
            collect_rel(root, &child, out)?;
        }
    }
    Ok(())
}

impl std::fmt::Display for ArtifactHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministico_y_sensible_al_orden() {
        let a = ArtifactHash::of_inputs(&[b"grep", b"abc123", b"--static"]);
        let b = ArtifactHash::of_inputs(&[b"grep", b"abc123", b"--static"]);
        assert_eq!(a, b);
        let c = ArtifactHash::of_inputs(&[b"abc123", b"grep", b"--static"]);
        assert_ne!(a, c);
    }

    #[test]
    fn of_bytes_lleva_prefijo() {
        let h = ArtifactHash::of_bytes(b"hola");
        assert!(h.as_str().starts_with("b3:"));
    }
}
