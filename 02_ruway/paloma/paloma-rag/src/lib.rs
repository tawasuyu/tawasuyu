//! paloma-rag — preguntale a tu correo.
//!
//! El otro lado de [`paloma_semantic`]: ésa **recupera** los mensajes parecidos
//! a una consulta; este crate los **lee** y, con un LLM, redacta una respuesta
//! en prosa **citando** de qué mails salió cada cosa (RAG: retrieval-augmented
//! generation). El usuario pregunta «¿en qué quedó lo de la factura del
//! proveedor?» y obtiene un párrafo con `[1] [2]` apuntando a los correos
//! exactos, en vez de tener que abrir y leer cinco hilos.
//!
//! ## Cómo se hospeda (sidebar)
//!
//! No es una app: es un **motor** que cualquier frontend monta como panel. pata
//! lo monta como sidebar (un diente del rail). Para no acoplarse a la app de
//! correo ni pelear por el índice, **lee la caché en disco de paloma de
//! sólo-lectura**: el corpus (`<cache>/<cuenta>/msgs-*.pc`, postcard) y el índice
//! semántico que `paloma-app` ya mantiene (`<cache>/semantic/<cuenta>.pc`). Si
//! paloma todavía no corrió (no hay índice), el motor no se engancha y el panel
//! avisa que hay que abrir paloma primero.
//!
//! ## El puente sync↔async, otra vez
//!
//! Igual que `paloma-semantic`: embeber la consulta y llamar al modelo son
//! **async** (daemon de embeddings + `pluma-llm`); el frontend es un bucle Elm
//! síncrono. [`RagEngine`] tiene su propio runtime y [`RagEngine::ask`] corre
//! todo fuera del hilo de UI, devolviendo el resultado por un callback que el
//! anfitrión convierte en su `Msg`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use directories::ProjectDirs;
use paloma_core::{Message, MessageId};
use paloma_semantic::{embed_query, Hit, SemanticIndex};
use pluma_llm::pluma_llm_core::{ChatClient, ChatRequest};
use rimay_verbo::Provider;
use tokio::runtime::Runtime;

/// Cuántos mensajes recupera el índice para fundamentar la respuesta. Acotado:
/// es el contexto que entra al modelo, no una lista de resultados para scrollear.
const TOP_K: usize = 6;
/// Coseno mínimo para considerar un mensaje. 0 = dejamos pasar el top-K y que el
/// prompt («si no está, decilo») filtre el ruido; subir esto recorta de más con
/// embeddings reales.
const MIN_SCORE: f32 = 0.0;
/// Caracteres del cuerpo de cada mensaje que entran al fragmento citado.
const BODY_CHARS: usize = 1200;
/// Tope del contexto total (suma de fragmentos) que se le manda al modelo.
const MAX_CONTEXT_CHARS: usize = 10_000;
/// Cota de tokens de la respuesta.
const MAX_TOKENS_ANSWER: u32 = 700;

/// Una fuente citada en la respuesta: el mensaje del que salió un fragmento, con
/// lo justo para mostrarlo en una lista («[1] Asunto — Remitente») y, en el
/// futuro, abrirlo (`id`).
#[derive(Debug, Clone)]
pub struct RagSource {
    pub id: MessageId,
    pub subject: String,
    /// `Nombre <email>` del remitente.
    pub from: String,
    /// Fecha del mensaje (Unix seconds).
    pub date: i64,
    pub mailbox: String,
    /// Parecido coseno `[0,1]` con la consulta.
    pub score: f32,
}

/// La respuesta del motor: el texto redactado por el modelo + las fuentes que lo
/// fundamentan, en el mismo orden que los `[n]` del texto.
#[derive(Debug, Clone)]
pub struct RagAnswer {
    pub answer: String,
    pub sources: Vec<RagSource>,
}

