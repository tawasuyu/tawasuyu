//! `willay-rag` — preguntale a tus eventos.
//!
//! El registro **semántico** de la búsqueda del centro de eventos (el literal
//! vive en `willay-store::buscar`): el usuario pregunta «¿cuándo copié la API
//! key?» o «la captura del error de ayer» y obtiene un párrafo que **cita** los
//! eventos exactos (`[1] [2]`), en vez de scrollear el feed.
//!
//! Es un [`rag_motor::RagMotor`] sobre los eventos: los lee del daemon willay
//! (federación: no reabre el sled), los embebe con `rimay-verbo`, recupera por
//! coseno y redacta con `pluma-llm`. Espeja a `paloma-rag` (correo) — mismo
//! patrón, otro corpus. El puente sync↔async es igual: runtime propio + `ask`
//! por callback fuera del hilo de UI.

use std::sync::Arc;

use pluma_llm::pluma_llm_core::{ChatClient, ChatRequest};
use rag_motor::{RagAnswer, RagError, RagMotor, RagSource};
use rimay_verbo::{EmbeddingVector, Provider};
use tokio::runtime::Runtime;
use willay_core::proto::{Respuesta, Solicitud};
use willay_core::Evento;
use willay_emit::Emisor;

/// Cuántos eventos trae del índice para fundamentar (contexto del modelo).
const TOP_K: usize = 6;
/// Coseno mínimo para considerar un evento (0 = deja pasar el top-K y que el
/// prompt filtre).
const MIN_SCORE: f32 = 0.0;
/// Caracteres del cuerpo de cada evento que entran al fragmento citado.
const BODY_CHARS: usize = 1200;
/// Tope del contexto total (suma de fragmentos) que va al modelo.
const MAX_CONTEXT_CHARS: usize = 10_000;
/// Cota de tokens de la respuesta.
const MAX_TOKENS_ANSWER: u32 = 700;
/// Cuántos eventos recientes trae del daemon para indexar.
const MAX_EVENTOS: u32 = 500;

/// El texto que se embebe de un evento: título + cuerpo (acotado). El cuerpo ya
/// trae lo buscable (body de la notif, texto del clip, ruta de la captura).
fn embeddable(e: &Evento) -> String {
    let mut s = e.titulo.clone();
    if !e.cuerpo.is_empty() {
        s.push('\n');
        s.push_str(&e.cuerpo);
    }
    s.chars().take(2000).collect()
}

/// El motor RAG del centro de eventos: runtime async + proveedor de embeddings +
/// cliente LLM + el corpus de eventos ya embebidos. Se construye con
/// [`Engine::try_build`]; si falta una pieza (daemon caído, sin eventos, sin
/// embeddings, sin LLM real), devuelve `None` y el panel queda «no disponible».
pub struct Engine {
    rt: Runtime,
    provider: Arc<dyn Provider>,
    client: Arc<dyn ChatClient>,
    corpus: Arc<Vec<(Evento, EmbeddingVector)>>,
    corpus_len: usize,
}

impl Engine {
    /// Arma el motor. `None` si el daemon willay no responde o no hay eventos, si
    /// no hay daemon de embeddings, o si no hay un backend LLM real (mock sólo con
    /// opt-in explícito).
    pub fn try_build() -> Option<Self> {
        let eventos = fetch_eventos()?;
        if eventos.is_empty() {
            return None;
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok()?;

        // Config GLOBAL del SO (wawa.ai): daemon de embeddings + backend LLM.
        let ai = wawa_config::WawaConfig::load().ai;
        let want_mock = std::env::var("WILLAY_SEMANTIC")
            .map(|v| v.eq_ignore_ascii_case("mock"))
            .unwrap_or(false);
        let socket = ai.semantic.socket.trim().to_string();
        let mock_dim = ai.semantic.effective_dim();
        let provider: Arc<dyn Provider> = rt.block_on(async {
            let connected = if socket.is_empty() {
                rimay_verbo::conectar().await
            } else {
                rimay_verbo::conectar_en(std::path::Path::new(&socket)).await
            };
            match connected {
                Ok(c) => Some(Arc::new(c) as Arc<dyn Provider>),
                Err(_) if want_mock => {
                    Some(Arc::new(rimay_verbo::MockProvider::new(mock_dim)) as Arc<dyn Provider>)
                }
                Err(_) => None,
            }
        })?;

        // Embebemos los eventos al armar (willay no tiene índice precomputado en
        // disco como paloma). Acotado a MAX_EVENTOS.
        let corpus = rt.block_on(embed_eventos(&*provider, eventos));
        if corpus.is_empty() {
            return None;
        }

        let (client, _real) = build_rag_llm(&ai.llm)?;
        let corpus_len = corpus.len();
        Some(Self { rt, provider, client, corpus: Arc::new(corpus), corpus_len })
    }
}

impl RagMotor for Engine {
    fn corpus_len(&self) -> usize {
        self.corpus_len
    }

