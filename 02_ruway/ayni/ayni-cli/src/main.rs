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

use std::collections::BTreeSet;
use std::error::Error;
use std::io::BufRead;
use std::sync::{Arc, Mutex};

use ayni_core::{
    AccionMembresia, AgoraId, Atestacion, CambioMembresia, Carga, Conversacion, MensajeNodo, Recibo,
};
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
    eprintln!("· escribí y Enter para enviar. Ctrl-D para salir.");
    eprintln!("· comandos P7: /miembros /confianza /admitir <hex> /expulsar <hex> /atestar <hex> [nivel] /recibo /ayuda\n");

    // --- hilo de red ---
    {
        let estado = estado.clone();
        let enlace = enlace.clone();
        let yo = yo.clone();
        std::thread::spawn(move || {
            for evento in rx {
                match evento {
                    EventoRed::Conectado(peer) => {
                        // saludar (clave X25519) + anunciar cabezas (anti-entropía).
                        let _ = enlace.enviar(
                            &peer,
                            &Sobre::Hola {
                                x25519: yo.clave_publica_x25519(),
                            },
                        );
                        let cabezas = estado.lock().unwrap().conv.cabezas();
                        let _ = enlace.enviar(&peer, &Sobre::Cabezas(cabezas));
                        eprintln!("· peer conectado: {}", peer.0);
                    }
                    EventoRed::Desconectado(peer) => {
                        eprintln!("· peer desconectado: {}", peer.0);
                    }
                    EventoRed::Sobre(_, Sobre::Hola { x25519 }) => {
                        estado.lock().unwrap().canal = Some(yo.canal_con(&x25519));
                        eprintln!("· canal E2EE disponible con el peer");
                    }
                    EventoRed::Sobre(peer, sobre) => {
                        // anti-entropía: procesar, responder pedidos, imprimir lo nuevo.
                        let (lineas, respuestas) = {
                            let mut e = estado.lock().unwrap();
                            let Estado { conv, fus, canal } = &mut *e;
                            let (nuevos, resp) = fus.procesar(conv, sobre, verificar_firma);
                            let lineas: Vec<String> = nuevos
                                .iter()
                                .filter_map(|id| conv.obtener(id))
                                .map(|n| formatear(n, canal.as_ref()))
                                .collect();
                            (lineas, resp)
                        };
                        for r in respuestas {
                            let _ = enlace.enviar(&peer, &r);
                        }
                        for l in lineas {
                            println!("{l}");
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
        let recortado = texto.trim();
        if recortado.is_empty() {
            continue;
        }
        // Los comandos P7 (confianza/membresía/recibos) empiezan por '/'.
        if let Some(resto) = recortado.strip_prefix('/') {
            manejar_comando(resto, &estado, &yo, &enlace)?;
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

/// Maneja un comando de la barra `/…` — la UX de P7 (confianza, membresía,
/// recibos). Los que mutan el grafo (admitir/expulsar/atestar/recibo) redactan
/// un nodo firmado, lo agregan al grafo local y lo difunden, igual que el texto.
fn manejar_comando(
    cmd: &str,
    estado: &Arc<Mutex<Estado>>,
    yo: &Arc<Identidad>,
    enlace: &Arc<EnlaceTcp>,
) -> Result<(), Box<dyn Error>> {
    let mut campos = cmd.split_whitespace();
    let verbo = campos.next().unwrap_or("");

    match verbo {
        "miembros" => {
            let m = estado.lock().unwrap().conv.membresia();
            eprintln!("· membresía vigente ({}):", m.len());
            for id in &m.miembros {
                let fund = if Some(*id) == m.fundador { " ·fundador" } else { "" };
                let yo_m = if *id == yo.agora_id() { " ←vos" } else { "" };
                eprintln!("    {}{}{}", hex_corto(id), fund, yo_m);
            }
        }
        "confianza" => {
            let conf = estado.lock().unwrap().conv.confianza_desde(&yo.agora_id());
            if conf.is_empty() {
                eprintln!("· tu grafo de confianza está vacío (nadie atestiguado todavía).");
            } else {
                eprintln!("· confías en (por caminos de atestaciones):");
                for (id, saltos) in &conf {
                    eprintln!("    {} · a {} salto/s", hex_corto(id), saltos);
                }
            }
        }
        "admitir" | "expulsar" | "atestar" => {
            let Some(prefijo) = campos.next() else {
                eprintln!("· uso: /{verbo} <hex> {}", if verbo == "atestar" { "[nivel 1..255]" } else { "" });
                return Ok(());
            };
            let sujeto = {
                let e = estado.lock().unwrap();
                resolver(&e.conv, yo, prefijo)
            };
            let Some(sujeto) = sujeto else {
                eprintln!("· no conozco a nadie cuyo id empiece por «{prefijo}» (mirá /miembros).");
                return Ok(());
            };
            let carga = match verbo {
                "admitir" => Carga::Membresia(CambioMembresia {
                    accion: AccionMembresia::Alta,
                    sujeto,
                }),
                "expulsar" => Carga::Membresia(CambioMembresia {
                    accion: AccionMembresia::Baja,
                    sujeto,
                }),
                _ => {
                    let nivel: u8 = campos.next().and_then(|s| s.parse().ok()).unwrap_or(5);
                    Carga::Atestacion(Atestacion { sujeto, nivel })
                }
            };
            difundir_carga(estado, yo, enlace, carga)?;
            eprintln!("· hecho: {verbo} {}", hex_corto(&sujeto));
        }
        "recibo" => {
            let cabezas = estado.lock().unwrap().conv.cabezas();
            if cabezas.is_empty() {
                eprintln!("· nada que acusar (conversación vacía).");
                return Ok(());
            }
            let n = cabezas.len();
            difundir_carga(estado, yo, enlace, Carga::Recibo(Recibo { vistos: cabezas }))?;
            eprintln!("· acuse de recibo enviado ({n} cabeza/s vista/s).");
        }
        "ayuda" | "" => {
            eprintln!("· /miembros · /confianza · /admitir <hex> · /expulsar <hex> · /atestar <hex> [nivel] · /recibo");
        }
        otro => eprintln!("· comando desconocido: «{otro}» (probá /ayuda)."),
    }
    Ok(())
}

/// Redacta un nodo con la carga dada, lo agrega al grafo local y lo difunde.
fn difundir_carga(
    estado: &Arc<Mutex<Estado>>,
    yo: &Arc<Identidad>,
    enlace: &Arc<EnlaceTcp>,
    carga: Carga,
) -> Result<(), Box<dyn Error>> {
    let nodo = {
        let mut e = estado.lock().unwrap();
        let nodo = e.conv.redactar(yo.agora_id(), carga, 0, |id| yo.firmar(id));
        e.conv.agregar(nodo.clone()).ok();
        nodo
    };
    enlace.difundir(&Sobre::Nodo(nodo))?;
    Ok(())
}

/// Resuelve un prefijo hex (el que imprime `/miembros`) a una identidad agora
/// CONOCIDA — un autor presente en el grafo, o uno mismo. Sin directorio
/// global: sólo se puede nombrar a quien ya dejó huella en la conversación.
fn resolver(conv: &Conversacion, yo: &Identidad, prefijo: &str) -> Option<AgoraId> {
    let pref = prefijo.to_lowercase();
    let mut candidatos: BTreeSet<AgoraId> = conv.nodos().map(|(_, n)| *n.autor()).collect();
    candidatos.insert(yo.agora_id());
    candidatos.into_iter().find(|id| hex_corto(id).starts_with(&pref))
}

/// Formatea un nodo entrante: `[autor] texto` (descifrando si hace falta y hay
/// canal). Las cargas sociales de P7 se muestran como actos legibles.
fn formatear(nodo: &MensajeNodo, canal: Option<&CanalSeguro>) -> String {
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
        Carga::Adjunto(a) => {
            format!("‹adjunto: {} · {} · {} B›", a.nombre, a.app, a.tamano)
        }
        Carga::Membresia(m) => match m.accion {
            AccionMembresia::Alta => format!("‹admite a {}›", hex_corto(&m.sujeto)),
            AccionMembresia::Baja => format!("‹expulsa a {}›", hex_corto(&m.sujeto)),
        },
        Carga::Atestacion(at) if at.nivel == 0 => {
            format!("‹retira su fe en {}›", hex_corto(&at.sujeto))
        }
        Carga::Atestacion(at) => {
            format!("‹da fe de {} (nivel {})›", hex_corto(&at.sujeto), at.nivel)
        }
        Carga::Recibo(r) => format!("‹acusa recibo de {} mensaje/s›", r.vistos.len()),
    };
    format!("[{autor}] {texto}")
}

/// Los primeros 3 bytes de un identificador, en hex.
fn hex_corto(bytes: &[u8]) -> String {
    bytes[..3].iter().map(|b| format!("{b:02x}")).collect()
}
