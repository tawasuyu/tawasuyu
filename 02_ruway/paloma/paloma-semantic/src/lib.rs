//! paloma-semantic — búsqueda **por significado** del correo.
//!
//! La búsqueda exacta de [`paloma_core::search`] cubre "¿dónde estaba ese
//! correo?" cuando recordás una palabra literal. Esta capa cubre la otra mitad:
//! "el mail donde **me hablaban de la factura atrasada**" aunque el mensaje
//! dijera "pago pendiente del mes". Embebe cada mensaje en un vector con
//! `rimay-verbo` y rankea por **similitud coseno** contra el vector de la
//! consulta.
//!
//! ## Por qué este crate existe (el puente sync↔async)
//!
//! `paloma-llimphi` es un bucle Elm **síncrono** (`update(Msg) -> ()`). Los
//! `Provider` de embeddings son **async** y, en producción, viven tras un socket
//! Unix (`rimay-verbo-daemon`). Este crate parte el problema en dos mitades:
//!
//! - **El cómputo de embeddings es async** y se hace fuera del hilo de UI:
//!   [`embed_messages`] y [`embed_query`] son `async` — el anfitrión las corre
//!   en su runtime (un worker) y despacha el resultado de vuelta como un `Msg`.
//! - **El ranking es síncrono y puro**: [`SemanticIndex::search`] sólo hace
//!   aritmética de vectores; el bucle de UI la llama sin bloquear nada.
//!
//! El índice se **persiste** ([`SemanticIndex::save`]/[`load`], postcard) junto
//! a la caché de mensajes, así no se re-embebe todo en cada arranque: sólo los
//! mensajes nuevos ([`SemanticIndex::missing`]).
//!
//! Es agnóstico al proveedor (mock, fastembed, Cohere…) y a la UI: sólo sabe de
//! `Message` y de vectores.

use std::collections::HashMap;
use std::path::Path;

use paloma_core::{Message, MessageId};
use rimay_verbo_core::{EmbeddingVector, ModelId, Provider};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Cuántos caracteres del cuerpo entran al texto a embeber. Acota el costo y
/// evita que un boletín gigante domine su propio vector; el asunto y el
/// remitente, que pesan más en relevancia, van completos.
const BODY_CHARS: usize = 1500;

/// Errores de la capa semántica.
#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("embeddings: {0}")]
    Embed(#[from] rimay_verbo_core::EmbedError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("códec: {0}")]
    Codec(String),
    /// El índice fue construido con otro modelo que la consulta: los espacios
    /// vectoriales son incomparables.
    #[error("modelo del índice ({index}) ≠ modelo de la consulta ({query})")]
    ModelMismatch { index: String, query: String },
}

impl From<postcard::Error> for SemanticError {
    fn from(e: postcard::Error) -> Self {
        SemanticError::Codec(e.to_string())
    }
}

/// El texto representativo de un mensaje para embeber: asunto + remitente +
/// inicio del cuerpo. El asunto se repite implícitamente al ir primero (los
/// modelos pesan el comienzo); el cuerpo se acota a [`BODY_CHARS`].
pub fn embeddable_text(msg: &Message) -> String {
    let from = msg.from.display_name();
    let body: String = msg.body_text.chars().take(BODY_CHARS).collect();
    // Colapsa whitespace del cuerpo para no gastar tokens en saltos de línea.
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{}\n{} <{}>\n{}", msg.subject, from, msg.from.email, body)
}

/// Embebe un lote de mensajes con `provider`. El anfitrión la corre en su
/// runtime async (fuera del hilo de UI) y luego ingiere el resultado al índice.
pub async fn embed_messages(
    provider: &dyn Provider,
    messages: &[Message],
) -> Result<Vec<(MessageId, EmbeddingVector)>, SemanticError> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<String> = messages.iter().map(embeddable_text).collect();
    let vectors = provider.embed_batch(&texts).await?;
    Ok(messages
        .iter()
        .map(|m| m.id.clone())
        .zip(vectors)
        .collect())
}

/// Embebe el texto de una consulta del usuario. Idéntico camino async que los
/// mensajes, así viven en el mismo espacio vectorial.
pub async fn embed_query(
    provider: &dyn Provider,
    query: &str,
) -> Result<EmbeddingVector, SemanticError> {
    Ok(provider.embed(query).await?)
}

/// Un resultado de búsqueda semántica: el mensaje y su parecido `[0,1]` con la
/// consulta (1 = idéntico en significado según el modelo).
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: MessageId,
    pub score: f32,
}

