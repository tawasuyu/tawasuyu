//! `pluma-llm-core` — el contrato del cliente LLM agnóstico de proveedor.
//!
//! Define el rasgo [`ChatClient`] y los tipos comunes (mensajes, request,
//! response, errores) que los backends implementan. Igual idea que
//! `rimay-verbo-core` con embeddings: una sola verdad del contrato, N
//! impls intercambiables (`pluma-llm-anthropic`, `pluma-llm-mock`, …).
//!
//! El contrato se mantiene MÍNIMO: solo lo que la suite necesita hoy.
//! - System prompt opcional (lo cachean los proveedores que soporten
//!   prompt caching — Anthropic lo hace si se marca explícitamente).
//! - Lista de mensajes alternados user/assistant.
//! - `max_tokens` + `temperature` configurables.
//! - Respuesta con `content` + `stop_reason` + `usage` opcional.
//!
//! Streaming queda fuera por ahora — el caso típico de pluma (traducir
//! una tabla de párrafos) es batch: pedir N veces y materializar. Cuando
//! aparezca un caso real de UX que requiera tokens en vivo, se añadirá
//! `stream(&self, req)` al rasgo sin romper la API actual.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Rol del mensaje en una conversación de chat. Los proveedores
/// mainstream usan exactamente estos dos — system se trata aparte
/// porque API distintas lo modelan distinto (Anthropic: campo top-level;
/// OpenAI: mensaje con role=system).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Lo escribió quien está usando la app.
    User,
    /// Lo escribió el modelo en una vuelta previa.
    Assistant,
}

/// Una imagen adjunta a un mensaje (visión multimodal). Los proveedores
/// que soportan visión (Anthropic, Gemini) la reciben como bloque de
/// contenido junto al texto; los que no, la ignoran y usan solo `content`.
///
/// Los bytes viajan en base64 (estándar, sin saltos de línea) para no
/// acoplar el contrato a `&[u8]` crudos al serializar la request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatImage {
    /// MIME type del contenido, p.ej. `"image/png"`, `"image/jpeg"`,
    /// `"image/webp"`, `"image/gif"`.
    pub media_type: String,
    /// Bytes de la imagen codificados en base64.
    pub data_base64: String,
}

impl ChatImage {
    /// Construye desde base64 ya codificado.
    pub fn new(media_type: impl Into<String>, data_base64: impl Into<String>) -> Self {
        Self {
            media_type: media_type.into(),
            data_base64: data_base64.into(),
        }
    }

    /// Construye desde bytes crudos, codificando a base64.
    pub fn from_bytes(media_type: impl Into<String>, bytes: &[u8]) -> Self {
        use base64::Engine as _;
        Self {
            media_type: media_type.into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}

/// Un mensaje dentro de la conversación. Además del texto en `content`
/// puede llevar imágenes (`images`) para los backends con visión.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Imágenes adjuntas (visión). Vacío = mensaje solo-texto. Tiene
    /// `serde(default)` para retrocompat con payloads previos sin el campo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ChatImage>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: Vec::new(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: Vec::new(),
        }
    }

    /// Mensaje de usuario con texto + imágenes (visión).
    pub fn user_con_imagenes(content: impl Into<String>, images: Vec<ChatImage>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images,
        }
    }

    /// `true` si el mensaje lleva al menos una imagen.
    pub fn tiene_imagenes(&self) -> bool {
        !self.images.is_empty()
    }
}

/// Petición de completion al modelo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Instrucción del sistema (ej. "Eres un traductor profesional al
    /// quechua del Cuzco. Conserva nombres propios y números."). Se le
    /// dice al modelo que **caché esto si puedes** — los proveedores que
    /// soporten prompt caching lo aprovechan al re-emitir el sistema
    /// idéntico en muchas requests cortas (caso típico: traducir N
    /// párrafos con el mismo system).
    pub system: Option<String>,
    /// Conversación. Para una sola request user→assistant, basta
    /// `vec![ChatMessage::user(prompt)]`.
    pub messages: Vec<ChatMessage>,
    /// Cota de tokens de salida.
    pub max_tokens: u32,
    /// Determinismo: 0.0 = casi determinista, 1.0 = creativo. Para
    /// traducción/extracción suele ir bajo (0.1–0.3); para reescritura
    /// creativa, más alto.
    pub temperature: f32,
}

impl ChatRequest {
    /// Constructor mínimo: un solo mensaje user, sin system.
    pub fn una_vuelta(prompt: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            system: None,
            messages: vec![ChatMessage::user(prompt)],
            max_tokens,
            temperature: 0.2,
        }
    }

    /// Encadenable: agrega un system prompt.
    pub fn con_sistema(mut self, system: impl Into<String>) -> Self {
        self.system = Some(system.into());
        self
    }

    /// Encadenable: ajusta la temperatura.
    pub fn con_temperatura(mut self, t: f32) -> Self {
        self.temperature = t.clamp(0.0, 1.0);
        self
    }
}

