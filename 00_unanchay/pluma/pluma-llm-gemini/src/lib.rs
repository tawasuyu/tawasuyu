//! `pluma-llm-gemini` — adapter contra Google Gemini API
//! (Generative Language).
//!
//! Gemini no habla la shape OpenAI: usa `contents` con `parts`,
//! `systemInstruction` aparte, y roles `user` / `model` (no `assistant`).
//! Este crate traduce los tipos genéricos de `pluma-llm-core` a esa shape
//! y de vuelta. La API key va en el query string (`?key=...`) según la
//! documentación oficial de AI Studio.
//!
//! ## Configuración
//!
//! ```no_run
//! # use pluma_llm_gemini::GeminiClient;
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! // Lee GEMINI_API_KEY (o GOOGLE_API_KEY como fallback — ambas son
//! // convenciones comunes en el ecosistema).
//! let cli = GeminiClient::from_env()?;
//!
//! // O explícito; modelo por defecto: `gemini-2.5-flash` (rápido, barato).
//! // Para Pro: `.with_model("gemini-2.5-pro")`.
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
const ENDPOINT_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";
/// `gemini-2.5-flash` — balance velocidad/calidad para transformaciones
/// de pluma (traducir, tono, resumir). Para más cabeza: `gemini-2.5-pro`.
const MODEL_DEFAULT: &str = "gemini-2.5-flash";
/// Convenciones de env: el ecosistema usa `GEMINI_API_KEY` (AI Studio) y
/// `GOOGLE_API_KEY` (Cloud). Aceptamos ambos en orden.
const ENV_KEY_PRIMARY: &str = "GEMINI_API_KEY";
const ENV_KEY_FALLBACK: &str = "GOOGLE_API_KEY";

/// Cliente Gemini implementando [`ChatClient`].
pub struct GeminiClient {
    http: reqwest::Client,
    endpoint_base: String,
    api_key: String,
    model: String,
}

impl GeminiClient {
    /// Lee la API key de `GEMINI_API_KEY` (preferida) o `GOOGLE_API_KEY`.
    pub fn from_env() -> Result<Self, ChatError> {
        let api_key = std::env::var(ENV_KEY_PRIMARY)
            .or_else(|_| std::env::var(ENV_KEY_FALLBACK))
            .map_err(|_| {
                ChatError::AuthMissing(format!("{ENV_KEY_PRIMARY} (o {ENV_KEY_FALLBACK})"))
            })?;
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
            endpoint_base: ENDPOINT_BASE.to_string(),
            api_key: api_key.into(),
            model: MODEL_DEFAULT.to_string(),
        })
    }

    /// Cambia el modelo. Válidos hoy: `gemini-2.5-pro`, `gemini-2.5-flash`,
    /// `gemini-2.5-flash-lite`, `gemini-2.0-flash`.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Cambia la base del endpoint — útil para proxies internos que
    /// reescriben hacia Gemini.
    pub fn with_endpoint_base(mut self, base: impl Into<String>) -> Self {
        self.endpoint_base = base.into();
        self
    }

    fn url_generate(&self) -> String {
        format!("{}/{}:generateContent", self.endpoint_base, self.model)
    }

    /// Header `x-goog-api-key` — alternativa documentada al query param.
    /// La preferimos para no mostrar la key en logs de access HTTP.
    fn headers(&self) -> Result<HeaderMap, ChatError> {
        let mut h = HeaderMap::new();
        h.insert("content-type", HeaderValue::from_static("application/json"));
        let val = HeaderValue::from_str(&self.api_key)
            .map_err(|_| ChatError::Backend("api key con bytes inválidos".to_string()))?;
        h.insert("x-goog-api-key", val);
        Ok(h)
    }
}

