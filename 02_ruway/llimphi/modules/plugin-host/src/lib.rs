//! llimphi-plugin-host — runtime de plugins WASM Tier 2 para apps Llimphi.
//!
//! Vea `docs/MODULES.md` (§Tier 2 — Plugins WASM) para el contrato
//! completo. En síntesis:
//!
//! - Un plugin es un `.wasm` + un `manifest.toml` hermano que declara
//!   `name`, `version`, `capabilities`, y los `Permissions` que pide.
//! - El host expone imports bajo el namespace `"plugin"`. Cada uno se
//!   gatea por un campo de `card_core::Permissions`: si el permiso falta,
//!   el import **no se enlaza** y el plugin trap-ea al intentar usarlo.
//! - El `.wasm` exporta `_invoke(cap_ptr, cap_len, arg_ptr, arg_len) -> i32`
//!   y una `memory` lineal.
//! - Invocar un plugin devuelve `PluginAction` — intención, no ejecución.
//!   El host decide cómo materializar `OpenAt`/`SetStatus` en su contexto.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use card_core::{FsPolicy, Permissions};
use serde::Deserialize;
use thiserror::Error;
use tracing::{info, warn};
use wasmi::{Caller, CompilationMode, Config, Engine, Linker, Memory, Module, Store};

// =====================================================================
// Manifest
// =====================================================================

/// Manifest sidecar (`manifest.toml`) que acompaña a cada `.wasm`.
///
/// El formato es estable: campos extra se ignoran con `#[serde(default)]`
/// donde aplica, para que plugins viejos sigan cargando si el host suma
/// metadatos opcionales.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    /// Capabilities que el plugin atiende. El host enruta invocaciones
    /// por el nombre exacto pasado a `PluginHost::invoke(_, cap, _)`.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Permisos que el plugin necesita para no trap-ear. Si el manifest
    /// pide más de lo que el host está dispuesto a conceder, la carga
    /// puede aceptarse "downgraded" — pero el plugin entonces trap-eará
    /// al intentar los imports que no se enlazaron. La política la fija
    /// quien llama a `PluginHost::load_*`.
    #[serde(default)]
    pub permissions: Permissions,
}

impl PluginManifest {
    pub fn from_toml(s: &str) -> Result<Self, PluginError> {
        toml::from_str(s).map_err(|e| PluginError::Manifest(e.to_string()))
    }
}

// =====================================================================
// Acciones y errores
// =====================================================================

/// Intención que el plugin emite. Igual que en los módulos Tier 1, el
/// plugin no sabe cómo el host materializa cada variante — sólo declara
/// qué quiere que pase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginAction {
    None,
    SetStatus(String),
    OpenAt { path: PathBuf, line: u32, col: u32 },
}

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("manifest inválido: {0}")]
    Manifest(String),
    #[error("no se pudo leer {0}: {1}")]
    Io(PathBuf, String),
    #[error("compilando wasm: {0}")]
    Compile(String),
    #[error("instanciando wasm: {0}")]
    Instantiate(String),
    #[error("plugin no exporta `_invoke` con la signatura esperada: {0}")]
    MissingEntry(String),
    #[error("trap durante la ejecución del plugin: {0}")]
    Trap(String),
    #[error("no existe plugin con id {0:?}")]
    UnknownPlugin(PluginId),
}

// =====================================================================
// Host
// =====================================================================

/// Identificador opaco de un plugin cargado. Sólo se construye desde
/// `PluginHost::load_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PluginId(u32);

struct LoadedPlugin {
    manifest: PluginManifest,
    module: Module,
}

/// Estado por invocación. Vive sólo durante un `invoke` — se descarta al
/// volver. Lo usamos como `Store::data()` para que los host imports
/// puedan emitir su `PluginAction` sin globals. Los permisos no viajan
/// aquí porque su efecto es link-time: los imports prohibidos
/// simplemente no se enlazan.
struct InvokeCtx {
    /// Acción a devolver al host. `RefCell` porque los closures de
    /// `func_wrap` toman `Caller` por referencia compartida.
    pending: RefCell<PluginAction>,
}

