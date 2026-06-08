//! `llimphi-module-selector` — abstracción de "abrir/guardar" portable
//! entre host (paths del FS) y wawa (khipus por hash).
//!
//! ## Por qué
//!
//! Una app tawasuyu que sólo conoce paths (`PathBuf`) se rompe en wawa,
//! donde el almacenamiento es direccionado por contenido (BLAKE3 + DAG)
//! y no existe el concepto de "carpeta /home/usuario". Pero la mayoría
//! de las apps no necesitan saber la diferencia: sólo quieren preguntar
//! "qué item quiere abrir el usuario" o "dónde guardo este blob".
//!
//! Este crate expone:
//! - El trait [`Selector`] con dos métodos: `list_candidates()` (para
//!   armar la UI del picker) y `realize(handle)` (para resolver el
//!   item elegido a bytes).
//! - Un `ItemHandle` opaco — la app no debe inspeccionarlo, sólo
//!   pasarlo de vuelta al selector.
//! - [`HostSelector`] con root path + extension filter (impl real).
//! - [`WawaSelector`] como **placeholder** con la API definida — la
//!   integración real con `akasha` / `wawa-kernel` ocurre cuando la
//!   suite empiece a correr in-cage. Por ahora exporta tipos y panica
//!   si se invoca, lo cual está bien: el código que lo construye
//!   queda compilable y las apps pueden tipar contra el trait.
//!
//! ## API mínima
//!
//! ```ignore
//! use llimphi_module_selector::{HostSelector, Selector};
//!
//! let sel = HostSelector::new("/home/usr/docs", &[".pluma", ".khipu"]);
//! let items = sel.list_candidates()?;
//! // (la app muestra `items.iter().map(|i| &i.display_name)` en su picker)
//! // user elige el index N:
//! let bytes = sel.realize(&items[N].handle)?;
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// Resultado de la operación — `String` como error para que no le
/// importe a la app si el backend es FS o wawa.
pub type SelectorResult<T> = Result<T, String>;

/// Item visible en el picker. `handle` es opaco — sólo el `Selector`
/// que lo emitió sabe interpretarlo.
#[derive(Debug, Clone)]
pub struct Item {
    /// Nombre legible para mostrar en el picker. Para `HostSelector`
    /// es el path relativo al root; para `WawaSelector` será el alias
    /// del khipu o un hash truncado si no tiene alias.
    pub display_name: String,
    /// Tamaño en bytes si se conoce — para mostrar al lado del nombre.
    /// `None` cuando es caro de calcular (e.g. khipu blob remoto).
    pub size_bytes: Option<u64>,
    pub handle: ItemHandle,
}

/// Handle opaco. Internamente puede ser un path (host) o un hash
/// (wawa). La app no debe construir uno a mano — lo recibe del
/// `Selector` y se lo devuelve al `realize()`.
#[derive(Debug, Clone)]
pub enum ItemHandle {
    /// Path absoluto en el FS del host.
    HostPath(PathBuf),
    /// Hash de contenido BLAKE3 (32 bytes hex) en el almacén wawa.
    /// La integración real lo resuelve via `almacen::cargar(hash)`.
    WawaHash([u8; 32]),
}

/// Trait que abstrae el medio de almacenamiento. Una app tawasuyu que
/// quiera funcionar tanto en host como en wawa toma un `&dyn Selector`
/// en su modelo en lugar de un `PathBuf` concreto.
pub trait Selector {
    /// Lista los items "abribles" según los criterios del selector
    /// (extensión, glob, scope). Para host suele ser un walk del root;
    /// para wawa, los khipus marcados con cierto namespace.
    fn list_candidates(&self) -> SelectorResult<Vec<Item>>;

    /// Resuelve un `ItemHandle` a los bytes del item.
    fn realize(&self, handle: &ItemHandle) -> SelectorResult<Vec<u8>>;

    /// Guarda `bytes` bajo el nombre lógico `name`. Devuelve el
    /// `ItemHandle` del item recién creado. Para host esto es
    /// `root.join(name) + write`; para wawa, ingerir en el almacén.
    fn save(&self, name: &str, bytes: &[u8]) -> SelectorResult<ItemHandle>;
}

// =====================================================================
// HostSelector — backend de FS clásico
// =====================================================================

/// Selector que walkea un root del filesystem y filtra por extensión.
/// Implementación lineal — para roots gigantes la app debería cachear
/// los candidates al arrancar (igual que hace el `file-picker` actual).
pub struct HostSelector {
    root: PathBuf,
    /// Lista de extensiones aceptadas (con el punto, ej. `".pluma"`).
    /// Vacío = todas.
    extensions: Vec<String>,
}

