//! `asistente-puente` — binario scaffolding del puente Linux entre el
//! `asistente.wasm` (kernel wawa, vía Akasha) y los LLMs externos
//! (vía `pluma-llm`).
//!
//! Dos modos de transporte, según el flag de línea de comandos:
//!
//! - **stdio** (default, sin args): un único turno
//!   `Consulta → Propuesta/Error` sobre stdin/stdout. Útil para tests o
//!   ejercicios con `printf` + `xxd`.
//! - **daemon** (`--socket <path>`): escucha en un Unix domain socket,
//!   acepta clientes en serie (uno a la vez), atiende N consultas por
//!   cliente hasta EOF. Útil para que el asistente Linux
//!   (`mirada-asistente-llimphi`) lo consulte sin tener que ejecutar un
//!   proceso nuevo por cada pregunta.
//!
//! El payload en ambos modos es `MensajeAsistente` en postcard binario
//! precedido por un `u32 LE` con la longitud del frame.
//!
//! El bind a un socket raw Akasha (multicast EtherType propio, dedup,
//! multiplexación por id entre nodos) es la siguiente vuelta — el modo
//! daemon Unix prueba la arquitectura sin pedir `cap_net_raw`.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

use asistente_puente::{
    construir_prompt_usuario, traducir_propuesta_llm, InterpretacionLlm, PROMPT_SISTEMA_WAWA,
};
use format::MensajeAsistente;
use pluma_llm_core::{ChatClient, ChatRequest};

mod akasha;

/// Cota dura del frame entrante. Una consulta razonable rara vez excede
/// algunos KB; un `u32` declarando muchísimos megabytes es señal de
/// transporte roto, lo rechazamos.
const MAX_FRAME: usize = 64 * 1024;

/// Cota de tokens de salida del LLM. Suficiente para una propuesta JSON
/// con `explicacion` de varias oraciones.
const MAX_TOKENS_RESPUESTA: u32 = 500;

fn main() {
    if let Err(e) = correr() {
        eprintln!("asistente-puente: {e}");
        std::process::exit(1);
    }
}

fn correr() -> Result<(), String> {
    let cli = parsear_args(std::env::args().skip(1))?;
    let llm = pluma_llm::from_env().map_err(|e| format!("inicializando pluma-llm: {e}"))?;
    // Runtime de Tokio compartido — un solo current-thread vale para
    // ambos modos (stdio = un turno; daemon = un cliente a la vez).
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("creando tokio runtime: {e}"))?;
    // Fase 60 v4 :: si el operador suministro --firma-clave, cargamos la
    // clave Ed25519 al arrancar. La misma seed/SK funciona en
    // `wawactl daemon-firma`. Solo el modo --akasha la usa hoy (los
    // demas modos solo manejan `MensajeAsistente`).
    let config_firma = match (cli.firma_clave.as_deref(), cli.firma_slot) {
        (Some(path), slot) => Some(akasha::ConfigFirma {
            clave: akasha::cargar_clave_privada(path)?,
            slot,
            log_path: cli.firma_log.clone(),
        }),
        (None, _) => None,
    };
    match cli.modo {
        Modo::Stdio => correr_stdio(&llm, &rt),
        Modo::Daemon { socket } => correr_daemon(&llm, &rt, &socket),
        Modo::Akasha { iface } => akasha::correr(&llm, &rt, &iface, config_firma.as_ref()),
    }
}

enum Modo {
    Stdio,
    Daemon { socket: String },
    Akasha { iface: String },
}

struct Cli {
    modo: Modo,
    /// Fase 60 v4 :: clave Ed25519 que el puente usa para firmar
    /// RequestFirma. Sin este flag, el puente responde Error a cualquier
    /// RequestFirma (el flujo LLM funciona igual).
    firma_clave: Option<String>,
    /// Fase 60 v4 :: slot del anillo AGORA_AUTH_RING (0=primaria,
    /// 1=secundaria, 2=recuperacion).
    firma_slot: u8,
    /// Fase 60 v4 :: log de auditoria de firmas emitidas/rechazadas.
    firma_log: String,
}

