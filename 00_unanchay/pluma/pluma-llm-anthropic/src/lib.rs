//! `pluma-llm-anthropic` — backend del trait `ChatClient` contra
//! `api.anthropic.com` (Messages API).
//!
//! Trae prompt caching ENCENDIDO por defecto sobre el `system` prompt:
//! cuando el caller emite N requests con el mismo system (caso típico al
//! traducir muchos párrafos con la misma instrucción de "traductor"),
//! la primera paga full input, las siguientes pagan el system como
//! cache read — ~10× más barato. La contabilidad expone los token counts
//! de cache hit / miss para que la app pueda mostrar el ahorro real.
//!
//! Sin caching del `messages`: en pluma el `system` es lo que se repite;
//! los `messages` cambian por párrafo. Quien quiera cachear segmentos
//! largos de `messages` puede hacerlo en una iteración futura.
//!
//! ## Configuración mínima
//!
//! ```no_run
//! # use pluma_llm_anthropic::AnthropicClient;
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // API key: env `ANTHROPIC_API_KEY` (o pasarla explícita en `with_api_key`).
//! // Modelo: por defecto `claude-sonnet-4-6` — balance calidad/costo.
//! let cli = AnthropicClient::from_env()?;
//! # Ok(()) }
//! ```

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_llm_core::{
    ChatClient, ChatError, ChatRequest, ChatResponse, ChatUsage, Role, StopReason,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Endpoint default del Messages API.
const ENDPOINT_DEFAULT: &str = "https://api.anthropic.com/v1/messages";
/// Versión del API documentada por Anthropic — se manda como header
/// `anthropic-version`. Si Anthropic publica una nueva, se bumpea aquí.
const API_VERSION: &str = "2023-06-01";
/// Modelo por defecto: Sonnet 4.6 es el sweet spot calidad/costo para
/// transformaciones de pluma (traducir, tono, resumir). Quien quiera
/// más cabeza usa Opus 4.7 vía `with_model("claude-opus-4-7")`;
/// quien quiera más barato usa Haiku 4.5
/// (`claude-haiku-4-5-20251001`).
const MODEL_DEFAULT: &str = "claude-sonnet-4-6";
/// Timeout por defecto de la request HTTP. Un párrafo se traduce en
/// pocos segundos; 60 s deja holgura para colas/red sin colgar la UI.
const TIMEOUT_DEFAULT_SECS: u64 = 60;

/// Cliente Anthropic Messages API que implementa [`ChatClient`].
pub struct AnthropicClient {
    http: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    cache_system: bool,
}

impl AnthropicClient {
    /// Construye un cliente leyendo la API key de `ANTHROPIC_API_KEY`.
    /// Si la variable no está, devuelve `ChatError::AuthMissing`.
    pub fn from_env() -> Result<Self, ChatError> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| ChatError::AuthMissing("ANTHROPIC_API_KEY".to_string()))?;
        Self::with_api_key(api_key)
    }

    /// Construye un cliente con una API key explícita. Útil cuando la
    /// key vive en un keyring del SO o en un archivo (no env var).
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
            cache_system: true,
        })
    }

    /// Encadenable: cambia el modelo. Anthropic ids válidos hoy:
    /// `claude-opus-4-7`, `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Encadenable: desactiva el prompt caching del system. Por defecto
    /// está encendido — apagarlo solo tiene sentido si el system cambia
    /// en cada request (cosa rara, pero el caller decide).
    pub fn sin_cache_system(mut self) -> Self {
        self.cache_system = false;
        self
    }

    /// Encadenable: cambia el endpoint. Útil para proxies internos o
    /// servicios compatible-Anthropic.
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Headers que requiere la Messages API.
    fn headers(&self) -> Result<HeaderMap, ChatError> {
        let mut h = HeaderMap::new();
        h.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| ChatError::Backend("api key con bytes inválidos".to_string()))?,
        );
        h.insert("anthropic-version", HeaderValue::from_static(API_VERSION));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        Ok(h)
    }
}

