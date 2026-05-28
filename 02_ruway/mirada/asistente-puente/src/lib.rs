//! `asistente-puente` — lógica pura del puente Linux entre `asistente.wasm`
//! (kernel wawa, vía Akasha) y los LLMs externos (vía `pluma-llm`).
//!
//! Este crate contiene las piezas testeables sin red ni I/O:
//!
//! - [`PROMPT_SISTEMA_WAWA`] — instrucciones que el puente le pone al LLM
//!   para que su respuesta sea procesable. Cita las acciones posibles
//!   (variantes de [`format::AccionPropuesta`]) y exige un JSON estricto.
//! - [`traducir_propuesta_llm`] — parsea la respuesta del LLM y devuelve
//!   una `format::AccionPropuesta` ya validada (lista para empaquetar en
//!   un `MensajeAsistente::Propuesta`). Pura. Testeada.
//! - [`construir_prompt_usuario`] — pega el `Contexto` recibido del
//!   asistente.wasm + el `prompt` humano + recordatorio del JSON
//!   esperado. La asimetría con `mirada-asistente-llimphi` (Linux side)
//!   es deliberada: aquí el contexto viene del kernel wawa, no de un
//!   `mirada-ctl windows`.
//!
//! El binario (`main.rs`) cablea esto sobre un transporte concreto. El
//! scaffolding actual usa stdin/stdout en postcard binario para que un
//! test end-to-end o un humano con `printf` puedan ejercitar el flujo;
//! el socket raw Akasha viene en una vuelta posterior (ver
//! `docs/ASISTENTE_WAWA.md` §3).

use format::{
    escribir_cabecera_cable, AccionPropuesta, Contexto, Hash, TipoCable, TAM_CABECERA_CABLE,
};
use serde::Deserialize;

/// El prompt de sistema que el puente le envia al LLM. Lista las acciones
/// posibles, exige JSON estricto y advierte sobre el modelo de seguridad
/// (la IA propone, el humano firma).
pub const PROMPT_SISTEMA_WAWA: &str = "Eres el asistente de un nodo wawa (sistema operativo bare-metal de la suite gioser). \
El usuario del nodo te describe lo que quiere hacer y tú respondes EXCLUSIVAMENTE con un \
objeto JSON con esta forma exacta:\n\
\n\
  {\"tipo\": \"lanzar\", \"plantilla\": N, \"explicacion\": \"breve por qué\"}\n\
  {\"tipo\": \"instalar\", \"manifiesto\": \"<64hex>\", \"explicacion\": \"...\"}\n\
  {\"tipo\": \"configurar\", \"config\": \"<64hex>\", \"explicacion\": \"...\"}\n\
  {\"tipo\": \"notar\", \"texto\": \"...\"}\n\
\n\
Si no entiendes la intención o no hay acción adecuada, responde:\n\
\n\
  {\"error\": \"razón breve\"}\n\
\n\
SIGNIFICADO de cada tipo:\n\
- `lanzar`: pide al kernel que abra la app N-ésima del manifiesto.\n\
  N es un índice 0..M-1 del catálogo que vendrá en el bloque\n\
  '# Estado actual'. NO inventes índices.\n\
- `instalar`: propone re-anclar el manifiesto al hash dado. El humano\n\
  TIENE que firmar antes de que el kernel acepte; nunca propongas esto\n\
  sin que el usuario lo haya pedido explícitamente.\n\
- `configurar`: igual que instalar pero para la `Configuracion` activa.\n\
  Misma firma humana obligatoria.\n\
- `notar`: NO ejerce ninguna acción. Útil para responder preguntas tipo\n\
  '¿cuántas apps tengo?' sin disparar cambios. El campo `texto` es lo\n\
  que el operador va a leer.\n\
\n\
REGLAS: (1) responde SOLO con el JSON, sin prosa antes ni después. \
(2) NO inventes hashes — sólo proponé `instalar`/`configurar` con un \
hash que el usuario haya nombrado. (3) Para acciones destructivas o no \
solicitadas, prefiere `notar` con una explicación.";

