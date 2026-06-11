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
use std::time::UNIX_EPOCH;

use crate::{Node, NodeId, NodeKind, Source, SourceMut};

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

/// La última modificación de `m` en epoch-ms, si se puede leer.
fn mtime_ms(m: &fs::Metadata) -> Option<u64> {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

/// Copia recursiva de `src` a `dst` (archivo o directorio). Usada por
/// [`SourceMut::copy_into`] cuando `fs::copy` no alcanza (directorios).
fn copiar_recursivo(src: &Path, dst: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        fs::create_dir(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let nombre = entry.file_name();
            copiar_recursivo(&entry.path(), &dst.join(nombre))?;
        }
        Ok(())
    } else {
        fs::copy(src, dst).map(|_| ())
    }
}

/// Resuelve un nombre de destino sin colisión dentro de `parent`: si `name`
/// ya existe, prueba `name (copia)`, `name (copia 2)`, … Devuelve la ruta
/// libre. Evita que copiar/mover pise un archivo existente en silencio.
fn ruta_libre(parent: &Path, name: &str) -> PathBuf {
    let candidato = parent.join(name);
    if !candidato.exists() {
        return candidato;
    }
    // Separa stem/extensión para insertar el sufijo antes del punto.
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 1.. {
        let sufijo = if n == 1 { " (copia)".to_string() } else { format!(" (copia {n})") };
        let p = parent.join(format!("{stem}{sufijo}{ext}"));
        if !p.exists() {
            return p;
        }
    }
    unreachable!("el rango 1.. siempre encuentra un nombre libre")
}

impl Source for PosixSource {
    fn label(&self) -> String {
        self.root.to_string_lossy().into_owned()
    }

