//! El asistente LLM del binario `paloma` (Eje 2: correo LLM-nativo).
//!
//! Implementa el trait `LlmAssistant` de `paloma-llimphi` sobre la fachada
//! `pluma-llm`: resume el hilo abierto y redacta borradores de respuesta. El
//! trabajo async (la llamada al modelo) corre en un runtime tokio propio y el
//! resultado vuelve al bucle de UI por `Handle::dispatch`.
//!
//! Backend por entorno (`PLUMA_LLM_BACKEND` o autodetección por API key, ver
//! `pluma-llm`). **Local-first**: con Ollama (`PLUMA_LLM_BACKEND=ollama` +
//! `PLUMA_LLM_MODEL=...`) el correo no sale de la máquina. Sin un backend real
//! (y sin opt-in explícito) el asistente **no se engancha** y los botones ✨ no
//! aparecen — no queremos resúmenes de un mock que sólo hace eco.

use std::sync::Arc;

use paloma_llimphi::{Handle, LlmAssistant, Msg};
use pluma_llm::pluma_llm_core::{ChatClient, ChatRequest};
use tokio::runtime::Runtime;

/// Cota de tokens de salida para cada tarea.
const MAX_TOKENS_SUMMARY: u32 = 600;
const MAX_TOKENS_DRAFT: u32 = 800;
/// Tope de caracteres del hilo que se le manda al modelo (acota costo/latencia;
/// los hilos enormes igual quedan bien cubiertos por el principio).
const MAX_CONTEXT_CHARS: usize = 12_000;

pub struct LlmHelper {
    rt: Runtime,
    client: Arc<dyn ChatClient>,
}

impl LlmHelper {
    /// Construye el asistente si hay un backend LLM **real** disponible (o si se
    /// forzó uno con `PLUMA_LLM_BACKEND`, p. ej. `mock` para dev). `None` cuando
    /// `from_env` cae al mock sin opt-in → la UI no muestra los botones ✨.
    pub fn try_build() -> Option<Self> {
        let explicit = std::env::var("PLUMA_LLM_BACKEND").is_ok();
        let client = pluma_llm::from_env().ok()?;
        if client.model_id() == "pluma-llm-mock" && !explicit {
            return None;
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok()?;
        eprintln!("paloma · asistente LLM: {}", client.model_id());
        Some(Self { rt, client })
    }

    /// Lanza una completion async y despacha el texto (envuelto con `wrap`) o un
    /// `Msg::LlmError`. Los locks/sin estado compartido: sólo clona el `Arc`.
    fn run(
        &self,
        system: &str,
        user: String,
        max_tokens: u32,
        handle: Handle<Msg>,
        wrap: fn(String) -> Msg,
    ) {
        let client = self.client.clone();
        let system = system.to_string();
        self.rt.spawn(async move {
            let req = ChatRequest::una_vuelta(user, max_tokens)
                .con_sistema(system)
                .con_temperatura(0.3);
            match client.complete(&req).await {
                Ok(resp) => handle.dispatch(wrap(resp.content.trim().to_string())),
                Err(e) => handle.dispatch(Msg::LlmError(format!("IA: {e}"))),
            }
        });
    }
}

impl LlmAssistant for LlmHelper {
    fn summarize(&self, thread_text: String, handle: Handle<Msg>) {
        let system = "Sos un asistente de correo. Resumí el hilo en el mismo idioma del \
                      hilo, en 3-5 viñetas concisas: de qué trata, qué se decidió y qué \
                      queda pendiente. Si hay acciones o fechas, listalas explícitamente. \
                      No inventes nada que no esté en el texto.";
        let user = format!("Resumí este hilo de correo:\n\n{}", truncar(&thread_text));
        self.run(system, user, MAX_TOKENS_SUMMARY, handle, Msg::LlmSummary);
    }

    fn draft_reply(&self, thread_text: String, handle: Handle<Msg>) {
        let system = "Sos un asistente de correo. Redactá SÓLO el cuerpo de una respuesta \
                      al último mensaje del hilo, en el mismo idioma del hilo. Tono cordial \
                      y profesional, directo, sin firma ni asunto. No inventes datos que no \
                      estén en el hilo; si falta info, dejá un hueco entre corchetes.";
        let user = format!("Hilo al que respondo:\n\n{}", truncar(&thread_text));
        self.run(system, user, MAX_TOKENS_DRAFT, handle, Msg::LlmDraft);
    }
}

/// Acota el contexto a [`MAX_CONTEXT_CHARS`] caracteres (corta por el final, que
/// suele ser el citado más viejo y menos relevante para responder/resumir).
fn truncar(s: &str) -> String {
    if s.chars().count() <= MAX_CONTEXT_CHARS {
        return s.to_string();
    }
    s.chars().take(MAX_CONTEXT_CHARS).collect::<String>() + "\n[…hilo recortado…]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_llm::{build_client, BackendKind, LlmConfig};

    fn rt() -> Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }

    #[test]
    fn truncar_acota_el_contexto() {
        let largo = "a".repeat(MAX_CONTEXT_CHARS + 500);
        let t = truncar(&largo);
        assert!(t.ends_with("recortado…]"));
        assert!(t.chars().count() <= MAX_CONTEXT_CHARS + 40);
        assert_eq!(truncar("corto"), "corto");
    }

    /// Certifica el camino de la request (build → ChatRequest con system+temp →
    /// complete → respuesta) contra el cliente mock, sin levantar la UI. El
    /// despacho por `Handle` es el mismo patrón ya probado en `semantic`.
    #[test]
    fn el_request_de_resumen_llega_al_cliente_y_responde() {
        let client =
            build_client(&LlmConfig { kind: BackendKind::Mock, ..Default::default() }).unwrap();
        let thread = "Asunto: factura\n\nDe: ana\nel pago vence el viernes\n\n";
        let req = ChatRequest::una_vuelta(
            format!("Resumí este hilo de correo:\n\n{}", truncar(thread)),
            MAX_TOKENS_SUMMARY,
        )
        .con_sistema("resumí el hilo")
        .con_temperatura(0.3);
        let resp = rt().block_on(client.complete(&req)).unwrap();
        // El mock hace eco del prompt → el "resumen" contiene el hilo original.
        assert!(resp.content.contains("el pago vence el viernes"));
    }
}