    fn ask(&self, query: String, done: Box<dyn FnOnce(Result<RagAnswer, RagError>) + Send>) {
        let provider = self.provider.clone();
        let client = self.client.clone();
        let corpus = self.corpus.clone();
        self.rt.spawn(async move {
            let r = answer(&*provider, &*client, &corpus, &query).await;
            done(r);
        });
    }
}

/// Trae los eventos recientes del daemon willay por el socket. `None` si el
/// daemon no está arriba o no respondió eventos.
fn fetch_eventos() -> Option<Vec<Evento>> {
    let mut em = Emisor::conectar().ok()?;
    match em.pedir(&Solicitud::Recientes(MAX_EVENTOS)).ok()? {
        Respuesta::Eventos(v) => Some(v),
        _ => None,
    }
}

/// Embebe el texto de cada evento (batch). Devuelve los pares evento↔vector; si
/// el batch falla entero, vacío (el motor no se arma).
async fn embed_eventos(provider: &dyn Provider, eventos: Vec<Evento>) -> Vec<(Evento, EmbeddingVector)> {
    let textos: Vec<String> = eventos.iter().map(embeddable).collect();
    match provider.embed_batch(&textos).await {
        Ok(vecs) if vecs.len() == eventos.len() => eventos.into_iter().zip(vecs).collect(),
        _ => Vec::new(),
    }
}

/// El pipeline RAG de una consulta: embeber → recuperar → fundamentar → redactar.
async fn answer(
    provider: &dyn Provider,
    client: &dyn ChatClient,
    corpus: &[(Evento, EmbeddingVector)],
    query: &str,
) -> Result<RagAnswer, RagError> {
    if corpus.is_empty() {
        return Err(RagError::SinDatos);
    }
    let qvec = provider.embed(query).await.map_err(|e| RagError::Embed(e.to_string()))?;
    let ranked = rankear(&qvec, corpus, TOP_K, MIN_SCORE);
    if ranked.is_empty() {
        return Err(RagError::SinResultados);
    }
    let (system, user, sources) = assemble(query, &ranked, corpus);
    if sources.is_empty() {
        return Err(RagError::SinResultados);
    }
    let req = ChatRequest::una_vuelta(user, MAX_TOKENS_ANSWER)
        .con_sistema(system)
        .con_temperatura(0.2);
    let resp = client.complete(&req).await.map_err(|e| RagError::Llm(e.to_string()))?;
    Ok(RagAnswer { answer: resp.content.trim().to_string(), sources })
}

/// Recupera por coseno: índice + score de los eventos más parecidos a `qvec`,
/// descendente, hasta `top_k` (desempate por índice, estable). **Puro**.
fn rankear(
    qvec: &EmbeddingVector,
    corpus: &[(Evento, EmbeddingVector)],
    top_k: usize,
    min: f32,
) -> Vec<(usize, f32)> {
    let mut hits: Vec<(usize, f32)> = corpus
        .iter()
        .enumerate()
        .filter_map(|(i, (_, v))| {
            let s = qvec.cosine(v).unwrap_or(0.0);
            (s >= min).then_some((i, s))
        })
        .collect();
    hits.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0))
    });
    hits.truncate(top_k);
    hits
}

