//! Recorrido de directorios. Sólo metadatos — no lee contenido.
//!
//! Usa `walkdir` (sequential). Para árboles muy grandes considerar
//! migrar a `jwalk` (paralelo); por ahora la simplicidad gana.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chasqui_card::{FileEntry, FileId};
use thiserror::Error;
use ulid::Ulid;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("ruta no existe: {0}")]
    NotFound(PathBuf),
    #[error("no se pudo leer: {0}")]
    Walk(String),
}

/// Configuración del scan.
#[derive(Debug, Clone)]
pub struct ScanConfig {
    /// Profundidad máxima (None = ilimitada).
    pub max_depth: Option<usize>,
    /// Sigue symlinks (default: false, evita ciclos).
    pub follow_links: bool,
    /// Ignora archivos ocultos (.dotfiles).
    pub skip_hidden: bool,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            max_depth: None,
            follow_links: false,
            skip_hidden: true,
        }
    }
}

/// Recorre `root` y devuelve un `FileEntry` por cada archivo regular.
/// Errores de permisos en sub-paths se ignoran silenciosamente.
pub fn scan_directory(root: &Path, config: &ScanConfig) -> Result<Vec<FileEntry>, ScanError> {
    if !root.exists() {
        return Err(ScanError::NotFound(root.to_path_buf()));
    }

    let mut walker = WalkDir::new(root).follow_links(config.follow_links);
    if let Some(d) = config.max_depth {
        walker = walker.max_depth(d);
    }

    let mut entries = Vec::new();
    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        if config.skip_hidden && is_hidden(entry.path()) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let extension = entry
            .path()
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase());

        entries.push(FileEntry {
            id: FileId::from(Ulid::new()),
            path: entry.path().to_path_buf(),
            content_hash: None,
            size: metadata.len(),
            mtime_ms,
            extension,
        });
    }
    Ok(entries)
}

/// `true` si alguno de los componentes del path empieza con `.`.
/// Excluye el primer componente (root) para no descartar el directorio raíz
/// si el usuario apuntó a un dotfile-dir explícito.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn scans_basic_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("a.rs"), "fn main(){}");
        write(&root.join("b.rs"), "fn b(){}");
        write(&root.join("data/x.json"), "{}");
        write(&root.join("data/y.json"), "{}");

        let files = scan_directory(root, &ScanConfig::default()).unwrap();
        assert_eq!(files.len(), 4);
        let exts: std::collections::BTreeSet<_> = files
            .iter()
            .filter_map(|f| f.extension.clone())
            .collect();
        assert!(exts.contains("rs"));
        assert!(exts.contains("json"));
    }

    #[test]
    fn skips_hidden_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("visible.txt"), "x");
        write(&root.join(".hidden"), "x");

        let files = scan_directory(root, &ScanConfig::default()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].path.ends_with("visible.txt"));
    }

    #[test]
    fn missing_root_errors() {
        let p = std::path::Path::new("/nonexistent-12345-abc");
        assert!(matches!(
            scan_directory(p, &ScanConfig::default()),
            Err(ScanError::NotFound(_))
        ));
    }

    #[test]
    fn max_depth_limits() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(&root.join("top.txt"), "x");
        write(&root.join("a/b/deep.txt"), "x");

        let cfg = ScanConfig {
            max_depth: Some(1),
            ..Default::default()
        };
        let files = scan_directory(root, &cfg).unwrap();
        // max_depth=1 incluye archivos en root pero no anidados profundos.
        let names: Vec<_> = files
            .iter()
            .filter_map(|f| f.path.file_name().and_then(|s| s.to_str()))
            .map(String::from)
            .collect();
        assert!(names.contains(&"top.txt".to_string()));
        assert!(!names.contains(&"deep.txt".to_string()));
    }
}