/// Parser minimal de argumentos. Sin clap — bandera de modo + tres
/// banderas opcionales de firma (--firma-clave / --firma-slot / --firma-log).
fn parsear_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut modo: Option<Modo> = None;
    let mut firma_clave: Option<String> = None;
    let mut firma_slot: u8 = 0;
    let mut firma_log: String = "asistente_puente_audit.log".to_string();
    let mut it = args.peekable();
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            "--socket" => {
                let path = it.next().ok_or("--socket requiere una ruta")?;
                fijar_modo(&mut modo, Modo::Daemon { socket: path })?;
            }
            "--akasha" => {
                let iface = it.next().ok_or("--akasha requiere un nombre de interfaz")?;
                fijar_modo(&mut modo, Modo::Akasha { iface })?;
            }
            "--firma-clave" => {
                let path = it.next().ok_or("--firma-clave requiere una ruta")?;
                firma_clave = Some(path);
            }
            "--firma-slot" => {
                let n = it.next().ok_or("--firma-slot requiere un numero (0/1/2)")?;
                firma_slot = n
                    .parse::<u8>()
                    .map_err(|_| format!("--firma-slot: `{n}` no es un u8"))?;
                if firma_slot > 2 {
                    return Err(format!(
                        "--firma-slot {firma_slot}: el anillo tiene 3 slots (0/1/2)"
                    ));
                }
            }
            "--firma-log" => {
                let path = it.next().ok_or("--firma-log requiere una ruta")?;
                firma_log = path;
            }
            otro => return Err(format!("argumento desconocido: {otro} (usá --help)")),
        }
    }
    Ok(Cli {
        modo: modo.unwrap_or(Modo::Stdio),
        firma_clave,
        firma_slot,
        firma_log,
    })
}

/// Fija el modo de transporte exactamente una vez — `--socket` y `--akasha`
/// son mutuamente excluyentes.
fn fijar_modo(actual: &mut Option<Modo>, nuevo: Modo) -> Result<(), String> {
    if actual.is_some() {
        return Err("--socket y --akasha son mutuamente excluyentes".into());
    }
    *actual = Some(nuevo);
    Ok(())
}

fn print_help() {
    eprintln!(
        "asistente-puente — puente entre asistente.wasm (wawa) y LLMs externos\n\
         \n\
         Uso:\n  \
           asistente-puente                    Un turno por stdin/stdout (postcard).\n  \
           asistente-puente --socket <path>    Daemon Unix socket; clientes en serie.\n  \
           asistente-puente --akasha <iface>   AF_PACKET sobre interfaz fisica.\n  \
           asistente-puente --help             Muestra esta ayuda.\n\
         \n\
         Fase 60 v4 :: el modo --akasha acepta firma de propuestas hash:\n  \
           --firma-clave <PATH>    Clave Ed25519 (seed 32 B o SK 64 B).\n  \
           --firma-slot <0|1|2>    Slot del anillo AGORA_AUTH_RING (default 0).\n  \
           --firma-log <PATH>      Audit log (default asistente_puente_audit.log).\n\
         \n\
         stdio y --socket transportan `MensajeAsistente` (postcard, prefijo u32 LE).\n\
         --akasha transporta `TipoCable` (binario corto: cabecera 12 B + payload)\n\
         sobre EtherType 0x88B6 — el protocolo que habla `asistente.wasm` en wawa.\n\
         Requiere permisos para abrir AF_PACKET (cap_net_raw, root o setcap).\n\
         \n\
         pluma-llm autodetecta el backend desde el entorno:\n  \
           ANTHROPIC_API_KEY, GEMINI_API_KEY, DEEPSEEK_API_KEY, COHERE_API_KEY,\n  \
           PLUMA_LLM_BACKEND=ollama  (entre otros). Sin credencial cae al Mock."
    );
}

// ---------------------------------------------------------------------
// Modo stdio
// ---------------------------------------------------------------------

fn correr_stdio(llm: &Arc<dyn ChatClient>, rt: &tokio::runtime::Runtime) -> Result<(), String> {
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let salida = atender_turno(&mut stdin, llm, rt)?;
    escribir_frame(&mut stdout, &salida)
}

