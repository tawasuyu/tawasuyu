//! La integración con `wasmi`: cargar un `.wasm`, gatear sus capacidades a nivel
//! de importación (port de `enlazar_capacidades` del kernel wawa) y despacharle
//! `mirada_tile` / `mirada_on_event` con fuel acotado.

use wasmi::{
    CompilationMode, Config, Engine, Error, Extern, Linker, Memory, Module, Store, TypedFunc,
};

use mirada_protocol::{BodyEvent, BrainCommand, Decorations, Rect, TileInput, WindowEffects, WindowId};

use crate::caps::{
    cap_for_import, cap_name, caps_list, CapsPlugin, CAP_ACTIONS, CAP_DECOR, CAP_EFFECTS, CAP_KEYS,
    CAP_SPAWN, CAP_WINDOW_CONTROL,
};
use crate::manifest::{PluginKind, ResolvedManifest};
use crate::trust::{authorize, TrustSet};

/// Presupuesto de combustible por llamada. Un `tile()`/`on_event()` desbocado
/// se queda sin fuel y trampa en vez de congelar el escritorio.
const FUEL: u64 = 200_000_000;

/// El contexto host que viaja en el `Store`: acumula lo que el plugin emite
/// durante una llamada. Dos canales: comandos directos al Cuerpo (`out`) y
/// **acciones de escritorio** (`actions`, forma textual de `DesktopAction`) que
/// el [`Conductor`](crate::Conductor) aplica al `Desktop` autoritativo — así un
/// reactor maneja ventanas sin poder romper la consistencia del estado.
#[derive(Default)]
pub struct HostCtx {
    pub out: Vec<BrainCommand>,
    pub actions: Vec<String>,
}

/// Un plugin cargado e instanciado, con sus puntos de ABI cacheados.
pub struct LoadedPlugin {
    store: Store<HostCtx>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    tile: Option<TypedFunc<(u32, u32), u64>>,
    on_event: Option<TypedFunc<(u32, u32), ()>>,
    /// Export opcional `mirada_configure`: presente si el plugin acepta config.
    configure: Option<TypedFunc<(u32, u32), ()>>,
    pub kind: PluginKind,
    pub priority: i32,
    pub name: String,
}

impl LoadedPlugin {
    /// Carga el `.wasm` del manifest: primero **autoriza** las capacidades
    /// peligrosas contra el anillo de confianza (firma sobre `blake3(wasm) ‖
    /// caps`), luego verifica las importaciones (fail-closed) e instancia.
    pub fn load(m: &ResolvedManifest, trust: &TrustSet) -> Result<LoadedPlugin, String> {
        let bytes = std::fs::read(&m.wasm_path)
            .map_err(|e| format!("no se pudo leer {}: {e}", m.wasm_path.display()))?;
        authorize(&bytes, m.granted, m.grant.as_ref(), trust)?;
        let mut p = Self::load_bytes(&bytes, m.kind, m.granted, m.priority, &m.name)?;
        if !m.config.is_empty() {
            p.configure(&m.config)?;
        }
        Ok(p)
    }

    /// Como [`load`](LoadedPlugin::load) pero desde bytes ya en memoria y **sin**
    /// verificación de firma — el camino de los tests (`include_bytes!`) y del
    /// despliegue empotrado, donde la confianza se establece por otra vía.
    pub fn load_bytes(
        bytes: &[u8],
        kind: PluginKind,
        granted: CapsPlugin,
        priority: i32,
        name: &str,
    ) -> Result<LoadedPlugin, String> {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.compilation_mode(CompilationMode::Eager);
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes)
            .map_err(|e| format!("módulo {name} inválido: {e}"))?;

        // --- Fail-closed: cada importación debe estar gateada y concedida. ---
        for imp in module.imports() {
            if imp.module() != "mirada_host" {
                return Err(format!(
                    "plugin {} importa {}::{} fuera del namespace mirada_host",
                    name,
                    imp.module(),
                    imp.name()
                ));
            }
            let cap = cap_for_import(imp.name()).ok_or_else(|| {
                format!("plugin {} importa mirada_host::{} desconocida", name, imp.name())
            })?;
            if cap != 0 && granted & cap == 0 {
                return Err(format!(
                    "plugin {} pide capacidad `{}` (import {}); el manifest concede {}",
                    name,
                    cap_name(cap),
                    imp.name(),
                    caps_list(granted)
                ));
            }
        }

        let mut store = Store::new(&engine, HostCtx::default());
        store.set_fuel(FUEL).map_err(|e| format!("fuel inicial: {e}"))?;

