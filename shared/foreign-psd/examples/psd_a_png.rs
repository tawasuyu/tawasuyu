//! `psd_a_png` — demo CLI: importa un `.psd`, compone su lienzo con
//! `tullpu-render` y escribe el resultado como `.png`.
//!
//! ```text
//! cargo run -p foreign-psd --example psd_a_png --release -- <entrada.psd> <salida.png>
//! ```
//!
//! Imprime un informe del import (capas, blend modes degradados) por stdout.

use std::env;
use std::process::ExitCode;

use foreign_psd::importar_psd;
use tullpu_render::{componer, AlmacenEnMemoria, FuenteBuffers};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("uso: {} <entrada.psd> <salida.png>", args[0]);
        return ExitCode::from(2);
    }
    let entrada = &args[1];
    let salida = &args[2];

    let bytes = match std::fs::read(entrada) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("no pude leer {entrada}: {e}");
            return ExitCode::from(1);
        }
    };

    let imp = match importar_psd(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("import falló: {e}");
            return ExitCode::from(1);
        }
    };

    println!("📥 PSD importado:");
    println!("    lienzo: {}×{}", imp.lienzo.width, imp.lienzo.height);
    println!("    capas:  {}", imp.informe.capas_importadas);
    println!("    buffers únicos: {}", imp.buffers.len());
    if imp.informe.caidas_a_normal.is_empty() {
        println!("    blend modes: todos soportados ✓");
    } else {
        println!("    blend modes degradados a Normal:");
        for (nombre, blend) in &imp.informe.caidas_a_normal {
            println!("      · '{}' ({})", nombre, blend);
        }
    }

    // Volcamos los buffers a un almacén en memoria y componemos.
    let mut almacen = AlmacenEnMemoria::nuevo();
    for (hash, bytes) in imp.buffers {
        almacen.buffers.insert(hash, bytes);
    }
    // Sanity: cada capa apuntada está en el almacén.
    for capa in &imp.lienzo.capas {
        if almacen.obtener(capa.contenido).is_none() {
            eprintln!("error interno: buffer faltante para capa '{}'", capa.nombre);
            return ExitCode::from(1);
        }
    }

    let imagen = match componer(&imp.lienzo, &almacen) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("composición falló: {e}");
            return ExitCode::from(1);
        }
    };

    if let Err(e) = imagen.save(salida) {
        eprintln!("no pude escribir {salida}: {e}");
        return ExitCode::from(1);
    }
    println!("✅ escrito {} ({}×{})", salida, imagen.width(), imagen.height());
    ExitCode::SUCCESS
}
