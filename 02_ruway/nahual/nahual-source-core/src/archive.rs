//! Adapter [`Source`] que monta un archivo contenedor (`.zip`, `.tar`,
//! `.tar.gz`) **como una carpeta navegable**.
//!
//! Es el "archivos como carpetas" de Directory Opus (UNIFICACION.md §2, F4.6):
//! doble-clic en un `.zip` POSIX lo empuja como una fuente montada y se navega
//! su jerarquía interna con la misma UI que el filesystem. Read-only —un
//! archivo comprimido es inmutable desde el punto de vista del front; editar
//! adentro no tiene semántica POSIX y por eso `writable()` queda en `None`,
//! igual que wawa/minga.
//!
//! El índice se construye **una vez al abrir**: los formatos traen una lista
//! *plana* de rutas (`a/b/c.txt`), de la que se sintetiza el árbol de
//! directorios (la mayoría de los `.tar` no traen entradas de directorio
//! explícitas). El [`NodeId`] de un nodo es su ruta dentro del archivo, sin
//! barra final; la raíz usa el centinela [`RAIZ`]. `read` reabre el archivo y
//! extrae la entrada pedida (zip por nombre, tar por recorrido en streaming).

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

use crate::{Node, NodeId, NodeKind, Source};

/// Id sintético de la raíz del archivo montado — su rol es contener las
/// entradas de nivel superior. No corresponde a ninguna entrada real.
const RAIZ: &str = "@archivo";

/// Tope de entradas a indexar. Un archivo con más se trunca para no
/// atragantar la UI; el resto queda sin listar (se reporta en el `label`).
const MAX_ENTRADAS: usize = 20_000;

/// Qué formato de contenedor se montó — decide cómo extraer una entrada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Formato {
    Zip,
    Tar,
    TarGz,
}

/// Metadata de una entrada indexada (sin su contenido).
#[derive(Clone, Debug)]
struct Entrada {
    /// Tamaño sin comprimir, en bytes.
    size: u64,
    is_dir: bool,
}

/// Fuente que navega el interior de un archivo contenedor cargado en índice.
pub struct ArchiveSource {
    path: PathBuf,
    formato: Formato,
    etiqueta: String,
    /// Toda entrada (archivos + directorios sintetizados) por su ruta
    /// normalizada (sin barra final, sin `./`).
    entradas: BTreeMap<String, Entrada>,
    /// Hijos directos de cada contenedor: `dir-path → [rutas hijas]`. La raíz
    /// usa la clave vacía `""`.
    hijos: BTreeMap<String, Vec<String>>,
    truncado: bool,
}

impl ArchiveSource {
    /// ¿La extensión de `path` sugiere un archivo montable? Barato (no toca
    /// disco) — el shell lo usa para decidir si intentar montar antes que
    /// previsualizar.
    pub fn es_archivo(path: &Path) -> bool {
        let n = path.file_name().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        n.ends_with(".zip")
            || n.ends_with(".tar")
            || n.ends_with(".tar.gz")
            || n.ends_with(".tgz")
    }

    /// Abre el archivo en `path`, olfatea su formato por contenido e indexa
    /// sus entradas. Error de I/O o si el formato no se reconoce.
    pub fn abrir(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let formato = olfatear(&path)?;
        let etiqueta = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let (planas, truncado) = match formato {
            Formato::Zip => listar_zip(&path)?,
            Formato::Tar => listar_tar(std::fs::File::open(&path)?)?,
            Formato::TarGz => listar_tar(GzDecoder::new(std::fs::File::open(&path)?))?,
        };

        let (entradas, hijos) = indexar(planas);
        Ok(Self { path, formato, etiqueta, entradas, hijos, truncado })
    }

    fn nodo_de(&self, ruta: &str) -> Node {
        let nombre = ruta.rsplit('/').next().unwrap_or(ruta).to_string();
        let e = self.entradas.get(ruta);
        let is_dir = e.map(|e| e.is_dir).unwrap_or(true);
        let mut nodo = Node::new(ruta.to_string(), nombre, is_dir);
        if is_dir {
            nodo = nodo.with_kind(NodeKind::Dir);
        } else if let Some(e) = e {
            nodo = nodo.with_size(e.size);
        }
        nodo
    }
}

