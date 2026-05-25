//! `cargo run -p wawa-explorer-core --example dump -- <ruta.img>`
//!
//! Volcado textual de una imagen Wawa: header del superbloque, manifiesto
//! si existe, y listado de objetos con su tamaño. Smoke test legible.

use std::path::PathBuf;

use wawa_explorer_core::{short_hex, Disco};

fn main() {
    let ruta = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        eprintln!("uso: dump <ruta.img>");
        std::process::exit(2);
    });

    let disco = match Disco::abrir(&ruta) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error abriendo {}: {e}", ruta.display());
            std::process::exit(1);
        }
    };

    let sb = disco.superbloque();
    println!("\n  wawa-explorer · {}", ruta.display());
    println!("  imagen: {} bytes ({} sectores)", disco.bytes_imagen(), disco.bytes_imagen() / 512);
    println!(
        "  superbloque: version {} · cursor sector {} · raíz {} · manifiesto {}",
        sb.version,
        sb.cursor,
        sb.raiz.as_ref().map(short_hex).unwrap_or_else(|| "—".into()),
        sb.manifiesto.as_ref().map(short_hex).unwrap_or_else(|| "—".into())
    );
    println!("  objetos en el grafo: {}\n", disco.cantidad_objetos());

    match disco.manifiesto() {
        Ok(Some(m)) => {
            println!("  manifiesto (versión {}, {} apps):", m.version, m.apps.len());
            for app in &m.apps {
                println!(
                    "    · {:<20} bytecode {}  region {}x{}+{}+{}  techo {} KiB  estado {}",
                    app.nombre,
                    short_hex(&app.bytecode),
                    app.region_ancho,
                    app.region_alto,
                    app.region_x,
                    app.region_y,
                    app.techo_memoria / 1024,
                    app.estado.as_ref().map(short_hex).unwrap_or_else(|| "—".into()),
                );
            }
        }
        Ok(None) => println!("  (sin manifiesto)"),
        Err(e) => println!("  error leyendo manifiesto: {e}"),
    }

    println!("\n  objetos:");
    let mut hashes: Vec<_> = disco.hashes().collect();
    hashes.sort();
    for h in hashes {
        let o = disco.objeto(h).unwrap();
        println!("    {}  {} bytes · {} hijos", short_hex(h), o.datos.len(), o.hijos.len());
    }
    println!();
}