// ---------------------------------------------------------------------
// Modo daemon
// ---------------------------------------------------------------------

fn correr_daemon(
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
    socket: &str,
) -> Result<(), String> {
    // Si el socket ya existe (proceso anterior caído sin cleanup), lo
    // removemos antes de bind — no podemos heredar la posición del
    // proceso muerto.
    if Path::new(socket).exists() {
        std::fs::remove_file(socket).map_err(|e| format!("borrando socket viejo: {e}"))?;
    }
    let listener =
        UnixListener::bind(socket).map_err(|e| format!("bind {socket}: {e}"))?;
    eprintln!("asistente-puente: escuchando en {socket}");
    eprintln!("asistente-puente: Ctrl-C para terminar");

    loop {
        let (stream, _) = listener
            .accept()
            .map_err(|e| format!("accept: {e}"))?;
        eprintln!("asistente-puente: cliente conectado");
        if let Err(e) = atender_cliente(stream, llm, rt) {
            eprintln!("asistente-puente: cliente cerrado por {e}");
        }
    }
}

/// Atiende un cliente del socket: bucle de turnos hasta EOF. Cada turno
/// es un par Consulta/Propuesta independiente — el cliente correlaciona
/// por `id` si quiere paralelismo, aunque hoy el servidor es serial.
fn atender_cliente(
    mut stream: UnixStream,
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
) -> Result<(), String> {
    loop {
        let salida = match atender_turno(&mut stream, llm, rt) {
            Ok(s) => s,
            Err(e) if e == "EOF" => return Ok(()),
            Err(e) => return Err(e),
        };
        escribir_frame(&mut stream, &salida)?;
    }
}

// ---------------------------------------------------------------------
// Lógica compartida: un turno
// ---------------------------------------------------------------------

/// Lee un `MensajeAsistente::Consulta` del lector, consulta al LLM, y
/// devuelve la `MensajeAsistente` que toca enviar de vuelta. EOF al
/// inicio del frame se propaga como `Err("EOF")` (el caller lo trata).
fn atender_turno<R: Read>(
    lector: &mut R,
    llm: &Arc<dyn ChatClient>,
    rt: &tokio::runtime::Runtime,
) -> Result<MensajeAsistente, String> {
    let entrada = leer_frame(lector)?;
    let mensaje = MensajeAsistente::deserializar(&entrada)
        .map_err(|e| format!("deserializando MensajeAsistente: {e}"))?;
    let (id, prompt, contexto) = match mensaje {
        MensajeAsistente::Consulta {
            id,
            prompt,
            contexto,
        } => (id, prompt, contexto),
        otro => return Err(format!("esperaba Consulta, recibí {otro:?}")),
    };
    let prompt_user = construir_prompt_usuario(&contexto, &prompt);
    let req = ChatRequest::una_vuelta(prompt_user, MAX_TOKENS_RESPUESTA)
        .con_sistema(PROMPT_SISTEMA_WAWA);
    let resp = rt.block_on(llm.complete(&req));
    Ok(traducir_a_mensaje(id, resp.map(|r| r.content)))
}

fn traducir_a_mensaje(
    id: u64,
    resp: Result<String, pluma_llm_core::ChatError>,
) -> MensajeAsistente {
    match resp {
        Ok(content) => match traducir_propuesta_llm(&content) {
            InterpretacionLlm::Propuesta {
                accion,
                explicacion,
                confianza,
            } => MensajeAsistente::Propuesta {
                id,
                accion,
                explicacion,
                confianza,
            },
            InterpretacionLlm::Rechazo(motivo) => MensajeAsistente::Error { id, motivo },
            InterpretacionLlm::Error(motivo) => MensajeAsistente::Error { id, motivo },
        },
        Err(e) => MensajeAsistente::Error {
            id,
            motivo: format!("transporte LLM: {e}"),
        },
    }
}

// ---------------------------------------------------------------------
// Codec de frames (u32 LE de longitud + payload postcard)
// ---------------------------------------------------------------------

