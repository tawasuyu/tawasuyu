// =============================================================================
//  ayni :: ayni-cli — chat soberano en la terminal
// -----------------------------------------------------------------------------
//  Dos terminales, dos identidades, una LAN. Escribís una línea: se firma con
//  tu clave Ed25519, viaja como un nodo del DAG por TCP, y aparece del otro
//  lado verificada. Los grafos convergen sin servidor. Es el MVP de P1 — feo,
//  pero el lazo vivo entero funciona y se palpa.
//
//      ayni --nombre Alicia --escuchar 127.0.0.1:7700
//      ayni --nombre Beto   --escuchar 127.0.0.1:7701 --conectar 127.0.0.1:7700
// =============================================================================

use std::io::BufRead;
use std::sync::{Arc, Mutex};

use ayni_core::{Carga, Conversacion, MensajeNodo};
use ayni_crypto::{verificar_firma, Identidad};
use ayni_sync::{EnlaceTcp, EventoRed, Fusionador, Sobre, Transporte};

use clap::Parser;

/// Chat soberano P1: DAG firmado sobre TCP LAN.
#[derive(Parser)]
#[command(name = "ayni", about = "Chat persona-a-persona soberano (P1: TCP LAN)")]
struct Args {
    /// Tu nombre. Deriva una identidad Ed25519 determinista (demo): mismo
    /// nombre, misma identidad. En producción la identidad viene del keystore
    /// cifrado de agora (ver ayni-crypto::Identidad::cargar_de_keystore).
    #[arg(long)]
    nombre: String,

    /// Dirección donde escuchar conexiones entrantes.
    #[arg(long, default_value = "127.0.0.1:7700")]
    escuchar: String,

    /// Dirección de un peer al que conectarse al arrancar (opcional).
    #[arg(long)]
    conectar: Option<String>,
}

/// El estado compartido entre el hilo de red y el de teclado.
struct Estado {
    conv: Conversacion,
    fus: Fusionador,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Identidad determinista desde el nombre (demo). BLAKE3 reparte el nombre en
    // los 32 bytes de semilla — dos nombres distintos, dos identidades distintas.
    let seed = *blake3::hash(args.nombre.as_bytes()).as_bytes();
    let yo = Identidad::desde_semilla(seed, args.nombre.clone());

    let estado = Arc::new(Mutex::new(Estado {
        conv: Conversacion::nueva(),
        fus: Fusionador::nuevo(),
    }));

    let (enlace, rx) = EnlaceTcp::escuchar(&args.escuchar)?;
    let enlace = Arc::new(enlace);

    eprintln!(
        "· ayni · {} [{}] escuchando en {}",
        args.nombre,
        hex_corto(&yo.agora_id()),
        enlace.direccion_local()
    );
    if let Some(peer) = &args.conectar {
        enlace.conectar(peer)?;
        eprintln!("· conectando a {peer} …");
    }
    eprintln!("· escribí y Enter para enviar. Ctrl-D para salir.\n");

    // --- hilo de red: aplica entrantes y vuelca el grafo a peers nuevos ---
    {
        let estado = estado.clone();
        let enlace = enlace.clone();
        std::thread::spawn(move || {
            for evento in rx {
                match evento {
                    EventoRed::Conectado(peer) => {
                        // poner al día al recién llegado con nuestro grafo.
                        let instantanea = estado.lock().unwrap().conv.instantanea();
                        let _ = enlace.enviar(&peer, &Sobre::Grafo(instantanea));
                        eprintln!("· peer conectado: {}", peer.0);
                    }
                    EventoRed::Desconectado(peer) => {
                        eprintln!("· peer desconectado: {}", peer.0);
                    }
                    EventoRed::Sobre(_, sobre) => {
                        let mut e = estado.lock().unwrap();
                        let Estado { conv, fus } = &mut *e;
                        let nuevos = match sobre {
                            Sobre::Nodo(n) => fus.aplicar_nodo(conv, n, verificar_firma),
                            Sobre::Grafo(ns) => fus.aplicar_lote(conv, ns, verificar_firma),
                        };
                        for id in &nuevos {
                            if let Some(nodo) = conv.obtener(id) {
                                imprimir(nodo);
                            }
                        }
                    }
                }
            }
        });
    }

    // --- hilo principal: teclado → nodo firmado → difusión ---
    let stdin = std::io::stdin();
    for linea in stdin.lock().lines() {
        let texto = linea?;
        if texto.trim().is_empty() {
            continue;
        }
        let nodo = {
            let mut e = estado.lock().unwrap();
            let nodo = e
                .conv
                .redactar(yo.agora_id(), Carga::Texto(texto), 0, |id| yo.firmar(id));
            e.conv.agregar(nodo.clone()).ok();
            nodo
        };
        enlace.difundir(&Sobre::Nodo(nodo))?;
    }

    eprintln!("\n· chau.");
    Ok(())
}

/// Pinta un nodo entrante: prefijo del autor + texto.
fn imprimir(nodo: &MensajeNodo) {
    let autor = hex_corto(nodo.autor());
    let texto = nodo.contenido.carga.texto().unwrap_or("‹no-texto›");
    println!("[{autor}] {texto}");
}

/// Los primeros 3 bytes de un identificador, en hex — suficiente para distinguir.
fn hex_corto(bytes: &[u8]) -> String {
    bytes[..3].iter().map(|b| format!("{b:02x}")).collect()
}
