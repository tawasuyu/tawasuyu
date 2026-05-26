//! `pluma-llm-openai-compatible` — adapter contra cualquier servicio que
//! hable la shape OpenAI Chat Completions.
//!
//! La mayoría de los proveedores que NO son Anthropic ni Gemini hablan
//! esta forma: DeepSeek, Groq, Together, vLLM self-hosted, Ollama en
//! modo `/v1/chat/completions`. Una sola implementación los cubre todos
//! por configuración de endpoint + api_key + modelo.
//!
//! ## Presets
//!
//! ```no_run
//! # use pluma_llm_openai_compatible::OpenAiCompatibleClient;
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // DeepSeek remoto (lee DEEPSEEK_API_KEY).
//! let cli = OpenAiCompatibleClient::deepseek_from_env()?;
//!
//! // Ollama corriendo en localhost — sin auth.
//! let cli = OpenAiCompatibleClient::ollama_local("llama3.1");
//!
//! // Custom: cualquier endpoint + opcionalmente sin auth (local) o con bearer.
//! let cli = OpenAiCompatibleClient::custom("http://10.0.0.5:8000/v1/chat/completions", None, "qwen2.5");
//! # Ok(()) }
//! ```
//!
//! ## Shape del wire
//!
//! Request:
//! ```json
//! { "model": "...", "messages": [{"role":"system","content":"..."},
//!   {"role":"user","content":"..."}], "max_tokens": 1024, "temperature": 0.2 }
//! ```
//!
//! Response:
//! ```json
//! { "choices": [{"message":{"role":"assistant","content":"..."},
//!   "finish_reason":"stop"}], "usage":{"prompt_tokens":N,"completion_tokens":M} }
//! ```
//!
//! El system prompt va como primer `ChatMessage` con role=`system` —
//! distinto de Anthropic donde es un campo top-level. La conversión la
//! hace `construir_payload`.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_llm_core::{
    ChatClient, ChatError, ChatRequest, ChatResponse, ChatUsage, Role, StopReason,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::time::Duration;

const TIMEOUT_DEFAULT_SECS: u64 = 60;

// Endpoints + variables de entorno conocidos.
const DEEPSEEK_ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
const DEEPSEEK_ENV: &str = "DEEPSEEK_API_KEY";
const DEEPSEEK_MODEL_DEFAULT: &str = "deepseek-chat";
const OLLAMA_ENDPOINT_DEFAULT: &str = "http://localhost:11434/v1/chat/completions";

/// Cliente para cualquier endpoint OpenAI-compatible. `api_key` es
/// `Option<String>` porque algunos servicios locales (Ollama) no la
/// piden — un `None` simplemente omite el header `Authorization`.
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: Option<String>,
    model: String,
}

impl OpenAiCompatibleClient {
    /// Constructor general: endpoint + api_key opcional + modelo.
    pub fn custom(
        endpoint: impl Into<String>,
        api_key: Option<String>,
        model: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_DEFAULT_SECS))
            .build()
            .expect("reqwest client");
        Self {
            http,
            endpoint: endpoint.into(),
            api_key,
            model: model.into(),
        }
    }

    /// Preset: DeepSeek con `DEEPSEEK_API_KEY` y modelo `deepseek-chat`.
    /// Para `deepseek-reasoner` u otro, encadenar `.with_model(...)`.
    pub fn deepseek_from_env() -> Result<Self, ChatError> {
        let api_key = std::env::var(DEEPSEEK_ENV)
            .map_err(|_| ChatError::AuthMissing(DEEPSEEK_ENV.to_string()))?;
        Ok(Self::custom(DEEPSEEK_ENDPOINT, Some(api_key), DEEPSEEK_MODEL_DEFAULT))
    }

    /// Preset: Ollama en localhost, modo OpenAI-compatible. No requiere
    /// API key. El `model` debe estar pulled previamente
    /// (`ollama pull llama3.1`). Para Ollama en otra máquina o puerto,
    /// usar `custom` con la URL completa.
    pub fn ollama_local(model: impl Into<String>) -> Self {
        Self::custom(OLLAMA_ENDPOINT_DEFAULT, None, model)
    }

    /// Encadenable: cambia el modelo.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Encadenable: ajusta el timeout HTTP. Útil para modelos locales
    /// grandes (Ollama con llama3.1 70B puede tardar varios minutos por
    /// request en CPU).
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.http = reqwest::Client::builder()
            .timeout(t)
            .build()
            .expect("reqwest client");
        self
    }

    fn headers(&self) -> Result<HeaderMap, ChatError> {
        let mut h = HeaderMap::new();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        if let Some(key) = &self.api_key {
            let val = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|_| ChatError::Backend("api key con bytes inválidos".to_string()))?;
            h.insert("authorization", val);
        }
        Ok(h)
    }
}

#[async_trait]
impl ChatClient for OpenAiCompatibleClient {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError> {
        let payload = construir_payload(req, &self.model);
        let resp = self
            .http
            .post(&self.endpoint)
            .headers(self.headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChatError::Network(format!("POST chat/completions: {e}")))?;

        let status = resp.status();
        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| ChatError::Network(format!("leer body: {e}")))?;

