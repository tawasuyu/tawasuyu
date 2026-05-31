//! Adapter [`Source`] sobre las **Mónadas semánticas** de nouser
//! (`chasqui-core`).
//!
//! A diferencia de POSIX (jerarquía de directorios) y wawa (DAG de
//! contenido), nouser agrupa archivos POSIX en *clusters* semánticos —
//! Mónadas— por directorio + afinidad. El árbol que expone es de DOS niveles:
//! la raíz lista las Mónadas (contenedores sintéticos), y cada Mónada lista
//! sus archivos miembro (hojas POSIX leíbles). Es la prueba de que el trait
//! [`Source`] generaliza más allá de árboles "físicos": un nodo contenedor no
//! tiene por qué existir como entidad en disco.
//!
//! Puro local y determinista: el pipeline scan→cluster usa
//! pseudo-embeddings deterministas cuando no hay daemon de embeddings, así
//! que no requiere red ni servicio. Detrás de la feature `nouser` para no
//! arrastrar el peso de `chasqui-core` (sled, walkdir) a quien sólo quiere
//! POSIX/wawa.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use chasqui_core::cluster::by_directory;
use chasqui_core::scanner::{scan_directory, ScanConfig};

use crate::{Node, NodeId, Source};

/// Id de la raíz sintética que lista las Mónadas.
const RAIZ: &str = "@monadas";
/// Prefijo de id de una Mónada (contenedor semántico).
const PREF_MONADA: &str = "m:";
/// Prefijo de id de un archivo miembro (hoja POSIX).
const PREF_ARCHIVO: &str = "f:";

struct MonadaVista {
    id: String,
    label: String,
    miembros: Vec<String>,
}

struct ArchivoVista {
    nombre: String,
    ruta: PathBuf,
}

/// Fuente que navega los archivos de un directorio agrupados en Mónadas.
pub struct NouserSource {
    etiqueta: String,
    monadas: Vec<MonadaVista>,
    archivos: HashMap<String, ArchivoVista>,
}

impl NouserSource {
    /// Escanea `dir` y clusteriza sus archivos en Mónadas. `min_archivos` es
    /// el tamaño mínimo de un cluster para promoverlo a Mónada (usar 1 para
    /// que hasta un directorio de un solo archivo aparezca).
    pub fn escanear(dir: impl AsRef<Path>, min_archivos: usize) -> io::Result<Self> {
        let dir = dir.as_ref();
        let files = scan_directory(dir, &ScanConfig::default()).map_err(io::Error::other)?;

        let archivos: HashMap<String, ArchivoVista> = files
            .iter()
            .map(|fe| {
                let nombre = fe
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| fe.path.to_string_lossy().into_owned());
                (fe.id.to_string(), ArchivoVista { nombre, ruta: fe.path.clone() })
            })
            .collect();

        let monadas = by_directory(&files, min_archivos)
            .into_iter()
            .map(|m| MonadaVista {
                id: m.id.to_string(),
                label: if m.label.is_empty() {
                    m.path_hint.clone().unwrap_or_else(|| m.id.to_string())
                } else {
                    m.label.clone()
                },
                miembros: m.members.iter().map(|f| f.to_string()).collect(),
            })
            .collect();

        let etiqueta = dir.to_string_lossy().into_owned();
        Ok(Self { etiqueta, monadas, archivos })
    }

    fn nodo_archivo(&self, fid: &str) -> Option<Node> {
        self.archivos
            .get(fid)
            .map(|a| Node::new(format!("{PREF_ARCHIVO}{fid}"), a.nombre.clone(), false))
    }
}

impl Source for NouserSource {
    fn label(&self) -> String {
        self.etiqueta.clone()
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, self.etiqueta.clone(), true)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        if id == RAIZ {
            return Ok(self
                .monadas
                .iter()
                .map(|m| {
                    Node::new(
                        format!("{PREF_MONADA}{}", m.id),
                        format!("{} ({})", m.label, m.miembros.len()),
                        true,
                    )
                })
                .collect());
        }
        if let Some(mid) = id.strip_prefix(PREF_MONADA) {
            let monada = self.monadas.iter().find(|m| m.id == mid).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("Mónada inexistente: {id}"))
            })?;
            return Ok(monada
                .miembros
                .iter()
                .filter_map(|fid| self.nodo_archivo(fid))
                .collect());
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("una hoja no tiene hijos: {id}"),
        ))
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        let fid = id.strip_prefix(PREF_ARCHIVO).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sólo los archivos miembro son leíbles: {id}"),
            )
        })?;
        let archivo = self.archivos.get(fid).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("archivo inexistente: {id}"))
        })?;
        std::fs::read(&archivo.ruta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn arbol() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("proyecto_a")).unwrap();
        fs::create_dir(dir.path().join("proyecto_b")).unwrap();
        let mut f = fs::File::create(dir.path().join("proyecto_a/uno.txt")).unwrap();
        f.write_all(b"contenido uno").unwrap();
        fs::File::create(dir.path().join("proyecto_a/dos.txt")).unwrap();
        fs::File::create(dir.path().join("proyecto_b/tres.rs")).unwrap();
        dir
    }

    #[test]
    fn navega_raiz_monadas_y_archivos() {
        let dir = arbol();
        let src = NouserSource::escanear(dir.path(), 1).unwrap();

        let root = src.root();
        assert!(root.is_container);
        let monadas = src.children(&root.id).unwrap();
        assert!(
            monadas.len() >= 2,
            "esperaba al menos 2 Mónadas (proyecto_a, proyecto_b), hubo {}",
            monadas.len()
        );
        assert!(monadas.iter().all(|m| m.is_container));

        // Encontrá la Mónada que contiene uno.txt y leé su contenido.
        let mut encontrado = false;
        for m in &monadas {
            let archivos = src.children(&m.id).unwrap();
            if let Some(uno) = archivos.iter().find(|a| a.name == "uno.txt") {
                assert!(!uno.is_container);
                assert_eq!(src.read(&uno.id).unwrap(), b"contenido uno");
                encontrado = true;
            }
        }
        assert!(encontrado, "ninguna Mónada contenía uno.txt");
    }

    #[test]
    fn read_de_monada_o_raiz_es_error() {
        let dir = arbol();
        let src = NouserSource::escanear(dir.path(), 1).unwrap();
        assert!(src.read(&RAIZ.to_string()).is_err());
        let monadas = src.children(&RAIZ.to_string()).unwrap();
        assert!(src.read(&monadas[0].id).is_err());
    }

    #[test]
    fn escanear_dir_inexistente_es_error() {
        assert!(NouserSource::escanear("/no/existe/jamas", 1).is_err());
    }
}
