// =============================================================================
//  ayni :: ayni-cli — chat soberano en la terminal
// -----------------------------------------------------------------------------
//  Dos terminales, dos identidades, una LAN. Escribís una línea: se firma con
//  tu clave Ed25519, viaja como un nodo del DAG por TCP, y aparece del otro
//  lado verificada. Los grafos convergen sin servidor. Es el MVP de P1.
//
//  Con --cifrar (P2), además se intercambian claves X25519 al conectar y los
//  mensajes viajan cifrados extremo-a-extremo (1:1): por el cable sólo va el
//  ciphertext; el claro nunca toca la red. La autoría sigue siendo pública.
//
//      ayni --nombre Alicia --escuchar 127.0.0.1:7700 --cifrar
//      ayni --nombre Beto --escuchar 127.0.0.1:7701 --conectar 127.0.0.1:7700 --cifrar
// =============================================================================

use std::io::BufRead;
use std::sync::{Arc, Mutex};

use ayni_core::{Carga, Conversacion, MensajeNodo};
use ayni_crypto::{verificar_firma, CanalSeguro, Identidad};
use ayni_sync::{EnlaceTcp, EventoRed, Fusionador, Sobre, Transporte};

use clap::Parser;

/// Chat soberano: DAG firmado sobre TCP LAN, con E2EE 1:1 opcional.
#[derive(Parser)]
#[command(name = "ayni", about = "Chat persona-a-persona soberano (P1 LAN + P2 E2EE 1:1)")]
struct Args {
    /// Tu nombre. Deriva una identidad Ed25519/X25519 determinista (demo): mismo
    /// nombre, misma identidad. En producción viene del keystore cifrado de agora.
    #[arg(long)]
    nombre: String,

    /// Dirección donde escuchar conexiones entrantes.
    #[arg(long, default_value = "127.0.0.1:7700")]
    escuchar: String,

    /// Dirección de un peer al que conectarse al arrancar (opcional).
    #[arg(long)]
    conectar: Option<String>,

    /// Cifrar extremo-a-extremo (1:1) los mensajes salientes hacia el peer.
    #[arg(long)]
    cifrar: bool,
}

/// Estado compartido entre el hilo de red y el de teclado.
struct Estado {
    conv: Conversacion,
    fus: Fusionador,
    /// Canal E2EE con el peer, una vez intercambiadas las claves X25519.
    canal: Option<CanalSeguro>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let seed = *blake3::hash(args.nombre.as_bytes()).as_bytes();
    let yo = Arc::new(Identidad::desde_semilla(seed, args.nombre.clone()));

    let estado = Arc::new(Mutex::new(Estado {
        conv: Conversacion::nueva(),
        fus: Fusionador::nuevo(),
        canal: None,
    }));

    let (enlace, rx) = EnlaceTcp::escuchar(&args.escuchar)?;
    let enlace = Arc::new(enlace);

    eprintln!(
        "· ayni · {} [{}] escuchando en {}{}",
        args.nombre,
        hex_corto(&yo.agora_id()),
        enlace.direccion_local(),
        if args.cifrar { " · E2EE on" } else { "" }
    );
    if let Some(peer) = &args.conectar {
        enlace.conectar(peer)?;
        eprintln!("· conectando a {peer} …");
    }
    eprintln!("· escribí y Enter para enviar. Ctrl-D para salir.\n");

    // --- hilo de red ---
    {
        let estado = estado.clone();
        let enlace = enlace.clone();
        let yo = yo.clone();
        std::thread::spawn(move || {
            for evento in rx {
                match evento {
                    EventoRed::Conectado(peer) => {
                        // saludar (clave X25519) + volcar nuestro grafo.
                        let _ = enlace.enviar(
                            &peer,
                            &Sobre::Hola {
                                x25519: yo.clave_publica_x25519(),
                            },
                        );
                        let instantanea = estado.lock().unwrap().conv.instantanea();
                        let _ = enlace.enviar(&peer, &Sobre::Grafo(instantanea));
                        eprintln!("· peer conectado: {}", peer.0);
                    }
                    EventoRed::Desconectado(peer) => {
                        eprintln!("· peer desconectado: {}", peer.0);
                    }
                    EventoRed::Sobre(_, Sobre::Hola { x25519 }) => {
                        estado.lock().unwrap().canal = Some(yo.canal_con(&x25519));
                        eprintln!("· canal E2EE disponible con el peer");
                    }
                    EventoRed::Sobre(_, sobre) => {
                        let mut e = estado.lock().unwrap();
                        let Estado { conv, fus, canal } = &mut *e;
                        let nuevos = match sobre {
                            Sobre::Nodo(n) => fus.aplicar_nodo(conv, n, verificar_firma),
                            Sobre::Grafo(ns) => fus.aplicar_lote(conv, ns, verificar_firma),
                            Sobre::Hola { .. } => unreachable!(),
                        };
                        for id in &nuevos {
                            if let Some(nodo) = conv.obtener(id) {
                                imprimir(nodo, canal.as_ref());
                            }
                        }
                    }
                }
            }
        });
    }

    // --- hilo principal: teclado → nodo (cifrado si corresponde) → difusión ---
    let stdin = std::io::stdin();
    for linea in stdin.lock().lines() {
        let texto = linea?;
        if texto.trim().is_empty() {
            continue;
        }
        let nodo = {
            let mut e = estado.lock().unwrap();
            // carga: cifrada si --cifrar y ya hay canal; si no, texto plano.
            let carga = match (args.cifrar, &e.canal) {
                (true, Some(canal)) => Carga::Cifrado(canal.cifrar(texto.as_bytes())),
                _ => Carga::Texto(texto),
            };
            let nodo = e.conv.redactar(yo.agora_id(), carga, 0, |id| yo.firmar(id));
            e.conv.agregar(nodo.clone()).ok();
            nodo
        };
        enlace.difundir(&Sobre::Nodo(nodo))?;
    }

    eprintln!("\n· chau.");
    Ok(())
}

/// Pinta un nodo entrante: prefijo del autor + texto (descifrando si hace falta
/// y hay canal).
fn imprimir(nodo: &MensajeNodo, canal: Option<&CanalSeguro>) {
    let autor = hex_corto(nodo.autor());
    let texto = match &nodo.contenido.carga {
        Carga::Texto(t) => t.clone(),
        Carga::Cifrado(blob) => match canal {
            Some(c) => match c.descifrar(blob) {
                Ok(claro) => String::from_utf8_lossy(&claro).into_owned(),
                Err(_) => "‹cifrado: no pude descifrar›".into(),
            },
            None => "‹cifrado: sin canal›".into(),
        },
    };
    println!("[{autor}] {texto}");
}

/// Los primeros 3 bytes de un identificador, en hex.
fn hex_corto(bytes: &[u8]) -> String {
    bytes[..3].iter().map(|b| format!("{b:02x}")).collect()
}
