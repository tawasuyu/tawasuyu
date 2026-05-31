//! Adapter [`Source`] sobre el filesystem POSIX vivo.
//!
//! Es lo que `nahual-file-explorer-llimphi` hacía a mano (`std::fs::read_dir`
//! + `Entry{name,is_dir}`), ahora detrás del trait común. El [`NodeId`] es la
//! ruta absoluta como string; los hijos vienen ordenados directorios-primero
//! y luego alfabético case-insensitive — el mismo orden presentable que el
//! explorador histórico.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{Node, NodeId, Source};

/// Fuente que navega un subárbol del filesystem POSIX a partir de una raíz.
pub struct PosixSource {
    root: PathBuf,
}

impl PosixSource {
    /// Crea la fuente anclada en `root`. No valida que exista — un `root`
    /// inválido simplemente devuelve `children` con error al navegarse.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

fn nombre_de(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

impl Source for PosixSource {
    fn label(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn root(&self) -> Node {
        Node::new(self.root.to_string_lossy().into_owned(), nombre_de(&self.root), true)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        let mut entries: Vec<(bool, String, String)> = Vec::new();
        for entry in fs::read_dir(Path::new(id))? {
            let entry = entry?;
            let path = entry.path();
            // `file_type` evita un stat extra; cae a metadata si es symlink.
            let is_dir = match entry.file_type() {
                Ok(ft) if ft.is_symlink() => fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false),
                Ok(ft) => ft.is_dir(),
                Err(_) => false,
            };
            entries.push((is_dir, nombre_de(&path), path.to_string_lossy().into_owned()));
        }
        // Directorios primero, luego alfabético case-insensitive — mismo
        // criterio que el explorador POSIX histórico.
        entries.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });
        Ok(entries
            .into_iter()
            .map(|(is_dir, name, id)| Node::new(id, name, is_dir))
            .collect())
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        fs::read(Path::new(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn arbol() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("zeta_dir")).unwrap();
        fs::create_dir(dir.path().join("alpha_dir")).unwrap();
        let mut f = fs::File::create(dir.path().join("hola.txt")).unwrap();
        f.write_all(b"contenido posix").unwrap();
        dir
    }

    #[test]
    fn root_y_children_navegan_y_ordenan() {
        let dir = arbol();
        let src = PosixSource::new(dir.path());
        let root = src.root();
        assert!(root.is_container);

        let kids = src.children(&root.id).unwrap();
        // dos dirs primero (alpha, zeta) y luego el archivo.
        assert_eq!(kids.len(), 3);
        assert_eq!(kids[0].name, "alpha_dir");
        assert!(kids[0].is_container);
        assert_eq!(kids[1].name, "zeta_dir");
        assert!(kids[1].is_container);
        assert_eq!(kids[2].name, "hola.txt");
        assert!(!kids[2].is_container);
    }

    #[test]
    fn read_hoja_devuelve_bytes() {
        let dir = arbol();
        let src = PosixSource::new(dir.path());
        let kids = src.children(&src.root().id).unwrap();
        let hola = kids.iter().find(|n| n.name == "hola.txt").unwrap();
        assert_eq!(src.read(&hola.id).unwrap(), b"contenido posix");
    }

    #[test]
    fn children_de_ruta_inexistente_es_error() {
        let src = PosixSource::new("/no/existe/jamas");
        assert!(src.children(&"/no/existe/jamas".to_string()).is_err());
    }
}
