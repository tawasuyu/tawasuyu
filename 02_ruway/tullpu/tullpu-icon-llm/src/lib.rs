//! `tullpu-icon-llm` — el lado **IA** del generador híbrido.
//!
//! Una descripción en lenguaje natural (`"un sobre de correo"`, `"reloj de
//! arena"`) → un [`IconSpec`]. La clave del diseño: la IA **propone datos** (un
//! `IconSpec` JSON), no dibuja píxeles; el compilador determinista de
//! `tullpu-icon-core` lo materializa en vectores limpios y recolorables. Eso
//! hace barata y segura la generación: si la IA devuelve basura o no hay
//! credenciales, caemos a [`tullpu_icon_core::derivar_spec`] (paramétrico puro).
//!
//! El backend es cualquier `ChatClient` de `pluma-llm` (Anthropic/Gemini/…/Mock).
//! Este crate no construye el cliente — el caller lo arma con
//! `pluma_llm::from_env()` y lo pasa por referencia.
//!
//! ```ignore
//! let chat = pluma_llm::from_env()?;          // cae a Mock sin creds
//! let spec = tullpu_icon_llm::generar(&*chat, "un rayo dentro de un círculo").await;
//! // spec siempre es válido: IA si pudo, derivación paramétrica si no.
//! ```

#![forbid(unsafe_code)]

use pluma_llm_core::{ChatClient, ChatRequest};
use tullpu_icon_core::{derivar_spec, IconSpec};

/// Instrucción de sistema: le enseña al modelo el esquema JSON exacto del
/// `IconSpec` (serde) y las reglas de composición. Pide JSON crudo, sin prosa.
pub const SYSTEM_PROMPT: &str = r#"Sos un generador de íconos vectoriales. Dada una descripción, devolvés EXCLUSIVAMENTE un JSON válido (sin markdown, sin explicación) que describe un ícono sobre una grilla 24×24 (origen arriba-izquierda, Y hacia abajo).

Esquema JSON:
{
  "nombre": "string-corto",
  "lienzo": 24.0,
  "capas": [ { "forma": <FORMA>, "pintura": <PINTURA> }, ... ]
}

FORMA (una de):
  {"Rect":{"x":f,"y":f,"w":f,"h":f}}
  {"RectRedondeado":{"x":f,"y":f,"w":f,"h":f,"r":f}}
  {"Elipse":{"cx":f,"cy":f,"rx":f,"ry":f}}
  {"Circulo":{"cx":f,"cy":f,"r":f}}
  {"PoligonoRegular":{"cx":f,"cy":f,"r":f,"lados":n}}
  {"Estrella":{"cx":f,"cy":f,"r_ext":f,"r_int":f,"puntas":n}}
  {"Linea":{"x1":f,"y1":f,"x2":f,"y2":f}}
  {"Path":{"comandos":[ {"MoverA":{"x":f,"y":f}}, {"LineaA":{"x":f,"y":f}}, {"CurvaA":{"c1x":f,"c1y":f,"c2x":f,"c2y":f,"x":f,"y":f}}, "Cerrar" ]}}

PINTURA (una de):
  {"Relleno": <COLOR>}
  {"Trazo": {"color": <COLOR>, "ancho": f}}
  {"RellenoYTrazo": {"relleno": <COLOR>, "trazo": <COLOR>, "ancho": f}}

COLOR (uno de):
  "Corriente"            (currentColor: lo pinta el tema; usalo para la forma principal)
  {"Rgba":[r,g,b,a]}     (0..255; usalo para acentos de color)

REGLAS:
- Entre 1 y 6 capas, pintadas de atrás hacia adelante.
- La forma/línea principal del símbolo va en "Corriente" (trazo de ancho 1.8–2.2).
- Agregá 1–2 acentos con {"Rgba":[...]} para darle vida (evitá íconos monocromos).
- Mantené todo dentro de 0..24, centrado alrededor de (12,12).
- Geometría simple y reconocible.

Ejemplo para "campana de notificación":
{"nombre":"campana","lienzo":24.0,"capas":[{"forma":{"Path":{"comandos":[{"MoverA":{"x":8.0,"y":17.0}},{"LineaA":{"x":16.0,"y":17.0}}]}},"pintura":{"Trazo":{"color":"Corriente","ancho":2.0}}},{"forma":{"Circulo":{"cx":16.0,"cy":7.0,"r":2.2}},"pintura":{"Relleno":{"Rgba":[229,85,106,255]}}}]}"#;

