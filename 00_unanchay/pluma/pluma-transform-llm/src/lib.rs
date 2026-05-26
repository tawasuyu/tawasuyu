//! `pluma-transform-llm` — ejecutores de transformación basados en un
//! `pluma_llm_core::ChatClient`.
//!
//! El patrón es siempre el mismo:
//!
//!   1. Para cada átomo de la madre, construir una request con el
//!      `system` prompt fijo (que el backend cachea) y un `user` que
//!      contiene SOLO el texto del párrafo + instrucción mínima.
//!   2. `chat.complete().await` produce el texto nuevo.
//!   3. Llenar `HashMap<Uuid_madre, String>` con las respuestas.
//!   4. Delegar en [`pluma_transform_tabla::EjecutorTraducirTabla`] (para
//!      Traducir) o construir directamente el cuerpo hija (para Tono,
//!      Resumir, Reescribir — usan la misma forma "atom-a-atom").
//!
//! Las cuatro variantes (`EjecutorTraducirLlm`, `EjecutorTonoLlm`,
//! `EjecutorResumirLlm`, `EjecutorReescribirLlm`) comparten 90% del
//! flujo y se factorizan en un helper privado [`aplicar_atom_a_atom`].
//! Cada una difiere en (a) qué `TipoTransformacion` acepta, (b) qué
//! `system` prompt usa, (c) qué `Intencion` lleva la hija, (d) qué
//! sufijo de branch_id le pone.
//!
//! ## Prompt caching
//!
//! Cuando el ChatClient es Anthropic, el system prompt se cachea
//! automáticamente (ver `pluma-llm-anthropic`). Eso amortiza el costo
//! de la primera request: traducir 50 párrafos con system fijo paga el
//! input del system una vez y lo lee cacheado las otras 49.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use pluma_align::{alinear_explicito, OrigenAlineamiento};
use pluma_core::NarrativeAtom;
use pluma_cuerpo::{Cuerpo, Intencion, Lengua};
use pluma_llm_core::{ChatClient, ChatError, ChatRequest};
use pluma_transform::{
    Ejecutor, ErrorEjecutor, ProductoTransformacion, TipoTransformacion, Transformacion,
};

// =============================================================================
//  Helper común: aplicar un transformer atom-a-atom y materializar la hija
// =============================================================================

/// Configuración compartida por todos los ejecutores LLM atom-a-atom.
struct ConfigAplicacion {
    /// Intención que llevará la hija.
    intencion: Intencion,
    /// Sufijo del `branch_id` de la hija (`madre.branch_id-{sufijo}`).
    sufijo_branch: String,
    /// Lengua de la hija — `Some(...)` para Traducir, `None` para Tono/
    /// Resumir/Reescribir (que conservan la lengua de la madre).
    lengua: Option<Lengua>,
    /// System prompt que el modelo recibe. Se mantiene fijo por todas
    /// las requests del lote para que el backend lo cachee.
    system_prompt: String,
    /// Cómo construir el `user` prompt a partir del texto del párrafo.
    /// Cerrada sobre los parámetros específicos del ejecutor (lengua,
    /// etiqueta de tono, palabras objetivo, etc.).
    user_prompt: Box<dyn Fn(&str) -> String + Send + Sync>,
    /// Cota de tokens de salida por átomo. Conservadora para evitar
    /// que el modelo divague.
    max_tokens: u32,
    /// Temperatura: baja para traducir/extraer; alta para reescribir
    /// creativo. La fija el ejecutor.
    temperature: f32,
}

