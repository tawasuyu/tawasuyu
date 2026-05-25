//! Smoke test del cliente AoE — pide un objeto por hash a peers Wawa de la red local.
//!
//! `sudo cargo run -p wawa-explorer-aoe --example solicitar -- <iface> <hash-hex>`
//!
//! Requiere CAP_NET_RAW (o root). El hash es hex de 64 caracteres (32 bytes).
//! Difunde `SolicitarObjeto(id)` por la interfaz y espera 3s una respuesta
//! `ProveedorObjeto(id, datos)` con hash coincidente.

use std::time::Duration;

use wawa_explorer_aoe::ClienteAoE;

fn main() {
    let mut args = std::env::args().skip(1);
    let iface = args.next().unwrap_or_else(|| {
        eprintln!("uso: solicitar <iface> <hash-hex>");
        std::process::exit(2);
    });
    let hash_hex = args.next().unwrap_or_else(|| {
        eprintln!("uso: solicitar <iface> <hash-hex>");
        std::process::exit(2);
    });

    let mut id = [0u8; 32];
    if hash_hex.len() != 64 {
        eprintln!("hash debe ser de 64 caracteres hex; recibí {}", hash_hex.len());
        std::process::exit(2);
    }
    for i in 0..32 {
        match u8::from_str_radix(&hash_hex[i * 2..i * 2 + 2], 16) {
            Ok(b) => id[i] = b,
            Err(_) => {
                eprintln!("hash no es hex válido en posición {}", i * 2);
                std::process::exit(2);
            }
        }
    }

    let cliente = match ClienteAoE::nuevo(&iface) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error abriendo socket en '{iface}': {e}");
            eprintln!("¿permisos? requiere CAP_NET_RAW. Probá:");
            eprintln!("  sudo setcap cap_net_raw=eip $(which solicitar)");
            std::process::exit(1);
        }
    };

    let mac = cliente.mac_local();
    println!(
        "  cliente AoE en {iface} · MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} · ifindex {}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], cliente.ifindex()
    );
    println!("  solicitando {hash_hex}  (timeout 3s)");

    match cliente.solicitar(id, Duration::from_secs(3)) {
        Ok(Some(datos)) => {
            println!("  respondió un peer con {} bytes (verificado blake3 == id)", datos.len());
            // Mostrar primeros 64 bytes en hex.
            for (i, b) in datos.iter().take(64).enumerate() {
                if i % 16 == 0 {
                    print!("\n    {i:04x}  ");
                }
                print!("{b:02x} ");
            }
            println!();
        }
        Ok(None) => {
            println!("  timeout — ningún peer respondió con el objeto");
        }
        Err(e) => {
            eprintln!("  error: {e}");
            std::process::exit(1);
        }
    }
}
