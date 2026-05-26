//! `pluma-notebook-kernel-llm` — kernel de notebook que delega en un
//! `pluma_llm_core::ChatClient`.
//!
//! Conecta dos mundos de la suite que viven en universos paralelos:
//! el notebook (DAG de celdas, ejecución reactiva, digests) y el LLM
//! transparente (Anthropic/Gemini/DeepSeek/Ollama/Mock detrás de un
//! trait). Una celda con `language = "llm-traducir-qu"` y source
//! `"El cóndor cruzó el cielo."` produce un output con el texto
//! traducido.
//!
//! ## Lenguajes reconocidos
//!
//! | `language`             | Qué hace                                           |
//! |------------------------|----------------------------------------------------|
//! | `llm-prompt`           | El source ES el prompt completo. Sin system.       |
//! | `llm-traducir-{LANG}`  | Traduce el source a `{LANG}` (qu, en, fr...).      |
//! | `llm-tono-{ETIQUETA}`  | Reescribe con tono (formal, casual, infantil...).  |
//! | `llm-resumir[-N]`      | Resume; si `N`, objetivo de palabras.              |
//! | `llm-reescribir`       | Primera línea del source = prompt; resto = texto.  |
//!
//! Cualquier otra `language` devuelve `KernelError::Runtime` con
//! mensaje claro. Los kernels reales (Python, WASM) se montan en
//! paralelo y un notebook mezcla celdas de varios.
//!
//! ## Cómo encaja
//!
//! El factory `pluma_llm::build_client(&cfg)` produce el `Arc<dyn ChatClient>`
//! que este kernel envuelve. Cambiar de IA = cambiar la config; el
//! notebook entero se ejecuta contra el backend de turno sin tocar las
//! celdas. Idéntica idea que el ejecutor `pluma-transform-llm` pero
//! del lado notebook.

#![forbid(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use pluma_llm_core::{ChatClient, ChatRequest};
use pluma_notebook_core::cell::{CellOutput, OutputPayload};
use pluma_notebook_exec::{Kernel, KernelError, KernelOutput};

/// Kernel de notebook respaldado por un `ChatClient`.
pub struct LlmKernel {
    chat: Arc<dyn ChatClient>,
    max_tokens_default: u32,
    temperature_default: f32,
}

impl LlmKernel {
    /// Construye desde cualquier `ChatClient` concreto.
    pub fn new<C: ChatClient + 'static>(chat: C) -> Self {
        Self::from_arc(Arc::new(chat))
    }

    /// Construye desde un `Arc<dyn ChatClient>` — flujo natural con el
    /// factory `pluma_llm::build_client`.
    pub fn from_arc(chat: Arc<dyn ChatClient>) -> Self {
        Self {
            chat,
            max_tokens_default: 1024,
            temperature_default: 0.3,
        }
    }

    /// Encadenable: cota de tokens por celda. Default 1024.
    pub fn con_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens_default = n;
        self
    }

    /// Encadenable: temperatura por defecto. Las acciones específicas
    /// pueden sobrescribirla — `llm-traducir-*` usa 0.1, `llm-reescribir`
    /// 0.6.
    pub fn con_temperatura(mut self, t: f32) -> Self {
        self.temperature_default = t.clamp(0.0, 1.0);
        self
    }
}

#[async_trait]
impl Kernel for LlmKernel {
    async fn execute(
        &self,
        source: &str,
        language: &str,
    ) -> Result<KernelOutput, KernelError> {
        let accion = parsear_language(language).ok_or_else(|| {
            KernelError::Runtime(format!(
                "language no soportado por LlmKernel: '{language}'. \
                 Soportados: llm-prompt | llm-traducir-X | llm-tono-X | \
                 llm-resumir[-N] | llm-reescribir"
            ))
        })?;

        let (system, user, temperatura) = construir_prompt(&accion, source);
        let mut req = ChatRequest::una_vuelta(user, self.max_tokens_default)
            .con_temperatura(temperatura.unwrap_or(self.temperature_default));
        if let Some(s) = system {
            req = req.con_sistema(s);
        }

        let resp = self
            .chat
            .complete(&req)
            .await
            .map_err(|e| KernelError::Runtime(format!("LLM: {e}")))?;
        let texto = resp.content.trim().to_string();
        Ok(CellOutput {
            stdout: texto.clone(),
            value: Some(format!("{}/{:?}", self.chat.model_id(), accion)),
            payload: OutputPayload::Text(texto),
        })
    }
}

