//! `pluma-llm` — fachada transparente sobre el stack LLM.
//!
//! Un solo punto de entrada: el caller declara qué backend quiere
//! (Anthropic, Gemini, DeepSeek, Ollama, Mock), opcionalmente el modelo
//! y la API key (si no, se lee de env), y recibe un `Arc<dyn ChatClient>`.
//! Desde ese momento el caller habla solo con el trait — no importa cuál
//! IA esté detrás.
//!
//! ## Ejemplo end-to-end
//!
//! ```no_run
//! # use pluma_llm::{build_client, BackendKind, LlmConfig};
//! # use pluma_llm_core::{ChatClient, ChatRequest};
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let cli = build_client(&LlmConfig {
//!     kind: BackendKind::Gemini,
//!     model: None,    // default por backend
//!     api_key: None,  // lee env
//!     endpoint: None, // default
//! })?;
//! let resp = cli.complete(&ChatRequest::una_vuelta("hola", 50)).await?;
//! println!("{}", resp.content);
//! # Ok(()) }
//! ```
//!
//! Cambiar de IA es UNA línea: `kind: BackendKind::Anthropic`. Cero
//! cambios en el resto del código del consumidor.
//!
//! ## Variables de entorno reconocidas
//!
//! - **Anthropic**: `ANTHROPIC_API_KEY` · default `claude-sonnet-4-6`.
//! - **Gemini**: `GEMINI_API_KEY` o `GOOGLE_API_KEY` · default `gemini-2.5-flash`.
//! - **DeepSeek**: `DEEPSEEK_API_KEY` · default `deepseek-chat`.
//! - **Ollama**: sin key · endpoint default `http://localhost:11434/v1/chat/completions`
//!   · `model` REQUERIDO (el caller dice qué tag pulled usar).
//! - **Mock**: sin key, sin red, eco determinista para tests.
//!
//! ## Selección por env
//!
//! [`from_env`] elige backend automático según `PLUMA_LLM_BACKEND`:
//! `"anthropic" | "gemini" | "deepseek" | "ollama" | "mock"`. Default si
//! la variable no está: el primer backend cuyo env de API esté presente,
//! en orden Anthropic → Gemini → DeepSeek, con fallback final a Mock.
//! Quien quiera más control, llama directo a [`build_client`].

#![forbid(unsafe_code)]

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use pluma_llm_core::{ChatClient, ChatError};

pub use pluma_llm_core; // re-export para que el caller solo dependa de este crate

/// Identidad del backend LLM concreto que se va a instanciar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Anthropic,
    Gemini,
    DeepSeek,
    Ollama,
    Mock,
}

impl BackendKind {
    /// Parsea una etiqueta string (case-insensitive). Útil para CLIs y
    /// config en archivos.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "anthropic" => Some(BackendKind::Anthropic),
            "gemini" | "google" => Some(BackendKind::Gemini),
            "deepseek" => Some(BackendKind::DeepSeek),
            "ollama" => Some(BackendKind::Ollama),
            "mock" => Some(BackendKind::Mock),
            _ => None,
        }
    }
}

/// Configuración de un backend. Campos opcionales caen a defaults
/// razonables por backend.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmConfig {
    pub kind: BackendKind,
    /// Modelo concreto. `None` = default del backend (ver doc del crate).
    /// Para Ollama, NO hay default — se exige modelo explícito.
    #[serde(default)]
    pub model: Option<String>,
    /// API key. `None` = lee del env. Anthropic y los demás backends
    /// remotos exigen una. Ollama y Mock la ignoran.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Endpoint custom. `None` = default del backend. Útil para proxies
    /// internos o servicios self-hosted.
    #[serde(default)]
    pub endpoint: Option<String>,
}

impl Default for BackendKind {
    fn default() -> Self {
        BackendKind::Mock
    }
}

