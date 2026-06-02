// =============================================================================
//  uya-cli — nodo headless de la videollamada.
// -----------------------------------------------------------------------------
//  Arranca el transporte + la captura sintética y reporta los eventos por
//  consola. Sirve como segundo extremo de una llamada o como prueba de la
//  señalización sin necesidad de GPU/ventana.
//
//      UYA_NOMBRE   nombre → identidad determinista (default "cli")
//      UYA_ESCUCHAR dirección de escucha (default 127.0.0.1:7801)
//      UYA_CONECTAR par(es) a conectar al arrancar (coma-separado, opcional)
// =============================================================================

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use uya_app::{hex_corto, iniciar_camara, Enlace, EventoUya};

fn main() {
    let nombre = env::var("UYA_NOMBRE").unwrap_or_else(|_| "cli".into());
    let bind: SocketAddr = env::var("UYA_ESCUCHAR")
        .unwrap_or_else(|_| "127.0.0.1:7801".into())
        .parse()
        .expect("UYA_ESCUCHAR debe ser una dirección válida (ip:puerto)");

    let (enlace, rx) = Enlace::abrir(nombre.clone(), Some(bind))
        .unwrap_or_else(|e| panic!("uya-cli: no pude escuchar en {bind}: {e}"));
    let enlace = Arc::new(enlace);

    if let Some(dir) = enlace.direccion_local() {
        println!(
            "uya-cli: {nombre} [{}] escuchando en {dir}",
            hex_corto(&enlace.yo())
        );
    }

    if let Ok(pares) = env::var("UYA_CONECTAR") {
        for par in pares.split(',').filter(|s| !s.trim().is_empty()) {
            match par.trim().parse::<SocketAddr>() {
                Ok(addr) => {
                    println!("uya-cli: conectando a {addr}");
                    enlace.conectar(addr);
                }
                Err(e) => eprintln!("uya-cli: dirección inválida '{par}': {e}"),
            }
        }
    }

    iniciar_camara(enlace.clone(), 192, 144, 12.0);
    // Audio: reproducción sobre la mezcla remota + captura de micrófono.
    let mezcla = enlace.mezcla();
    let _sink = uya_app::iniciar_reproduccion(mezcla.clone());
    uya_app::iniciar_microfono(enlace.clone());

    // Reporte: agregamos los cuadros por participante para no inundar la salida.
    let mut cuadros = std::collections::HashMap::<[u8; 32], u64>::new();
    let mut ultimo_reporte = std::time::Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(EventoUya::Entra { id, nombre }) => {
                println!("  + entra {nombre} [{}]", hex_corto(&id))
            }
            Ok(EventoUya::Sale { id }) => println!("  - sale [{}]", hex_corto(&id)),
            Ok(EventoUya::Estado {
                id,
                camara,
                microfono,
            }) => println!(
                "  ~ estado [{}] cam={camara} mic={microfono}",
                hex_corto(&id)
            ),
            Ok(EventoUya::Cuadro { id, .. }) => {
                *cuadros.entry(id).or_insert(0) += 1;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if ultimo_reporte.elapsed() >= Duration::from_secs(2) {
            for (id, n) in &cuadros {
                println!("  · [{}] {n} cuadros", hex_corto(id));
            }
            println!("  ♪ {} muestras de audio recibidas", mezcla.lock().recibidas());
            ultimo_reporte = std::time::Instant::now();
        }
    }
}