impl Source for ArchiveSource {
    fn label(&self) -> String {
        if self.truncado {
            format!("{} (truncado a {} entradas)", self.etiqueta, MAX_ENTRADAS)
        } else {
            self.etiqueta.clone()
        }
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, self.etiqueta.clone(), true).with_kind(NodeKind::Synthetic)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        // La raíz sintética mapea a la clave vacía del índice de hijos.
        let clave = if id == RAIZ { "" } else { id.as_str() };
        if id != RAIZ && !self.entradas.contains_key(clave) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("entrada inexistente en el archivo: {id}"),
            ));
        }
        let mut nodos: Vec<Node> = self
            .hijos
            .get(clave)
            .map(|v| v.iter().map(|r| self.nodo_de(r)).collect())
            .unwrap_or_default();
        // Contenedores primero, luego por nombre — orden presentable estable.
        nodos.sort_by(|a, b| {
            b.is_container.cmp(&a.is_container).then_with(|| a.name.cmp(&b.name))
        });
        Ok(nodos)
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        if id == RAIZ {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "la raíz del archivo no tiene contenido leíble",
            ));
        }
        match self.entradas.get(id) {
            Some(e) if e.is_dir => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("la entrada es un directorio: {id}"),
            )),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("entrada inexistente en el archivo: {id}"),
            )),
            Some(_) => match self.formato {
                Formato::Zip => extraer_zip(&self.path, id),
                Formato::Tar => extraer_tar(std::fs::File::open(&self.path)?, id),
                Formato::TarGz => {
                    extraer_tar(GzDecoder::new(std::fs::File::open(&self.path)?), id)
                }
            },
        }
    }
}

/// Detecta el formato por el magic del header (no por la extensión): `PK` →
/// zip, `ustar` en off 257 → tar, `1f 8b` → gzip (que asumimos envuelve tar).
fn olfatear(path: &Path) -> io::Result<Formato> {
    let mut f = std::fs::File::open(path)?;
    let mut head = [0u8; 512];
    let n = f.read(&mut head)?;
    let head = &head[..n];
    if head.starts_with(b"PK\x03\x04") || head.starts_with(b"PK\x05\x06") {
        return Ok(Formato::Zip);
    }
    if head.len() >= 262 && &head[257..262] == b"ustar" {
        return Ok(Formato::Tar);
    }
    if head.starts_with(&[0x1F, 0x8B]) {
        return Ok(Formato::TarGz);
    }
    Err(io::Error::new(io::ErrorKind::InvalidData, "formato de archivo no reconocido"))
}

/// Normaliza una ruta interna: quita `./` inicial y la barra final, colapsa
/// vacío. Devuelve `None` para rutas que se reducen a la raíz.
fn normalizar(raw: &str) -> Option<String> {
    let r = raw.trim_start_matches("./").trim_end_matches('/');
    if r.is_empty() {
        None
    } else {
        Some(r.to_string())
    }
}

fn listar_zip(path: &Path) -> io::Result<(Vec<(String, Entrada)>, bool)> {
    let f = std::fs::File::open(path)?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("no es un ZIP válido: {e}"))
    })?;
    let total = ar.len();
    let mut out = Vec::with_capacity(total.min(MAX_ENTRADAS));
    for i in 0..total.min(MAX_ENTRADAS) {
        let e = ar.by_index_raw(i).map_err(io::Error::other)?;
        let raw = e.name().to_string();
        let is_dir = e.is_dir();
        if let Some(ruta) = normalizar(&raw) {
            out.push((ruta, Entrada { size: e.size(), is_dir }));
        }
    }
    Ok((out, total > MAX_ENTRADAS))
}

fn listar_tar<R: Read>(reader: R) -> io::Result<(Vec<(String, Entrada)>, bool)> {
    let mut ar = tar::Archive::new(reader);
    let mut out = Vec::new();
    let mut truncado = false;
    for item in ar.entries()? {
        let e = item?;
        if out.len() >= MAX_ENTRADAS {
            truncado = true;
            break;
        }
        let is_dir = e.header().entry_type().is_dir();
        let size = e.header().size().unwrap_or(0);
        let raw = e.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some(ruta) = normalizar(&raw) {
            out.push((ruta, Entrada { size, is_dir }));
        }
    }
    Ok((out, truncado))
}