/// Errores de generación por IA. No los expone [`generar`] (que siempre cae a
/// derivación), pero sí [`generar_estricto`] para quien quiera distinguir.
#[derive(Debug)]
pub enum Error {
    /// El backend de chat falló (red, credenciales, etc.).
    Chat(String),
    /// La respuesta no era un `IconSpec` JSON parseable.
    Parse(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Chat(e) => write!(f, "fallo del backend de chat: {e}"),
            Error::Parse(e) => write!(f, "respuesta no parseable a IconSpec: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Quita cercas de markdown (```json … ```) y recorta a las llaves externas,
/// para tolerar modelos que envuelven el JSON en prosa o fences.
fn extraer_json(s: &str) -> &str {
    let s = s.trim();
    // Recorta al primer '{' y al último '}' — el objeto JSON externo.
    match (s.find('{'), s.rfind('}')) {
        (Some(i), Some(j)) if j >= i => &s[i..=j],
        _ => s,
    }
}

/// Genera un `IconSpec` por IA, devolviendo error si el backend o el parseo
/// fallan. La descripción se pasa tal cual; el esquema lo fija [`SYSTEM_PROMPT`].
pub async fn generar_estricto(chat: &dyn ChatClient, descripcion: &str) -> Result<IconSpec, Error> {
    let req = ChatRequest::una_vuelta(descripcion.to_string(), 900)
        .con_sistema(SYSTEM_PROMPT)
        .con_temperatura(0.3);
    let resp = chat.complete(&req).await.map_err(|e| Error::Chat(e.to_string()))?;
    let json = extraer_json(&resp.content);
    serde_json::from_str::<IconSpec>(json).map_err(|e| Error::Parse(e.to_string()))
}

/// Genera un `IconSpec` para `descripcion`, **siempre** con éxito: intenta por
/// IA y, si el backend o el parseo fallan (p.ej. backend Mock sin credenciales),
/// cae a [`derivar_spec`] (paramétrico determinista sobre la descripción). Es el
/// camino híbrido recomendado.
pub async fn generar(chat: &dyn ChatClient, descripcion: &str) -> IconSpec {
    match generar_estricto(chat, descripcion).await {
        Ok(spec) => spec,
        Err(_) => derivar_spec(descripcion),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pluma_llm_core::{ChatError, ChatResponse};

    /// ChatClient de prueba que devuelve un texto fijo (simula un LLM).
    struct FakeChat {
        salida: String,
    }

    #[async_trait]
    impl ChatClient for FakeChat {
        fn model_id(&self) -> &str {
            "fake"
        }
        async fn complete(&self, _req: &ChatRequest) -> Result<ChatResponse, ChatError> {
            Ok(ChatResponse { content: self.salida.clone(), stop_reason: None, usage: None })
        }
    }

    const SPEC_OK: &str = r#"```json
{"nombre":"rayo","lienzo":24.0,"capas":[
  {"forma":{"Circulo":{"cx":12.0,"cy":12.0,"r":9.0}},"pintura":{"Trazo":{"color":"Corriente","ancho":2.0}}},
  {"forma":{"Path":{"comandos":[{"MoverA":{"x":13.0,"y":5.0}},{"LineaA":{"x":9.0,"y":13.0}},{"LineaA":{"x":12.0,"y":13.0}},{"LineaA":{"x":11.0,"y":19.0}},{"LineaA":{"x":16.0,"y":10.0}},{"LineaA":{"x":12.0,"y":10.0}},"Cerrar"]}},"pintura":{"Relleno":{"Rgba":[245,197,66,255]}}}
]}
```"#;

    #[tokio::test]
    async fn parsea_respuesta_ia_con_fences() {
        let chat = FakeChat { salida: SPEC_OK.to_string() };
        let spec = generar_estricto(&chat, "un rayo en un círculo").await.expect("parsea");
        assert_eq!(spec.nombre, "rayo");
        assert_eq!(spec.capas.len(), 2);
    }

    #[tokio::test]
    async fn basura_cae_a_derivacion_y_no_falla() {
        let chat = FakeChat { salida: "perdón, no puedo ayudar con eso".to_string() };
        // generar_estricto falla…
        assert!(generar_estricto(&chat, "lo que sea").await.is_err());
        // …pero generar SIEMPRE da un spec (derivación paramétrica).
        let spec = generar(&chat, "lo que sea").await;
        assert!(!spec.capas.is_empty());
    }

    #[test]
    fn extraer_json_recorta_prosa() {
        assert_eq!(extraer_json("blah {\"a\":1} fin"), "{\"a\":1}");
        assert_eq!(extraer_json("```json\n{\"x\":2}\n```"), "{\"x\":2}");
    }
}