/// Índice de embeddings de los mensajes de una cuenta. Mapea cada `MessageId` a
/// su vector; todos comparten el mismo `model`. Serializable a disco para no
/// re-embeber en cada arranque.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIndex {
    model: ModelId,
    vectors: HashMap<MessageId, Vec<f32>>,
}

impl SemanticIndex {
    /// Índice vacío para un modelo dado. Todos los vectores que ingiera deben
    /// ser de este modelo.
    pub fn new(model: ModelId) -> Self {
        Self {
            model,
            vectors: HashMap::new(),
        }
    }

    /// El modelo de este índice.
    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// Cuántos mensajes tiene embebidos.
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// ¿Ya está embebido este mensaje?
    pub fn contains(&self, id: &MessageId) -> bool {
        self.vectors.contains_key(id)
    }

    /// De una lista de mensajes, los que **todavía no** están en el índice —
    /// lo que hay que embeber en el próximo refresco (embedding incremental).
    pub fn missing<'a>(&self, messages: &'a [Message]) -> Vec<&'a Message> {
        messages
            .iter()
            .filter(|m| !self.vectors.contains_key(&m.id))
            .collect()
    }

    /// Ingiere los vectores producidos por [`embed_messages`]. Ignora (con un
    /// recuento) los de modelo distinto al del índice — no se pueden comparar.
    /// Devuelve cuántos se incorporaron.
    pub fn ingest(&mut self, entries: impl IntoIterator<Item = (MessageId, EmbeddingVector)>) -> usize {
        let mut n = 0;
        for (id, vec) in entries {
            if vec.model == self.model {
                self.vectors.insert(id, vec.values);
                n += 1;
            }
        }
        n
    }

    /// Quita del índice los mensajes que ya no están en `keep` (purga lo
    /// borrado/expirado para que el índice no crezca sin fin).
    pub fn retain(&mut self, keep: &[MessageId]) {
        let keep: std::collections::HashSet<&MessageId> = keep.iter().collect();
        self.vectors.retain(|id, _| keep.contains(id));
    }

    /// Rankea los mensajes del índice por similitud con `query`. **Síncrono y
    /// puro** — lo llama el bucle de UI sin bloquear. Devuelve hasta `top_k`
    /// hits con score ≥ `min_score`, de mayor a menor parecido.
    pub fn search(
        &self,
        query: &EmbeddingVector,
        top_k: usize,
        min_score: f32,
    ) -> Result<Vec<Hit>, SemanticError> {
        if query.model != self.model {
            return Err(SemanticError::ModelMismatch {
                index: self.model.to_string(),
                query: query.model.to_string(),
            });
        }
        let qnorm = norm(&query.values);
        if qnorm == 0.0 {
            return Ok(Vec::new());
        }
        let mut hits: Vec<Hit> = self
            .vectors
            .iter()
            .filter_map(|(id, v)| {
                let s = cosine(&query.values, v, qnorm);
                (s >= min_score).then(|| Hit {
                    id: id.clone(),
                    score: s,
                })
            })
            .collect();
        // Mayor score primero; desempate estable por id para resultados
        // reproducibles entre corridas.
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        hits.truncate(top_k);
        Ok(hits)
    }

    /// Serializa el índice a `path` (postcard, escritura atómica vía `.tmp`).
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SemanticError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = postcard::to_allocvec(self)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Carga un índice de `path`. Si el archivo no existe, devuelve un índice
    /// vacío con el `model` dado (primer arranque).
    pub fn load(path: impl AsRef<Path>, model: ModelId) -> Result<Self, SemanticError> {
        match std::fs::read(path.as_ref()) {
            Ok(bytes) => {
                let idx: SemanticIndex = postcard::from_bytes(&bytes)?;
                Ok(idx)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new(model)),
            Err(e) => Err(e.into()),
        }
    }
}

