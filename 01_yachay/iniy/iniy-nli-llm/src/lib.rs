//! iniy-nli-llm — backend NLI vía LLM.
//!
//! Implementa `iniy_nli::MotorNli` sobre cualquier `pluma_llm_core::ChatClient`
//! (Anthropic, Gemini, DeepSeek, Ollama, Mock — los cinco backends de `pluma-llm`).
//! El system prompt es estable e idéntico en todas las requests del lote para
//! que el prompt-caching (Anthropic / DeepSeek server-side) lo amortice.
//!
//! Costo: 1 request por par. Para corpus grandes, considerar pre-filtrar con
//! `iniy_nli::MotorNliLexico` y solo enviar al LLM los pares por encima de un
//! umbral léxico.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use iniy_core::{Asercion, RelacionNli};
use iniy_nli::MotorNli;
use pluma_llm_core::{ChatClient, ChatRequest};
use serde::Deserialize;
use std::sync::Arc;

const SYSTEM_PROMPT: &str = r#"Eres un evaluador de Natural Language Inference (NLI) en español.

Recibirás dos oraciones marcadas con PREMISA e HIPÓTESIS y debes decidir la relación lógica entre ellas:
- entailment: la premisa, si es verdadera, IMPLICA que la hipótesis también lo es.
- contradiction: la premisa, si es verdadera, IMPLICA que la hipótesis es FALSA.
- neutral: la premisa no implica nada sobre la hipótesis (puede ser verdadera o falsa independientemente).

Tu respuesta DEBE ser ÚNICAMENTE un objeto JSON con TRES campos numéricos en [0,1] que sumen aproximadamente 1.0:
{"entailment": float, "contradiction": float, "neutral": float}

REGLAS ESTRICTAS:
- NO incluyas explicación, prefijo, sufijo, comillas adicionales, markdown ni "```json".
- Solo el JSON, en una línea.
- Si los tres valores no suman 1.0 exactamente, está bien — el cliente los normaliza.
- Considera la semántica, no solo el solapamiento léxico. "Sin duda" y "es evidente" suben entailment. "Jamás" frente a "siempre" sobre el mismo sujeto sube contradiction. Si los temas no se relacionan, neutral=1."#;

#[derive(Clone)]
pub struct MotorNliLlm {
    pub chat: Arc<dyn ChatClient>,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl MotorNliLlm {
    pub fn nuevo(chat: Arc<dyn ChatClient>) -> Self {
        Self {
            chat,
            max_tokens: 96,
            temperature: 0.0,
        }
    }

    pub fn con_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }
}