/// Construye el árbol a partir de la lista plana: sintetiza los directorios
/// intermedios que el formato no declara y arma el mapa `dir → hijos`.
fn indexar(
    planas: Vec<(String, Entrada)>,
) -> (BTreeMap<String, Entrada>, BTreeMap<String, Vec<String>>) {
    let mut entradas: BTreeMap<String, Entrada> = BTreeMap::new();
    // Toda ruta que es un directorio (declarada o ancestro sintetizado).
    let mut dirs: BTreeSet<String> = BTreeSet::new();

    for (ruta, e) in &planas {
        if e.is_dir {
            dirs.insert(ruta.clone());
        }
        // Cada componente padre es un directorio.
        let mut acc = String::new();
        for comp in ruta.split('/') {
            if !acc.is_empty() {
                // El padre acumulado hasta acá es un directorio.
                dirs.insert(acc.clone());
            }
            if acc.is_empty() {
                acc.push_str(comp);
            } else {
                acc.push('/');
                acc.push_str(comp);
            }
        }
    }

    for d in &dirs {
        entradas.insert(d.clone(), Entrada { size: 0, is_dir: true });
    }
    for (ruta, e) in planas {
        if !e.is_dir {
            // Un archivo nunca debe quedar pisado por un dir sintetizado.
            entradas.insert(ruta, e);
        }
    }

    // Mapa de hijos: el padre de `a/b/c` es `a/b`; el de `a` es `""` (raíz).
    let mut hijos: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ruta in entradas.keys() {
        let padre = match ruta.rsplit_once('/') {
            Some((p, _)) => p.to_string(),
            None => String::new(),
        };
        hijos.entry(padre).or_default().push(ruta.clone());
    }
    (entradas, hijos)
}

fn extraer_zip(path: &Path, nombre: &str) -> io::Result<Vec<u8>> {
    let f = std::fs::File::open(path)?;
    let mut ar = zip::ZipArchive::new(f).map_err(io::Error::other)?;
    // El nombre guardado puede traer `./`; localizamos por nombre normalizado
    // (el mismo criterio con que se indexó) y extraemos por índice.
    let idx = (0..ar.len()).find(|&i| {
        ar.by_index_raw(i)
            .ok()
            .and_then(|e| normalizar(e.name()))
            .as_deref()
            == Some(nombre)
    });
    let idx = idx
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no en el zip: {nombre}")))?;
    let mut e = ar.by_index(idx).map_err(io::Error::other)?;
    let mut buf = Vec::with_capacity(e.size() as usize);
    e.read_to_end(&mut buf)?;
    Ok(buf)
}