/// Helper compartido: hace el lote contra el ChatClient. Recibe la madre y un
/// índice `Uuid → &NarrativeAtom` para resolver el texto de cada
/// párrafo. Devuelve el `ProductoTransformacion` completo.
async fn ejecutar_lote(
    chat: &dyn ChatClient,
    cfg: &ConfigAplicacion,
    t: &Transformacion,
    madre: &Cuerpo,
    atoms: &HashMap<Uuid, &NarrativeAtom>,
    ahora: u64,
) -> Result<ProductoTransformacion, ErrorEjecutor> {
    if madre.orden.is_empty() {
        return Err(ErrorEjecutor::MadreInvalida(
            "madre vacía — nada que transformar",
        ));
    }
    if !madre.orden.iter().any(|id| atoms.contains_key(id)) {
        return Err(ErrorEjecutor::MadreInvalida(
            "el índice no contiene ningún átomo de la madre",
        ));
    }

    let mut hija = Cuerpo::nuevo(
        format!("{}-{}", madre.branch_id, cfg.sufijo_branch),
        format!(
            "{} ({})",
            madre.metadatos.nombre_legible,
            etiqueta_intencion(&cfg.intencion)
        ),
        cfg.intencion.clone(),
        ahora,
    )
    .deriva_de(madre.id, ahora);
    if let Some(l) = &cfg.lengua {
        hija = hija.con_lengua(l.clone());
    }

    let mut atoms_nuevos: Vec<NarrativeAtom> = Vec::with_capacity(madre.orden.len());
    let mut pares: Vec<(Uuid, Uuid, f32)> = Vec::with_capacity(madre.orden.len());

    for &id_madre in &madre.orden {
        let Some(atom_madre) = atoms.get(&id_madre) else {
            // Madre tiene un Uuid que no está en el índice — saltar.
            // La hija queda con un hueco en esa posición.
            continue;
        };
        let texto_madre = atom_madre.content.as_str();
        let user = (cfg.user_prompt)(texto_madre);
        let req = ChatRequest::una_vuelta(user, cfg.max_tokens)
            .con_sistema(cfg.system_prompt.clone())
            .con_temperatura(cfg.temperature);
        let resp = chat
            .complete(&req)
            .await
            .map_err(mapear_chat_error)?;
        let texto_hija = limpiar_respuesta(&resp.content);

        let atom_hija = NarrativeAtom::new(texto_hija, &hija.branch_id);
        let id_hija = atom_hija.id;
        atoms_nuevos.push(atom_hija);
        hija.agregar(id_hija, ahora);
        pares.push((id_madre, id_hija, 1.0));
    }

    let carta = alinear_explicito(
        madre,
        &hija,
        &pares,
        OrigenAlineamiento::Derivado {
            transformacion: t.id,
            timestamp: ahora,
        },
    );

    Ok(ProductoTransformacion {
        hija,
        atoms_nuevos,
        carta,
    })
}

/// Traduce un error del ChatClient al error del trait Ejecutor.
fn mapear_chat_error(e: ChatError) -> ErrorEjecutor {
    ErrorEjecutor::Backend(format!("LLM: {e}"))
}

/// Recorta espacios alrededor + comillas envolventes que algunos modelos
/// agregan por celo. Modesto: si el modelo respondió con prefijos tipo
/// "Aquí tienes la traducción:", el caller debe ajustar el system prompt
/// — no intentamos parsing heurístico.
fn limpiar_respuesta(s: &str) -> String {
    let t = s.trim();
    if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
        || (t.starts_with('«') && t.ends_with('»'))
    {
        let mut chars: Vec<char> = t.chars().collect();
        chars.pop();
        chars.remove(0);
        chars.into_iter().collect::<String>().trim().to_string()
    } else {
        t.to_string()
    }
}

fn etiqueta_intencion(i: &Intencion) -> String {
    match i {
        Intencion::Original => "original".to_string(),
        Intencion::Traduccion => "traducción".to_string(),
        Intencion::Tono { etiqueta } => format!("tono: {etiqueta}"),
        Intencion::Resumen { palabras_objetivo: Some(n) } => format!("resumen ≈{n}p"),
        Intencion::Resumen { palabras_objetivo: None } => "resumen".to_string(),
        Intencion::Reescritura { .. } => "reescritura".to_string(),
        Intencion::Anotacion => "anotación".to_string(),
        Intencion::Custom { kind } => kind.clone(),
    }
}

// =============================================================================
//  EjecutorTraducirLlm — TipoTransformacion::Traducir
// =============================================================================

/// Traductor LLM. Por cada párrafo de la madre, una request al modelo
/// con el system "traduce a X" cacheado. Materializa el cuerpo hija con
/// `Intencion::Traduccion`, branch_id `{madre}-{lengua}`, lengua anotada.
///
/// No es genérico sobre el backend: usa `Arc<dyn ChatClient>` para encajar
/// con el factory `pluma_llm::build_client` sin vueltas de tipos. La
/// indirección vtable es ínfima comparada con un round-trip HTTP a un LLM.
pub struct EjecutorTraducirLlm {
    chat: Arc<dyn ChatClient>,
    lengua_destino: Lengua,
    system_prompt: String,
    max_tokens: u32,
    temperature: f32,
}

