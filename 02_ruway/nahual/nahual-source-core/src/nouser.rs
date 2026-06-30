//! Adapter [`Source`] sobre el **grafo de Mónadas** de nouser
//! (`chasqui-core`).
//!
//! A diferencia de POSIX (jerarquía de directorios) y wawa (DAG de
//! contenido), nouser agrupa archivos POSIX en Mónadas semánticas. Desde
//! la Fase del grafo, una Mónada ya no es un cluster plano: es un nodo de
//! un **DAG** que puede contener archivos *y otras Mónadas*
//! (`submonads`), y cuya membresía puede derivarse de una regla
//! intensional (`query`) en vez de curarse a mano. Este adapter proyecta
//! ese grafo por el trait [`Source`]:
//!
//! - la raíz `@monadas` lista las Mónadas **de nivel superior** (las que
//!   ninguna otra contiene);
//! - los hijos de una Mónada son sus **sub-Mónadas** (contenedores
//!   sintéticos) seguidas de sus **archivos efectivos** (hojas POSIX
//!   leíbles), resueltos con [`chasqui_core::resolve`];
//! - como la identidad de un nodo es su id (no su ruta de navegación), la
//!   *misma* Mónada o archivo puede aparecer bajo varios padres — la
//!   multi-pertenencia que el modelo permite, proyectada sin duplicar.
//!
//! Puro local y determinista: el pipeline scan→cluster usa
//! pseudo-embeddings deterministas cuando no hay daemon de embeddings, así
//! que no requiere red ni servicio. Detrás de la feature `nouser` para no
//! arrastrar el peso de `chasqui-core` (sled, walkdir) a quien sólo quiere
//! POSIX/wawa.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use chasqui_core::ulid::Ulid;
use chasqui_core::cluster::by_directory;
use chasqui_core::{FileEntry, MonadId, MonadManifest};
use chasqui_core::db::MonadDb;
use chasqui_core::resolve;
use chasqui_core::scanner::{scan_directory, ScanConfig};

use crate::{Node, NodeId, NodeKind, Source};

/// Id de la raíz sintética que lista las Mónadas de nivel superior.
const RAIZ: &str = "@monadas";
/// Prefijo de id de una Mónada (contenedor sintético).
const PREF_MONADA: &str = "m:";
/// Prefijo de id de un archivo miembro (hoja POSIX).
const PREF_ARCHIVO: &str = "f:";

/// Fuente que navega el grafo de Mónadas de un directorio.
pub struct NouserSource {
    etiqueta: String,
    db: MonadDb,
}

impl NouserSource {
    /// Escanea `dir` y clusteriza sus archivos en Mónadas. `min_archivos`
    /// es el tamaño mínimo de un cluster para promoverlo a Mónada (usar 1
    /// para que hasta un directorio de un solo archivo aparezca).
    ///
    /// El clustering inicial (`by_directory`) produce un bosque plano de
    /// un nivel; el adapter ya navega el grafo completo, así que cualquier
    /// sub-Mónada o Mónada intensional que se agregue después (edición,
    /// re-clustering jerárquico) aparece sin tocar esta capa.
    pub fn escanear(dir: impl AsRef<Path>, min_archivos: usize) -> io::Result<Self> {
        let dir = dir.as_ref();
        let files = scan_directory(dir, &ScanConfig::default()).map_err(io::Error::other)?;
        let monadas = by_directory(&files, min_archivos);

        let mut db = MonadDb::new();
        db.ingest_files(files);
        db.replace_monads(monadas);

        Ok(Self { etiqueta: dir.to_string_lossy().into_owned(), db })
    }

    /// Construye la fuente sobre un grafo ya armado. Es la vía para montar
    /// un grafo con sub-Mónadas / Mónadas intensionales (construido por la
    /// capa de edición o por tests) sin pasar por `escanear`.
    pub fn from_db(label: impl Into<String>, db: MonadDb) -> Self {
        Self { etiqueta: label.into(), db }
    }

    /// Acceso de sólo lectura al grafo subyacente.
    pub fn db(&self) -> &MonadDb {
        &self.db
    }

    /// Mónadas de nivel superior: las que ninguna otra contiene como
    /// sub-Mónada (las raíces del bosque de contención).
    fn top_level(&self) -> Vec<&MonadManifest> {
        let mut contenidas: BTreeSet<MonadId> = BTreeSet::new();
        for m in self.db.monads() {
            contenidas.extend(m.submonads.iter().copied());
        }
        self.db
            .monads()
            .filter(|m| !contenidas.contains(&m.id))
            .collect()
    }

    /// Nodo de una Mónada (contenedor sintético). La etiqueta muestra el
    /// conteo de hijos directos (sub-Mónadas + archivos cacheados) sin
    /// resolver la query — barato para listar.
    fn nodo_monada(m: &MonadManifest) -> Node {
        let hijos = m.cardinality as usize + m.submonads.len();
        Node::new(format!("{PREF_MONADA}{}", m.id), format!("{} ({})", m.label, hijos), true)
            .with_kind(NodeKind::Synthetic)
    }

