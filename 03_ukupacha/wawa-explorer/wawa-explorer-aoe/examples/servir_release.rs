//! `servir_release` — la mitad "Akasha + servir" del lazo Rust→wawa en vivo.
//!
//! Toma el directorio que produce `agora-cli wawa publicar` (un `<hash>.obj`
//! por objeto del grafo + `anuncio.bin`) y, sobre la interfaz Ethernet dada:
//!
//!   1. difunde `AnunciarCanal{...}` —la recomendación firmada de release—,
//!   2. atiende los `SolicitarObjeto` que la wawa receptora dispara para
//!      descargar el canal, el manifiesto y los bytecodes que le falten,
//!   3. repite el anuncio cada pocos segundos durante la sesión, porque el
//!      broadcast L2 no es confiable y la wawa puede arrancar tarde.
//!
//! Uso (requiere CAP_NET_RAW o root para el raw socket):
//!
//!   sudo -E cargo run -p wawa-explorer-aoe --example servir_release -- \
//!       <iface> <dir_release> [segundos]
//!
//! `segundos` por defecto 120. Cortá con Ctrl-C cuando la wawa haya absorbido
//! el release (su baliza serial lo confirma) y el operador haya aceptado en
//! `mudanza`.
//!
//! LÍMITE: objetos > 1486 B (MAX_PAYLOAD_AKASHA) no caben en un frame y se
//! OMITEN — `publicar` ya avisa cuáles. El chunking es el hito siguiente.

use std::collections::HashMap;
use std::time::Duration;

use wawa_explorer_aoe::ClienteAoE;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "uso: servir_release <iface> <dir_release> [segundos]\n\
             ej : sudo -E cargo run -p wawa-explorer-aoe --example servir_release -- eth0 ./release"
        );
        std::process::exit(2);
    }
    let iface = &args[1];
    let dir = std::path::Path::new(&args[2]);
    let total_seg: u64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(120);

    let objetos = match cargar_objetos(dir) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("servir_release: no pude cargar objetos de {}: {e}", dir.display());
            std::process::exit(1);
        }
    };
    let anuncio = match cargar_anuncio(dir) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("servir_release: anuncio.bin inválido: {e}");
            std::process::exit(1);
        }
    };

    let cliente = match ClienteAoE::nuevo(iface) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("servir_release: no pude abrir {iface}: {e}\n(¿CAP_NET_RAW / root?)");
            std::process::exit(1);
        }
    };

    println!(
        "servir_release: {} objetos en {} sobre {iface} (MAC {})",
        objetos.len(),
        dir.display(),
        mac_hex(&cliente.mac_local()),
    );
    println!("  canal {} · raiz {}", hex(&anuncio.canal), hex(&anuncio.raiz));
    println!("  anunciando + sirviendo {total_seg}s (Ctrl-C para cortar)");

    // Lazo: anunciar, servir una ventana corta, repetir. Así el anuncio se
    // re-emite mientras seguimos atendiendo pulls — robusto ante pérdida L2 y
    // ante una wawa que arranca después de nosotros.
    let inicio = std::time::Instant::now();
    let total = Duration::from_secs(total_seg);
    let mut servidos = 0u64;
    while inicio.elapsed() < total {
        if let Err(e) = cliente.anunciar_canal(
            anuncio.canal,
            anuncio.raiz,
            anuncio.autor,
            anuncio.timestamp,
            anuncio.firma,
        ) {
            eprintln!("servir_release: fallo al anunciar: {e}");
        }
        let restante = total.saturating_sub(inicio.elapsed());
        match cliente.servir(&objetos, restante.min(Duration::from_secs(5))) {
            Ok(stats) => {
                servidos += stats.servidos;
                if stats.servidos > 0 || stats.omitidos_grandes > 0 {
                    println!(
                        "  +{} servidos, {} ignorados, {} OMITIDOS por tamaño (acum servidos={})",
                        stats.servidos, stats.ignorados, stats.omitidos_grandes, servidos
                    );
                }
            }
            Err(e) => {
                eprintln!("servir_release: error sirviendo: {e}");
                break;
            }
        }
    }
    println!("servir_release: fin. Total objetos servidos: {servidos}");
}

/// El anuncio firmado, des-serializado del layout fijo de 168 B de `anuncio.bin`.
struct Anuncio {
    canal: [u8; 32],
    raiz: [u8; 32],
    autor: [u8; 32],
    timestamp: u64,
    firma: [u8; 64],
}

fn cargar_anuncio(dir: &std::path::Path) -> Result<Anuncio, String> {
    let bytes = std::fs::read(dir.join("anuncio.bin")).map_err(|e| e.to_string())?;
    if bytes.len() != 168 {
        return Err(format!("esperaba 168 B, hallé {}", bytes.len()));
    }
    let mut canal = [0u8; 32];
    let mut raiz = [0u8; 32];
    let mut autor = [0u8; 32];
    let mut firma = [0u8; 64];
    canal.copy_from_slice(&bytes[0..32]);
    raiz.copy_from_slice(&bytes[32..64]);
    autor.copy_from_slice(&bytes[64..96]);
    let mut ts = [0u8; 8];
    ts.copy_from_slice(&bytes[96..104]);
    firma.copy_from_slice(&bytes[104..168]);
    Ok(Anuncio {
        canal,
        raiz,
        autor,
        timestamp: u64::from_le_bytes(ts),
        firma,
    })
}

/// Carga todos los `<hash>.obj` del directorio en un mapa id→payload. El nombre
/// del archivo (64 hex) ES el hash; lo verificamos re-hasheando el contenido.
fn cargar_objetos(dir: &std::path::Path) -> Result<HashMap<[u8; 32], Vec<u8>>, String> {
    let mut mapa = HashMap::new();
    for entrada in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entrada = entrada.map_err(|e| e.to_string())?;
        let ruta = entrada.path();
        let Some(nombre) = ruta.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(hex_hash) = nombre.strip_suffix(".obj") else {
            continue;
        };
        let id = match parse_hex32(hex_hash) {
            Some(h) => h,
            None => {
                eprintln!("servir_release: nombre no-hash ignorado: {nombre}");
                continue;
            }
        };
        let datos = std::fs::read(&ruta).map_err(|e| e.to_string())?;
        let calculado = *blake3::hash(&datos).as_bytes();
        if calculado != id {
            return Err(format!(
                "objeto {nombre}: el contenido no hashea a su nombre (¿corrupto?)"
            ));
        }
        mapa.insert(id, datos);
    }
    if mapa.is_empty() {
        return Err("no hallé ningún <hash>.obj".to_string());
    }
    Ok(mapa)
}

fn parse_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn mac_hex(m: &[u8; 6]) -> String {
    m.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":")
}