impl HostSelector {
    pub fn new(root: impl Into<PathBuf>, extensions: &[&str]) -> Self {
        Self {
            root: root.into(),
            extensions: extensions.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn accept(&self, path: &Path) -> bool {
        if self.extensions.is_empty() {
            return true;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            return false;
        };
        self.extensions.iter().any(|ext| name.ends_with(ext))
    }

    fn walk(&self, dir: &Path, out: &mut Vec<Item>) -> SelectorResult<()> {
        let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Saltamos directorios "ruidosos" (target, .git, node_modules).
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if matches!(name, "target" | ".git" | "node_modules" | ".idea") {
                        continue;
                    }
                }
                self.walk(&path, out)?;
            } else if self.accept(&path) {
                let display_name = path
                    .strip_prefix(&self.root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string());
                let size_bytes = entry.metadata().ok().map(|m| m.len());
                out.push(Item {
                    display_name,
                    size_bytes,
                    handle: ItemHandle::HostPath(path),
                });
            }
        }
        Ok(())
    }
}

impl Selector for HostSelector {
    fn list_candidates(&self) -> SelectorResult<Vec<Item>> {
        let mut out = Vec::new();
        self.walk(&self.root, &mut out)?;
        Ok(out)
    }

    fn realize(&self, handle: &ItemHandle) -> SelectorResult<Vec<u8>> {
        match handle {
            ItemHandle::HostPath(p) => std::fs::read(p).map_err(|e| e.to_string()),
            ItemHandle::WawaHash(_) => Err("HostSelector no resuelve hashes wawa".into()),
        }
    }

    fn save(&self, name: &str, bytes: &[u8]) -> SelectorResult<ItemHandle> {
        let path = self.root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(ItemHandle::HostPath(path))
    }
}

// =====================================================================
// WawaSelector — placeholder para integración con akasha/almacen
// =====================================================================

/// Selector para entorno wawa. **No implementado** — la integración real
/// requiere bindings al `wawa-kernel::almacen` (BLAKE3 + log + GC), que
/// vive fuera del workspace global. Por ahora expone la API para que el
/// código que la usa compile, y panica en runtime para flaggear que
/// alguien intentó usarla antes de tiempo.
///
/// Cuando llegue la integración real:
/// 1. `wawa-kernel` exporta una crate `wawa-almacen-client` cross-bound
///    accesible desde apps WASM.
/// 2. `WawaSelector::new(namespace)` se conecta a ese cliente.
/// 3. `list_candidates()` consulta `almacen::listar(namespace)`.
/// 4. `realize(WawaHash(h))` invoca `almacen::cargar(h)`.
/// 5. `save(name, bytes)` invoca `almacen::ingerir(bytes)` y registra
///    el alias `name → hash`.
pub struct WawaSelector {
    /// Namespace lógico (ej. `"pluma.documentos"`) — el almacén filtra
    /// los khipus marcados con este tag.
    pub namespace: String,
}

impl WawaSelector {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self { namespace: namespace.into() }
    }
}

impl Selector for WawaSelector {
    fn list_candidates(&self) -> SelectorResult<Vec<Item>> {
        Err(format!(
            "WawaSelector('{}') sin backend wawa registrado — pendiente de integración con wawa-almacen-client",
            self.namespace
        ))
    }

    fn realize(&self, _handle: &ItemHandle) -> SelectorResult<Vec<u8>> {
        Err("WawaSelector::realize sin backend wawa registrado".into())
    }

    fn save(&self, _name: &str, _bytes: &[u8]) -> SelectorResult<ItemHandle> {
        Err("WawaSelector::save sin backend wawa registrado".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_selector_accept_with_extensions() {
        let s = HostSelector::new("/tmp", &[".pluma", ".khipu"]);
        assert!(s.accept(Path::new("/tmp/foo.pluma")));
        assert!(s.accept(Path::new("/tmp/bar.khipu")));
        assert!(!s.accept(Path::new("/tmp/baz.txt")));
    }

    #[test]
    fn host_selector_empty_extensions_accepts_all() {
        let s = HostSelector::new("/tmp", &[]);
        assert!(s.accept(Path::new("/tmp/anything.rs")));
        assert!(s.accept(Path::new("/tmp/anything.unknown")));
    }

    #[test]
    fn wawa_selector_returns_err_until_backend_lands() {
        let s = WawaSelector::new("pluma.documentos");
        assert!(s.list_candidates().is_err());
    }
}