    /// Nodo de un archivo miembro (hoja POSIX). Usa el tamaño/mtime ya
    /// capturados por el scanner — sin `stat` extra.
    fn nodo_archivo(f: &FileEntry) -> Node {
        let nombre = f
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.path.to_string_lossy().into_owned());
        let mut nodo = Node::new(format!("{PREF_ARCHIVO}{}", f.id), nombre, false).with_size(f.size);
        if f.mtime_ms > 0 {
            nodo = nodo.with_mtime(f.mtime_ms);
        }
        nodo
    }
}

/// Parsea un id `m:<ulid>` a su [`MonadId`].
fn parse_monada(id: &str) -> Option<MonadId> {
    id.strip_prefix(PREF_MONADA)
        .and_then(|s| Ulid::from_string(s).ok())
}

/// Parsea un id `f:<ulid>` a su `FileId`.
fn parse_archivo(id: &str) -> Option<Ulid> {
    id.strip_prefix(PREF_ARCHIVO)
        .and_then(|s| Ulid::from_string(s).ok())
}

impl Source for NouserSource {
    fn label(&self) -> String {
        self.etiqueta.clone()
    }

    fn root(&self) -> Node {
        Node::new(RAIZ, self.etiqueta.clone(), true).with_kind(NodeKind::Synthetic)
    }

    fn children(&self, id: &NodeId) -> io::Result<Vec<Node>> {
        if id == RAIZ {
            return Ok(self.top_level().into_iter().map(Self::nodo_monada).collect());
        }
        if let Some(mid) = parse_monada(id) {
            if self.db.monad(mid).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Mónada inexistente: {id}"),
                ));
            }
            // Sub-Mónadas primero (contenedores), luego archivos efectivos
            // (curados ∪ intensional ∪ pines), resueltos por el grafo.
            let mut hijos: Vec<Node> = resolve::child_monads(&self.db, mid)
                .into_iter()
                .map(Self::nodo_monada)
                .collect();
            for fid in resolve::effective_members(&self.db, mid) {
                if let Some(f) = self.db.file(fid) {
                    hijos.push(Self::nodo_archivo(f));
                }
            }
            return Ok(hijos);
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("una hoja no tiene hijos: {id}"),
        ))
    }

    fn read(&self, id: &NodeId) -> io::Result<Vec<u8>> {
        let fid = parse_archivo(id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("sólo los archivos miembro son leíbles: {id}"),
            )
        })?;
        let archivo = self.db.file(fid).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("archivo inexistente: {id}"))
        })?;
        std::fs::read(&archivo.path)
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

    #[test]
    fn proyecta_dag_de_submonadas() {
        // Grafo armado a mano: "Fotos" contiene al álbum "Viaje" (sub-Mónada)
        // que a su vez contiene un archivo. Verifica que el adapter baja un
        // nivel de contención: root → Fotos → [Viaje] → [foto].
        let dir = tempfile::tempdir().unwrap();
        let foto = dir.path().join("playa.jpg");
        fs::File::create(&foto).unwrap();
        let files =
            scan_directory(dir.path(), &ScanConfig::default()).map_err(io::Error::other).unwrap();
        let foto_id = files[0].id;

        let mut db = MonadDb::new();
        db.ingest_files(files);

        let mut album = MonadManifest::new("Viaje");
        album.members.insert(foto_id);
        album.touch();
        let album_id = album.id;

        let mut fotos = MonadManifest::new("Fotos");
        fotos.submonads.insert(album_id);
        fotos.touch();
        let fotos_id = fotos.id;

        db.insert_monad(album);
        db.insert_monad(fotos);

        let src = NouserSource::from_db("test", db);

        // La raíz lista sólo Fotos (Viaje está contenida, no es top-level).
        let top = src.children(&RAIZ.to_string()).unwrap();
        assert_eq!(top.len(), 1, "sólo Fotos es de nivel superior");
        assert!(top[0].name.starts_with("Fotos"));

        // Bajar a Fotos muestra a Viaje como contenedor.
        let hijos = src.children(&format!("{PREF_MONADA}{fotos_id}")).unwrap();
        assert_eq!(hijos.len(), 1);
        assert!(hijos[0].is_container && hijos[0].name.starts_with("Viaje"));

        // Bajar a Viaje muestra la foto leíble.
        let nietos = src.children(&format!("{PREF_MONADA}{album_id}")).unwrap();
        assert_eq!(nietos.len(), 1);
        assert_eq!(nietos[0].name, "playa.jpg");
        assert!(src.read(&nietos[0].id).is_ok());
    }

    #[test]
    fn proyecta_monada_intensional() {
        // Una Mónada intensional "Imágenes" (query Lens=Gallery) capta el
        // .png del corpus sin tenerlo en members.
        let dir = tempfile::tempdir().unwrap();
        fs::File::create(dir.path().join("a.png")).unwrap();
        fs::File::create(dir.path().join("b.rs")).unwrap();
        let files =
            scan_directory(dir.path(), &ScanConfig::default()).map_err(io::Error::other).unwrap();

        let mut db = MonadDb::new();
        db.ingest_files(files);

        let mut img = MonadManifest::new("Imágenes");
        img.query = Some(chasqui_core::MonadQuery::imagenes());
        img.touch();
        let img_id = img.id;
        db.insert_monad(img);

        let src = NouserSource::from_db("test", db);
        let hijos = src.children(&format!("{PREF_MONADA}{img_id}")).unwrap();
        assert_eq!(hijos.len(), 1, "sólo el png entra por la query");
        assert_eq!(hijos[0].name, "a.png");
    }
}