#[async_trait]
impl MotorNli for MotorNliLlm {
    async fn evaluar(&self, premisa: &Asercion, hipotesis: &Asercion) -> Result<RelacionNli> {
        let user = format!(
            "PREMISA: {}\nHIPÓTESIS: {}\n\nJSON:",
            premisa.texto.trim(),
            hipotesis.texto.trim()
        );
        let req = ChatRequest::una_vuelta(user, self.max_tokens)
            .con_sistema(SYSTEM_PROMPT)
            .con_temperatura(self.temperature);
        let resp = self
            .chat
            .complete(&req)
            .await
            .map_err(|e| anyhow!("LLM falló: {e}"))?;
        match parsear_rel(&resp.content) {
            Ok(r) => Ok(normalizar(r)),
            Err(e) => {
                tracing::warn!(model = self.chat.model_id(), error = %e, content = %resp.content, "parse NLI falló; usando neutral");
                Ok(RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 })
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RelJson {
    entailment: f32,
    contradiction: f32,
    neutral: f32,
}

/// Extrae el primer JSON `{...}` del texto. Tolera prefijo/sufijo de texto y
/// también wrappers `\`\`\`json … \`\`\``.
pub fn parsear_rel(s: &str) -> Result<RelacionNli> {
    let inicio = s
        .find('{')
        .with_context(|| format!("respuesta sin '{{': {s:?}"))?;
    let fin = s
        .rfind('}')
        .with_context(|| format!("respuesta sin '}}': {s:?}"))?;
    if fin <= inicio {
        return Err(anyhow!("braces invertidos en {s:?}"));
    }
    let json = &s[inicio..=fin];
    let r: RelJson = serde_json::from_str(json)
        .with_context(|| format!("JSON inválido tras extracción: {json:?}"))?;
    Ok(RelacionNli {
        entailment: r.entailment.clamp(0.0, 1.0),
        contradiction: r.contradiction.clamp(0.0, 1.0),
        neutral: r.neutral.clamp(0.0, 1.0),
    })
}

/// Si la suma no es ~1.0, normaliza proporcionalmente. Si los tres son 0,
/// resuelve a neutral=1.0.
pub fn normalizar(r: RelacionNli) -> RelacionNli {
    let s = r.entailment + r.contradiction + r.neutral;
    if s < 1e-6 {
        return RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 1.0 };
    }
    RelacionNli {
        entailment: r.entailment / s,
        contradiction: r.contradiction / s,
        neutral: r.neutral / s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iniy_core::{AsercionId, ChunkId, DocId, Opinion};
    use pluma_llm_mock::MockChatClient;

    fn asercion(t: &str) -> Asercion {
        Asercion {
            id: AsercionId::nuevo(),
            doc_id: DocId::nuevo(),
            chunk_id: ChunkId::nuevo(),
            texto: t.into(),
            opinion_autoral: Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap(),
        }
    }

    #[test]
    fn parsear_rel_acepta_json_puro() {
        let r = parsear_rel(r#"{"entailment": 0.8, "contradiction": 0.1, "neutral": 0.1}"#).unwrap();
        assert!((r.entailment - 0.8).abs() < 1e-5);
    }

    #[test]
    fn parsear_rel_tolera_prefijo_y_sufijo() {
        let r = parsear_rel("Aquí va: {\"entailment\":0.7,\"contradiction\":0.2,\"neutral\":0.1} listo").unwrap();
        assert!((r.entailment - 0.7).abs() < 1e-5);
        assert!((r.contradiction - 0.2).abs() < 1e-5);
    }

    #[test]
    fn parsear_rel_tolera_wrapper_markdown() {
        let r = parsear_rel("```json\n{\"entailment\":0.0,\"contradiction\":0.9,\"neutral\":0.1}\n```").unwrap();
        assert!((r.contradiction - 0.9).abs() < 1e-5);
    }

    #[test]
    fn normalizar_reescala_si_no_suma_uno() {
        let r = normalizar(RelacionNli { entailment: 2.0, contradiction: 1.0, neutral: 1.0 });
        assert!((r.entailment - 0.5).abs() < 1e-5);
        assert!((r.contradiction - 0.25).abs() < 1e-5);
        assert!((r.neutral - 0.25).abs() < 1e-5);
    }

    #[test]
    fn normalizar_resuelve_ceros_a_neutral() {
        let r = normalizar(RelacionNli { entailment: 0.0, contradiction: 0.0, neutral: 0.0 });
        assert_eq!(r.neutral, 1.0);
    }

    #[tokio::test]
    async fn motor_llm_con_mock_devuelve_lo_predicho() {
        // El mock matchea por substring del USER message; el system se ignora
        // en el matching del mock pero igual se envía.
        let mock = MockChatClient::default()
            .con_respuesta("PREMISA", r#"{"entailment": 0.0, "contradiction": 0.9, "neutral": 0.1}"#);
        let motor = MotorNliLlm::nuevo(Arc::new(mock));
        let r = motor
            .evaluar(&asercion("El sol sale siempre"), &asercion("El sol jamás sale"))
            .await
            .unwrap();
        assert!((r.contradiction - 0.9).abs() < 1e-5);
    }

    #[tokio::test]
    async fn motor_llm_cae_a_neutral_si_respuesta_no_parsea() {
        let mock = MockChatClient::default().con_respuesta("PREMISA", "esto no es JSON, lo lamento");
        let motor = MotorNliLlm::nuevo(Arc::new(mock));
        let r = motor
            .evaluar(&asercion("X"), &asercion("Y"))
            .await
            .unwrap();
        assert_eq!(r.neutral, 1.0);
    }
}
