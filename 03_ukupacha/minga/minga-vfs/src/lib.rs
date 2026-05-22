//! `minga-vfs`: proyecta el repositorio de Minga —direccionado por
//! contenido semántico— como un filesystem FUSE de **sólo lectura**.
//!
//! Minga guarda código como un grafo de `StoredNode`s identificados por
//! `ContentHash`; los archivos ingeridos son las raíces del MST. Este
//! crate convierte ese grafo en algo que cualquier herramienta Unix
//! (`ls`, `cat`, `grep`, un editor) puede recorrer, sin exponer `sled`
//! ni la API del store.
//!
//! ## Layout del filesystem
//!
//! ```text
//! <punto-de-montaje>/
//! ├── README          explicación del propio VFS
//! ├── roots/          un archivo por raíz del MST (cada archivo ingerido)
//! │   └── <hash64>    código fuente reconstruido, formato normalizado
//! └── cas/            cualquier nodo del store, resuelto bajo demanda
//!     └── <hash64>    S-expression del subárbol con ese hash
//! ```
//!
//! `roots/` **se enumera** (`ls` lista todas las raíces). `cas/` no se
//! enumera —son potencialmente decenas de miles de nodos— pero
//! `cas/<hash>` resuelve cualquier hash conocido: ése es el "blob por
//! hash bajo demanda". El mismo hash bajo `roots/` y bajo `cas/` da dos
//! vistas del mismo nodo: fuente reconstruida vs. árbol literal.
//!
//! ## Arquitectura (separabilidad)
//!
//! - [`render`] — `SemanticNode` → texto. Lógica pura, sin IO ni FUSE;
//!   reutilizable por un frontend web o TUI.
//! - [`source`] — el contrato [`NodeSource`] y sus backends (`sled`
//!   vía [`RepoSource`], memoria vía [`MemSource`]).
//! - `fs` — el único módulo acoplado a `fuser`: traduce el contrato a
//!   la `Filesystem` trait.

mod fs;
pub mod render;
pub mod source;

pub use fs::MingaFs;
pub use source::{reconstruct, MemSource, NodeSource, RepoSource};

use std::io;
use std::path::Path;

use fuser::MountOption;

/// Opciones de montaje comunes: sólo lectura, etiquetado como `minga`
/// para que aparezca legible en `mount` / `df`.
fn mount_options() -> Vec<MountOption> {
    vec![
        MountOption::RO,
        MountOption::FSName("minga".to_string()),
        MountOption::Subtype("minga".to_string()),
    ]
}

/// Monta el VFS en `mountpoint` y **bloquea** hasta que se desmonte
/// (`fusermount -u <punto>`, `umount`, o una señal al proceso).
///
/// El punto de montaje debe ser un directorio existente. El filesystem
/// es de sólo lectura: toda escritura falla con `EROFS`/`EACCES`.
pub fn mount<S, P>(source: S, mountpoint: P) -> io::Result<()>
where
    S: NodeSource,
    P: AsRef<Path>,
{
    fuser::mount2(MingaFs::new(source), mountpoint, &mount_options())
}

/// Como [`mount`] pero spawnea un hilo de fondo y retorna de inmediato.
/// La sesión queda viva mientras el `BackgroundSession` no se dropee;
/// dropearlo desmonta el filesystem.
pub fn spawn_mount<S, P>(source: S, mountpoint: P) -> io::Result<fuser::BackgroundSession>
where
    S: NodeSource + Send + 'static,
    P: AsRef<Path>,
{
    fuser::spawn_mount2(MingaFs::new(source), mountpoint, &mount_options())
}
