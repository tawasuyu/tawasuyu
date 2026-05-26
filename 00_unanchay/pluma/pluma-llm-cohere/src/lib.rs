//! `pluma-llm-cohere` — backend del trait `ChatClient` contra
//! `api.cohere.com/v2/chat`.
//!
//! Request shape parecida a OpenAI (`messages` con `role` + `content`),
//! pero la response es distinta: un solo `message` con
//! `content: [{type:"text", text:"..."}]` (estilo Anthropic). Por eso
//! va en un crate aparte en lugar de reusar
//! `pluma-llm-openai-compatible`.
//!
//! ## Configuración
//!
//! ```no_run
//! # use pluma_llm_cohere::CohereClient;
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // Lee COHERE_API_KEY del env.
//! let cli = CohereClient::from_env()?;
//! // Modelo default: `command-a-03-2025` (Command A, top-of-line de
//! // Cohere). Para Command-R: `.with_model("command-r-08-2024")`.
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_llm_core::{
    ChatClient, ChatError, ChatRequest, ChatResponse, ChatUsage, Role, StopReason,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::time::Duration;

const TIMEOUT_DEFAULT_SECS: u64 = 60;
const ENDPOINT_DEFAULT: &str = "https://api.cohere.com/v2/chat";
const MODEL_DEFAULT: &str = "command-a-03-2025";
const ENV_KEY: &str = "COHERE_API_KEY";

/// Cliente Cohere v2 implementando [`ChatClient`].
pub struct CohereClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl CohereClient {
    /// Lee la API key de `COHERE_API_KEY`.
    pub fn from_env() -> Result<Self, ChatError> {
        let api_key =
            std::env::var(ENV_KEY).map_err(|_| ChatError::AuthMissing(ENV_KEY.to_string()))?;
        Self::with_api_key(api_key)
    }

    /// Construye con una API key explícita.
    pub fn with_api_key(api_key: impl Into<String>) -> Result<Self, ChatError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_DEFAULT_SECS))
            .build()
            .map_err(|e| ChatError::Network(format!("construir reqwest client: {e}")))?;
        Ok(Self {
            http,
            endpoint: ENDPOINT_DEFAULT.to_string(),
            api_key: api_key.into(),
            model: MODEL_DEFAULT.to_string(),
        })
    }

    /// Cambia el modelo. Válidos hoy: `command-a-03-2025` (default),
    /// `command-r-plus-08-2024`, `command-r-08-2024`.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Cambia el endpoint — útil para proxies internos.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    fn headers(&self) -> Result<HeaderMap, ChatError> {
        let mut h = HeaderMap::new();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        let val = HeaderValue::from_str(&format!("Bearer {}", self.api_key))
            .map_err(|_| ChatError::Backend("api key con bytes inválidos".to_string()))?;
        h.insert("authorization", val);
        Ok(h)
    }
}

#[async_trait]
impl ChatClient for CohereClient {
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
            .map_err(|e| ChatError::Network(format!("POST v2/chat: {e}")))?;

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
            let mensaje = match serde_json::from_slice::<CohereError>(&body_bytes) {
                Ok(env) => env.message,
                Err(_) => String::from_utf8_lossy(&body_bytes).into_owned(),
            };
            return Err(ChatError::Backend(format!("HTTP {status}: {mensaje}")));
        }

        let parsed: CohereResponse = serde_json::from_slice(&body_bytes)
            .map_err(|e| ChatError::Backend(format!("parseo response: {e}")))?;

        // El `message.content` es un array de bloques `{type,text}`. Solo
        // recogemos los de tipo "text".
        let content = parsed
            .message
            .content
            .into_iter()
            .filter_map(|b| match b {
                CohereContentBlock::Text { text } => Some(text),
            })
            .collect::<Vec<_>>()
            .join("");

        // Cohere reporta `usage.tokens` (total) y `usage.billed_units`
        // (lo que cobra). Preferimos `tokens` para mostrar la actividad
        // real del modelo; el caller que quiera contabilidad de costo
        // mira `billed_units` por separado en una iteración futura.
        let usage = parsed.usage.and_then(|u| u.tokens).map(|t| ChatUsage {
            input_tokens: t.input_tokens.unwrap_or(0),
            output_tokens: t.output_tokens.unwrap_or(0),
            // Cohere no expone caching de prompts hoy.
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        });

        Ok(ChatResponse {
            content,
            stop_reason: parsed.finish_reason.map(StopReason),
            usage,
        })
    }
}

/// Compone el payload v2/chat: messages con role/content (string plano),
/// model, max_tokens, temperature. El `system` de pluma se mete como
/// primer mensaje con role=system (igual que OpenAI; distinto de
/// Anthropic donde el system es top-level).
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

// -------- Tipos del wire Cohere v2 --------

#[derive(Debug, Deserialize)]
struct CohereResponse {
    message: CohereMessage,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    usage: Option<CohereUsage>,
}

#[derive(Debug, Deserialize)]
struct CohereMessage {
    #[serde(default)]
    content: Vec<CohereContentBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CohereContentBlock {
    Text { text: String },
}

#[derive(Debug, Deserialize)]
struct CohereUsage {
    #[serde(default)]
    tokens: Option<CohereTokens>,
}

#[derive(Debug, Deserialize)]
struct CohereTokens {
    #[serde(default)]
    input_tokens: Option<u32>,
    #[serde(default)]
    output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct CohereError {
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_core::ChatMessage;

    #[test]
    fn payload_sin_system_solo_user() {
        let req = ChatRequest::una_vuelta("hola", 50);
        let p = construir_payload(&req, "command-a-03-2025");
        assert_eq!(p["model"], "command-a-03-2025");
        assert_eq!(p["messages"].as_array().unwrap().len(), 1);
        assert_eq!(p["messages"][0]["role"], "user");
    }

    #[test]
    fn payload_con_system_inserta_primer_mensaje_system() {
        let req = ChatRequest::una_vuelta("x", 50).con_sistema("Eres traductor.");
        let p = construir_payload(&req, "m");
        let msgs = p["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "Eres traductor.");
    }

    #[test]
    fn parsea_response_completa_con_content_y_usage() {
        let body = serde_json::json!({
            "id": "abc",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "hola"},
                    {"type": "text", "text": " mundo"}
                ]
            },
            "finish_reason": "COMPLETE",
            "usage": {
                "tokens": {"input_tokens": 12, "output_tokens": 3},
                "billed_units": {"input_tokens": 12, "output_tokens": 3}
            }
        });
        let parsed: CohereResponse = serde_json::from_value(body).unwrap();
        let texto: String = parsed
            .message
            .content
            .into_iter()
            .filter_map(|b| match b {
                CohereContentBlock::Text { text } => Some(text),
            })
            .collect();
        assert_eq!(texto, "hola mundo");
        assert_eq!(parsed.finish_reason.as_deref(), Some("COMPLETE"));
        let u = parsed.usage.unwrap().tokens.unwrap();
        assert_eq!(u.input_tokens, Some(12));
        assert_eq!(u.output_tokens, Some(3));
    }

    #[test]
    fn roles_assistant_pasa_como_assistant() {
        let req = ChatRequest {
            system: None,
            max_tokens: 1,
            temperature: 0.0,
            messages: vec![ChatMessage::assistant("hola"), ChatMessage::user("¿qué?")],
        };
        let p = construir_payload(&req, "m");
        assert_eq!(p["messages"][0]["role"], "assistant");
        assert_eq!(p["messages"][1]["role"], "user");
    }
}