#[async_trait]
impl ChatClient for GeminiClient {
    fn model_id(&self) -> &str {
        &self.model
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError> {
        let payload = construir_payload(req);
        let resp = self
            .http
            .post(self.url_generate())
            .headers(self.headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChatError::Network(format!("POST generateContent: {e}")))?;

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
            // Gemini devuelve `{"error":{"message":...,"status":"..."}}`.
            let mensaje = match serde_json::from_slice::<GeminiErrorEnvelope>(&body_bytes) {
                Ok(env) => env.error.message,
                Err(_) => String::from_utf8_lossy(&body_bytes).into_owned(),
            };
            return Err(ChatError::Backend(format!("HTTP {status}: {mensaje}")));
        }

        let parsed: GeminiResponse = serde_json::from_slice(&body_bytes)
            .map_err(|e| ChatError::Backend(format!("parseo response: {e}")))?;

        let primer = parsed.candidates.into_iter().next().ok_or_else(|| {
            ChatError::Backend("response sin candidates".to_string())
        })?;
        // Concatenar las parts.text del content.
        let content = primer
            .content
            .map(|c| {
                c.parts
                    .into_iter()
                    .filter_map(|p| p.text)
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let stop_reason = primer.finish_reason.map(StopReason);

        // Gemini reporta `cachedContentTokenCount` cuando se usó cache de
        // sistema (endpoint /cachedContents, no implementado aquí).
        let usage = parsed.usage_metadata.map(|u| ChatUsage {
            input_tokens: u.prompt_token_count.unwrap_or(0),
            output_tokens: u.candidates_token_count.unwrap_or(0),
            cache_read_input_tokens: u.cached_content_token_count.unwrap_or(0),
            cache_creation_input_tokens: 0,
        });

        Ok(ChatResponse {
            content,
            stop_reason,
            usage,
        })
    }
}

/// Traduce un `ChatRequest` a la shape Gemini:
/// - `system` → top-level `systemInstruction: {parts: [{text:...}]}`.
/// - `messages` → array `contents`, role mapeado (`user` → `user`,
///   `assistant` → `model`), cada uno con `parts: [{text:...}]`.
/// - `max_tokens` + `temperature` → `generationConfig`.
fn construir_payload(req: &ChatRequest) -> serde_json::Value {
    let contents: Vec<serde_json::Value> = req
        .messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::User => "user",
                Role::Assistant => "model",
            };
            // Texto primero, luego las imágenes como `inlineData`
            // (camelCase, igual que el resto del payload Gemini).
            let mut parts: Vec<serde_json::Value> =
                vec![serde_json::json!({"text": m.content})];
            for img in &m.images {
                parts.push(serde_json::json!({
                    "inlineData": {
                        "mimeType": img.media_type,
                        "data": img.data_base64,
                    }
                }));
            }
            serde_json::json!({
                "role": role,
                "parts": parts,
            })
        })
        .collect();

    let mut payload = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": req.max_tokens,
            "temperature": req.temperature,
        },
    });

    if let Some(sys) = &req.system {
        payload.as_object_mut().unwrap().insert(
            "systemInstruction".to_string(),
            serde_json::json!({"parts": [{"text": sys}]}),
        );
    }

    payload
}

// -------- Tipos del wire Gemini --------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiUsage {
    #[serde(default)]
    prompt_token_count: Option<u32>,
    #[serde(default)]
    candidates_token_count: Option<u32>,
    #[serde(default)]
    cached_content_token_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorEnvelope {
    error: GeminiErrorBody,
}

#[derive(Debug, Deserialize)]
struct GeminiErrorBody {
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_core::ChatMessage;

    #[test]
    fn payload_sin_system_omite_systemInstruction() {
        let req = ChatRequest::una_vuelta("hola", 50);
        let p = construir_payload(&req);
        assert!(p.get("systemInstruction").is_none());
        let contents = p["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "hola");
        assert_eq!(p["generationConfig"]["maxOutputTokens"], 50);
    }

    #[test]
    fn payload_con_system_lleva_systemInstruction_top_level() {
        let req = ChatRequest::una_vuelta("traduce esto", 100)
            .con_sistema("Eres traductor.");
        let p = construir_payload(&req);
        let si = p.get("systemInstruction").expect("system presente");
        assert_eq!(si["parts"][0]["text"], "Eres traductor.");
    }

    #[test]
    fn assistant_se_mapea_a_role_model() {
        let req = ChatRequest {
            system: None,
            max_tokens: 1,
            temperature: 0.0,
            messages: vec![
                ChatMessage::user("u"),
                ChatMessage::assistant("a"),
            ],
        };
        let p = construir_payload(&req);
        assert_eq!(p["contents"][0]["role"], "user");
        assert_eq!(p["contents"][1]["role"], "model");
    }

    #[test]
    fn mensaje_con_imagen_agrega_part_inline_data() {
        use pluma_llm_core::ChatImage;
        let req = ChatRequest {
            system: None,
            max_tokens: 100,
            temperature: 0.0,
            messages: vec![ChatMessage::user_con_imagenes(
                "describe",
                vec![ChatImage::new("image/jpeg", "Zm9v")],
            )],
        };
        let p = construir_payload(&req);
        let parts = &p["contents"][0]["parts"];
        assert_eq!(parts[0]["text"], "describe");
        assert_eq!(parts[1]["inlineData"]["mimeType"], "image/jpeg");
        assert_eq!(parts[1]["inlineData"]["data"], "Zm9v");
    }

    #[test]
    fn url_generate_incluye_modelo_y_endpoint() {
        let cli = GeminiClient::with_api_key("k").unwrap();
        assert!(cli.url_generate().contains("gemini-2.5-flash:generateContent"));
        let cli = cli.with_model("gemini-2.5-pro");
        assert!(cli.url_generate().ends_with("gemini-2.5-pro:generateContent"));
    }

    #[test]
    fn parsea_response_con_candidates_y_usage() {
        let body = serde_json::json!({
            "candidates": [{
                "content": {"parts": [{"text": "huk"}, {"text": " iskay"}], "role": "model"},
                "finishReason": "STOP"
            }],
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 5,
                "cachedContentTokenCount": 80
            }
        });
        let parsed: GeminiResponse = serde_json::from_value(body).unwrap();
        let cand = &parsed.candidates[0];
        let content = cand.content.as_ref().unwrap();
        let texto: String = content
            .parts
            .iter()
            .filter_map(|p| p.text.clone())
            .collect();
        assert_eq!(texto, "huk iskay");
        let u = parsed.usage_metadata.unwrap();
        assert_eq!(u.prompt_token_count, Some(100));
        assert_eq!(u.cached_content_token_count, Some(80));
    }
}
