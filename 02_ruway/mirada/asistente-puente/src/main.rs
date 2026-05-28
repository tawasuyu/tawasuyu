//! `asistente-puente` — binario scaffolding (stdio-based, sin Akasha real).
//!
//! Lee un `MensajeAsistente::Consulta` codificado en postcard binario por
//! stdin (precedido por un `u32 LE` con la longitud del frame), llama al
//! LLM via `pluma-llm`, traduce la respuesta a una `AccionPropuesta`, y
//! emite un `MensajeAsistente::Propuesta` (o `Error`) por stdout en el
//! mismo formato.
//!
//! El loop es uno-a-uno: una consulta entra, una respuesta sale. Eso es
//! suficiente para probar el contrato end-to-end con un test runner o un
//! humano que use `printf` y `xxd`. El socket raw Akasha (que tiene
//! multiplexación, broadcast, dedup) viene en una vuelta posterior; el
//! contrato del payload (postcard sobre `MensajeAsistente`) ya queda
//! estable.
//!
//! Uso:
//!
//! ```text
//! cat consulta.bin | asistente-puente > respuesta.bin
//! ```
//!
//! Sin credenciales de LLM, cae al backend Mock (pluma-llm) y siempre
//! responde con `Notar { texto: "(mock) ..." }` — útil para tests sin
//! tokens.

use std::io::{self, Read, Write};

use asistente_puente::{
    construir_prompt_usuario, traducir_propuesta_llm, InterpretacionLlm, PROMPT_SISTEMA_WAWA,
};
use format::MensajeAsistente;
use pluma_llm_core::ChatRequest;

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
    // 1. Leer un frame con prefijo de longitud.
    let mut stdin = io::stdin().lock();
    let mut long_buf = [0u8; 4];
    stdin
        .read_exact(&mut long_buf)
        .map_err(|e| format!("leyendo prefijo de longitud: {e}"))?;
    let largo = u32::from_le_bytes(long_buf) as usize;
    if largo == 0 || largo > MAX_FRAME {
        return Err(format!("longitud de frame fuera de rango: {largo}"));
    }
    let mut payload = vec![0u8; largo];
    stdin
        .read_exact(&mut payload)
        .map_err(|e| format!("leyendo payload de {largo} B: {e}"))?;

    // 2. Decodificar el MensajeAsistente. Tiene que ser una Consulta.
    let mensaje = MensajeAsistente::deserializar(&payload)
        .map_err(|e| format!("deserializando MensajeAsistente: {e}"))?;
    let (id, prompt, contexto) = match mensaje {
        MensajeAsistente::Consulta {
            id,
            prompt,
            contexto,
        } => (id, prompt, contexto),
        otro => return Err(format!("esperaba Consulta, recibí {otro:?}")),
    };

    // 3. Construir el ChatRequest y consultar al LLM dentro de un runtime
    //    Tokio current-thread (el binario es sync; el LLM es async).
    let llm = pluma_llm::from_env().map_err(|e| format!("inicializando pluma-llm: {e}"))?;
    let prompt_user = construir_prompt_usuario(&contexto, &prompt);
    let req = ChatRequest::una_vuelta(prompt_user, MAX_TOKENS_RESPUESTA)
        .con_sistema(PROMPT_SISTEMA_WAWA);

    let resp = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("creando tokio runtime: {e}"))?
        .block_on(llm.complete(&req));

    // 4. Mapear el resultado a un MensajeAsistente de salida.
    let salida = match resp {
        Ok(r) => match traducir_propuesta_llm(&r.content) {
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
    };

    // 5. Serializar y escribir con prefijo de longitud.
    let bytes = salida
        .serializar()
        .map_err(|e| format!("serializando salida: {e}"))?;
    let largo = bytes.len() as u32;
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(&largo.to_le_bytes())
        .map_err(|e| format!("escribiendo prefijo: {e}"))?;
    stdout
        .write_all(&bytes)
        .map_err(|e| format!("escribiendo payload: {e}"))?;
    stdout.flush().map_err(|e| format!("flush stdout: {e}"))?;
    Ok(())
}