/// Error de configuración o de instanciación del backend.
#[derive(Debug, Error)]
pub enum BuildError {
    /// Algo del [`ChatClient`] concreto falló al construir (typically
    /// `AuthMissing` cuando la env var no está).
    #[error("inicialización del backend falló: {0}")]
    Chat(#[from] ChatError),
    /// Configuración incompleta — p.ej. Ollama sin `model`.
    #[error("config incompleta: {0}")]
    Config(String),
}

/// Construye un cliente concreto según `cfg`. Devuelve un
/// `Arc<dyn ChatClient>` — el caller habla solo con el trait y puede
/// cambiar de backend cambiando UNA variante del enum.
pub fn build_client(cfg: &LlmConfig) -> Result<Arc<dyn ChatClient>, BuildError> {
    match cfg.kind {
        BackendKind::Anthropic => {
            let mut cli = match &cfg.api_key {
                Some(k) => pluma_llm_anthropic::AnthropicClient::with_api_key(k.clone())?,
                None => pluma_llm_anthropic::AnthropicClient::from_env()?,
            };
            if let Some(m) = &cfg.model {
                cli = cli.with_model(m.clone());
            }
            if let Some(ep) = &cfg.endpoint {
                cli = cli.with_endpoint(ep.clone());
            }
            Ok(Arc::new(cli))
        }
        BackendKind::Gemini => {
            let mut cli = match &cfg.api_key {
                Some(k) => pluma_llm_gemini::GeminiClient::with_api_key(k.clone())?,
                None => pluma_llm_gemini::GeminiClient::from_env()?,
            };
            if let Some(m) = &cfg.model {
                cli = cli.with_model(m.clone());
            }
            if let Some(ep) = &cfg.endpoint {
                cli = cli.with_endpoint_base(ep.clone());
            }
            Ok(Arc::new(cli))
        }
        BackendKind::DeepSeek => {
            let cli = match &cfg.api_key {
                Some(k) => {
                    let endpoint = cfg
                        .endpoint
                        .clone()
                        .unwrap_or_else(|| "https://api.deepseek.com/chat/completions".into());
                    let model = cfg.model.clone().unwrap_or_else(|| "deepseek-chat".into());
                    pluma_llm_openai_compatible::OpenAiCompatibleClient::custom(
                        endpoint,
                        Some(k.clone()),
                        model,
                    )
                }
                None => {
                    let mut cli =
                        pluma_llm_openai_compatible::OpenAiCompatibleClient::deepseek_from_env()?;
                    if let Some(m) = &cfg.model {
                        cli = cli.with_model(m.clone());
                    }
                    cli
                }
            };
            Ok(Arc::new(cli))
        }
        BackendKind::Ollama => {
            let model = cfg.model.clone().ok_or_else(|| {
                BuildError::Config(
                    "Ollama exige `model` explícito (p.ej. \"llama3.1\", \"qwen2.5\") — sin default seguro".into(),
                )
            })?;
            let cli = if let Some(ep) = &cfg.endpoint {
                pluma_llm_openai_compatible::OpenAiCompatibleClient::custom(
                    ep.clone(),
                    None,
                    model,
                )
            } else {
                pluma_llm_openai_compatible::OpenAiCompatibleClient::ollama_local(model)
            };
            Ok(Arc::new(cli))
        }
        BackendKind::Mock => Ok(Arc::new(pluma_llm_mock::MockChatClient::default())),
    }
}

/// Elige backend según `PLUMA_LLM_BACKEND`. Si la variable no está,
/// detecta automáticamente: usa el primer backend cuyo env de API esté
/// definido, en orden Anthropic → Gemini → DeepSeek. Fallback final:
/// Mock (deterministic, sin red).
///
/// El modelo (`PLUMA_LLM_MODEL`) y endpoint (`PLUMA_LLM_ENDPOINT`)
/// también pueden venir por env — útil para CI/sandbox sin tocar código.
pub fn from_env() -> Result<Arc<dyn ChatClient>, BuildError> {
    let kind = std::env::var("PLUMA_LLM_BACKEND")
        .ok()
        .and_then(|s| BackendKind::parse(&s))
        .unwrap_or_else(detectar_backend_por_env);
    let model = std::env::var("PLUMA_LLM_MODEL").ok();
    let endpoint = std::env::var("PLUMA_LLM_ENDPOINT").ok();
    build_client(&LlmConfig {
        kind,
        model,
        api_key: None, // siempre via env del backend específico
        endpoint,
    })
}

/// Detecta qué backend usar por presencia de la API key correspondiente.
/// Heurística honesta — si nadie expuso credenciales, cae a Mock para no
/// fallar el arranque del proceso.
fn detectar_backend_por_env() -> BackendKind {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        return BackendKind::Anthropic;
    }
    if std::env::var("GEMINI_API_KEY").is_ok() || std::env::var("GOOGLE_API_KEY").is_ok() {
        return BackendKind::Gemini;
    }
    if std::env::var("DEEPSEEK_API_KEY").is_ok() {
        return BackendKind::DeepSeek;
    }
    BackendKind::Mock
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn parse_acepta_aliases() {
        assert_eq!(BackendKind::parse("anthropic"), Some(BackendKind::Anthropic));
        assert_eq!(BackendKind::parse("ANTHROPIC"), Some(BackendKind::Anthropic));
        assert_eq!(BackendKind::parse("google"), Some(BackendKind::Gemini));
        assert_eq!(BackendKind::parse("gemini"), Some(BackendKind::Gemini));
        assert_eq!(BackendKind::parse("deepseek"), Some(BackendKind::DeepSeek));
        assert_eq!(BackendKind::parse("ollama"), Some(BackendKind::Ollama));
        assert_eq!(BackendKind::parse("mock"), Some(BackendKind::Mock));
        assert_eq!(BackendKind::parse("openai"), None);
        assert_eq!(BackendKind::parse(""), None);
    }