/// Errores legibles que el panel muestra tal cual.
#[derive(Debug, thiserror::Error)]
pub enum RagError {
    #[error("todavía no hay correo indexado — abrí paloma y dejá que indexe")]
    EmptyIndex,
    #[error("no encontré correos relevantes para eso")]
    NoHits,
    #[error("embeddings: {0}")]
    Embed(String),
    #[error("IA: {0}")]
    Llm(String),
}

/// El motor RAG: runtime async + proveedor de embeddings + cliente LLM + índice
/// semántico (sólo-lectura) + corpus indexado por `MessageId`. Se construye con
/// [`RagEngine::try_build`]; si falta cualquier pieza (caché, daemon, LLM),
/// devuelve `None` y el anfitrión deja el panel en «no disponible».
pub struct RagEngine {
    rt: Runtime,
    provider: Arc<dyn Provider>,
    client: Arc<dyn ChatClient>,
    index: Arc<SemanticIndex>,
    corpus: Arc<HashMap<MessageId, Message>>,
    corpus_len: usize,
}

impl RagEngine {
    /// Arma el motor leyendo la caché de paloma. `None` si no hay caché con
    /// correo, si no hay daemon de embeddings, si no hay índice semántico, o si
    /// no hay un backend LLM real (mock sólo con `PLUMA_LLM_BACKEND` explícito).
    pub fn try_build() -> Option<Self> {
        let cache = cache_root()?;
        let (account_dir, sem_path) = discover(&cache)?;
        let sem_path = sem_path?; // sin índice no podemos recuperar

        let corpus_vec = load_corpus(&account_dir);
        if corpus_vec.is_empty() {
            return None;
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok()?;

        // Proveedor de embeddings: el daemon de rimay-verbo, o mock sólo bajo
        // opt-in explícito (mismo criterio que `paloma-app::semantic`).
        let want_mock = std::env::var("PALOMA_SEMANTIC")
            .map(|v| v.eq_ignore_ascii_case("mock"))
            .unwrap_or(false);
        let provider: Arc<dyn Provider> = rt.block_on(async {
            match rimay_verbo::conectar().await {
                Ok(c) => Some(Arc::new(c) as Arc<dyn Provider>),
                Err(_) if want_mock => {
                    Some(Arc::new(rimay_verbo::MockProvider::new(384)) as Arc<dyn Provider>)
                }
                Err(_) => None,
            }
        })?;

        // Índice que paloma-app ya construyó. Si no decodifica o quedó vacío,
        // no hay nada que recuperar.
        let model = provider.model_id().clone();
        let index = SemanticIndex::load(&sem_path, model).ok()?;
        if index.is_empty() {
            return None;
        }

        // Cliente LLM (Eje 2): mismo criterio local-first que el resto de la
        // suite. Sin backend real (y sin opt-in), no hay generación → sin panel.
        let explicit = std::env::var("PLUMA_LLM_BACKEND").is_ok();
        let client = pluma_llm::from_env().ok()?;
        if client.model_id() == "pluma-llm-mock" && !explicit {
            return None;
        }

        let corpus: HashMap<MessageId, Message> =
            corpus_vec.into_iter().map(|m| (m.id.clone(), m)).collect();
        let corpus_len = corpus.len();
        Some(Self {
            rt,
            provider,
            client,
            index: Arc::new(index),
            corpus: Arc::new(corpus),
            corpus_len,
        })
    }

    /// Cuántos mensajes hay en el corpus leído de disco.
    pub fn corpus_len(&self) -> usize {
        self.corpus_len
    }

    /// Cuántos mensajes están embebidos en el índice (los que se pueden recuperar).
    pub fn index_len(&self) -> usize {
        self.index.len()
    }

    /// Lanza la consulta en el runtime async y entrega el resultado por `done`
    /// (que corre en un hilo del runtime, no en el de UI). El anfitrión convierte
    /// el `Result` en su `Msg` y lo despacha a su bucle. No bloquea.
    pub fn ask<F>(&self, query: String, done: F)
    where
        F: FnOnce(Result<RagAnswer, RagError>) + Send + 'static,
    {
        let provider = self.provider.clone();
        let client = self.client.clone();
        let index = self.index.clone();
        let corpus = self.corpus.clone();
        self.rt.spawn(async move {
            let result = answer(&*provider, &*client, &index, &corpus, &query).await;
            done(result);
        });
    }
}

/// El pipeline RAG de una consulta: embeber → recuperar → fundamentar → redactar.
async fn answer(
    provider: &dyn Provider,
    client: &dyn ChatClient,
    index: &SemanticIndex,
    corpus: &HashMap<MessageId, Message>,
    query: &str,
) -> Result<RagAnswer, RagError> {
    if index.is_empty() {
        return Err(RagError::EmptyIndex);
    }
    let qvec = embed_query(provider, query)
        .await
        .map_err(|e| RagError::Embed(e.to_string()))?;
    let hits = index
        .search(&qvec, TOP_K, MIN_SCORE)
        .map_err(|e| RagError::Embed(e.to_string()))?;
    if hits.is_empty() {
        return Err(RagError::NoHits);
    }
    let (system, user, sources) = assemble(query, &hits, corpus);
    if sources.is_empty() {
        return Err(RagError::NoHits);
    }
    let req = ChatRequest::una_vuelta(user, MAX_TOKENS_ANSWER)
        .con_sistema(system)
        .con_temperatura(0.2);
    let resp = client
        .complete(&req)
        .await
        .map_err(|e| RagError::Llm(e.to_string()))?;
    Ok(RagAnswer {
        answer: resp.content.trim().to_string(),
        sources,
    })
}

/// Arma el prompt RAG a partir de los hits y el corpus: el `system` que fija las
/// reglas (citar `[n]`, no inventar), el `user` con la pregunta + los fragmentos
/// numerados, y las `RagSource` en el mismo orden que esos números. **Puro**
/// (sin red ni disco), para poder testearlo. Los hits sin mensaje en el corpus
/// se saltan; el contexto se acota a [`MAX_CONTEXT_CHARS`].
pub fn assemble(
    query: &str,
    hits: &[Hit],
    corpus: &HashMap<MessageId, Message>,
) -> (String, String, Vec<RagSource>) {
    let system = "Sos un asistente que responde preguntas sobre el correo del usuario. \
        Respondé SÓLO con la información de los fragmentos numerados que siguen; citá cada \
        afirmación con el número de su fragmento entre corchetes, p. ej. [1] o [2]. Si la \
        respuesta no está en los fragmentos, decilo con claridad en vez de inventar. Respondé \
        en el idioma de la pregunta, breve y al grano."
        .to_string();

    let mut sources: Vec<RagSource> = Vec::new();
    let mut frags = String::new();
    let mut used = 0usize;
    for hit in hits {
        let Some(msg) = corpus.get(&hit.id) else {
            continue;
        };
        let n = sources.len() + 1;
        let from = format!("{} <{}>", msg.from.display_name(), msg.from.email);
        let body: String = msg.body_text.chars().take(BODY_CHARS).collect();
        let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let frag = format!(
            "[{n}] De: {from} · Asunto: {asunto} · Fecha: {fecha}\n{body}\n\n",
            asunto = msg.subject,
            fecha = ymd(msg.date),
        );
        // Cortamos al pasarnos, pero siempre dejamos entrar al menos uno.
        if used + frag.len() > MAX_CONTEXT_CHARS && !sources.is_empty() {
            break;
        }
        used += frag.len();
        frags.push_str(&frag);
        sources.push(RagSource {
            id: msg.id.clone(),
            subject: msg.subject.clone(),
            from,
            date: msg.date,
            mailbox: msg.mailbox.clone(),
            score: hit.score,
        });
    }

    let user = format!(
        "Pregunta: {query}\n\nFragmentos del correo:\n\n{frags}Respondé la pregunta citando \
         entre corchetes los números de los fragmentos que uses."
    );
    (system, user, sources)
}

/// Raíz de la caché de paloma (`~/.cache/paloma` en Linux), la misma que usa
/// `paloma-app`. `None` si la plataforma no expone `ProjectDirs`.
fn cache_root() -> Option<PathBuf> {
    ProjectDirs::from("org", "tawasuyu", "paloma").map(|d| d.cache_dir().to_path_buf())
}

/// Descubre qué cuenta usar dentro de la caché: el directorio (que no sea
/// `semantic`) con más snapshots de mensajes, y la ruta de su índice semántico
/// si existe. Evita reconstruir el saneo del `account_id` (que difiere entre
/// `paloma-store` y `paloma-app`): localiza los archivos por su forma en disco.
fn discover(cache: &Path) -> Option<(PathBuf, Option<PathBuf>)> {
    let mut best: Option<(PathBuf, usize)> = None;
    for entry in std::fs::read_dir(cache).ok()?.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == "semantic" || name.is_empty() {
            continue;
        }
        let n = count_msg_snapshots(&dir);
        if n == 0 {
            continue;
        }
        if best.as_ref().map(|(_, b)| n > *b).unwrap_or(true) {
            best = Some((dir, n));
        }
    }
    let (account_dir, _) = best?;