        if status == 401 || status == 403 {
            return Err(ChatError::AuthInvalid);
        }
        if status == 429 {
            return Err(ChatError::RateLimited);
        }
        if !status.is_success() {
            // Algunos servicios devuelven `{"error":{"message":...}}`;
            // otros texto plano. Probamos JSON primero, caemos a string.
            let mensaje = match serde_json::from_slice::<OpenAiErrorEnvelope>(&body_bytes) {
                Ok(env) => env.error.message,
                Err(_) => String::from_utf8_lossy(&body_bytes).into_owned(),
            };
            return Err(ChatError::Backend(format!("HTTP {status}: {mensaje}")));
        }

        let parsed: OpenAiChatResponse = serde_json::from_slice(&body_bytes)
            .map_err(|e| ChatError::Backend(format!("parseo response: {e}")))?;

        // Concatenar el contenido del primer choice. Servicios bien
        // comportados devuelven 1 choice; si vinieran más, los ignoramos
        // (el contrato de `ChatResponse` es UNA respuesta).
        let primer = parsed.choices.into_iter().next().ok_or_else(|| {
            ChatError::Backend("response sin choices".to_string())
        })?;
        let content = primer.message.content.unwrap_or_default();
        let stop_reason = primer.finish_reason.map(StopReason);

        // Algunos servicios (DeepSeek) reportan tokens cacheados en
        // `prompt_tokens_details.cached_tokens` o `prompt_cache_hit_tokens`.
        // Aceptamos ambos campos por compat.
        let usage = parsed.usage.map(|u| {
            let cached = u
                .prompt_cache_hit_tokens
                .or(u.prompt_tokens_details.and_then(|d| d.cached_tokens))
                .unwrap_or(0);
            ChatUsage {
                input_tokens: u.prompt_tokens.unwrap_or(0),
                output_tokens: u.completion_tokens.unwrap_or(0),
                cache_read_input_tokens: cached,
                // OpenAI-shape no expone cache_creation separado.
                cache_creation_input_tokens: 0,
            }
        });

        Ok(ChatResponse {
            content,
            stop_reason,
            usage,
        })
    }
}

/// Traduce un `ChatRequest` a la shape OpenAI. El `system` se pone como
/// primer mensaje con role=system; el orden user/assistant se conserva
/// tal cual.
fn construir_payload(req: &ChatRequest, modelo: &str) -> serde_json::Value {
    let mut mensajes: Vec<serde_json::Value> = Vec::with_capacity(req.messages.len() + 1);
    if let Some(sys) = &req.system {
        mensajes.push(serde_json::json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        let role = match m.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        mensajes.push(serde_json::json!({"role": role, "content": m.content}));
    }
    serde_json::json!({
        "model": modelo,
        "messages": mensajes,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
    })
}

// -------- Tipos del wire OpenAI-compatible --------

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessageOut,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessageOut {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u32>,
    #[serde(default)]
    completion_tokens: Option<u32>,
    /// Campo de DeepSeek y otros que reportan caching del input directo.
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
    /// Forma alternativa (OpenAI moderna, vLLM): `prompt_tokens_details.cached_tokens`.
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorEnvelope {
    error: OpenAiErrorBody,
}

#[derive(Debug, Deserialize)]
struct OpenAiErrorBody {
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_core::ChatMessage;

    #[test]
    fn payload_sin_system_solo_pone_user() {
        let req = ChatRequest::una_vuelta("hola", 50);
        let p = construir_payload(&req, "deepseek-chat");
        assert_eq!(p["model"], "deepseek-chat");
        assert_eq!(p["messages"].as_array().unwrap().len(), 1);
        assert_eq!(p["messages"][0]["role"], "user");
        assert_eq!(p["messages"][0]["content"], "hola");
    }

    #[test]
    fn payload_con_system_inserta_primer_mensaje_system() {
        let req = ChatRequest::una_vuelta("hola", 50)
            .con_sistema("Eres un asistente.");
        let p = construir_payload(&req, "deepseek-chat");
        let msgs = p["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "Eres un asistente.");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn presets_construyen_endpoints_y_modelos_esperados() {
        let ollama = OpenAiCompatibleClient::ollama_local("llama3.1");
        assert_eq!(ollama.endpoint, OLLAMA_ENDPOINT_DEFAULT);
        assert_eq!(ollama.model, "llama3.1");
        assert!(ollama.api_key.is_none());

        let custom = OpenAiCompatibleClient::custom(
            "http://x/v1/chat/completions",
            Some("k".into()),
            "qwen2.5",
        );
        assert_eq!(custom.model, "qwen2.5");
        assert_eq!(custom.api_key.as_deref(), Some("k"));
    }

    #[test]
    fn with_model_encadena() {
        let cli = OpenAiCompatibleClient::ollama_local("a")
            .with_model("b");
        assert_eq!(cli.model_id(), "b");
    }

    #[test]
    fn roles_se_mapean_user_assistant() {
        let req = ChatRequest {
            system: None,
            max_tokens: 10,
            temperature: 0.0,
            messages: vec![
                ChatMessage::user("U"),
                ChatMessage::assistant("A"),
            ],
        };
        let p = construir_payload(&req, "m");
        assert_eq!(p["messages"][0]["role"], "user");
        assert_eq!(p["messages"][1]["role"], "assistant");
    }
}