pub struct PluginHost {
    engine: Engine,
    plugins: Vec<LoadedPlugin>,
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost {
    pub fn new() -> Self {
        // Eager: mismo modo que arje-wasm, comportamiento predecible y
        // los traps de compilación salen en `load_*`, no en `invoke`.
        let mut config = Config::default();
        config.compilation_mode(CompilationMode::Eager);
        Self { engine: Engine::new(&config), plugins: Vec::new() }
    }

    /// Carga `dir/plugin.wasm` + `dir/manifest.toml`. Por convención el
    /// `.wasm` se llama igual que el directorio o `plugin.wasm`. Probamos
    /// ambos para ser indulgentes con el packaging.
    pub fn load_from_dir(&mut self, dir: impl AsRef<Path>) -> Result<PluginId, PluginError> {
        let dir = dir.as_ref();
        let manifest_path = dir.join("manifest.toml");
        let manifest_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| PluginError::Io(manifest_path.clone(), e.to_string()))?;
        let manifest = PluginManifest::from_toml(&manifest_str)?;

        let candidates = [dir.join("plugin.wasm"), dir.join(format!("{}.wasm", manifest.name))];
        let (wasm_path, wasm_bytes) = candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok().map(|b| (p.clone(), b)))
            .ok_or_else(|| {
                PluginError::Io(dir.join("plugin.wasm"), "no encontré ningún .wasm".into())
            })?;

        let _ = wasm_path;
        self.load_bytes(manifest, &wasm_bytes)
    }

    /// Carga un plugin desde bytes ya en memoria (útil en tests y para
    /// plugins embebidos en el binario del host).
    pub fn load_bytes(
        &mut self,
        manifest: PluginManifest,
        wasm_bytes: &[u8],
    ) -> Result<PluginId, PluginError> {
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| PluginError::Compile(e.to_string()))?;
        let id = PluginId(self.plugins.len() as u32);
        info!(
            plugin = %manifest.name,
            version = %manifest.version,
            capabilities = ?manifest.capabilities,
            "plugin Tier 2 cargado"
        );
        self.plugins.push(LoadedPlugin { manifest, module });
        Ok(id)
    }

    pub fn manifest(&self, id: PluginId) -> Result<&PluginManifest, PluginError> {
        self.plugins
            .get(id.0 as usize)
            .map(|p| &p.manifest)
            .ok_or(PluginError::UnknownPlugin(id))
    }

    /// Devuelve la unión de capabilities de todos los plugins cargados —
    /// la lista que el host enrola en su Card antes de `spawn_sidecar()`.
    pub fn all_capabilities(&self) -> Vec<String> {
        let mut caps: Vec<String> =
            self.plugins.iter().flat_map(|p| p.manifest.capabilities.iter().cloned()).collect();
        caps.sort();
        caps.dedup();
        caps
    }

    /// Invoca una capability sobre el plugin indicado. `args` se entrega
    /// tal cual al plugin (bytes opacos — la app y el plugin acuerdan el
    /// schema). El retorno colapsa el `_invoke` exit code y la
    /// `PluginAction` que el plugin haya emitido.
    pub fn invoke(
        &self,
        id: PluginId,
        capability: &str,
        args: &[u8],
    ) -> Result<PluginAction, PluginError> {
        let plugin = self.plugins.get(id.0 as usize).ok_or(PluginError::UnknownPlugin(id))?;
        let ctx = InvokeCtx { pending: RefCell::new(PluginAction::None) };
        let mut store = Store::new(&self.engine, ctx);
        let linker = build_linker(&self.engine, &plugin.manifest.permissions)?;

        // wasmi 1.0: `instantiate_and_start` corre la `(start)` section
        // si la hay; nuestros plugins no la usan — su entrada es
        // `_invoke`, llamada explícitamente más abajo.
        let instance = linker
            .instantiate_and_start(&mut store, &plugin.module)
            .map_err(|e| PluginError::Instantiate(e.to_string()))?;

        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| PluginError::MissingEntry("plugin sin export `memory`".into()))?;

        // Escribimos cap + args al inicio de la memoria del plugin. v0
        // del ABI: layout fijo, no negociado. Si el plugin necesita más
        // espacio se va a cualquier offset por encima — su asunto.
        let cap_bytes = capability.as_bytes();
        write_memory(&mut store, memory, 0, cap_bytes)?;
        let args_off = cap_bytes.len();
        write_memory(&mut store, memory, args_off, args)?;

        let func = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&store, "_invoke")
            .map_err(|e| PluginError::MissingEntry(e.to_string()))?;

        let _exit = func
            .call(
                &mut store,
                (0, cap_bytes.len() as i32, args_off as i32, args.len() as i32),
            )
            .map_err(|e| PluginError::Trap(e.to_string()))?;

        let action = store.data().pending.borrow().clone();
        Ok(action)
    }
}

