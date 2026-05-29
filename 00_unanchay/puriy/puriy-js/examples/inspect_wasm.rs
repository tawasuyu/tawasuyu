//! Inspecciona el .wasm de QuickJS: lista imports y exports.
//!
//! `cargo run -p puriy-js --example inspect_wasm`

use wasmi::{Engine, Module};

fn main() {
    let bytes = include_bytes!("../runtime/qjs-wasi-reactor.wasm");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes[..]).expect("módulo válido");

    println!("=== IMPORTS ===");
    for imp in module.imports() {
        let ty = match imp.ty() {
            wasmi::ExternType::Func(f) => format!("func{:?}", f),
            wasmi::ExternType::Global(g) => format!("global{:?}", g),
            wasmi::ExternType::Memory(m) => format!("memory{:?}", m),
            wasmi::ExternType::Table(t) => format!("table{:?}", t),
        };
        println!("  {}::{} = {}", imp.module(), imp.name(), ty);
    }

    println!("\n=== EXPORTS ===");
    for exp in module.exports() {
        let ty = match exp.ty() {
            wasmi::ExternType::Func(f) => format!("func{:?}", f),
            wasmi::ExternType::Global(g) => format!("global{:?}", g),
            wasmi::ExternType::Memory(m) => format!("memory{:?}", m),
            wasmi::ExternType::Table(t) => format!("table{:?}", t),
        };
        println!("  {} = {}", exp.name(), ty);
    }
}
