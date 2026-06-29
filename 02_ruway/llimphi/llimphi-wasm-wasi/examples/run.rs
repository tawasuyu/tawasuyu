//! Corre un módulo WASI de consola y vuelca su salida.
//!
//! `cargo run -p llimphi-wasm-wasi --example run -- programa.wasm [args…]`

use llimphi_wasm_wasi::{detect_kind, run_console, WasmKind};

fn main() {
    let mut a = std::env::args().skip(1);
    let path = a.next().unwrap_or_else(|| {
        eprintln!("uso: run <programa.wasm> [args…]");
        std::process::exit(2);
    });
    let args: Vec<String> = a.collect();
    let wasm = std::fs::read(&path).expect("leer wasm");

    eprintln!("clase del módulo: {:?}", detect_kind(&wasm));
    if detect_kind(&wasm) != WasmKind::WasiConsole {
        eprintln!("(no es un módulo WASI de consola)");
    }

    let out = run_console(&wasm, &args, &[], &[]).expect("correr");
    print!("{}", out.stdout_text());
    eprint!("{}", out.stderr_text());
    std::process::exit(out.exit_code);
}