fn extraer_tar<R: Read>(reader: R, nombre: &str) -> io::Result<Vec<u8>> {
    let mut ar = tar::Archive::new(reader);
    for item in ar.entries()? {
        let mut e = item?;
        let ruta = e.path().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if normalizar(&ruta).as_deref() == Some(nombre) {
            let mut buf = Vec::new();
            e.read_to_end(&mut buf)?;
            return Ok(buf);
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, format!("no en el tar: {nombre}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Forja un `.zip` con `dir/a.txt`, `dir/sub/b.txt` y `raiz.txt`. La
    /// entrada de directorio intermedia `dir/sub/` NO se declara — fuerza la
    /// síntesis del árbol.
    fn zip_sintetico() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("t.zip");
        let f = std::fs::File::create(&ruta).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts: zip::write::FileOptions<()> = zip::write::FileOptions::default();
        zw.start_file("raiz.txt", opts).unwrap();
        zw.write_all(b"soy raiz").unwrap();
        zw.add_directory("dir/", opts).unwrap();
        zw.start_file("dir/a.txt", opts).unwrap();
        zw.write_all(b"contenido a").unwrap();
        zw.start_file("dir/sub/b.txt", opts).unwrap();
        zw.write_all(b"profundo b").unwrap();
        zw.finish().unwrap();
        (dir, ruta)
    }

    fn tar_sintetico(gz: bool) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join(if gz { "t.tar.gz" } else { "t.tar" });
        let f = std::fs::File::create(&ruta).unwrap();
        let escribir = |w: &mut dyn Write| {
            let mut tw = tar::Builder::new(w);
            let h = |tw: &mut tar::Builder<&mut dyn Write>, name: &str, data: &[u8]| {
                let mut head = tar::Header::new_gnu();
                head.set_size(data.len() as u64);
                head.set_mode(0o644);
                head.set_cksum();
                tw.append_data(&mut head, name, data).unwrap();
            };
            h(&mut tw, "raiz.txt", b"soy raiz");
            h(&mut tw, "dir/a.txt", b"contenido a");
            h(&mut tw, "dir/sub/b.txt", b"profundo b");
            tw.finish().unwrap();
        };
        if gz {
            let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
            escribir(&mut enc);
            enc.finish().unwrap();
        } else {
            let mut f = f;
            escribir(&mut f);
        }
        (dir, ruta)
    }

    fn afirmar_arbol(src: &ArchiveSource) {
        let root = src.root();
        assert_eq!(root.id, RAIZ);
        assert!(root.is_container);

        let top = src.children(&root.id).unwrap();
        let nombres: Vec<&str> = top.iter().map(|n| n.name.as_str()).collect();
        // `dir` (contenedor) primero, luego `raiz.txt`.
        assert_eq!(nombres, vec!["dir", "raiz.txt"]);
        assert!(top[0].is_container);
        assert!(!top[1].is_container);

        // Raíz leíble.
        assert_eq!(src.read(&"raiz.txt".to_string()).unwrap(), b"soy raiz");

        // Descender a `dir`: `sub` (sintetizado) + `a.txt`.
        let kids = src.children(&"dir".to_string()).unwrap();
        let kn: Vec<&str> = kids.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(kn, vec!["sub", "a.txt"]);
        let sub = kids.iter().find(|n| n.name == "sub").unwrap();
        assert!(sub.is_container, "el directorio intermedio se sintetiza");
        assert_eq!(src.read(&"dir/a.txt".to_string()).unwrap(), b"contenido a");

        // Profundo.
        let prof = src.children(&"dir/sub".to_string()).unwrap();
        assert_eq!(prof.len(), 1);
        assert_eq!(src.read(&"dir/sub/b.txt".to_string()).unwrap(), b"profundo b");
    }

    #[test]
    fn zip_se_navega_como_carpeta() {
        let (_d, ruta) = zip_sintetico();
        assert!(ArchiveSource::es_archivo(&ruta));
        let src = ArchiveSource::abrir(&ruta).unwrap();
        assert_eq!(src.formato, Formato::Zip);
        afirmar_arbol(&src);
    }

    #[test]
    fn tar_se_navega_como_carpeta() {
        let (_d, ruta) = tar_sintetico(false);
        let src = ArchiveSource::abrir(&ruta).unwrap();
        assert_eq!(src.formato, Formato::Tar);
        afirmar_arbol(&src);
    }

    #[test]
    fn targz_se_navega_como_carpeta() {
        let (_d, ruta) = tar_sintetico(true);
        let src = ArchiveSource::abrir(&ruta).unwrap();
        assert_eq!(src.formato, Formato::TarGz);
        afirmar_arbol(&src);
    }

    #[test]
    fn read_de_directorio_y_basura_es_error() {
        let (_d, ruta) = zip_sintetico();
        let src = ArchiveSource::abrir(&ruta).unwrap();
        assert!(src.read(&"dir".to_string()).is_err()); // es dir
        assert!(src.read(&"no/existe".to_string()).is_err()); // inexistente
        assert!(src.read(&RAIZ.to_string()).is_err()); // raíz sintética
        assert!(src.children(&"no/existe".to_string()).is_err());
    }

    #[test]
    fn read_only_sin_writable() {
        let (_d, ruta) = zip_sintetico();
        let src = ArchiveSource::abrir(&ruta).unwrap();
        assert!(src.writable().is_none(), "un archivo comprimido es inmutable");
    }

    #[test]
    fn no_archivo_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let ruta = dir.path().join("plano.txt");
        std::fs::write(&ruta, b"no soy un archivo contenedor").unwrap();
        assert!(ArchiveSource::abrir(&ruta).is_err());
    }
}