/// El JSON del LLM. Distinto del enum `AccionPropuesta` del kernel porque
/// el LLM nos da strings hex en lugar de bytes, y discrimina por un campo
/// `tipo` que aquí mapeamos a la variante.
#[derive(Debug, Deserialize)]
#[serde(tag = "tipo", rename_all = "lowercase")]
enum PropuestaLlm {
    Lanzar {
        plantilla: u32,
        #[serde(default)]
        explicacion: String,
    },
    Instalar {
        manifiesto: String,
        #[serde(default)]
        explicacion: String,
    },
    Configurar {
        config: String,
        #[serde(default)]
        explicacion: String,
    },
    Notar {
        texto: String,
    },
}

/// Forma alternativa: el LLM dice "no puedo" / "no quiero".
#[derive(Debug, Deserialize)]
struct RechazoLlm {
    error: String,
}

/// El resultado de interpretar la respuesta del LLM. Mantiene una capa de
/// distancia entre "lo que dijo el modelo" y "lo que enviamos al kernel"
/// — el llamante decide si lo empaqueta en `MensajeAsistente::Propuesta`
/// o en `MensajeAsistente::Error`.
#[derive(Debug, PartialEq)]
pub enum InterpretacionLlm {
    /// Acción válida y lista para mandar al kernel. `confianza` resume la
    /// calidad del parseo: `1.0` si el LLM produjo JSON limpio con todos
    /// los campos esperados; `0.8` si tuvo que limpiar markdown fences;
    /// más abajo si tuvo que adivinar algo (no implementado todavía).
    Propuesta {
        accion: AccionPropuesta,
        explicacion: String,
        confianza: f32,
    },
    /// El LLM respondió `{error: ...}`. No es un error de transporte —
    /// es el modelo declinando.
    Rechazo(String),
    /// No pudimos parsear nada útil. La cadena trae el motivo y un eco
    /// del crudo para que el humano vea qué dijo el modelo.
    Error(String),
}

/// Interpreta la respuesta cruda del LLM. Pura: sin red, sin grafo.
/// Tolerante a markdown fences (modelos reales suelen envolver JSON en
/// ```json ... ```), a prosa alrededor, y a campos extra (los ignoramos).
pub fn traducir_propuesta_llm(texto: &str) -> InterpretacionLlm {
    let Some(json) = extraer_objeto_json(texto) else {
        return InterpretacionLlm::Error(format!("respuesta sin JSON: {texto}"));
    };
    // El rechazo se chequea primero porque tiene shape más estricto
    // (solo un campo `error`). Una propuesta con `error` además no
    // tiene sentido — el modelo está confundido y preferimos cazarlo
    // como rechazo.
    if let Ok(r) = serde_json::from_str::<RechazoLlm>(json) {
        return InterpretacionLlm::Rechazo(r.error);
    }
    let propuesta = match serde_json::from_str::<PropuestaLlm>(json) {
        Ok(p) => p,
        Err(e) => {
            return InterpretacionLlm::Error(format!("JSON no reconocido ({e}): {texto}"));
        }
    };
    // Si vino envuelto en markdown fences, marcamos menor confianza —
    // el modelo no siguió perfectamente las instrucciones. Para 1.0
    // exigimos JSON puro al principio del texto.
    let confianza = if texto.trim_start().starts_with('{') {
        1.0
    } else {
        0.8
    };
    match propuesta {
        PropuestaLlm::Lanzar {
            plantilla,
            explicacion,
        } => InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::LanzarApp { plantilla },
            explicacion,
            confianza,
        },
        PropuestaLlm::Notar { texto } => InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::Notar { texto },
            explicacion: String::new(),
            confianza,
        },
        PropuestaLlm::Instalar {
            manifiesto,
            explicacion,
        } => match hex_a_hash(&manifiesto) {
            Some(h) => InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::InstalarApp {
                    manifiesto_propuesto: h,
                },
                explicacion,
                confianza,
            },
            None => InterpretacionLlm::Error(format!(
                "hash de manifiesto invalido ({} chars): {manifiesto}",
                manifiesto.len()
            )),
        },
        PropuestaLlm::Configurar {
            config,
            explicacion,
        } => match hex_a_hash(&config) {
            Some(h) => InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::CambiarConfiguracion {
                    config_propuesta: h,
                },
                explicacion,
                confianza,
            },
            None => InterpretacionLlm::Error(format!(
                "hash de configuracion invalido ({} chars): {config}",
                config.len()
            )),
        },
    }
}