    // Índice semántico: primero el del nombre de la cuenta, si no el primero que
    // haya en `semantic/`.
    let sem_dir = cache.join("semantic");
    let account_name = account_dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let preferred = sem_dir.join(format!("{account_name}.pc"));
    let sem = if preferred.is_file() {
        Some(preferred)
    } else {
        std::fs::read_dir(&sem_dir).ok().and_then(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .find(|p| p.extension().and_then(|s| s.to_str()) == Some("pc"))
        })
    };
    Some((account_dir, sem))
}

/// Cuántos archivos `msgs-*.pc` hay en un directorio de cuenta.
fn count_msg_snapshots(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.starts_with("msgs-") && n.ends_with(".pc"))
                .unwrap_or(false)
        })
        .count()
}

/// Lee todos los `msgs-*.pc` de un directorio de cuenta y junta sus mensajes.
/// Best-effort: salta los blobs ilegibles (versión vieja/corruptos).
fn load_corpus(account_dir: &Path) -> Vec<Message> {
    let mut all = Vec::new();
    let Ok(rd) = std::fs::read_dir(account_dir) else {
        return all;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let is_snapshot = p
            .file_name()
            .and_then(|s| s.to_str())
            .map(|n| n.starts_with("msgs-") && n.ends_with(".pc"))
            .unwrap_or(false);
        if !is_snapshot {
            continue;
        }
        if let Ok(bytes) = std::fs::read(&p) {
            if let Ok(msgs) = postcard::from_bytes::<Vec<Message>>(&bytes) {
                all.extend(msgs);
            }
        }
    }
    all
}