/// Razón por la que el modelo dejó de generar tokens. Los strings son
/// libres porque cada proveedor usa los suyos (Anthropic:
/// `"end_turn" | "max_tokens" | "stop_sequence"`; OpenAI:
/// `"stop" | "length"`). Los consumidores que quieran lógica condicional
/// deben tratar el string como opaco.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopReason(pub String);

/// Contabilidad de tokens reportada por el proveedor — útil para tracking
/// de costo y diagnóstico. `None` cuando el proveedor no la expone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Tokens leídos de un cache de prompt-caching (Anthropic). 0 si el
    /// backend no soporta o si no hubo hit.
    pub cache_read_input_tokens: u32,
    /// Tokens escritos al cache (primer write tras un miss).
    pub cache_creation_input_tokens: u32,
}

/// Respuesta del modelo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Texto generado.
    pub content: String,
    /// Razón de parada, si el proveedor la reporta.
    pub stop_reason: Option<StopReason>,
    /// Contabilidad de tokens, si el proveedor la reporta.
    pub usage: Option<ChatUsage>,
}

/// Errores típicos. Los específicos del backend (HTTP status raros,
/// payload inesperado) van en `Backend(String)` con mensaje propio del
/// adapter.
#[derive(Debug, Error)]
pub enum ChatError {
    /// El backend no encontró credenciales (ej. `ANTHROPIC_API_KEY` sin
    /// definir). Los consumidores deciden si caer a un mock o pedirle al
    /// usuario que configure.
    #[error("falta credencial del backend: {0}")]
    AuthMissing(String),
    /// El backend rechazó la credencial (401/403). Distinto de
    /// `AuthMissing`: aquí SÍ hay clave pero no sirve.
    #[error("credencial inválida")]
    AuthInvalid,
    /// El servicio devolvió 429 / cuota superada. El caller puede
    /// retornar al usuario o esperar y reintentar con backoff.
    #[error("rate limited por el backend")]
    RateLimited,
    /// Error de red/transporte (DNS, TLS, timeout, etc.).
    #[error("error de red: {0}")]
    Network(String),
    /// Cualquier otra cosa que el backend reporte como inesperada.
    #[error("error del backend: {0}")]
    Backend(String),
    /// El caller canceló la operación (señal, drop, ctrl-c).
    #[error("cancelado")]
    Cancelled,
}

/// El cliente LLM. Cada backend (Anthropic, OpenAI-compatible, mock,
/// ollama local cuando se sume) implementa este rasgo.
#[async_trait]
pub trait ChatClient: Send + Sync {
    /// Nombre del modelo que este cliente atiende — para logging y para
    /// que el caller pueda anotar en metadatos qué modelo produjo qué
    /// salida (auditoría de derivaciones en pluma).
    fn model_id(&self) -> &str;

    /// Ejecuta una request de chat y devuelve la respuesta.
    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError>;
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn una_vuelta_construye_request_mínimo() {
        let r = ChatRequest::una_vuelta("hola", 100);
        assert_eq!(r.messages.len(), 1);
        assert_eq!(r.messages[0].role, Role::User);
        assert_eq!(r.messages[0].content, "hola");
        assert!(r.system.is_none());
        assert_eq!(r.max_tokens, 100);
    }

    #[test]
    fn con_temperatura_clampea() {
        let r = ChatRequest::una_vuelta("x", 10).con_temperatura(2.5);
        assert_eq!(r.temperature, 1.0);
        let r = ChatRequest::una_vuelta("x", 10).con_temperatura(-0.5);
        assert_eq!(r.temperature, 0.0);
    }

    #[test]
    fn chat_image_from_bytes_codifica_base64() {
        let img = ChatImage::from_bytes("image/png", &[0, 1, 2, 3]);
        assert_eq!(img.media_type, "image/png");
        assert_eq!(img.data_base64, "AAECAw==");
    }

    #[test]
    fn mensaje_con_imagenes_y_retrocompat_serde() {
        let m = ChatMessage::user_con_imagenes(
            "¿qué hay acá?",
            vec![ChatImage::new("image/jpeg", "Zm9v")],
        );
        assert!(m.tiene_imagenes());
        // Un mensaje solo-texto NO serializa el campo `images` (sigue
        // produciendo el mismo JSON que antes de agregar visión).
        let plano = ChatMessage::user("hola");
        let json = serde_json::to_string(&plano).unwrap();
        assert_eq!(json, r#"{"role":"user","content":"hola"}"#);
        // Y un JSON viejo sin `images` deserializa igual.
        let back: ChatMessage =
            serde_json::from_str(r#"{"role":"user","content":"x"}"#).unwrap();
        assert!(!back.tiene_imagenes());
    }

    #[test]
    fn encadenable_combina_sistema_y_temperatura() {
        let r = ChatRequest::una_vuelta("traducir", 200)
            .con_sistema("Eres un traductor.")
            .con_temperatura(0.1);
        assert_eq!(r.system.as_deref(), Some("Eres un traductor."));
        assert!((r.temperature - 0.1).abs() < 1e-6);
    }
}