/// Norma euclidiana.
fn norm(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Coseno entre la consulta (cuya norma ya conocemos) y un vector del índice.
/// Asume misma dimensión (garantizada por el modelo compartido).
fn cosine(q: &[f32], v: &[f32], qnorm: f32) -> f32 {
    let vnorm = norm(v);
    if vnorm == 0.0 {
        return 0.0;
    }
    let dot: f32 = q.iter().zip(v).map(|(a, b)| a * b).sum();
    (dot / (qnorm * vnorm)).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_core::{Address, Flags};
    use rimay_verbo_mock::MockProvider;

    fn msg(id: &str, subject: &str, body: &str) -> Message {
        Message {
            id: MessageId(id.to_string()),
            from: Address::named("Quien Sea", "quien@example.com"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.to_string(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: body.to_string(),
            body_html: None,
            flags: Flags::default(),
            signature: Default::default(),
            mailbox: "INBOX".to_string(),
        }
    }

    #[tokio::test]
    async fn la_consulta_identica_al_mensaje_rankea_primero() {
        let p = MockProvider::new(64);
        let msgs = vec![
            msg("a", "factura del mes", "el pago pendiente vence el viernes"),
            msg("b", "asado del domingo", "traé carbón y provoleta"),
            msg("c", "reunión de equipo", "movemos la daily a las 10"),
        ];
        let mut idx = SemanticIndex::new(p.model_id().clone());
        let n = idx.ingest(embed_messages(&p, &msgs).await.unwrap());
        assert_eq!(n, 3);
        assert_eq!(idx.len(), 3);

        // El mock es determinista: embeber exactamente el texto del mensaje "a"
        // da su mismo vector → coseno 1.0 → primero.
        let q = embed_query(&p, &embeddable_text(&msgs[0])).await.unwrap();
        let hits = idx.search(&q, 3, 0.0).unwrap();
        assert_eq!(hits[0].id, MessageId("a".to_string()));
        assert!((hits[0].score - 1.0).abs() < 1e-4, "score={}", hits[0].score);
    }

    #[tokio::test]
    async fn top_k_y_min_score_acotan() {
        let p = MockProvider::new(32);
        let msgs: Vec<Message> = (0..10)
            .map(|i| msg(&format!("m{i}"), &format!("asunto {i}"), &format!("cuerpo numero {i}")))
            .collect();
        let mut idx = SemanticIndex::new(p.model_id().clone());
        idx.ingest(embed_messages(&p, &msgs).await.unwrap());

        let q = embed_query(&p, "asunto 3 cuerpo numero 3").await.unwrap();
        let hits = idx.search(&q, 3, 0.0).unwrap();
        assert!(hits.len() <= 3);
        // min_score alto deja pasar muy pocos (o ninguno).
        let estrictos = idx.search(&q, 10, 0.999).unwrap();
        assert!(estrictos.len() <= hits.len());
    }

    #[tokio::test]
    async fn embedding_incremental_solo_los_nuevos() {
        let p = MockProvider::new(16);
        let viejos = vec![msg("a", "uno", "cuerpo uno"), msg("b", "dos", "cuerpo dos")];
        let mut idx = SemanticIndex::new(p.model_id().clone());
        idx.ingest(embed_messages(&p, &viejos).await.unwrap());

        let todos = vec![
            msg("a", "uno", "cuerpo uno"),
            msg("b", "dos", "cuerpo dos"),
            msg("c", "tres", "cuerpo tres"),
        ];
        let faltan = idx.missing(&todos);
        assert_eq!(faltan.len(), 1);
        assert_eq!(faltan[0].id, MessageId("c".to_string()));
    }

    #[tokio::test]
    async fn modelo_distinto_no_se_ingiere_ni_se_busca() {
        let p16 = MockProvider::new(16);
        let p32 = MockProvider::new(32);
        let msgs = vec![msg("a", "hola", "mundo")];

        let mut idx = SemanticIndex::new(p16.model_id().clone());
        // Vectores de p32 a un índice p16: se descartan.
        let n = idx.ingest(embed_messages(&p32, &msgs).await.unwrap());
        assert_eq!(n, 0);
        assert!(idx.is_empty());

        // Consulta de otro modelo: error explícito, no resultados basura.
        idx.ingest(embed_messages(&p16, &msgs).await.unwrap());
        let q = embed_query(&p32, "hola mundo").await.unwrap();
        assert!(matches!(idx.search(&q, 5, 0.0), Err(SemanticError::ModelMismatch { .. })));
    }

    #[tokio::test]
    async fn roundtrip_a_disco() {
        let p = MockProvider::new(16);
        let msgs = vec![msg("a", "hola", "mundo"), msg("b", "chau", "luna")];
        let mut idx = SemanticIndex::new(p.model_id().clone());
        idx.ingest(embed_messages(&p, &msgs).await.unwrap());

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("indice.pc");
        idx.save(&path).unwrap();

        let cargado = SemanticIndex::load(&path, p.model_id().clone()).unwrap();
        assert_eq!(cargado.len(), 2);
        assert!(cargado.contains(&MessageId("a".to_string())));

        // Cargar de una ruta inexistente → índice vacío del modelo dado.
        let vacio = SemanticIndex::load(dir.path().join("nope.pc"), p.model_id().clone()).unwrap();
        assert!(vacio.is_empty());
    }
}