impl EjecutorTraducirLlm {
    /// Construye con system prompt razonable por defecto desde cualquier
    /// `ChatClient` concreto (lo envolvemos en `Arc<dyn>`).
    pub fn new<C: ChatClient + 'static>(
        chat: C,
        lengua_destino: impl Into<Lengua>,
    ) -> Self {
        Self::from_arc(Arc::new(chat), lengua_destino)
    }

    /// Construye desde un `Arc<dyn ChatClient>` ya armado — flujo natural
    /// con `pluma_llm::build_client(&cfg)`.
    pub fn from_arc(
        chat: Arc<dyn ChatClient>,
        lengua_destino: impl Into<Lengua>,
    ) -> Self {
        let lengua = lengua_destino.into();
        let system = format!(
            "Eres un traductor profesional al {lengua}. Traduce con \
             precisión el párrafo que el usuario te pase. Conserva nombres \
             propios, números y formato. NO agregues comentario, NO \
             prefijes la respuesta, NO uses comillas. Devuelve SOLO el \
             párrafo traducido."
        );
        Self {
            chat,
            lengua_destino: lengua,
            system_prompt: system,
            max_tokens: 1024,
            temperature: 0.2,
        }
    }

    /// Encadenable: sobrescribe el system prompt — útil para variantes
    /// regionales (`"Traduce al quechua del Cuzco preservando ortografía
    /// cusqueña"`).
    pub fn con_system_prompt(mut self, s: impl Into<String>) -> Self {
        self.system_prompt = s.into();
        self
    }

    /// Variante operativa que recibe el índice de átomos — flujo
    /// recomendado, porque el LLM necesita el TEXTO de la madre y el
    /// trait `Ejecutor::aplicar` solo da Uuids.
    pub async fn aplicar_con_atoms(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        // Validar que el tipo y la lengua coinciden.
        let lengua_esperada = match &t.tipo {
            TipoTransformacion::Traducir { lengua_destino } => lengua_destino,
            _ => return Err(ErrorEjecutor::TipoNoSoportado),
        };
        if lengua_esperada != &self.lengua_destino {
            return Err(ErrorEjecutor::Backend(format!(
                "lengua_destino ({}) no coincide con el ejecutor ({})",
                lengua_esperada, self.lengua_destino
            )));
        }
        let lengua = self.lengua_destino.clone();
        let cfg = ConfigAplicacion {
            intencion: Intencion::Traduccion,
            sufijo_branch: lengua.clone(),
            lengua: Some(lengua),
            system_prompt: self.system_prompt.clone(),
            user_prompt: Box::new(|texto: &str| texto.to_string()),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };
        ejecutar_lote(&*self.chat, &cfg, t, madre, atoms, ahora).await
    }
}

#[async_trait]
impl Ejecutor for EjecutorTraducirLlm {
    /// El trait no recibe el índice de átomos. Aquí señalamos al caller
    /// que use el método inherente `aplicar_con_atoms`.
    async fn aplicar(
        &self,
        _t: &Transformacion,
        _madre: &Cuerpo,
        _ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        Err(ErrorEjecutor::Backend(
            "EjecutorTraducirLlm requiere el índice de átomos — usar \
             `aplicar_con_atoms`. El trait Ejecutor::aplicar solo expone \
             Uuids y no puede resolver el texto que el LLM necesita."
                .to_string(),
        ))
    }
}

// =============================================================================
//  EjecutorTonoLlm — TipoTransformacion::Tono
// =============================================================================

/// Reescribe cada párrafo con otro tono (formal, casual, técnico, infantil…).
pub struct EjecutorTonoLlm {
    chat: Arc<dyn ChatClient>,
    etiqueta: String,
    system_prompt: String,
    max_tokens: u32,
    temperature: f32,
}

