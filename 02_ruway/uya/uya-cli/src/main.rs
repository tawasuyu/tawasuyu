// =============================================================================
//  uya-cli — nodo headless de la videollamada.
// -----------------------------------------------------------------------------
//  Arranca el transporte + la captura sintética y reporta los eventos por
//  consola. Sirve como segundo extremo de una llamada o como prueba de la
//  señalización sin necesidad de GPU/ventana.
//
//      UYA_NOMBRE   nombre → identidad determinista (default "cli")
//      UYA_ESCUCHAR multiaddr de escucha (default /ip4/0.0.0.0/tcp/0)
//      UYA_CONECTAR multiaddr(s) dialable(s) a conectar (coma-separado, con
//                   /p2p/<peerid>; lo imprime el otro nodo al arrancar)
// =============================================================================

use std::env;
use std::sync::Arc;
use std::time::Duration;

use uya_app::{hex_corto, iniciar_camara, Enlace, EventoUya};

fn main() {
    let nombre = env::var("UYA_NOMBRE").unwrap_or_else(|_| "cli".into());
    let bind = env::var("UYA_ESCUCHAR").unwrap_or_else(|_| "/ip4/0.0.0.0/tcp/0".into());

    let (enlace, rx) = Enlace::abrir(nombre.clone(), &bind)
        .unwrap_or_else(|e| panic!("uya-cli: no pude escuchar en {bind}: {e}"));
    let enlace = Arc::new(enlace);

    println!(
        "uya-cli: {nombre} [{}] dialable en\n  {}",
        hex_corto(&enlace.yo()),
        enlace.direccion_local()
    );

    if let Ok(pares) = env::var("UYA_CONECTAR") {
        for par in pares.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            println!("uya-cli: conectando a {par}");
            enlace.conectar(par);
        }
    }

    // Descubrimiento por sala: anunciarse y encontrar a los demás por nombre.
    if let Ok(sala) = env::var("UYA_SALA") {
        let bootstrap: Vec<String> = env::var("UYA_BOOTSTRAP")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        println!("uya-cli: uniéndome a la sala '{sala}' ({} bootstrap)", bootstrap.len());
        // Baliza LAN (zero-config) + DHT (para WAN/bootstrap): ambas alimentan la malla.
        uya_app::iniciar_baliza_lan(enlace.clone(), sala.clone());
        enlace.unir_sala(sala, bootstrap);
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
            Ok(EventoUya::Mensaje { nombre, texto, .. }) => {
                println!("  💬 {nombre}: {texto}");
            }
            Ok(EventoUya::Voz { id, hablando }) => {
                if hablando {
                    println!("  🔊 habla [{}]", hex_corto(&id));
                }
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
