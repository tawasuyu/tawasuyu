//! Certifica la frontera física host-side: un host import gateado por permiso
//! sólo se enlaza si el bit está. Sin él, un guest que lo importe NO instancia.
//!
//! Usamos un módulo WAT mínimo (compilado en runtime, sin toolchain wasm32) que
//! importa `tawa.host_net_request` — la función gateada por `PERMISO_RED`.

use llimphi_wasm_runner::build_linker;
use wasmi::{CompilationMode, Config, Engine, Module, Store};

/// Módulo que importa la función gateada y exporta `memory`.
const WAT_IMPORTA_RED: &str = r#"
(module
  (import "tawa" "host_net_request" (func $req (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "ping") (result i32)
    (call $req (i32.const 0) (i32.const 0))))
"#;

fn intenta_instanciar(permisos: u32) -> Result<(), String> {
    let wasm = wat::parse_str(WAT_IMPORTA_RED).expect("WAT válido");
    let mut config = Config::default();
    config.compilation_mode(CompilationMode::Eager);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &wasm).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    let linker = build_linker(&engine, permisos)?;
    linker
        .instantiate_and_start(&mut store, &module)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[test]
fn con_permiso_red_instancia() {
    // Con PERMISO_RED concedido, el import se enlaza y el módulo instancia.
    assert!(intenta_instanciar(format::PERMISO_RED).is_ok());
}

#[test]
fn sin_permiso_red_no_instancia() {
    // Sin el bit, `host_net_request` no se enlaza: el import queda sin resolver
    // y la instanciación falla. Frontera física: no hay tabla que escalar.
    assert!(intenta_instanciar(0).is_err());
}