/// Fecha `YYYY-MM-DD` (UTC) de un timestamp Unix en segundos, sin dependencias de
/// calendario (algoritmo civil de Howard Hinnant). Para rotular fuentes.
fn ymd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    // days desde 1970-01-01; pasamos a la era de marzo.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use paloma_core::{Address, Flags};

    fn msg(id: &str, subject: &str, body: &str) -> Message {
        Message {
            id: MessageId(id.to_string()),
            from: Address::named("Ana Pérez", "ana@x.com"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.to_string(),
            date: 1_700_000_000, // 2023-11-14
            in_reply_to: None,
            references: vec![],
            body_text: body.to_string(),
            body_html: None,
            flags: Flags::default(),
            signature: Default::default(),
            mailbox: "INBOX".to_string(),
            cuerpos: Vec::new(),
            signer: None,
        }
    }

    fn corpus(msgs: Vec<Message>) -> HashMap<MessageId, Message> {
        msgs.into_iter().map(|m| (m.id.clone(), m)).collect()
    }

    #[test]
    fn ymd_conocido() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn assemble_numera_y_arma_fuentes_en_orden() {
        let c = corpus(vec![
            msg("<a@x>", "factura del proveedor", "el pago vence el viernes"),
            msg("<b@x>", "reunión", "movemos la daily"),
        ]);
        let hits = vec![
            Hit { id: MessageId("<a@x>".into()), score: 0.9 },
            Hit { id: MessageId("<b@x>".into()), score: 0.4 },
        ];
        let (system, user, sources) = assemble("¿qué pasa con la factura?", &hits, &c);

        // El system fija la regla de citar.
        assert!(system.contains("[1]"));
        // Las fuentes salen en el orden de los hits y se numeran 1..n.
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].id.0, "<a@x>");
        assert_eq!(sources[0].subject, "factura del proveedor");
        assert!(sources[0].from.contains("ana@x.com"));
        // El user lleva la pregunta y ambos fragmentos numerados.
        assert!(user.contains("¿qué pasa con la factura?"));
        assert!(user.contains("[1] De:"));
        assert!(user.contains("[2] De:"));
        assert!(user.contains("el pago vence el viernes"));
    }

    /// Certifica el pipeline completo de [`answer`] con tipos reales pero sin red
    /// ni disco: embeddings deterministas (`MockProvider`), índice en memoria y un
    /// `ChatClient` mock que **hace eco** del prompt. Así se prueba retrieval →
    /// fundamentación → generación de punta a punta (lo que el sidebar dispara en
    /// cada consulta), sin levantar el daemon ni un backend LLM.
    #[tokio::test]
    async fn answer_recupera_fundamenta_y_redacta() {
        use pluma_llm::{build_client, BackendKind, LlmConfig};
        use rimay_verbo::MockProvider;

        let p = MockProvider::new(64);
        let factura = msg("<a@x>", "factura del proveedor", "el pago vence el viernes 12");
        let asado = msg("<b@x>", "asado del domingo", "traé carbón y provoleta");
        let msgs = vec![factura.clone(), asado];

        let mut idx = SemanticIndex::new(p.model_id().clone());
        idx.ingest(paloma_semantic::embed_messages(&p, &msgs).await.unwrap());
        let corpus: HashMap<MessageId, Message> =
            msgs.into_iter().map(|m| (m.id.clone(), m)).collect();

        let client =
            build_client(&LlmConfig { kind: BackendKind::Mock, ..Default::default() }).unwrap();

        // Consultamos con el texto exacto del mail de la factura: el mock es
        // determinista, así que ese mail rankea primero (coseno 1.0).
        let query = paloma_semantic::embeddable_text(&factura);
        let ans = answer(&p, &*client, &idx, &corpus, &query).await.unwrap();

        // Recuperó al menos una fuente y la primera es la factura.
        assert!(!ans.sources.is_empty());
        assert_eq!(ans.sources[0].id.0, "<a@x>");
        // El cliente mock hace eco del prompt → la respuesta contiene el cuerpo
        // del fragmento citado: prueba que el contexto llegó al modelo.
        assert!(
            ans.answer.contains("el pago vence el viernes 12"),
            "answer={:?}",
            ans.answer
        );
    }

    #[test]
    fn assemble_salta_hits_sin_mensaje() {
        let c = corpus(vec![msg("<a@x>", "hola", "mundo")]);
        let hits = vec![
            Hit { id: MessageId("<fantasma@x>".into()), score: 0.9 },
            Hit { id: MessageId("<a@x>".into()), score: 0.8 },
        ];
        let (_s, user, sources) = assemble("hola", &hits, &c);
        // Sólo el que existe entra, y se renumera como [1].
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id.0, "<a@x>");
        assert!(user.contains("[1] De:"));
        assert!(!user.contains("[2]"));
    }
}
