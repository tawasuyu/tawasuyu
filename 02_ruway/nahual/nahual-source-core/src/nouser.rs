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
use std::sync::{Arc, RwLock};

use chasqui_core::ulid::Ulid;
use chasqui_core::cluster::by_directory;
use chasqui_core::{edit, FileEntry, FileId, Lens, MonadId, MonadManifest};
use chasqui_core::db::MonadDb;
use chasqui_core::resolve;
use chasqui_core::scanner::{scan_directory, ScanConfig};

use crate::{MonadGraphMut, Node, NodeId, NodeKind, Source};

/// Id de la raíz sintética que lista las Mónadas de nivel superior.
const RAIZ: &str = "@monadas";
/// Prefijo de id de una Mónada (contenedor sintético).
const PREF_MONADA: &str = "m:";
/// Prefijo de id de un archivo miembro (hoja POSIX).
const PREF_ARCHIVO: &str = "f:";

/// Mime-hint del lente de una Mónada — el **vehículo para que el lente
/// cruce la frontera `dyn Source`**. El front (`nahual-shell`) navega un
/// `Box<dyn Source>` y no puede preguntar el `dominant_lens` de chasqui;
/// pero sí lee `Node.mime_hint`. Etiquetamos cada nodo-Mónada con
/// `monada/<lente>` (namespace propio, no colisiona con mimes de archivo,
/// y un contenedor sintético nunca se discierne ni se abre como hoja), y el
/// front mapea ese hint a la vista/app de la Mónada. `None` para `Grid`
/// (sin lente fuerte: el front usa su vista por defecto).
pub fn lens_mime(lens: Lens) -> Option<&'static str> {
    Some(match lens {
        Lens::Gallery => "monada/gallery",
        Lens::Code => "monada/code",
        Lens::Database => "monada/database",
        Lens::Markdown => "monada/markdown",
        Lens::Tree => "monada/tree",
        Lens::Grid => return None,
    })
}

/// Fuente que navega el grafo de Mónadas de un directorio.
///
/// El grafo vive detrás de `Arc<RwLock<…>>` (interior mutability) para que la
/// cara [`MonadGraphMut`] pueda editarlo con `&self` —igual que `SourceMut`
/// edita el filesystem con `&self`— manteniendo `Source` object-safe. Los
/// lectores (navegación) toman el lock de lectura; las ediciones, el de
/// escritura.
pub struct NouserSource {
    etiqueta: String,
    db: Arc<RwLock<MonadDb>>,
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

        Ok(Self::from_db(dir.to_string_lossy().into_owned(), db))
    }

    /// Construye la fuente sobre un grafo ya armado. Es la vía para montar
    /// un grafo con sub-Mónadas / Mónadas intensionales (construido por la
    /// capa de edición o por tests) sin pasar por `escanear`.
    pub fn from_db(label: impl Into<String>, db: MonadDb) -> Self {
        Self { etiqueta: label.into(), db: Arc::new(RwLock::new(db)) }
    }

    /// Corre `f` con acceso de sólo lectura al grafo subyacente (toma el lock
    /// de lectura). Para consultas que el front quiera hacer sobre el grafo
    /// (p. ej. `resolve::transitive_files`) sin exponer el lock.
    pub fn with_db<R>(&self, f: impl FnOnce(&MonadDb) -> R) -> R {
        f(&self.db.read().expect("MonadDb lock envenenado"))
    }

    /// Mónadas de nivel superior (las que ninguna otra contiene) como nodos.
    fn top_level_nodes(db: &MonadDb) -> Vec<Node> {
        let mut contenidas: BTreeSet<MonadId> = BTreeSet::new();
        for m in db.monads() {
            contenidas.extend(m.submonads.iter().copied());
        }
        db.monads()
            .filter(|m| !contenidas.contains(&m.id))
            .map(Self::nodo_monada)
            .collect()
    }

    /// Nodo de una Mónada (contenedor sintético). La etiqueta muestra el
    /// conteo de hijos directos (sub-Mónadas + archivos cacheados) sin
    /// resolver la query — barato para listar.
    fn nodo_monada(m: &MonadManifest) -> Node {
        let hijos = m.cardinality as usize + m.submonads.len();
        let mut nodo =
            Node::new(format!("{PREF_MONADA}{}", m.id), format!("{} ({})", m.label, hijos), true)
                .with_kind(NodeKind::Synthetic);
        // El lente viaja por mime_hint para que el front despache la vista.
        if let Some(hint) = lens_mime(m.dominant_lens) {
            nodo = nodo.with_mime_hint(hint);
        }
        nodo
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
        let db = self.db.read().expect("MonadDb lock envenenado");
        if id == RAIZ {
            return Ok(Self::top_level_nodes(&db));
        }
        if let Some(mid) = parse_monada(id) {
            if db.monad(mid).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Mónada inexistente: {id}"),
                ));
            }
            // Sub-Mónadas primero (contenedores), luego archivos efectivos
            // (curados ∪ intensional ∪ pines), resueltos por el grafo.
            let mut hijos: Vec<Node> = resolve::child_monads(&db, mid)
                .into_iter()
                .map(Self::nodo_monada)
                .collect();
            for fid in resolve::effective_members(&db, mid) {
                if let Some(f) = db.file(fid) {
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
        // Sacá la ruta bajo el lock y soltalo antes del I/O de disco.
        let path = self
            .db
            .read()
            .expect("MonadDb lock envenenado")
            .file(fid)
            .map(|f| f.path.clone())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("archivo inexistente: {id}"))
            })?;
        std::fs::read(&path)
    }

    fn monad_graph(&self) -> Option<&dyn MonadGraphMut> {
        Some(self)
    }
}

