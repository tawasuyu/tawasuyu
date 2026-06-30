//! `nahual-source-core` — la **fuente navegable** agnóstica del front
//! universal (Brahman, Fase 3).
//!
//! El norte de `nahual-shell` es abrir, con la misma UI, "una raíz de minga,
//! un objeto de wawa, una Mónada de nouser o un archivo POSIX" (ver
//! `/BRAHMAN.md`, sección "proliferación de exploradores"). Cada uno de esos
//! mundos tiene su propio modelo de árbol —`std::fs`, un DAG BLAKE3, clusters
//! semánticos— pero todos comparten la misma forma mínima: una **raíz**, una
//! manera de **listar hijos** de un contenedor, y una manera de **leer los
//! bytes** de una hoja. Eso es [`Source`].
//!
//! La proliferación de exploradores NO se cura fusionando sus datos
//! (incompatibles), sino poniéndolos detrás de esta interfaz común: el shell
//! deja de saber de `PathBuf` y pasa a navegar `dyn Source`. Hoy hay tres
//! adapters reales:
//!
//! - [`posix::PosixSource`] — el filesystem POSIX vivo (lo que el shell ya
//!   hacía, ahora detrás del trait).
//! - [`wawa::WawaImgSource`] — los objetos content-addressed de una imagen
//!   wawa `.img`, navegando el DAG por hash. Puro local, sin red ni daemon.
//! - `nouser::NouserSource` (feature `nouser`) — las Mónadas semánticas de
//!   `chasqui-core`: clusters de archivos, un árbol que NO existe en disco.
//! - `minga::MingaSource` (feature `minga`) — el grafo CAS de AST de un repo
//!   minga: un DAG de nodos de código etiquetados por su `kind`.
//!
//! Cada uno es una *forma de árbol* distinta (jerarquía física, DAG de
//! contenido, clusters semánticos, DAG de AST) y aun así caben en el mismo
//! trait — esa es la prueba de que la abstracción aguanta. Los cuatro mundos
//! que el BRAHMAN.md nombra (POSIX · wawa · nouser · minga) son ahora una
//! sola espina.

#![forbid(unsafe_code)]

pub mod archive;
pub mod dispositivo;
pub mod navigator;
pub mod posix;
pub mod wawa;
#[cfg(feature = "nouser")]
pub mod nouser;
#[cfg(feature = "nouser-daemon")]
pub mod nouser_daemon;
#[cfg(feature = "minga")]
pub mod minga;

pub use archive::ArchiveSource;
pub use dispositivo::{es_id_de_dispositivo, DispositivoInfo, DispositivosSource};
pub use navigator::{Navigator, Opened, SortDir, SortKey, ViewMode};
pub use posix::PosixSource;
pub use wawa::WawaImgSource;
#[cfg(feature = "nouser")]
pub use nouser::{lens_mime, NouserSource};
/// Re-export del modelo de Mónada de chasqui, para que el front (el shell)
/// despache por el lente / construya grafos sin depender de chasqui
/// directamente — `nahual-source-core` es su seam hacia el dominio nouser.
#[cfg(feature = "nouser")]
pub use chasqui_core::{
    cluster, db::MonadDb, edit, resolve, scanner, FileEntry, FileId, Lens, MonadId, MonadManifest,
    MonadQuery,
};
#[cfg(feature = "nouser-daemon")]
pub use nouser_daemon::NouserDaemonSource;
#[cfg(feature = "minga")]
pub use minga::MingaSource;

/// Identidad opaca de un nodo DENTRO de su fuente. El shell la trata como
/// caja negra (la guarda para volver a pedir hijos o leer), salvo para
/// derivar la identidad de contenido al despachar el visor.
///
/// La codificación es decisión de cada [`Source`]: POSIX usa la ruta
/// absoluta; wawa usa el hash en hex. No mezclar ids entre fuentes.
pub type NodeId = String;

/// Naturaleza de un nodo — para iconografía y para la columna "tipo" de la
/// vista detalle. Es ortogonal a `is_container` (que es la bandera de
/// navegabilidad): un `Archive` o un `Symlink`-a-dir pueden ser contenedores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NodeKind {
    /// Archivo regular (hoja POSIX o equivalente).
    #[default]
    File,
    /// Directorio / contenedor jerárquico.
    Dir,
    /// Enlace simbólico (POSIX). `is_container` refleja a qué apunta.
    Symlink,
    /// Archivo-contenedor (zip/tar) montable como carpeta.
    Archive,
    /// Contenedor sintético que no existe como entidad física (la raíz
    /// `@imagen` de wawa, una Mónada de nouser, la raíz `@nodos` de minga).
    Synthetic,
}