    fn root(&self) -> Node {
        Node::new(self.root.to_string_lossy().into_owned(), nombre_de(&self.root), true)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        let mut nodos: Vec<Node> = Vec::new();
        for entry in fs::read_dir(Path::new(id))? {
            let entry = entry?;
            let path = entry.path();
            let ft = entry.file_type().ok();
            let es_symlink = ft.map(|t| t.is_symlink()).unwrap_or(false);
            // `is_container` sigue (si es symlink) al destino: un enlace a dir
            // se navega. El `kind` en cambio conserva que ES un symlink.
            let is_dir = match ft {
                Some(t) if t.is_symlink() => {
                    fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
                }
                Some(t) => t.is_dir(),
                None => false,
            };
            let kind = if es_symlink {
                NodeKind::Symlink
            } else if is_dir {
                NodeKind::Dir
            } else {
                NodeKind::File
            };
            // Metadata sin seguir el enlace (tamaño/mtime de la entrada misma).
            let meta = entry.metadata().ok();
            let mut nodo = Node::new(path.to_string_lossy().into_owned(), nombre_de(&path), is_dir)
                .with_kind(kind);
            if let Some(m) = &meta {
                // El tamaño sólo tiene sentido para archivos regulares.
                if !is_dir {
                    nodo = nodo.with_size(m.len());
                }
                if let Some(ms) = mtime_ms(m) {
                    nodo = nodo.with_mtime(ms);
                }
            }
            nodos.push(nodo);
        }
        // Directorios primero, luego alfabético case-insensitive — mismo
        // criterio que el explorador POSIX histórico.
        nodos.sort_by(|a, b| {
            b.is_container
                .cmp(&a.is_container)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(nodos)
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        fs::read(Path::new(id))
    }

    fn writable(&self) -> Option<&dyn SourceMut> {
        Some(self)
    }
}

impl SourceMut for PosixSource {
    fn create_dir(&self, parent: &NodeId, name: &str) -> io::Result<NodeId> {
        let dest = Path::new(parent).join(name);
        fs::create_dir(&dest)?;
        Ok(dest.to_string_lossy().into_owned())
    }

    fn create_file(&self, parent: &NodeId, name: &str) -> io::Result<NodeId> {
        let dest = Path::new(parent).join(name);
        // `create_new` falla si ya existe — no pisamos en silencio.
        fs::OpenOptions::new().write(true).create_new(true).open(&dest)?;
        Ok(dest.to_string_lossy().into_owned())
    }

    fn delete(&self, id: &NodeId) -> io::Result<()> {
        let path = Path::new(id);
        let meta = fs::symlink_metadata(path)?;
        if meta.is_dir() {
            fs::remove_dir_all(path)
        } else {
            fs::remove_file(path)
        }
    }

    fn rename(&self, id: &NodeId, new_name: &str) -> io::Result<NodeId> {
        let path = Path::new(id);
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("sin directorio padre: {id}"))
        })?;
        let dest = parent.join(new_name);
        if dest.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("ya existe: {}", dest.display()),
            ));
        }
        fs::rename(path, &dest)?;
        Ok(dest.to_string_lossy().into_owned())
    }

    fn move_into(&self, id: &NodeId, new_parent: &NodeId) -> io::Result<NodeId> {
        let src = Path::new(id);
        let name = src.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("sin nombre de archivo: {id}"))
        })?;
        let dest = ruta_libre(Path::new(new_parent), &name.to_string_lossy());
        // `fs::rename` falla entre filesystems distintos; ahí caemos a
        // copiar+borrar para que mover funcione igual cruzando montajes.
        match fs::rename(src, &dest) {
            Ok(()) => Ok(dest.to_string_lossy().into_owned()),
            Err(_) => {
                copiar_recursivo(src, &dest)?;
                let meta = fs::symlink_metadata(src)?;
                if meta.is_dir() {
                    fs::remove_dir_all(src)?;
                } else {
                    fs::remove_file(src)?;
                }
                Ok(dest.to_string_lossy().into_owned())
            }
        }
    }

    fn copy_into(&self, id: &NodeId, new_parent: &NodeId) -> io::Result<NodeId> {
        let src = Path::new(id);
        let name = src.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("sin nombre de archivo: {id}"))
        })?;
        let dest = ruta_libre(Path::new(new_parent), &name.to_string_lossy());
        copiar_recursivo(src, &dest)?;
        Ok(dest.to_string_lossy().into_owned())
    }

    fn write(&self, id: &NodeId, bytes: &[u8]) -> io::Result<()> {
        fs::write(Path::new(id), bytes)
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

    #[test]
    fn children_traen_metadata_real() {
        let dir = arbol();
        let src = PosixSource::new(dir.path());
        let kids = src.children(&src.root().id).unwrap();
        let hola = kids.iter().find(|n| n.name == "hola.txt").unwrap();
        assert_eq!(hola.kind, NodeKind::File);
        assert_eq!(hola.size, Some(b"contenido posix".len() as u64));
        assert!(hola.mtime.is_some());
        let alpha = kids.iter().find(|n| n.name == "alpha_dir").unwrap();
        assert_eq!(alpha.kind, NodeKind::Dir);
        assert_eq!(alpha.size, None); // los dirs no llevan tamaño
    }

    #[test]
    fn posix_expone_cara_mutable() {
        let dir = arbol();
        let src = PosixSource::new(dir.path());
        assert!(Source::writable(&src).is_some());
    }

    #[test]
    fn crear_renombrar_y_borrar() {
        let dir = tempfile::tempdir().unwrap();
        let src = PosixSource::new(dir.path());
        let root = src.root().id;

        // Crear dir + archivo.
        let sub = src.create_dir(&root, "nuevo").unwrap();
        assert!(Path::new(&sub).is_dir());
        let archivo = src.create_file(&sub, "a.txt").unwrap();
        assert!(Path::new(&archivo).is_file());
        // create_file no pisa: segundo intento falla.
        assert!(src.create_file(&sub, "a.txt").is_err());

        // Escribir y leer de vuelta.
        src.write(&archivo, b"hola").unwrap();
        assert_eq!(src.read(&archivo).unwrap(), b"hola");

        // Renombrar.
        let renombrado = src.rename(&archivo, "b.txt").unwrap();
        assert!(!Path::new(&archivo).exists());
        assert!(renombrado.ends_with("b.txt"));
        assert_eq!(src.read(&renombrado).unwrap(), b"hola");

        // Borrar (dir recursivo).
        src.delete(&sub).unwrap();
        assert!(!Path::new(&sub).exists());
    }

    #[test]
    fn copiar_y_mover_entre_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let src = PosixSource::new(dir.path());
        let root = src.root().id;
        let a = src.create_dir(&root, "a").unwrap();
        let b = src.create_dir(&root, "b").unwrap();
        let f = src.create_file(&a, "doc.txt").unwrap();
        src.write(&f, b"x").unwrap();

        // Copiar a → b: el original sigue.
        let copia = src.copy_into(&f, &b).unwrap();
        assert!(Path::new(&f).exists());
        assert!(copia.ends_with("doc.txt"));
        assert_eq!(src.read(&copia).unwrap(), b"x");

        // Copiar de nuevo: no pisa, usa sufijo " (copia)".
        let copia2 = src.copy_into(&f, &b).unwrap();
        assert!(copia2.contains("(copia)"));

        // Mover a → b: el original desaparece.
        let movido = src.move_into(&f, &b).unwrap();
        assert!(!Path::new(&f).exists());
        assert_eq!(src.read(&movido).unwrap(), b"x");
    }

    #[test]
    fn copia_recursiva_de_directorio() {
        let dir = tempfile::tempdir().unwrap();
        let src = PosixSource::new(dir.path());
        let root = src.root().id;
        let a = src.create_dir(&root, "arbolito").unwrap();
        let sub = src.create_dir(&a, "rama").unwrap();
        let hoja = src.create_file(&sub, "hoja.txt").unwrap();
        src.write(&hoja, b"contenido").unwrap();
        let dest = src.create_dir(&root, "destino").unwrap();

        let copia = src.copy_into(&a, &dest).unwrap();
        // La copia preserva el subárbol entero.
        let copia_hoja = Path::new(&copia).join("rama").join("hoja.txt");
        assert!(copia_hoja.exists());
        assert_eq!(fs::read(&copia_hoja).unwrap(), b"contenido");
        // El original intacto.
        assert!(Path::new(&hoja).exists());
    }
}