// =====================================================================
// Host imports — gateados por Permissions
// =====================================================================

fn build_linker(
    engine: &Engine,
    perms: &Permissions,
) -> Result<Linker<InvokeCtx>, PluginError> {
    let mut linker = Linker::<InvokeCtx>::new(engine);

    // log — siempre disponible. Aún plugins sin permisos pueden trazar.
    linker
        .func_wrap("plugin", "log", |caller: Caller<'_, InvokeCtx>, ptr: i32, len: i32| {
            if let Some(s) = read_utf8(&caller, ptr, len) {
                info!("[plugin] {s}");
            }
        })
        .map_err(|e| PluginError::Instantiate(e.to_string()))?;

    // set_status — siempre disponible. No toca recursos del sistema.
    linker
        .func_wrap("plugin", "set_status", |caller: Caller<'_, InvokeCtx>, ptr: i32, len: i32| {
            if let Some(s) = read_utf8(&caller, ptr, len) {
                *caller.data().pending.borrow_mut() = PluginAction::SetStatus(s);
            }
        })
        .map_err(|e| PluginError::Instantiate(e.to_string()))?;

    // open_at — requiere filesystem >= read-only. Si el permiso falta NO
    // enlazamos el import: el plugin trap-eará al invocarlo, que es la
    // semántica correcta para un sandbox.
    if matches!(perms.filesystem, FsPolicy::ReadOnly | FsPolicy::ReadWrite) {
        linker
            .func_wrap(
                "plugin",
                "open_at",
                |caller: Caller<'_, InvokeCtx>, ptr: i32, len: i32, line: i32, col: i32| {
                    if let Some(s) = read_utf8(&caller, ptr, len) {
                        *caller.data().pending.borrow_mut() = PluginAction::OpenAt {
                            path: PathBuf::from(s),
                            line: line.max(0) as u32,
                            col: col.max(0) as u32,
                        };
                    }
                },
            )
            .map_err(|e| PluginError::Instantiate(e.to_string()))?;
    } else {
        warn!(
            "plugin sin permiso filesystem — `plugin.open_at` no enlazado; \
             llamarlo trap-eará"
        );
    }

    Ok(linker)
}

// =====================================================================
// Helpers de memoria
// =====================================================================

fn read_utf8(caller: &Caller<'_, InvokeCtx>, ptr: i32, len: i32) -> Option<String> {
    let memory = caller.get_export("memory")?.into_memory()?;
    let bytes = read_memory(caller, memory, ptr, len)?;
    String::from_utf8(bytes).ok()
}

fn read_memory(
    caller: &Caller<'_, InvokeCtx>,
    memory: Memory,
    ptr: i32,
    len: i32,
) -> Option<Vec<u8>> {
    let ptr = ptr.max(0) as usize;
    let len = len.max(0) as usize;
    let data = memory.data(caller);
    if ptr.saturating_add(len) > data.len() {
        return None;
    }
    Some(data[ptr..ptr + len].to_vec())
}

fn write_memory(
    store: &mut Store<InvokeCtx>,
    memory: Memory,
    off: usize,
    bytes: &[u8],
) -> Result<(), PluginError> {
    memory
        .write(store, off, bytes)
        .map_err(|e| PluginError::Trap(format!("write_memory off={off} len={}: {e}", bytes.len())))
}