/// Construye el prompt user que se envía al LLM. Pega el contexto del
/// nodo wawa + el prompt humano. Pura: el llamante lo pasa a
/// `pluma_llm_core::ChatRequest::una_vuelta(...)`.
pub fn construir_prompt_usuario(ctx: &Contexto, prompt_humano: &str) -> String {
    let mut s = String::new();
    s.push_str("# Estado actual del nodo wawa\n\n");
    s.push_str(&format!("Apps en el manifiesto ({}):\n", ctx.apps.len()));
    for (i, nombre) in ctx.apps.iter().enumerate() {
        s.push_str(&format!("  [{i}] {nombre}\n"));
    }
    if let Some(h) = ctx.manifiesto_actual {
        s.push_str(&format!("Manifiesto vigente: {}\n", hex_de_hash(&h)));
    }
    if let Some(h) = ctx.configuracion_activa {
        s.push_str(&format!("Configuración activa: {}\n", hex_de_hash(&h)));
    }
    s.push_str("\n# Petición del operador\n\n");
    s.push_str(prompt_humano);
    s.push_str("\n\nResponde con el JSON exacto según las instrucciones del sistema.");
    s
}

// ---------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------

/// Encuentra el primer objeto JSON balanceado por `{` y `}` dentro de
/// `texto`. Tolerante a prosa y markdown fences alrededor.
fn extraer_objeto_json(texto: &str) -> Option<&str> {
    let bytes = texto.as_bytes();
    let inicio = texto.find('{')?;
    let mut prof: usize = 0;
    for (offset, &b) in bytes[inicio..].iter().enumerate() {
        match b {
            b'{' => prof += 1,
            b'}' => {
                prof -= 1;
                if prof == 0 {
                    return Some(&texto[inicio..=inicio + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Convierte una cadena de 64 chars hex a `Hash` (`[u8; 32]`). `None` si
/// el largo es distinto o algún carácter no es hex.
fn hex_a_hash(hex: &str) -> Option<Hash> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        match u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
            Ok(b) => out[i] = b,
            Err(_) => return None,
        }
    }
    Some(out)
}

/// Hex-encode de un `Hash` a 64 chars. Para volcar al prompt visible.
fn hex_de_hash(h: &Hash) -> String {
    let mut s = String::with_capacity(64);
    for b in h {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ---------------------------------------------------------------------
// Empaquetado wire — `AccionPropuesta` / `InterpretacionLlm` → bytes para
// el cable Akasha. Espejo simetrico del parser de la app `asistente.wasm`.
// ---------------------------------------------------------------------

/// Empaqueta una `InterpretacionLlm` en bytes listos para enviar por el
/// cable. Devuelve el (tipo, bytes) sin la cabecera; el caller agrega
/// la cabecera con [`format::escribir_cabecera_cable`].
///
/// Codificacion por variante (igual que en `apps/asistente/src/lib.rs`):
/// - `Notar(texto)`          → `(TipoCable::PropuestaNotar, texto_ascii)`
/// - `LanzarApp(plantilla)`  → `(TipoCable::PropuestaLanzarApp, u32 BE)`
/// - `InstalarApp(hash)`     → `(TipoCable::PropuestaInstalarApp, hash)`
/// - `CambiarConfig(hash)`   → `(TipoCable::PropuestaCambiarConfig, hash)`
/// - `Rechazo(motivo)`       → `(TipoCable::Error, motivo_ascii)`
/// - `Error(motivo)`         → `(TipoCable::Error, motivo_ascii)`
pub fn empaquetar_cable(interp: &InterpretacionLlm) -> (TipoCable, Vec<u8>) {
    match interp {
        InterpretacionLlm::Propuesta { accion, .. } => match accion {
            AccionPropuesta::Notar { texto } => {
                (TipoCable::PropuestaNotar, texto.as_bytes().to_vec())
            }
            AccionPropuesta::LanzarApp { plantilla } => (
                TipoCable::PropuestaLanzarApp,
                plantilla.to_be_bytes().to_vec(),
            ),
            AccionPropuesta::InstalarApp {
                manifiesto_propuesto,
            } => (
                TipoCable::PropuestaInstalarApp,
                manifiesto_propuesto.to_vec(),
            ),
            AccionPropuesta::CambiarConfiguracion { config_propuesta } => (
                TipoCable::PropuestaCambiarConfig,
                config_propuesta.to_vec(),
            ),
        },
        InterpretacionLlm::Rechazo(motivo) | InterpretacionLlm::Error(motivo) => {
            (TipoCable::Error, motivo.as_bytes().to_vec())
        }
    }
}

/// Construye un frame del cable completo (cabecera 12 B + payload) listo
/// para inyectar en AF_PACKET con la MAC destino que el caller decida.
/// El kernel pondra la cabecera Ethernet con la MAC origen segun la
/// interfaz a la que esta bindeado el socket.
pub fn construir_frame(id: u64, interp: &InterpretacionLlm) -> Vec<u8> {
    let (tipo, payload) = empaquetar_cable(interp);
    let mut frame = vec![0u8; TAM_CABECERA_CABLE + payload.len()];
    escribir_cabecera_cable(&mut frame, tipo, id).expect("cabe");
    frame[TAM_CABECERA_CABLE..].copy_from_slice(&payload);
    frame
}

/// Fase 60 v4 :: empaqueta una `Firma` Ed25519 ya autorizada por el
/// operador en un frame `TipoCable::Firma`. Wire: cabecera 12 B + 65 B
/// `[slot | firma]`. La app `asistente.wasm` lo decodifica espejado.
pub fn construir_frame_firma(id: u64, slot: u8, firma: &[u8; 64]) -> Vec<u8> {
    let mut frame = vec![0u8; TAM_CABECERA_CABLE + 65];
    escribir_cabecera_cable(&mut frame, TipoCable::Firma, id).expect("cabe");
    frame[TAM_CABECERA_CABLE] = slot;
    frame[TAM_CABECERA_CABLE + 1..].copy_from_slice(firma);
    frame
}

/// Fase 60 v4 :: empaqueta un `Error` con un motivo libre. Util para
/// rechazar `RequestFirma` cuando el puente no tiene clave configurada,
/// o cuando el tipo de objeto es desconocido.
pub fn construir_frame_error(id: u64, motivo: &str) -> Vec<u8> {
    let payload = motivo.as_bytes();
    let mut frame = vec![0u8; TAM_CABECERA_CABLE + payload.len()];
    escribir_cabecera_cable(&mut frame, TipoCable::Error, id).expect("cabe");
    frame[TAM_CABECERA_CABLE..].copy_from_slice(payload);
    frame
}

/// Fase 60 v4 :: parser del payload de `TipoCable::RequestFirma`. Devuelve
/// `(tipo_obj, hash)` o `None` si el payload no es de 33 bytes o el byte
/// de tipo es desconocido. El llamante usa `tipo_obj` para elegir el
/// prefijo legacy (`wawa::sign_request::` vs `wawa::sign_config::`) al
/// dialogar con `daemon-firma` — aqui solo separamos los campos.
pub fn leer_request_firma(payload: &[u8]) -> Option<(u8, [u8; 32])> {
    if payload.len() != 33 {
        return None;
    }
    let tipo_obj = payload[0];
    if tipo_obj != format::TIPO_OBJETO_CUADERNO
        && tipo_obj != format::TIPO_OBJETO_CONFIGURACION
    {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&payload[1..33]);
    Some((tipo_obj, hash))
}

// ---------------------------------------------------------------------
// Tests — lógica pura, sin red.
// ---------------------------------------------------------------------

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn traducir_lanzar_canonico() {
        let resp = r#"{"tipo": "lanzar", "plantilla": 7, "explicacion": "abre pluma"}"#;
        match traducir_propuesta_llm(resp) {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::LanzarApp { plantilla },
                explicacion,
                confianza,
            } => {
                assert_eq!(plantilla, 7);
                assert_eq!(explicacion, "abre pluma");
                assert_eq!(confianza, 1.0);
            }
            otro => panic!("esperaba Propuesta::LanzarApp, obtuve {otro:?}"),
        }
    }

    #[test]
    fn traducir_lanzar_con_markdown_fences_baja_confianza() {
        let resp = "Claro:\n```json\n{\"tipo\": \"lanzar\", \"plantilla\": 3}\n```";
        match traducir_propuesta_llm(resp) {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::LanzarApp { plantilla },
                confianza,
                ..
            } => {
                assert_eq!(plantilla, 3);
                assert!(confianza < 1.0, "fences bajan confianza");
            }
            otro => panic!("esperaba Propuesta::LanzarApp, obtuve {otro:?}"),
        }
    }

    #[test]
    fn traducir_notar_no_ejerce_accion() {
        let resp = r#"{"tipo": "notar", "texto": "tienes 12 apps"}"#;
        match traducir_propuesta_llm(resp) {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::Notar { texto },
                ..
            } => {
                assert_eq!(texto, "tienes 12 apps");
            }
            otro => panic!("esperaba Propuesta::Notar, obtuve {otro:?}"),
        }
    }

    #[test]
    fn traducir_instalar_con_hash_valido() {
        let hex = "ab".repeat(32); // 64 chars
        let resp = format!(
            r#"{{"tipo": "instalar", "manifiesto": "{hex}", "explicacion": "v2"}}"#
        );
        match traducir_propuesta_llm(&resp) {
            InterpretacionLlm::Propuesta {
                accion:
                    AccionPropuesta::InstalarApp {
                        manifiesto_propuesto,
                    },
                ..
            } => {
                assert_eq!(manifiesto_propuesto, [0xAB; 32]);
            }
            otro => panic!("esperaba Propuesta::InstalarApp, obtuve {otro:?}"),
        }
    }

    #[test]
    fn traducir_instalar_hash_invalido_es_error() {
        let resp = r#"{"tipo": "instalar", "manifiesto": "cafe", "explicacion": "corto"}"#;
        assert!(matches!(
            traducir_propuesta_llm(resp),
            InterpretacionLlm::Error(_),
        ));
    }

    #[test]
    fn traducir_configurar_con_hash_valido() {
        let hex = "cd".repeat(32);
        let resp = format!(r#"{{"tipo": "configurar", "config": "{hex}"}}"#);
        match traducir_propuesta_llm(&resp) {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::CambiarConfiguracion { config_propuesta },
                ..
            } => {
                assert_eq!(config_propuesta, [0xCD; 32]);
            }
            otro => panic!("esperaba Propuesta::CambiarConfiguracion, obtuve {otro:?}"),
        }
    }

    #[test]
    fn traducir_rechazo_explicito() {
        let resp = r#"{"error": "no entendi"}"#;
        assert_eq!(
            traducir_propuesta_llm(resp),
            InterpretacionLlm::Rechazo("no entendi".to_string()),
        );
    }

    #[test]
    fn traducir_tipo_desconocido_es_error() {
        // El modelo inventó un tipo. Como `PropuestaLlm` es estricto
        // (variantes lowercase), esto cae como JSON no reconocido.
        let resp = r#"{"tipo": "explotar", "args": []}"#;
        assert!(matches!(
            traducir_propuesta_llm(resp),
            InterpretacionLlm::Error(_),
        ));
    }

    #[test]
    fn traducir_sin_json_es_error() {
        assert!(matches!(
            traducir_propuesta_llm("hola, no se que decirte"),
            InterpretacionLlm::Error(_),
        ));
    }

    #[test]
    fn construir_prompt_incluye_apps_y_peticion() {
        let ctx = Contexto {
            apps: vec!["pluma".into(), "bitacora".into()],
            manifiesto_actual: Some([0x11; 32]),
            configuracion_activa: None,
        };
        let prompt = construir_prompt_usuario(&ctx, "abre pluma");
        assert!(prompt.contains("[0] pluma"));
        assert!(prompt.contains("[1] bitacora"));
        assert!(prompt.contains("abre pluma"));
        assert!(prompt.contains("Manifiesto vigente"));
        // Sin configuracion: el bloque no aparece.
        assert!(!prompt.contains("Configuración activa"));
    }

    #[test]
    fn extraer_json_balanceado() {
        let texto = r#"prosa {"a": {"b": 2}} mas prosa"#;
        assert_eq!(extraer_objeto_json(texto), Some(r#"{"a": {"b": 2}}"#));
    }

    #[test]
    fn hex_round_trip() {
        let h: Hash = [0xAB; 32];
        let s = hex_de_hash(&h);
        assert_eq!(s.len(), 64);
        assert_eq!(hex_a_hash(&s), Some(h));
    }

    #[test]
    fn empaquetar_notar_lleva_texto_ascii() {
        let interp = InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::Notar {
                texto: "hola".into(),
            },
            explicacion: "".into(),
            confianza: 1.0,
        };
        let (tipo, payload) = empaquetar_cable(&interp);
        assert_eq!(tipo, TipoCable::PropuestaNotar);
        assert_eq!(payload, b"hola");
    }

    #[test]
    fn empaquetar_lanzar_lleva_u32_be() {
        let interp = InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::LanzarApp { plantilla: 0x01020304 },
            explicacion: "".into(),
            confianza: 1.0,
        };
        let (tipo, payload) = empaquetar_cable(&interp);
        assert_eq!(tipo, TipoCable::PropuestaLanzarApp);
        assert_eq!(payload, vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn empaquetar_instalar_lleva_hash_32B() {
        let interp = InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::InstalarApp {
                manifiesto_propuesto: [0xAB; 32],
            },
            explicacion: "".into(),
            confianza: 1.0,
        };
        let (tipo, payload) = empaquetar_cable(&interp);
        assert_eq!(tipo, TipoCable::PropuestaInstalarApp);
        assert_eq!(payload.len(), 32);
        assert!(payload.iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn empaquetar_rechazo_es_tipo_error() {
        let interp = InterpretacionLlm::Rechazo("no se".into());
        let (tipo, payload) = empaquetar_cable(&interp);
        assert_eq!(tipo, TipoCable::Error);
        assert_eq!(payload, b"no se");
    }

    #[test]
    fn empaquetar_error_es_tipo_error() {
        let interp = InterpretacionLlm::Error("JSON malformado".into());
        let (tipo, payload) = empaquetar_cable(&interp);
        assert_eq!(tipo, TipoCable::Error);
        assert_eq!(payload, b"JSON malformado");
    }

    #[test]
    fn construir_frame_es_decodificable_por_format() {
        // El frame que escribimos tiene que ser legible por
        // `format::leer_cabecera_cable` — espejo del parser que vive
        // en `apps/asistente`.
        let interp = InterpretacionLlm::Propuesta {
            accion: AccionPropuesta::LanzarApp { plantilla: 5 },
            explicacion: "abrir pluma".into(),
            confianza: 1.0,
        };
        let frame = construir_frame(42, &interp);
        let (tipo, id) = format::leer_cabecera_cable(&frame).expect("decodifica");
        assert_eq!(tipo, TipoCable::PropuestaLanzarApp);
        assert_eq!(id, 42);
        assert_eq!(&frame[TAM_CABECERA_CABLE..], &5u32.to_be_bytes());
    }

    // ---------------------------------------------------------------
    // Fase 60 v4 :: helpers de RequestFirma / Firma
    // ---------------------------------------------------------------

    #[test]
    fn construir_frame_firma_round_trip() {
        let firma = [0xAB; 64];
        let frame = construir_frame_firma(123, 1, &firma);
        let (tipo, id) = format::leer_cabecera_cable(&frame).expect("cabecera valida");
        assert_eq!(tipo, TipoCable::Firma);
        assert_eq!(id, 123);
        assert_eq!(frame[TAM_CABECERA_CABLE], 1, "slot");
        assert_eq!(&frame[TAM_CABECERA_CABLE + 1..], &firma);
        assert_eq!(frame.len(), TAM_CABECERA_CABLE + 65);
    }

    #[test]
    fn construir_frame_error_lleva_motivo_ascii() {
        let frame = construir_frame_error(7, "sin clave");
        let (tipo, id) = format::leer_cabecera_cable(&frame).expect("cabecera valida");
        assert_eq!(tipo, TipoCable::Error);
        assert_eq!(id, 7);
        assert_eq!(&frame[TAM_CABECERA_CABLE..], b"sin clave");
    }

    #[test]
    fn leer_request_firma_acepta_cuaderno_y_config() {
        // tipo_obj = 1 (cuaderno)
        let mut payload = [0u8; 33];
        payload[0] = format::TIPO_OBJETO_CUADERNO;
        for i in 0..32 {
            payload[1 + i] = (i as u8).wrapping_add(0x10);
        }
        let (tipo, hash) = leer_request_firma(&payload).expect("33 B + tipo conocido");
        assert_eq!(tipo, format::TIPO_OBJETO_CUADERNO);
        assert_eq!(hash[0], 0x10);
        assert_eq!(hash[31], 0x10 + 31);

        // tipo_obj = 2 (configuracion)
        payload[0] = format::TIPO_OBJETO_CONFIGURACION;
        let (tipo, _) = leer_request_firma(&payload).expect("acepta config");
        assert_eq!(tipo, format::TIPO_OBJETO_CONFIGURACION);
    }

    #[test]
    fn leer_request_firma_rechaza_largos_y_tipos_ajenos() {
        // Largo distinto.
        assert!(leer_request_firma(&[1u8; 32]).is_none());
        assert!(leer_request_firma(&[1u8; 34]).is_none());
        // Tipo de objeto desconocido — defensivo contra cable basura.
        let mut payload = [0u8; 33];
        payload[0] = 99;
        assert!(leer_request_firma(&payload).is_none());
    }
}

// ---------------------------------------------------------------------
// Tests de integración: Consulta → MockChatClient → Propuesta.
// Validan el contrato completo del puente sin red ni grafo.
// ---------------------------------------------------------------------

#[cfg(test)]
mod integracion {
    use super::*;
    use pluma_llm_core::{ChatClient, ChatRequest};
    use pluma_llm_mock::MockChatClient;

    fn ctx_demo() -> Contexto {
        Contexto {
            apps: vec!["pluma".into(), "bitacora".into(), "tonada".into()],
            manifiesto_actual: Some([0x11; 32]),
            configuracion_activa: None,
        }
    }

    /// Helper: simula el flujo completo del puente sobre un mock.
    fn flujo(mock: &MockChatClient, prompt: &str) -> InterpretacionLlm {
        let ctx = ctx_demo();
        let user = construir_prompt_usuario(&ctx, prompt);
        let req = ChatRequest::una_vuelta(user, 500).con_sistema(PROMPT_SISTEMA_WAWA);
        let resp = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio rt")
            .block_on(mock.complete(&req))
            .expect("mock no falla");
        traducir_propuesta_llm(&resp.content)
    }

    #[test]
    fn flujo_lanzar_pluma_indice_correcto() {
        // El mock responde con el índice 0 (pluma); el puente lo
        // mapea a `LanzarApp { plantilla: 0 }`.
        let mock = MockChatClient::default().con_respuesta(
            "abre pluma",
            r#"{"tipo": "lanzar", "plantilla": 0, "explicacion": "es la primera"}"#,
        );
        match flujo(&mock, "abre pluma para tomar notas") {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::LanzarApp { plantilla },
                ..
            } => assert_eq!(plantilla, 0),
            otro => panic!("esperaba LanzarApp, obtuve {otro:?}"),
        }
    }

    #[test]
    fn flujo_notar_responde_pregunta() {
        let mock = MockChatClient::default().con_respuesta(
            "cuantas apps",
            r#"{"tipo": "notar", "texto": "tienes 3 apps: pluma, bitacora, tonada"}"#,
        );
        match flujo(&mock, "cuantas apps tengo?") {
            InterpretacionLlm::Propuesta {
                accion: AccionPropuesta::Notar { texto },
                ..
            } => {
                assert!(texto.contains("3 apps"));
                assert!(texto.contains("pluma"));
            }
            otro => panic!("esperaba Notar, obtuve {otro:?}"),
        }
    }

    #[test]
    fn flujo_modelo_se_niega() {
        let mock = MockChatClient::default().con_respuesta(
            "destruir",
            r#"{"error": "no destruyo cosas a mansalva"}"#,
        );
        match flujo(&mock, "destruir todo") {
            InterpretacionLlm::Rechazo(motivo) => assert!(motivo.contains("destruyo")),
            otro => panic!("esperaba Rechazo, obtuve {otro:?}"),
        }
    }

    #[test]
    fn flujo_modelo_responde_con_basura_genera_error() {
        let mock = MockChatClient::default().con_respuesta(
            "vacio",
            "Lo siento, hoy mi modelo está mareado.",
        );
        assert!(matches!(
            flujo(&mock, "vacio"),
            InterpretacionLlm::Error(_),
        ));
    }

    #[test]
    fn flujo_instalar_con_hash_de_la_red() {
        // Caso realista: el operador habilita un canal y el modelo
        // sugiere instalar el manifiesto que vino por él. El hash es
        // dato del puente (en este test, fingido).
        let hex = "fe".repeat(32);
        let mock = MockChatClient::default().con_respuesta(
            "instalar",
            &format!(
                r#"{{"tipo": "instalar", "manifiesto": "{hex}", "explicacion": "v3 del canal"}}"#
            ),
        );
        match flujo(&mock, "instalar la version nueva") {
            InterpretacionLlm::Propuesta {
                accion:
                    AccionPropuesta::InstalarApp {
                        manifiesto_propuesto,
                    },
                explicacion,
                ..
            } => {
                assert_eq!(manifiesto_propuesto, [0xFE; 32]);
                assert!(explicacion.contains("v3"));
            }
            otro => panic!("esperaba InstalarApp, obtuve {otro:?}"),
        }
    }
}