/// Un nodo del árbol de una [`Source`]: lo mínimo que la UI necesita para
/// pintar una fila y decidir si se puede descender. Los campos de metadata
/// son `Option` a propósito — una fuente que no los tiene (clusters
/// sintéticos, DAGs anónimos) devuelve `None` y la columna sale "—".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    /// Identidad estable dentro de la fuente (ver [`NodeId`]).
    pub id: NodeId,
    /// Nombre legible para la fila.
    pub name: String,
    /// `true` si se puede descender (tiene hijos / es directorio); `false`
    /// si es una hoja que se abre en el visor.
    pub is_container: bool,
    /// Tamaño en bytes, si la fuente lo conoce.
    pub size: Option<u64>,
    /// Última modificación en epoch-ms, si la fuente lo conoce.
    pub mtime: Option<u64>,
    /// Naturaleza del nodo (para icono y columna "tipo").
    pub kind: NodeKind,
    /// MIME ya sabido por la fuente — evita re-discernir el contenido si la
    /// fuente ya lo tiene (p. ej. una entrada de archivo con tipo declarado).
    pub mime_hint: Option<String>,
}

impl Node {
    /// Atajo para construir un nodo. `kind` se deriva de `is_container`
    /// (`Dir`/`File`); usá los `with_*` para refinar metadata.
    pub fn new(id: impl Into<NodeId>, name: impl Into<String>, is_container: bool) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            is_container,
            size: None,
            mtime: None,
            kind: if is_container { NodeKind::Dir } else { NodeKind::File },
            mime_hint: None,
        }
    }

    /// Fija el tamaño en bytes.
    pub fn with_size(mut self, size: u64) -> Self {
        self.size = Some(size);
        self
    }

    /// Fija la última modificación en epoch-ms.
    pub fn with_mtime(mut self, mtime_ms: u64) -> Self {
        self.mtime = Some(mtime_ms);
        self
    }

    /// Fija la naturaleza del nodo (anula la derivada de `is_container`).
    pub fn with_kind(mut self, kind: NodeKind) -> Self {
        self.kind = kind;
        self
    }

    /// Fija el MIME conocido por la fuente.
    pub fn with_mime_hint(mut self, mime: impl Into<String>) -> Self {
        self.mime_hint = Some(mime.into());
        self
    }
}

/// Una fuente navegable: el contrato agnóstico que el front universal
/// consume. Object-safe a propósito — el shell guarda `Box<dyn Source>` y
/// puede apilar fuentes (descender de un `.img` POSIX a su DAG wawa).
///
/// `Send + Sync` para poder escanear en un worker (`Handle::spawn`) sin
/// devolver el árbol entero por el canal de mensajes.
pub trait Source: Send + Sync {
    /// Nombre humano de la fuente — para breadcrumb / título del panel.
    fn label(&self) -> String;

    /// El nodo raíz desde el que se empieza a navegar.
    fn root(&self) -> Node;

    /// Hijos directos de un contenedor, en orden ya presentable. Error si el
    /// id no existe o no es un contenedor.
    fn children(&self, id: &NodeId) -> std::io::Result<Vec<Node>>;

    /// Bytes de una hoja — para discernir (`shuma-discern`) y visualizar.
    /// Error si el id no existe o no tiene contenido leíble.
    fn read(&self, id: &NodeId) -> std::io::Result<Vec<u8>>;

    /// Si la fuente soporta mutación, devuelve su cara [`SourceMut`]; si es
    /// read-only (wawa/minga son CAS inmutables, nouser es derivado), `None`.
    ///
    /// El front gatea la UI con esto: sin `SourceMut`, los ítems de operación
    /// (crear/borrar/renombrar/mover/copiar) salen deshabilitados. Frontera
    /// honesta — no se finge escritura sobre lo que no la tiene.
    fn writable(&self) -> Option<&dyn SourceMut> {
        None
    }

    /// Si la fuente es un **grafo de Mónadas editable**, devuelve su cara
    /// [`MonadGraphMut`]; si no (POSIX/wawa/minga), `None`. Es el análogo de
    /// [`writable`](Self::writable) para editar la **organización** en vez de
    /// los archivos: submonadizar, fusionar, renombrar, borrar Mónadas. La
    /// frontera honesta vuelve a aplicar — sólo el grafo de nouser la ofrece.
    fn monad_graph(&self) -> Option<&dyn MonadGraphMut> {
        None
    }
}