#[async_trait]
impl ChatClient for AnthropicClient {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError> {
        let payload = construir_payload(req, &self.model, self.cache_system);
        let resp = self
            .http
            .post(&self.endpoint)
            .headers(self.headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChatError::Network(format!("POST messages: {e}")))?;

        let status = resp.status();
        let body_bytes = resp
            .bytes()
            .await
            .map_err(|e| ChatError::Network(format!("leer body: {e}")))?;

        // Distinguir errores comunes ANTES de intentar deserializar como
        // respuesta exitosa — el body de error tiene shape distinto.
        if status == 401 || status == 403 {
            return Err(ChatError::AuthInvalid);
        }
        if status == 429 {
            return Err(ChatError::RateLimited);
        }
        if !status.is_success() {
            // El body trae JSON `{ "type": "error", "error": { "message": ... } }`.
            let mensaje = match serde_json::from_slice::<ErrorEnvelope>(&body_bytes) {
                Ok(env) => env.error.message,
                Err(_) => String::from_utf8_lossy(&body_bytes).into_owned(),
            };
            return Err(ChatError::Backend(format!("HTTP {status}: {mensaje}")));
        }

        let parsed: AnthropicMessagesResponse = serde_json::from_slice(&body_bytes)
            .map_err(|e| ChatError::Backend(format!("parseo response: {e}")))?;

        let content = parsed
            .content
            .into_iter()
            .filter_map(|b| match b {
                AnthropicContentBlock::Text { text } => Some(text),
            })
            .collect::<Vec<_>>()
            .join("");

        let usage = parsed.usage.map(|u| ChatUsage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_read_input_tokens: u.cache_read_input_tokens.unwrap_or(0),
            cache_creation_input_tokens: u.cache_creation_input_tokens.unwrap_or(0),
        });

        Ok(ChatResponse {
            content,
            stop_reason: parsed.stop_reason.map(StopReason),
            usage,
        })
    }
}

/// Traduce un [`ChatRequest`] al payload JSON que Anthropic espera. El
/// `system` se envía como bloque cacheable cuando `cache_system` está
/// activo — un bloque `{type:"text", text:..., cache_control:{type:"ephemeral"}}`
/// dentro de un array — para que la siguiente request con system idéntico
/// caiga en cache.
fn construir_payload(
    req: &ChatRequest,
    modelo: &str,
    cache_system: bool,
) -> serde_json::Value {
    let mensajes: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            if m.images.is_empty() {
                // Solo-texto: `content` como string plano (idéntico al
                // payload previo a visión — no rompe caching ni tests).
                serde_json::json!({
                    "role": role,
                    "content": m.content,
                })
            } else {
                // Multimodal: array de bloques. Anthropic recomienda las
                // imágenes ANTES del texto. Cada imagen es un bloque
                // `image` con `source` base64.
                let mut blocks: Vec<serde_json::Value> = m
                    .images
                    .iter()
                    .map(|img| {
                        serde_json::json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": img.media_type,
                                "data": img.data_base64,
                            }
                        })
                    })
                    .collect();
                blocks.push(serde_json::json!({"type": "text", "text": m.content}));
                serde_json::json!({ "role": role, "content": blocks })
            }
        })
        .collect();

    let mut payload = serde_json::json!({
        "model": modelo,
        "max_tokens": req.max_tokens,
        "temperature": req.temperature,
        "messages": mensajes,
    });

    if let Some(sys) = req.system.as_deref() {
        let system_value = if cache_system {
            // Array de bloques con `cache_control: ephemeral` — la API
            // reusa el cache para hasta 5 minutos si el contenido del
            // bloque no cambia. Es la forma documentada de prompt caching.
            serde_json::json!([{
                "type": "text",
                "text": sys,
                "cache_control": {"type": "ephemeral"}
            }])
        } else {
            // String plano — sin caching.
            serde_json::json!(sys)
        };
        payload
            .as_object_mut()
            .expect("payload es object")
            .insert("system".to_string(), system_value);
    }

    payload
}