impl EjecutorTonoLlm {
    pub fn new<C: ChatClient + 'static>(chat: C, etiqueta: impl Into<String>) -> Self {
        Self::from_arc(Arc::new(chat), etiqueta)
    }

    pub fn from_arc(chat: Arc<dyn ChatClient>, etiqueta: impl Into<String>) -> Self {
        let etiqueta = etiqueta.into();
        let system = format!(
            "Reescribes cada párrafo que recibes con tono {etiqueta}, \
             conservando significado, nombres propios y números. NO \
             agregues comentario, NO uses comillas, NO prefijes. Devuelve \
             SOLO el párrafo reescrito."
        );
        Self {
            chat,
            etiqueta,
            system_prompt: system,
            max_tokens: 1024,
            temperature: 0.4,
        }
    }

    pub async fn aplicar_con_atoms(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        let etiqueta_esperada = match &t.tipo {
            TipoTransformacion::Tono { etiqueta } => etiqueta,
            _ => return Err(ErrorEjecutor::TipoNoSoportado),
        };
        if etiqueta_esperada != &self.etiqueta {
            return Err(ErrorEjecutor::Backend(format!(
                "etiqueta de tono ({}) no coincide con el ejecutor ({})",
                etiqueta_esperada, self.etiqueta
            )));
        }
        let cfg = ConfigAplicacion {
            intencion: Intencion::Tono { etiqueta: self.etiqueta.clone() },
            sufijo_branch: format!("tono-{}", self.etiqueta),
            lengua: None,
            system_prompt: self.system_prompt.clone(),
            user_prompt: Box::new(|texto: &str| texto.to_string()),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };
        ejecutar_lote(&*self.chat, &cfg, t, madre, atoms, ahora).await
    }
}

#[async_trait]
impl Ejecutor for EjecutorTonoLlm {
    async fn aplicar(
        &self,
        _t: &Transformacion,
        _madre: &Cuerpo,
        _ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        Err(ErrorEjecutor::Backend(
            "EjecutorTonoLlm requiere `aplicar_con_atoms`".to_string(),
        ))
    }
}

// =============================================================================
//  EjecutorResumirLlm — TipoTransformacion::Resumir
// =============================================================================

/// Resume cada párrafo a un objetivo de palabras (o el LLM decide si es None).
pub struct EjecutorResumirLlm {
    chat: Arc<dyn ChatClient>,
    palabras_objetivo: Option<u32>,
    system_prompt: String,
    max_tokens: u32,
    temperature: f32,
}

impl EjecutorResumirLlm {
    pub fn new<C: ChatClient + 'static>(chat: C, palabras_objetivo: Option<u32>) -> Self {
        Self::from_arc(Arc::new(chat), palabras_objetivo)
    }

    pub fn from_arc(chat: Arc<dyn ChatClient>, palabras_objetivo: Option<u32>) -> Self {
        let n = palabras_objetivo
            .map(|n| format!("aproximadamente {n} palabras"))
            .unwrap_or_else(|| "lo más conciso posible".to_string());
        let system = format!(
            "Resumes cada párrafo que recibes a {n}, conservando hechos \
             y nombres propios clave. NO agregues comentario, NO prefijes, \
             NO uses comillas. Devuelve SOLO el resumen."
        );
        Self {
            chat,
            palabras_objetivo,
            system_prompt: system,
            max_tokens: 512,
            temperature: 0.2,
        }
    }

    pub async fn aplicar_con_atoms(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        let pob_esperado = match &t.tipo {
            TipoTransformacion::Resumir { palabras_objetivo } => palabras_objetivo,
            _ => return Err(ErrorEjecutor::TipoNoSoportado),
        };
        if pob_esperado != &self.palabras_objetivo {
            return Err(ErrorEjecutor::Backend(format!(
                "palabras_objetivo ({pob_esperado:?}) no coincide con el ejecutor ({:?})",
                self.palabras_objetivo
            )));
        }
        let cfg = ConfigAplicacion {
            intencion: Intencion::Resumen { palabras_objetivo: self.palabras_objetivo },
            sufijo_branch: "resumen".to_string(),
            lengua: None,
            system_prompt: self.system_prompt.clone(),
            user_prompt: Box::new(|texto: &str| texto.to_string()),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };
        ejecutar_lote(&*self.chat, &cfg, t, madre, atoms, ahora).await
    }
}

#[async_trait]
impl Ejecutor for EjecutorResumirLlm {
    async fn aplicar(
        &self,
        _t: &Transformacion,
        _madre: &Cuerpo,
        _ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        Err(ErrorEjecutor::Backend(
            "EjecutorResumirLlm requiere `aplicar_con_atoms`".to_string(),
        ))
    }
}