/// Lee un frame del lector. Devuelve `Err("EOF")` si el lector cierra
/// en el primer byte (clean shutdown). Cualquier otro error es un fallo.
fn leer_frame<R: Read>(lector: &mut R) -> Result<Vec<u8>, String> {
    let mut long_buf = [0u8; 4];
    if let Err(e) = lector.read_exact(&mut long_buf) {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            return Err("EOF".into());
        }
        return Err(format!("leyendo prefijo de longitud: {e}"));
    }
    let largo = u32::from_le_bytes(long_buf) as usize;
    if largo == 0 || largo > MAX_FRAME {
        return Err(format!("longitud de frame fuera de rango: {largo}"));
    }
    let mut payload = vec![0u8; largo];
    lector
        .read_exact(&mut payload)
        .map_err(|e| format!("leyendo payload de {largo} B: {e}"))?;
    Ok(payload)
}

fn escribir_frame<W: Write>(escritor: &mut W, mensaje: &MensajeAsistente) -> Result<(), String> {
    let bytes = mensaje
        .serializar()
        .map_err(|e| format!("serializando salida: {e}"))?;
    let largo = bytes.len() as u32;
    escritor
        .write_all(&largo.to_le_bytes())
        .map_err(|e| format!("escribiendo prefijo: {e}"))?;
    escritor
        .write_all(&bytes)
        .map_err(|e| format!("escribiendo payload: {e}"))?;
    escritor.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------
// Tests del parser de args y del codec — sin red.
// ---------------------------------------------------------------------

#[cfg(test)]
mod pruebas_args {
    use super::*;

    #[test]
    fn args_vacio_es_stdio() {
        let cli = parsear_args(std::iter::empty()).expect("ok");
        assert!(matches!(cli.modo, Modo::Stdio));
        assert!(cli.firma_clave.is_none());
        assert_eq!(cli.firma_slot, 0);
    }

    #[test]
    fn args_socket_con_path() {
        let v = vec!["--socket".to_string(), "/tmp/x.sock".to_string()];
        let cli = parsear_args(v.into_iter()).expect("ok");
        match cli.modo {
            Modo::Daemon { socket } => assert_eq!(socket, "/tmp/x.sock"),
            otro => panic!("esperaba Daemon, obtuve {:?}", matches!(otro, Modo::Stdio)),
        }
    }

    #[test]
    fn args_socket_sin_path_es_error() {
        let v = vec!["--socket".to_string()];
        assert!(parsear_args(v.into_iter()).is_err());
    }

    #[test]
    fn args_desconocido_es_error() {
        let v = vec!["--inventado".to_string()];
        assert!(parsear_args(v.into_iter()).is_err());
    }

    // Fase 60 v4 :: nuevos flags de firma.

    #[test]
    fn args_firma_clave_y_slot() {
        let v = vec![
            "--akasha".to_string(),
            "eth0".to_string(),
            "--firma-clave".to_string(),
            "/etc/wawa/op.sk".to_string(),
            "--firma-slot".to_string(),
            "1".to_string(),
        ];
        let cli = parsear_args(v.into_iter()).expect("ok");
        assert!(matches!(cli.modo, Modo::Akasha { .. }));
        assert_eq!(cli.firma_clave.as_deref(), Some("/etc/wawa/op.sk"));
        assert_eq!(cli.firma_slot, 1);
    }

    #[test]
    fn args_firma_slot_fuera_de_rango() {
        let v = vec![
            "--firma-slot".to_string(),
            "3".to_string(),
        ];
        let err = match parsear_args(v.into_iter()) {
            Err(e) => e,
            Ok(_) => panic!("esperaba error de slot fuera de rango"),
        };
        assert!(err.contains("3 slots"));
    }

    #[test]
    fn args_socket_y_akasha_son_excluyentes() {
        let v = vec![
            "--socket".to_string(),
            "/tmp/x.sock".to_string(),
            "--akasha".to_string(),
            "eth0".to_string(),
        ];
        let err = match parsear_args(v.into_iter()) {
            Err(e) => e,
            Ok(_) => panic!("esperaba error de exclusion mutua"),
        };
        assert!(err.contains("mutuamente excluyentes"));
    }

    #[test]
    fn args_firma_log_default() {
        let cli = parsear_args(std::iter::empty()).expect("ok");
        assert_eq!(cli.firma_log, "asistente_puente_audit.log");
    }
}

#[cfg(test)]
mod pruebas_codec {
    use super::*;
    use format::{AccionPropuesta, Contexto, MensajeAsistente};
    use std::io::Cursor;

    #[test]
    fn frame_ida_y_vuelta() {
        // Escribir un frame con un MensajeAsistente, releerlo, verificar
        // que decodifica al mismo enum.
        let original = MensajeAsistente::Propuesta {
            id: 7,
            accion: AccionPropuesta::Notar {
                texto: "test".into(),
            },
            explicacion: "ok".into(),
            confianza: 1.0,
        };
        let mut buf: Vec<u8> = Vec::new();
        escribir_frame(&mut buf, &original).expect("escribir");
        let mut cursor = Cursor::new(buf);
        let bytes = leer_frame(&mut cursor).expect("leer");
        let leido = MensajeAsistente::deserializar(&bytes).expect("deserializar");
        assert_eq!(leido, original);
    }

    #[test]
    fn frame_eof_limpio_se_reporta_como_eof() {
        let mut cursor = Cursor::new(Vec::<u8>::new());
        assert_eq!(leer_frame(&mut cursor).unwrap_err(), "EOF");
    }

    #[test]
    fn frame_longitud_cero_rechazada() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0u32.to_le_bytes());
        let mut cursor = Cursor::new(buf);
        let err = leer_frame(&mut cursor).unwrap_err();
        assert!(err.contains("fuera de rango"));
    }

    #[test]
    fn frame_longitud_excesiva_rechazada() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(u32::MAX).to_le_bytes());
        let mut cursor = Cursor::new(buf);
        let err = leer_frame(&mut cursor).unwrap_err();
        assert!(err.contains("fuera de rango"));
    }

    #[test]
    fn frame_payload_truncado_rechazado() {
        // Anuncia 100 B pero entrega 10.
        let mut buf = Vec::new();
        buf.extend_from_slice(&100u32.to_le_bytes());
        buf.extend_from_slice(&[0u8; 10]);
        let mut cursor = Cursor::new(buf);
        assert!(leer_frame(&mut cursor).is_err());
    }

    #[test]
    fn dos_frames_consecutivos_se_leen_independientes() {
        // El cliente puede mandar muchos turnos en la misma conexión.
        let a = MensajeAsistente::Error {
            id: 1,
            motivo: "a".into(),
        };
        let b = MensajeAsistente::Error {
            id: 2,
            motivo: "b".into(),
        };
        let mut buf: Vec<u8> = Vec::new();
        escribir_frame(&mut buf, &a).unwrap();
        escribir_frame(&mut buf, &b).unwrap();
        let mut cursor = Cursor::new(buf);
        let leido_a = MensajeAsistente::deserializar(&leer_frame(&mut cursor).unwrap()).unwrap();
        let leido_b = MensajeAsistente::deserializar(&leer_frame(&mut cursor).unwrap()).unwrap();
        assert_eq!(leido_a, a);
        assert_eq!(leido_b, b);
    }

    #[test]
    fn traducir_a_mensaje_propuesta_ok() {
        let salida = traducir_a_mensaje(
            42,
            Ok(r#"{"tipo": "notar", "texto": "hola"}"#.to_string()),
        );
        match salida {
            MensajeAsistente::Propuesta {
                id,
                accion: AccionPropuesta::Notar { texto },
                ..
            } => {
                assert_eq!(id, 42);
                assert_eq!(texto, "hola");
            }
            otro => panic!("esperaba Propuesta::Notar, obtuve {otro:?}"),
        }
        // Suprimir warning de Contexto no usado en este test.
        let _ = Contexto::default();
    }

    #[test]
    fn traducir_a_mensaje_error_de_transporte() {
        let salida = traducir_a_mensaje(
            7,
            Err(pluma_llm_core::ChatError::Cancelled),
        );
        match salida {
            MensajeAsistente::Error { id, motivo } => {
                assert_eq!(id, 7);
                assert!(motivo.contains("transporte LLM"));
            }
            otro => panic!("esperaba Error, obtuve {otro:?}"),
        }
    }
}
