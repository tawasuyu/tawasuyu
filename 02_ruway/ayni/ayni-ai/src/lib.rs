// =============================================================================
//  ayni :: ayni-ai — multilienzo del chat
// -----------------------------------------------------------------------------
//  `proponer(mensaje, intencion, chat)` devuelve un texto DERIVADO —traducción,
//  resumen, cambio de tono— que la app le muestra al humano. La regla es
//  "máquina propone, humano firma": esta función NO toca el grafo ni envía nada;
//  sólo redacta. Si el humano acepta, la app convierte el texto en un nodo
//  firmado por él (un mensaje suyo), nunca por la IA.
//
//  Detrás está la fachada `pluma-llm`: con credenciales usa el backend real;
//  sin ellas cae a Mock determinista (los tests y el arranque sin API key
//  funcionan igual).
// =============================================================================

use std::sync::Arc;

use pluma_llm_core::{ChatClient, ChatRequest};

/// Qué lienzo derivado proponer sobre un mensaje.
pub enum Intencion {
    /// Traducir a un idioma (nombre o código: "inglés", "en", "quechua"…).
    Traducir { idioma: String },
    /// Resumir, opcionalmente apuntando a un número de palabras.
    Resumir { palabras: Option<u32> },
    /// Reescribir en un tono dado ("formal", "cálido", "conciso"…).
    Tono { etiqueta: String },
    /// Reescribir según una instrucción libre.
    Reescribir { instruccion: String },
}

/// Falla al proponer un lienzo.
#[derive(Debug, thiserror::Error)]
pub enum ErrorAi {
    /// El backend LLM falló (sin credenciales válidas, red caída, rate-limit…).
    #[error("ayni-ai :: el LLM falló: {0}")]
    Llm(String),
}

impl Intencion {
    /// El system prompt + el techo de tokens para esta intención. Los prompts
    /// espejan los de `pluma-transform-llm`: instruir a devolver SÓLO el texto
    /// resultante, sin preámbulo ni comillas.
    fn receta(&self) -> (String, u32) {
        match self {
            Intencion::Traducir { idioma } => (
                format!(
                    "Eres un traductor profesional al {idioma}. Traduce con precisión \
                     el texto del usuario. Conserva nombres propios, números y formato. \
                     NO agregues comentario, NO prefijes, NO uses comillas. Devuelve SOLO \
                     la traducción."
                ),
                1024,
            ),
            Intencion::Resumir { palabras } => {
                let meta = palabras
                    .map(|n| format!(" en aproximadamente {n} palabras"))
                    .unwrap_or_default();
                (
                    format!(
                        "Eres un sintetizador preciso. Resume{meta} el texto del usuario, \
                         conservando lo esencial. Devuelve SOLO el resumen, sin preámbulo."
                    ),
                    512,
                )
            }
            Intencion::Tono { etiqueta } => (
                format!(
                    "Reescribe el texto del usuario en tono {etiqueta}, conservando su \
                     significado. Devuelve SOLO el texto reescrito, sin comentario."
                ),
                1024,
            ),
            Intencion::Reescribir { instruccion } => (
                format!(
                    "Reescribe el texto del usuario según esta instrucción: {instruccion}. \
                     Devuelve SOLO el resultado, sin comentario."
                ),
                1024,
            ),
        }
    }
}

/// Propone un lienzo derivado de `mensaje` según `intencion`, usando `chat`.
/// Devuelve sólo el texto propuesto —la máquina propone; firmar y enviar es del
/// humano—. Temperatura baja para que la propuesta sea fiel, no creativa.
pub async fn proponer(
    mensaje: &str,
    intencion: &Intencion,
    chat: &dyn ChatClient,
) -> Result<String, ErrorAi> {
    let (system, max_tokens) = intencion.receta();
    let req = ChatRequest::una_vuelta(mensaje, max_tokens)
        .con_sistema(system)
        .con_temperatura(0.2);
    let resp = chat
        .complete(&req)
        .await
        .map_err(|e| ErrorAi::Llm(format!("{e:?}")))?;
    Ok(resp.content.trim().to_string())
}

/// El cliente LLM por defecto: autodetecta backend por entorno y cae a Mock sin
/// credenciales (ver `pluma_llm::from_env`). Lo que la app usa para no cablear
/// la elección de backend a mano.
pub fn cliente_por_defecto() -> Result<Arc<dyn ChatClient>, ErrorAi> {
    pluma_llm::from_env().map_err(|e| ErrorAi::Llm(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_llm_mock::MockChatClient;

    #[tokio::test]
    async fn propone_traduccion_con_mock() {
        // el Mock responde según substrings del prompt del usuario.
        let mock = MockChatClient::default().con_respuesta("hola", "hello");
        let salida = proponer(
            "hola",
            &Intencion::Traducir { idioma: "inglés".into() },
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(salida, "hello");
    }

    #[tokio::test]
    async fn propone_resumen_con_mock() {
        let mock = MockChatClient::default().con_respuesta("informe largo", "TL;DR breve");
        let salida = proponer(
            "este es un informe largo con muchos detalles",
            &Intencion::Resumir { palabras: Some(5) },
            &mock,
        )
        .await
        .unwrap();
        assert_eq!(salida, "TL;DR breve");
    }

    #[tokio::test]
    async fn cliente_por_defecto_sin_credenciales_es_mock_y_no_explota() {
        // sin API keys en el entorno de test, from_env cae a Mock: proponer
        // devuelve el eco determinista del Mock, no un error.
        let chat = cliente_por_defecto().unwrap();
        let salida = proponer("texto", &Intencion::Resumir { palabras: None }, &*chat)
            .await
            .unwrap();
        assert!(!salida.is_empty(), "el Mock siempre responde algo");
    }
}
