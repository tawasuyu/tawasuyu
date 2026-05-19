//! `brahman-wit-info` — inspecciona un archivo WIT y lista sus worlds.
//!
//! Uso:
//! ```sh
//! cargo run -p brahman-card-wit --example brahman-wit-info -- shared_wit/protocol.wit
//! ```

use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("uso: brahman-wit-info <ruta.wit>");
            return ExitCode::from(2);
        }
    };

    let worlds = match brahman_card_wit::parse_wit_file(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("error parseando {path}: {e}");
            return ExitCode::from(1);
        }
    };

    if worlds.is_empty() {
        println!("(ningún world declarado)");
        return ExitCode::SUCCESS;
    }

    println!("{} world(s):", worlds.len());
    for w in &worlds {
        println!();
        println!("  package: {}", w.package);
        println!("  world:   {}", w.world);
        if !w.imports.is_empty() {
            println!("  imports: {}", w.imports.join(", "));
        }
        if !w.exports.is_empty() {
            println!("  exports: {}", w.exports.join(", "));
        }
    }
    ExitCode::SUCCESS
}