/// La cara **editable del grafo de Mónadas**. Trait aparte de [`SourceMut`]
/// porque opera sobre la *organización* (qué Mónada contiene qué), no sobre
/// el filesystem: ningún archivo se mueve en disco. Sólo el grafo de nouser
/// lo implementa.
///
/// Las operaciones toman `&self` (como [`SourceMut`]) — el grafo mutable vive
/// detrás de interior mutability en el adapter, así el trait queda object-safe
/// tras un `&dyn` y el front lo llama sin exclusividad. Los ids son [`NodeId`]
/// opacos (los mismos que la navegación entrega), así el front no necesita
/// conocer los tipos de chasqui.
pub trait MonadGraphMut: Source {
    /// **Submonadiza**: crea una Mónada hija de `parent` con `label`, le
    /// traslada los `members` (ids de archivos y/o sub-Mónadas que hoy cuelgan
    /// de `parent`) y la cuelga de `parent`. Devuelve el id de la hija. Es la
    /// operación canónica de "agrupar esta selección en su propia Mónada".
    fn submonadize(
        &self,
        parent: &NodeId,
        label: &str,
        members: &[NodeId],
    ) -> std::io::Result<NodeId>;

    /// Renombra la Mónada `id`.
    fn rename_monad(&self, id: &NodeId, label: &str) -> std::io::Result<()>;

    /// Fusiona `from` en `into`: traslada el contenido de `from` a `into`,
    /// repunta a los padres de `from` y la borra.
    fn merge_monads(&self, into: &NodeId, from: &NodeId) -> std::io::Result<()>;

    /// Borra la Mónada `id` y la desvincula de todo padre. No borra archivos
    /// ni sub-Mónadas — sólo disuelve el agrupamiento.
    fn delete_monad(&self, id: &NodeId) -> std::io::Result<()>;

    /// **Anclaje en disco** de la Mónada `id`, si tiene uno: el directorio del
    /// que se clusterizó. Es lo que el front abre cuando "abrís la Mónada con
    /// una app" (galería→tullpu, etc.) — la app recibe ese directorio.
    /// `None` para Mónadas intensionales/derivadas sin raíz física.
    fn monad_open_target(&self, id: &NodeId) -> Option<String>;
}

/// La cara **mutable** de una fuente. Trait aparte (no métodos en [`Source`])
/// para que las fuentes inmutables —wawa y minga son content-addressed, sus
/// objetos no se editan en sitio; nouser es un derivado de POSIX— no tengan
/// que fingir operaciones que no existen. Hoy sólo [`posix::PosixSource`] lo
/// implementa.
///
/// Todas las operaciones toman `&self` (no `&mut`): el estado mutable vive en
/// el filesystem, no en el adapter, así que no hace falta exclusividad. Eso
/// también mantiene el trait object-safe detrás de un `&dyn`.
pub trait SourceMut: Source {
    /// Crea un directorio `name` dentro del contenedor `parent`. Devuelve el
    /// [`NodeId`] del nuevo directorio.
    fn create_dir(&self, parent: &NodeId, name: &str) -> std::io::Result<NodeId>;

    /// Crea un archivo vacío `name` dentro de `parent`. Devuelve su id.
    fn create_file(&self, parent: &NodeId, name: &str) -> std::io::Result<NodeId>;

    /// Borra el nodo `id` (recursivo si es contenedor).
    fn delete(&self, id: &NodeId) -> std::io::Result<()>;

    /// Renombra el nodo `id` a `new_name`, dentro de su mismo contenedor.
    /// Devuelve el nuevo id (la identidad puede cambiar — en POSIX el id ES
    /// la ruta).
    fn rename(&self, id: &NodeId, new_name: &str) -> std::io::Result<NodeId>;

    /// Mueve `id` al contenedor `new_parent`. Devuelve el id resultante.
    fn move_into(&self, id: &NodeId, new_parent: &NodeId) -> std::io::Result<NodeId>;

    /// Copia `id` (recursivo si es contenedor) dentro de `new_parent`.
    /// Devuelve el id de la copia.
    fn copy_into(&self, id: &NodeId, new_parent: &NodeId) -> std::io::Result<NodeId>;

    /// Sobrescribe los bytes de la hoja `id`.
    fn write(&self, id: &NodeId, bytes: &[u8]) -> std::io::Result<()>;
}

/// Codifica 32 bytes a hex en minúscula (64 chars). Compartido por el
/// adapter wawa y reusable por cualquier fuente content-addressed futura.
pub(crate) fn to_hex(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Decodifica 64 chars hex a 32 bytes. `None` si la longitud o los dígitos
/// son inválidos.
pub(crate) fn from_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = (bytes[2 * i] as char).to_digit(16)?;
        let lo = (bytes[2 * i + 1] as char).to_digit(16)?;
        *slot = (hi * 16 + lo) as u8;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
            0xee, 0xff, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ];
        let hex = to_hex(&h);
        assert_eq!(hex.len(), 64);
        assert_eq!(from_hex(&hex), Some(h));
    }

    #[test]
    fn from_hex_rechaza_basura() {
        assert_eq!(from_hex("corto"), None);
        assert_eq!(from_hex(&"z".repeat(64)), None);
    }
}