    #[test]
    fn default_kind_es_mock() {
        let cfg = LlmConfig::default();
        assert_eq!(cfg.kind, BackendKind::Mock);
    }

    #[tokio::test]
    async fn build_mock_y_devuelve_chat_client_funcional() {
        let cli = build_client(&LlmConfig {
            kind: BackendKind::Mock,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cli.model_id(), "pluma-llm-mock");
        // No probamos el eco aquí (ya lo cubre pluma-llm-mock); solo
        // que el cliente está vivo.
    }

    #[test]
    fn build_ollama_sin_model_es_config_error() {
        let cfg = LlmConfig {
            kind: BackendKind::Ollama,
            model: None,
            ..Default::default()
        };
        // No usamos `panic!("{otro:?}")` porque `Arc<dyn ChatClient>` no
        // implementa Debug; matcheamos por discriminante.
        match build_client(&cfg) {
            Err(BuildError::Config(msg)) => assert!(msg.contains("Ollama")),
            Err(otro) => panic!("esperaba Config, fue otro error: {otro}"),
            Ok(_) => panic!("esperaba Config error, hubo Ok"),
        }
    }

    #[test]
    fn build_ollama_con_model_y_endpoint_custom() {
        let cfg = LlmConfig {
            kind: BackendKind::Ollama,
            model: Some("llama3.1".into()),
            endpoint: Some("http://10.0.0.5:11434/v1/chat/completions".into()),
            ..Default::default()
        };
        let cli = build_client(&cfg).unwrap();
        assert_eq!(cli.model_id(), "llama3.1");
    }

    #[test]
    fn serde_roundtrip_de_llmconfig() {
        let cfg = LlmConfig {
            kind: BackendKind::Gemini,
            model: Some("gemini-2.5-pro".into()),
            api_key: None,
            endpoint: None,
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let r: LlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(r.kind, BackendKind::Gemini);
        assert_eq!(r.model.as_deref(), Some("gemini-2.5-pro"));
    }
}