/// Arma el prompt RAG a partir de los eventos rankeados: el `system` con las
/// reglas (citar `[n]`, no inventar), el `user` con la pregunta + fragmentos
/// numerados, y las `RagSource` en el mismo orden. **Puro** (sin red ni disco),
/// para testearlo. El contexto se acota a [`MAX_CONTEXT_CHARS`].
fn assemble(
    query: &str,
    ranked: &[(usize, f32)],
    corpus: &[(Evento, EmbeddingVector)],
) -> (String, String, Vec<RagSource>) {
    let system = "Sos un asistente que responde preguntas sobre los eventos del escritorio del \
        usuario (notificaciones, capturas de pantalla, portapapeles). Respondé SÓLO con la \
        información de los fragmentos numerados que siguen; citá cada afirmación con el número de \
        su fragmento entre corchetes, p. ej. [1] o [2]. Si la respuesta no está en los fragmentos, \
        decilo con claridad en vez de inventar. Respondé en el idioma de la pregunta, breve y al grano."
        .to_string();

    let mut sources: Vec<RagSource> = Vec::new();
    let mut frags = String::new();
    let mut used = 0usize;
    for (i, score) in ranked {
        let Some((e, _)) = corpus.get(*i) else {
            continue;
        };
        let n = sources.len() + 1;
        let secs = (e.ts_usec / 1_000_000) as i64;
        let cuerpo: String = e.cuerpo.chars().take(BODY_CHARS).collect();
        let cuerpo = cuerpo.split_whitespace().collect::<Vec<_>>().join(" ");
        let frag = format!(
            "[{n}] {clase} · {origen} · {fecha}\n{titulo}\n{cuerpo}\n\n",
            clase = e.clase.slug(),
            origen = e.origen,
            fecha = ymd(secs),
            titulo = e.titulo,
        );
        if used + frag.len() > MAX_CONTEXT_CHARS && !sources.is_empty() {
            break;
        }
        used += frag.len();
        frags.push_str(&frag);
        sources.push(RagSource {
            id: e.id_hex(),
            subject: e.titulo.clone(),
            from: e.origen.clone(),
            date: secs,
            mailbox: e.clase.slug().to_string(),
            score: *score,
        });
    }

    let user = format!(
        "Pregunta: {query}\n\nFragmentos de tus eventos:\n\n{frags}Respondé la pregunta citando \
         entre corchetes los números de los fragmentos que uses."
    );
    (system, user, sources)
}

/// Construye el cliente LLM del RAG desde la config global (`wawa.ai.llm`); si no
/// se fijó backend, cae a `from_env` (local-first). `None` si el backend nombrado
/// es desconocido o si el resultante es el Mock sin opt-in explícito. Mismo
/// criterio que `paloma-rag`.
fn build_rag_llm(s: &wawa_config::LlmSettings) -> Option<(Arc<dyn ChatClient>, bool)> {
    use pluma_llm::{build_client, BackendKind, LlmConfig};
    let explicit = s.is_set() || std::env::var("PLUMA_LLM_BACKEND").is_ok();
    let client: Arc<dyn ChatClient> = if s.is_set() {
        let kind = match s.backend.trim().to_lowercase().as_str() {
            "anthropic" => BackendKind::Anthropic,
            "gemini" => BackendKind::Gemini,
            "deepseek" => BackendKind::DeepSeek,
            "cohere" => BackendKind::Cohere,
            "ollama" => BackendKind::Ollama,
            "mock" => BackendKind::Mock,
            _ => return None,
        };
        let nz = |v: &str| {
            let v = v.trim();
            (!v.is_empty()).then(|| v.to_string())
        };
        build_client(&LlmConfig {
            kind,
            model: nz(&s.model),
            api_key: nz(&s.api_key),
            endpoint: nz(&s.endpoint),
        })
        .ok()?
    } else {
        pluma_llm::from_env().ok()?
    };
    if client.model_id() == "pluma-llm-mock" && !explicit {
        return None;
    }
    Some((client, true))
}