// =============================================================================
//  EjecutorReescribirLlm — TipoTransformacion::Reescribir
// =============================================================================

/// Reescritura libre dictada por un prompt humano arbitrario (el `prompt`
/// del `TipoTransformacion::Reescribir`).
pub struct EjecutorReescribirLlm {
    chat: Arc<dyn ChatClient>,
    prompt: String,
    max_tokens: u32,
    temperature: f32,
}

impl EjecutorReescribirLlm {
    pub fn new<C: ChatClient + 'static>(chat: C, prompt: impl Into<String>) -> Self {
        Self::from_arc(Arc::new(chat), prompt)
    }

    pub fn from_arc(chat: Arc<dyn ChatClient>, prompt: impl Into<String>) -> Self {
        Self {
            chat,
            prompt: prompt.into(),
            max_tokens: 1024,
            temperature: 0.6,
        }
    }

    pub async fn aplicar_con_atoms(
        &self,
        t: &Transformacion,
        madre: &Cuerpo,
        atoms: &HashMap<Uuid, &NarrativeAtom>,
        ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        let prompt_esperado = match &t.tipo {
            TipoTransformacion::Reescribir { prompt } => prompt,
            _ => return Err(ErrorEjecutor::TipoNoSoportado),
        };
        if prompt_esperado != &self.prompt {
            return Err(ErrorEjecutor::Backend(
                "prompt de reescritura no coincide con el ejecutor".to_string(),
            ));
        }
        let system = format!(
            "Sigue la instrucción al pie de la letra para cada párrafo \
             que recibas. Instrucción: \"{}\". NO agregues comentario, \
             NO prefijes, NO uses comillas. Devuelve SOLO el párrafo \
             resultado.",
            self.prompt
        );
        let cfg = ConfigAplicacion {
            intencion: Intencion::Reescritura { prompt: self.prompt.clone() },
            sufijo_branch: "reescrito".to_string(),
            lengua: None,
            system_prompt: system,
            user_prompt: Box::new(|texto: &str| texto.to_string()),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };
        ejecutar_lote(&*self.chat, &cfg, t, madre, atoms, ahora).await
    }
}