/// Acciones que entiende este kernel. Independiente del backend: solo
/// describe QUÉ hacer; el cliente concreto lo lleva el LlmKernel.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Accion {
    /// El source es el prompt completo. Sin system, sin transformación
    /// de envoltorio. Útil para experimentación rápida.
    Prompt,
    /// Traducir a una lengua dada.
    Traducir { lengua: String },
    /// Reescribir con un tono.
    Tono { etiqueta: String },
    /// Resumir, opcionalmente a un número de palabras.
    Resumir { palabras: Option<u32> },
    /// Reescribir: primera línea del source es el prompt, resto el texto.
    Reescribir,
}

fn parsear_language(language: &str) -> Option<Accion> {
    if language == "llm-prompt" {
        return Some(Accion::Prompt);
    }
    if let Some(rest) = language.strip_prefix("llm-traducir-") {
        if rest.is_empty() {
            return None;
        }
        return Some(Accion::Traducir { lengua: rest.to_string() });
    }
    if let Some(rest) = language.strip_prefix("llm-tono-") {
        if rest.is_empty() {
            return None;
        }
        return Some(Accion::Tono { etiqueta: rest.to_string() });
    }
    if language == "llm-resumir" {
        return Some(Accion::Resumir { palabras: None });
    }
    if let Some(rest) = language.strip_prefix("llm-resumir-") {
        // `rest` debe parsear como u32; si no, language inválido.
        return rest
            .parse::<u32>()
            .ok()
            .map(|n| Accion::Resumir { palabras: Some(n) });
    }
    if language == "llm-reescribir" {
        return Some(Accion::Reescribir);
    }
    None
}

