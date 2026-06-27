//! El agente: una configuración de IA con identidad, backend y permisos.

use serde::{Deserialize, Serialize};

/// Un agente IA configurable. Es la unidad que el usuario crea/edita en el
/// wawapanel: a qué proveedor pega, con qué persona, y qué puede hacer.
///
/// El `backend` es un [`wawa_config::LlmSettings`] **propio del agente** — así
/// se pueden mezclar proveedores (un agente Claude, otro Ollama local) sin
/// tocar el `[ai.llm]` global del SO. Si `backend.is_set()` es `false`, el host
/// hereda el backend global (resolución por `from_env`), igual que hoy.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Agente {
    /// Identificador estable (uuid v4). No cambia al renombrar.
    pub id: String,
    /// Nombre visible: "Asistente", "DevOps", "Traductor"…
    pub nombre: String,
    /// Una línea de qué es/para qué sirve (se muestra en el selector).
    #[serde(default)]
    pub descripcion: String,
    /// Backend propio: proveedor + modelo + API key + endpoint. `backend`
    /// vacío = heredar el `[ai.llm]` global del SO.
    #[serde(default)]
    pub backend: wawa_config::LlmSettings,
    /// Instrucción de sistema (persona/rol). Vacío = persona genérica.
    #[serde(default)]
    pub system_prompt: String,
    /// Determinismo 0.0–1.0 (bajo para tareas técnicas, alto para creativo).
    #[serde(default = "temperatura_default")]
    pub temperatura: f32,
    /// Tope de tokens de salida por turno.
    #[serde(default = "max_tokens_default")]
    pub max_tokens: u32,
    /// Qué acciones de control puede **proponer** el agente.
    #[serde(default)]
    pub capacidades: Capacidades,
    /// Color de acento (hex `#rrggbb`) para la UI; `None` = el del theme.
    #[serde(default)]
    pub color: Option<String>,
}

fn temperatura_default() -> f32 {
    0.4
}
fn max_tokens_default() -> u32 {
    1024
}

impl Agente {
    /// Un agente nuevo con `id` aleatorio y defaults razonables.
    pub fn nuevo(nombre: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            nombre: nombre.into(),
            descripcion: String::new(),
            backend: wawa_config::LlmSettings::default(),
            system_prompt: String::new(),
            temperatura: temperatura_default(),
            max_tokens: max_tokens_default(),
            capacidades: Capacidades::default(),
            color: None,
        }
    }

    /// Encadenable: fija la persona (system prompt).
    pub fn con_persona(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = system_prompt.into();
        self
    }

    /// Encadenable: fija el backend del agente.
    pub fn con_backend(mut self, backend: wawa_config::LlmSettings) -> Self {
        self.backend = backend;
        self
    }

    /// Encadenable: habilita las acciones de control del escritorio.
    pub fn con_control(mut self) -> Self {
        self.capacidades.control = true;
        self
    }

    /// Encadenable: descripción corta.
    pub fn con_descripcion(mut self, d: impl Into<String>) -> Self {
        self.descripcion = d.into();
        self
    }
}

/// Qué acciones de control (atipay) puede **proponer** el agente. Nunca ejecuta
/// solo: propone una [`crate::AccionPropuesta`] y el usuario aprueba (la misma
/// doctrina de `:hacé`). Sin `control`, el agente es sólo-charla.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Capacidades {
    /// Si `false`, el agente no propone acciones (charla pura).
    #[serde(default)]
    pub control: bool,
    /// Lista blanca de superficies atipay permitidas, por prefijo
    /// (`"mirada"`, `"sistema"`, `"sandokan"`, `"shuma"`). Vacío = todas las del
    /// catálogo estándar. Una acción cuya superficie no esté acá se rechaza al
    /// interpretarla, aunque el modelo la haya elegido.
    #[serde(default)]
    pub superficies: Vec<String>,
}

impl Capacidades {
    /// `true` si la superficie con este prefijo está permitida (lista blanca
    /// vacía = todo permitido).
    pub fn permite_superficie(&self, prefijo: &str) -> bool {
        self.superficies.is_empty() || self.superficies.iter().any(|s| s == prefijo)
    }
}