// -------- Tipos del wire de Anthropic (parseo de la respuesta) --------

#[derive(Debug, Deserialize)]
struct AnthropicMessagesResponse {
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text { text: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: Option<u32>,
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize, Serialize)]
struct ErrorBody {
    #[serde(default)]
    message: String,
    #[serde(default, rename = "type")]
    _kind: String,
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_core::ChatMessage;

    #[test]
    fn payload_sin_system_no_lleva_el_campo() {
        let req = ChatRequest::una_vuelta("hola", 50);
        let p = construir_payload(&req, "claude-sonnet-4-6", true);
        assert!(p.get("system").is_none());
        assert_eq!(p["model"], "claude-sonnet-4-6");
        assert_eq!(p["max_tokens"], 50);
        assert_eq!(p["messages"][0]["role"], "user");
        assert_eq!(p["messages"][0]["content"], "hola");
    }

    #[test]
    fn payload_con_system_cacheado_emite_bloque_ephemeral() {
        let req = ChatRequest::una_vuelta("texto", 50)
            .con_sistema("Eres un traductor.");
        let p = construir_payload(&req, "claude-sonnet-4-6", true);
        let system = p.get("system").expect("system presente");
        assert!(system.is_array());
        let bloque = &system[0];
        assert_eq!(bloque["type"], "text");
        assert_eq!(bloque["text"], "Eres un traductor.");
        assert_eq!(bloque["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn payload_con_cache_desactivado_emite_system_plano() {
        let req = ChatRequest::una_vuelta("x", 50)
            .con_sistema("Eres un traductor.");
        let p = construir_payload(&req, "claude-sonnet-4-6", false);
        let system = p.get("system").expect("system presente");
        assert_eq!(system, "Eres un traductor.");
    }

    // (Test de `from_env` sin variable omitido: Rust 2024 marca
    // `std::env::remove_var` y `set_var` como unsafe — tocar el entorno
    // del proceso desde tests es race-prone y `forbid(unsafe_code)` lo
    // bloquea. La lógica que devuelve `AuthMissing` queda cubierta por
    // inspección del código y por uso end-to-end del cliente.)

    #[test]
    fn roles_se_mapean_correctamente_en_el_payload() {
        let req = ChatRequest {
            system: None,
            max_tokens: 10,
            temperature: 0.0,
            messages: vec![
                ChatMessage::user("U"),
                ChatMessage::assistant("A"),
                ChatMessage::user("U2"),
            ],
        };
        let p = construir_payload(&req, "m", true);
        assert_eq!(p["messages"][0]["role"], "user");
        assert_eq!(p["messages"][1]["role"], "assistant");
        assert_eq!(p["messages"][2]["role"], "user");
        assert_eq!(p["messages"][2]["content"], "U2");
    }

    #[test]
    fn mensaje_con_imagen_emite_bloques_image_y_text() {
        use pluma_llm_core::ChatImage;
        let req = ChatRequest {
            system: None,
            max_tokens: 100,
            temperature: 0.0,
            messages: vec![ChatMessage::user_con_imagenes(
                "¿qué hay en la foto?",
                vec![ChatImage::new("image/png", "AAEC")],
            )],
        };
        let p = construir_payload(&req, "claude-sonnet-4-6", false);
        let content = &p["messages"][0]["content"];
        assert!(content.is_array(), "content debe ser array con imagen");
        // Imagen primero, texto después.
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content[0]["source"]["type"], "base64");
        assert_eq!(content[0]["source"]["media_type"], "image/png");
        assert_eq!(content[0]["source"]["data"], "AAEC");
        assert_eq!(content[1]["type"], "text");
        assert_eq!(content[1]["text"], "¿qué hay en la foto?");
    }
}