/// Compone (system, user, temperatura) para cada acción. La temperatura
/// es opcional — `None` deja al kernel usar su default.
fn construir_prompt(accion: &Accion, source: &str) -> (Option<String>, String, Option<f32>) {
    match accion {
        Accion::Prompt => (None, source.to_string(), None),
        Accion::Traducir { lengua } => (
            Some(format!(
                "Eres un traductor profesional al {lengua}. Traduce con \
                 precisión el texto que el usuario te pase. Conserva nombres \
                 propios, números y formato. NO agregues comentario, NO \
                 prefijes la respuesta, NO uses comillas. Devuelve SOLO el \
                 texto traducido."
            )),
            source.to_string(),
            Some(0.1),
        ),
        Accion::Tono { etiqueta } => (
            Some(format!(
                "Reescribes cada texto con tono {etiqueta}, conservando \
                 significado, nombres propios y números. NO agregues \
                 comentario, NO uses comillas, NO prefijes. Devuelve SOLO \
                 el texto reescrito."
            )),
            source.to_string(),
            Some(0.4),
        ),
        Accion::Resumir { palabras } => {
            let n = palabras
                .map(|n| format!("aproximadamente {n} palabras"))
                .unwrap_or_else(|| "lo más conciso posible".to_string());
            (
                Some(format!(
                    "Resumes el texto a {n}, conservando hechos y nombres \
                     propios clave. NO agregues comentario, NO prefijes, NO \
                     uses comillas. Devuelve SOLO el resumen."
                )),
                source.to_string(),
                Some(0.2),
            )
        }
        Accion::Reescribir => {
            // Primera línea = prompt; el resto = texto.
            let mut it = source.splitn(2, '\n');
            let prompt = it.next().unwrap_or("").trim();
            let texto = it.next().unwrap_or("").trim();
            (
                Some(format!(
                    "Sigue la instrucción al pie de la letra para el texto \
                     que recibes. Instrucción: \"{prompt}\". NO agregues \
                     comentario, NO prefijes, NO uses comillas. Devuelve \
                     SOLO el texto resultado."
                )),
                texto.to_string(),
                Some(0.6),
            )
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_mock::MockChatClient;

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn parsear_language_cubre_los_lenguajes_soportados() {
        assert_eq!(parsear_language("llm-prompt"), Some(Accion::Prompt));
        assert_eq!(
            parsear_language("llm-traducir-qu"),
            Some(Accion::Traducir { lengua: "qu".into() })
        );
        assert_eq!(
            parsear_language("llm-tono-formal"),
            Some(Accion::Tono { etiqueta: "formal".into() })
        );
        assert_eq!(
            parsear_language("llm-resumir"),
            Some(Accion::Resumir { palabras: None })
        );
        assert_eq!(
            parsear_language("llm-resumir-50"),
            Some(Accion::Resumir { palabras: Some(50) })
        );
        assert_eq!(parsear_language("llm-reescribir"), Some(Accion::Reescribir));

        // Inválidos.
        assert_eq!(parsear_language("python"), None);
        assert_eq!(parsear_language("llm-traducir-"), None);
        assert_eq!(parsear_language("llm-tono-"), None);
        assert_eq!(parsear_language("llm-resumir-abc"), None);
    }

    #[test]
    fn traducir_pide_al_chat_y_devuelve_payload_text() {
        let chat = MockChatClient::default().con_respuesta("hola", "huk");
        let kernel = LlmKernel::new(chat);
        let salida = rt()
            .block_on(kernel.execute("hola", "llm-traducir-qu"))
            .unwrap();
        match salida.payload {
            OutputPayload::Text(t) => assert_eq!(t, "huk"),
            otro => panic!("esperaba Text, fue {otro:?}"),
        }
    }

    #[test]
    fn reescribir_separa_prompt_y_texto_por_la_primera_nueva_linea() {
        // La closure del mock no ve el system; matchea sobre el último
        // user message — que en este path es el TEXTO sin la primera línea.
        let chat = MockChatClient::default()
            .con_respuesta("texto del cuerpo", "REESCRITO");
        let kernel = LlmKernel::new(chat);
        let source = "Convierte a sarcasmo\ntexto del cuerpo";
        let salida = rt()
            .block_on(kernel.execute(source, "llm-reescribir"))
            .unwrap();
        assert_eq!(salida.stdout, "REESCRITO");
    }

    #[test]
    fn language_no_soportado_es_runtime_error() {
        let kernel = LlmKernel::new(MockChatClient::default());
        let err = rt()
            .block_on(kernel.execute("x", "ruby"))
            .unwrap_err();
        match err {
            KernelError::Runtime(msg) => assert!(msg.contains("no soportado")),
        }
    }

    #[test]
    fn prompt_libre_no_pone_system() {
        // Verificamos vía el helper construir_prompt directamente: el
        // contrato es "Prompt no lleva system".
        let (sys, user, temp) = construir_prompt(&Accion::Prompt, "qué es la vida");
        assert!(sys.is_none());
        assert_eq!(user, "qué es la vida");
        assert!(temp.is_none());
    }

    #[test]
    fn traducir_fuerza_temperatura_baja() {
        let (_, _, temp) = construir_prompt(
            &Accion::Traducir { lengua: "qu".into() },
            "x",
        );
        assert_eq!(temp, Some(0.1));
    }

    #[test]
    fn resumir_con_palabras_objetivo_lo_anota_en_system() {
        let (sys, _, _) = construir_prompt(
            &Accion::Resumir { palabras: Some(50) },
            "texto",
        );
        let s = sys.expect("hay system");
        assert!(s.contains("50 palabras"));
    }
}
