//! `rag-motor` — la abstracción agnóstica de un **motor RAG**.
//!
//! El widget `rag` monta un panel «preguntá en lenguaje natural y te respondo
//! citando fuentes». Hoy hay dos corpus posibles: el correo (`paloma-rag`) y el
//! centro de eventos (`willay-rag`). Para que el widget sea un frontend
//! intercambiable —regla #2 del repo— ambos motores implementan el mismo trait
//! [`RagMotor`] y devuelven los mismos tipos de cita ([`RagSource`]/[`RagAnswer`]),
//! genéricos: un id opaco, un asunto, una procedencia, fecha y score. El widget
//! no sabe si detrás hay mails o capturas.

/// Una fuente citada en la respuesta: lo justo para mostrarla en una lista
/// (`[n] asunto — procedencia`) y, en el futuro, abrirla por `id`.
#[derive(Debug, Clone)]
pub struct RagSource {
    /// Id opaco de la fuente en su corpus (MessageId del correo, hex del evento…).
    pub id: String,
    /// Título/asunto de la fuente.
    pub subject: String,
    /// De dónde viene: remitente del mail, origen del evento…
    pub from: String,
    /// Fecha de la fuente (Unix seconds).
    pub date: i64,
    /// Contenedor lógico: carpeta del mail, clase del evento…
    pub mailbox: String,
    /// Parecido coseno `[0,1]` con la consulta.
    pub score: f32,
}

/// La respuesta del motor: el texto redactado + las fuentes que lo fundamentan,
/// en el mismo orden que los `[n]` del texto.
#[derive(Debug, Clone)]
pub struct RagAnswer {
    pub answer: String,
    pub sources: Vec<RagSource>,
}

/// Errores legibles que el panel muestra tal cual.
#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("todavía no hay nada indexado")]
    SinDatos,
    #[error("no encontré nada relevante para eso")]
    SinResultados,
    #[error("embeddings: {0}")]
    Embed(String),
    #[error("IA: {0}")]
    Llm(String),
}

/// Un motor RAG: sabe cuántas piezas tiene su corpus y responde una consulta
/// fuera del hilo de UI, entregando el resultado por callback (que el anfitrión
/// convierte en su `Msg`). `Send + Sync` para vivir tras el `Arc<Mutex<…>>` del
/// widget y poder lanzar trabajo a su propio runtime.
pub trait RagMotor: Send + Sync {
    /// Cuántas piezas hay en el corpus (mensajes, eventos…). Para el rótulo
    /// «N a mano. Preguntá…».
    fn corpus_len(&self) -> usize;

    /// Lanza la consulta y entrega el resultado por `done` (que corre fuera del
    /// hilo de UI). No bloquea.
    fn ask(&self, query: String, done: Box<dyn FnOnce(Result<RagAnswer, RagError>) + Send>);
}
