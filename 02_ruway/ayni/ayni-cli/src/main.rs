// =============================================================================
//  ayni :: ayni-cli — chat soberano en la terminal
// -----------------------------------------------------------------------------
//  La cara de terminal del MISMO `ayni-app::Nucleo` que pinta la GUI. Dos
//  identidades, una red (TCP directo o minga P2P), grafos firmados que convergen
//  sin servidor; con persistencia local-first, cifrado 1:1 opcional, adjuntos y
//  los hechos sociales de P7 (membresía/atestaciones/recibos simétricos).
//
//      ayni --nombre Alicia --escuchar 127.0.0.1:7700 --cifrar
//      ayni --nombre Beto --escuchar 127.0.0.1:7701 --conectar 127.0.0.1:7700 --cifrar
//      # o sobre minga (libp2p):
//      ayni --nombre Ana --transporte minga --escuchar /ip4/127.0.0.1/tcp/7800
//
//  Comandos en el prompt: /miembros /confianza /admitir <hex> /expulsar <hex>
//                         /atestar <hex> [nivel] /adjuntar <ruta> /recibo /ayuda
// =============================================================================

use std::io::BufRead;
use std::sync::{Arc, Mutex};

use ayni_app::{hex_corto, Enlace, Identidad, Nucleo, Tipo};

use clap::Parser;

/// Chat soberano: DAG firmado sobre TCP/minga, con E2EE 1:1 opcional y P7.
#[derive(Parser)]
#[command(name = "ayni", about = "Chat persona-a-persona soberano (DAG firmado, P1–P7)")]
struct Args {
    /// Tu nombre. Deriva una identidad Ed25519/X25519 determinista (demo).
    #[arg(long)]
    nombre: String,

    /// Transporte: `tcp` (LAN directo) o `minga` (libp2p P2P).
    #[arg(long, default_value = "tcp")]
    transporte: String,

    /// Dirección donde escuchar (formato según transporte).
    #[arg(long)]
    escuchar: Option<String>,

    /// Dirección de un peer al que conectarse al arrancar (opcional).
    #[arg(long)]
    conectar: Option<String>,

    /// Ruta del store local (sled). Por defecto `./ayni-<nombre>.db`.
    #[arg(long)]
    data: Option<String>,

    /// Cifrar extremo-a-extremo (1:1) los mensajes salientes.
    #[arg(long)]
    cifrar: bool,

    /// Emitir recibos (simétrico: actívenlo ambos lados para verse).
    #[arg(long)]
    recibos: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let tipo = Tipo::desde_nombre(&args.transporte);
    let bind = args
        .escuchar
        .clone()
        .unwrap_or_else(|| tipo.bind_por_defecto().into());

    let seed = *blake3::hash(args.nombre.as_bytes()).as_bytes();
    let identidad = Identidad::desde_semilla(seed, args.nombre.clone());
    let yo = identidad.agora_id();

    let ruta = args
        .data
        .clone()
        .unwrap_or_else(|| format!("./ayni-{}.db", args.nombre));
    let nucleo = Arc::new(Mutex::new(Nucleo::nuevo(
        identidad,
        Some(std::path::Path::new(&ruta)),
        args.cifrar,
        args.recibos,
    )));

    let (enlace, rx) = Enlace::abrir(tipo, &bind).map_err(|e| format!("no pude abrir el transporte: {e}"))?;
    let enlace = Arc::new(enlace);

    eprintln!(
        "· ayni · {} [{}] · {} en {}{}{}",
        args.nombre,
        hex_corto(&yo),
        enlace.etiqueta(),
        enlace.direccion_local(),
        if args.cifrar { " · E2EE" } else { "" },
        if args.recibos { " · recibos" } else { "" },
    );
    if let Some(peer) = &args.conectar {
        enlace.conectar(peer).map_err(|e| format!("no pude conectar a {peer}: {e}"))?;
        eprintln!("· conectando a {peer} …");
    }
    eprintln!("· escribí y Enter para enviar. /ayuda para comandos. Ctrl-D para salir.\n");