        let mut linker: Linker<HostCtx> = Linker::new(&engine);
        register_host_fns(&mut linker, granted).map_err(|e| format!("enlace: {e}"))?;

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| format!("instanciación de {} falló: {e}", name))?;

        let alloc = instance
            .get_typed_func::<u32, u32>(&store, "alloc")
            .map_err(|_| format!("plugin {} no exporta `alloc`", name))?;
        let memory = instance
            .get_memory(&store, "memory")
            .ok_or_else(|| format!("plugin {} no exporta `memory`", name))?;

        let (tile, on_event) = match kind {
            PluginKind::Layout => {
                let t = instance
                    .get_typed_func::<(u32, u32), u64>(&store, "mirada_tile")
                    .map_err(|_| format!("plugin layout {} sin `mirada_tile`", name))?;
                (Some(t), None)
            }
            PluginKind::Reactor => {
                let o = instance
                    .get_typed_func::<(u32, u32), ()>(&store, "mirada_on_event")
                    .map_err(|_| format!("plugin reactor {} sin `mirada_on_event`", name))?;
                (None, Some(o))
            }
        };

        // `mirada_configure` es opcional: presente si el plugin acepta config.
        let configure = instance
            .get_typed_func::<(u32, u32), ()>(&store, "mirada_configure")
            .ok();

        Ok(LoadedPlugin {
            store,
            memory,
            alloc,
            tile,
            on_event,
            configure,
            kind,
            priority,
            name: name.to_string(),
        })
    }

    /// Reserva en el guest y escribe `bytes`, devolviendo el puntero.
    fn write_input(&mut self, bytes: &[u8]) -> Result<u32, String> {
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let ptr = self
            .alloc
            .call(&mut self.store, bytes.len() as u32)
            .map_err(|e| format!("alloc del plugin {}: {e}", self.name))?;
        self.memory
            .write(&mut self.store, ptr as usize, bytes)
            .map_err(|e| format!("escritura en {}: {e}", self.name))?;
        Ok(ptr)
    }

    /// Empuja la cadena de config al plugin (`mirada_configure`), una vez, antes
    /// de cualquier `tile`/`on_event`. Falla si el plugin no exporta el punto de
    /// entrada (el manifest declaró `config:` para un plugin que no la acepta).
    pub fn configure(&mut self, config: &str) -> Result<(), String> {
        let configure = self
            .configure
            .ok_or_else(|| format!("plugin {} no acepta config (sin `mirada_configure`)", self.name))?;
        let bytes = config.as_bytes();
        let ptr = self.write_input(bytes)?;
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        configure
            .call(&mut self.store, (ptr, bytes.len() as u32))
            .map_err(|e| format!("mirada_configure de {}: {e}", self.name))?;
        Ok(())
    }

    /// Despacha `mirada_tile` y devuelve la geometría que el plugin decidió.
    pub fn call_tile(&mut self, input: &TileInput) -> Result<Vec<(WindowId, Rect)>, String> {
        let tile = self
            .tile
            .ok_or_else(|| format!("{} no es plugin de layout", self.name))?;
        let bytes = postcard::to_stdvec(input).map_err(|e| e.to_string())?;
        let ptr = self.write_input(&bytes)?;
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let packed = tile
            .call(&mut self.store, (ptr, bytes.len() as u32))
            .map_err(|e| format!("mirada_tile de {}: {e}", self.name))?;
        if packed == 0 {
            return Ok(Vec::new());
        }
        let out_ptr = (packed >> 32) as usize;
        let out_len = (packed & 0xffff_ffff) as usize;
        let mut buf = vec![0u8; out_len];
        self.memory
            .read(&self.store, out_ptr, &mut buf)
            .map_err(|e| format!("lectura de salida de {}: {e}", self.name))?;
        postcard::from_bytes(&buf).map_err(|e| format!("salida postcard de {}: {e}", self.name))
    }

    /// Despacha `mirada_on_event` y devuelve los comandos que el plugin emitió.
    pub fn call_on_event(&mut self, event: &BodyEvent) -> Result<Vec<BrainCommand>, String> {
        let on_event = self
            .on_event
            .ok_or_else(|| format!("{} no es reactor", self.name))?;
        let bytes = postcard::to_stdvec(event).map_err(|e| e.to_string())?;
        let ptr = self.write_input(&bytes)?;
        self.store.data_mut().out.clear();
        self.store.data_mut().actions.clear();
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        on_event
            .call(&mut self.store, (ptr, bytes.len() as u32))
            .map_err(|e| format!("mirada_on_event de {}: {e}", self.name))?;
        Ok(std::mem::take(&mut self.store.data_mut().out))
    }

    /// Las **acciones de escritorio** (forma textual de `DesktopAction`) que el
    /// reactor pidió en la última llamada a [`call_on_event`](Self::call_on_event).
    /// El [`Conductor`](crate::Conductor) las parsea y aplica al `Desktop`.
    pub fn take_actions(&mut self) -> Vec<String> {
        std::mem::take(&mut self.store.data_mut().actions)
    }
}