/// Mapea un [`edit::EditError`] a `io::Error` para cruzar la frontera del trait.
fn edit_err(e: edit::EditError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, e.to_string())
}

/// Exige que `id` sea una Mónada (`m:<ulid>`).
fn exigir_monada(id: &NodeId) -> io::Result<MonadId> {
    parse_monada(id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("no es una Mónada: {id}")))
}

impl MonadGraphMut for NouserSource {
    fn submonadize(
        &self,
        parent: &NodeId,
        label: &str,
        members: &[NodeId],
    ) -> io::Result<NodeId> {
        let pid = exigir_monada(parent)?;
        // Partí la selección: ids `f:` son archivos, `m:` son sub-Mónadas.
        let mut files: Vec<FileId> = Vec::new();
        let mut subs: Vec<MonadId> = Vec::new();
        for m in members {
            if let Some(fid) = parse_archivo(m) {
                files.push(fid);
            } else if let Some(mid) = parse_monada(m) {
                subs.push(mid);
            }
        }
        let mut db = self.db.write().expect("MonadDb lock envenenado");
        let hija = edit::submonadize(&mut db, pid, label, &files, &subs).map_err(edit_err)?;
        Ok(format!("{PREF_MONADA}{hija}"))
    }

    fn rename_monad(&self, id: &NodeId, label: &str) -> io::Result<()> {
        let mid = exigir_monada(id)?;
        let mut db = self.db.write().expect("MonadDb lock envenenado");
        edit::rename(&mut db, mid, label).map_err(edit_err)
    }

    fn merge_monads(&self, into: &NodeId, from: &NodeId) -> io::Result<()> {
        let i = exigir_monada(into)?;
        let f = exigir_monada(from)?;
        let mut db = self.db.write().expect("MonadDb lock envenenado");
        edit::merge(&mut db, i, f).map_err(edit_err)
    }

    fn delete_monad(&self, id: &NodeId) -> io::Result<()> {
        let mid = exigir_monada(id)?;
        let mut db = self.db.write().expect("MonadDb lock envenenado");
        edit::delete_monad(&mut db, mid);
        Ok(())
    }

    fn monad_open_target(&self, id: &NodeId) -> Option<String> {
        let mid = parse_monada(id)?;
        let db = self.db.read().ok()?;
        db.monad(mid).and_then(|m| m.path_hint.clone())
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
    fn nodo_monada_lleva_el_lente_en_mime_hint() {
        // El lente de la Mónada cruza la frontera Source por mime_hint, para
        // que el front pueda fijar la vista al entrar.
        let mut db = MonadDb::new();
        let mut foto = MonadManifest::new("Fotos");
        foto.dominant_lens = Lens::Gallery;
        foto.members.insert(Ulid::new()); // miembro fantasma → no-vacía
        foto.touch();
        db.insert_monad(foto);

        let src = NouserSource::from_db("t", db);
        let top = src.children(&RAIZ.to_string()).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].mime_hint.as_deref(), Some("monada/gallery"));
    }

    #[test]
    fn editar_grafo_via_monad_graph() {
        // Editá el grafo a TRAVÉS del trait object `dyn Source` —el mismo camino
        // que el shell tiene— y verificá que la navegación refleja el cambio.
        let dir = tempfile::tempdir().unwrap();
        for n in ["a.rs", "b.rs", "c.rs"] {
            fs::File::create(dir.path().join(n)).unwrap();
        }
        let files =
            scan_directory(dir.path(), &ScanConfig::default()).map_err(io::Error::other).unwrap();
        let mut db = MonadDb::new();
        db.ingest_files(files.clone());

        let mut padre = MonadManifest::new("todo");
        for f in &files {
            padre.members.insert(f.id);
        }
        padre.touch();
        let padre_node = format!("{PREF_MONADA}{}", padre.id);
        db.insert_monad(padre);

        // Detrás de un `Box<dyn Source>`, como lo ve el shell.
        let src: Box<dyn Source> = Box::new(NouserSource::from_db("t", db));
        let graph = src.monad_graph().expect("nouser ofrece MonadGraphMut");

        // Submonadizá dos de los tres archivos a una hija "sub".
        let seleccion: Vec<NodeId> =
            vec![format!("{PREF_ARCHIVO}{}", files[0].id), format!("{PREF_ARCHIVO}{}", files[1].id)];
        let hija = graph.submonadize(&padre_node, "sub", &seleccion).unwrap();
        assert!(hija.starts_with(PREF_MONADA));

        // Navegando el padre: ahora hay 1 archivo + la hija (contenedor).
        let hijos = src.children(&padre_node).unwrap();
        let contenedores = hijos.iter().filter(|n| n.is_container).count();
        let archivos = hijos.iter().filter(|n| !n.is_container).count();
        assert_eq!(contenedores, 1, "la hija aparece como sub-Mónada");
        assert_eq!(archivos, 1, "el padre soltó 2 de 3 archivos");

        // La hija tiene los 2 trasladados.
        assert_eq!(src.children(&hija).unwrap().len(), 2);

        // Renombrar y borrar también cruzan el trait.
        graph.rename_monad(&hija, "viaje").unwrap();
        assert!(src.children(&padre_node).unwrap().iter().any(|n| n.name.starts_with("viaje")));
        graph.delete_monad(&hija).unwrap();
        assert!(!src.children(&padre_node).unwrap().iter().any(|n| n.is_container));
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
