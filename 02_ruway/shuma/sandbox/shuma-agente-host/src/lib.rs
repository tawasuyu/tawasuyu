//! # shuma-agente-host — corre un turno de conversación
//!
//! El núcleo [`shuma_agente`] es sync y sin red: arma el `ChatRequest` e
//! interpreta la respuesta, pero no habla con ningún backend. Acá vive ese
//! pegamento: resolver el backend (propio del agente, o el `[ai.llm]` global del
//! SO como fallback, o `from_env`), correr `pluma-llm` en un runtime efímero, y
//! devolver los [`BloqueSalida`] ya interpretados.
//!
//! Es **bloqueante** a propósito: el host lo llama en un thread aparte
//! (`Handle::spawn`), igual que el `run_llm_blocking` del shell — el bucle Elm
//! nunca se cuelga esperando la red.

use shuma_agente::{motor, Agente, BloqueSalida, Conversacion};

/// El desenlace de un turno: los bloques interpretados y, si el backend lo
/// reporta, el conteo de tokens (para mostrar costo en la UI).
#[derive(Debug, Clone)]
pub struct Respuesta {
    pub bloques: Vec<BloqueSalida>,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Corre un turno: toma la conversación (con el último mensaje del usuario ya
/// agregado) + el agente + el backend global de fallback, y devuelve la
/// respuesta interpretada. Bloqueante.
///
/// Resolución de backend: si `agente.backend` fija uno, se usa ese; si no, el
/// `fallback_global` (típicamente `WawaConfig::load().ai.llm`); si tampoco está
/// fijo, `pluma-llm::from_env` (Mock si no hay credenciales — nunca cuelga).
pub fn responder(
    conv: &Conversacion,
    agente: &Agente,
    fallback_global: &wawa_config::LlmSettings,
) -> Result<Respuesta, String> {
    responder_streaming(conv, agente, fallback_global, |_| {})
}

/// Como [`responder`] pero **emitiendo la salida a medida que llega**: `on_delta`
/// se llama con cada fragmento de texto. Útil para que la UI pinte la respuesta
/// progresiva (paridad con Claude CLI). Devuelve la respuesta final interpretada.
///
/// Sólo es incremental si el backend soporta streaming (hoy `claude-cli`); el
/// resto cae al default no-incremental del trait (emite todo al final).
pub fn responder_streaming(
    conv: &Conversacion,
    agente: &Agente,
    fallback_global: &wawa_config::LlmSettings,
    mut on_delta: impl FnMut(&str) + Send,
) -> Result<Respuesta, String> {
    use pluma_llm::pluma_llm_core::ChatClient;

    let req = motor::construir_request(conv, agente);
    let backend = if agente.backend.is_set() {
        &agente.backend
    } else {
        fallback_global
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?;

    let resp = rt.block_on(async {
        let client: std::sync::Arc<dyn ChatClient> =
            build_client(backend).map_err(|e| format!("sin backend LLM: {e}"))?;
        client.stream(&req, &mut on_delta).await.map_err(|e| e.to_string())
    })?;

    let bloques = motor::interpretar_respuesta(&resp.content, agente);
    let (input_tokens, output_tokens) = resp
        .usage
        .map(|u| (u.input_tokens, u.output_tokens))
        .unwrap_or((0, 0));
    Ok(Respuesta { bloques, input_tokens, output_tokens })
}

/// Traduce los `LlmSettings` planos al `LlmConfig` de pluma-llm y construye el
/// cliente. Idéntico criterio que el `build_llm_client` del shell — duplicado
/// mínimo a propósito (no vale acoplar shell-llimphi y este crate por una fn).
fn build_client(
    s: &wawa_config::LlmSettings,
) -> Result<std::sync::Arc<dyn pluma_llm::pluma_llm_core::ChatClient>, String> {
    use pluma_llm::{build_client, BackendKind, LlmConfig};
    if !s.is_set() {
        return pluma_llm::from_env().map_err(|e| e.to_string());
    }
    let kind = match s.backend.trim().to_lowercase().as_str() {
        "anthropic" => BackendKind::Anthropic,
        "gemini" => BackendKind::Gemini,
        "deepseek" => BackendKind::DeepSeek,
        "cohere" => BackendKind::Cohere,
        "ollama" => BackendKind::Ollama,
        "claude-cli" | "claude-code" => BackendKind::ClaudeCli,
        "mock" => BackendKind::Mock,
        other => return Err(format!("backend LLM desconocido: «{other}»")),
    };
    let none_if_empty = |v: &str| {
        let v = v.trim();
        (!v.is_empty()).then(|| v.to_string())
    };
    let cfg = LlmConfig {
        kind,
        model: none_if_empty(&s.model),
        api_key: none_if_empty(&s.api_key),
        endpoint: none_if_empty(&s.endpoint),
    };
    build_client(&cfg).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip completo contra el backend Mock (sin red ni credenciales):
    /// usuario pregunta → el host corre pluma-llm → la respuesta se interpreta
    /// en bloques. Prueba que el contrato núcleo↔host cierra de punta a punta.
    #[test]
    fn round_trip_con_mock() {
        let mut backend = wawa_config::LlmSettings::default();
        backend.backend = "mock".into();
        let agente = Agente::nuevo("Asistente").con_backend(backend);

        let mut conv = Conversacion::nueva(&agente.id, 0);
        conv.agregar_usuario("hola, ¿cómo estás?", 1);

        let global = wawa_config::LlmSettings::default();
        let resp = responder(&conv, &agente, &global).expect("mock no debería fallar");
        assert!(!resp.bloques.is_empty(), "el mock siempre devuelve algo");
    }
}
