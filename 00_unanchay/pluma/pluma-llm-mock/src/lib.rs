//! `pluma-llm-mock` — backend LLM determinista para tests.
//!
//! No habla con ningún servicio. Tiene dos modos:
//!
//! 1. **Tabla**: el caller registra `(prompt_substr, respuesta)` y el
//!    mock devuelve la respuesta cuyo `prompt_substr` aparece primero en
//!    el último `ChatMessage::user` de la request. Útil cuando se sabe
//!    qué prompts esperar (tests de integración de `pluma-transform-llm`).
//!
//! 2. **Eco**: si nada coincide, devuelve la última request del usuario
//!    prefijada con un string configurable (default `"mock:: "`). Eso
//!    permite que un test que olvidó preparar la tabla aún produzca
//!    salida razonable y deterministica.
//!
//! No simula latencia, no falla aleatoriamente, no consume tokens. La
//! contabilidad reportada en `ChatUsage` es siempre 0.

#![forbid(unsafe_code)]

use async_trait::async_trait;
use pluma_llm_core::{
    ChatClient, ChatError, ChatRequest, ChatResponse, ChatUsage, Role, StopReason,
};

/// Cliente LLM mock. Cero red, cero latencia, salida deterministica.
pub struct MockChatClient {
    /// Pares `(substring_a_buscar, respuesta)`. La primera que coincida
    /// en el último mensaje user gana. Orden importa.
    tabla: Vec<(String, String)>,
    /// Prefijo del eco para prompts no cubiertos por la tabla.
    eco_prefix: String,
    /// `model_id` reportado — útil para distinguir varios mocks en una
    /// suite de tests.
    model_id: String,
}

impl Default for MockChatClient {
    fn default() -> Self {
        Self {
            tabla: Vec::new(),
            eco_prefix: "mock:: ".to_string(),
            model_id: "pluma-llm-mock".to_string(),
        }
    }
}

impl MockChatClient {
    /// Constructor encadenable: registra un par. `substring` se busca en
    /// el último `ChatMessage::user` del request — el primero que matchee
    /// gana, en orden de registro.
    pub fn con_respuesta(
        mut self,
        substring: impl Into<String>,
        respuesta: impl Into<String>,
    ) -> Self {
        self.tabla.push((substring.into(), respuesta.into()));
        self
    }

    /// Cambia el prefijo de eco — útil para que un test vea de qué
    /// mock viene la salida cuando hay varios.
    pub fn con_eco_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.eco_prefix = prefix.into();
        self
    }

    /// Anota un `model_id` distinto del default.
    pub fn con_model_id(mut self, id: impl Into<String>) -> Self {
        self.model_id = id.into();
        self
    }

    /// Devuelve el último mensaje user del request, o `""` si no hay.
    /// Helper para tests que quieran inspeccionar qué se le pidió.
    fn ultimo_user(req: &ChatRequest) -> &str {
        req.messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_str())
            .unwrap_or("")
    }
}

#[async_trait]
impl ChatClient for MockChatClient {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    async fn complete(&self, req: &ChatRequest) -> Result<ChatResponse, ChatError> {
        let prompt = Self::ultimo_user(req);
        // Buscar coincidencia en la tabla.
        let content = self
            .tabla
            .iter()
            .find(|(needle, _)| prompt.contains(needle))
            .map(|(_, resp)| resp.clone())
            .unwrap_or_else(|| format!("{}{prompt}", self.eco_prefix));

        Ok(ChatResponse {
            content,
            stop_reason: Some(StopReason("mock_end".to_string())),
            usage: Some(ChatUsage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            }),
        })
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_core::ChatMessage;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn tabla_gana_sobre_eco() {
        let cli = MockChatClient::default()
            .con_respuesta("traducir", "TRADUCCIÓN_DUMMY");
        let req = ChatRequest::una_vuelta("por favor traducir esto al qu", 50);
        let resp = rt().block_on(cli.complete(&req)).unwrap();
        assert_eq!(resp.content, "TRADUCCIÓN_DUMMY");
    }

    #[test]
    fn eco_cae_cuando_no_hay_coincidencia() {
        let cli = MockChatClient::default();
        let req = ChatRequest::una_vuelta("hola mundo", 50);
        let resp = rt().block_on(cli.complete(&req)).unwrap();
        assert_eq!(resp.content, "mock:: hola mundo");
    }

    #[test]
    fn primer_match_de_la_tabla_gana() {
        let cli = MockChatClient::default()
            .con_respuesta("alfa", "PRIMERO")
            .con_respuesta("beta", "SEGUNDO");
        let req = ChatRequest::una_vuelta("alfa beta", 50);
        let resp = rt().block_on(cli.complete(&req)).unwrap();
        assert_eq!(resp.content, "PRIMERO");
    }

    #[test]
    fn model_id_y_eco_prefix_configurables() {
        let cli = MockChatClient::default()
            .con_model_id("test-2")
            .con_eco_prefix("[ECO] ");
        assert_eq!(cli.model_id(), "test-2");
        let req = ChatRequest::una_vuelta("xyz", 1);
        let resp = rt().block_on(cli.complete(&req)).unwrap();
        assert_eq!(resp.content, "[ECO] xyz");
    }

    #[test]
    fn usa_el_ultimo_mensaje_user_aunque_haya_assistant_intercalado() {
        let cli = MockChatClient::default().con_respuesta("DOS", "B");
        let req = ChatRequest {
            system: None,
            max_tokens: 10,
            temperature: 0.0,
            messages: vec![
                ChatMessage::user("UNO"),
                ChatMessage::assistant("ASS"),
                ChatMessage::user("DOS"),
            ],
        };
        let resp = rt().block_on(cli.complete(&req)).unwrap();
        assert_eq!(resp.content, "B");
    }
}