/// Fecha `YYYY-MM-DD` (UTC) de un timestamp Unix en segundos, sin dependencias de
/// calendario (algoritmo civil de Howard Hinnant). Para rotular fuentes.
fn ymd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use willay_core::{Clase, Payload};

    fn ev(clase: Clase, ts_secs: u64, titulo: &str, cuerpo: &str) -> Evento {
        Evento::nuevo(clase, ts_secs * 1_000_000, "test", titulo, cuerpo, Payload::Nada)
    }

    #[tokio::test]
    async fn rankear_ordena_por_coseno_descendente() {
        use rimay_verbo::MockProvider;
        let p = MockProvider::new(64);
        let a = ev(Clase::Clip, 1, "git push", "git push origin main");
        let b = ev(Clase::Captura, 2, "factura", "el pago vence el viernes");
        let corpus = embed_eventos(&p, vec![a.clone(), b]).await;
        // Consulta = el texto exacto de `a` → mock determinista, coseno 1.0 con a.
        let q = p.embed(&embeddable(&a)).await.unwrap();
        let r = rankear(&q, &corpus, 6, 0.0);
        assert_eq!(r[0].0, 0, "el evento idéntico rankea primero");
        assert!(r[0].1 >= r[1].1, "scores descendentes");
    }

    #[tokio::test]
    async fn assemble_numera_cita_y_mapea_a_ragsource() {
        use rimay_verbo::MockProvider;
        let p = MockProvider::new(32);
        let a = ev(Clase::Clip, 1_700_000_000, "API key", "sk-secreta-123");
        let b = ev(Clase::Notificacion, 1_700_000_001, "build ok", "compilación terminó");
        let corpus = embed_eventos(&p, vec![a.clone(), b]).await;
        let ranked = vec![(0usize, 0.9f32), (1usize, 0.4f32)];
        let (system, user, sources) = assemble("¿cuál era la api key?", &ranked, &corpus);

        assert!(system.contains("[1]"), "el system fija citar [n]");
        assert_eq!(sources.len(), 2);
        // Mapeo evento→RagSource genérico.
        assert_eq!(sources[0].id, a.id_hex());
        assert_eq!(sources[0].subject, "API key");
        assert_eq!(sources[0].from, "test");
        assert_eq!(sources[0].mailbox, "clip");
        assert!(user.contains("¿cuál era la api key?"));
        assert!(user.contains("[1] clip"));
        assert!(user.contains("[2] notificacion"));
        assert!(user.contains("sk-secreta-123"));
    }

    /// Pipeline completo con tipos reales pero sin red ni disco: embeddings
    /// deterministas (MockProvider), corpus en memoria y un ChatClient mock que
    /// hace eco del prompt. Certifica recuperar → fundamentar → redactar.
    #[tokio::test]
    async fn answer_recupera_fundamenta_y_redacta() {
        use pluma_llm::{build_client, BackendKind, LlmConfig};
        use rimay_verbo::MockProvider;

        let p = MockProvider::new(64);
        let cap = ev(Clase::Captura, 100, "Captura DP-1", "el error de compilación rojo");
        let clip = ev(Clase::Clip, 200, "git push", "git push origin main");
        let corpus = embed_eventos(&p, vec![cap.clone(), clip]).await;

        let client =
            build_client(&LlmConfig { kind: BackendKind::Mock, ..Default::default() }).unwrap();

        // Consulta = texto del evento de la captura → rankea primero (coseno 1.0).
        let query = embeddable(&cap);
        let ans = answer(&p, &*client, &corpus, &query).await.unwrap();

        assert!(!ans.sources.is_empty());
        assert_eq!(ans.sources[0].id, cap.id_hex());
        // El mock hace eco del prompt → la respuesta lleva el cuerpo citado.
        assert!(ans.answer.contains("el error de compilación rojo"), "answer={:?}", ans.answer);
    }
}