    // --- hilo de red: cada EventoRed pasa por el núcleo; imprimimos lo nuevo. ---
    {
        let nucleo = nucleo.clone();
        let enlace = enlace.clone();
        std::thread::spawn(move || {
            for evento in rx {
                let mut n = nucleo.lock().unwrap();
                let nuevos = n.al_evento(enlace.as_ref(), evento);
                for id in &nuevos {
                    if let Some(nodo) = n.conv.obtener(id) {
                        if *nodo.autor() != yo {
                            println!("[{}] {}", hex_corto(nodo.autor()), n.texto_visible(nodo));
                        }
                    }
                }
            }
        });
    }

    // --- hilo principal: teclado → texto o comando → núcleo. ---
    let stdin = std::io::stdin();
    for linea in stdin.lock().lines() {
        let texto = linea?;
        let recortado = texto.trim();
        if recortado.is_empty() {
            continue;
        }
        let mut n = nucleo.lock().unwrap();
        if let Some(cmd) = recortado.strip_prefix('/') {
            let aviso = comando(&mut n, enlace.as_ref(), cmd);
            eprintln!("· {aviso}");
        } else {
            n.enviar_texto(enlace.as_ref(), recortado);
        }
    }

    eprintln!("\n· chau.");
    Ok(())
}

/// Ejecuta un comando del prompt y devuelve una línea de estado.
fn comando(nucleo: &mut Nucleo, enlace: &Enlace, cmd: &str) -> String {
    let mut campos = cmd.split_whitespace();
    let verbo = campos.next().unwrap_or("");
    match verbo {
        "miembros" => {
            let m = nucleo.conv.membresia();
            let mut s = format!("membresía ({}):", m.len());
            for id in &m.miembros {
                let fund = if Some(*id) == m.fundador { " ·fund" } else { "" };
                s.push_str(&format!("\n    {}{}", hex_corto(id), fund));
            }
            s
        }
        "confianza" => {
            let conf = nucleo.conv.confianza_desde(&nucleo.yo());
            if conf.is_empty() {
                "confianza: vacía (nadie atestiguado)".into()
            } else {
                let mut s = String::from("confianza:");
                for (id, saltos) in &conf {
                    s.push_str(&format!("\n    {} · {} salto/s", hex_corto(id), saltos));
                }
                s
            }
        }
        "adjuntar" | "adj" => {
            let ruta = campos.collect::<Vec<_>>().join(" ");
            if ruta.is_empty() {
                return "uso: /adjuntar <ruta>".into();
            }
            match nucleo.adjuntar(enlace, &ruta) {
                Ok(n) => format!("adjuntado: {n}"),
                Err(e) => e,
            }
        }
        "admitir" | "expulsar" | "atestar" => {
            let Some(pref) = campos.next() else {
                return format!("uso: /{verbo} <hex>");
            };
            let Some(sujeto) = nucleo.resolver(pref) else {
                return format!("no conozco a «{pref}» (mirá /miembros)");
            };
            match verbo {
                "admitir" => {
                    nucleo.admitir(enlace, sujeto);
                    format!("admitiste a {}", hex_corto(&sujeto))
                }
                "expulsar" => {
                    nucleo.expulsar(enlace, sujeto);
                    format!("expulsaste a {}", hex_corto(&sujeto))
                }
                _ => {
                    let nivel: u8 = campos.next().and_then(|s| s.parse().ok()).unwrap_or(5);
                    nucleo.atestar(enlace, sujeto, nivel);
                    format!("das fe de {} (nivel {nivel})", hex_corto(&sujeto))
                }
            }
        }
        "recibo" => {
            nucleo.acusar_cabezas(enlace);
            "acuse de recibo enviado".into()
        }
        "ayuda" | "" => {
            "/miembros /confianza /admitir <hex> /expulsar <hex> /atestar <hex> [nivel] /adjuntar <ruta> /recibo".into()
        }
        otro => format!("comando desconocido: «{otro}» (/ayuda)"),
    }
}