#[async_trait]
impl Ejecutor for EjecutorReescribirLlm {
    async fn aplicar(
        &self,
        _t: &Transformacion,
        _madre: &Cuerpo,
        _ahora: u64,
    ) -> Result<ProductoTransformacion, ErrorEjecutor> {
        Err(ErrorEjecutor::Backend(
            "EjecutorReescribirLlm requiere `aplicar_con_atoms`".to_string(),
        ))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_mock::MockChatClient;

    fn madre_es(textos: &[&str]) -> (Cuerpo, Vec<NarrativeAtom>) {
        let mut c = Cuerpo::nuevo("es", "es (original)", Intencion::Original, 100);
        let atoms: Vec<NarrativeAtom> =
            textos.iter().map(|t| NarrativeAtom::new(*t, "es")).collect();
        for a in &atoms {
            c.agregar(a.id, 101);
        }
        (c, atoms)
    }

    fn indice(atoms: &[NarrativeAtom]) -> HashMap<Uuid, &NarrativeAtom> {
        atoms.iter().map(|a| (a.id, a)).collect()
    }

    #[tokio::test]
    async fn traducir_llm_emite_una_request_por_atom_y_arma_hija() {
        let (madre, atoms) = madre_es(&["uno", "dos", "tres"]);
        let idx = indice(&atoms);
        // Mock con respuestas indexadas: cada prompt user contiene la
        // palabra original; tabla la traduce a quechua.
        let chat = MockChatClient::default()
            .con_respuesta("uno", "huk")
            .con_respuesta("dos", "iskay")
            .con_respuesta("tres", "kimsa");
        let ej = EjecutorTraducirLlm::new(chat, "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "tester",
            200,
        );
        let prod = ej.aplicar_con_atoms(&t, &madre, &idx, 200).await.unwrap();

        assert_eq!(prod.atoms_nuevos.len(), 3);
        assert_eq!(prod.atoms_nuevos[0].content.as_str(), "huk");
        assert_eq!(prod.atoms_nuevos[1].content.as_str(), "iskay");
        assert_eq!(prod.atoms_nuevos[2].content.as_str(), "kimsa");
        assert_eq!(prod.carta.hebras.len(), 3);
        assert_eq!(prod.hija.metadatos.lengua.as_deref(), Some("qu"));
        assert_eq!(prod.hija.branch_id, "es-qu");
    }

    #[tokio::test]
    async fn traducir_lengua_mismatch_devuelve_backend_error() {
        let (madre, atoms) = madre_es(&["x"]);
        let idx = indice(&atoms);
        let ej = EjecutorTraducirLlm::new(MockChatClient::default(), "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "en".into() },
            "x",
            1,
        );
        match ej.aplicar_con_atoms(&t, &madre, &idx, 1).await {
            Err(ErrorEjecutor::Backend(msg)) => assert!(msg.contains("no coincide")),
            otro => panic!("esperaba Backend, fue {otro:?}"),
        }
    }

    #[tokio::test]
    async fn trait_aplicar_sin_atoms_falla_con_mensaje_orientativo() {
        let (madre, _atoms) = madre_es(&["x"]);
        let ej = EjecutorTraducirLlm::new(MockChatClient::default(), "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "x",
            1,
        );
        match Ejecutor::aplicar(&ej, &t, &madre, 1).await {
            Err(ErrorEjecutor::Backend(msg)) => {
                assert!(msg.contains("aplicar_con_atoms"));
            }
            otro => panic!("esperaba Backend con guía, fue {otro:?}"),
        }
    }

    #[tokio::test]
    async fn tono_emite_intencion_tono_con_etiqueta() {
        let (madre, atoms) = madre_es(&["hola"]);
        let idx = indice(&atoms);
        let chat = MockChatClient::default().con_respuesta("hola", "HOLA SEÑOR");
        let ej = EjecutorTonoLlm::new(chat, "formal");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Tono { etiqueta: "formal".into() },
            "x",
            1,
        );
        let prod = ej.aplicar_con_atoms(&t, &madre, &idx, 1).await.unwrap();
        assert_eq!(prod.atoms_nuevos[0].content.as_str(), "HOLA SEÑOR");
        assert!(matches!(
            prod.hija.metadatos.intencion,
            Intencion::Tono { ref etiqueta } if etiqueta == "formal"
        ));
        assert_eq!(prod.hija.branch_id, "es-tono-formal");
    }

    #[tokio::test]
    async fn resumir_palabras_objetivo_debe_coincidir() {
        let (madre, atoms) = madre_es(&["a"]);
        let idx = indice(&atoms);
        let ej = EjecutorResumirLlm::new(MockChatClient::default(), Some(50));
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Resumir { palabras_objetivo: Some(100) },
            "x",
            1,
        );
        match ej.aplicar_con_atoms(&t, &madre, &idx, 1).await {
            Err(ErrorEjecutor::Backend(msg)) => assert!(msg.contains("no coincide")),
            otro => panic!("esperaba Backend, fue {otro:?}"),
        }
    }

    #[tokio::test]
    async fn limpiar_respuesta_quita_comillas_envolventes() {
        assert_eq!(limpiar_respuesta(r#""hola""#), "hola");
        assert_eq!(limpiar_respuesta("«hola»"), "hola");
        assert_eq!(limpiar_respuesta("  hola  "), "hola");
        // Comillas internas no se tocan.
        assert_eq!(limpiar_respuesta(r#"dice "hola" y se va"#), r#"dice "hola" y se va"#);
    }

    #[tokio::test]
    async fn madre_vacia_es_madre_invalida() {
        let madre = Cuerpo::nuevo("es", "es", Intencion::Original, 100);
        let idx: HashMap<Uuid, &NarrativeAtom> = HashMap::new();
        let ej = EjecutorTraducirLlm::new(MockChatClient::default(), "qu");
        let t = Transformacion::nueva(
            madre.id,
            Uuid::new_v4(),
            TipoTransformacion::Traducir { lengua_destino: "qu".into() },
            "x",
            1,
        );
        assert!(matches!(
            ej.aplicar_con_atoms(&t, &madre, &idx, 1).await,
            Err(ErrorEjecutor::MadreInvalida(_))
        ));
    }
}