/// Registra las funciones host gateadas por capacidad. `host_log` siempre; el
/// resto sólo si su bit está concedido — el espejo host-side de
/// `enlazar_capacidades` (kernel wawa). Lo que no se registre queda físicamente
/// fuera del alcance del módulo.
fn register_host_fns(linker: &mut Linker<HostCtx>, granted: CapsPlugin) -> Result<(), Error> {
    use wasmi::Caller;

    // --- host_log (sin capacidad) ---
    linker.func_wrap(
        "mirada_host",
        "host_log",
        |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
            let bytes = read_guest(&mut caller, ptr, len)?;
            if let Ok(s) = core::str::from_utf8(&bytes) {
                eprintln!("[plugin] {s}");
            }
            Ok(())
        },
    )?;

    if granted & CAP_SPAWN != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_spawn",
            |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
                let bytes = read_guest(&mut caller, ptr, len)?;
                let cmd = String::from_utf8_lossy(&bytes).into_owned();
                caller.data_mut().out.push(BrainCommand::Spawn(cmd));
                Ok(())
            },
        )?;
    }

    if granted & CAP_WINDOW_CONTROL != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_close",
            |mut caller: Caller<'_, HostCtx>, id: u64| {
                caller.data_mut().out.push(BrainCommand::Close(id));
            },
        )?;
        linker.func_wrap(
            "mirada_host",
            "host_emit_kill",
            |mut caller: Caller<'_, HostCtx>, id: u64| {
                caller.data_mut().out.push(BrainCommand::Kill(id));
            },
        )?;
    }

    if granted & CAP_KEYS != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_keys",
            |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
                let bytes = read_guest(&mut caller, ptr, len)?;
                let keys: Vec<String> = postcard::from_bytes(&bytes)
                    .map_err(|_| Error::new("host_emit_keys: postcard inválido"))?;
                caller.data_mut().out.push(BrainCommand::GrabKeys(keys));
                Ok(())
            },
        )?;
    }

    if granted & CAP_DECOR != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_decor",
            |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
                let bytes = read_guest(&mut caller, ptr, len)?;
                let d: Decorations = postcard::from_bytes(&bytes)
                    .map_err(|_| Error::new("host_emit_decor: postcard inválido"))?;
                caller.data_mut().out.push(BrainCommand::SetDecorations(d));
                Ok(())
            },
        )?;
        linker.func_wrap(
            "mirada_host",
            "host_emit_cursor",
            |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
                let bytes = read_guest(&mut caller, ptr, len)?;
                let name = String::from_utf8_lossy(&bytes).into_owned();
                caller.data_mut().out.push(BrainCommand::SetCursor(name));
                Ok(())
            },
        )?;
    }

    if granted & CAP_ACTIONS != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_action",
            |mut caller: Caller<'_, HostCtx>, ptr: u32, len: u32| -> Result<(), Error> {
                let bytes = read_guest(&mut caller, ptr, len)?;
                let action = String::from_utf8_lossy(&bytes).into_owned();
                caller.data_mut().actions.push(action);
                Ok(())
            },
        )?;
    }

    if granted & CAP_EFFECTS != 0 {
        linker.func_wrap(
            "mirada_host",
            "host_emit_effects",
            |mut caller: Caller<'_, HostCtx>, id: u64, opacity: u32, flags: u32| {
                let effects = WindowEffects {
                    opacity: opacity.min(255) as u8,
                    shadow: flags & 1 != 0,
                };
                caller.data_mut().out.push(BrainCommand::SetEffects(vec![(id, effects)]));
            },
        )?;
    }

    Ok(())
}

/// Copia `[ptr, ptr+len)` de la memoria lineal del guest a un `Vec` propio,
/// validando límites. Copiar antes de tocar `data_mut` evita aliasar el `Store`
/// (la disciplina de reentrancy del kernel wawa).
fn read_guest(
    caller: &mut wasmi::Caller<'_, HostCtx>,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, Error> {
    let mem = caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| Error::new("el plugin no exporta su memoria lineal"))?;
    let data = mem.data(&caller);
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .filter(|e| *e <= data.len())
        .ok_or_else(|| Error::new("ptr/len del plugin desbordó la memoria lineal"))?;
    Ok(data[start..end].to_vec())
}
