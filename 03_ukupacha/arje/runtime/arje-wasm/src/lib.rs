//! Encarnación de Payload::Wasm vía wasmi.
//!
//! Cada Ente Wasm corre en un hilo dedicado (wasmi es síncrono) que se
//! comunica con el grafo vía un identificador propio. El thread::JoinHandle
//! se descarta — el ciclo de vida del Wasm se controla por su `entry`
//! function: cuando retorna, el Ente se considera disuelto.
//!
//! ## Host imports expuestos
//!   - `ente.log(ptr: i32, len: i32)`           imprime una string UTF-8
//!   - `ente.exit(code: i32)`                   solicita salida del Ente
//!
//! Más adelante: `ente.bus_call`, `ente.cap_invoke`, etc.

use arje_card::EntityCard;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};
use wasmi::{Caller, Engine, Linker, Memory, Module, Store};

/// Estado por instancia Wasm. Se accede tanto desde host imports (vía
/// `Caller::data()`) como desde el thread runner para estado de salida.
pub struct WasmEnte {
    pub id: ulid::Ulid,
    pub label: String,
    pub exit_code: Arc<AtomicI32>,
}

/// Encarna un payload Wasm en un hilo dedicado. Devuelve un identificador
/// no-PID que el grafo trata como Ente Virtual con cuerpo de cómputo.
pub fn incarnate_wasm(card: &EntityCard, module_bytes: Vec<u8>, entry: String) -> anyhow::Result<()> {
    let label = card.label.clone();
    let id = card.id;
    let exit_code = Arc::new(AtomicI32::new(0));
    let exit_code_handle = exit_code.clone();

    std::thread::Builder::new()
        .name(format!("wasm-{label}"))
        .spawn(move || {
            if let Err(e) = run_wasm(WasmEnte { id, label: label.clone(), exit_code: exit_code_handle.clone() }, &module_bytes, &entry) {
                error!(?e, %label, "Wasm ente terminó con error");
                exit_code_handle.store(-1, Ordering::Relaxed);
            }
        })?;
    Ok(())
}

fn run_wasm(ente: WasmEnte, module_bytes: &[u8], entry: &str) -> anyhow::Result<()> {
    let engine = Engine::default();
    let module = Module::new(&engine, module_bytes)
        .map_err(|e| anyhow::anyhow!("Wasm module compile: {e}"))?;
    let mut store = Store::new(&engine, ente);
    let mut linker = <Linker<WasmEnte>>::new(&engine);

    linker.func_wrap("ente", "log", |caller: Caller<'_, WasmEnte>, ptr: i32, len: i32| {
        host_log(caller, ptr, len);
    })?;

    linker.func_wrap("ente", "exit", |mut caller: Caller<'_, WasmEnte>, code: i32| {
        caller.data_mut().exit_code.store(code, Ordering::Relaxed);
    })?;

    let pre = linker.instantiate(&mut store, &module)
        .map_err(|e| anyhow::anyhow!("Wasm instantiate: {e}"))?;
    let instance = pre.start(&mut store)
        .map_err(|e| anyhow::anyhow!("Wasm start: {e}"))?;

    let func = instance.get_typed_func::<(), ()>(&store, entry)
        .map_err(|e| anyhow::anyhow!("Wasm get_func {entry}: {e}"))?;

    info!(label = %store.data().label, %entry, "Wasm ente ejecutando");
    func.call(&mut store, ()).map_err(|e| anyhow::anyhow!("Wasm call {entry}: {e}"))?;
    let code = store.data().exit_code.load(Ordering::Relaxed);
    info!(label = %store.data().label, code, "Wasm ente terminó");
    Ok(())
}

fn host_log(caller: Caller<'_, WasmEnte>, ptr: i32, len: i32) {
    let memory = match caller.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => {
            warn!("Wasm ente sin memoria exportada — log ignorado");
            return;
        }
    };
    let data = read_memory(&caller, memory, ptr, len);
    match std::str::from_utf8(&data) {
        Ok(s) => info!(label = %caller.data().label, "[wasm] {s}"),
        Err(_) => warn!(label = %caller.data().label, "Wasm log con bytes no UTF-8"),
    }
}

fn read_memory(caller: &Caller<'_, WasmEnte>, memory: Memory, ptr: i32, len: i32) -> Vec<u8> {
    let ptr = ptr.max(0) as usize;
    let len = len.max(0) as usize;
    let data = memory.data(caller);
    if ptr.saturating_add(len) > data.len() {
        return Vec::new();
    }
    data[ptr..ptr + len].to_vec()
}

/// Módulo WAT mínimo de demostración. Llama a `ente.log` con "hola fractal".
/// Compilado a binario Wasm en runtime con `wat`.
pub fn demo_module_bytes() -> anyhow::Result<Vec<u8>> {
    let wat = r#"
(module
  (import "ente" "log"  (func $log  (param i32 i32)))
  (import "ente" "exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "hola fractal desde wasm")
  (func (export "_start")
    (call $log (i32.const 0) (i32.const 23))
    (call $exit (i32.const 0))
  )
)
"#;
    Ok(wat::parse_str(wat)?)
}
